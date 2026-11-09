//! Keyboard logic: hotkey chords to keysyms and modifier masks, and key events to key presses,
//! honouring the server's keyboard layout (core keyboard mapping plus the XKB group in the
//! event state).

use nomouse_core::{Key, KeyPress, Modifiers};

/// X keysym values used here (from `X11/keysymdef.h`).
pub(crate) mod sym {
    pub(crate) const SPACE: u32 = 0x20;
    pub(crate) const TAB: u32 = 0xff09;
    /// What Shift+Tab produces on most layouts.
    pub(crate) const ISO_LEFT_TAB: u32 = 0xfe20;
    pub(crate) const RETURN: u32 = 0xff0d;
    pub(crate) const KP_ENTER: u32 = 0xff8d;
    pub(crate) const ESCAPE: u32 = 0xff1b;
    pub(crate) const BACKSPACE: u32 = 0xff08;
    pub(crate) const LEFT: u32 = 0xff51;
    pub(crate) const UP: u32 = 0xff52;
    pub(crate) const RIGHT: u32 = 0xff53;
    pub(crate) const DOWN: u32 = 0xff54;
    pub(crate) const F1: u32 = 0xffbe;
    pub(crate) const NUM_LOCK: u32 = 0xff7f;
    pub(crate) const SHIFT_L: u32 = 0xffe1;
    pub(crate) const CONTROL_L: u32 = 0xffe3;
    pub(crate) const ALT_L: u32 = 0xffe9;
    pub(crate) const SUPER_L: u32 = 0xffeb;
}

/// Core modifier mask bits.
pub(crate) mod mask {
    pub(crate) const SHIFT: u16 = 1;
    pub(crate) const LOCK: u16 = 1 << 1;
    pub(crate) const CONTROL: u16 = 1 << 2;
    pub(crate) const MOD1: u16 = 1 << 3;
    pub(crate) const MOD4: u16 = 1 << 6;
    /// Shift, Control, Mod1..Mod5: the bits that distinguish chords.
    pub(crate) const MODIFIERS: u16 = 0xff;
}

/// The keysym of a hotkey's key.
pub(crate) const fn hotkey_keysym(key: Key) -> u32 {
    match key {
        Key::Char(c) => c as u32,
        Key::F(n) => sym::F1 + n as u32 - 1,
        Key::Space => sym::SPACE,
        Key::Tab => sym::TAB,
        Key::Enter => sym::RETURN,
        Key::Escape => sym::ESCAPE,
        Key::Backspace => sym::BACKSPACE,
        Key::Semicolon => ';' as u32,
        Key::Comma => ',' as u32,
        Key::Period => '.' as u32,
        Key::Slash => '/' as u32,
    }
}

/// The modifier keycodes down in `keys` (a `QueryKeymap` bitmap: bit `code % 8` of byte
/// `code / 8`), from the modifier mapping `mapping` (`per` keycodes for each of the eight
/// modifiers, zero for an empty slot). Lock and the Num Lock modifier (`numlock`, a mask) are
/// left out: they are toggles, not keys held for a moment.
pub(crate) fn pressed_modifiers(mapping: &[u8], per: usize, keys: &[u8], numlock: u16) -> Vec<u8> {
    let mut pressed = Vec::new();
    for (index, codes) in mapping.chunks(per.max(1)).enumerate() {
        let bit = u32::try_from(index)
            .ok()
            .and_then(|i| 1_u16.checked_shl(i))
            .unwrap_or(0);
        if bit == mask::LOCK || bit == numlock {
            continue;
        }
        for &code in codes {
            let down = keys
                .get(usize::from(code / 8))
                .is_some_and(|byte| byte & (1 << (code % 8)) != 0);
            if code != 0 && down && !pressed.contains(&code) {
                pressed.push(code);
            }
        }
    }
    pressed
}

/// The core modifier mask of a chord (Alt is Mod1, Super is Mod4 on every common layout).
pub(crate) const fn modifier_mask(modifiers: Modifiers) -> u16 {
    let mut value = 0;
    if modifiers.shift {
        value |= mask::SHIFT;
    }
    if modifiers.ctrl {
        value |= mask::CONTROL;
    }
    if modifiers.alt {
        value |= mask::MOD1;
    }
    if modifiers.meta {
        value |= mask::MOD4;
    }
    value
}

