//! One activation of hint mode: label the elements, follow typed keys, produce the action.
//!
//! Keys while labels are shown:
//!
//! - label characters narrow the labels and pick one;
//! - Backspace removes the last typed character, Escape cancels;
//! - for click actions, `1` toggles Shift, `2` Ctrl, `3` Alt and `4` Super (Command on macOS,
//!   the Windows key on Windows), held around the click (cmd/ctrl+click opens a link in a new
//!   tab); labels never contain digits, so these keys never clash with a label.
//!
//! In search mode ([`HintSession::search`]) typed characters first build the query: labels are
//! shown only on elements whose name fuzzily matches it, best match first. Enter clicks the best
//! match; Tab freezes the query so label characters (and the modifier digits) pick another match;
//! Backspace with no label typed returns to editing the query.

use crate::action::{Action, ActionKind};
use crate::element::{Element, Point, Role};
use crate::filter::{FilterState, LabelFilter};
use crate::hints::{Alphabet, Hint, assign_hints, generate_labels};
use crate::hotkey::{KeyPress, Modifiers};
use crate::search::{is_searchable, rank};

/// What a key press did to the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    /// The prefix or the search query changed (or a character was removed); redraw the hints.
    Narrowed,
    /// The key matched nothing and was ignored.
    Rejected,
    /// A modifier key toggled; the next click holds these modifiers.
    ModifiersChanged(Modifiers),
    /// Drag mode: the start point was picked; hints are shown again for the end point.
    DragStarted(Point),
    /// Scroll mode: the scroll target was picked; the caller enters scroll mode there.
    ScrollTarget(Point),
    /// Perform this action and end the session.
    Perform(Action),
    /// The user cancelled (Escape); hide the overlay.
    Cancelled,
}

/// Search-by-text state: the candidates and the query typed so far.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Search {
    candidates: Vec<Element>,
    query: String,
    /// Tab was pressed: characters pick labels instead of editing the query.
    picking: bool,
}

/// The state of one hint-mode activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintSession {
    alphabet: Alphabet,
    hints: Vec<Hint>,
    filter: LabelFilter,
    kind: ActionKind,
    drag_from: Option<Point>,
    modifiers: Modifiers,
    search: Option<Search>,
}

impl HintSession {
    /// Labels the elements an action of `kind` targets (others are dropped).
    ///
    /// Scroll mode labels the scrollable containers ([`Role::ScrollArea`]), so scrolling acts on
    /// the area rather than on a control inside it; when the backend reports no scroll area the
    /// clickable elements are labelled instead. Every other kind labels the clickable elements.
    pub fn new(alphabet: &Alphabet, elements: Vec<Element>, kind: ActionKind) -> Self {
        let targets = if kind == ActionKind::Scroll {
            let (areas, rest): (Vec<Element>, Vec<Element>) = elements
                .into_iter()
                .partition(|e| e.role == Role::ScrollArea && e.enabled && !e.bounds.is_empty());
            if areas.is_empty() {
                rest.into_iter().filter(Element::is_clickable).collect()
            } else {
                areas
            }
        } else {
            elements.into_iter().filter(Element::is_clickable).collect()
        };
        let mut session = Self::empty(alphabet, kind, None);
        session.set_hints(assign_hints(alphabet, targets));
        session
    }

    /// Starts search-by-text over the named `elements` (interactive ones and static text): no
    /// label is shown until a query is typed.
    pub fn search(alphabet: &Alphabet, elements: Vec<Element>, kind: ActionKind) -> Self {
        let candidates = elements.into_iter().filter(is_searchable).collect();
        Self::empty(
            alphabet,
            kind,
            Some(Search {
                candidates,
                query: String::new(),
                picking: false,
            }),
        )
    }

    fn empty(alphabet: &Alphabet, kind: ActionKind, search: Option<Search>) -> Self {
        Self {
            alphabet: alphabet.clone(),
            hints: Vec::new(),
            filter: LabelFilter::new(Vec::new()),
            kind,
            drag_from: None,
            modifiers: Modifiers::NONE,
            search,
        }
    }

    fn set_hints(&mut self, hints: Vec<Hint>) {
        self.filter = LabelFilter::new(hints.iter().map(|hint| hint.label.clone()).collect());
        self.hints = hints;
    }

    /// Relabels the search candidates matching the query, best match first.
    fn rematch(&mut self) {
        let Some(search) = &self.search else {
            return;
        };
        let matches = rank(&search.candidates, &search.query);
        let labels = generate_labels(&self.alphabet, matches.len());
        let hints = labels
            .into_iter()
            .zip(matches)
            .map(|(label, element)| Hint {
                label,
                element: element.clone(),
            })
            .collect();
        self.set_hints(hints);
    }

    /// All hints of the session: in label order, or best match first in search mode.
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

