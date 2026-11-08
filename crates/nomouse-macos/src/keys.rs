//! macOS virtual key codes (`kVK_*` from Carbon's `Events.h`) for hotkeys and typed keys.
//!
//! Hotkeys match by key *position* on the ANSI layout, as macOS shortcuts do; typed hint
//! characters come from the layout-translated character, so labels work on any layout.

use nomouse_core::{Hotkey, Key, KeyPress, Modifiers};
use nomouse_sys_macos::{Chord, ModifierFlags};

const RETURN: u16 = 36;
const TAB: u16 = 48;
const SPACE: u16 = 49;
const DELETE: u16 = 51;
const ESCAPE: u16 = 53;
const KEYPAD_ENTER: u16 = 76;
const LEFT: u16 = 123;
const RIGHT: u16 = 124;
const DOWN: u16 = 125;
const UP: u16 = 126;

/// ANSI key codes for `a`..`z`.
const LETTERS: [u16; 26] = [
    0, 11, 8, 2, 14, 3, 5, 4, 34, 38, 40, 37, 46, 45, 31, 35, 12, 15, 1, 17, 32, 9, 13, 7, 16, 6,
];
/// ANSI key codes for `0`..`9`.
const DIGITS: [u16; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
/// Key codes for F1..F20 (macOS has no F21-F24 keys).
const FUNCTION: [u16; 20] = [
    122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106, 64, 79, 80, 90,
];

/// The virtual key code of `key`, or `None` when macOS has no such key (F21 to F24).
pub fn keycode(key: Key) -> Option<u16> {
    match key {
        Key::Char(c) if c.is_ascii_lowercase() => LETTERS
            .get(usize::from(u8::try_from(c).ok()? - b'a'))
            .copied(),
        Key::Char(c) => c
            .to_digit(10)
            .and_then(|digit| DIGITS.get(usize::try_from(digit).ok()?).copied()),
        Key::F(n) => FUNCTION.get(usize::from(n.checked_sub(1)?)).copied(),
        Key::Space => Some(SPACE),
        Key::Tab => Some(TAB),
        Key::Enter => Some(RETURN),
        Key::Escape => Some(ESCAPE),
        Key::Backspace => Some(DELETE),
        Key::Semicolon => Some(41),
        Key::Comma => Some(43),
        Key::Slash => Some(44),
        Key::Period => Some(47),
    }
}

/// Maps nomouse modifiers to macOS ones (`meta` is Command, `alt` is Option).
pub const fn modifier_flags(modifiers: Modifiers) -> ModifierFlags {
    ModifierFlags {
        control: modifiers.ctrl,
        option: modifiers.alt,
        shift: modifiers.shift,
        command: modifiers.meta,
    }
}

/// The event-tap chord for a hotkey, or `None` when its key does not exist on macOS.
pub fn chord(hotkey: Hotkey) -> Option<Chord> {
    Some(Chord {
        keycode: keycode(hotkey.key)?,
        modifiers: modifier_flags(hotkey.modifiers),
    })
}

/// Translates a key press seen during a session. Special keys map by key code; anything else
/// by the character it types (lowercased). `None` for keys with no meaning (modifier-only,
/// dead keys, function keys).
pub fn key_press(keycode: u16, character: Option<char>) -> Option<KeyPress> {
    match keycode {
        RETURN | KEYPAD_ENTER => Some(KeyPress::Enter),
        TAB => Some(KeyPress::Tab),
        DELETE => Some(KeyPress::Backspace),
        ESCAPE => Some(KeyPress::Escape),
        LEFT => Some(KeyPress::Left),
        RIGHT => Some(KeyPress::Right),
        DOWN => Some(KeyPress::Down),
        UP => Some(KeyPress::Up),
        _ => character
            .filter(|c| !c.is_control())
            .map(|c| KeyPress::Char(c.to_lowercase().next().unwrap_or(c))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_digits_and_named_keys() {
        assert_eq!(keycode(Key::Char('a')), Some(0));
        assert_eq!(keycode(Key::Char('s')), Some(1));
        assert_eq!(keycode(Key::Char('z')), Some(6));
        assert_eq!(keycode(Key::Char('0')), Some(29));
        assert_eq!(keycode(Key::Char('5')), Some(23));
        assert_eq!(keycode(Key::Space), Some(49));
        assert_eq!(keycode(Key::Slash), Some(44));
        assert_eq!(keycode(Key::F(1)), Some(122));
        assert_eq!(keycode(Key::F(20)), Some(90));
        assert_eq!(keycode(Key::F(21)), None);
        assert_eq!(keycode(Key::F(0)), None);
    }

    #[test]
    fn letter_codes_are_distinct() {
        let mut codes = LETTERS.to_vec();
        codes.extend(DIGITS);
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 36);
    }

    #[test]
    fn chords_map_meta_to_command() {
        let modifiers = Modifiers {
            ctrl: true,
            meta: true,
            ..Modifiers::default()
        };
        let chord = chord(Hotkey::new(modifiers, Key::Space)).unwrap();
        assert_eq!(chord.keycode, 49);
        assert!(chord.modifiers.control && chord.modifiers.command);
        assert!(!chord.modifiers.option && !chord.modifiers.shift);
    }

    #[test]
    fn session_keys() {
        assert_eq!(key_press(53, None), Some(KeyPress::Escape));
        assert_eq!(key_press(51, Some('\u{8}')), Some(KeyPress::Backspace));
        assert_eq!(key_press(76, None), Some(KeyPress::Enter));
        assert_eq!(key_press(126, None), Some(KeyPress::Up));
        assert_eq!(key_press(0, Some('A')), Some(KeyPress::Char('a')));
        assert_eq!(key_press(0, Some('\u{1}')), None);
        assert_eq!(key_press(56, None), None);
    }
}
