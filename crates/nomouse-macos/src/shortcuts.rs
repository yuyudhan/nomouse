//! Hotkeys as the menu bar app shows and records them: the key pressed in a shortcut recorder
//! turned into a [`Hotkey`], the `⌃⌥⇧⌘` spelling macOS uses, and the conflict check against
//! macOS's own keyboard shortcuts (System Settings > Keyboard > Keyboard Shortcuts), read from
//! `com.apple.symbolichotkeys`.

use std::fmt::Write as _;

use nomouse_core::{Hotkey, Key, Modifiers};

use crate::keys;

const RETURN: u16 = 36;
const TAB: u16 = 48;
const SPACE: u16 = 49;
const DELETE: u16 = 51;
const ESCAPE: u16 = 53;
const KEYPAD_ENTER: u16 = 76;

/// What a key press in a recording shortcut recorder means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// Escape alone: stop recording and keep the old shortcut.
    Cancel,
    /// The new shortcut.
    Hotkey(Hotkey),
    /// The press cannot be a hotkey; why, for the note below the recorder.
    Invalid(&'static str),
}

/// The key a recorder saw: function and editing keys by their fixed key code, character keys
/// by the character they type on the current layout (`characters`, without modifiers), else by
/// their ANSI position.
fn key_for(keycode: u16, characters: &str) -> Option<Key> {
    let fixed = match keycode {
        SPACE => Some(Key::Space),
        TAB => Some(Key::Tab),
        RETURN | KEYPAD_ENTER => Some(Key::Enter),
        DELETE => Some(Key::Backspace),
        ESCAPE => Some(Key::Escape),
        _ => (1..=20)
            .map(Key::F)
            .find(|key| keys::keycode(*key) == Some(keycode)),
    };
    if fixed.is_some() {
        return fixed;
    }
    let typed = characters
        .chars()
        .next()
        .map(|c| c.to_ascii_lowercase())
        .and_then(|c| match c {
            'a'..='z' | '0'..='9' => Some(Key::Char(c)),
            ';' => Some(Key::Semicolon),
            ',' => Some(Key::Comma),
            '.' => Some(Key::Period),
            '/' => Some(Key::Slash),
            _ => None,
        });
    typed.or_else(|| {
        ('a'..='z')
            .chain('0'..='9')
            .map(Key::Char)
            .chain([Key::Semicolon, Key::Comma, Key::Period, Key::Slash])
            .find(|key| keys::keycode(*key) == Some(keycode))
    })
}

/// Turns a key press in a shortcut recorder into a hotkey.
pub fn recorded(keycode: u16, characters: &str, modifiers: Modifiers) -> Recorded {
    if keycode == ESCAPE && modifiers.is_empty() {
        return Recorded::Cancel;
    }
    let Some(key) = key_for(keycode, characters) else {
        return Recorded::Invalid("That key cannot start nomouse; try a letter, digit or Space.");
    };
    if modifiers.is_empty() {
        return Recorded::Invalid("Hold Control, Option, Shift or Command with the key.");
    }
    Recorded::Hotkey(Hotkey::new(modifiers, key))
}

/// The hotkey as macOS menus write shortcuts: `⌃⌥⇧⌘` then the key (`⌃Space`, `⇧⌘/`).
pub fn symbols(hotkey: Hotkey) -> String {
    let m = hotkey.modifiers;
    let mut text = String::new();
    for (on, symbol) in [(m.ctrl, '⌃'), (m.alt, '⌥'), (m.shift, '⇧'), (m.meta, '⌘')] {
        if on {
            text.push(symbol);
        }
    }
    match hotkey.key {
        Key::Char(c) => text.push(c.to_ascii_uppercase()),
        Key::F(n) => {
            let _ = write!(text, "F{n}");
        }
        Key::Space => text.push_str("Space"),
        Key::Tab => text.push('⇥'),
        Key::Enter => text.push('↩'),
        Key::Escape => text.push('⎋'),
        Key::Backspace => text.push('⌫'),
        Key::Semicolon => text.push(';'),
        Key::Comma => text.push(','),
        Key::Period => text.push('.'),
        Key::Slash => text.push('/'),
    }
    text
}

