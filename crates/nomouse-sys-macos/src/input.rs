//! Synthetic pointer input through Quartz events (`CGEventCreateMouseEvent`, `CGEventPost`).
//!
//! Coordinates are global points with the origin at the top-left of the primary screen, the
//! same space Accessibility reports positions in.

use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton,
    CGScrollEventUnit,
};

use crate::types::{Button, ModifierFlags};

/// Quartz refused to create an event (out of memory or an invalid parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventCreationFailed;

impl std::fmt::Display for EventCreationFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Quartz could not create the synthetic event")
    }
}

impl std::error::Error for EventCreationFailed {}

/// What a mouse event does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventKind {
    /// The pointer moves with no button held.
    Move,
    /// `button` goes down.
    Down(Button),
    /// `button` goes up.
    Up(Button),
    /// The pointer moves while `button` is held.
    Drag(Button),
}

const fn raw_button(button: Button) -> CGMouseButton {
    match button {
        Button::Left => CGMouseButton::Left,
        Button::Right => CGMouseButton::Right,
        Button::Middle => CGMouseButton::Center,
    }
}

const fn raw_kind(kind: MouseEventKind) -> (CGEventType, CGMouseButton) {
    match kind {
        MouseEventKind::Move => (CGEventType::MouseMoved, CGMouseButton::Left),
        MouseEventKind::Down(button) => (
            match button {
                Button::Left => CGEventType::LeftMouseDown,
                Button::Right => CGEventType::RightMouseDown,
                Button::Middle => CGEventType::OtherMouseDown,
            },
            raw_button(button),
        ),
        MouseEventKind::Up(button) => (
            match button {
                Button::Left => CGEventType::LeftMouseUp,
                Button::Right => CGEventType::RightMouseUp,
                Button::Middle => CGEventType::OtherMouseUp,
            },
            raw_button(button),
        ),
        MouseEventKind::Drag(button) => (
            match button {
                Button::Left => CGEventType::LeftMouseDragged,
                Button::Right => CGEventType::RightMouseDragged,
                Button::Middle => CGEventType::OtherMouseDragged,
            },
            raw_button(button),
        ),
    }
}

/// Converts modifier keys to `CGEventFlags`.
pub fn event_flags(modifiers: ModifierFlags) -> CGEventFlags {
    let mut flags = CGEventFlags::empty();
    flags.set(CGEventFlags::MaskControl, modifiers.control);
    flags.set(CGEventFlags::MaskAlternate, modifiers.option);
    flags.set(CGEventFlags::MaskShift, modifiers.shift);
    flags.set(CGEventFlags::MaskCommand, modifiers.command);
    flags
}

/// Reads modifier keys from `CGEventFlags`.
pub fn modifier_flags(flags: CGEventFlags) -> ModifierFlags {
    ModifierFlags {
        control: flags.contains(CGEventFlags::MaskControl),
        option: flags.contains(CGEventFlags::MaskAlternate),
        shift: flags.contains(CGEventFlags::MaskShift),
        command: flags.contains(CGEventFlags::MaskCommand),
    }
}

fn post(event: &CFRetained<CGEvent>) {
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(event));
}

/// Posts one mouse event at (`x`, `y`).
///
/// `click_state` is the click count the event belongs to (1 for a single click, 2 for the
/// second click of a double click); it is ignored for moves.
///
/// # Errors
/// [`EventCreationFailed`] when Quartz cannot create the event.
pub fn post_mouse(
    kind: MouseEventKind,
    x: f64,
    y: f64,
    click_state: i64,
    modifiers: ModifierFlags,
) -> Result<(), EventCreationFailed> {
    let (event_type, button) = raw_kind(kind);
    let event = CGEvent::new_mouse_event(None, event_type, CGPoint::new(x, y), button)
        .ok_or(EventCreationFailed)?;
    if !matches!(kind, MouseEventKind::Move) {
        CGEvent::set_integer_value_field(
            Some(&event),
            CGEventField::MouseEventClickState,
            click_state,
        );
    }
    CGEvent::set_flags(Some(&event), event_flags(modifiers));
    post(&event);
    Ok(())
}

/// Posts a line-based scroll-wheel event at the current pointer position. Positive
/// `vertical` scrolls content up (the wheel turned away from the user), positive `horizontal`
/// scrolls content left, as Quartz defines them.
///
/// # Errors
/// [`EventCreationFailed`] when Quartz cannot create the event.
pub fn post_scroll(vertical: i32, horizontal: i32) -> Result<(), EventCreationFailed> {
    let event =
        CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Line, 2, vertical, horizontal, 0)
            .ok_or(EventCreationFailed)?;
    post(&event);
    Ok(())
}

/// Whether macOS currently lets this process observe global keyboard events (the Input
/// Monitoring permission, `CGPreflightListenEventAccess`). Never prompts.
pub fn can_listen_events() -> bool {
    objc2_core_graphics::CGPreflightListenEventAccess()
}

/// Whether macOS currently lets this process post synthetic events
/// (`CGPreflightPostEventAccess`, granted with Accessibility). Never prompts.
pub fn can_post_events() -> bool {
    objc2_core_graphics::CGPreflightPostEventAccess()
}
