//! Drives one hint-mode activation against a platform: scan, label, follow keys, act.

use std::time::{Duration, Instant};

use nomouse_core::{
    Action, ActionKind, Alphabet, Frame, Hint, HintSession, InputEvent, KeyBindings, Platform,
    PlatformError, Rect, Scope, ScrollEvent, ScrollSession, SessionEvent,
};

/// How long continuous mode waits after a click before re-scanning, so the app can update.
pub(crate) const SETTLE: Duration = Duration::from_millis(50);

/// How often the session loop wakes up while waiting for keys (and checks for a dismissal
/// request from the command socket).
const POLL: Duration = Duration::from_millis(50);

/// Homerow bindings: how long after Return a second Return makes a double click.
const DOUBLE_PRESS: Duration = Duration::from_millis(300);

/// What one activation does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Pick one hint and perform `ActionKind` on it.
    Once(ActionKind),
    /// Keep re-labelling after every action until Escape.
    Continuous(ActionKind),
    /// Scroll the largest scroll area (or a picked element) with keys until Escape.
    Scroll,
    /// Type to filter elements by name; Enter (or a label after Tab) clicks a match.
    Search,
}

impl Mode {
    const fn kind(self) -> ActionKind {
        match self {
            Self::Once(kind) | Self::Continuous(kind) => kind,
            Self::Scroll => ActionKind::Scroll,
            Self::Search => ActionKind::Click,
        }
    }

    fn session(self, settings: &Settings, elements: Vec<nomouse_core::Element>) -> HintSession {
        let alphabet = &settings.alphabet;
        let session = match self {
            Self::Search => HintSession::search(alphabet, elements, self.kind()),
            Self::Once(_) | Self::Continuous(_) | Self::Scroll => {
                HintSession::new(alphabet, elements, self.kind())
            }
        };
        session.with_bindings(settings.keys)
    }
}

/// Draws the session: its matching labels and highlight, plus the query box in search mode.
fn draw(platform: &mut dyn Platform, session: &HintSession) -> Result<(), PlatformError> {
    let hints: Vec<Hint> = session.visible();
    platform.show_frame(&Frame {
        hints: &hints,
        typed: session.typed(),
        query: session.query(),
        focused: session.focused().map(|hint| hint.label.as_str()),
        area: None,
    })
}

/// Settings the driver needs from the configuration.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    /// Label alphabet.
    pub(crate) alphabet: Alphabet,
    /// Where elements come from.
    pub(crate) scope: Scope,
    /// Wheel steps per scroll key press.
    pub(crate) scroll_step: i32,
    /// Wheel steps per Shift-scroll key press.
    pub(crate) dash_step: i32,
    /// Which keys act on a hint.
    pub(crate) keys: KeyBindings,
    /// `ignore.apps`: apps where activation shows nothing.
    pub(crate) ignore: Vec<String>,
}

/// What waiting for the next key produced.
enum Wait {
    /// A key while the overlay holds the keyboard.
    Key(nomouse_core::KeyEvent),
    /// An activation hotkey or a command-socket request came in: the labels toggle off.
    Dismissed,
    /// Nothing within the timeout.
    Timeout,
}

/// Waits up to `timeout` for a key; a hotkey, or `dismissed()` turning true meanwhile, ends
/// the activation (pressing the activation hotkey again toggles the labels off).
fn wait(
    platform: &mut dyn Platform,
    timeout: Duration,
    dismissed: &dyn Fn() -> bool,
) -> Result<Wait, PlatformError> {
    Ok(match platform.next_event(timeout)? {
        Some(InputEvent::Key(key)) => Wait::Key(key),
        Some(InputEvent::Hotkey(_)) => Wait::Dismissed,
        None if dismissed() => Wait::Dismissed,
        None => Wait::Timeout,
    })
}

/// How an activation ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Actions were performed (the count; continuous mode may perform several).
    Performed(usize),
    /// The user pressed Escape before acting.
    Cancelled,
    /// The scan found nothing to label.
    NoElements,
    /// The focused window belongs to an app in `ignore.apps` (the entry naming it).
    Ignored(String),
}