/// The key equivalent AppKit draws for `key` at the right edge of a menu item: the lowercase
/// character, or the private-use character of a function or editing key.
pub fn menu_key(key: Key) -> String {
    match key {
        Key::Char(c) => c.to_string(),
        // NSF1FunctionKey is U+F704; F1 to F24 follow it.
        Key::F(n) => char::from_u32(0xF704 + u32::from(n.saturating_sub(1)))
            .map(String::from)
            .unwrap_or_default(),
        Key::Space => " ".to_owned(),
        Key::Tab => "\t".to_owned(),
        Key::Enter => "\r".to_owned(),
        Key::Escape => "\u{1b}".to_owned(),
        Key::Backspace => "\u{8}".to_owned(),
        Key::Semicolon => ";".to_owned(),
        Key::Comma => ",".to_owned(),
        Key::Period => ".".to_owned(),
        Key::Slash => "/".to_owned(),
    }
}

/// One of macOS's own keyboard shortcuts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemShortcut {
    /// Its title in System Settings.
    pub name: &'static str,
    /// The virtual key code.
    pub keycode: u16,
    /// The modifiers.
    pub modifiers: Modifiers,
}

/// `com.apple.symbolichotkeys` modifier mask bits (as `NSEvent` modifier flags).
const SHIFT_MASK: u64 = 1 << 17;
const CONTROL_MASK: u64 = 1 << 18;
const OPTION_MASK: u64 = 1 << 19;
const COMMAND_MASK: u64 = 1 << 20;

/// The modifiers in a `com.apple.symbolichotkeys` mask (other bits, such as Fn, are ignored).
const fn from_mask(mask: u64) -> Modifiers {
    Modifiers {
        ctrl: mask & CONTROL_MASK != 0,
        alt: mask & OPTION_MASK != 0,
        shift: mask & SHIFT_MASK != 0,
        meta: mask & COMMAND_MASK != 0,
    }
}

/// The shortcuts macOS turns on by default that a nomouse hotkey could collide with, by
/// their `com.apple.symbolichotkeys` id: (id, name, key code, modifiers).
const DEFAULTS: &[(u32, &str, u16, Modifiers)] = &[
    (
        7,
        "Move focus to the menu bar",
        120,
        from_mask(CONTROL_MASK),
    ),
    (8, "Move focus to the Dock", 99, from_mask(CONTROL_MASK)),
    (
        9,
        "Move focus to the active or next window",
        118,
        from_mask(CONTROL_MASK),
    ),
    (
        10,
        "Move focus to the window toolbar",
        96,
        from_mask(CONTROL_MASK),
    ),
    (
        11,
        "Move focus to the floating window",
        97,
        from_mask(CONTROL_MASK),
    ),
    (
        28,
        "Save picture of screen as a file",
        20,
        from_mask(SHIFT_MASK | COMMAND_MASK),
    ),
    (
        29,
        "Copy picture of screen to the clipboard",
        20,
        from_mask(CONTROL_MASK | SHIFT_MASK | COMMAND_MASK),
    ),
    (
        30,
        "Save picture of selected area as a file",
        21,
        from_mask(SHIFT_MASK | COMMAND_MASK),
    ),
    (
        31,
        "Copy picture of selected area to the clipboard",
        21,
        from_mask(CONTROL_MASK | SHIFT_MASK | COMMAND_MASK),
    ),
    (
        52,
        "Turn Dock hiding on/off",
        2,
        from_mask(OPTION_MASK | COMMAND_MASK),
    ),
    (
        60,
        "Select the previous input source",
        SPACE,
        from_mask(CONTROL_MASK),
    ),
    (
        61,
        "Select next source in Input menu",
        SPACE,
        from_mask(CONTROL_MASK | OPTION_MASK),
    ),
    (64, "Show Spotlight search", SPACE, from_mask(COMMAND_MASK)),
    (
        65,
        "Show Finder search window",
        SPACE,
        from_mask(OPTION_MASK | COMMAND_MASK),
    ),
    (
        98,
        "Show Help menu",
        44,
        from_mask(SHIFT_MASK | COMMAND_MASK),
    ),
    (
        184,
        "Screenshot and recording options",
        23,
        from_mask(SHIFT_MASK | COMMAND_MASK),
    ),
];

