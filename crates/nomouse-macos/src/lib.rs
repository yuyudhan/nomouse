//! macOS backend for nomouse: AXUIElement elements, NSPanel overlay, CGEvent input.
//!
//! The backend implements the [`nomouse_core::Platform`] traits; see `docs/architecture.md`.
//! OS-specific modules are compiled only on `target_os = "macos"`, so the crate checks on every OS.
#![forbid(unsafe_code)]

/// The OS this backend serves.
pub const TARGET_OS: &str = "macos";

/// What the user must do so nomouse can read the accessibility tree on this OS.
pub const ACCESSIBILITY_REQUIREMENT: &str =
    "Grant nomouse Accessibility access in System Settings > Privacy & Security > Accessibility.";
