//! macOS virtual key codes (`kVK_*` from Carbon's `Events.h`) for hotkeys and typed keys.
//!
//! A hotkey on a character key matches the key that types that character on the current
//! layout (`ctrl+alt+/` on Dvorak is the key labelled `/`, not the ANSI `/` position). When
//! no key types it alone (digits on AZERTY, every letter on a non-Latin layout) the hotkey
//! falls back to the ANSI position, as macOS's own shortcuts do. Typed hint characters come
//! from the layout-translated character, so labels type as shown on any layout.

use nomouse_core::{Hotkey, Key, KeyPress, Modifiers};
use nomouse_sys_macos::{Chord, KeyLayout, ModifierFlags};

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
/// Key codes of the numeric keypad (`kVK_ANSI_KeypadDecimal`..`kVK_ANSI_Keypad9`), which type
/// the same digits and operators on every layout and are never what a hotkey means.
const KEYPAD: std::ops::RangeInclusive<u16> = 65..=92;

/// The ANSI virtual key code of `key`, or `None` when macOS has no such key (F21 to F24).
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

/// The character a character key types, or `None` for keys that sit at a fixed position on
/// every layout (space, tab, enter, escape, backspace, function keys).
const fn typed_character(key: Key) -> Option<char> {
    match key {
        Key::Char(c) => Some(c),
        Key::Semicolon => Some(';'),
        Key::Comma => Some(','),
        Key::Slash => Some('/'),
        Key::Period => Some('.'),
        Key::F(_) | Key::Space | Key::Tab | Key::Enter | Key::Escape | Key::Backspace => None,
    }
}