/// The `ignore.apps` entry naming the focused window's app, if any. A failure to identify the
/// app is logged and ignores nothing.
fn ignored(platform: &mut dyn Platform, ignore: &[String]) -> Option<String> {
    if ignore.is_empty() {
        return None;
    }
    match platform.focused_app() {
        Ok(Some(app)) => {
            let names: Vec<&str> = app.names().collect();
            tracing::debug!("focused app: {}", names.join(", "));
            app.ignored_by(ignore).map(str::to_owned)
        }
        Ok(None) => None,
        Err(error) => {
            tracing::warn!("identifying the focused app: {error}");
            None
        }
    }
}

/// Hides the overlay however the activation ends (errors included).
struct Shown<'a> {
    platform: &'a mut dyn Platform,
}

impl Drop for Shown<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.platform.hide() {
            tracing::warn!("hiding the overlay: {error}");
        }
    }
}

/// Draws scroll mode: the active area outlined, every area labelled with its digit.
fn draw_scroll(platform: &mut dyn Platform, scroll: &ScrollSession) -> Result<(), PlatformError> {
    let hints = scroll.hints();
    platform.show_frame(&Frame {
        hints: &hints,
        focused: scroll.active_label(),
        area: Some(scroll.active()),
        ..Frame::default()
    })
}

/// Scroll mode: the pointer rests on the active area (wheel events go where the pointer is)
/// and the keys scroll it, until Escape or Enter. Returns the scrolls performed.
fn scroll_mode(
    platform: &mut dyn Platform,
    mut scroll: ScrollSession,
    dismissed: &dyn Fn() -> bool,
) -> Result<usize, PlatformError> {
    platform.perform(&Action::hover(scroll.active().center()))?;
    // The overlay stays up: it holds the keyboard for the scroll keys.
    draw_scroll(platform, &scroll)?;
    let mut scrolls = 0;
    loop {
        let key = match wait(platform, POLL, dismissed)? {
            Wait::Key(key) => key,
            Wait::Dismissed => return Ok(scrolls),
            Wait::Timeout => continue,
        };
        match scroll.key(key) {
            ScrollEvent::Done => return Ok(scrolls),
            ScrollEvent::Ignored => {}
            ScrollEvent::Switched => {
                platform.perform(&Action::hover(scroll.active().center()))?;
                draw_scroll(platform, &scroll)?;
            }
            ScrollEvent::Scroll { dx, dy } => {
                platform.perform(&Action::scroll(scroll.active().center(), dx, dy))?;
                scrolls += 1;
            }
        }
    }
}

/// Logs the labels of a scan (debug) and, for the first, how long labelling took (info).
fn log_scan(hints: &[Hint], scans: usize, started: Instant) {
    tracing::debug!("scan {scans}: {} hints", hints.len());
    for hint in hints {
        let element = &hint.element;
        tracing::debug!(
            "hint {} = {:?} {:?}",
            hint.label,
            element.role,
            element.name
        );
    }
    if scans == 1 {
        tracing::info!(
            "labels shown {} ms after the trigger ({} hints)",
            started.elapsed().as_millis(),
            hints.len()
        );
    }
}

/// The outcome of an activation that ends without a new action after `performed` ones.
const fn ended(performed: usize) -> Outcome {
    if performed > 0 {
        Outcome::Performed(performed)
    } else {
        Outcome::Cancelled
    }
}

