//! One activation of hint mode: label the elements, follow typed keys, produce the action.
//!
//! Keys while labels are shown:
//!
//! - label characters narrow the labels and pick one;
//! - Backspace removes the last typed character, Escape cancels;
//! - for click actions, `1` toggles Shift, `2` Ctrl, `3` Alt and `4` Super (Command on macOS,
//!   the Windows key on Windows), held around the click (cmd/ctrl+click opens a link in a new
//!   tab); labels never contain digits, so these keys never clash with a label;
//! - focus-then-act: one hint is highlighted (the first in reading order to begin with);
//!   Tab, Down and Ctrl-N move the highlight forward, Shift-Tab, Up and Ctrl-P back (wrapping),
//!   and Return acts on it. Modifiers held with Return are held around the click.
//!
//! With the Homerow bindings ([`KeyBindings::Homerow`]) a typed label highlights its hint
//! instead of acting; then Return clicks, Return twice double clicks, Shift-Return right clicks
//! and Cmd-Return (Super) cmd-clicks the highlighted hint.
//!
//! In search mode ([`HintSession::search`]) typed characters first build the query: labels are
//! shown only on elements whose text (name, value, description or role) fuzzily matches it,
//! best match first (and highlighted); spaces are ignored, and `*` or `;` alone matches all.
//! Enter acts on the highlighted match; Tab freezes the query so label characters (and the
//! modifier digits) pick another match; Backspace with no label typed returns to editing the
//! query.

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionKind, MouseButton};
use crate::element::{Element, Point, Role};
use crate::filter::{FilterState, LabelFilter};
use crate::hints::{Alphabet, Hint, assign_hints, generate_labels};
use crate::hotkey::{KeyEvent, KeyPress, Modifiers};
use crate::search::{is_searchable, rank};

/// Which keys act on a hint (`hints.keys` in the config).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyBindings {
    /// Typing a label acts at once; Return acts on the highlighted hint.
    #[default]
    Nomouse,
    /// Homerow's: typing a label highlights it; Return clicks, Return twice double clicks,
    /// Shift-Return right clicks, Cmd-Return cmd-clicks.
    Homerow,
}

/// What a key press did to the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    /// The prefix or the search query changed (or a character was removed); redraw the hints.
    Narrowed,
    /// The highlight moved to another hint; redraw.
    Highlighted,
    /// The key matched nothing and was ignored.
    Rejected,
    /// A modifier key toggled; the next click holds these modifiers.
    ModifiersChanged(Modifiers),
    /// Drag mode: the start point was picked; hints are shown again for the end point.
    DragStarted(Point),
    /// Scroll mode: the scroll target was picked; the caller enters scroll mode there.
    ScrollTarget(Point),
    /// Homerow bindings: Return was pressed once. A second Return turns this action into a
    /// double click; any other key, or [`HintSession::expire`] once the double-press time
    /// has passed, performs it as it is.
    Armed(Action),
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
    bindings: KeyBindings,
    drag_from: Option<Point>,
    modifiers: Modifiers,
    search: Option<Search>,
    /// Index into `hints` of the highlighted hint; always a visible one, `None` only when
    /// nothing is visible.
    focus: Option<usize>,
    /// Homerow bindings: the click a second Return would turn into a double click.
    armed: Option<Action>,
}

/// How a key moves the highlight: `Some(true)` forward (Tab, Down, Ctrl-N), `Some(false)`
/// back (Shift-Tab, Up, Ctrl-P).
fn focus_step(key: KeyPress, modifiers: Modifiers) -> Option<bool> {
    let chord = |press: KeyPress, held: Modifiers| key == press && modifiers == held;
    let none = Modifiers::NONE;
    if chord(KeyPress::Tab, none)
        || chord(KeyPress::Down, none)
        || chord(KeyPress::Char('n'), Modifiers::CTRL)
    {
        Some(true)
    } else if chord(KeyPress::Tab, Modifiers::SHIFT)
        || chord(KeyPress::Up, none)
        || chord(KeyPress::Char('p'), Modifiers::CTRL)
    {
        Some(false)
    } else {
        None
    }
}

