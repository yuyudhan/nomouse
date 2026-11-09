//! Plain data types shared by the wrappers and their callers.
//!
//! These compile on every OS so the backend's pure logic can build and test them anywhere.

/// A mouse button as Quartz numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    /// Primary button.
    Left,
    /// Secondary button.
    Right,
    /// Middle button (Quartz "center", button number 2).
    Middle,
}

/// Modifier keys, as carried in `CGEventFlags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ModifierFlags {
    /// Control.
    pub control: bool,
    /// Option.
    pub option: bool,
    /// Shift.
    pub shift: bool,
    /// Command.
    pub command: bool,
}

/// A global hotkey as the event tap matches it: a virtual key code plus exact modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    /// macOS virtual key code (`kVK_*`).
    pub keycode: u16,
    /// Modifiers that must be held, and no others.
    pub modifiers: ModifierFlags,
}

/// A keyboard layout: what each key types, so hotkeys can follow characters, not positions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyLayout {
    /// The input source id, such as `com.apple.keylayout.Dvorak`.
    pub id: String,
    /// `(virtual key code, character it types alone, character it types with Shift)`, in
    /// ascending key-code order; keys that type nothing printable are left out.
    pub keys: Vec<(u16, Option<char>, Option<char>)>,
}

/// A key-down event seen by the event tap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapEvent {
    /// A registered chord was pressed; the payload is the id given to `EventTap::register`.
    Hotkey(u32),
    /// A key went down while the keyboard was grabbed.
    Key {
        /// macOS virtual key code (`kVK_*`).
        keycode: u16,
        /// The character the key types under the current layout, if exactly one.
        character: Option<char>,
        /// Modifiers held.
        modifiers: ModifierFlags,
    },
}

/// A display, in Cocoa coordinates (origin bottom-left of the primary screen, y up, points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenFrame {
    /// Left edge.
    pub x: f64,
    /// Bottom edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// An RGB colour with alpha, each channel 0.0 to 1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    /// Red.
    pub red: f64,
    /// Green.
    pub green: f64,
    /// Blue.
    pub blue: f64,
    /// Alpha.
    pub alpha: f64,
}

/// The overlay's look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LabelStyle {
    /// Font size in points.
    pub font_size: f64,
    /// Label background.
    pub background: Rgba,
    /// Untyped label characters.
    pub foreground: Rgba,
    /// Already-typed label characters.
    pub typed_foreground: Rgba,
    /// Label border.
    pub border: Rgba,
}

/// One label to draw on the panel of screen `screen` (index into the screens the overlay was
/// given). (`x`, `y`) is where the label's top-left corner goes, in that panel's view
/// coordinates (origin bottom-left, y up, points); the overlay sizes the box to the text and
/// keeps it inside the panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    /// Index of the screen whose panel draws the label.
    pub screen: usize,
    /// Left edge of the label box.
    pub x: f64,
    /// Top edge of the label box.
    pub y: f64,
    /// The label text.
    pub text: String,
    /// How many leading characters were already typed (drawn in `typed_foreground`).
    pub typed: usize,
}
