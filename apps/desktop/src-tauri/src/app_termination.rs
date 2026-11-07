//! macOS asks the app delegate before Cmd+Q, Dock quit, logout, restart or shutdown
//! terminates the process. tao's delegate does not answer, so AppKit used to quit
//! at once and skip the confirmation and shutdown in `request_quit`. Nero now
//! answers "later", runs that same flow, and replies when it has finished or the
//! person chose to keep recording. Logout waits for the reply.
use std::sync::atomic::{AtomicBool, Ordering};

static REPLY_OWED: AtomicBool = AtomicBool::new(false);

/// Completes a pending system termination. Returns false when none was pending,
/// so the caller exits or stays by its own means.
pub fn reply(app: &tauri::AppHandle, terminate: bool) -> bool {
    if !REPLY_OWED.swap(false, Ordering::AcqRel) {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        let sent = app.run_on_main_thread(move || macos::reply(terminate));
        if sent.is_err() && terminate {
            app.exit(0);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, terminate);
    true
}

#[cfg(target_os = "macos")]
pub use macos::install;

#[cfg(target_os = "macos")]
mod macos {
    use super::REPLY_OWED;
    use crate::DesktopState;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
    use objc2::{class, msg_send, sel};
    use std::sync::atomic::Ordering;
    use std::sync::OnceLock;
    use tauri::Manager;

    static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

    // NSApplicationTerminateReply
    const CANCEL: usize = 0;
    const NOW: usize = 1;
    const LATER: usize = 2;

    pub fn install(app: &tauri::AppHandle) {
        if APP.set(app.clone()).is_err() {
            return;
        }
        unsafe {
            let application: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let delegate: *mut AnyObject = msg_send![application, delegate];
            if delegate.is_null() {
                return;
            }
            let delegate_class = objc2::ffi::object_getClass(delegate) as *mut AnyClass;
            let handler: extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize =
                should_terminate;
            // Adds the method only when the delegate class has none of its own.
            objc2::ffi::class_addMethod(
                delegate_class,
                sel!(applicationShouldTerminate:),
                std::mem::transmute::<
                    extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize,
                    Imp,
                >(handler),
                c"Q@:@".as_ptr(),
            );
        }
    }

    extern "C-unwind" fn should_terminate(_: &AnyObject, _: Sel, _: *mut AnyObject) -> usize {
        let Some(app) = APP.get() else {
            return NOW;
        };
        let Some(state) = app.try_state::<DesktopState>() else {
            return NOW;
        };
        if state.quitting.load(Ordering::Acquire) {
            return NOW;
        }
        if REPLY_OWED.swap(true, Ordering::AcqRel) {
            // The earlier request is still waiting on the same confirmation.
            crate::show_main(app);
            return CANCEL;
        }
        crate::ask_quit(app);
        LATER
    }

    pub fn reply(terminate: bool) {
        unsafe {
            let application: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let _: () =
                msg_send![application, replyToApplicationShouldTerminate: Bool::new(terminate)];
        }
    }
}
