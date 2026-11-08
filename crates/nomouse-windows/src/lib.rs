//! Windows backend for nomouse: UI Automation elements, layered-window overlay, SendInput input.
//!
//! The backend implements the [`nomouse_core::Platform`] traits; see `docs/architecture.md`.
//! OS-specific modules are compiled only on `target_os = "windows"`, so the crate checks on every OS.
#![forbid(unsafe_code)]

/// The OS this backend serves.
pub const TARGET_OS: &str = "windows";

/// What the user must do so nomouse can read the accessibility tree on this OS.
pub const ACCESSIBILITY_REQUIREMENT: &str = "UI Automation is always on; run nomouse at the same integrity level as the apps it should reach (elevated apps need an elevated nomouse).";
