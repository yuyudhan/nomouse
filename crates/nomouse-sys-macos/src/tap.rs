//! A Quartz event tap (`CGEventTapCreate`) that catches global hotkeys and, while a hint
//! session runs, swallows every key press and queues it for nomouse.
//!
//! The tap is an active (filtering) session tap, which macOS allows only for processes trusted
//! for Accessibility. It lives on the run loop of the thread that created it; events arrive
//! while that thread runs its run loop (see [`crate::app::pump`]). When macOS disables the tap
//! because a callback was slow, or because of secure input, the callback re-enables it.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr::{self, NonNull};

use objc2_core_foundation::{
    CFMachPort, CFRetained, CFRunLoop, CFRunLoopSource, kCFRunLoopCommonModes,
};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventMask, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType,
};

use crate::input::modifier_flags;
use crate::types::{Chord, ModifierFlags, TapEvent};

/// The event tap could not be created: the process is not trusted for Accessibility (or Input
/// Monitoring is denied), or there is no run loop on this thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TapUnavailable;

impl std::fmt::Display for TapUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "cannot create the keyboard event tap; grant Accessibility (and Input Monitoring) access",
        )
    }
}

impl std::error::Error for TapUnavailable {}

#[derive(Default)]
struct TapState {
    chords: Vec<(u32, Chord)>,
    grab: bool,
    queue: VecDeque<TapEvent>,
}

/// What the callback reaches through `user_info`. The port and the re-enable counter live
/// outside the `RefCell`, so a disable event re-enables the tap even while the state is
/// borrowed (for example when the event arrives re-entrantly during `register`).
#[derive(Default)]
struct Shared {
    state: RefCell<TapState>,
    port: OnceCell<CFRetained<CFMachPort>>,
    reenabled: Cell<u32>,
}

/// Whether `kind` is macOS telling the tap it was switched off.
fn is_disable(kind: CGEventType) -> bool {
    kind == CGEventType::TapDisabledByTimeout || kind == CGEventType::TapDisabledByUserInput
}

/// Re-enables `port` (when set) with `enable` and counts it. Needs no borrow of the state.
fn reenable<P>(port: Option<&P>, counter: &Cell<u32>, enable: impl FnOnce(&P)) {
    if let Some(port) = port {
        enable(port);
        counter.set(counter.get().saturating_add(1));
    }
}

/// What the callback decided for one event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Pass,
    Swallow,
}

impl TapState {
    fn key_down(
        &mut self,
        keycode: u16,
        character: Option<char>,
        modifiers: ModifierFlags,
    ) -> Verdict {
        let chord = Chord { keycode, modifiers };
        if let Some(&(id, _)) = self
            .chords
            .iter()
            .find(|(_, registered)| *registered == chord)
        {
            self.queue.push_back(TapEvent::Hotkey(id));
            return Verdict::Swallow;
        }
        if self.grab {
            self.queue.push_back(TapEvent::Key {
                keycode,
                character,
                modifiers,
            });
            return Verdict::Swallow;
        }
        Verdict::Pass
    }
}

/// An installed event tap. Dropping it removes the tap.
pub struct EventTap {
    // Boxed so the address handed to the callback stays fixed; dropped after the tap is gone.
    shared: Box<Shared>,
    port: CFRetained<CFMachPort>,
    source: CFRetained<CFRunLoopSource>,
    run_loop: CFRetained<CFRunLoop>,
}

const fn bit(kind: CGEventType) -> CGEventMask {
    1 << kind.0
}

impl EventTap {
    /// Installs the tap on the current thread's run loop.
    ///
    /// # Errors
    /// [`TapUnavailable`] when macOS refuses the tap (missing permission).
    pub fn new() -> Result<Self, TapUnavailable> {
        let shared = Box::new(Shared::default());
        let user_info: *mut c_void = ptr::from_ref::<Shared>(&shared).cast_mut().cast();
        let mask = bit(CGEventType::KeyDown) | bit(CGEventType::KeyUp);
        // SAFETY: `callback` matches CGEventTapCallBack and only dereferences `user_info` as the
        // `Shared` it is; the box outlives the tap because `Drop` disables and
        // invalidates the port before the box is freed.
        let port = unsafe {
            CGEvent::tap_create(
                CGEventTapLocation::SessionEventTap,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                mask,
                Some(callback),
                user_info,
            )
        }
        .ok_or(TapUnavailable)?;
        let source = CFMachPort::new_run_loop_source(None, Some(&port), 0).ok_or(TapUnavailable)?;
        let run_loop = CFRunLoop::current().ok_or(TapUnavailable)?;
        // SAFETY: kCFRunLoopCommonModes is an immutable CFString constant.
        let modes = unsafe { kCFRunLoopCommonModes };
        run_loop.add_source(Some(&source), modes);
        // The cell is fresh, so this cannot fail.
        shared.port.set(port.clone()).ok();
        CGEvent::tap_enable(&port, true);
        Ok(Self {
            shared,
            port,
            source,
            run_loop,
        })
    }

