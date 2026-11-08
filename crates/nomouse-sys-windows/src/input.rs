//! Synthetic mouse and modifier-key input through `SendInput`.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
    MOUSEINPUT, SendInput, VIRTUAL_KEY,
};

use crate::error::last_error;
use crate::types::{Button, InputStep, SysError, SysErrorKind};

fn mouse(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                // The wheel delta is a signed value carried in an unsigned field.
                mouseData: u32::from_ne_bytes(data.to_ne_bytes()),
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn to_input(step: InputStep) -> INPUT {
    match step {
        InputStep::Move { x, y } => mouse(
            x,
            y,
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        ),
        InputStep::Button { button, down } => {
            let flags = match (button, down) {
                (Button::Left, true) => MOUSEEVENTF_LEFTDOWN,
                (Button::Left, false) => MOUSEEVENTF_LEFTUP,
                (Button::Right, true) => MOUSEEVENTF_RIGHTDOWN,
                (Button::Right, false) => MOUSEEVENTF_RIGHTUP,
                (Button::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
                (Button::Middle, false) => MOUSEEVENTF_MIDDLEUP,
            };
            mouse(0, 0, 0, flags)
        }
        InputStep::Wheel(delta) => mouse(0, 0, delta, MOUSEEVENTF_WHEEL),
        InputStep::HorizontalWheel(delta) => mouse(0, 0, delta, MOUSEEVENTF_HWHEEL),
        InputStep::Key { vk, down } => INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: if down {
                        KEYBD_EVENT_FLAGS(0)
                    } else {
                        KEYEVENTF_KEYUP
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    }
}

/// Injects `steps` in order as one uninterruptible `SendInput` batch.
///
/// # Errors
/// [`SysErrorKind::AccessDenied`] when UIPI blocked the input (the foreground window belongs to
/// a process of higher integrity) or another error when fewer events were inserted.
pub fn send_input(steps: &[InputStep]) -> Result<(), SysError> {
    if steps.is_empty() {
        return Ok(());
    }
    let inputs: Vec<INPUT> = steps.iter().copied().map(to_input).collect();
    let size = i32::try_from(size_of::<INPUT>()).unwrap_or(i32::MAX);
    // SAFETY: `inputs` is a live, fully initialised slice and `size` is the size of one element,
    // as SendInput requires.
    let inserted = unsafe { SendInput(&inputs, size) };
    if usize::try_from(inserted).is_ok_and(|count| count == inputs.len()) {
        return Ok(());
    }
    let mut error = last_error("SendInput");
    if error.code == 0 {
        // SendInput reports UIPI blocking with a short count but no error code.
        error.kind = SysErrorKind::AccessDenied;
        error.message = format!(
            "only {inserted} of {} input events were inserted; the target window probably runs elevated",
            inputs.len()
        );
    }
    Err(error)
}
