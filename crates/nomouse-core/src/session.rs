//! One activation of hint mode: label the elements, follow typed keys, produce the action.

use crate::action::{Action, ActionKind};
use crate::element::{Element, Point};
use crate::filter::{FilterState, LabelFilter};
use crate::hints::{Alphabet, Hint, assign_hints};
use crate::hotkey::KeyPress;

/// What a key press did to the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    /// The prefix changed (or a character was removed); redraw the matching hints.
    Narrowed,
    /// The key matched nothing and was ignored.
    Rejected,
    /// Drag mode: the start point was picked; hints are shown again for the end point.
    DragStarted(Point),
    /// Scroll mode: the scroll target was picked; the caller enters scroll mode there.
    ScrollTarget(Point),
    /// Perform this action and end the session.
    Perform(Action),
    /// The user cancelled (Escape); hide the overlay.
    Cancelled,
}

/// The state of one hint-mode activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintSession {
    hints: Vec<Hint>,
    filter: LabelFilter,
    kind: ActionKind,
    drag_from: Option<Point>,
}

impl HintSession {
    /// Labels the clickable `elements` (others are dropped) for an action of `kind`.
    pub fn new(alphabet: &Alphabet, elements: Vec<Element>, kind: ActionKind) -> Self {
        let clickable = elements.into_iter().filter(Element::is_clickable).collect();
        let hints = assign_hints(alphabet, clickable);
        let filter = LabelFilter::new(hints.iter().map(|hint| hint.label.clone()).collect());
        Self {
            hints,
            filter,
            kind,
            drag_from: None,
        }
    }

    /// All hints of the session, in label order.
    pub fn hints(&self) -> &[Hint] {
        &self.hints
    }

    /// The hints matching the typed prefix, ready for [`crate::Overlay::show`].
    pub fn visible(&self) -> Vec<Hint> {
        self.filter
            .matching()
            .map(|index| self.hints[index].clone())
            .collect()
    }

    /// The prefix typed so far.
    pub fn typed(&self) -> &str {
        self.filter.typed()
    }

    /// Whether there is nothing to label.
    pub fn is_empty(&self) -> bool {
        self.hints.is_empty()
    }

    /// Applies one key press.
    pub fn key(&mut self, key: KeyPress) -> SessionEvent {
        match key {
            KeyPress::Escape => SessionEvent::Cancelled,
            KeyPress::Backspace => {
                if self.filter.pop() {
                    SessionEvent::Narrowed
                } else {
                    SessionEvent::Rejected
                }
            }
            KeyPress::Char(c) => match self.filter.push(c) {
                FilterState::Pending => SessionEvent::Narrowed,
                FilterState::Rejected => SessionEvent::Rejected,
                FilterState::Matched(index) => self.select(self.hints[index].element.target()),
            },
            KeyPress::Enter
            | KeyPress::Tab
            | KeyPress::Up
            | KeyPress::Down
            | KeyPress::Left
            | KeyPress::Right => SessionEvent::Rejected,
        }
    }

    fn select(&mut self, point: Point) -> SessionEvent {
        self.filter.reset();
        if let Some(action) = self.kind.at(point) {
            return SessionEvent::Perform(action);
        }
        match (self.kind, self.drag_from) {
            (ActionKind::Drag, None) => {
                self.drag_from = Some(point);
                SessionEvent::DragStarted(point)
            }
            (ActionKind::Drag, Some(from)) => SessionEvent::Perform(Action::drag(from, point)),
            _ => SessionEvent::ScrollTarget(point),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementId, Rect, Role};

    fn elements() -> Vec<Element> {
        (0..3)
            .map(|i| Element {
                id: ElementId(i),
                role: if i == 2 { Role::Text } else { Role::Button },
                name: format!("e{i}"),
                bounds: Rect::new(0, i32::try_from(i).unwrap() * 100, 10, 10),
                enabled: true,
            })
            .collect()
    }

    #[test]
    fn click_flow() {
        let mut session = HintSession::new(&Alphabet::default(), elements(), ActionKind::Click);
        assert_eq!(session.hints().len(), 2);
        assert_eq!(session.key(KeyPress::Char('z')), SessionEvent::Rejected);
        assert_eq!(
            session.key(KeyPress::Char('s')),
            SessionEvent::Perform(Action::click(Point::new(5, 105)))
        );
    }

    #[test]
    fn drag_needs_two_picks() {
        let mut session = HintSession::new(&Alphabet::default(), elements(), ActionKind::Drag);
        assert_eq!(
            session.key(KeyPress::Char('a')),
            SessionEvent::DragStarted(Point::new(5, 5))
        );
        assert_eq!(session.visible().len(), 2);
        assert_eq!(
            session.key(KeyPress::Char('s')),
            SessionEvent::Perform(Action::drag(Point::new(5, 5), Point::new(5, 105)))
        );
    }

    #[test]
    fn scroll_escape_backspace() {
        let many: Vec<Element> = (0..20)
            .map(|i| Element {
                id: ElementId(i),
                role: Role::Button,
                name: String::new(),
                bounds: Rect::new(i32::try_from(i).unwrap() * 20, 0, 10, 10),
                enabled: true,
            })
            .collect();
        let mut session = HintSession::new(&Alphabet::default(), many, ActionKind::Scroll);
        assert_eq!(session.key(KeyPress::Backspace), SessionEvent::Rejected);
        let first = session.hints()[19].label.clone();
        let mut chars = first.chars();
        assert_eq!(
            session.key(KeyPress::Char(chars.next().unwrap())),
            SessionEvent::Narrowed
        );
        assert!(session.visible().len() < 20);
        assert_eq!(session.key(KeyPress::Backspace), SessionEvent::Narrowed);
        assert_eq!(session.typed(), "");
        assert_eq!(session.key(KeyPress::Escape), SessionEvent::Cancelled);
        for c in first.chars() {
            session.key(KeyPress::Char(c));
        }
        assert_eq!(session.key(KeyPress::Enter), SessionEvent::Rejected);
    }

    #[test]
    fn scroll_target() {
        let mut session = HintSession::new(&Alphabet::default(), elements(), ActionKind::Scroll);
        assert_eq!(
            session.key(KeyPress::Char('a')),
            SessionEvent::ScrollTarget(Point::new(5, 5))
        );
        assert!(!session.is_empty());
    }
}
