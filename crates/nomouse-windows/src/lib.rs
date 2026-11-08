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

/// The OS this backend serves.
pub const TARGET_OS: &str = "windows";

/// What the user must do so nomouse can read the accessibility tree on this OS.
pub const ACCESSIBILITY_REQUIREMENT: &str = "UI Automation is always on; run nomouse at the same integrity level as the apps it should reach (elevated apps need an elevated nomouse).";
