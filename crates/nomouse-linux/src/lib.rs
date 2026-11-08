//! Linux backend for nomouse: AT-SPI elements, an X11 overlay, XTEST input and XGrabKey
//! hotkeys.
//!
//! [`platform`] builds the backend for the current session; [`doctor`] reports what works.
//! Wayland sessions are detected and refused with a typed error until milestone M4.
//! OS-specific modules are compiled only on `target_os = "linux"`, so the crate checks on
//! every OS.
#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
mod a11y;
#[cfg(target_os = "linux")]
mod linux;
pub mod session;
#[cfg(target_os = "linux")]
mod x11;

#[cfg(target_os = "linux")]
pub use linux::{LinuxPlatform, doctor, platform};

/// The OS this backend serves.
pub const TARGET_OS: &str = "linux";

/// What the user must do so nomouse can read the accessibility tree on this OS.
pub const ACCESSIBILITY_REQUIREMENT: &str = "AT-SPI must be running: GNOME and KDE start it by default; on other desktops start `at-spi-bus-launcher` and set `GTK_MODULES=gail:atk-bridge`, `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1`, and run Chromium/Electron apps with `--force-renderer-accessibility`.";
