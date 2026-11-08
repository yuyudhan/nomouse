//! Safe wrappers over the UI Automation (COM), Win32 window, `SendInput`, hotkey, hook and
//! layered-window FFI that nomouse needs on `target_os = "windows"`.
//!
//! This is one of the two crates where `unsafe` is permitted (see `docs/architecture.md`,
//! "Unsafe policy"). Rules: every public item is safe to call; every `unsafe` block carries a
//! `// SAFETY:` comment (enforced by `clippy::undocumented_unsafe_blocks`); unsafe functions
//! use explicit inner `unsafe` blocks (`unsafe_op_in_unsafe_fn`).
//!
//! The plain data types in [`types`] compile on every OS so the backend's pure logic can be
//! tested anywhere; the FFI modules exist only on Windows. All FFI objects are thread-bound
//! (`!Send`) and must live on the thread that drives the event loop.
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod types;

#[cfg(windows)]
mod com;
#[cfg(windows)]
mod error;
#[cfg(windows)]
mod events;
#[cfg(windows)]
mod input;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod uia;
#[cfg(windows)]
mod window;

pub use types::*;

#[cfg(windows)]
pub use com::ComApartment;
#[cfg(windows)]
pub use events::EventPump;
#[cfg(windows)]
pub use input::send_input;
#[cfg(windows)]
pub use overlay::OverlayWindow;
#[cfg(windows)]
pub use uia::Automation;
#[cfg(windows)]
pub use window::{
    BASE_DPI, dpi_at, dpi_awareness, enable_per_monitor_dpi, find_descendant, find_window,
    foreground_window, is_elevated, is_shown, top_level_windows, virtual_screen, window_info,
};
