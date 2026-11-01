//! Guard AppKit's menu, Dock, and OS termination requests.
//! Winit owns NSApplication and its delegate; neither is replaced here. We add
//! only its absent applicationShouldTerminate: method, using AppKit's documented
//! synchronous Now/Cancel replies (Later would stall winit's default-mode events).
//! https://developer.apple.com/documentation/appkit/nsapplicationdelegate/applicationshouldterminate(_:)
#[cfg(target_os = "macos")]
mod macos {
    use eframe::egui;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{NSApplication, NSMenu};
    use objc2_foundation::{NSObject, NSObjectProtocol};
    use std::{
        cell::RefCell,
        sync::atomic::{AtomicBool, Ordering},
    };

    static REQUESTED: AtomicBool = AtomicBool::new(false);
    // Fail closed until SessionWindows publishes its complete end-of-frame
    // state. Root marks this blocked before entering each update/modal action.
    static BLOCKED: AtomicBool = AtomicBool::new(true);
    static DELEGATE_HOOKED: AtomicBool = AtomicBool::new(false);
    struct Ivars {
        context: egui::Context,
    }
    define_class!(
        // SAFETY: NSObject has no subclass invariants; this main-thread-only
        // target owns a Context and defines no custom Drop behavior.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "RustableGuardedQuitTarget"]
        #[ivars = Ivars]
        struct QuitTarget;
        unsafe impl NSObjectProtocol for QuitTarget {}
        impl QuitTarget {
            // AppKit target/action convention: void action(id sender).
            #[unsafe(method(validateMenuItem:))]
            fn validate_menu_item(&self, item: &objc2_app_kit::NSMenuItem) -> bool {
                item.action() != Some(sel!(rustableCheckForUpdates:)) || crate::updates::can_check()
            }
            #[unsafe(method(rustableCheckForUpdates:))]
            fn check_for_updates(&self, _sender: Option<&AnyObject>) {
                crate::updates::request_check();
                self.ivars().context.request_repaint_of(egui::ViewportId::ROOT);
            }
            #[unsafe(method(rustableRequestQuit:))]
            fn request_quit(&self, _sender: Option<&AnyObject>) {
                request(&self.ivars().context);
            }
        }
    );
    fn request(context: &egui::Context) {
        REQUESTED.store(true, Ordering::Release);
        context.request_repaint_of(egui::ViewportId::ROOT);
    }
    // AppKit NSApplicationTerminateReply is NSUInteger. Only add a previously
    // absent method to the known winit delegate; never replace its methods or
    // delegate object. NSTerminateNow follows winit's willTerminate->LoopExiting
    // path, where eframe saves/flushes before destroying the application.
    extern "C-unwind" fn should_terminate(
        _delegate: &AnyObject,
        _selector: objc2::runtime::Sel,
        _application: &NSApplication,
    ) -> usize {
        termination_reply(BLOCKED.load(Ordering::Acquire), || {
            TARGET.with(|slot| {
                if let Some(target) = slot.borrow().as_ref() {
                    request(&target.ivars().context);
                }
            });
        })
    }
    fn termination_reply(blocked: bool, request_guard: impl FnOnce()) -> usize {
        if blocked {
            request_guard();
            0 // NSTerminateCancel; root focuses the existing guarded workspace.
        } else {
            1 // NSTerminateNow; no modal run loop and no cancelled clean logout.
        }
    }
    fn install_delegate_hook(application: &NSApplication) -> bool {
        if DELEGATE_HOOKED.load(Ordering::Acquire) {
            return true;
        }
        let Some(delegate) = application.delegate() else {
            return false;
        };
        let object: &AnyObject = (*delegate).as_ref();
        let class = object.class();
        let selector = sel!(applicationShouldTerminate:);
        if class.name() != c"WinitApplicationDelegate" || class.instance_method(selector).is_some()
        {
            return false;
        }
        use objc2::Encode;
        let encoding = std::ffi::CString::new(format!("{}@:@", usize::ENCODING))
            .expect("Objective-C type encoding");
        // SAFETY: exact AppKit selector ABI: NSUInteger(id,SEL,NSApplication*).
        // The class is runtime-owned mutable metadata, this runs on the main
        // thread, and class_addMethod refuses to overwrite an existing method.
        let installed = unsafe {
            let implementation: objc2::runtime::Imp = std::mem::transmute(
                should_terminate
                    as extern "C-unwind" fn(
                        &AnyObject,
                        objc2::runtime::Sel,
                        &NSApplication,
                    ) -> usize,
            );
            objc2::ffi::class_addMethod(
                (class as *const objc2::runtime::AnyClass).cast_mut(),
                selector,
                implementation,
                encoding.as_ptr(),
            )
            .as_bool()
        };
        DELEGATE_HOOKED.store(installed, Ordering::Release);
        installed
    }
    thread_local! {
        // NSMenuItem.target is weak, so retain the receiver for the UI thread's
        // lifetime. No dangling callback if the SessionWindows value moves.
        static TARGET: RefCell<Option<Retained<QuitTarget>>> = const { RefCell::new(None) };
    }
    fn retarget(menu: &NSMenu, target: &QuitTarget) -> bool {
        for item in menu.itemArray() {
            #[cfg(feature = "qa")]
            if item.tag() == QA_QUIT_TAG {
                continue;
            }
            if item.action() == Some(sel!(terminate:)) {
                // SAFETY: target implements the exact action signature and is
                // retained in TARGET before returning to the AppKit eventloop.
                unsafe {
                    item.setTarget(Some(target));
                    item.setAction(Some(sel!(rustableRequestQuit:)));
                }
                return true;
            }
            if item.action() == Some(sel!(rustableRequestQuit:)) {
                return true;
            }
            if let Some(submenu) = item.submenu()
                && retarget(&submenu, target)
            {
                return true;
            }
        }
        false
    }
    #[cfg(feature = "qa")]
    const QA_QUIT_TAG: isize = 0x52535154;
    #[cfg(feature = "qa")]
    fn install_qa_system_quit(menu: &NSMenu, application: &NSApplication) {
        use objc2_app_kit::{NSEventModifierFlags, NSMenuItem};
        use objc2_foundation::NSString;
        if menu.itemWithTag(QA_QUIT_TAG).is_some() {
            return;
        }
        for item in menu.itemArray() {
            if item.action() == Some(sel!(rustableRequestQuit:)) {
                // SAFETY: NSApplication implements terminate: with the AppKit
                // action ABI. This QA-only item intentionally bypasses the
                // menu shortcut shim to exercise applicationShouldTerminate:.
                let probe = unsafe {
                    let probe = NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(application.mtm()),
                        &NSString::from_str("Test system quit"),
                        Some(sel!(terminate:)),
                        &NSString::from_str("q"),
                    );
                    probe.setTarget(Some(application));
                    probe
                };
                probe.setTag(QA_QUIT_TAG);
                probe.setKeyEquivalentModifierMask(
                    NSEventModifierFlags::Command | NSEventModifierFlags::Option,
                );
                menu.addItem(&probe);
                return;
            }
            if let Some(submenu) = item.submenu() {
                install_qa_system_quit(&submenu, application);
            }
        }
    }
    pub(super) fn install(context: &egui::Context) -> bool {
        let Some(main_thread) = MainThreadMarker::new() else {
            return false;
        };
        let application = NSApplication::sharedApplication(main_thread);
        let Some(menu) = application.mainMenu() else {
            return false;
        };
        TARGET.with(|slot| {
            let mut slot = slot.borrow_mut();
            let target = slot.get_or_insert_with(|| {
                let allocated = QuitTarget::alloc(main_thread).set_ivars(Ivars {
                    context: context.clone(),
                });
                // SAFETY: NSObject init initializes the allocated subclass.
                unsafe { msg_send![super(allocated), init] }
            });
            let menu_installed = retarget(&menu, target);
            let delegate_installed = install_delegate_hook(&application);
            if menu_installed && crate::updates::available()
                && let Some(app_menu) = menu.itemAtIndex(0).and_then(|item| item.submenu())
                && app_menu.itemWithTag(0x52535550).is_none()
            {
                use objc2_app_kit::NSMenuItem;
                use objc2_foundation::NSString;
                // SAFETY: this retained main-thread target implements the exact action ABI.
                let item = unsafe {
                    let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(main_thread), &NSString::from_str("Check for Updates…"),
                        Some(sel!(rustableCheckForUpdates:)), &NSString::from_str(""),
                    );
                    item.setTarget(Some(target));
                    item
                };
                item.setTag(0x52535550);
                app_menu.insertItem_atIndex(&item, 1.min(app_menu.numberOfItems()));
            }
            #[cfg(feature = "qa")]
            if menu_installed && delegate_installed {
                install_qa_system_quit(&menu, &application);
            }
            menu_installed && delegate_installed
        })
    }
    pub(super) fn take_requested() -> bool {
        REQUESTED.swap(false, Ordering::AcqRel)
    }
    pub(super) fn set_blocked(blocked: bool) {
        BLOCKED.store(blocked, Ordering::Release);
    }
    #[cfg(test)]
    #[test]
    fn requests_are_coalesced_and_consumed_once() {
        let context = egui::Context::default();
        assert!(!take_requested());
        request(&context);
        request(&context);
        assert!(take_requested());
        assert!(!take_requested());
    }
    #[cfg(test)]
    #[test]
    fn clean_termination_is_immediate_and_blocked_termination_requests_guard() {
        let requested = std::cell::Cell::new(false);
        assert_eq!(termination_reply(false, || requested.set(true)), 1);
        assert!(!requested.get());
        assert_eq!(termination_reply(true, || requested.set(true)), 0);
        assert!(requested.get());
        // NSApplicationTerminateReply is NSUInteger, not Objective-C BOOL.
        use objc2::Encode;
        assert_eq!(
            usize::ENCODING,
            objc2_app_kit::NSApplicationTerminateReply::ENCODING
        );
    }
}

/// Mark blocked at update entry; publish the complete root/child guard state at
/// update exit. The initial state is blocked until a complete frame is known.
pub fn set_blocked(blocked: bool) {
    #[cfg(target_os = "macos")]
    macos::set_blocked(blocked);
    #[cfg(not(target_os = "macos"))]
    let _ = blocked;
}

/// Install after the native app menu exists. False means retry on a later frame.
pub fn install(context: &eframe::egui::Context) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::install(context)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = context;
        true
    }
}
/// Consume one coalesced native menu quit request without terminating AppKit.
pub fn take_requested() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::take_requested()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
