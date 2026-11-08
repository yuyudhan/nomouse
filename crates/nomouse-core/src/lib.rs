//! Platform-independent core of nomouse.
//!
//! - [`element`]: the element model backends report.
//! - [`hints`] and [`filter`]: label generation and narrowing as keys are typed.
//! - [`action`]: pointer actions.
//! - [`hotkey`]: hotkey chords and overlay key presses.
//! - [`config`]: the TOML configuration.
//! - [`platform`]: the traits each OS backend implements.
//! - [`session`]: one hint-mode activation, tying the above together.
#![forbid(unsafe_code)]

pub mod action;
pub mod config;
pub mod element;
pub mod filter;
pub mod hints;
pub mod hotkey;
pub mod platform;
pub mod session;

pub use action::{Action, ActionKind, MouseButton};
pub use config::{Config, ConfigError};
pub use element::{Element, ElementId, Point, Rect, Role};
pub use filter::{FilterState, LabelFilter};
pub use hints::{Alphabet, AlphabetError, Hint, assign_hints, generate_labels};
pub use hotkey::{Hotkey, HotkeyError, Key, KeyPress, Modifiers};
pub use platform::{
    ElementSource, HotkeyId, HotkeySource, InputEvent, InputInjector, Overlay, Platform,
    PlatformError, Scope,
};
pub use session::{HintSession, SessionEvent};
