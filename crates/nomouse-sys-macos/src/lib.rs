//! Safe wrappers over the Accessibility (AXUIElement), CGEvent and AppKit FFI that nomouse needs on `target_os = "macos"`.
//!
//! This is one of the two crates where `unsafe` is permitted (see `docs/architecture.md`,
//! "Unsafe policy"). Rules: every public item is safe to call; every `unsafe` block carries a
//! `// SAFETY:` comment (enforced by `clippy::undocumented_unsafe_blocks`); unsafe functions
//! use explicit inner `unsafe` blocks (`unsafe_op_in_unsafe_fn`).
//!
//! [`error`] and [`types`] are plain Rust and build on every OS; the wrappers build only on
//! macOS.
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod error;
pub mod types;

#[cfg(target_os = "macos")]
pub mod app;
#[cfg(target_os = "macos")]
pub mod ax;
#[cfg(target_os = "macos")]
pub mod input;
#[cfg(target_os = "macos")]
pub mod overlay;
#[cfg(target_os = "macos")]
pub mod tap;

pub use error::AxError;
pub use types::{Button, Chord, Label, LabelStyle, ModifierFlags, Rgba, ScreenFrame, TapEvent};
