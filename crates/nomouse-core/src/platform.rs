//! The platform traits every OS backend implements.
//!
//! A backend is split by responsibility so each part can be tested and replaced alone:
//! [`ElementSource`] reads the accessibility tree, [`Overlay`] draws hint labels and holds the
//! keyboard while visible, [`InputInjector`] performs pointer actions, and [`HotkeySource`]
//! registers global hotkeys and delivers input events to the binary's event loop.
//!
//! All methods are blocking and called from one thread (the event loop); backends that need
//! their own threads hide them. Coordinates are global screen pixels ([`crate::Point`]).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::element::Element;
use crate::hints::Hint;
use crate::hotkey::{Hotkey, KeyEvent};

/// Which part of the screen to collect elements from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// Only the focused window.
    #[default]
    FocusedWindow,
    /// Every visible window on every screen.
    Screen,
}

/// A failure reported by a backend.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    /// The OS accessibility service is off or unreachable; the message says how to enable it.
    #[error("accessibility is not available: {0}")]
    AccessibilityUnavailable(String),
    /// The OS denied a permission (macOS Accessibility, Wayland virtual pointer, ...).
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// No window has focus, so [`Scope::FocusedWindow`] has nothing to read.
    #[error("no focused window")]
    NoFocusedWindow,
    /// Another program already owns the hotkey.
    #[error("hotkey {0} is already taken")]
    HotkeyTaken(Hotkey),
    /// The backend cannot do this on the current session (for example, no layer-shell).
    #[error("unsupported on this session: {0}")]
    Unsupported(String),
    /// Any other backend error, with its message.
    #[error("{0}")]
    Backend(String),
}

/// Reads clickable elements from the accessibility tree.
pub trait ElementSource {
    /// Returns a snapshot of the elements in `scope`, in any order.
    ///
    /// The snapshot includes disabled and zero-sized elements; callers filter with
    /// [`Element::is_clickable`]. Element ids need only be unique within one snapshot.
    ///
    /// # Errors
    /// [`PlatformError::AccessibilityUnavailable`] when the tree cannot be read,
    /// [`PlatformError::NoFocusedWindow`] for [`Scope::FocusedWindow`] without focus.
    fn elements(&mut self, scope: Scope) -> Result<Vec<Element>, PlatformError>;
}

/// Everything one overlay frame draws, for [`Overlay::show_frame`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Frame<'a> {
    /// The hints still matching `typed`.
    pub hints: &'a [Hint],
    /// The label prefix typed so far: backends highlight it and may dim the rest.
    pub typed: &'a str,
    /// Search-by-text: the query typed so far, drawn in a search box (even when empty or
    /// when nothing matches); `None` outside search mode.
    pub query: Option<&'a str>,
    /// The label of the highlighted hint (focus-then-act), drawn in inverted colours.
    pub focused: Option<&'a str>,
}

/// Draws hint labels above all windows and captures the keyboard while shown.
pub trait Overlay {
    /// Shows `hints` (replacing any shown before) and grabs the keyboard, so key presses arrive
    /// as [`InputEvent::Key`] instead of reaching the focused app.
    ///
    /// `typed` is the prefix typed so far: backends highlight it and may dim the rest.
    /// The caller passes only the hints still matching `typed`.
    ///
    /// # Errors
    /// Fails when the overlay window or the keyboard grab cannot be created.
    fn show(&mut self, hints: &[Hint], typed: &str) -> Result<(), PlatformError>;

    /// Shows a whole [`Frame`]: its hints and typed prefix like [`Overlay::show`], plus the
    /// search box and the highlighted hint, and grabs the keyboard.
    ///
    /// The default shows only the labels; the Linux, macOS and Windows backends draw the
    /// search box, and the Linux backends the highlight.
    ///
    /// # Errors
    /// Fails when the overlay window or the keyboard grab cannot be created.
    fn show_frame(&mut self, frame: &Frame<'_>) -> Result<(), PlatformError> {
        self.show(frame.hints, frame.typed)
    }

    /// Hides the overlay and releases the keyboard. Calling it while hidden is a no-op.
    ///
    /// # Errors
    /// Fails only when the display connection is lost.
    fn hide(&mut self) -> Result<(), PlatformError>;
}

/// Performs synthetic pointer input.
pub trait InputInjector {
    /// Performs `action`, leaving the pointer at [`Action::end_point`].
    ///
    /// # Errors
    /// [`PlatformError::PermissionDenied`] when the OS blocks synthetic input.
    fn perform(&mut self, action: &Action) -> Result<(), PlatformError>;
}

/// Identifies a hotkey registered with [`HotkeySource::register`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HotkeyId(pub u32);

/// An input event for the event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// A registered global hotkey was pressed (also while the [`Overlay`] holds the keyboard,
    /// where pressing an activation hotkey again dismisses the labels).
    Hotkey(HotkeyId),
    /// A key was pressed while the [`Overlay`] held the keyboard.
    Key(KeyEvent),
}

/// Registers global hotkeys and delivers input events.
pub trait HotkeySource {
    /// Registers `hotkey` globally and returns its id; the caller keeps the mapping.
    ///
    /// # Errors
    /// [`PlatformError::HotkeyTaken`] when another program owns the chord.
    fn register(&mut self, hotkey: Hotkey) -> Result<HotkeyId, PlatformError>;

    /// Unregisters a hotkey. Unknown ids are a no-op.
    ///
    /// # Errors
    /// Fails only when the display connection is lost.
    fn unregister(&mut self, id: HotkeyId) -> Result<(), PlatformError>;

    /// Waits up to `timeout` for the next event; `Ok(None)` on timeout.
    ///
    /// # Errors
    /// Fails when the event connection is lost.
    fn next_event(&mut self, timeout: Duration) -> Result<Option<InputEvent>, PlatformError>;
}

/// A complete backend for one OS session.
pub trait Platform: ElementSource + Overlay + InputInjector + HotkeySource {
    /// A short backend name for logs and `nomouse doctor`, e.g. `"linux-x11"`.
    fn name(&self) -> &'static str;
}