/// The virtual key code of `key` on `layout`: the lowest key code outside the keypad that
/// types the key's character without Shift, else the ANSI position ([`keycode`]).
pub fn keycode_on(key: Key, layout: Option<&KeyLayout>) -> Option<u16> {
    let typed = typed_character(key).and_then(|wanted| {
        layout?
            .keys
            .iter()
            .find(|(code, alone, _)| *alone == Some(wanted) && !KEYPAD.contains(code))
            .map(|(code, _, _)| *code)
    });
    typed.or_else(|| keycode(key))
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

/// The event-tap chord for a hotkey on `layout` (`None`: ANSI positions), or `None` when its
/// key does not exist on macOS.
pub fn chord(hotkey: Hotkey, layout: Option<&KeyLayout>) -> Option<Chord> {
    Some(Chord {
        keycode: keycode_on(hotkey.key, layout)?,
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
        let chord = chord(Hotkey::new(modifiers, Key::Space), None).unwrap();
        assert_eq!(chord.keycode, 49);
        assert!(chord.modifiers.control && chord.modifiers.command);
        assert!(!chord.modifiers.option && !chord.modifiers.shift);
    }

    /// A layout table captured from macOS with `UCKeyTranslate` (see `testdata/layouts/`).
    fn captured(name: &str, table: &str) -> KeyLayout {
        let character = |hex: &str| {
            (hex != "-").then(|| {
                char::from_u32(u32::from_str_radix(hex, 16).expect("hex code point"))
                    .expect("a character")
            })
        };
        let keys = table
            .lines()
            .filter(|line| !line.starts_with('#'))
            .map(|line| {
                let fields: Vec<&str> = line.split('\t').collect();
                let [code, alone, shifted] = fields[..] else {
                    panic!("{name}: bad line {line:?}");
                };
                (
                    code.parse().expect("key code"),
                    character(alone),
                    character(shifted),
                )
            })
            .collect();
        KeyLayout {
            id: format!("com.apple.keylayout.{name}"),
            keys,
        }
    }

    fn layouts() -> [KeyLayout; 5] {
        [
            captured("US", include_str!("../testdata/layouts/us.tsv")),
            captured("Dvorak", include_str!("../testdata/layouts/dvorak.tsv")),
            captured("Colemak", include_str!("../testdata/layouts/colemak.tsv")),
            captured("French", include_str!("../testdata/layouts/azerty.tsv")),
            captured("German", include_str!("../testdata/layouts/german.tsv")),
        ]
    }

    fn on(layout: &KeyLayout, key: Key) -> Option<u16> {
        keycode_on(key, Some(layout))
    }

    #[test]
    fn the_us_layout_matches_ansi_positions() {
        let [us, ..] = layouts();
        let keys = ('a'..='z').chain('0'..='9').map(Key::Char).chain([
            Key::Semicolon,
            Key::Comma,
            Key::Slash,
            Key::Period,
        ]);
        for key in keys {
            assert_eq!(on(&us, key), keycode(key), "{key}");
        }
    }

    #[test]
    fn hotkeys_follow_the_character_on_each_layout() {
        let [_, dvorak, colemak, azerty, german] = layouts();
        // Dvorak: `s` is on the ANSI `;` key, `/` on the ANSI `[` key, `o` on the ANSI `s` key.
        assert_eq!(on(&dvorak, Key::Char('s')), Some(41));
        assert_eq!(on(&dvorak, Key::Slash), Some(33));
        assert_eq!(on(&dvorak, Key::Char('o')), Some(1));
        assert_eq!(on(&dvorak, Key::Semicolon), Some(6));
        // Colemak: `f` is on the ANSI `e` key, `o` on the ANSI `;` key.
        assert_eq!(on(&colemak, Key::Char('f')), Some(14));
        assert_eq!(on(&colemak, Key::Char('o')), Some(41));
        // AZERTY: `a` and `q` swap, `z` and `w` swap, `m` is on the ANSI `;` key.
        assert_eq!(on(&azerty, Key::Char('a')), Some(12));
        assert_eq!(on(&azerty, Key::Char('q')), Some(0));
        assert_eq!(on(&azerty, Key::Char('z')), Some(13));
        assert_eq!(on(&azerty, Key::Char('m')), Some(41));
        // German: `y` and `z` swap.
        assert_eq!(on(&german, Key::Char('z')), Some(16));
        assert_eq!(on(&german, Key::Char('y')), Some(6));
    }

    #[test]
    fn every_letter_hotkey_is_on_the_key_that_types_it() {
        for layout in layouts() {
            for letter in 'a'..='z' {
                let code = on(&layout, Key::Char(letter));
                let typed = layout
                    .keys
                    .iter()
                    .find(|(key, _, _)| Some(*key) == code)
                    .and_then(|(_, alone, _)| *alone);
                assert_eq!(typed, Some(letter), "{} {letter}", layout.id);
            }
        }
    }

    #[test]
    fn characters_no_key_types_alone_fall_back_to_ansi_positions() {
        let [.., azerty, _] = layouts();
        // AZERTY types digits with Shift only: `1` stays on the ANSI `1` key, as in macOS.
        assert_eq!(on(&azerty, Key::Char('1')), Some(18));
        assert_eq!(on(&azerty, Key::Char('0')), Some(29));
        // A non-Latin layout types no ASCII letter: every letter stays on its ANSI key.
        let cyrillic = KeyLayout {
            id: "com.apple.keylayout.Russian".to_owned(),
            keys: vec![(0, Some('ф'), Some('Ф')), (1, Some('ы'), Some('Ы'))],
        };
        assert_eq!(on(&cyrillic, Key::Char('a')), Some(0));
        assert_eq!(on(&cyrillic, Key::Char('s')), Some(1));
        // Keys at fixed positions never move.
        assert_eq!(on(&azerty, Key::Space), Some(49));
        assert_eq!(on(&azerty, Key::F(5)), Some(96));
    }

    #[test]
    fn click_modifiers_map_to_event_flags() {
        let cmd_shift = Modifiers {
            meta: true,
            shift: true,
            ..Modifiers::NONE
        };
        let action = nomouse_core::Action::click(nomouse_core::Point::new(1, 1));
        let flags = modifier_flags(action.with_modifiers(cmd_shift).modifiers());
        assert_eq!(
            flags,
            ModifierFlags {
                control: false,
                option: false,
                shift: true,
                command: true,
            }
        );
        assert_eq!(modifier_flags(action.modifiers()), ModifierFlags::default());
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
