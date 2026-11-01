//! Platform update lifecycle. Sparkle owns macOS discovery, verification and UI;
//! Rustable owns the decision to release an update's deferred restart.
#[cfg(all(target_os = "macos", feature = "sparkle-updater"))]
mod macos {
    use eframe::egui;
    use objc2::MainThreadMarker;
    use objc2_foundation::{NSBundle, NSString};
    use sparkle_updater::{RelaunchContinuation, SparkleUpdater, UpdaterConfig};
    use std::{cell::RefCell, rc::Rc};

    struct State {
        updater: SparkleUpdater,
        restart: Rc<RefCell<Option<RelaunchContinuation>>>,
        check_requested: bool,
    }
    thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }

    pub fn initialize(ctx: &egui::Context) {
        // Development and QA binaries must never replace themselves with a public release.
        if cfg!(debug_assertions) || cfg!(feature = "qa") { return; }
        let Some(main_thread) = MainThreadMarker::new() else { return; };
        let bundle = NSBundle::mainBundle();
        let configured = ["SUFeedURL", "SUPublicEDKey"].iter().all(|key| {
            bundle.objectForInfoDictionaryKey(&NSString::from_str(key))
                .and_then(|value| value.downcast::<NSString>().ok())
                .is_some_and(|value| !value.to_string().is_empty())
        });
        if !configured { return; }
        let restart = Rc::new(RefCell::new(None));
        let pending = restart.clone();
        let context = ctx.clone();
        let config = UpdaterConfig {
            relaunch_handler: Some(Rc::new(move |_, continuation| {
                *pending.borrow_mut() = Some(continuation);
                context.request_repaint_of(egui::ViewportId::ROOT);
            })),
            ..Default::default()
        };
        match SparkleUpdater::new(main_thread, config) {
            Ok(Some(updater)) => STATE.with(|slot| *slot.borrow_mut() = Some(State {
                updater, restart, check_requested: false,
            })),
            Ok(None) => {},
            Err(error) => eprintln!("Automatic updates could not start: {error}"),
        }
    }
    pub fn available() -> bool { STATE.with(|slot| slot.borrow().is_some()) }
    pub fn can_check() -> bool { STATE.with(|slot| slot.borrow().as_ref().is_some_and(|state| state.updater.can_check_for_updates().unwrap_or(false))) }
    pub fn request_check() {
        STATE.with(|slot| { if let Some(state) = slot.borrow_mut().as_mut() { state.check_requested = true; } });
    }
    pub fn poll() {
        STATE.with(|slot| {
            if let Some(state) = slot.borrow_mut().as_mut()
                && std::mem::take(&mut state.check_requested)
                && state.updater.can_check_for_updates().unwrap_or(false)
            { let _ = state.updater.check_for_updates(); }
        });
    }
    pub fn restart_pending() -> bool {
        STATE.with(|slot| slot.borrow().as_ref().is_some_and(|state| state.restart.borrow().is_some()))
    }
    pub fn resume_restart() {
        let pending = STATE.with(|slot| slot.borrow().as_ref().and_then(|state| state.restart.borrow_mut().take()));
        if let Some(pending) = pending { pending.resume(MainThreadMarker::new().expect("AppKit main thread")); }
    }
}

#[cfg(all(target_os = "macos", feature = "sparkle-updater"))]
pub use macos::*;

#[cfg(not(all(target_os = "macos", feature = "sparkle-updater")))]
pub fn initialize(_: &eframe::egui::Context) {}
#[cfg(all(target_os = "macos", not(feature = "sparkle-updater")))]
pub fn available() -> bool { false }
#[cfg(all(target_os = "macos", not(feature = "sparkle-updater")))]
pub fn request_check() {}
#[cfg(not(all(target_os = "macos", feature = "sparkle-updater")))]
pub fn poll() {}
#[cfg(not(all(target_os = "macos", feature = "sparkle-updater")))]
pub fn restart_pending() -> bool { false }
#[cfg(not(all(target_os = "macos", feature = "sparkle-updater")))]
pub fn resume_restart() {}

#[cfg(all(target_os = "macos", not(feature = "sparkle-updater")))]
pub fn can_check() -> bool { false }