/// Whether a character key types text: no modifier but Shift is held.
const fn types_text(modifiers: Modifiers) -> bool {
    !(modifiers.ctrl || modifiers.alt || modifiers.meta)
}

impl HintSession {
    /// Labels the elements an action of `kind` targets (others are dropped).
    ///
    /// Scroll mode labels the scrollable containers ([`Role::ScrollArea`]), so scrolling acts on
    /// the area rather than on a control inside it; when the backend reports no scroll area the
    /// clickable elements are labelled instead. Every other kind labels the clickable elements.
    /// Elements scrolled out of view ([`Element::offscreen`]) get no label: it could not be seen.
    pub fn new(alphabet: &Alphabet, elements: Vec<Element>, kind: ActionKind) -> Self {
        let visible = elements.into_iter().filter(|e| !e.offscreen);
        let targets = if kind == ActionKind::Scroll {
            let (areas, rest): (Vec<Element>, Vec<Element>) = visible
                .partition(|e| e.role == Role::ScrollArea && e.enabled && !e.bounds.is_empty());
            if areas.is_empty() {
                rest.into_iter().filter(Element::is_clickable).collect()
            } else {
                areas
            }
        } else {
            visible.filter(Element::is_clickable).collect()
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

    /// The same session with `bindings` deciding what a typed label and Return do.
    #[must_use]
    pub const fn with_bindings(mut self, bindings: KeyBindings) -> Self {
        self.bindings = bindings;
        self
    }

    fn empty(alphabet: &Alphabet, kind: ActionKind, search: Option<Search>) -> Self {
        Self {
            alphabet: alphabet.clone(),
            hints: Vec::new(),
            filter: LabelFilter::new(Vec::new()),
            kind,
            bindings: KeyBindings::default(),
            drag_from: None,
            modifiers: Modifiers::NONE,
            search,
            focus: None,
            armed: None,
        }
    }

    fn set_hints(&mut self, hints: Vec<Hint>) {
        self.filter = LabelFilter::new(hints.iter().map(|hint| hint.label.clone()).collect());
        self.hints = hints;
        self.focus = None;
        self.refocus();
    }

    /// Keeps the highlight on a visible hint: the current one while it is visible, else the
    /// first visible one.
    fn refocus(&mut self) {
        let mut visible = self.filter.matching();
        let first = visible.next();
        if let Some(focus) = self.focus
            && (first == Some(focus) || visible.any(|index| index == focus))
        {
            return;
        }
        self.focus = first;
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

    /// The highlighted hint, which Return acts on; always one of [`HintSession::visible`].
    pub fn focused(&self) -> Option<&Hint> {
        self.focus.and_then(|index| self.hints.get(index))
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

    /// Whether `action` (just performed from this session) left-clicked a text field, where
    /// continuous mode ends so the keyboard stays with the app for typing.
    pub fn types_into(&self, action: &Action) -> bool {
        let Action::Click {
            at,
            button: MouseButton::Left,
            ..
        } = *action
        else {
            return false;
        };
        self.hints
            .iter()
            .find(|hint| hint.element.target() == at)
            .is_some_and(|hint| hint.element.role.takes_text())
    }

    /// Whether there is nothing to label (a search session is never empty while it has
    /// candidates, even before a query is typed).
    pub fn is_empty(&self) -> bool {
        match &self.search {
            Some(search) => search.candidates.is_empty(),
            None => self.hints.is_empty(),
        }
    }

    /// The action of a pending [`SessionEvent::Armed`] once the double-press time has passed
    /// without a second Return; `None` when nothing is armed.
    pub const fn expire(&mut self) -> Option<Action> {
        self.armed.take()
    }

    /// Applies one key press (a bare [`KeyPress`] is a press without modifiers).
    pub fn key(&mut self, event: impl Into<KeyEvent>) -> SessionEvent {
        let KeyEvent { key, modifiers } = event.into();
        if key == KeyPress::Escape {
            self.armed = None;
            return SessionEvent::Cancelled;
        }
        if let Some(armed) = self.armed.take() {
            if key == KeyPress::Enter && modifiers.is_empty() {
                let double = Action::double_click(armed.end_point());
                return SessionEvent::Perform(double.with_modifiers(armed.modifiers()));
            }
            return SessionEvent::Perform(armed);
        }
        match self.search.as_ref().map(|search| search.picking) {
            Some(false) => self.query_key(key, modifiers),
            Some(true) => self.pick_key(key, modifiers),
            None => self.label_key(key, modifiers),
        }
    }

    /// A key while the search query is being edited.
    fn query_key(&mut self, key: KeyPress, modifiers: Modifiers) -> SessionEvent {
        match key {
            KeyPress::Enter => return self.enter(modifiers),
            KeyPress::Tab if modifiers.is_empty() => {
                return self.set_picking(!self.hints.is_empty());
            }
            _ => {}
        }
        if let Some(forward) = focus_step(key, modifiers) {
            return self.move_focus(forward);
        }
        let Some(search) = self.search.as_mut() else {
            return SessionEvent::Rejected;
        };
        match key {
            KeyPress::Char(c) if types_text(modifiers) => search.query.push(c),
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
    fn pick_key(&mut self, key: KeyPress, modifiers: Modifiers) -> SessionEvent {
        match key {
            KeyPress::Backspace if self.filter.typed().is_empty() => self.set_picking(false),
            other => self.label_key(other, modifiers),
        }
    }

    /// A key in normal hint mode (or picking a search match).
    fn label_key(&mut self, key: KeyPress, modifiers: Modifiers) -> SessionEvent {
        if let Some(forward) = focus_step(key, modifiers) {
            return self.move_focus(forward);
        }
        match key {
            KeyPress::Enter => self.enter(modifiers),
            KeyPress::Backspace => {
                if self.filter.pop() {
                    self.refocus();
                    SessionEvent::Narrowed
                } else {
                    SessionEvent::Rejected
                }
            }
            KeyPress::Char(_) if !types_text(modifiers) => SessionEvent::Rejected,
            KeyPress::Char(c @ '1'..='4') => self.toggle_modifier(c),
            KeyPress::Char(c) => match self.filter.push(c) {
                FilterState::Pending => {
                    self.refocus();
                    SessionEvent::Narrowed
                }
                FilterState::Rejected => SessionEvent::Rejected,
                FilterState::Matched(index) => match self.bindings {
                    KeyBindings::Nomouse => {
                        self.select(self.hints[index].element.target(), Modifiers::NONE)
                    }
                    KeyBindings::Homerow => {
                        self.filter.reset();
                        self.focus = Some(index);
                        SessionEvent::Highlighted
                    }
                },
            },
            KeyPress::Escape
            | KeyPress::Tab
            | KeyPress::Up
            | KeyPress::Down
            | KeyPress::Left
            | KeyPress::Right => SessionEvent::Rejected,
        }
    }

    /// Moves the highlight to the next (`forward`) or previous visible hint, wrapping.
    fn move_focus(&mut self, forward: bool) -> SessionEvent {
        let visible: Vec<usize> = self.filter.matching().collect();
        let count = visible.len();
        if count == 0 {
            return SessionEvent::Rejected;
        }
        let at = self
            .focus
            .and_then(|focus| visible.iter().position(|index| *index == focus))
            .unwrap_or(0);
        let next = if forward {
            (at + 1) % count
        } else {
            (at + count - 1) % count
        };
        self.focus = Some(visible[next]);
        SessionEvent::Highlighted
    }

    /// Return: acts on the highlighted hint, holding `held` (the modifiers pressed with
    /// Return) besides the toggled ones.
    fn enter(&mut self, held: Modifiers) -> SessionEvent {
        let Some(point) = self.focused().map(|hint| hint.element.target()) else {
            return SessionEvent::Rejected;
        };
        if self.bindings == KeyBindings::Nomouse || !self.kind.is_click() {
            return self.select(point, held);
        }
        let others = Modifiers {
            shift: false,
            ..held
        };
        let modifiers = self.modifiers.union(others);
        self.filter.reset();
        if held.shift {
            return SessionEvent::Perform(Action::right_click(point).with_modifiers(modifiers));
        }
        let Some(action) = self.kind.at(point) else {
            return SessionEvent::Rejected;
        };
        let action = action.with_modifiers(modifiers);
        if held.is_empty()
            && matches!(
                action,
                Action::Click {
                    button: MouseButton::Left,
                    count: 1,
                    ..
                }
            )
        {
            self.armed = Some(action);
            return SessionEvent::Armed(action);
        }
        SessionEvent::Perform(action)
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

    fn select(&mut self, point: Point, held: Modifiers) -> SessionEvent {
        self.filter.reset();
        self.refocus();
        if let Some(action) = self.kind.at(point) {
            return SessionEvent::Perform(action.with_modifiers(self.modifiers.union(held)));
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
                description: String::new(),
                value: String::new(),
                bounds: Rect::new(0, i32::try_from(i).unwrap() * 100, 10, 10),
                enabled: true,
                offscreen: false,
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
    fn a_left_click_on_a_text_field_types_into_it() {
        let mut fields = elements();
        fields[1].role = Role::TextField;
        let mut session = HintSession::new(&Alphabet::default(), fields, ActionKind::Click);
        let SessionEvent::Perform(action) = session.key(KeyPress::Char('s')) else {
            panic!("no click");
        };
        assert!(session.types_into(&action));
        // The button above, a right click on the field, and a hover over it do not.
        assert!(!session.types_into(&Action::click(Point::new(5, 5))));
        assert!(!session.types_into(&Action::right_click(Point::new(5, 105))));
        assert!(!session.types_into(&Action::hover(Point::new(5, 105))));
        // A ctrl-click or a double click still focuses the field.
        assert!(session.types_into(&Action::double_click(Point::new(5, 105))));
        assert!(session.types_into(&action.with_modifiers(Modifiers::CTRL)));
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
                description: String::new(),
                value: String::new(),
                bounds: Rect::new(i32::try_from(i).unwrap() * 20, 0, 10, 10),
                enabled: true,
                offscreen: false,
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
        // Return acts on the highlighted hint (narrowing moved it into the typed group).
        let focused = session.focused().unwrap().element.target();
        assert_eq!(
            session.key(KeyPress::Enter),
            SessionEvent::ScrollTarget(focused)
        );
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
            description: String::new(),
            value: String::new(),
            bounds: Rect::new(0, y, 10, 10),
            enabled: true,
            offscreen: false,
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
    fn offscreen_elements_are_searched_but_not_labelled() {
        // Two "Next" links: one scrolled above the viewport (exhaustive browser collection),
        // one visible further down; plus an off-viewport "Archive" link and a scroll area.
        let mut above = element(7, Role::Link, "Next", -900);
        above.offscreen = true;
        let mut archive = element(8, Role::Link, "Archive", 4000);
        archive.offscreen = true;
        let mut hidden_area = element(9, Role::ScrollArea, "Comments", 3000);
        hidden_area.offscreen = true;
        let mut elements = page();
        elements.extend([
            above,
            element(10, Role::Link, "Next", 600),
            archive,
            hidden_area,
        ]);

        let clicks = HintSession::new(&Alphabet::default(), elements.clone(), ActionKind::Click);
        let ids: Vec<u64> = clicks.hints().iter().map(|h| h.element.id.0).collect();
        assert_eq!(ids, vec![1, 2, 4, 6, 10]);
        let scroll = HintSession::new(&Alphabet::default(), elements.clone(), ActionKind::Scroll);
        assert_eq!(scroll.hints().len(), 1);
        assert_eq!(scroll.hints()[0].element.id, ElementId(5));

        // Search reaches the off-viewport link, and a visible match beats an off-viewport one
        // with the same score even though the latter is higher up the page.
        let mut search =
            HintSession::search(&Alphabet::default(), elements.clone(), ActionKind::Click);
        type_query(&mut search, "archive");
        assert_eq!(
            search.key(KeyPress::Enter),
            SessionEvent::Perform(Action::click(Point::new(5, 4005)))
        );
        let mut search = HintSession::search(&Alphabet::default(), elements, ActionKind::Click);
        type_query(&mut search, "next");
        let ids: Vec<u64> = search.visible().iter().map(|h| h.element.id.0).collect();
        assert_eq!(ids, vec![10, 7]);
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

    fn row(count: u64) -> Vec<Element> {
        (0..count)
            .map(|i| element(i, Role::Button, "", i32::try_from(i).unwrap() * 100))
            .collect()
    }

    fn focused_id(session: &HintSession) -> Option<u64> {
        session.focused().map(|hint| hint.element.id.0)
    }

    #[test]
    fn highlight_starts_first_and_moves_with_tab_arrows_and_ctrl_n_p() {
        let mut session = HintSession::new(&Alphabet::default(), row(3), ActionKind::Click);
        assert_eq!(focused_id(&session), Some(0));
        let forward = [
            KeyPress::Tab.into(),
            KeyPress::Down.into(),
            KeyPress::Char('n').with(Modifiers::CTRL),
        ];
        for (key, expected) in forward.into_iter().zip([1, 2, 0]) {
            assert_eq!(session.key(key), SessionEvent::Highlighted);
            assert_eq!(focused_id(&session), Some(expected));
        }
        let back = [
            KeyPress::Tab.with(Modifiers::SHIFT),
            KeyPress::Up.into(),
            KeyPress::Char('p').with(Modifiers::CTRL),
        ];
        for (key, expected) in back.into_iter().zip([2, 1, 0]) {
            assert_eq!(session.key(key), SessionEvent::Highlighted);
            assert_eq!(focused_id(&session), Some(expected));
        }
        // Ctrl with any other letter neither moves the highlight nor types a label.
        assert_eq!(
            session.key(KeyPress::Char('a').with(Modifiers::CTRL)),
            SessionEvent::Rejected
        );
        assert_eq!(session.typed(), "");
    }

    #[test]
    fn tab_twice_then_return_clicks_the_third_hint() {
        let mut session = HintSession::new(&Alphabet::default(), row(4), ActionKind::Click);
        session.key(KeyPress::Tab);
        session.key(KeyPress::Tab);
        assert_eq!(
            session.key(KeyPress::Enter),
            SessionEvent::Perform(Action::click(Point::new(5, 205)))
        );
    }

    #[test]
    fn return_holds_its_modifiers_and_the_toggled_ones() {
        let mut session = HintSession::new(&Alphabet::default(), row(2), ActionKind::Click);
        session.key(KeyPress::Down);
        session.key(KeyPress::Char('1'));
        let held = Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::NONE
        };
        assert_eq!(
            session.key(KeyPress::Enter.with(Modifiers::CTRL)),
            SessionEvent::Perform(Action::click(Point::new(5, 105)).with_modifiers(held))
        );
    }

    #[test]
    fn narrowing_keeps_the_highlight_on_a_visible_hint() {
        let mut session = HintSession::new(&Alphabet::default(), row(20), ActionKind::Click);
        let last = session.hints()[19].label.clone();
        let first_char = last.chars().next().unwrap();
        assert_eq!(focused_id(&session), Some(0));
        assert_eq!(
            session.key(KeyPress::Char(first_char)),
            SessionEvent::Narrowed
        );
        let visible = session.visible();
        let focused = session.focused().unwrap().clone();
        assert!(visible.contains(&focused));
        assert_eq!(visible[0], focused);
        // Moving within the narrowed set stays inside it.
        for _ in 0..=visible.len() {
            session.key(KeyPress::Tab);
            assert!(session.visible().contains(session.focused().unwrap()));
        }
    }

    #[test]
    fn homerow_label_highlights_and_return_clicks_after_the_double_press_time() {
        let mut session = HintSession::new(&Alphabet::default(), row(3), ActionKind::Click)
            .with_bindings(KeyBindings::Homerow);
        assert_eq!(session.key(KeyPress::Char('d')), SessionEvent::Highlighted);
        assert_eq!(focused_id(&session), Some(2));
        assert_eq!(session.typed(), "");
        assert_eq!(session.visible().len(), 3);
        let click = Action::click(Point::new(5, 205));
        assert_eq!(session.key(KeyPress::Enter), SessionEvent::Armed(click));
        assert_eq!(session.expire(), Some(click));
        assert_eq!(session.expire(), None);
    }

    #[test]
    fn homerow_return_twice_double_clicks() {
        let mut session = HintSession::new(&Alphabet::default(), row(3), ActionKind::Click)
            .with_bindings(KeyBindings::Homerow);
        session.key(KeyPress::Char('s'));
        assert!(matches!(
            session.key(KeyPress::Enter),
            SessionEvent::Armed(_)
        ));
        assert_eq!(
            session.key(KeyPress::Enter),
            SessionEvent::Perform(Action::double_click(Point::new(5, 105)))
        );
    }

    #[test]
    fn homerow_another_key_after_return_performs_the_single_click() {
        let mut session = HintSession::new(&Alphabet::default(), row(3), ActionKind::Click)
            .with_bindings(KeyBindings::Homerow);
        session.key(KeyPress::Enter);
        assert_eq!(
            session.key(KeyPress::Char('s')),
            SessionEvent::Perform(Action::click(Point::new(5, 5)))
        );
    }

    #[test]
    fn homerow_shift_return_right_clicks_and_cmd_return_cmd_clicks() {
        let homerow = || {
            HintSession::new(&Alphabet::default(), row(3), ActionKind::Click)
                .with_bindings(KeyBindings::Homerow)
        };
        let mut session = homerow();
        session.key(KeyPress::Char('s'));
        assert_eq!(
            session.key(KeyPress::Enter.with(Modifiers::SHIFT)),
            SessionEvent::Perform(Action::right_click(Point::new(5, 105)))
        );
        let mut session = homerow();
        session.key(KeyPress::Char('2'));
        assert_eq!(
            session.key(KeyPress::Enter.with(Modifiers::META)),
            SessionEvent::Perform(Action::click(Point::new(5, 5)).with_modifiers(Modifiers {
                ctrl: true,
                meta: true,
                ..Modifiers::NONE
            }))
        );
    }

    #[test]
    fn homerow_escape_while_armed_cancels() {
        let mut session = HintSession::new(&Alphabet::default(), row(2), ActionKind::Click)
            .with_bindings(KeyBindings::Homerow);
        session.key(KeyPress::Enter);
        assert_eq!(session.key(KeyPress::Escape), SessionEvent::Cancelled);
        assert_eq!(session.expire(), None);
    }

    #[test]
    fn homerow_non_click_kinds_act_on_return_at_once() {
        let mut session = HintSession::new(&Alphabet::default(), row(2), ActionKind::Hover)
            .with_bindings(KeyBindings::Homerow);
        assert_eq!(session.key(KeyPress::Char('s')), SessionEvent::Highlighted);
        assert_eq!(
            session.key(KeyPress::Enter),
            SessionEvent::Perform(Action::hover(Point::new(5, 105)))
        );
    }

    #[test]
    fn search_highlights_the_best_match_and_arrows_move_it() {
        let mut session = HintSession::search(&Alphabet::default(), page(), ActionKind::Click);
        assert_eq!(session.focused(), None);
        assert_eq!(session.key(KeyPress::Down), SessionEvent::Rejected);
        type_query(&mut session, "file");
        assert_eq!(focused_id(&session), Some(3));
        assert_eq!(session.key(KeyPress::Down), SessionEvent::Highlighted);
        assert_eq!(focused_id(&session), Some(2));
        // Ctrl+letter is not query text.
        assert_eq!(
            session.key(KeyPress::Char('x').with(Modifiers::CTRL)),
            SessionEvent::Rejected
        );
        assert_eq!(session.query(), Some("file"));
        assert_eq!(
            session.key(KeyPress::Enter),
            SessionEvent::Perform(Action::click(Point::new(5, 105)))
        );
    }
}
