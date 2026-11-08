//! Synthetic input on Wayland: the wlroots virtual pointer and virtual keyboard protocols
//! when the compositor offers them (Sway, Hyprland), else a `/dev/uinput` device (KDE, or any
//! compositor reading libinput devices).
//!
//! Both replay the XTEST plan of [`crate::x11::input::plan`]: buttons 1-3 are clicks, 4-7 wheel
//! steps, key codes are X keycodes (evdev + 8).

use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, EventType, InputEvent, KeyCode, RelativeAxisCode,
    UinputAbsSetup,
};
use nomouse_core::{Modifiers, PlatformError, Rect};
use wayland_client::Proxy as _;
use wayland_client::protocol::wl_pointer;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1;

use crate::x11::input::Fake;

/// evdev codes.
pub(crate) mod code {
    /// Left button.
    pub(crate) const BTN_LEFT: u16 = 0x110;
    /// Right button.
    pub(crate) const BTN_RIGHT: u16 = 0x111;
    /// Middle button.
    pub(crate) const BTN_MIDDLE: u16 = 0x112;
    /// Left Ctrl.
    pub(crate) const KEY_LEFTCTRL: u16 = 29;
    /// Left Shift.
    pub(crate) const KEY_LEFTSHIFT: u16 = 42;
    /// Left Alt.
    pub(crate) const KEY_LEFTALT: u16 = 56;
    /// Left Super.
    pub(crate) const KEY_LEFTMETA: u16 = 125;
}

/// X keycodes are evdev codes plus 8.
const X_KEYCODE_OFFSET: u16 = 8;

/// How long the compositor needs to pick up a new uinput device before it reacts to it.
const UINPUT_SETTLE: Duration = Duration::from_millis(400);

/// The X keycodes (evdev + 8) of the left modifier keys held for `modifiers`, in press order.
pub(crate) fn modifier_keycodes(modifiers: Modifiers) -> Vec<u8> {
    [
        (modifiers.ctrl, code::KEY_LEFTCTRL),
        (modifiers.shift, code::KEY_LEFTSHIFT),
        (modifiers.alt, code::KEY_LEFTALT),
        (modifiers.meta, code::KEY_LEFTMETA),
    ]
    .into_iter()
    .filter_map(|(on, key)| on.then_some(key))
    .filter_map(|key| u8::try_from(key + X_KEYCODE_OFFSET).ok())
    .collect()
}

/// The standard XKB modifier mask bit of a left modifier key (`Shift`, `Control`, `Mod1`,
/// `Mod4`), for the virtual keyboard's `modifiers` request.
pub(crate) const fn modifier_bit(evdev: u16) -> u32 {
    match evdev {
        code::KEY_LEFTSHIFT => 1,
        code::KEY_LEFTCTRL => 1 << 2,
        code::KEY_LEFTALT => 1 << 3,
        code::KEY_LEFTMETA => 1 << 6,
        _ => 0,
    }
}

/// What one plan step does, independent of the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// Absolute motion, relative to the layout's top-left corner.
    Motion(u32, u32),
    /// A button (evdev code) goes down or up.
    Button(u16, bool),
    /// Wheel steps: vertical (`true`) or horizontal, positive down/right.
    Wheel(bool, i32),
    /// A key (evdev code) goes down or up.
    Key(u16, bool),
    /// Wait so applications see distinct events.
    Pause(u64),
}

fn evdev_button(button: u8) -> Option<u16> {
    match button {
        1 => Some(code::BTN_LEFT),
        2 => Some(code::BTN_MIDDLE),
        3 => Some(code::BTN_RIGHT),
        _ => None,
    }
}

/// Translates one XTEST step into layout-relative terms; `None` drops it (wheel releases).
pub(crate) fn step(fake: Fake, layout: Rect) -> Option<Step> {
    let relative = |value: i32, origin: i32, span: u32| {
        let offset =
            (i64::from(value) - i64::from(origin)).clamp(0, i64::from(span.saturating_sub(1)));
        u32::try_from(offset).unwrap_or(0)
    };
    Some(match fake {
        Fake::Motion(at) => Step::Motion(
            relative(at.x, layout.x, layout.width),
            relative(at.y, layout.y, layout.height),
        ),
        Fake::Press(button) | Fake::Release(button) => {
            let down = matches!(fake, Fake::Press(_));
            match (evdev_button(button), button) {
                (Some(code), _) => Step::Button(code, down),
                (None, 4) if down => Step::Wheel(true, -1),
                (None, 5) if down => Step::Wheel(true, 1),
                (None, 6) if down => Step::Wheel(false, -1),
                (None, 7) if down => Step::Wheel(false, 1),
                _ => return None,
            }
        }
        Fake::KeyDown(keycode) => {
            Step::Key(u16::from(keycode).checked_sub(X_KEYCODE_OFFSET)?, true)
        }
        Fake::KeyUp(keycode) => Step::Key(u16::from(keycode).checked_sub(X_KEYCODE_OFFSET)?, false),
        Fake::Pause(millis) => Step::Pause(millis),
    })
}

