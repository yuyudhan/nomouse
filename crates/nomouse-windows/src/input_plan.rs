//! Turns core [`Action`]s into `SendInput` batches.

use nomouse_core::{Action, Modifiers, MouseButton, Point};
use nomouse_sys_windows::{Button, Edges, InputStep};

use crate::keys::modifier_keys;

/// `WHEEL_DELTA`: one wheel notch.
const WHEEL_DELTA: i64 = 120;
/// Intermediate pointer moves during a drag, so targets see motion between press and release.
const DRAG_STEPS: i32 = 8;

/// Input batches; the injector pauses briefly between batches (drag start and end).
pub(crate) type Plan = Vec<Vec<InputStep>>;

/// Normalises a screen coordinate on one axis of the virtual desktop to `0..=65535`.
///
/// Windows maps a normalized value `n` back to `origin + n * length / 65536` (truncating), so
/// rounding up here lands exactly on the requested pixel.
fn normalize(value: i32, origin: i32, length: u32) -> i32 {
    if length == 0 {
        return 0;
    }
    let length = i64::from(length);
    let offset = (i64::from(value) - i64::from(origin)).clamp(0, length - 1);
    let normalized = (offset * 65_536 + length - 1) / length;
    i32::try_from(normalized.min(65_535)).unwrap_or(65_535)
}

/// An absolute move to `point` on the virtual desktop `screen`.
pub(crate) fn move_to(point: Point, screen: Edges) -> InputStep {
    InputStep::Move {
        x: normalize(point.x, screen.left, screen.width()),
        y: normalize(point.y, screen.top, screen.height()),
    }
}

const fn button(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
    }
}

fn click_steps(at: Point, which: MouseButton, count: u8, screen: Edges) -> Vec<InputStep> {
    let which = button(which);
    let mut steps = vec![move_to(at, screen)];
    for _ in 0..count.max(1) {
        steps.push(InputStep::Button {
            button: which,
            down: true,
        });
        steps.push(InputStep::Button {
            button: which,
            down: false,
        });
    }
    steps
}

/// The raw wheel delta for `steps` notches (the binary already applied `scroll.step`).
fn wheel_delta(steps: i32) -> i32 {
    let delta = i64::from(steps) * WHEEL_DELTA;
    i32::try_from(delta).unwrap_or(if delta < 0 { i32::MIN } else { i32::MAX })
}

fn lerp(from: i32, to: i32, step: i32) -> i32 {
    let value = i64::from(from)
        + (i64::from(to) - i64::from(from)) * i64::from(step) / i64::from(DRAG_STEPS);
    i32::try_from(value).unwrap_or(to)
}

/// The input batches for `action` on virtual desktop `screen`.
pub(crate) fn plan(action: &Action, screen: Edges) -> Plan {
    match *action {
        Action::Click {
            at,
            button,
            count,
            modifiers,
        } => vec![modifier_click(at, button, count, modifiers, screen)],
        Action::Hover { at } => vec![vec![move_to(at, screen)]],
        Action::Drag {
            from,
            to,
            button: which,
        } => {
            let which = button(which);
            let press = vec![
                move_to(from, screen),
                InputStep::Button {
                    button: which,
                    down: true,
                },
            ];
            let motion = (1..=DRAG_STEPS)
                .map(|step| {
                    move_to(
                        Point::new(lerp(from.x, to.x, step), lerp(from.y, to.y, step)),
                        screen,
                    )
                })
                .collect();
            let release = vec![InputStep::Button {
                button: which,
                down: false,
            }];
            vec![press, motion, release]
        }
        Action::Scroll { at, dx, dy } => {
            let mut steps = vec![move_to(at, screen)];
            if dy != 0 {
                // Positive dy scrolls down, which is a negative (towards the user) wheel delta.
                steps.push(InputStep::Wheel(wheel_delta(dy).saturating_neg()));
            }
            if dx != 0 {
                steps.push(InputStep::HorizontalWheel(wheel_delta(dx)));
            }
            vec![steps]
        }
    }
}

