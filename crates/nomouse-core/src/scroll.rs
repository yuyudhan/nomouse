//! Scroll mode: the scrollable areas of a window, the active one, and what each key does.
//!
//! The largest area is active to begin with, so the scroll keys work at once:
//!
//! - `j` `k` `h` `l` (or the arrows) scroll by `scroll.step` wheel steps, with Shift by
//!   `scroll.dash_step`; `d` (or Space) and `u` scroll five steps' worth down and up;
//! - `gg` scrolls to the top and `G` to the bottom;
//! - Tab and Shift-Tab make the next and previous area active (in reading order), and the
//!   digits `1`-`9`, `0` the first to tenth;
//! - Escape or Enter ends scroll mode.

use crate::element::{Element, ElementId, Rect, Role, reading_order};
use crate::hints::Hint;
use crate::hotkey::{KeyEvent, KeyPress, Modifiers};

/// Wheel steps `gg` and `G` send: enough to reach either end of a list a few hundred rows long.
pub const EDGE_STEPS: i32 = 500;

/// How many wheel steps `d` and `u` send per `scroll.step`.
const PAGE: i32 = 5;

/// Labels of the areas, by position: digits pick an area.
const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];

/// What a key did in scroll mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollEvent {
    /// Scroll the active area by these wheel steps (positive `dy` scrolls down).
    Scroll {
        /// Horizontal steps.
        dx: i32,
        /// Vertical steps.
        dy: i32,
    },
    /// Another area became active: move the pointer there and redraw.
    Switched,
    /// The key did nothing (or was the first `g` of `gg`).
    Ignored,
    /// Leave scroll mode.
    Done,
}

/// One scroll-mode activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrollSession {
    areas: Vec<Rect>,
    active: usize,
    step: i32,
    dash_step: i32,
    /// The first `g` of `gg` was typed.
    pending_g: bool,
}

fn area_size(rect: Rect) -> u64 {
    u64::from(rect.width) * u64::from(rect.height)
}

impl ScrollSession {
    /// Scroll mode over `areas` (sorted into reading order) with the largest one active; `None`
    /// without an area. `step` and `dash_step` are wheel steps per key press.
    pub fn new(mut areas: Vec<Rect>, step: i32, dash_step: i32) -> Option<Self> {
        areas.retain(|area| !area.is_empty());
        areas.sort_by_key(|area| (area.y, area.x));
        let largest = areas
            .iter()
            .enumerate()
            .max_by(|(a_index, a), (b_index, b)| {
                // The first of equally large areas wins.
                area_size(**a)
                    .cmp(&area_size(**b))
                    .then(b_index.cmp(a_index))
            })
            .map(|(index, _)| index)?;
        Some(Self {
            areas,
            active: largest,
            step,
            dash_step,
            pending_g: false,
        })
    }

    /// The scroll areas of `elements` ([`Role::ScrollArea`], enabled, with a size).
    pub fn areas_of(elements: &[Element]) -> Vec<Rect> {
        let mut areas: Vec<&Element> = elements
            .iter()
            .filter(|e| e.role == Role::ScrollArea && e.enabled && !e.bounds.is_empty())
            .collect();
        areas.sort_by(|a, b| reading_order(a, b));
        areas.into_iter().map(|e| e.bounds).collect()
    }

    /// The areas, in reading order.
    pub fn areas(&self) -> &[Rect] {
        &self.areas
    }

    /// The active area: scroll keys scroll it.
    pub fn active(&self) -> Rect {
        self.areas[self.active]
    }

    /// The digit labels of the first ten areas, for the overlay.
    pub fn hints(&self) -> Vec<Hint> {
        self.areas
            .iter()
            .zip(DIGITS)
            .enumerate()
            .map(|(index, (bounds, label))| Hint {
                label: label.to_owned(),
                element: Element {
                    id: ElementId(u64::try_from(index).unwrap_or(u64::MAX)),
                    role: Role::ScrollArea,
                    name: String::new(),
                    description: String::new(),
                    value: String::new(),
                    bounds: *bounds,
                    enabled: true,
                    offscreen: false,
                },
            })
            .collect()
    }

