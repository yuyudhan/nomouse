//! macOS backend for nomouse: AXUIElement elements, NSPanel overlay, CGEvent input.
//!
//! The backend implements the [`nomouse_core::Platform`] traits; see `docs/architecture.md`.
//! OS-specific modules are compiled only on `target_os = "macos"`, so the crate checks on every
//! OS; the pure logic (role mapping, coordinates, key codes, the tree walk) is tested everywhere.
//!
//! - [`roles`]: which Accessibility roles get hints.
//! - [`geometry`]: global top-left (Accessibility, Quartz) versus Cocoa bottom-left coordinates.
//! - [`keys`]: virtual key codes for hotkeys and typed keys.
//! - [`icon_watch`]: whether the menu bar item came up, and its retries when it did not.
//! - [`walk`]: the bounded, cycle-safe tree walk over any [`walk::AxTree`].
//! - [`style`]: the overlay look from the configuration.
//! - [`setup`]: the guided first run's permission and its texts.
//! - [`shortcuts`]: recorded shortcuts, their `⌃⌥⇧⌘` spelling and macOS's own shortcuts.
//! - `menubar` (macOS): the menu bar app, its settings window and the first run's window.
#![forbid(unsafe_code)]

pub mod bundle;
pub mod geometry;
pub mod icon_watch;
pub mod keys;
pub mod roles;
pub mod setup;
pub mod shortcuts;
pub mod style;
pub mod walk;

#[cfg(target_os = "macos")]
mod ax_tree;
#[cfg(target_os = "macos")]
pub mod menubar;
#[cfg(target_os = "macos")]
mod platform;

#[cfg(target_os = "macos")]
pub use platform::MacPlatform;

use nomouse_core::setup::Permission;
use nomouse_core::{Config, Platform, PlatformError};

/// The OS this backend serves.
pub const TARGET_OS: &str = "macos";

/// What the user must do so nomouse can read the accessibility tree on this OS.
pub const ACCESSIBILITY_REQUIREMENT: &str = "Grant nomouse Accessibility access in System \
    Settings > Privacy & Security > Accessibility (Device Control and Data Access on macOS 27).";

/// Creates the macOS backend. Call on the main thread; shows the Accessibility prompt when
/// trust is missing.
///
/// # Errors
/// [`PlatformError::PermissionDenied`] without Accessibility trust,
/// [`PlatformError::Unsupported`] off the main thread or on another OS.
#[cfg(target_os = "macos")]
pub fn platform(config: &Config) -> Result<Box<dyn Platform>, PlatformError> {
    Ok(Box::new(MacPlatform::new(config)?))
}

/// Creates the macOS backend; on other systems this always fails.
///
/// # Errors
/// Always [`PlatformError::Unsupported`]: this build is not for macOS.
#[cfg(not(target_os = "macos"))]
pub fn platform(_config: &Config) -> Result<Box<dyn Platform>, PlatformError> {
    Err(PlatformError::Unsupported(
        "the macOS backend runs only on macOS".to_owned(),
    ))
}

/// The permissions the guided first run walks through on macOS: Accessibility.
pub fn first_run_permissions() -> Vec<Permission> {
    setup::permissions()
}

/// Checks and requests Accessibility trust for the guided first run.
#[cfg(target_os = "macos")]
pub fn first_run_probe() -> Box<dyn nomouse_core::setup::Probe> {
    Box::new(setup::TrustProbe)
}

/// The first run's menu bar item and Welcome window, for a nomouse started without a terminal
/// (`nomouse.app`); `None` off the main thread.
#[cfg(target_os = "macos")]
pub fn first_run_dialogs() -> Option<Box<dyn nomouse_core::setup::Presenter>> {
    menubar::Onboarding::new()
        .ok()
        .map(|onboarding| Box::new(onboarding) as Box<dyn nomouse_core::setup::Presenter>)
}

/// Environment checks for `nomouse doctor`: (check name, `Ok(detail)` or `Err(problem and fix)`).
#[cfg(target_os = "macos")]
pub fn doctor(config: &Config) -> Vec<(String, Result<String, String>)> {
    use nomouse_sys_macos::{app, ax, input};

    let mut checks = vec![("macOS version".to_owned(), Ok(app::os_version()))];
    let trusted = ax::is_process_trusted(false);
    checks.push((
        "Accessibility".to_owned(),
        setup::accessibility_check(trusted, &setup::grant_holder()),
    ));
    checks.push((
        "Synthetic input".to_owned(),
        if input::can_post_events() {
            Ok("clicks and scrolls can be posted".to_owned())
        } else {
            Err(format!(
                "posting events is blocked. {ACCESSIBILITY_REQUIREMENT}"
            ))
        },
    ));
    let listen = if input::can_listen_events() {
        "granted"
    } else {
        "not granted (not needed: the keyboard tap runs under Accessibility)"
    };
    checks.push(("Input Monitoring".to_owned(), Ok(listen.to_owned())));
    let on = input::secure_input_enabled();
    let holder = if on { secure_input_holder() } else { None };
    checks.push((
        "Secure input".to_owned(),
        setup::secure_input_check(on, holder),
    ));
    let apps = &config.macos.enhanced_ui_apps;
    checks.push((
        "Chromium/Electron opt-in".to_owned(),
        Ok(if apps.is_empty() {
            "macos.enhanced_ui_apps is empty".to_owned()
        } else {
            format!("AXEnhancedUserInterface for {}", apps.join(", "))
        }),
    ));
    checks
}

/// The pid and app name holding secure event input, from the console session in the I/O
/// Registry; `None` when `ioreg` fails or does not name it.
#[cfg(target_os = "macos")]
fn secure_input_holder() -> Option<(i32, Option<String>)> {
    let output = std::process::Command::new("/usr/sbin/ioreg")
        .args(["-l", "-w", "0", "-d", "1"])
        .output()
        .ok()?;
    let pid = setup::secure_input_pid(&String::from_utf8_lossy(&output.stdout))?;
    Some((pid, nomouse_sys_macos::app::process_name(pid)))
}

/// Environment checks for `nomouse doctor`; on other systems, one failing check.
#[cfg(not(target_os = "macos"))]
pub fn doctor(_config: &Config) -> Vec<(String, Result<String, String>)> {
    vec![(
        "macOS backend".to_owned(),
        Err("this build is not running on macOS".to_owned()),
    )]
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn other_systems_report_unsupported() {
        let config = Config::default();
        assert!(matches!(
            platform(&config),
            Err(PlatformError::Unsupported(_))
        ));
        let checks = doctor(&config);
        assert_eq!(checks.len(), 1);
        assert!(checks[0].1.is_err());
    }
}
