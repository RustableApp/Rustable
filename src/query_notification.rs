//! Query completion alerts contain no SQL, connection details, or row data.
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome { Finished, Failed }

pub(crate) fn take_background_completion(pending: &mut Option<Outcome>, focused: bool) -> Option<Outcome> {
    pending.take().filter(|_| !focused)
}

pub(crate) fn prepare() {
    #[cfg(all(target_os = "macos", not(test)))]
    macos::prepare();
}

pub(crate) fn show(_ctx: &egui::Context, _outcome: Outcome) {
    #[cfg(all(target_os = "macos", not(test)))]
    macos::show(_ctx, _outcome);
}

#[cfg(all(target_os = "macos", not(test)))]
mod macos {
    use super::*;
    use block2::RcBlock;
    use objc2::{define_class, msg_send, AnyThread};
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, ProtocolObject};
    use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
    use objc2_user_notifications::*;
    use std::cell::OnceCell;
    use std::sync::atomic::{AtomicU64, Ordering};

    define_class!(
        // SAFETY: Stateless NSObject subclass. The callback touches no AppKit
        // state and may be invoked on Notification Center's callback thread.
        #[unsafe(super(NSObject))]
        #[name = "RustableQueryNotificationDelegate"]
        struct Delegate;
        unsafe impl NSObjectProtocol for Delegate {}
        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn present(&self, _center: &UNUserNotificationCenter, _notification: &UNNotification,
                completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
                // The query's window was unfocused when this alert was posted.
                // Show it even if a different Rustable window is in front.
                completion.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,));
            }
        }
    );

    thread_local! {
        // Notification Center keeps only a weak delegate reference.
        static DELEGATE: OnceCell<Retained<Delegate>> = const { OnceCell::new() };
    }
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    fn center() -> Option<Retained<UNUserNotificationCenter>> {
        // UNUserNotificationCenter requires an app bundle; cargo run and unit
        // tests must never call it as an unbundled command-line executable.
        if !NSBundle::mainBundle().bundlePath().to_string().ends_with(".app") {
            return None;
        }
        Some(UNUserNotificationCenter::currentNotificationCenter())
    }

    pub(super) fn prepare() {
        let Some(center) = center() else { return; };
        DELEGATE.with(|delegate| {
            delegate.get_or_init(|| {
                // SAFETY: NSObject init initializes this stateless subclass.
                let target: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(), init] };
                center.setDelegate(Some(ProtocolObject::from_ref(&*target)));
                center.requestAuthorizationWithOptions_completionHandler(
                    UNAuthorizationOptions::Alert,
                    &RcBlock::new(|_granted: Bool, _error: *mut NSError| {}),
                );
                target
            });
        });
    }

    pub(super) fn show(ctx: &egui::Context, outcome: Outcome) {
        let Some(center) = center() else { return; };
        prepare();
        let ctx = ctx.clone();
        let viewport = ctx.viewport_id();
        // The OS remembers authorization/denial; this also handles completion
        // while the first permission prompt is still being answered.
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert,
            &RcBlock::new(move |granted: Bool, _error: *mut NSError| {
                if !granted.as_bool() || ctx.input_for(viewport, |input| input.focused) { return; }
                let content = UNMutableNotificationContent::new();
                let (title, body) = match outcome {
                    Outcome::Finished => ("Query finished", "Your query completed. Return to Rustable to view the results."),
                    Outcome::Failed => ("Query failed", "Your query ended with an error. Return to Rustable for details."),
                };
                content.setTitle(&NSString::from_str(title));
                content.setBody(&NSString::from_str(body));
                let id = format!("rustable-query-{}-{}", std::process::id(), NEXT_ID.fetch_add(1, Ordering::Relaxed));
                let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                    &NSString::from_str(&id), &content, None,
                );
                UNUserNotificationCenter::currentNotificationCenter()
                    .addNotificationRequest_withCompletionHandler(&request, None);
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notifications_are_once_only_and_do_not_wait_for_focus_loss() {
        for outcome in [Outcome::Finished, Outcome::Failed] {
            let mut pending = Some(outcome);
            assert_eq!(take_background_completion(&mut pending, false), Some(outcome));
            assert_eq!(take_background_completion(&mut pending, false), None);
            pending = Some(outcome);
            assert_eq!(take_background_completion(&mut pending, true), None);
            assert_eq!(take_background_completion(&mut pending, false), None);
        }
    }
}