    /// The label of the active area, highlighted by the overlay.
    pub fn active_label(&self) -> Option<&'static str> {
        DIGITS.get(self.active).copied()
    }

    fn switch_to(&mut self, index: usize) -> ScrollEvent {
        if index >= self.areas.len() || index == self.active {
            return ScrollEvent::Ignored;
        }
        self.active = index;
        ScrollEvent::Switched
    }

    /// Applies one key press (a bare [`KeyPress`] is a press without modifiers). An upper-case
    /// letter counts as its lower-case letter with Shift, whichever way the backend reports it.
    pub fn key(&mut self, event: impl Into<KeyEvent>) -> ScrollEvent {
        let KeyEvent { key, mut modifiers } = event.into();
        let key = match key {
            KeyPress::Char(c) if c.is_uppercase() => {
                modifiers = modifiers.union(Modifiers::SHIFT);
                KeyPress::Char(c.to_lowercase().next().unwrap_or(c))
            }
            other => other,
        };
        let first_g = std::mem::take(&mut self.pending_g);
        let plain = modifiers.is_empty();
        let shift = modifiers == Modifiers::SHIFT;
        let line = if shift { self.dash_step } else { self.step };
        let page = self.step.saturating_mul(PAGE);
        let scroll = |dx: i32, dy: i32| ScrollEvent::Scroll { dx, dy };
        match key {
            KeyPress::Escape | KeyPress::Enter => ScrollEvent::Done,
            _ if !(plain || shift) => ScrollEvent::Ignored,
            KeyPress::Char('j') | KeyPress::Down => scroll(0, line),
            KeyPress::Char('k') | KeyPress::Up => scroll(0, -line),
            KeyPress::Char('h') | KeyPress::Left => scroll(-line, 0),
            KeyPress::Char('l') | KeyPress::Right => scroll(line, 0),
            KeyPress::Char('g') if shift => scroll(0, EDGE_STEPS),
            KeyPress::Char('g') if first_g => scroll(0, -EDGE_STEPS),
            KeyPress::Char('g') => {
                self.pending_g = true;
                ScrollEvent::Ignored
            }
            KeyPress::Char('d' | ' ') if plain => scroll(0, page),
            KeyPress::Char('u') if plain => scroll(0, -page),
            KeyPress::Tab if plain => self.switch_to((self.active + 1) % self.areas.len()),
            KeyPress::Tab => {
                let count = self.areas.len();
                self.switch_to((self.active + count - 1) % count)
            }
            KeyPress::Char(digit @ '0'..='9') if plain => {
                let index = DIGITS.iter().position(|d| d.starts_with(digit));
                index.map_or(ScrollEvent::Ignored, |index| self.switch_to(index))
            }
            _ => ScrollEvent::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> ScrollSession {
        // A small area on top, the large one below it, a medium one to its right.
        let areas = vec![
            Rect::new(0, 400, 300, 600),
            Rect::new(0, 0, 100, 100),
            Rect::new(400, 400, 200, 200),
        ];
        ScrollSession::new(areas, 3, 10).unwrap()
    }

    #[test]
    fn largest_area_is_active_and_areas_are_in_reading_order() {
        let s = session();
        assert_eq!(s.areas()[0], Rect::new(0, 0, 100, 100));
        assert_eq!(s.active(), Rect::new(0, 400, 300, 600));
        assert_eq!(s.active_label(), Some("2"));
        let labels: Vec<String> = s.hints().into_iter().map(|h| h.label).collect();
        assert_eq!(labels, ["1", "2", "3"]);
        assert_eq!(ScrollSession::new(Vec::new(), 3, 10), None);
        assert_eq!(ScrollSession::new(vec![Rect::new(0, 0, 0, 5)], 3, 10), None);
        let ties = ScrollSession::new(
            vec![Rect::new(50, 0, 10, 10), Rect::new(0, 0, 10, 10)],
            3,
            10,
        );
        assert_eq!(ties.map(|s| s.active()), Some(Rect::new(0, 0, 10, 10)));
    }

    #[test]
    fn hjkl_and_arrows_scroll_by_step_and_shift_by_dash_step() {
        let mut s = session();
        let cases = [
            (KeyPress::Char('j'), (0, 3)),
            (KeyPress::Down, (0, 3)),
            (KeyPress::Char('k'), (0, -3)),
            (KeyPress::Up, (0, -3)),
            (KeyPress::Char('h'), (-3, 0)),
            (KeyPress::Left, (-3, 0)),
            (KeyPress::Char('l'), (3, 0)),
            (KeyPress::Right, (3, 0)),
            (KeyPress::Char('d'), (0, 15)),
            (KeyPress::Char(' '), (0, 15)),
            (KeyPress::Char('u'), (0, -15)),
        ];
        for (key, (dx, dy)) in cases {
            assert_eq!(s.key(key), ScrollEvent::Scroll { dx, dy }, "{key:?}");
        }
        for (key, (dx, dy)) in [
            ('j', (0, 10)),
            ('k', (0, -10)),
            ('h', (-10, 0)),
            ('l', (10, 0)),
        ] {
            assert_eq!(
                s.key(KeyPress::Char(key).with(Modifiers::SHIFT)),
                ScrollEvent::Scroll { dx, dy }
            );
        }
        assert_eq!(
            s.key(KeyPress::Char('j').with(Modifiers::CTRL)),
            ScrollEvent::Ignored
        );
    }

    #[test]
    fn gg_goes_to_the_top_and_shift_g_to_the_bottom() {
        let mut s = session();
        assert_eq!(s.key(KeyPress::Char('g')), ScrollEvent::Ignored);
        assert_eq!(
            s.key(KeyPress::Char('g')),
            ScrollEvent::Scroll {
                dx: 0,
                dy: -EDGE_STEPS
            }
        );
        // A key between the two `g`s cancels the first.
        s.key(KeyPress::Char('g'));
        s.key(KeyPress::Char('x'));
        assert_eq!(s.key(KeyPress::Char('g')), ScrollEvent::Ignored);
        assert_eq!(
            s.key(KeyPress::Char('g').with(Modifiers::SHIFT)),
            ScrollEvent::Scroll {
                dx: 0,
                dy: EDGE_STEPS
            }
        );
        // A backend that reports Shift-g as `G` gets the same.
        assert_eq!(
            s.key(KeyPress::Char('G')),
            ScrollEvent::Scroll {
                dx: 0,
                dy: EDGE_STEPS
            }
        );
    }

    #[test]
    fn tab_cycles_and_digits_jump() {
        let mut s = session();
        assert_eq!(s.key(KeyPress::Tab), ScrollEvent::Switched);
        assert_eq!(s.active_label(), Some("3"));
        assert_eq!(s.key(KeyPress::Tab), ScrollEvent::Switched);
        assert_eq!(s.active_label(), Some("1"));
        assert_eq!(
            s.key(KeyPress::Tab.with(Modifiers::SHIFT)),
            ScrollEvent::Switched
        );
        assert_eq!(s.active_label(), Some("3"));
        assert_eq!(s.key(KeyPress::Char('2')), ScrollEvent::Switched);
        assert_eq!(s.active(), Rect::new(0, 400, 300, 600));
        assert_eq!(s.key(KeyPress::Char('2')), ScrollEvent::Ignored);
        assert_eq!(s.key(KeyPress::Char('9')), ScrollEvent::Ignored);
        assert_eq!(s.key(KeyPress::Char('0')), ScrollEvent::Ignored);
        let mut single = ScrollSession::new(vec![Rect::new(0, 0, 10, 10)], 3, 10).unwrap();
        assert_eq!(single.key(KeyPress::Tab), ScrollEvent::Ignored);
    }

    #[test]
    fn escape_and_enter_end_and_other_keys_are_ignored() {
        let mut s = session();
        assert_eq!(s.key(KeyPress::Escape), ScrollEvent::Done);
        assert_eq!(s.key(KeyPress::Enter), ScrollEvent::Done);
        assert_eq!(s.key(KeyPress::Char('x')), ScrollEvent::Ignored);
        assert_eq!(s.key(KeyPress::Backspace), ScrollEvent::Ignored);
    }

    #[test]
    fn scroll_areas_of_elements() {
        let element = |role, bounds, enabled| Element {
            id: ElementId(0),
            role,
            name: String::new(),
            description: String::new(),
            value: String::new(),
            bounds,
            enabled,
            offscreen: false,
        };
        let elements = [
            element(Role::Button, Rect::new(0, 0, 10, 10), true),
            element(Role::ScrollArea, Rect::new(0, 50, 10, 10), true),
            element(Role::ScrollArea, Rect::new(0, 20, 10, 10), true),
            element(Role::ScrollArea, Rect::new(0, 30, 10, 10), false),
            element(Role::ScrollArea, Rect::new(0, 40, 0, 10), true),
        ];
        assert_eq!(
            ScrollSession::areas_of(&elements),
            [Rect::new(0, 20, 10, 10), Rect::new(0, 50, 10, 10)]
        );
    }
}