fn name_of(id: u32) -> &'static str {
    DEFAULTS
        .iter()
        .find(|(known, ..)| *known == id)
        .map_or("A macOS keyboard shortcut", |(_, name, ..)| name)
}

/// One entry of `AppleSymbolicHotKeys`: whether it is on, and its key code and modifiers
/// when it has a key (`parameters` = [character, key code, modifier mask]).
fn entry(value: &serde_json::Value) -> Option<(bool, Option<(u16, Modifiers)>)> {
    let enabled = match value.get("enabled")? {
        serde_json::Value::Bool(on) => *on,
        other => other.as_u64().is_some_and(|on| on != 0),
    };
    let key = value
        .get("value")
        .and_then(|value| value.get("parameters"))
        .and_then(serde_json::Value::as_array)
        .and_then(|parameters| {
            let keycode = u16::try_from(parameters.get(1)?.as_u64()?).ok()?;
            let mask = parameters.get(2)?.as_u64()?;
            // 65535 is "no key" (a shortcut that is only a modifier or unset).
            (keycode != u16::MAX).then_some((keycode, from_mask(mask)))
        });
    Some((enabled, key))
}

/// The macOS shortcuts in effect: the defaults, changed or switched off as `json` (the
/// `com.apple.symbolichotkeys` preferences as JSON, `None` when there are none) says.
pub fn system_shortcuts(json: Option<&str>) -> Vec<SystemShortcut> {
    let mut shortcuts: Vec<(u32, Option<SystemShortcut>)> = DEFAULTS
        .iter()
        .map(|(id, name, keycode, modifiers)| {
            (
                *id,
                Some(SystemShortcut {
                    name,
                    keycode: *keycode,
                    modifiers: *modifiers,
                }),
            )
        })
        .collect();
    let parsed = json.and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok());
    let entries = parsed
        .as_ref()
        .and_then(|root| root.get("AppleSymbolicHotKeys"))
        .and_then(serde_json::Value::as_object);
    for (id, value) in entries.into_iter().flatten() {
        let (Ok(id), Some((enabled, key))) = (id.parse::<u32>(), entry(value)) else {
            continue;
        };
        let shortcut = match (enabled, key) {
            (true, Some((keycode, modifiers))) => Some(SystemShortcut {
                name: name_of(id),
                keycode,
                modifiers,
            }),
            // Off, or on without a key of its own: the default no longer applies.
            (false, _) => None,
            (true, None) => DEFAULTS.iter().find(|(known, ..)| *known == id).map(
                |(_, name, keycode, modifiers)| SystemShortcut {
                    name,
                    keycode: *keycode,
                    modifiers: *modifiers,
                },
            ),
        };
        match shortcuts.iter_mut().find(|(known, _)| *known == id) {
            Some(slot) => slot.1 = shortcut,
            None => shortcuts.push((id, shortcut)),
        }
    }
    shortcuts
        .into_iter()
        .filter_map(|(_, shortcut)| shortcut)
        .collect()
}

