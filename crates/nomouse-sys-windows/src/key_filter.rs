//! What the session keyboard hook does with each key: pure logic, compiled and tested on
//! every OS.
//!
//! Modifier keys pass through so the system's modifier state stays consistent; every other key
//! is swallowed (key-downs are delivered to the session). Passing modifiers through has one
//! catch: a swallowed Alt+letter leaves the app seeing a lone Alt press and release, which
//! activates its menu bar (and a lone Win opens Start). So when a key was swallowed while Alt or
//! Win was held, the release of that modifier is preceded by an unassigned "mask" key (the
//! technique `AutoHotkey` uses): the app then sees Alt+mask, not a lone Alt.

/// The unassigned virtual key injected as the mask (`0xE8`).
pub const MASK_VK: u32 = 0xE8;

const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;
const VK_MENU: u32 = 0x12;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;

/// Alt or Win: modifiers whose lone press-and-release triggers something in the shell or app.
const fn is_menu_modifier(vk: u32) -> bool {
    matches!(vk, VK_MENU | VK_LMENU | VK_RMENU | VK_LWIN | VK_RWIN)
}

/// Whether `vk` is a modifier key, which the grab lets through.
pub const fn is_modifier(vk: u32) -> bool {
    is_menu_modifier(vk)
        || matches!(
            vk,
            VK_SHIFT | VK_LSHIFT | VK_RSHIFT | VK_CONTROL | VK_LCONTROL | VK_RCONTROL
        )
}

/// The hook's decision for one key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// Let the event through untouched.
    Pass,
    /// Inject the mask key, then let this modifier release through.
    MaskThenPass,
    /// Swallow the event; `deliver` is true for key-downs the session should receive.
    Swallow {
        /// Queue the key for the session.
        deliver: bool,
    },
}

/// The grab's state between key events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct KeyFilter {
    /// A key was swallowed while Alt or Win was held, and that modifier is still down.
    masked_modifier_pending: bool,
}

impl KeyFilter {
    /// A filter with nothing pending.
    pub const fn new() -> Self {
        Self {
            masked_modifier_pending: false,
        }
    }

    /// Decides what to do with a key event while grabbing. `menu_modifier_held` is whether Alt
    /// or Win is down at the time of the event; `injected` marks synthetic events.
    pub const fn on_key(
        &mut self,
        vk: u32,
        down: bool,
        menu_modifier_held: bool,
        injected: bool,
    ) -> Verdict {
        if injected && vk == MASK_VK {
            return Verdict::Pass;
        }
        if is_menu_modifier(vk) {
            if !down && self.masked_modifier_pending {
                self.masked_modifier_pending = false;
                return Verdict::MaskThenPass;
            }
            return Verdict::Pass;
        }
        if is_modifier(vk) {
            return Verdict::Pass;
        }
        if menu_modifier_held {
            self.masked_modifier_pending = true;
        }
        Verdict::Swallow { deliver: down }
    }

    /// Called when the grab ends: whether the mask must be sent now because Alt or Win is
    /// still held after a swallowed key (its release would otherwise reach the app alone).
    pub const fn on_release(&mut self, menu_modifier_held: bool) -> bool {
        let mask = self.masked_modifier_pending && menu_modifier_held;
        self.masked_modifier_pending = false;
        mask
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: u32 = 0x41;

    #[test]
    fn plain_keys_are_swallowed_and_modifiers_pass() {
        let mut filter = KeyFilter::default();
        assert_eq!(
            filter.on_key(KEY_A, true, false, false),
            Verdict::Swallow { deliver: true }
        );
        assert_eq!(
            filter.on_key(KEY_A, false, false, false),
            Verdict::Swallow { deliver: false }
        );
        assert_eq!(filter.on_key(VK_LSHIFT, true, false, false), Verdict::Pass);
        assert_eq!(filter.on_key(VK_LMENU, true, true, false), Verdict::Pass);
        assert_eq!(filter.on_key(VK_LMENU, false, false, false), Verdict::Pass);
    }

    #[test]
    fn alt_letter_masks_the_alt_release() {
        let mut filter = KeyFilter::default();
        assert_eq!(filter.on_key(VK_LMENU, true, true, false), Verdict::Pass);
        // Auto-repeated Alt downs do not clear the pending mask.
        assert_eq!(
            filter.on_key(KEY_A, true, true, false),
            Verdict::Swallow { deliver: true }
        );
        assert_eq!(filter.on_key(VK_LMENU, true, true, false), Verdict::Pass);
        assert_eq!(
            filter.on_key(KEY_A, false, true, false),
            Verdict::Swallow { deliver: false }
        );
        // The injected mask itself passes through the grab.
        assert_eq!(filter.on_key(MASK_VK, true, true, true), Verdict::Pass);
        assert_eq!(
            filter.on_key(VK_LMENU, false, true, false),
            Verdict::MaskThenPass
        );
        // Once masked, the next lone Alt is the user's own and passes untouched.
        assert_eq!(filter.on_key(VK_LMENU, true, true, false), Verdict::Pass);
        assert_eq!(filter.on_key(VK_LMENU, false, true, false), Verdict::Pass);
    }

    #[test]
    fn win_letter_masks_the_win_release() {
        let mut filter = KeyFilter::default();
        filter.on_key(VK_LWIN, true, true, false);
        filter.on_key(KEY_A, true, true, false);
        assert_eq!(
            filter.on_key(VK_RWIN, false, true, false),
            Verdict::MaskThenPass
        );
    }

    #[test]
    fn ending_the_grab_with_alt_held_masks_once() {
        let mut filter = KeyFilter::default();
        filter.on_key(KEY_A, true, true, false);
        assert!(filter.on_release(true));
        assert!(!filter.on_release(true));
        filter.on_key(KEY_A, true, true, false);
        // Alt already released: nothing to mask.
        assert!(!filter.on_release(false));
        // Ctrl+letter never needs a mask.
        filter.on_key(KEY_A, true, false, false);
        assert!(!filter.on_release(true));
    }
}
