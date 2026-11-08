//! Virtual-key codes: hotkey chords to `RegisterHotKey` arguments, hook key-downs to
//! [`KeyPress`].

use nomouse_core::{Hotkey, Key, KeyPress, Modifiers};
use nomouse_sys_windows::HotkeyModifiers;

/// `VK_BACK`.
const VK_BACK: u32 = 0x08;
/// `VK_TAB`.
const VK_TAB: u32 = 0x09;
/// `VK_RETURN`.
const VK_RETURN: u32 = 0x0D;
/// `VK_ESCAPE`.
const VK_ESCAPE: u32 = 0x1B;
/// `VK_SPACE`.
const VK_SPACE: u32 = 0x20;
/// `VK_LEFT`.
const VK_LEFT: u32 = 0x25;
/// `VK_UP`.
const VK_UP: u32 = 0x26;
/// `VK_RIGHT`.
const VK_RIGHT: u32 = 0x27;
/// `VK_DOWN`.
const VK_DOWN: u32 = 0x28;
/// `VK_F1`; F1 to F24 are consecutive.
const VK_F1: u32 = 0x70;
/// `VK_OEM_1`, `;` on US layouts.
const VK_OEM_1: u32 = 0xBA;
/// `VK_OEM_COMMA`.
const VK_OEM_COMMA: u32 = 0xBC;
/// `VK_OEM_PERIOD`.
const VK_OEM_PERIOD: u32 = 0xBE;
/// `VK_OEM_2`, `/` on US layouts.
const VK_OEM_2: u32 = 0xBF;

/// `VK_SHIFT`.
pub(crate) const VK_SHIFT: u16 = 0x10;
/// `VK_CONTROL`.
pub(crate) const VK_CONTROL: u16 = 0x11;
/// `VK_MENU` (Alt).
pub(crate) const VK_MENU: u16 = 0x12;
/// `VK_LWIN`.
pub(crate) const VK_LWIN: u16 = 0x5B;

/// The virtual-key code of a chord key.
pub(crate) fn virtual_key(key: Key) -> u32 {
    match key {
        Key::Char(c) => u32::from(c.to_ascii_uppercase()),
        Key::F(n) => VK_F1 + u32::from(n.saturating_sub(1)),
        Key::Space => VK_SPACE,
        Key::Tab => VK_TAB,
        Key::Enter => VK_RETURN,
        Key::Escape => VK_ESCAPE,
        Key::Backspace => VK_BACK,
        Key::Semicolon => VK_OEM_1,
        Key::Comma => VK_OEM_COMMA,
        Key::Period => VK_OEM_PERIOD,
        Key::Slash => VK_OEM_2,
    }
}

/// `RegisterHotKey` modifiers and key for a chord.
pub(crate) fn hotkey_args(hotkey: Hotkey) -> (HotkeyModifiers, u32) {
    let Modifiers {
        ctrl,
        alt,
        shift,
        meta,
    } = hotkey.modifiers;
    (
        HotkeyModifiers {
            ctrl,
            alt,
            shift,
            win: meta,
        },
        virtual_key(hotkey.key),
    )
}

/// Modifier virtual keys to hold for a modifier click, in press order.
pub(crate) fn modifier_keys(modifiers: Modifiers) -> Vec<u16> {
    [
        (modifiers.ctrl, VK_CONTROL),
        (modifiers.alt, VK_MENU),
        (modifiers.shift, VK_SHIFT),
        (modifiers.meta, VK_LWIN),
    ]
    .into_iter()
    .filter_map(|(on, vk)| on.then_some(vk))
    .collect()
}

/// The overlay key press for a hook key-down, or `None` for keys the session ignores.
///
/// Letters are reported lowercase whatever Shift says; punctuation follows the US layout.
pub(crate) fn key_press(vk: u32, shift: bool) -> Option<KeyPress> {
    let press = match vk {
        VK_BACK => KeyPress::Backspace,
        VK_ESCAPE => KeyPress::Escape,
        VK_RETURN => KeyPress::Enter,
        VK_TAB => KeyPress::Tab,
        VK_UP => KeyPress::Up,
        VK_DOWN => KeyPress::Down,
        VK_LEFT => KeyPress::Left,
        VK_RIGHT => KeyPress::Right,
        VK_SPACE => KeyPress::Char(' '),
        VK_OEM_1 => KeyPress::Char(if shift { ':' } else { ';' }),
        VK_OEM_COMMA => KeyPress::Char(if shift { '<' } else { ',' }),
        VK_OEM_PERIOD => KeyPress::Char(if shift { '>' } else { '.' }),
        VK_OEM_2 => KeyPress::Char(if shift { '?' } else { '/' }),
        0x41..=0x5A => KeyPress::Char(char::from_u32(vk)?.to_ascii_lowercase()),
        0x30..=0x39 if !shift => KeyPress::Char(char::from_u32(vk)?),
        _ => return None,
    };
    Some(press)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_keys_map_to_virtual_keys() {
        assert_eq!(virtual_key(Key::Char('a')), 0x41);
        assert_eq!(virtual_key(Key::Char('7')), 0x37);
        assert_eq!(virtual_key(Key::F(1)), 0x70);
        assert_eq!(virtual_key(Key::F(24)), 0x87);
        assert_eq!(virtual_key(Key::Space), 0x20);
        assert_eq!(virtual_key(Key::Slash), 0xBF);
    }

    #[test]
    fn hotkey_args_carry_modifiers() {
        let hotkey: Hotkey = "ctrl+alt+space".parse().unwrap();
        let (modifiers, vk) = hotkey_args(hotkey);
        assert!(modifiers.ctrl && modifiers.alt && !modifiers.shift && !modifiers.win);
        assert_eq!(vk, 0x20);
    }

    #[test]
    fn key_presses() {
        assert_eq!(key_press(0x41, true), Some(KeyPress::Char('a')));
        assert_eq!(key_press(0x4B, false), Some(KeyPress::Char('k')));
        assert_eq!(key_press(0x31, false), Some(KeyPress::Char('1')));
        assert_eq!(key_press(0x31, true), None);
        assert_eq!(key_press(0x1B, false), Some(KeyPress::Escape));
        assert_eq!(key_press(0x08, false), Some(KeyPress::Backspace));
        assert_eq!(key_press(0x28, false), Some(KeyPress::Down));
        assert_eq!(key_press(0xBA, false), Some(KeyPress::Char(';')));
        assert_eq!(key_press(0x70, false), None);
    }

    #[test]
    fn modifier_order() {
        let all = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            meta: true,
        };
        assert_eq!(modifier_keys(all), vec![0x11, 0x12, 0x10, 0x5B]);
        assert_eq!(modifier_keys(Modifiers::default()), [] as [u16; 0]);
    }
}