/// The masks to grab so the chord works with Caps Lock and Num Lock in any state.
pub(crate) fn grab_variants(base: u16, numlock: u16) -> Vec<u16> {
    let mut variants = vec![base, base | mask::LOCK];
    if numlock != 0 && numlock != mask::LOCK {
        variants.extend([base | numlock, base | numlock | mask::LOCK]);
    }
    variants
}

/// Whether an event `state` is the chord `base`, ignoring the lock modifiers.
pub(crate) const fn chord_matches(state: u16, base: u16, numlock: u16) -> bool {
    state & mask::MODIFIERS & !(mask::LOCK | numlock) == base
}

/// The keysym → character rule of the X protocol: Latin-1 directly, Unicode keysyms with
/// the `0x0100_0000` offset.
pub(crate) fn keysym_char(keysym: u32) -> Option<char> {
    match keysym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(keysym),
        0x0100_0100..=0x0110_ffff => char::from_u32(keysym - 0x0100_0000),
        _ => None,
    }
}

/// The overlay key press for a keysym; `None` for modifiers and unmapped keys.
pub(crate) fn key_press(keysym: u32) -> Option<KeyPress> {
    let press = match keysym {
        sym::ESCAPE => KeyPress::Escape,
        sym::BACKSPACE => KeyPress::Backspace,
        sym::RETURN | sym::KP_ENTER => KeyPress::Enter,
        sym::TAB | sym::ISO_LEFT_TAB => KeyPress::Tab,
        sym::UP => KeyPress::Up,
        sym::DOWN => KeyPress::Down,
        sym::LEFT => KeyPress::Left,
        sym::RIGHT => KeyPress::Right,
        other => KeyPress::Char(keysym_char(other)?.to_lowercase().next()?),
    };
    Some(press)
}

/// The modifiers held in an event `state` (Caps Lock and Num Lock are not modifiers here).
pub(crate) const fn modifiers(state: u16) -> Modifiers {
    Modifiers {
        ctrl: state & mask::CONTROL != 0,
        alt: state & mask::MOD1 != 0,
        shift: state & mask::SHIFT != 0,
        meta: state & mask::MOD4 != 0,
    }
}

/// The server's keycode → keysym table (`GetKeyboardMapping`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct KeyMap {
    min_keycode: u8,
    per_keycode: usize,
    keysyms: Vec<u32>,
}

impl KeyMap {
    /// Wraps a `GetKeyboardMapping` reply starting at `min_keycode`.
    pub(crate) fn new(min_keycode: u8, per_keycode: u8, keysyms: Vec<u32>) -> Self {
        Self {
            min_keycode,
            per_keycode: usize::from(per_keycode),
            keysyms,
        }
    }

    fn row(&self, keycode: u8) -> &[u32] {
        if self.per_keycode == 0 || keycode < self.min_keycode {
            return &[];
        }
        let start = usize::from(keycode - self.min_keycode) * self.per_keycode;
        self.keysyms
            .get(start..start + self.per_keycode)
            .unwrap_or(&[])
    }

    /// Every keycode producing `keysym` without a shift level (level 1 of any group), then
    /// those producing it on another level.
    pub(crate) fn keycodes(&self, keysym: u32) -> Vec<u8> {
        let count = self.keysyms.len() / self.per_keycode.max(1);
        let codes = (0..count).filter_map(|i| {
            u8::try_from(i)
                .ok()
                .and_then(|i| self.min_keycode.checked_add(i))
        });
        let (base, shifted): (Vec<u8>, Vec<u8>) = codes
            .filter(|code| self.row(*code).contains(&keysym))
            .partition(|code| {
                self.row(*code)
                    .iter()
                    .enumerate()
                    .any(|(i, s)| *s == keysym && i % 2 == 0)
            });
        base.into_iter().chain(shifted).collect()
    }

