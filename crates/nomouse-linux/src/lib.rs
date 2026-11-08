//! Linux backend for nomouse: AT-SPI elements, X11 and wlroots Wayland overlays and input.
//!
//! The backend implements the [`nomouse_core::Platform`] traits; see `docs/architecture.md`.
//! OS-specific modules are compiled only on `target_os = "linux"`, so the crate checks on every OS.
#![forbid(unsafe_code)]

/// The OS this backend serves.
pub const TARGET_OS: &str = "linux";

/// What the user must do so nomouse can read the accessibility tree on this OS.
pub const ACCESSIBILITY_REQUIREMENT: &str = "AT-SPI must be running: GNOME and KDE start it by default; on other desktops start `at-spi-bus-launcher` and set `GTK_MODULES=gail:atk-bridge`, `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1`, and run Chromium/Electron apps with `--force-renderer-accessibility`.";
