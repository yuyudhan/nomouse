//! Hotkey chords (`ctrl+alt+space`) and the key presses delivered while the overlay is active.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Modifier keys of a chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    /// Control.
    pub ctrl: bool,
    /// Alt / Option.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
    /// Super / Command / Windows key.
    pub meta: bool,
}

impl Modifiers {
    /// No modifier held.
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
    };

    /// Shift only.
    pub const SHIFT: Self = Self {
        shift: true,
        ..Self::NONE
    };

    /// Control only.
    pub const CTRL: Self = Self {
        ctrl: true,
        ..Self::NONE
    };

    /// Super / Command only.
    pub const META: Self = Self {
        meta: true,
        ..Self::NONE
    };

    /// Whether no modifier is held.
    pub const fn is_empty(self) -> bool {
        !(self.ctrl || self.alt || self.shift || self.meta)
    }

    /// The modifiers held in either `self` or `other`.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self {
            ctrl: self.ctrl || other.ctrl,
            alt: self.alt || other.alt,
            shift: self.shift || other.shift,
            meta: self.meta || other.meta,
        }
    }
}

/// The non-modifier key of a chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A lowercase ASCII letter or digit.
    Char(char),
    /// A function key, 1 to 24.
    F(u8),
    /// Space bar.
    Space,
    /// Tab.
    Tab,
    /// Enter / Return.
    Enter,
    /// Escape.
    Escape,
    /// Backspace.
    Backspace,
    /// `;`
    Semicolon,
    /// `,`
    Comma,
    /// `.`
    Period,
    /// `/`
    Slash,
}

impl Key {
    fn parse(name: &str) -> Option<Self> {
        let named = match name {
            "space" => Some(Self::Space),
            "tab" => Some(Self::Tab),
            "enter" | "return" => Some(Self::Enter),
            "escape" | "esc" => Some(Self::Escape),
            "backspace" => Some(Self::Backspace),
            "semicolon" | ";" => Some(Self::Semicolon),
            "comma" | "," => Some(Self::Comma),
            "period" | "." => Some(Self::Period),
            "slash" | "/" => Some(Self::Slash),
            _ => None,
        };
        if named.is_some() {
            return named;
        }
        let mut chars = name.chars();
        if let (Some(c), None) = (chars.next(), chars.clone().next()) {
            return (c.is_ascii_lowercase() || c.is_ascii_digit()).then_some(Self::Char(c));
        }
        let number = name.strip_prefix('f')?.parse::<u8>().ok()?;
        (1..=24).contains(&number).then_some(Self::F(number))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Char(c) => write!(f, "{c}"),
            Self::F(n) => write!(f, "f{n}"),
            Self::Space => f.write_str("space"),
            Self::Tab => f.write_str("tab"),
            Self::Enter => f.write_str("enter"),
            Self::Escape => f.write_str("escape"),
            Self::Backspace => f.write_str("backspace"),
            Self::Semicolon => f.write_str("semicolon"),
            Self::Comma => f.write_str("comma"),
            Self::Period => f.write_str("period"),
            Self::Slash => f.write_str("slash"),
        }
    }
}

/// A global hotkey chord such as `ctrl+alt+space`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hotkey {
    /// Held modifiers.
    pub modifiers: Modifiers,
    /// The key pressed with them.
    pub key: Key,
}

impl Hotkey {
    /// Creates a hotkey.
    pub const fn new(modifiers: Modifiers, key: Key) -> Self {
        Self { modifiers, key }
    }

    /// The chord as menus and settings show it: `Ctrl+Shift+Space`, `Ctrl+Alt+/`,
    /// `Super+F5` (modifiers in the order Ctrl, Alt, Shift, Super).
    pub fn human(&self) -> String {
        let m = self.modifiers;
        let mut text = String::new();
        for (on, name) in [
            (m.ctrl, "Ctrl+"),
            (m.alt, "Alt+"),
            (m.shift, "Shift+"),
            (m.meta, "Super+"),
        ] {
            if on {
                text.push_str(name);
            }
        }
        match self.key {
            Key::Char(c) => text.push(c.to_ascii_uppercase()),
            Key::F(n) => {
                text.push('F');
                text.push_str(&n.to_string());
            }
            Key::Space => text.push_str("Space"),
            Key::Tab => text.push_str("Tab"),
            Key::Enter => text.push_str("Enter"),
            Key::Escape => text.push_str("Esc"),
            Key::Backspace => text.push_str("Backspace"),
            Key::Semicolon => text.push(';'),
            Key::Comma => text.push(','),
            Key::Period => text.push('.'),
            Key::Slash => text.push('/'),
        }
        text
    }
}