/// The wlroots virtual pointer and, when offered, the virtual keyboard.
pub(crate) struct Virtual {
    /// The pointer.
    pub pointer: zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
    /// The keyboard for modifier keys; `None` without the protocol or the seat keymap.
    pub keyboard: Option<zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1>,
    /// Held modifier mask.
    pub modifiers: u32,
    /// Timestamps are milliseconds since this instant.
    pub epoch: Instant,
}

impl Virtual {
    fn time(&self) -> u32 {
        u32::try_from(self.epoch.elapsed().as_millis() % u128::from(u32::MAX)).unwrap_or(0)
    }

    /// Sends one step; pauses are the caller's (it must flush first).
    pub(crate) fn send(&mut self, step: Step, layout: Rect) {
        let time = self.time();
        match step {
            Step::Motion(x, y) => {
                self.pointer
                    .motion_absolute(time, x, y, layout.width.max(1), layout.height.max(1));
                self.pointer.frame();
            }
            Step::Button(code, down) => {
                let state = if down {
                    wl_pointer::ButtonState::Pressed
                } else {
                    wl_pointer::ButtonState::Released
                };
                self.pointer.button(time, u32::from(code), state);
                self.pointer.frame();
            }
            Step::Wheel(vertical, steps) => {
                let axis = if vertical {
                    wl_pointer::Axis::VerticalScroll
                } else {
                    wl_pointer::Axis::HorizontalScroll
                };
                // 15 is libinput's angle per wheel click, what toolkits expect per discrete step.
                let value = f64::from(steps) * 15.0;
                if self.pointer.version() >= 2 {
                    self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
                    self.pointer.axis_discrete(time, axis, value, steps);
                } else {
                    self.pointer.axis(time, axis, value);
                }
                self.pointer.frame();
            }
            Step::Key(code, down) => {
                let Some(keyboard) = &self.keyboard else {
                    tracing::warn!("no virtual keyboard: the modifier key is not held");
                    return;
                };
                let bit = modifier_bit(code);
                if down {
                    self.modifiers |= bit;
                } else {
                    self.modifiers &= !bit;
                }
                keyboard.key(time, u32::from(code), u32::from(down));
                keyboard.modifiers(self.modifiers, 0, 0, 0);
            }
            Step::Pause(_) => {}
        }
    }
}

fn uinput_error(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::Backend(format!("/dev/uinput: {error}"))
}

/// An absolute pointer plus modifier keys on `/dev/uinput`. libinput treats a device with
/// `ABS_X`/`ABS_Y` and mouse buttons as an absolute pointer (like a VM tablet), which the
/// compositor maps onto the whole layout.
pub(crate) struct Uinput {
    device: VirtualDevice,
    layout: Rect,
}

