//! Pointer actions as XTEST event sequences.
//!
//! [`plan`] turns an action into primitive fake events (pure, tested); the X11 connection
//! sends them with `XTestFakeInput`.

use nomouse_core::{Action, MouseButton, Point};

/// One primitive synthetic event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fake {
    /// Absolute pointer motion.
    Motion(Point),
    /// Button press (1 left, 2 middle, 3 right, 4-7 wheel).
    Press(u8),
    /// Button release.
    Release(u8),
    /// Key press by keycode (modifiers for modifier-clicks).
    KeyDown(u8),
    /// Key release by keycode.
    KeyUp(u8),
    /// A pause in milliseconds so applications see distinct events (drags, double clicks).
    Pause(u64),
}

const fn button(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
    }
}

/// Intermediate motion steps of a drag, so toolkits register movement past their threshold.
const DRAG_STEPS: i32 = 8;

fn click(fakes: &mut Vec<Fake>, button: u8) {
    fakes.extend([Fake::Press(button), Fake::Release(button)]);
}

fn wheel(fakes: &mut Vec<Fake>, negative: u8, positive: u8, steps: i32) {
    let code = if steps < 0 { negative } else { positive };
    for _ in 0..steps.unsigned_abs() {
        click(fakes, code);
    }
}

/// How long held modifiers settle before the pointer acts, so the app sees them as held.
const MODIFIER_SETTLE_MS: u64 = 30;

/// The event sequence for `action`, holding the `modifiers` keycodes down around it.
pub(crate) fn plan(action: &Action, modifiers: &[u8]) -> Vec<Fake> {
    let mut fakes: Vec<Fake> = modifiers.iter().map(|code| Fake::KeyDown(*code)).collect();
    if !modifiers.is_empty() {
        fakes.push(Fake::Pause(MODIFIER_SETTLE_MS));
    }
    match *action {
        Action::Click {
            at,
            button: b,
            count,
            ..
        } => {
            fakes.push(Fake::Motion(at));
            for i in 0..count.max(1) {
                if i > 0 {
                    fakes.push(Fake::Pause(40));
                }
                click(&mut fakes, button(b));
            }
        }
        Action::Hover { at } => fakes.push(Fake::Motion(at)),
        Action::Drag {
            from,
            to,
            button: b,
        } => {
            fakes.extend([Fake::Motion(from), Fake::Press(button(b)), Fake::Pause(30)]);
            for step in 1..=DRAG_STEPS {
                let lerp = |a: i32, b: i32| {
                    let value = i64::from(a)
                        + (i64::from(b) - i64::from(a)) * i64::from(step) / i64::from(DRAG_STEPS);
                    i32::try_from(value).unwrap_or(b)
                };
                fakes.extend([
                    Fake::Motion(Point::new(lerp(from.x, to.x), lerp(from.y, to.y))),
                    Fake::Pause(10),
                ]);
            }
            fakes.push(Fake::Release(button(b)));
        }
        Action::Scroll { at, dx, dy } => {
            fakes.push(Fake::Motion(at));
            wheel(&mut fakes, 4, 5, dy);
            wheel(&mut fakes, 6, 7, dx);
        }
    }
    fakes.extend(modifiers.iter().rev().map(|code| Fake::KeyUp(*code)));
    fakes
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: Point = Point::new(5, 6);

    #[test]
    fn clicks() {
        assert_eq!(
            plan(&Action::click(P), &[]),
            vec![Fake::Motion(P), Fake::Press(1), Fake::Release(1)]
        );
        assert_eq!(
            plan(&Action::right_click(P), &[]),
            vec![Fake::Motion(P), Fake::Press(3), Fake::Release(3)]
        );
        let double = plan(&Action::double_click(P), &[]);
        assert_eq!(double.iter().filter(|f| **f == Fake::Press(1)).count(), 2);
        let middle = Action::Click {
            at: P,
            button: MouseButton::Middle,
            count: 1,
            modifiers: nomouse_core::Modifiers::NONE,
        };
        assert_eq!(plan(&middle, &[])[1], Fake::Press(2));
    }

    #[test]
    fn modifier_click_wraps_keys() {
        let fakes = plan(&Action::click(P), &[37, 50]);
        assert_eq!(fakes[..2], [Fake::KeyDown(37), Fake::KeyDown(50)]);
        assert_eq!(fakes[fakes.len() - 2..], [Fake::KeyUp(50), Fake::KeyUp(37)]);
    }

    #[test]
    fn scroll_uses_wheel_buttons() {
        let fakes = plan(&Action::scroll(P, -1, 3), &[]);
        assert_eq!(fakes.iter().filter(|f| **f == Fake::Press(5)).count(), 3);
        assert_eq!(fakes.iter().filter(|f| **f == Fake::Press(6)).count(), 1);
        let up = plan(&Action::scroll(P, 2, -2), &[]);
        assert_eq!(up.iter().filter(|f| **f == Fake::Press(4)).count(), 2);
        assert_eq!(up.iter().filter(|f| **f == Fake::Press(7)).count(), 2);
    }

    #[test]
    fn drag_moves_while_pressed() {
        let to = Point::new(85, 6);
        let fakes = plan(&Action::drag(P, to), &[]);
        assert_eq!(fakes[..2], [Fake::Motion(P), Fake::Press(1)]);
        assert_eq!(fakes.last(), Some(&Fake::Release(1)));
        assert!(fakes.contains(&Fake::Motion(to)));
        assert!(fakes.contains(&Fake::Motion(Point::new(45, 6))));
        assert_eq!(plan(&Action::hover(P), &[]), vec![Fake::Motion(P)]);
    }
}