/// Why a hotkey string failed to parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HotkeyError {
    /// The string was empty.
    #[error("empty hotkey")]
    Empty,
    /// A modifier appeared twice.
    #[error("modifier {0:?} repeated in hotkey")]
    DuplicateModifier(String),
    /// The last part is not a known key.
    #[error("unknown key {0:?} in hotkey")]
    UnknownKey(String),
    /// A part before the last is not a modifier.
    #[error("{0:?} is not a modifier (ctrl, alt, shift, super)")]
    NotAModifier(String),
    /// The chord has no modifier, which would steal a plain key globally.
    #[error("hotkey {0:?} needs at least one modifier")]
    NoModifier(String),
}

impl FromStr for Hotkey {
    type Err = HotkeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lowered = s.trim().to_ascii_lowercase();
        if lowered.is_empty() {
            return Err(HotkeyError::Empty);
        }
        let parts: Vec<&str> = lowered.split('+').map(str::trim).collect();
        let (key_name, modifier_names) = parts.split_last().ok_or(HotkeyError::Empty)?;
        let key =
            Key::parse(key_name).ok_or_else(|| HotkeyError::UnknownKey((*key_name).to_owned()))?;
        let mut modifiers = Modifiers::default();
        for name in modifier_names {
            let flag = match *name {
                "ctrl" | "control" => &mut modifiers.ctrl,
                "alt" | "option" => &mut modifiers.alt,
                "shift" => &mut modifiers.shift,
                "super" | "meta" | "cmd" | "command" | "win" => &mut modifiers.meta,
                other => return Err(HotkeyError::NotAModifier(other.to_owned())),
            };
            if *flag {
                return Err(HotkeyError::DuplicateModifier((*name).to_owned()));
            }
            *flag = true;
        }
        if modifiers == Modifiers::default() {
            return Err(HotkeyError::NoModifier(s.to_owned()));
        }
        Ok(Self { modifiers, key })
    }
}

impl TryFrom<String> for Hotkey {
    type Error = HotkeyError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<Hotkey> for String {
    fn from(value: Hotkey) -> Self {
        value.to_string()
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.modifiers;
        for (on, name) in [
            (m.ctrl, "ctrl"),
            (m.alt, "alt"),
            (m.shift, "shift"),
            (m.meta, "super"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

/// A key delivered while the overlay holds the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyPress {
    /// A printable character, as typed (backends lowercase letters; Shift is reported in
    /// [`KeyEvent::modifiers`]).
    Char(char),
    /// Backspace: undo the last typed character.
    Backspace,
    /// Escape: leave the current mode.
    Escape,
    /// Enter.
    Enter,
    /// Tab.
    Tab,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
}

impl KeyPress {
    /// This key pressed with `modifiers` held.
    pub const fn with(self, modifiers: Modifiers) -> KeyEvent {
        KeyEvent {
            key: self,
            modifiers,
        }
    }
}

/// A key press with the modifiers held at the time, as the overlay receives it.
///
/// Shift+Tab is `Tab` with `shift`, Ctrl+N is `Char('n')` with `ctrl`, `G` is `Char('g')` with
/// `shift`. Caps Lock is not a modifier: it changes nothing here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    /// The key.
    pub key: KeyPress,
    /// The modifiers held.
    pub modifiers: Modifiers,
}

impl From<KeyPress> for KeyEvent {
    fn from(key: KeyPress) -> Self {
        key.with(Modifiers::NONE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_displays() {
        let hotkey: Hotkey = "Ctrl + Alt + Space".parse().unwrap();
        assert!(hotkey.modifiers.ctrl && hotkey.modifiers.alt && !hotkey.modifiers.shift);
        assert_eq!(hotkey.key, Key::Space);
        assert_eq!(hotkey.to_string(), "ctrl+alt+space");
        assert_eq!("cmd+shift+f12".parse::<Hotkey>().unwrap().key, Key::F(12));
        assert_eq!("super+j".parse::<Hotkey>().unwrap().to_string(), "super+j");
    }

    #[test]
    fn human_names_follow_menu_conventions() {
        let human = |text: &str| text.parse::<Hotkey>().unwrap().human();
        assert_eq!(human("shift+ctrl+space"), "Ctrl+Shift+Space");
        assert_eq!(human("ctrl+alt+/"), "Ctrl+Alt+/");
        assert_eq!(human("cmd+f5"), "Super+F5");
        assert_eq!(human("alt+escape"), "Alt+Esc");
        assert_eq!(human("super+shift+j"), "Shift+Super+J");
    }

    #[test]
    fn rejects_bad_chords() {
        assert_eq!("".parse::<Hotkey>(), Err(HotkeyError::Empty));
        assert!(matches!(
            "space".parse::<Hotkey>(),
            Err(HotkeyError::NoModifier(_))
        ));
        assert!(matches!(
            "ctrl+ctrl+a".parse::<Hotkey>(),
            Err(HotkeyError::DuplicateModifier(_))
        ));
        assert!(matches!(
            "ctrl+f25".parse::<Hotkey>(),
            Err(HotkeyError::UnknownKey(_))
        ));
        assert!(matches!(
            "a+b".parse::<Hotkey>(),
            Err(HotkeyError::NotAModifier(_))
        ));
    }
}