/// The macOS shortcut `hotkey` collides with, if any (compared by ANSI key position).
pub fn conflict(hotkey: Hotkey, shortcuts: &[SystemShortcut]) -> Option<&SystemShortcut> {
    let keycode = keys::keycode(hotkey.key)?;
    shortcuts
        .iter()
        .find(|shortcut| shortcut.keycode == keycode && shortcut.modifiers == hotkey.modifiers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hotkey(text: &str) -> Hotkey {
        text.parse().unwrap()
    }

    #[test]
    fn a_chord_records_as_the_character_it_types() {
        // Dvorak's `/` key sits at the ANSI `[` position (33); the typed character wins.
        assert_eq!(
            recorded(33, "/", from_mask(CONTROL_MASK | OPTION_MASK)),
            Recorded::Hotkey(hotkey("ctrl+alt+slash"))
        );
        assert_eq!(
            recorded(0, "A", Modifiers::META),
            Recorded::Hotkey(hotkey("super+a"))
        );
        assert_eq!(
            recorded(SPACE, " ", from_mask(SHIFT_MASK | COMMAND_MASK)),
            Recorded::Hotkey(hotkey("shift+super+space"))
        );
    }

    #[test]
    fn function_keys_and_unknown_characters_fall_back_to_key_codes() {
        assert_eq!(
            recorded(96, "\u{F708}", Modifiers::CTRL),
            Recorded::Hotkey(hotkey("ctrl+f5"))
        );
        // Shift-1 types `!`; the ANSI position says 1.
        assert_eq!(
            recorded(18, "!", from_mask(CONTROL_MASK | SHIFT_MASK)),
            Recorded::Hotkey(hotkey("ctrl+shift+1"))
        );
    }

    #[test]
    fn escape_cancels_and_bare_or_unknown_keys_are_refused() {
        assert_eq!(
            recorded(ESCAPE, "\u{1b}", Modifiers::NONE),
            Recorded::Cancel
        );
        assert_eq!(
            recorded(ESCAPE, "\u{1b}", Modifiers::CTRL),
            Recorded::Hotkey(hotkey("ctrl+escape"))
        );
        assert!(matches!(
            recorded(0, "a", Modifiers::NONE),
            Recorded::Invalid(_)
        ));
        // The left arrow is no hotkey key.
        assert!(matches!(
            recorded(123, "\u{F702}", Modifiers::CTRL),
            Recorded::Invalid(_)
        ));
    }

    #[test]
    fn symbols_follow_the_macos_modifier_order() {
        assert_eq!(symbols(hotkey("super+shift+ctrl+alt+k")), "⌃⌥⇧⌘K");
        assert_eq!(symbols(hotkey("ctrl+space")), "⌃Space");
        assert_eq!(symbols(hotkey("alt+f12")), "⌥F12");
        assert_eq!(menu_key(Key::F(1)), "\u{F704}");
        assert_eq!(menu_key(Key::Char('j')), "j");
    }

    #[test]
    fn defaults_apply_without_preferences() {
        let shortcuts = system_shortcuts(None);
        let spotlight = conflict(hotkey("super+space"), &shortcuts).unwrap();
        assert_eq!(spotlight.name, "Show Spotlight search");
        assert!(conflict(hotkey("ctrl+alt+shift+space"), &shortcuts).is_none());
    }

    #[test]
    fn preferences_switch_defaults_off_and_rebind_them() {
        // Spotlight off; the previous input source moved to ctrl+alt+shift+space; an
        // unknown shortcut on cmd+alt+j.
        let json = r#"{"AppleSymbolicHotKeys": {
            "64": {"enabled": false, "value": {"parameters": [32, 49, 1048576], "type": "standard"}},
            "60": {"enabled": true, "value": {"parameters": [32, 49, 917504], "type": "standard"}},
            "250": {"enabled": 1, "value": {"parameters": [106, 38, 1572864], "type": "standard"}},
            "251": {"enabled": true, "value": {"parameters": [65535, 65535, 0], "type": "standard"}}
        }}"#;
        let shortcuts = system_shortcuts(Some(json));
        assert!(conflict(hotkey("super+space"), &shortcuts).is_none());
        assert!(conflict(hotkey("ctrl+space"), &shortcuts).is_none());
        assert_eq!(
            conflict(hotkey("ctrl+alt+shift+space"), &shortcuts).map(|found| found.name),
            Some("Select the previous input source")
        );
        assert_eq!(
            conflict(hotkey("alt+super+j"), &shortcuts).map(|found| found.name),
            Some("A macOS keyboard shortcut")
        );
    }

    #[test]
    fn unreadable_preferences_keep_the_defaults() {
        assert_eq!(system_shortcuts(Some("not json")), system_shortcuts(None));
    }
}
