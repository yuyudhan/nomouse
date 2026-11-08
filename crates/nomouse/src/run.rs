//! Drives one hint-mode activation against a platform: scan, label, follow keys, act.

use std::time::{Duration, Instant};

use nomouse_core::{
    Action, ActionKind, Alphabet, HintSession, InputEvent, KeyPress, Platform, PlatformError,
    Point, Scope, SessionEvent,
};

/// How long continuous mode waits after a click before re-scanning, so the app can update.
pub(crate) const SETTLE: Duration = Duration::from_millis(50);

/// How often the session loop wakes up while waiting for keys.
const POLL: Duration = Duration::from_millis(250);

/// What one activation does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Pick one hint and perform `ActionKind` on it.
    Once(ActionKind),
    /// Keep re-labelling after every action until Escape.
    Continuous(ActionKind),
    /// Pick a scroll target, then scroll with keys until Escape.
    Scroll,
}

impl Mode {
    const fn kind(self) -> ActionKind {
        match self {
            Self::Once(kind) | Self::Continuous(kind) => kind,
            Self::Scroll => ActionKind::Scroll,
        }
    }
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
}

/// How an activation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Actions were performed (the count; continuous mode may perform several).
    Performed(usize),
    /// The user pressed Escape before acting.
    Cancelled,
    /// The scan found nothing to label.
    NoElements,
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

/// The scroll for a key in scroll mode; `None` ends scroll mode.
pub(crate) fn scroll_delta(key: KeyPress, step: i32) -> Option<(i32, i32)> {
    let page = step.saturating_mul(5);
    match key {
        KeyPress::Char('j') | KeyPress::Down => Some((0, step)),
        KeyPress::Char('k') | KeyPress::Up => Some((0, -step)),
        KeyPress::Char('h') | KeyPress::Left => Some((-step, 0)),
        KeyPress::Char('l') | KeyPress::Right => Some((step, 0)),
        KeyPress::Char('d' | ' ') => Some((0, page)),
        KeyPress::Char('u') => Some((0, -page)),
        KeyPress::Escape | KeyPress::Enter => None,
        _ => Some((0, 0)),
    }
}

fn scroll_mode(platform: &mut dyn Platform, at: Point, step: i32) -> Result<usize, PlatformError> {
    platform.perform(&Action::hover(at))?;
    // Showing no labels keeps the keyboard grabbed for the scroll keys.
    platform.show(&[], "")?;
    let mut scrolls = 0;
    loop {
        let Some(InputEvent::Key(key)) = platform.next_event(POLL)? else {
            continue;
        };
        match scroll_delta(key, step) {
            None => return Ok(scrolls),
            Some((0, 0)) => {}
            Some((dx, dy)) => {
                platform.perform(&Action::scroll(at, dx, dy))?;
                scrolls += 1;
            }
        }
    }
}

/// Runs one activation in `mode`. `started` is when the trigger arrived, for the
/// hotkey-to-labels timing logged at info level.
///
/// # Errors
/// Platform errors from scanning, drawing or acting; the overlay is hidden either way.
pub(crate) fn activate(
    platform: &mut dyn Platform,
    settings: &Settings,
    mode: Mode,
    started: Instant,
) -> Result<Outcome, PlatformError> {
    let guard = Shown { platform };
    let platform = &mut *guard.platform;
    let mut performed = 0;
    let mut scans = 0_usize;
    loop {
        let elements = platform.elements(settings.scope)?;
        let mut session = HintSession::new(&settings.alphabet, elements, mode.kind());
        if session.is_empty() {
            return Ok(if performed > 0 {
                Outcome::Performed(performed)
            } else {
                Outcome::NoElements
            });
        }
        platform.show(&session.visible(), "")?;
        scans += 1;
        tracing::debug!("scan {scans}: {} hints", session.hints().len());
        for hint in session.hints() {
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
                session.hints().len()
            );
        }
        let action = loop {
            let Some(InputEvent::Key(key)) = platform.next_event(POLL)? else {
                continue;
            };
            match session.key(key) {
                SessionEvent::Rejected => {}
                SessionEvent::Narrowed | SessionEvent::DragStarted(_) => {
                    platform.show(&session.visible(), session.typed())?;
                }
                SessionEvent::Cancelled => {
                    return Ok(if performed > 0 {
                        Outcome::Performed(performed)
                    } else {
                        Outcome::Cancelled
                    });
                }
                SessionEvent::ScrollTarget(at) => {
                    platform.hide()?;
                    let scrolls = scroll_mode(platform, at, settings.scroll_step)?;
                    return Ok(Outcome::Performed(performed + scrolls));
                }
                SessionEvent::Perform(action) => break action,
            }
        };
        platform.hide()?;
        platform.perform(&action)?;
        performed += 1;
        if !matches!(mode, Mode::Continuous(_)) {
            return Ok(Outcome::Performed(performed));
        }
        std::thread::sleep(SETTLE);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use nomouse_core::{
        Element, ElementId, ElementSource, Hint, Hotkey, HotkeyId, HotkeySource, InputInjector,
        Overlay, Rect, Role,
    };

    use super::*;

    #[derive(Default)]
    struct Fake {
        elements: Vec<Element>,
        keys: VecDeque<KeyPress>,
        performed: Vec<Action>,
        scans: usize,
        shown: Vec<(usize, String)>,
        visible: bool,
    }

    impl ElementSource for Fake {
        fn elements(&mut self, _scope: Scope) -> Result<Vec<Element>, PlatformError> {
            self.scans += 1;
            Ok(self.elements.clone())
        }
    }

    impl Overlay for Fake {
        fn show(&mut self, hints: &[Hint], typed: &str) -> Result<(), PlatformError> {
            self.shown.push((hints.len(), typed.to_owned()));
            self.visible = true;
            Ok(())
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
                .map(|key| Some(InputEvent::Key(key)))
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
                bounds: Rect::new(i32::try_from(i).unwrap() * 100, 0, 10, 10),
                enabled: true,
            })
            .collect();
        let keys = keys
            .chars()
            .map(|c| match c {
                '!' => KeyPress::Escape,
                '<' => KeyPress::Backspace,
                other => KeyPress::Char(other),
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
        }
    }

    fn run(platform: &mut Fake, mode: Mode) -> Result<Outcome, PlatformError> {
        activate(platform, &settings(), mode, Instant::now())
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
    fn scroll_mode_scrolls_until_escape() {
        let mut p = fake(2, "ajjkdx!");
        assert_eq!(run(&mut p, Mode::Scroll), Ok(Outcome::Performed(4)));
        assert_eq!(p.performed[0], Action::hover(Point::new(5, 5)));
        assert_eq!(p.performed[1], Action::scroll(Point::new(5, 5), 0, 3));
        assert_eq!(p.performed[3], Action::scroll(Point::new(5, 5), 0, -3));
        assert_eq!(p.performed[4], Action::scroll(Point::new(5, 5), 0, 15));
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
            activate(&mut p, &step, Mode::Scroll, Instant::now()),
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
    fn scroll_keys() {
        assert_eq!(scroll_delta(KeyPress::Char('h'), 2), Some((-2, 0)));
        assert_eq!(scroll_delta(KeyPress::Right, 2), Some((2, 0)));
        assert_eq!(scroll_delta(KeyPress::Char('u'), 2), Some((0, -10)));
        assert_eq!(scroll_delta(KeyPress::Enter, 2), None);
        assert_eq!(scroll_delta(KeyPress::Tab, 2), Some((0, 0)));
    }
}