impl Uinput {
    /// Creates the device for a layout of `layout`'s size.
    ///
    /// # Errors
    /// When `/dev/uinput` cannot be opened (see the udev rule in the README) or set up.
    pub(crate) fn new(layout: Rect) -> Result<Self, PlatformError> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for key in [
            code::BTN_LEFT,
            code::BTN_RIGHT,
            code::BTN_MIDDLE,
            code::KEY_LEFTCTRL,
            code::KEY_LEFTSHIFT,
            code::KEY_LEFTALT,
            code::KEY_LEFTMETA,
        ] {
            keys.insert(KeyCode::new(key));
        }
        let mut wheels = AttributeSet::<RelativeAxisCode>::new();
        wheels.insert(RelativeAxisCode::REL_WHEEL);
        wheels.insert(RelativeAxisCode::REL_HWHEEL);
        let axis = |code, span: u32| {
            UinputAbsSetup::new(
                code,
                AbsInfo::new(
                    0,
                    0,
                    i32::try_from(span.max(2) - 1).unwrap_or(i32::MAX),
                    0,
                    0,
                    1,
                ),
            )
        };
        let device = VirtualDevice::builder()
            .map_err(uinput_error)?
            .name("nomouse virtual pointer")
            .with_keys(&keys)
            .map_err(uinput_error)?
            .with_relative_axes(&wheels)
            .map_err(uinput_error)?
            .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_X, layout.width))
            .map_err(uinput_error)?
            .with_absolute_axis(&axis(AbsoluteAxisCode::ABS_Y, layout.height))
            .map_err(uinput_error)?
            .build()
            .map_err(uinput_error)?;
        std::thread::sleep(UINPUT_SETTLE);
        Ok(Self { device, layout })
    }

    /// The layout the device was created for (a layout change needs a new device).
    pub(crate) const fn layout(&self) -> Rect {
        self.layout
    }

    /// Sends one step.
    ///
    /// # Errors
    /// When the device write fails.
    pub(crate) fn send(&mut self, step: Step) -> Result<(), PlatformError> {
        let events = match step {
            Step::Motion(x, y) => vec![
                InputEvent::new(
                    EventType::ABSOLUTE.0,
                    AbsoluteAxisCode::ABS_X.0,
                    i32::try_from(x).unwrap_or(i32::MAX),
                ),
                InputEvent::new(
                    EventType::ABSOLUTE.0,
                    AbsoluteAxisCode::ABS_Y.0,
                    i32::try_from(y).unwrap_or(i32::MAX),
                ),
            ],
            Step::Button(code, down) | Step::Key(code, down) => {
                vec![InputEvent::new(EventType::KEY.0, code, i32::from(down))]
            }
            Step::Wheel(vertical, steps) => {
                // Positive REL_WHEEL scrolls up; positive steps scroll down.
                let (axis, value) = if vertical {
                    (RelativeAxisCode::REL_WHEEL, -steps)
                } else {
                    (RelativeAxisCode::REL_HWHEEL, steps)
                };
                vec![InputEvent::new(EventType::RELATIVE.0, axis.0, value)]
            }
            Step::Pause(_) => return Ok(()),
        };
        self.device.emit(&events).map_err(uinput_error)
    }
}

/// Whether `/dev/uinput` can be opened for writing by this user.
pub(crate) fn uinput_writable() -> Result<(), String> {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .map(drop)
        .map_err(|e| format!("/dev/uinput: {e}"))
}

#[cfg(test)]
mod tests {
    use nomouse_core::Point;

    use super::*;

    const LAYOUT: Rect = Rect {
        x: -100,
        y: 0,
        width: 3200,
        height: 1000,
    };

    #[test]
    fn motion_is_layout_relative_and_clamped() {
        assert_eq!(
            step(Fake::Motion(Point::new(0, 10)), LAYOUT),
            Some(Step::Motion(100, 10))
        );
        assert_eq!(
            step(Fake::Motion(Point::new(-500, 5000)), LAYOUT),
            Some(Step::Motion(0, 999))
        );
    }

    #[test]
    fn buttons_wheels_and_keys() {
        assert_eq!(
            step(Fake::Press(1), LAYOUT),
            Some(Step::Button(code::BTN_LEFT, true))
        );
        assert_eq!(
            step(Fake::Release(3), LAYOUT),
            Some(Step::Button(code::BTN_RIGHT, false))
        );
        assert_eq!(step(Fake::Press(4), LAYOUT), Some(Step::Wheel(true, -1)));
        assert_eq!(step(Fake::Press(5), LAYOUT), Some(Step::Wheel(true, 1)));
        assert_eq!(step(Fake::Press(6), LAYOUT), Some(Step::Wheel(false, -1)));
        assert_eq!(step(Fake::Press(7), LAYOUT), Some(Step::Wheel(false, 1)));
        assert_eq!(step(Fake::Release(5), LAYOUT), None);
        assert_eq!(
            step(Fake::KeyDown(37), LAYOUT),
            Some(Step::Key(code::KEY_LEFTCTRL, true))
        );
        assert_eq!(step(Fake::KeyDown(3), LAYOUT), None);
        assert_eq!(step(Fake::Pause(40), LAYOUT), Some(Step::Pause(40)));
    }

    #[test]
    fn modifiers_become_x_keycodes_and_mask_bits() {
        let ctrl_shift = Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::NONE
        };
        assert_eq!(modifier_keycodes(ctrl_shift), vec![37, 50]);
        assert_eq!(modifier_keycodes(Modifiers::NONE), Vec::<u8>::new());
        assert_eq!(modifier_bit(code::KEY_LEFTCTRL), 4);
        assert_eq!(modifier_bit(code::KEY_LEFTMETA), 64);
        assert_eq!(modifier_bit(1), 0);
    }
}
