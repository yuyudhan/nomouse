//! Linux backend for nomouse: AT-SPI elements; on X11 an overlay window, XTEST input and
//! XGrabKey hotkeys; on wlroots Wayland compositors (Sway, Hyprland) a layer-shell overlay,
//! virtual pointer/keyboard (or `/dev/uinput`) input and compositor-bound hotkeys.
//!
//! [`platform`] builds the backend for the current session; [`doctor`] reports what works.
//! The Wayland backend is the default `wayland` feature; without it Wayland sessions are
//! refused with a typed error.
//! OS-specific modules are compiled only on `target_os = "linux"`, so the crate checks on
//! every OS.
#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
mod a11y;
#[cfg(target_os = "linux")]
mod appearance;
#[cfg(target_os = "linux")]
mod first_run;
#[cfg(target_os = "linux")]
mod linux;
pub mod session;
#[cfg(all(target_os = "linux", feature = "wayland"))]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;

#[cfg(target_os = "linux")]
pub use first_run::{
    ACCESSIBILITY_SWITCH, first_run_dialogs, first_run_permissions, first_run_probe,
};
#[cfg(target_os = "linux")]
pub use linux::{LinuxPlatform, doctor, platform};
#[cfg(all(target_os = "linux", feature = "wayland"))]
pub use wayland::platform::WaylandPlatform;

/// The OS this backend serves.
pub const TARGET_OS: &str = "linux";

/// What the user must do so nomouse can read the accessibility tree on this OS.
pub const ACCESSIBILITY_REQUIREMENT: &str = "AT-SPI must be running: GNOME and KDE start it by default; on other desktops start `at-spi-bus-launcher` and set `GTK_MODULES=gail:atk-bridge`, `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1`, and run Chromium/Electron apps with `--force-renderer-accessibility`.";