    /// The label prefix typed so far.
    pub fn typed(&self) -> &str {
        self.filter.typed()
    }

    /// The search query, or `None` outside search mode.
    pub fn query(&self) -> Option<&str> {
        self.search.as_ref().map(|search| search.query.as_str())
    }

    /// The modifiers the next click will hold.
    pub const fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Whether there is nothing to label (a search session is never empty while it has
    /// candidates, even before a query is typed).
    pub fn is_empty(&self) -> bool {
        match &self.search {
            Some(search) => search.candidates.is_empty(),
            None => self.hints.is_empty(),
        }
    }

    /// Applies one key press.
    pub fn key(&mut self, key: KeyPress) -> SessionEvent {
        if key == KeyPress::Escape {
            return SessionEvent::Cancelled;
        }
        match self.search.as_ref().map(|search| search.picking) {
            Some(false) => self.query_key(key),
            Some(true) => self.pick_key(key),
            None => self.label_key(key),
        }
    }

    /// A key while the search query is being edited.
    fn query_key(&mut self, key: KeyPress) -> SessionEvent {
        match key {
            KeyPress::Enter => return self.pick_best(),
            KeyPress::Tab => return self.set_picking(!self.hints.is_empty()),
            _ => {}
        }
        let Some(search) = self.search.as_mut() else {
            return SessionEvent::Rejected;
        };
        match key {
            KeyPress::Char(c) => search.query.push(c),
            KeyPress::Backspace if search.query.pop().is_some() => {}
            _ => return SessionEvent::Rejected,
        }
        self.rematch();
        SessionEvent::Narrowed
    }

    /// Switches search mode between editing the query and picking labels; `Rejected` when
    /// `picking` is false and nothing changes (Tab with no match).
    fn set_picking(&mut self, picking: bool) -> SessionEvent {
        match self.search.as_mut() {
            Some(search) if search.picking != picking => {
                search.picking = picking;
                SessionEvent::Narrowed
            }
            _ => SessionEvent::Rejected,
        }
    }

    /// A key after Tab in search mode: labels pick, Backspace past the prefix edits the query.
    fn pick_key(&mut self, key: KeyPress) -> SessionEvent {
        match key {
            KeyPress::Backspace if self.filter.typed().is_empty() => self.set_picking(false),
            KeyPress::Enter if self.filter.typed().is_empty() => self.pick_best(),
            other => self.label_key(other),
        }
    }

    fn pick_best(&mut self) -> SessionEvent {
        match self.hints.first() {
            Some(hint) => self.select(hint.element.target()),
            None => SessionEvent::Rejected,
        }
    }

    /// A key in normal hint mode (or picking a search match).
    fn label_key(&mut self, key: KeyPress) -> SessionEvent {
        match key {
            KeyPress::Backspace => {
                if self.filter.pop() {
                    SessionEvent::Narrowed
                } else {
                    SessionEvent::Rejected
                }
            }
            KeyPress::Char(c @ '1'..='4') => self.toggle_modifier(c),
            KeyPress::Char(c) => match self.filter.push(c) {
                FilterState::Pending => SessionEvent::Narrowed,
                FilterState::Rejected => SessionEvent::Rejected,
                FilterState::Matched(index) => self.select(self.hints[index].element.target()),
            },
            KeyPress::Escape
            | KeyPress::Enter
            | KeyPress::Tab
            | KeyPress::Up
            | KeyPress::Down
            | KeyPress::Left
            | KeyPress::Right => SessionEvent::Rejected,
        }
    }

    fn toggle_modifier(&mut self, key: char) -> SessionEvent {
        if !self.kind.is_click() {
            return SessionEvent::Rejected;
        }
        let m = &mut self.modifiers;
        match key {
            '1' => m.shift = !m.shift,
            '2' => m.ctrl = !m.ctrl,
            '3' => m.alt = !m.alt,
            _ => m.meta = !m.meta,
        }
        SessionEvent::ModifiersChanged(self.modifiers)
    }

    fn select(&mut self, point: Point) -> SessionEvent {
        self.filter.reset();
        if let Some(action) = self.kind.at(point) {
            return SessionEvent::Perform(action.with_modifiers(self.modifiers));
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

    fn element(id: u64, role: Role, name: &str, y: i32) -> Element {
        Element {
            id: ElementId(id),
            role,
            name: name.to_owned(),
            bounds: Rect::new(0, y, 10, 10),
            enabled: true,
        }
    }

    fn page() -> Vec<Element> {
        vec![
            element(1, Role::Link, "Profile", 0),
            element(2, Role::Button, "Open file", 100),
            element(3, Role::Text, "File", 200),
            element(4, Role::Button, "Help", 300),
            element(5, Role::ScrollArea, "Files list", 400),
            element(6, Role::Button, "", 500),
        ]
    }

    fn type_query(session: &mut HintSession, query: &str) {
        for c in query.chars() {
            assert_eq!(session.key(KeyPress::Char(c)), SessionEvent::Narrowed);
        }
    }

    #[test]
    fn scroll_mode_labels_scroll_areas_only() {
        let mut session = HintSession::new(&Alphabet::default(), page(), ActionKind::Scroll);
        assert_eq!(session.hints().len(), 1);
        assert_eq!(session.hints()[0].element.id, ElementId(5));
        assert_eq!(
            session.key(KeyPress::Char('a')),
            SessionEvent::ScrollTarget(Point::new(5, 405))
        );
        let clicks = HintSession::new(&Alphabet::default(), page(), ActionKind::Click);
        assert!(clicks.hints().iter().all(|h| h.element.id != ElementId(5)));
    }

    #[test]
    fn search_filters_by_name_and_enter_clicks_the_best_match() {
        let mut session = HintSession::search(&Alphabet::default(), page(), ActionKind::Click);
        assert!(!session.is_empty());
        assert_eq!(session.query(), Some(""));
        assert_eq!(session.visible(), Vec::<Hint>::new());
        assert_eq!(session.key(KeyPress::Enter), SessionEvent::Rejected);
        type_query(&mut session, "FIL");
        assert_eq!(session.query(), Some("FIL"));
        let ids: Vec<u64> = session.visible().iter().map(|h| h.element.id.0).collect();
        assert_eq!(ids, vec![3, 2, 1]);
        assert_eq!(session.visible()[0].label, "a");
        assert_eq!(session.key(KeyPress::Backspace), SessionEvent::Narrowed);
        assert_eq!(session.query(), Some("FI"));
        type_query(&mut session, "l");
        assert_eq!(
            session.key(KeyPress::Enter),
            SessionEvent::Perform(Action::click(Point::new(5, 205)))
        );
    }

    #[test]
    fn search_tab_then_label_picks_another_match_with_modifiers() {
        let mut session = HintSession::search(&Alphabet::default(), page(), ActionKind::Click);
        assert_eq!(session.key(KeyPress::Tab), SessionEvent::Rejected);
        type_query(&mut session, "file");
        assert_eq!(session.key(KeyPress::Tab), SessionEvent::Narrowed);
        // Backspace with no label typed goes back to the query; Tab returns to picking.
        assert_eq!(session.key(KeyPress::Backspace), SessionEvent::Narrowed);
        assert_eq!(session.query(), Some("file"));
        assert_eq!(session.key(KeyPress::Tab), SessionEvent::Narrowed);
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        assert_eq!(
            session.key(KeyPress::Char('2')),
            SessionEvent::ModifiersChanged(ctrl)
        );
        assert_eq!(session.key(KeyPress::Char('z')), SessionEvent::Rejected);
        assert_eq!(
            session.key(KeyPress::Char('s')),
            SessionEvent::Perform(Action::click(Point::new(5, 105)).with_modifiers(ctrl))
        );
    }

    #[test]
    fn search_escape_cancels_and_empty_backspace_is_rejected() {
        let mut session = HintSession::search(&Alphabet::default(), page(), ActionKind::Click);
        assert_eq!(session.key(KeyPress::Backspace), SessionEvent::Rejected);
        type_query(&mut session, "zzz");
        assert_eq!(session.visible(), Vec::<Hint>::new());
        assert_eq!(session.key(KeyPress::Enter), SessionEvent::Rejected);
        assert_eq!(session.key(KeyPress::Escape), SessionEvent::Cancelled);
        let nothing = HintSession::search(&Alphabet::default(), Vec::new(), ActionKind::Click);
        assert!(nothing.is_empty());
    }

    #[test]
    fn modifier_keys_toggle_for_clicks_only() {
        let mut session = HintSession::new(&Alphabet::default(), elements(), ActionKind::Click);
        let all = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            meta: true,
        };
        for key in ['1', '2', '3', '4'] {
            session.key(KeyPress::Char(key));
        }
        assert_eq!(session.modifiers(), all);
        assert_eq!(
            session.key(KeyPress::Char('3')),
            SessionEvent::ModifiersChanged(Modifiers { alt: false, ..all })
        );
        assert_eq!(
            session.key(KeyPress::Char('a')),
            SessionEvent::Perform(
                Action::click(Point::new(5, 5)).with_modifiers(Modifiers { alt: false, ..all })
            )
        );
        let mut drag = HintSession::new(&Alphabet::default(), elements(), ActionKind::Drag);
        assert_eq!(drag.key(KeyPress::Char('2')), SessionEvent::Rejected);
        assert_eq!(drag.modifiers(), Modifiers::NONE);
    }
}
