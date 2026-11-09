//! The current keyboard layout, from Text Input Sources (`TISCopyCurrentKeyboardLayoutInputSource`)
//! and `UCKeyTranslate`, so hotkeys can match the key that types a character on the user's
//! layout (Dvorak, AZERTY, ...) rather than its ANSI position.

use std::ffi::c_void;
use std::ptr::{self, NonNull};

use objc2_core_foundation::{CFData, CFRetained, CFString, CFType};

use crate::types::KeyLayout;

/// `kUCKeyActionDown`.
const KEY_ACTION_DOWN: u16 = 0;
/// `kUCKeyTranslateNoDeadKeysMask`: a dead key yields its own character, not a pending accent.
const NO_DEAD_KEYS: u32 = 1;
/// `shiftKey >> 8`, the Shift bit of `UCKeyTranslate`'s modifier state.
const SHIFT: u32 = 0x02;
/// Virtual key codes above this are not keys of a keyboard.
const KEY_CODES: u16 = 128;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *mut c_void;
    fn TISGetInputSourceProperty(source: *mut c_void, key: *const CFString) -> *mut c_void;
    static kTISPropertyUnicodeKeyLayoutData: *const CFString;
    static kTISPropertyInputSourceID: *const CFString;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const c_void,
        key_code: u16,
        key_action: u16,
        modifier_key_state: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: usize,
        actual_length: *mut usize,
        string: *mut u16,
    ) -> i32;
}

/// The current keyboard layout input source, retained.
fn current_source() -> Option<CFRetained<CFType>> {
    // SAFETY: no arguments; returns a +1 reference (the Copy rule) or null.
    let raw = unsafe { TISCopyCurrentKeyboardLayoutInputSource() };
    let raw = NonNull::new(raw.cast::<CFType>())?;
    // SAFETY: `raw` is a valid CF object we own one reference to, released by `CFRetained`.
    Some(unsafe { CFRetained::from_raw(raw) })
}

/// A property of `source` (the Get rule: valid while `source` is retained), or null.
fn property(source: &CFType, key: *const CFString) -> *mut c_void {
    // SAFETY: `source` is a live input source and `key` one of the TIS property constants.
    unsafe { TISGetInputSourceProperty(ptr::from_ref(source).cast_mut().cast(), key) }
}

/// The id of `source`, such as `com.apple.keylayout.US`.
fn source_id(source: &CFType) -> Option<String> {
    // SAFETY: reading an immutable framework constant.
    let key = unsafe { kTISPropertyInputSourceID };
    // SAFETY: the id property is a CFString owned by `source`, alive for this borrow.
    let id = unsafe { property(source, key).cast::<CFString>().as_ref() }?;
    Some(id.to_string())
}

/// The id of the current keyboard layout, cheap enough to poll for layout changes.
pub fn current_id() -> Option<String> {
    let source = current_source()?;
    source_id(&source)
}

/// The character `key_code` types with `modifiers` on the `UCKeyboardLayout` at `layout`, when
/// it is exactly one printable character.
fn translate(layout: *const c_void, key_code: u16, modifiers: u32, keyboard: u32) -> Option<char> {
    let mut dead_keys = 0_u32;
    let mut length = 0_usize;
    let mut buffer = [0_u16; 4];
    // SAFETY: `layout` points at the `uchr` data of an input source the caller keeps
    // retained; every out-pointer refers to a live local, and `buffer` holds `buffer.len()`
    // UTF-16 units.
    let status = unsafe {
        UCKeyTranslate(
            layout,
            key_code,
            KEY_ACTION_DOWN,
            modifiers,
            keyboard,
            NO_DEAD_KEYS,
            &raw mut dead_keys,
            buffer.len(),
            &raw mut length,
            buffer.as_mut_ptr(),
        )
    };
    if status != 0 || length != 1 {
        return None;
    }
    char::from_u32(u32::from(buffer[0])).filter(|c| !c.is_control())
}

/// What every key of the current keyboard layout types alone and with Shift. `None` when the
/// current input source has no key layout data (some input methods), in which case hotkeys
/// stay on ANSI positions.
pub fn current() -> Option<KeyLayout> {
    let source = current_source()?;
    let id = source_id(&source).unwrap_or_default();
    // SAFETY: reading an immutable framework constant.
    let key = unsafe { kTISPropertyUnicodeKeyLayoutData };
    // SAFETY: the layout data property is a CFData owned by `source`, alive for this borrow.
    let data = unsafe { property(&source, key).cast::<CFData>().as_ref() }?;
    let layout = data.byte_ptr().cast::<c_void>();
    if layout.is_null() {
        return None;
    }
    // SAFETY: no arguments; reads the keyboard type of the session.
    let keyboard = u32::from(unsafe { LMGetKbdType() });
    let keys = (0..KEY_CODES)
        .filter_map(|code| {
            let alone = translate(layout, code, 0, keyboard);
            let shifted = translate(layout, code, SHIFT, keyboard);
            (alone.is_some() || shifted.is_some()).then_some((code, alone, shifted))
        })
        .collect();
    Some(KeyLayout { id, keys })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_layout_matches_its_id_and_lists_keys_in_order() {
        let Some(layout) = current() else {
            return;
        };
        assert_eq!(current_id().as_deref(), Some(layout.id.as_str()));
        assert!(layout.keys.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }
}