    /// The keysym a key event produces: XKB group from state bits 13-14, shift level from
    /// Shift (inverted by Caps Lock for letters).
    pub(crate) fn keysym(&self, keycode: u8, state: u16) -> u32 {
        let row = self.row(keycode);
        let group = usize::from((state >> 13) & 0b11);
        let column = |group: usize, level: usize| {
            row.get(group * 2 + level)
                .copied()
                .filter(|keysym| *keysym != 0)
        };
        let lower = column(group, 0).or_else(|| column(0, 0)).unwrap_or(0);
        let upper = column(group, 1).or_else(|| {
            keysym_char(lower)
                .filter(|c| c.is_alphabetic())
                .and_then(|c| c.to_uppercase().next())
                .map(u32::from)
        });
        let is_letter = keysym_char(lower).is_some_and(char::is_alphabetic);
        let shift = state & mask::SHIFT != 0;
        let lock = state & mask::LOCK != 0 && is_letter;
        if shift == lock {
            lower
        } else {
            upper.unwrap_or(lower)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// keycode 10: a/A, Cyrillic ф/Ф in group 2; 11: space; 12: Escape; 13: `Num_Lock`.
    fn map() -> KeyMap {
        KeyMap::new(
            10,
            4,
            vec![
                u32::from('a'),
                u32::from('A'),
                0x0100_0444,
                0x0100_0424,
                sym::SPACE,
                0,
                0,
                0,
                sym::ESCAPE,
                0,
                0,
                0,
                sym::NUM_LOCK,
                0,
                0,
                0,
            ],
        )
    }

    #[test]
    fn keysyms_follow_shift_lock_and_group() {
        let map = map();
        assert_eq!(map.keysym(10, 0), u32::from('a'));
        assert_eq!(map.keysym(10, mask::SHIFT), u32::from('A'));
        assert_eq!(map.keysym(10, mask::LOCK), u32::from('A'));
        assert_eq!(map.keysym(10, mask::LOCK | mask::SHIFT), u32::from('a'));
        assert_eq!(map.keysym(10, 1 << 13), 0x0100_0444);
        assert_eq!(map.keysym(11, mask::LOCK), sym::SPACE);
        assert_eq!(map.keysym(200, 0), 0);
    }

    #[test]
    fn keycodes_for_keysym() {
        let map = map();
        assert_eq!(map.keycodes(u32::from('a')), vec![10]);
        assert_eq!(map.keycodes(sym::ESCAPE), vec![12]);
        assert_eq!(map.keycodes(sym::F1), Vec::<u8>::new());
    }

    #[test]
    fn key_presses() {
        assert_eq!(key_press(u32::from('S')), Some(KeyPress::Char('s')));
        assert_eq!(key_press(sym::ESCAPE), Some(KeyPress::Escape));
        assert_eq!(key_press(sym::KP_ENTER), Some(KeyPress::Enter));
        assert_eq!(key_press(0x0100_0444), Some(KeyPress::Char('ф')));
        assert_eq!(key_press(sym::SHIFT_L), None);
        assert_eq!(key_press(sym::ISO_LEFT_TAB), Some(KeyPress::Tab));
    }

    #[test]
    fn event_state_gives_held_modifiers_but_not_locks() {
        let state = mask::SHIFT | mask::CONTROL | mask::LOCK | 0x10;
        assert_eq!(
            modifiers(state),
            Modifiers {
                ctrl: true,
                shift: true,
                ..Modifiers::NONE
            }
        );
        let alt_super = modifiers(mask::MOD1 | mask::MOD4);
        assert!(alt_super.alt && alt_super.meta && !alt_super.ctrl);
        assert_eq!(modifiers(mask::LOCK), Modifiers::NONE);
    }

    #[test]
    fn pressed_modifiers_skip_locks_empty_slots_and_released_keys() {
        // Two keycodes per modifier: Shift 50/62, Lock 66, Control 37, Mod1 64, Mod2 (Num
        // Lock) 77, Mod4 133; Mod3 and Mod5 empty.
        let mapping = [50, 62, 66, 0, 37, 0, 64, 0, 77, 0, 0, 0, 133, 0, 0, 0];
        let mut keys = [0_u8; 32];
        for code in [62_u8, 66, 77, 133, 10] {
            keys[usize::from(code / 8)] |= 1 << (code % 8);
        }
        assert_eq!(pressed_modifiers(&mapping, 2, &keys, 0x10), [62, 133]);
        assert_eq!(
            pressed_modifiers(&mapping, 2, &[0; 32], 0x10),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn chords_and_lock_variants() {
        let ctrl_shift = Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        };
        let base = modifier_mask(ctrl_shift);
        assert_eq!(base, mask::CONTROL | mask::SHIFT);
        assert_eq!(grab_variants(base, 0x10).len(), 4);
        assert_eq!(grab_variants(base, 0).len(), 2);
        assert!(chord_matches(base | mask::LOCK | 0x10, base, 0x10));
        assert!(!chord_matches(mask::CONTROL, base, 0x10));
        assert_eq!(hotkey_keysym(Key::F(12)), sym::F1 + 11);
        assert_eq!(hotkey_keysym(Key::Char('j')), u32::from('j'));
    }
}