    /// Starts matching `chord`; key presses of it are swallowed and queued as
    /// [`TapEvent::Hotkey`] with `id`. Re-registering an id replaces its chord.
    pub fn register(&self, id: u32, chord: Chord) {
        let mut state = self.shared.state.borrow_mut();
        state.chords.retain(|(existing, _)| *existing != id);
        state.chords.push((id, chord));
    }

    /// Stops matching the chord registered under `id`; unknown ids are ignored.
    pub fn unregister(&self, id: u32) {
        self.shared
            .state
            .borrow_mut()
            .chords
            .retain(|(existing, _)| *existing != id);
    }

    /// Whether every other key press is swallowed and queued as [`TapEvent::Key`].
    pub fn set_grab(&self, grab: bool) {
        self.shared.state.borrow_mut().grab = grab;
    }

    /// The next queued event, if any.
    pub fn pop(&self) -> Option<TapEvent> {
        self.shared.state.borrow_mut().queue.pop_front()
    }

    /// Whether the tap is currently enabled.
    pub fn is_enabled(&self) -> bool {
        CGEvent::tap_is_enabled(&self.port)
    }

    /// How many times the callback re-enabled the tap after macOS disabled it.
    pub fn reenable_count(&self) -> u32 {
        self.shared.reenabled.get()
    }
}

impl Drop for EventTap {
    fn drop(&mut self) {
        CGEvent::tap_enable(&self.port, false);
        // SAFETY: kCFRunLoopCommonModes is an immutable CFString constant.
        let modes = unsafe { kCFRunLoopCommonModes };
        self.run_loop.remove_source(Some(&self.source), modes);
        self.source.invalidate();
        self.port.invalidate();
        self.shared.port.take();
    }
}

fn unicode_char(event: &CGEvent) -> Option<char> {
    let mut buffer = [0_u16; 4];
    let mut length: std::ffi::c_ulong = 0;
    // SAFETY: both pointers refer to live locals; the buffer holds `buffer.len()` UTF-16 units.
    unsafe {
        CGEvent::keyboard_get_unicode_string(
            Some(event),
            buffer.len() as std::ffi::c_ulong,
            &raw mut length,
            buffer.as_mut_ptr(),
        );
    }
    let length = usize::try_from(length).ok()?.min(buffer.len());
    let mut chars = char::decode_utf16(buffer[..length].iter().copied());
    match (chars.next(), chars.next()) {
        (Some(Ok(c)), None) => Some(c),
        _ => None,
    }
}

/// The tap callback.
///
/// # Safety
/// Called by Quartz with `user_info` as registered in [`EventTap::new`] and a valid event.
unsafe extern "C-unwind" fn callback(
    _proxy: CGEventTapProxy,
    kind: CGEventType,
    event: NonNull<CGEvent>,
    user_info: *mut c_void,
) -> *mut CGEvent {
    let passed = event.as_ptr();
    // SAFETY: `user_info` is the address of the boxed `Shared` that outlives the tap.
    let Some(shared) = (unsafe { user_info.cast::<Shared>().as_ref() }) else {
        return passed;
    };
    if is_disable(kind) {
        reenable(shared.port.get(), &shared.reenabled, |port| {
            CGEvent::tap_enable(port, true);
        });
        return passed;
    }
    let Ok(mut state) = shared.state.try_borrow_mut() else {
        return passed;
    };
    // SAFETY: Quartz passes a valid event that lives for the duration of the callback.
    let event_ref = unsafe { event.as_ref() };
    let raw_keycode =
        CGEvent::integer_value_field(Some(event_ref), CGEventField::KeyboardEventKeycode);
    let Ok(keycode) = u16::try_from(raw_keycode) else {
        return passed;
    };
    let verdict = if kind == CGEventType::KeyDown {
        let modifiers = modifier_flags(CGEvent::flags(Some(event_ref)));
        state.key_down(keycode, unicode_char(event_ref), modifiers)
    } else if state.grab {
        Verdict::Swallow
    } else {
        Verdict::Pass
    };
    match verdict {
        Verdict::Pass => passed,
        Verdict::Swallow => ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disable_events_are_recognised() {
        assert!(is_disable(CGEventType::TapDisabledByTimeout));
        assert!(is_disable(CGEventType::TapDisabledByUserInput));
        assert!(!is_disable(CGEventType::KeyDown));
    }

    #[test]
    fn reenables_while_state_is_borrowed() {
        let shared = Shared::default();
        let _held = shared.state.borrow_mut();
        let port = 7_u8;
        let enabled = Cell::new(false);
        reenable(Some(&port), &shared.reenabled, |_| enabled.set(true));
        assert!(enabled.get());
        assert_eq!(shared.reenabled.get(), 1);
        reenable(None::<&u8>, &shared.reenabled, |_| enabled.set(false));
        assert!(enabled.get());
        assert_eq!(shared.reenabled.get(), 1);
    }
}
