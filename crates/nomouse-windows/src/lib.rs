//! Windows backend for nomouse: UI Automation elements, layered-window overlay, SendInput input.
//!
//! The backend implements the [`nomouse_core::Platform`] traits; see `docs/architecture.md`.
//! All logic lives in [`Backend`], written against the small [`System`] seam and tested with a
//! fake on every OS; on Windows, [`NativeSystem`] implements the seam over
//! `nomouse-sys-windows`, and [`platform`] / [`doctor`] are what the binary calls.
#![forbid(unsafe_code)]

mod backend;
mod classify;
mod input_plan;
mod keys;
#[cfg(windows)]
mod native;
mod render;
mod scope;

pub use backend::{Backend, System};
#[cfg(windows)]
pub use native::{NativeSystem, WindowsPlatform, doctor, platform, platform_with};
pub use nomouse_sys_windows::{WindowHandle, WindowInfo};
pub use scope::ScopeOptions;

/// Off Windows the backend cannot run: always [`PlatformError::Unsupported`].
///
/// # Errors
/// Always.
#[cfg(not(windows))]
pub fn platform(
    _config: &nomouse_core::Config,
) -> Result<Box<dyn nomouse_core::Platform>, nomouse_core::PlatformError> {
    Err(nomouse_core::PlatformError::Unsupported(
        "the Windows backend runs only on Windows".to_owned(),
    ))
}

/// Off Windows: one failing check saying the backend needs Windows.
#[cfg(not(windows))]
pub fn doctor(_config: &nomouse_core::Config) -> Vec<(String, Result<String, String>)> {
    vec![(
        "Windows backend".to_owned(),
        Err("this backend runs only on Windows".to_owned()),
    )]
}

/// The OS this backend serves.
pub const TARGET_OS: &str = "windows";

/// What the user must do so nomouse can read the accessibility tree on this OS.
pub const ACCESSIBILITY_REQUIREMENT: &str = "UI Automation is always on; run nomouse at the same integrity level as the apps it should reach (elevated apps need an elevated nomouse).";
