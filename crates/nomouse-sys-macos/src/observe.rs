//! Accessibility notifications from one app (`AXObserver`): whether its tree changed since the
//! last look, so a page that finishes loading while labels are shown is scanned at once.

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::{self, NonNull};

use objc2_application_services::{AXObserver, AXUIElement};
use objc2_core_foundation::{
    CFRetained, CFRunLoop, CFRunLoopSource, CFString, kCFRunLoopDefaultMode,
};

use crate::ax::AxElement;
use crate::error::AxError;

/// Watches one app's accessibility notifications on the main run loop (pumped by
/// [`crate::app::App::pump`]). Dropping it stops the notifications.
#[derive(Debug)]
pub struct AppObserver {
    pid: i32,
    /// Owned only to keep the observer (and its notifications) alive.
    _observer: CFRetained<AXObserver>,
    source: CFRetained<CFRunLoopSource>,
    /// Set by the callback; boxed so its address stays put for the callback's `refcon`.
    changed: Box<Cell<bool>>,
}

/// Called on the main run loop for each notification; `refcon` is the observer's `changed`.
unsafe extern "C-unwind" fn notified(
    _observer: NonNull<AXObserver>,
    _element: NonNull<AXUIElement>,
    _notification: NonNull<CFString>,
    refcon: *mut c_void,
) {
    // SAFETY: `refcon` is the `Cell<bool>` an `AppObserver` registered and owns; the callback
    // runs only on the main run loop, and the observer's run-loop source is removed (in
    // `Drop`) before the cell is freed, so the pointer is valid and nothing else borrows it.
    if let Some(changed) = unsafe { refcon.cast::<Cell<bool>>().as_ref() } {
        changed.set(true);
    }
}

impl AppObserver {
    /// Watches `notifications` (such as `AXLoadComplete`) of every element of app `pid`.
    /// Notifications the app does not post are skipped.
    ///
    /// # Errors
    /// The [`AxError`] of creating the observer (the app is gone, Accessibility is not
    /// trusted), or of the first notification when none could be added.
    pub fn new(pid: i32, notifications: &[&str]) -> Result<Self, AxError> {
        let mut raw: *mut AXObserver = ptr::null_mut();
        // SAFETY: `notified` matches `AXObserverCallback`, and `raw` is a valid slot that
        // receives a +1 observer on success.
        let code = unsafe { AXObserver::create(pid, Some(notified), NonNull::from(&mut raw)) };
        if let Some(error) = AxError::from_code(code.0) {
            return Err(error);
        }
        let observer = NonNull::new(raw).ok_or(AxError::Failure)?;
        // SAFETY: on success `raw` holds a +1 reference, which `CFRetained` now owns.
        let observer = unsafe { CFRetained::from_raw(observer) };
        let changed = Box::new(Cell::new(false));
        let refcon = ptr::from_ref::<Cell<bool>>(&changed)
            .cast_mut()
            .cast::<c_void>();
        let app = AxElement::application(pid);
        let mut added = 0_usize;
        let mut first_error = None;
        for name in notifications {
            let name = CFString::from_str(name);
            // SAFETY: the observer, element and name are valid CF objects; `refcon` points at
            // `changed`, which outlives the registration (see `notified`).
            let code = unsafe { observer.add_notification(app.raw(), &name, refcon) };
            match AxError::from_code(code.0) {
                None => added += 1,
                Some(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        if added == 0
            && let Some(error) = first_error
        {
            return Err(error);
        }
        // SAFETY: the observer is valid; its run-loop source is returned +0 and retained here.
        let source = unsafe { observer.run_loop_source() };
        let main = CFRunLoop::main().ok_or(AxError::Failure)?;
        // SAFETY: kCFRunLoopDefaultMode is an immutable CFString constant.
        let mode = unsafe { kCFRunLoopDefaultMode };
        main.add_source(Some(&source), mode);
        Ok(Self {
            pid,
            _observer: observer,
            source,
            changed,
        })
    }

    /// The app this observer watches.
    pub const fn pid(&self) -> i32 {
        self.pid
    }

    /// Whether a notification came since the last call.
    pub fn take_changed(&self) -> bool {
        self.changed.replace(false)
    }
}

impl Drop for AppObserver {
    /// Takes the source off the run loop, so no callback runs once `changed` is freed (the
    /// fields drop after this, in order).
    fn drop(&mut self) {
        if let Some(main) = CFRunLoop::main() {
            // SAFETY: kCFRunLoopDefaultMode is an immutable CFString constant.
            let mode = unsafe { kCFRunLoopDefaultMode };
            main.remove_source(Some(&self.source), mode);
        }
    }
}
