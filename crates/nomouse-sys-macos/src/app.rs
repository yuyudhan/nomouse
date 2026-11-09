//! The AppKit application object, the run loop, screens and running applications.

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSApplication, NSApplicationActivationPolicy, NSEventMask, NSRunningApplication, NSScreen,
    NSWorkspace,
};
use objc2_core_foundation::{CFRunLoop, kCFRunLoopDefaultMode};
use objc2_foundation::{NSArray, NSDate, NSDefaultRunLoopMode, NSProcessInfo, NSString};

use crate::types::ScreenFrame;

/// AppKit objects may only be used on the main thread, and nomouse was called elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotMainThread;

impl std::fmt::Display for NotMainThread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the macOS backend must be created and used on the main thread")
    }
}

impl std::error::Error for NotMainThread {}

/// The process's `NSApplication`, configured as an accessory app (no Dock icon, never takes
/// focus from the app being labelled).
#[derive(Debug)]
pub struct App {
    app: Retained<NSApplication>,
    mtm: MainThreadMarker,
}

impl App {
    /// Creates or fetches the shared application.
    ///
    /// # Errors
    /// [`NotMainThread`] when called off the main thread.
    pub fn new() -> Result<Self, NotMainThread> {
        let mtm = MainThreadMarker::new().ok_or(NotMainThread)?;
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
        Ok(Self { app, mtm })
    }

    /// The main-thread marker, for creating other AppKit objects.
    pub const fn main_thread(&self) -> MainThreadMarker {
        self.mtm
    }

    /// Runs the run loop for up to `seconds` (returning early once a source fired, such as the
    /// event tap), then dispatches pending AppKit events.
    pub fn pump(&self, seconds: f64) {
        // SAFETY: kCFRunLoopDefaultMode is an immutable CFString constant.
        let mode = unsafe { kCFRunLoopDefaultMode };
        CFRunLoop::run_in_mode(mode, seconds, true);
        let past = NSDate::distantPast();
        loop {
            // SAFETY: NSDefaultRunLoopMode is an immutable NSString constant, and the date is
            // a valid NSDate; the call only dequeues events already pending.
            let event = unsafe {
                self.app.nextEventMatchingMask_untilDate_inMode_dequeue(
                    NSEventMask::Any,
                    Some(&past),
                    NSDefaultRunLoopMode,
                    true,
                )
            };
            let Some(event) = event else { break };
            self.app.sendEvent(&event);
        }
    }

    /// Every screen's frame, the primary (menu bar) screen first.
    pub fn screens(&self) -> Vec<ScreenFrame> {
        NSScreen::screens(self.mtm)
            .iter()
            .map(|screen| {
                let frame = screen.frame();
                ScreenFrame {
                    x: frame.origin.x,
                    y: frame.origin.y,
                    width: frame.size.width,
                    height: frame.size.height,
                }
            })
            .collect()
    }

    /// Each screen's backing scale factor (2.0 on Retina), in [`App::screens`] order.
    pub fn screen_scales(&self) -> Vec<f64> {
        NSScreen::screens(self.mtm)
            .iter()
            .map(|screen| screen.backingScaleFactor())
            .collect()
    }

    /// Whether the app's effective appearance is dark (Dark Aqua, or its high-contrast
    /// variant): it follows System Settings > Appearance, including its Auto schedule, as the
    /// run loop delivers the change.
    pub fn prefers_dark(&self) -> bool {
        // SAFETY: both names are immutable NSString constants exported by AppKit.
        let (light, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
        let names = NSArray::from_slice(&[light, dark]);
        self.app
            .effectiveAppearance()
            .bestMatchFromAppearancesWithNames(&names)
            .is_some_and(|best| best.isEqualToString(dark))
    }

    /// Shows a modal alert with `title`, `message` and `buttons` (the first is the default and
    /// answers Return), in front of other apps, and returns the index of the button clicked.
    pub fn alert(&self, title: &str, message: &str, buttons: &[&str]) -> usize {
        let alert = NSAlert::new(self.mtm);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(message));
        for button in buttons {
            alert.addButtonWithTitle(&NSString::from_str(button));
        }
        // An agent app (no Dock icon) is not activated by showing a window; `activate` exists
        // from macOS 14. Earlier systems activated the app when it was opened.
        if self.app.respondsToSelector(sel!(activate)) {
            self.app.activate();
        }
        let response = alert.runModal();
        usize::try_from(response - NSAlertFirstButtonReturn).unwrap_or(0)
    }
}

/// The process id of the frontmost application, the one owning the menu bar.
pub fn frontmost_pid() -> Option<i32> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier())
        .filter(|pid| *pid > 0)
}

/// The bundle identifier (such as `com.apple.TextEdit`) of the running application `pid`.
pub fn bundle_id(pid: i32) -> Option<String> {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|app| app.bundleIdentifier())
        .map(|id| id.to_string())
}

/// The executable's file name (such as `Terminal`) of the running application `pid`.
pub fn process_name(pid: i32) -> Option<String> {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|app| app.executableURL())
        .and_then(|url| url.lastPathComponent())
        .map(|name| name.to_string())
        .filter(|name| !name.is_empty())
}

/// A running application, as `NSWorkspace` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningApp {
    /// Process id.
    pub pid: i32,
    /// Bundle identifier, when the app has one.
    pub bundle_id: Option<String>,
    /// Whether it is a regular app (has a Dock icon and windows), as opposed to an accessory
    /// or background agent (which may still own menu bar extras).
    pub regular: bool,
}

/// Every running application that can have UI (regular and accessory apps).
pub fn running_apps() -> Vec<RunningApp> {
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter_map(|app| {
            let policy = app.activationPolicy();
            let regular = policy == NSApplicationActivationPolicy::Regular;
            let accessory = policy == NSApplicationActivationPolicy::Accessory;
            let pid = app.processIdentifier();
            (pid > 0 && (regular || accessory)).then(|| RunningApp {
                pid,
                bundle_id: app.bundleIdentifier().map(|id| id.to_string()),
                regular,
            })
        })
        .collect()
}

/// The macOS version string, such as `Version 15.1 (Build 24B83)`.
pub fn os_version() -> String {
    NSProcessInfo::processInfo()
        .operatingSystemVersionString()
        .to_string()
}
