//! Turning a key press in the hotkey recorder into a nomouse [`Hotkey`].
//!
//! The recorder translates the hardware keycode with no modifiers, so the key name here is the
//! unshifted key: Shift+/ arrives as `slash` with Shift held, never as `question`.

use nomouse_core::{Hotkey, Key, Modifiers};

/// What a key press means to the recorder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// A complete chord.
    Hotkey(Hotkey),
    /// Escape: stop recording and keep the old hotkey.
    Cancel,
    /// A modifier on its own: keep waiting for the key.
    Pending,
    /// A supported key without a modifier, which would steal that key everywhere.
    NeedsModifier(String),
    /// A key nomouse cannot use in a hotkey (the GDK key name).
    Unsupported(String),
}

impl Recorded {
    /// What to tell the user, for every outcome but a hotkey and waiting.
    pub fn message(&self) -> Option<String> {
        match self {
            Self::Hotkey(_) | Self::Pending => None,
            Self::Cancel => Some("Recording cancelled".to_owned()),
            Self::NeedsModifier(key) => Some(format!(
                "{key} needs a modifier: hold Ctrl, Alt, Shift or Super with it"
            )),
            Self::Unsupported(name) => Some(format!(
                "{name} cannot be a hotkey: use a letter, digit, F1-F24, Space, Tab, Enter, \
                 Backspace or ; , . /"
            )),
        }
    }
}

/// Whether `name` (a GDK key name) is a modifier or lock key on its own.
fn is_modifier(name: &str) -> bool {
    const PREFIXES: [&str; 8] = [
        "Shift_",
        "Control_",
        "Alt_",
        "Meta_",
        "Super_",
        "Hyper_",
        "ISO_Level",
        "ISO_Group",
    ];
    PREFIXES.iter().any(|prefix| name.starts_with(prefix))
        || matches!(
            name,
            "Caps_Lock" | "Num_Lock" | "Shift_Lock" | "Mode_switch"
        )
}

/// The nomouse key for a GDK key name (`a`, `7`, `F5`, `space`, `slash`, `Return`, …).
fn key(name: &str) -> Option<Key> {
    let named = match name {
        "space" => Key::Space,
        "Tab" | "ISO_Left_Tab" => Key::Tab,
        "Return" | "KP_Enter" | "ISO_Enter" => Key::Enter,
        "BackSpace" => Key::Backspace,
        "semicolon" => Key::Semicolon,
        "comma" => Key::Comma,
        "period" => Key::Period,
        "slash" => Key::Slash,
        _ => {
            let mut chars = name.chars();
            if let (Some(c), None) = (chars.next(), chars.next()) {
                let c = c.to_ascii_lowercase();
                return (c.is_ascii_lowercase() || c.is_ascii_digit()).then_some(Key::Char(c));
            }
            let number: u8 = name.strip_prefix('F')?.parse().ok()?;
            return (1..=24).contains(&number).then_some(Key::F(number));
        }
    };
    Some(named)
}

/// Maps a key press to what the recorder does with it: `name` is the GDK name of the key
/// translated without modifiers, `modifiers` the Ctrl, Alt, Shift and Super held.
pub fn hotkey_from_key(name: &str, modifiers: Modifiers) -> Recorded {
    if name == "Escape" {
        return Recorded::Cancel;
    }
    if is_modifier(name) {
        return Recorded::Pending;
    }
    match key(name) {
        None => Recorded::Unsupported(name.to_owned()),
        Some(key) if modifiers.is_empty() => {
            // Shown without the leading `+` that `human()` puts after modifiers.
            Recorded::NeedsModifier(Hotkey::new(Modifiers::NONE, key).human())
        }
        Some(key) => Recorded::Hotkey(Hotkey::new(modifiers, key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL_SHIFT: Modifiers = Modifiers {
        ctrl: true,
        shift: true,
        ..Modifiers::NONE
    };

    fn chord(name: &str, modifiers: Modifiers) -> String {
        match hotkey_from_key(name, modifiers) {
            Recorded::Hotkey(hotkey) => hotkey.to_string(),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn maps_every_supported_key() {
        assert_eq!(chord("a", Modifiers::CTRL), "ctrl+a");
        assert_eq!(chord("A", Modifiers::CTRL), "ctrl+a");
        assert_eq!(chord("7", Modifiers::META), "super+7");
        assert_eq!(chord("F5", Modifiers::CTRL), "ctrl+f5");
        assert_eq!(chord("F24", Modifiers::CTRL), "ctrl+f24");
        assert_eq!(chord("space", CTRL_SHIFT), "ctrl+shift+space");
        assert_eq!(chord("Tab", Modifiers::CTRL), "ctrl+tab");
        assert_eq!(chord("Return", Modifiers::CTRL), "ctrl+enter");
        assert_eq!(chord("KP_Enter", Modifiers::CTRL), "ctrl+enter");
        assert_eq!(chord("BackSpace", Modifiers::CTRL), "ctrl+backspace");
        assert_eq!(chord("semicolon", Modifiers::CTRL), "ctrl+semicolon");
        assert_eq!(chord("comma", Modifiers::CTRL), "ctrl+comma");
        assert_eq!(chord("period", Modifiers::CTRL), "ctrl+period");
        let all = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            meta: true,
        };
        assert_eq!(chord("b", all), "ctrl+alt+shift+super+b");
    }

    #[test]
    fn shift_slash_stays_slash() {
        // The recorder translates the keycode without modifiers, so Shift+/ is `slash`.
        assert_eq!(chord("slash", Modifiers::SHIFT), "shift+slash");
        assert_eq!(
            hotkey_from_key("question", Modifiers::SHIFT),
            Recorded::Unsupported("question".to_owned())
        );
    }

    #[test]
    fn escape_cancels_and_modifiers_wait() {
        assert_eq!(hotkey_from_key("Escape", Modifiers::NONE), Recorded::Cancel);
        assert_eq!(hotkey_from_key("Escape", Modifiers::CTRL), Recorded::Cancel);
        for name in [
            "Shift_L",
            "Control_R",
            "Alt_L",
            "Super_L",
            "ISO_Level3_Shift",
            "Caps_Lock",
        ] {
            assert_eq!(
                hotkey_from_key(name, Modifiers::CTRL),
                Recorded::Pending,
                "{name}"
            );
        }
        assert_eq!(
            hotkey_from_key("Return", Modifiers::NONE).message(),
            Some("Enter needs a modifier: hold Ctrl, Alt, Shift or Super with it".to_owned())
        );
    }

    #[test]
    fn refuses_plain_and_unknown_keys() {
        assert_eq!(
            hotkey_from_key("a", Modifiers::NONE),
            Recorded::NeedsModifier("A".to_owned())
        );
        for name in ["F25", "F0", "Left", "Home", "minus", "Delete", "é"] {
            assert_eq!(
                hotkey_from_key(name, Modifiers::CTRL),
                Recorded::Unsupported(name.to_owned()),
                "{name}"
            );
        }
        assert!(
            hotkey_from_key("Left", Modifiers::CTRL)
                .message()
                .is_some_and(|text| text.starts_with("Left cannot"))
        );
    }
}
