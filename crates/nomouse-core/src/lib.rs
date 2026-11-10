//! Platform-independent core of nomouse.
//!
//! - [`element`]: the element model backends report.
//! - [`hints`] and [`filter`]: label generation and narrowing as keys are typed.
//! - [`action`]: pointer actions.
//! - [`hotkey`]: hotkey chords and overlay key presses.
//! - [`config`]: the TOML configuration.
//! - [`flavour`]: the release or development build, and the names each claims.
//! - [`settings`]: the settings model over `config.toml`: edited, saved atomically and
//!   watched, plus the fields a settings window shows.
//! - [`platform`]: the traits each OS backend implements.
//! - [`session`]: one hint-mode activation, tying the above together.
//! - [`search`]: fuzzy matching of a typed query against element names (search-by-text).
//! - [`reveal`]: waiting for an element scrolled into view to come to rest.
//! - [`placement`]: where a hint label goes, centred on its element and kept on screen.
//! - [`setup`]: the guided first run that walks the user through missing permissions.
//! - [`scroll`]: scroll mode's areas and keys.
//! - [`apps`]: the focused window's application, for `ignore.apps`.
//! - [`update`]: the update check (is a newer release out).
//! - [`menu`]: the status menu (tray icon, menu-bar item) for the daemon's state.
//! - `contract` (feature `testing`): the backend contract suite and an in-memory fake.
#![forbid(unsafe_code)]

pub mod action;
pub mod apps;
pub mod config;
#[cfg(any(test, feature = "testing"))]
pub mod contract;
pub mod element;
pub mod filter;
pub mod flavour;
#[cfg(feature = "glyph")]
pub mod glyph;
pub mod hints;
pub mod hotkey;
pub mod menu;
pub mod placement;
pub mod platform;
pub mod reveal;
pub mod scroll;
pub mod search;
pub mod session;
pub mod settings;
pub mod setup;
pub mod update;

pub use action::{Action, ActionKind, MouseButton};
pub use apps::AppIdentity;
pub use config::{Config, ConfigError};
pub use element::{Element, ElementId, Point, Rect, Role};
pub use filter::{FilterState, LabelFilter};
pub use flavour::Flavour;
pub use hints::{Alphabet, AlphabetError, Hint, assign_hints, generate_labels};
pub use hotkey::{Hotkey, HotkeyError, Key, KeyEvent, KeyPress, Modifiers};
pub use platform::{
    ElementSource, Frame, HotkeyId, HotkeySource, InputEvent, InputInjector, Overlay, Platform,
    PlatformError, Scope,
};
pub use scroll::{ScrollEvent, ScrollSession};
pub use session::{HintSession, KeyBindings, SessionEvent};
pub use settings::{Settings, SettingsError};