/// A click with `modifiers` held: press modifiers, click `count` times, release them in
/// reverse order. One batch, so no other input interleaves with the held keys.
fn modifier_click(
    at: Point,
    which: MouseButton,
    count: u8,
    modifiers: Modifiers,
    screen: Edges,
) -> Vec<InputStep> {
    let keys = modifier_keys(modifiers);
    let mut steps: Vec<InputStep> = keys
        .iter()
        .map(|&vk| InputStep::Key { vk, down: true })
        .collect();
    steps.extend(click_steps(at, which, count, screen));
    steps.extend(
        keys.iter()
            .rev()
            .map(|&vk| InputStep::Key { vk, down: false }),
    );
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Edges = Edges::new(-1920, 0, 1920, 1080);

    /// What Windows does with a normalized coordinate.
    fn denormalize(n: i32, origin: i32, length: u32) -> i32 {
        origin + i32::try_from(i64::from(n) * i64::from(length) / 65_536).unwrap()
    }

    #[test]
    fn normalization_round_trips_every_pixel() {
        for length in [1u32, 2, 799, 1080, 3840, 7680] {
            for offset in 0..i32::try_from(length).unwrap() {
                let n = normalize(-100 + offset, -100, length);
                assert!((0..=65_535).contains(&n));
                assert_eq!(
                    denormalize(n, -100, length),
                    -100 + offset,
                    "length {length}"
                );
            }
        }
    }

    #[test]
    fn normalization_clamps() {
        assert_eq!(normalize(-5000, 0, 100), 0);
        assert_eq!(denormalize(normalize(5000, 0, 100), 0, 100), 99);
        assert_eq!(normalize(3, 0, 0), 0);
    }

    #[test]
    fn double_right_click() {
        let at = Point::new(0, 540);
        let plan = plan(
            &Action::Click {
                at,
                button: MouseButton::Right,
                count: 2,
                modifiers: Modifiers::NONE,
            },
            SCREEN,
        );
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].len(), 5);
        assert_eq!(plan[0][0], move_to(at, SCREEN));
        assert_eq!(
            plan[0][3],
            InputStep::Button {
                button: Button::Right,
                down: true
            }
        );
    }

    #[test]
    fn scroll_uses_notches_and_sign() {
        let plan = plan(&Action::scroll(Point::new(1, 1), -1, 2), SCREEN);
        assert_eq!(
            &plan[0][1..],
            &[InputStep::Wheel(-240), InputStep::HorizontalWheel(-120)]
        );
        assert_eq!(wheel_delta(i32::MAX), i32::MAX);
    }

    #[test]
    fn drag_moves_in_steps() {
        let from = Point::new(-1000, 100);
        let to = Point::new(-200, 900);
        let plan = plan(&Action::drag(from, to), SCREEN);
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[1].len(), 8);
        assert_eq!(plan[1][7], move_to(to, SCREEN));
        assert_eq!(
            plan[2],
            vec![InputStep::Button {
                button: Button::Left,
                down: false
            }]
        );
    }

    #[test]
    fn modifier_click_wraps_keys() {
        let modifiers = Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        };
        let plan = plan(
            &Action::click(Point::new(5, 5)).with_modifiers(modifiers),
            SCREEN,
        );
        assert_eq!(plan.len(), 1);
        let steps = &plan[0];
        assert_eq!(steps.len(), 7);
        assert_eq!(
            steps[0],
            InputStep::Key {
                vk: 0x11,
                down: true
            }
        );
        assert_eq!(
            steps[1],
            InputStep::Key {
                vk: 0x10,
                down: true
            }
        );
        assert_eq!(
            steps[steps.len() - 2],
            InputStep::Key {
                vk: 0x10,
                down: false
            }
        );
        assert_eq!(
            steps[steps.len() - 1],
            InputStep::Key {
                vk: 0x11,
                down: false
            }
        );
    }
}