/// Runs one activation in `mode`. `started` is when the trigger arrived, for the
/// hotkey-to-labels timing logged at info level. `dismissed` reports a request to toggle the
/// labels off that arrives outside the platform's events (the daemon's command socket). When
/// the focused window's app is in `ignore.apps`, nothing is scanned or shown.
///
/// # Errors
/// Platform errors from scanning, drawing or acting; the overlay is hidden either way.
pub(crate) fn activate(
    platform: &mut dyn Platform,
    settings: &Settings,
    mode: Mode,
    started: Instant,
    dismissed: &dyn Fn() -> bool,
) -> Result<Outcome, PlatformError> {
    if let Some(pattern) = ignored(platform, &settings.ignore) {
        return Ok(Outcome::Ignored(pattern));
    }
    let guard = Shown { platform };
    let platform = &mut *guard.platform;
    let mut performed = 0;
    let mut scans = 0_usize;
    loop {
        let elements = platform.elements(settings.scope)?;
        if mode == Mode::Scroll {
            let areas = ScrollSession::areas_of(&elements);
            if let Some(scroll) =
                ScrollSession::new(areas, settings.scroll_step, settings.dash_step)
            {
                log_scan(&scroll.hints(), 1, started);
                return Ok(Outcome::Performed(scroll_mode(
                    platform, scroll, dismissed,
                )?));
            }
        }
        let mut session = mode.session(settings, elements);
        if session.is_empty() {
            return Ok(if performed > 0 {
                Outcome::Performed(performed)
            } else {
                Outcome::NoElements
            });
        }
        draw(platform, &session)?;
        scans += 1;
        log_scan(session.hints(), scans, started);
        let action = loop {
            let key = match wait(platform, POLL, dismissed)? {
                Wait::Key(key) => key,
                Wait::Dismissed => return Ok(ended(performed)),
                Wait::Timeout => continue,
            };
            match session.key(key) {
                SessionEvent::Rejected => {}
                SessionEvent::Narrowed
                | SessionEvent::Highlighted
                | SessionEvent::DragStarted(_) => draw(platform, &session)?,
                SessionEvent::ModifiersChanged(modifiers) => {
                    tracing::debug!("next click holds {modifiers:?}");
                }
                SessionEvent::Cancelled => return Ok(ended(performed)),
                SessionEvent::ScrollTarget(at) => {
                    // No scroll area: the picked element is the one to scroll.
                    let bounds = session
                        .hints()
                        .iter()
                        .map(|hint| hint.element.bounds)
                        .find(|bounds| bounds.center() == at)
                        .unwrap_or_else(|| Rect::new(at.x, at.y, 1, 1));
                    let step = (settings.scroll_step, settings.dash_step);
                    let scrolls = ScrollSession::new(vec![bounds], step.0, step.1)
                        .map_or(Ok(0), |scroll| scroll_mode(platform, scroll, dismissed))?;
                    return Ok(Outcome::Performed(performed + scrolls));
                }
                SessionEvent::Armed(single) => match wait(platform, DOUBLE_PRESS, dismissed)? {
                    Wait::Key(key) => match session.key(key) {
                        SessionEvent::Perform(action) => break action,
                        _ => return Ok(ended(performed)),
                    },
                    Wait::Dismissed => return Ok(ended(performed)),
                    Wait::Timeout => break session.expire().unwrap_or(single),
                },
                SessionEvent::Perform(action) => break action,
            }
        };
        platform.hide()?;
        platform.perform(&action)?;
        performed += 1;
        if !matches!(mode, Mode::Continuous(_)) {
            return Ok(Outcome::Performed(performed));
        }
        if session.types_into(&action) {
            tracing::debug!("continuous mode ends at a text field");
            return Ok(Outcome::Performed(performed));
        }
        std::thread::sleep(SETTLE);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use nomouse_core::{
        AppIdentity, Element, ElementId, ElementSource, Hint, Hotkey, HotkeyId, HotkeySource,
        InputInjector, KeyPress, Modifiers, Overlay, Point, Rect, Role,
    };

    use super::*;

    #[derive(Default)]
    struct Fake {
        elements: Vec<Element>,
        /// The script: `None` is a poll that times out.
        keys: VecDeque<Option<InputEvent>>,
        performed: Vec<Action>,
        scans: usize,
        shown: Vec<(usize, String)>,
        visible: bool,
        queries: Vec<String>,
        focused: Vec<Option<String>>,
        /// The outlined scroll area of each frame.
        outlined: Vec<Option<Rect>>,
        /// The focused window's app.
        app: Option<AppIdentity>,
    }

    impl ElementSource for Fake {
        fn elements(&mut self, _scope: Scope) -> Result<Vec<Element>, PlatformError> {
            self.scans += 1;
            Ok(self.elements.clone())
        }

        fn focused_app(&mut self) -> Result<Option<AppIdentity>, PlatformError> {
            Ok(self.app.clone())
        }
    }

    impl Overlay for Fake {
        fn show(&mut self, hints: &[Hint], typed: &str) -> Result<(), PlatformError> {
            self.shown.push((hints.len(), typed.to_owned()));
            self.visible = true;
            Ok(())
        }

        fn show_frame(&mut self, frame: &Frame<'_>) -> Result<(), PlatformError> {
            if let Some(query) = frame.query {
                self.queries.push(query.to_owned());
            }
            self.focused.push(frame.focused.map(str::to_owned));
            self.outlined.push(frame.area);
            self.show(frame.hints, frame.typed)
        }

        fn hide(&mut self) -> Result<(), PlatformError> {
            self.visible = false;
            Ok(())
        }
    }

    impl InputInjector for Fake {
        fn perform(&mut self, action: &Action) -> Result<(), PlatformError> {
            self.performed.push(*action);
            Ok(())
        }
    }

    impl HotkeySource for Fake {
        fn register(&mut self, _hotkey: Hotkey) -> Result<HotkeyId, PlatformError> {
            Ok(HotkeyId(1))
        }

        fn unregister(&mut self, _id: HotkeyId) -> Result<(), PlatformError> {
            Ok(())
        }

        fn next_event(&mut self, _timeout: Duration) -> Result<Option<InputEvent>, PlatformError> {
            self.keys
                .pop_front()
                .ok_or_else(|| PlatformError::Backend("script exhausted".to_owned()))
        }
    }

    impl Platform for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
    }

    fn fake(count: u64, keys: &str) -> Fake {
        let elements = (0..count)
            .map(|i| Element {
                id: ElementId(i),
                role: Role::Button,
                name: String::new(),
                description: String::new(),
                value: String::new(),
                bounds: Rect::new(i32::try_from(i).unwrap() * 100, 0, 10, 10),
                enabled: true,
                offscreen: false,
            })
            .collect();
        // `!` Escape, `<` Backspace, `=` Enter, `>` Tab, `^` Shift-Tab, `+` Shift-Enter,
        // `@` an activation hotkey, `_` a poll that times out, an upper-case letter that letter
        // with Shift; anything else types itself.
        let keys = keys
            .chars()
            .map(|c| {
                let key = match c {
                    '@' => return Some(InputEvent::Hotkey(HotkeyId(1))),
                    '_' => return None,
                    '!' => KeyPress::Escape.into(),
                    '<' => KeyPress::Backspace.into(),
                    '=' => KeyPress::Enter.into(),
                    '>' => KeyPress::Tab.into(),
                    '^' => KeyPress::Tab.with(Modifiers::SHIFT),
                    '+' => KeyPress::Enter.with(Modifiers::SHIFT),
                    upper if upper.is_ascii_uppercase() => {
                        KeyPress::Char(upper.to_ascii_lowercase()).with(Modifiers::SHIFT)
                    }
                    other => KeyPress::Char(other).into(),
                };
                Some(InputEvent::Key(key))
            })
            .collect();
        Fake {
            elements,
            keys,
            ..Fake::default()
        }
    }

    fn settings() -> Settings {
        Settings {
            alphabet: Alphabet::default(),
            scope: Scope::FocusedWindow,
            scroll_step: 3,
            dash_step: 15,
            keys: KeyBindings::Nomouse,
            ignore: Vec::new(),
        }
    }

    fn run(platform: &mut Fake, mode: Mode) -> Result<Outcome, PlatformError> {
        activate(platform, &settings(), mode, Instant::now(), &|| false)
    }

    fn homerow(platform: &mut Fake) -> Result<Outcome, PlatformError> {
        let homerow = Settings {
            keys: KeyBindings::Homerow,
            ..settings()
        };
        activate(
            platform,
            &homerow,
            Mode::Once(ActionKind::Click),
            Instant::now(),
            &|| false,
        )
    }

    #[test]
    fn tab_twice_then_return_clicks_the_third_label_and_draws_the_highlight() {
        let mut p = fake(4, ">>=");
        assert_eq!(
            run(&mut p, Mode::Once(ActionKind::Click)),
            Ok(Outcome::Performed(1))
        );
        assert_eq!(p.performed, vec![Action::click(Point::new(205, 5))]);
        let focused: Vec<Option<&str>> = p.focused.iter().map(Option::as_deref).collect();
        assert_eq!(focused, vec![Some("a"), Some("s"), Some("d")]);
        let mut back = fake(3, "^=");
        run(&mut back, Mode::Once(ActionKind::Click)).unwrap();
        assert_eq!(back.performed, vec![Action::click(Point::new(205, 5))]);
    }

    #[test]
    fn activation_hotkey_while_shown_dismisses() {
        let mut p = fake(3, "a@");
        let mut once = fake(30, "a@");
        assert_eq!(
            run(&mut once, Mode::Once(ActionKind::Click)),
            Ok(Outcome::Cancelled)
        );
        assert!(!once.visible);
        assert_eq!(once.performed, Vec::new());
        // Continuous mode keeps the clicks made before the toggle.
        assert_eq!(
            run(&mut p, Mode::Continuous(ActionKind::Click)),
            Ok(Outcome::Performed(1))
        );
        assert!(!p.visible);
        let mut scrolling = fake(2, "aj@");
        assert_eq!(run(&mut scrolling, Mode::Scroll), Ok(Outcome::Performed(1)));
        assert!(!scrolling.visible);
    }

    #[test]
    fn a_socket_request_while_shown_dismisses() {
        let mut p = fake(3, "_");
        let outcome = activate(
            &mut p,
            &settings(),
            Mode::Once(ActionKind::Click),
            Instant::now(),
            &|| true,
        );
        assert_eq!(outcome, Ok(Outcome::Cancelled));
        assert!(!p.visible);
    }

    #[test]
    fn homerow_return_clicks_once_the_double_press_time_passes() {
        let mut p = fake(3, "s=_");
        assert_eq!(homerow(&mut p), Ok(Outcome::Performed(1)));
        assert_eq!(p.performed, vec![Action::click(Point::new(105, 5))]);
    }

    #[test]
    fn homerow_return_twice_double_clicks_and_shift_return_right_clicks() {
        let mut p = fake(3, "s==");
        assert_eq!(homerow(&mut p), Ok(Outcome::Performed(1)));
        assert_eq!(p.performed, vec![Action::double_click(Point::new(105, 5))]);
        let mut right = fake(3, "d+");
        assert_eq!(homerow(&mut right), Ok(Outcome::Performed(1)));
        assert_eq!(
            right.performed,
            vec![Action::right_click(Point::new(205, 5))]
        );
        let mut escaped = fake(3, "s=!");
        assert_eq!(homerow(&mut escaped), Ok(Outcome::Cancelled));
        assert_eq!(escaped.performed, Vec::new());
    }

    fn named(names: &[&str], keys: &str) -> Fake {
        let mut p = fake(u64::try_from(names.len()).unwrap(), keys);
        for (element, name) in p.elements.iter_mut().zip(names) {
            (*name).clone_into(&mut element.name);
        }
        p
    }

    #[test]
    fn search_mode_draws_the_query_and_enter_clicks_the_best_match() {
        let mut p = named(&["Profile", "Save", "File"], "fil=");
        assert_eq!(run(&mut p, Mode::Search), Ok(Outcome::Performed(1)));
        assert_eq!(p.queries, vec!["", "f", "fi", "fil"]);
        assert_eq!(p.shown[0].0, 0);
        assert_eq!(p.shown[3].0, 2);
        assert_eq!(p.performed, vec![Action::click(Point::new(205, 5))]);
    }

    #[test]
    fn search_mode_tab_label_and_modifier_click() {
        let mut p = named(&["Profile", "Save", "File"], "fil>2s");
        assert_eq!(run(&mut p, Mode::Search), Ok(Outcome::Performed(1)));
        let ctrl = nomouse_core::Modifiers {
            ctrl: true,
            ..nomouse_core::Modifiers::NONE
        };
        assert_eq!(
            p.performed,
            vec![Action::click(Point::new(5, 5)).with_modifiers(ctrl)]
        );
        let mut cancelled = named(&["Save"], "sa!");
        assert_eq!(run(&mut cancelled, Mode::Search), Ok(Outcome::Cancelled));
        assert_eq!(cancelled.performed, Vec::<Action>::new());
    }

    /// A button, a small scroll area and a large one below it, with the key script `keys`.
    fn scroll_areas(keys: &str) -> Fake {
        let mut p = fake(3, keys);
        p.elements[1].role = Role::ScrollArea;
        p.elements[1].bounds = Rect::new(0, 100, 100, 100);
        p.elements[2].role = Role::ScrollArea;
        p.elements[2].bounds = Rect::new(0, 300, 400, 400);
        p
    }

    const SMALL: Point = Point::new(50, 150);
    const LARGE: Point = Point::new(200, 500);

    #[test]
    fn scroll_mode_starts_on_the_largest_area_outlined() {
        let mut p = scroll_areas("j!");
        assert_eq!(run(&mut p, Mode::Scroll), Ok(Outcome::Performed(1)));
        assert_eq!(
            p.performed,
            [Action::hover(LARGE), Action::scroll(LARGE, 0, 3)]
        );
        // Both areas carry a digit label; the large one (second in reading order) is active.
        assert_eq!(p.shown[0].0, 2);
        assert_eq!(p.focused, [Some("2".to_owned())]);
        assert_eq!(p.outlined, [Some(Rect::new(0, 300, 400, 400))]);
        assert!(!p.visible);
    }

    #[test]
    fn scroll_mode_tab_and_digits_switch_areas() {
        let mut p = scroll_areas(">j^j1j2j!");
        assert_eq!(run(&mut p, Mode::Scroll), Ok(Outcome::Performed(4)));
        assert_eq!(
            p.performed,
            [
                Action::hover(LARGE),
                Action::hover(SMALL),
                Action::scroll(SMALL, 0, 3),
                Action::hover(LARGE),
                Action::scroll(LARGE, 0, 3),
                Action::hover(SMALL),
                Action::scroll(SMALL, 0, 3),
                Action::hover(LARGE),
                Action::scroll(LARGE, 0, 3),
            ]
        );
        let large = Some(Rect::new(0, 300, 400, 400));
        let small = Some(Rect::new(0, 100, 100, 100));
        assert_eq!(p.outlined, [large, small, large, small, large]);
    }

    #[test]
    fn scroll_mode_gg_g_and_shift_dash() {
        let mut p = scroll_areas("GggJKHLxg!");
        assert_eq!(run(&mut p, Mode::Scroll), Ok(Outcome::Performed(6)));
        let edge = nomouse_core::scroll::EDGE_STEPS;
        assert_eq!(
            p.performed[1..],
            [
                Action::scroll(LARGE, 0, edge),
                Action::scroll(LARGE, 0, -edge),
                Action::scroll(LARGE, 0, 15),
                Action::scroll(LARGE, 0, -15),
                Action::scroll(LARGE, -15, 0),
                Action::scroll(LARGE, 15, 0),
            ]
        );
    }

    #[test]
    fn scroll_mode_ends_on_enter_and_on_the_hotkey() {
        let mut entered = scroll_areas("j=j");
        assert_eq!(run(&mut entered, Mode::Scroll), Ok(Outcome::Performed(1)));
        let mut dismissed = scroll_areas("_j@j");
        assert_eq!(run(&mut dismissed, Mode::Scroll), Ok(Outcome::Performed(1)));
        assert!(!dismissed.visible);
    }

    #[test]
    fn one_click() {
        let mut p = fake(3, "zs");
        assert_eq!(
            run(&mut p, Mode::Once(ActionKind::Click)),
            Ok(Outcome::Performed(1))
        );
        assert_eq!(p.performed, vec![Action::click(Point::new(105, 5))]);
        assert!(!p.visible);
        assert_eq!(p.scans, 1);
    }

    #[test]
    fn narrowing_redraws_with_the_prefix() {
        let mut p = fake(20, "a<!");
        assert_eq!(
            run(&mut p, Mode::Once(ActionKind::Click)),
            Ok(Outcome::Cancelled)
        );
        assert_eq!(p.shown[1].1, "a");
        assert!(p.shown[1].0 < 20);
        assert_eq!(p.shown[2], (20, String::new()));
        assert_eq!(p.performed, Vec::new());
    }

    #[test]
    fn continuous_rescans_until_escape() {
        let mut p = fake(3, "asd!");
        assert_eq!(
            run(&mut p, Mode::Continuous(ActionKind::RightClick)),
            Ok(Outcome::Performed(3))
        );
        assert_eq!(p.scans, 4);
        assert!(p.performed.iter().all(|a| matches!(
            a,
            Action::Click {
                button: nomouse_core::MouseButton::Right,
                ..
            }
        )));
        assert!(!p.visible);
    }

    #[test]
    fn continuous_mode_ends_after_clicking_a_text_field() {
        // Buttons a and d, a text field s: the click on s ends the mode, and the keys after it
        // stay unread for the app.
        let mut p = fake(3, "asd!");
        p.elements[1].role = Role::TextField;
        assert_eq!(
            run(&mut p, Mode::Continuous(ActionKind::Click)),
            Ok(Outcome::Performed(2))
        );
        assert_eq!(
            p.performed,
            [
                Action::click(Point::new(5, 5)),
                Action::click(Point::new(105, 5))
            ]
        );
        assert_eq!(p.scans, 2);
        assert_eq!(p.keys.len(), 2);
        assert!(!p.visible);
    }

    #[test]
    fn scroll_mode_without_areas_scrolls_the_picked_element() {
        let mut p = fake(2, "ajjkdx!");
        assert_eq!(run(&mut p, Mode::Scroll), Ok(Outcome::Performed(4)));
        assert_eq!(p.performed[0], Action::hover(Point::new(5, 5)));
        assert_eq!(p.performed[1], Action::scroll(Point::new(5, 5), 0, 3));
        assert_eq!(p.performed[3], Action::scroll(Point::new(5, 5), 0, -3));
        assert_eq!(p.performed[4], Action::scroll(Point::new(5, 5), 0, 15));
        // The picked element is outlined once it is the scroll target.
        assert_eq!(p.outlined.last(), Some(&Some(Rect::new(0, 0, 10, 10))));
        assert!(!p.visible);
    }

    #[test]
    fn scroll_actions_carry_step_times_direction() {
        let mut p = fake(2, "ajkhl!");
        let step = Settings {
            scroll_step: 7,
            ..settings()
        };
        assert_eq!(
            activate(&mut p, &step, Mode::Scroll, Instant::now(), &|| false),
            Ok(Outcome::Performed(4))
        );
        let at = Point::new(5, 5);
        assert_eq!(
            p.performed[1..],
            [
                Action::scroll(at, 0, 7),
                Action::scroll(at, 0, -7),
                Action::scroll(at, -7, 0),
                Action::scroll(at, 7, 0),
            ]
        );
    }

    #[test]
    fn an_ignored_app_gets_no_scan_and_no_labels() {
        let ignoring = Settings {
            ignore: vec!["kitty".to_owned(), "org.gnome.Terminal".to_owned()],
            ..settings()
        };
        let terminal = AppIdentity {
            app_id: Some("org.gnome.Terminal".to_owned()),
            ..AppIdentity::default()
        };
        for mode in [Mode::Once(ActionKind::Click), Mode::Scroll, Mode::Search] {
            let mut p = fake(3, "a");
            p.app = Some(terminal.clone());
            assert_eq!(
                activate(&mut p, &ignoring, mode, Instant::now(), &|| false),
                Ok(Outcome::Ignored("org.gnome.Terminal".to_owned()))
            );
            assert_eq!((p.scans, p.shown.len()), (0, 0));
            assert_eq!(p.performed, Vec::new());
        }
        // Another app, or no ignore list, labels as usual.
        let mut other = fake(3, "a");
        other.app = Some(AppIdentity {
            process: Some("gedit".to_owned()),
            ..AppIdentity::default()
        });
        let once = Mode::Once(ActionKind::Click);
        assert_eq!(
            activate(&mut other, &ignoring, once, Instant::now(), &|| false),
            Ok(Outcome::Performed(1))
        );
        let mut unlisted = fake(3, "a");
        unlisted.app = Some(terminal);
        assert_eq!(run(&mut unlisted, once), Ok(Outcome::Performed(1)));
    }

    #[test]
    fn nothing_to_label() {
        let mut p = fake(0, "");
        assert_eq!(
            run(&mut p, Mode::Once(ActionKind::Click)),
            Ok(Outcome::NoElements)
        );
    }

    #[test]
    fn errors_still_hide_the_overlay() {
        let mut p = fake(3, "");
        assert!(run(&mut p, Mode::Once(ActionKind::Click)).is_err());
        assert!(!p.visible);
    }

    #[test]
    fn dash_step_setting_sets_shift_scrolls() {
        let mut p = scroll_areas("J!");
        let dash = Settings {
            dash_step: 40,
            ..settings()
        };
        assert_eq!(
            activate(&mut p, &dash, Mode::Scroll, Instant::now(), &|| false),
            Ok(Outcome::Performed(1))
        );
        assert_eq!(p.performed[1], Action::scroll(LARGE, 0, 40));
    }
}
