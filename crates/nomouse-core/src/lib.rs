//! Platform-independent core of nomouse.
//!
//! - [`element`]: the element model backends report.
//! - [`hints`] and [`filter`]: label generation and narrowing as keys are typed.
//! - [`action`]: pointer actions.
//! - [`hotkey`]: hotkey chords and overlay key presses.
//! - [`config`]: the TOML configuration.
//! - [`platform`]: the traits each OS backend implements.
//! - [`session`]: one hint-mode activation, tying the above together.
//! - [`search`]: fuzzy matching of a typed query against element names (search-by-text).
//! - [`setup`]: the guided first run that walks the user through missing permissions.
//! - `contract` (feature `testing`): the backend contract suite and an in-memory fake.
#![forbid(unsafe_code)]

pub mod action;
pub mod config;
#[cfg(any(test, feature = "testing"))]
pub mod contract;
pub mod element;
pub mod filter;
#[cfg(feature = "glyph")]
pub mod glyph;
pub mod hints;
pub mod hotkey;
pub mod platform;
pub mod search;
pub mod session;
pub mod setup;

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
