//! The backend contract: behaviour every [`Platform`] must show, checked the same way for the
//! in-memory [`FakePlatform`], each backend's fake system, and each real OS session.
//!
//! [`check_platform`] runs every check and returns the violations it found. It never
//! assumes a particular desktop: errors are allowed where the trait documents them (no
//! accessibility grant, no focused window, a taken hotkey), so the suite also passes on CI
//! runners without an Accessibility permission. It only hovers the pointer (never clicks), so
//! running it on a live session is harmless.
//!
//! Compiled with the `testing` feature; backends enable it from their `dev-dependencies`.

use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::time::{Duration, Instant};

use crate::action::Action;
use crate::element::{Element, ElementId, Point, Rect, Role};
use crate::hints::Hint;
use crate::hotkey::{Hotkey, KeyPress};
use crate::platform::{
    ElementSource, Frame, HotkeyId, HotkeySource, InputEvent, InputInjector, Overlay, Platform,
    PlatformError, Scope,
};

/// How long [`HotkeySource::next_event`] may overrun its timeout before the contract fails
/// (the default [`Probe::event_slack`]).
pub const EVENT_TIMEOUT_SLACK: Duration = Duration::from_secs(2);

/// One broken contract rule: which check and what the backend did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The check that failed, e.g. `"overlay: hide while hidden"`.
    pub check: &'static str,
    /// What the backend returned instead.
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.check, self.detail)
    }
}

/// Where the contract may point the pointer and draw its probe label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    /// A point on screen that is safe to hover (nothing happens on hover there).
    pub hover_at: Point,
    /// How long `next_event` may overrun its timeout.
    pub event_slack: Duration,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            hover_at: Point::new(5, 5),
            event_slack: EVENT_TIMEOUT_SLACK,
        }
    }
}

/// Runs every contract check against `platform` and returns the violations (empty = pass).
#[must_use]
pub fn check_platform<P: Platform + ?Sized>(platform: &mut P, probe: Probe) -> Vec<Violation> {
    let mut violations = Vec::new();
    check_name(platform, &mut violations);
    check_elements(platform, &mut violations);
    check_overlay(platform, probe, &mut violations);
    check_hotkeys(platform, &mut violations);
    check_events(platform, probe, &mut violations);
    check_input(platform, probe, &mut violations);
    violations
}

/// Panics listing every violation; for use in `#[test]` functions.
///
/// # Panics
/// When [`check_platform`] reports any violation.
pub fn assert_platform<P: Platform + ?Sized>(platform: &mut P, probe: Probe) {
    let violations = check_platform(platform, probe);
    assert!(
        violations.is_empty(),
        "{} broke the backend contract:\n{}",
        platform.name(),
        violations
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn violation(out: &mut Vec<Violation>, check: &'static str, detail: impl Into<String>) {
    out.push(Violation {
        check,
        detail: detail.into(),
    });
}

fn check_name<P: Platform + ?Sized>(platform: &P, out: &mut Vec<Violation>) {
    let name = platform.name();
    let well_formed = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !well_formed {
        violation(out, "name: short kebab-case", format!("{name:?}"));
    }
}

fn check_elements<P: Platform + ?Sized>(platform: &mut P, out: &mut Vec<Violation>) {
    for scope in [Scope::FocusedWindow, Scope::Screen] {
        match platform.elements(scope) {
            Ok(elements) => {
                let mut ids = HashSet::new();
                if let Some(duplicate) = elements.iter().find(|e| !ids.insert(e.id)) {
                    violation(
                        out,
                        "elements: ids unique within a snapshot",
                        format!("{scope:?}: {:?} repeats", duplicate.id),
                    );
                }
            }
            Err(
                PlatformError::AccessibilityUnavailable(_)
                | PlatformError::PermissionDenied(_)
                | PlatformError::NoFocusedWindow,
            ) => {}
            Err(other) => violation(
                out,
                "elements: only documented errors",
                format!("{scope:?}: {other:?}"),
            ),
        }
    }
}

fn probe_hint(at: Point) -> Hint {
    Hint {
        label: "a".to_owned(),
        element: Element {
            id: ElementId(0),
            role: Role::Button,
            name: "nomouse contract probe".to_owned(),
            description: String::new(),
            value: String::new(),
            bounds: Rect::new(at.x, at.y, 40, 20),
            enabled: true,
            offscreen: false,
        },
    }
}

fn check_overlay<P: Platform + ?Sized>(platform: &mut P, probe: Probe, out: &mut Vec<Violation>) {
    for attempt in ["first", "second"] {
        if let Err(error) = platform.hide() {
            violation(
                out,
                "overlay: hide while hidden is a no-op",
                format!("{attempt} hide: {error:?}"),
            );
        }
    }
    let hints = [probe_hint(probe.hover_at)];
    let label = hints[0].label.as_str();
    let frame = |hints, typed, query, focused| Frame {
        hints,
        typed,
        query,
        focused,
        area: None,
    };
    // A frame with neither a query, a highlight nor an outline goes through `show`, the rest
    // through `show_frame`.
    let steps: [(&str, Frame<'_>); 8] = [
        ("show", frame(&hints, "", None, None)),
        ("show replaces", frame(&hints, "a", None, None)),
        ("show nothing", frame(&[], "", None, None)),
        ("highlighted hint", frame(&hints, "", None, Some(label))),
        (
            "search box, empty query",
            frame(&hints, "", Some(""), Some(label)),
        ),
        (
            "search box with a query",
            frame(&hints, "", Some("probe"), None),
        ),
        (
            "search box, nothing matches",
            frame(&[], "", Some("zzz"), None),
        ),
        (
            "scroll area outlined",
            Frame {
                area: Some(hints[0].element.bounds),
                ..frame(&hints, "", None, Some(label))
            },
        ),
    ];
    for (step, frame) in steps {
        let result = if frame.query.is_none() && frame.focused.is_none() && frame.area.is_none() {
            platform.show(frame.hints, frame.typed)
        } else {
            platform.show_frame(&frame)
        };
        match result {
            Ok(()) | Err(PlatformError::PermissionDenied(_) | PlatformError::Unsupported(_)) => {}
            Err(error) => violation(out, "overlay: show", format!("{step}: {error:?}")),
        }
    }
    for attempt in ["hide", "hide again"] {
        if let Err(error) = platform.hide() {
            violation(
                out,
                "overlay: hide after show",
                format!("{attempt}: {error:?}"),
            );
        }
    }
}

fn check_hotkeys<P: Platform + ?Sized>(platform: &mut P, out: &mut Vec<Violation>) {
    if let Err(error) = platform.unregister(HotkeyId(u32::MAX)) {
        violation(
            out,
            "hotkeys: unregistering an unknown id is a no-op",
            format!("{error:?}"),
        );
    }
    let chords = ["ctrl+alt+shift+f23", "ctrl+alt+shift+f24"];
    let mut ids = Vec::new();
    for chord in chords {
        let Ok(hotkey) = chord.parse::<Hotkey>() else {
            violation(out, "hotkeys: probe chord parses", chord);
            continue;
        };
        match platform.register(hotkey) {
            Ok(id) => ids.push((hotkey, id)),
            Err(PlatformError::HotkeyTaken(_) | PlatformError::Unsupported(_)) => {}
            Err(error) => violation(out, "hotkeys: register", format!("{chord}: {error:?}")),
        }
    }
    if ids.len() == 2 && ids[0].1 == ids[1].1 {
        violation(out, "hotkeys: distinct ids", format!("{ids:?}"));
    }
    for &(hotkey, _) in &ids {
        match platform.register(hotkey) {
            Err(PlatformError::HotkeyTaken(_)) => {}
            Ok(duplicate) => {
                violation(
                    out,
                    "hotkeys: a chord registered twice is HotkeyTaken",
                    format!("{hotkey}: got a second id {duplicate:?}"),
                );
                if let Err(error) = platform.unregister(duplicate) {
                    violation(out, "hotkeys: unregister", format!("{hotkey}: {error:?}"));
                }
            }
            Err(error) => violation(
                out,
                "hotkeys: a chord registered twice is HotkeyTaken",
                format!("{hotkey}: {error:?}"),
            ),
        }
    }
    for (hotkey, id) in ids {
        if let Err(error) = platform.unregister(id) {
            violation(out, "hotkeys: unregister", format!("{hotkey}: {error:?}"));
            continue;
        }
        match platform.register(hotkey) {
            Ok(again) => {
                if let Err(error) = platform.unregister(again) {
                    violation(out, "hotkeys: unregister", format!("{hotkey}: {error:?}"));
                }
            }
            Err(error) => violation(
                out,
                "hotkeys: a chord is free again after unregister",
                format!("{hotkey}: {error:?}"),
            ),
        }
    }
}

fn check_events<P: Platform + ?Sized>(platform: &mut P, probe: Probe, out: &mut Vec<Violation>) {
    let timeout = Duration::from_millis(50);
    let started = Instant::now();
    let result = platform.next_event(timeout);
    let waited = started.elapsed();
    if let Err(error) = result {
        violation(
            out,
            "events: next_event on an idle session",
            format!("{error:?}"),
        );
    }
    if waited > timeout + probe.event_slack {
        violation(
            out,
            "events: next_event honours its timeout",
            format!("waited {waited:?} for a {timeout:?} timeout"),
        );
    }
}

fn check_input<P: Platform + ?Sized>(platform: &mut P, probe: Probe, out: &mut Vec<Violation>) {
    match platform.perform(&Action::hover(probe.hover_at)) {
        Ok(()) | Err(PlatformError::PermissionDenied(_)) => {}
        Err(error) => violation(out, "input: hover", format!("{error:?}")),
    }
}

/// An in-memory [`Platform`] that follows the contract: scripted elements and events, and a
/// record of everything shown and performed. The reference fake for tests of code above the
/// backends.
#[derive(Debug, Default)]
pub struct FakePlatform {
    /// What [`ElementSource::elements`] returns for every scope.
    pub elements: Vec<Element>,
    /// Events [`HotkeySource::next_event`] delivers, in order; `Ok(None)` once empty.
    pub events: VecDeque<InputEvent>,
    /// Hotkeys another program owns: [`HotkeySource::register`] reports them taken.
    pub taken: Vec<Hotkey>,
    /// Every [`Action`] performed, in order.
    pub performed: Vec<Action>,
    /// Every [`Overlay::show`] call: label count and typed prefix.
    pub shown: Vec<(usize, String)>,
    /// Whether the overlay is visible (and holds the keyboard).
    pub visible: bool,
    /// The registered hotkeys.
    pub registered: Vec<(HotkeyId, Hotkey)>,
    /// How many times elements were read.
    pub scans: usize,
    next_id: u32,
}

impl FakePlatform {
    /// A fake with `elements` on screen and `keys` typed while the overlay is shown.
    #[must_use]
    pub fn new(elements: Vec<Element>, keys: impl IntoIterator<Item = KeyPress>) -> Self {
        Self {
            elements,
            events: keys
                .into_iter()
                .map(|key| InputEvent::Key(key.into()))
                .collect(),
            ..Self::default()
        }
    }
}

impl ElementSource for FakePlatform {
    fn elements(&mut self, _scope: Scope) -> Result<Vec<Element>, PlatformError> {
        self.scans += 1;
        Ok(self.elements.clone())
    }
}

impl Overlay for FakePlatform {
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

impl InputInjector for FakePlatform {
    fn perform(&mut self, action: &Action) -> Result<(), PlatformError> {
        self.performed.push(*action);
        Ok(())
    }
}

impl HotkeySource for FakePlatform {
    fn register(&mut self, hotkey: Hotkey) -> Result<HotkeyId, PlatformError> {
        if self.taken.contains(&hotkey) || self.registered.iter().any(|(_, h)| *h == hotkey) {
            return Err(PlatformError::HotkeyTaken(hotkey));
        }
        self.next_id += 1;
        let id = HotkeyId(self.next_id);
        self.registered.push((id, hotkey));
        Ok(id)
    }

    fn unregister(&mut self, id: HotkeyId) -> Result<(), PlatformError> {
        self.registered.retain(|(known, _)| *known != id);
        Ok(())
    }

    fn next_event(&mut self, _timeout: Duration) -> Result<Option<InputEvent>, PlatformError> {
        Ok(self.events.pop_front())
    }
}

impl Platform for FakePlatform {
    fn name(&self) -> &'static str {
        "fake"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fake_passes_the_contract() {
        let mut fake = FakePlatform::new(
            vec![probe_hint(Point::new(0, 0)).element],
            [KeyPress::Escape],
        );
        assert_platform(&mut fake, Probe::default());
        assert!(!fake.visible);
        assert_eq!(fake.registered, Vec::new());
        assert_eq!(fake.performed, vec![Action::hover(Point::new(5, 5))]);
    }

    /// A platform that breaks every rule, to prove each check can fail.
    struct Broken;

    impl ElementSource for Broken {
        fn elements(&mut self, _scope: Scope) -> Result<Vec<Element>, PlatformError> {
            let element = probe_hint(Point::new(0, 0)).element;
            Ok(vec![element.clone(), element])
        }
    }

    impl Overlay for Broken {
        fn show(&mut self, _hints: &[Hint], _typed: &str) -> Result<(), PlatformError> {
            Err(PlatformError::Backend("no overlay".to_owned()))
        }

        fn hide(&mut self) -> Result<(), PlatformError> {
            Err(PlatformError::Backend("not shown".to_owned()))
        }
    }

    impl InputInjector for Broken {
        fn perform(&mut self, _action: &Action) -> Result<(), PlatformError> {
            Err(PlatformError::Backend("no input".to_owned()))
        }
    }

    impl HotkeySource for Broken {
        fn register(&mut self, _hotkey: Hotkey) -> Result<HotkeyId, PlatformError> {
            Ok(HotkeyId(7))
        }

        fn unregister(&mut self, _id: HotkeyId) -> Result<(), PlatformError> {
            Err(PlatformError::Backend("unknown".to_owned()))
        }

        fn next_event(&mut self, _timeout: Duration) -> Result<Option<InputEvent>, PlatformError> {
            Err(PlatformError::Backend("closed".to_owned()))
        }
    }

    impl Platform for Broken {
        fn name(&self) -> &'static str {
            "Broken Backend"
        }
    }

    #[test]
    fn every_check_can_fail() {
        let checks: HashSet<&str> = check_platform(&mut Broken, Probe::default())
            .into_iter()
            .map(|v| v.check)
            .collect();
        for expected in [
            "name: short kebab-case",
            "elements: ids unique within a snapshot",
            "overlay: hide while hidden is a no-op",
            "overlay: show",
            "overlay: hide after show",
            "hotkeys: unregistering an unknown id is a no-op",
            "hotkeys: distinct ids",
            "hotkeys: a chord registered twice is HotkeyTaken",
            "hotkeys: unregister",
            "events: next_event on an idle session",
            "input: hover",
        ] {
            assert!(
                checks.contains(expected),
                "{expected} not reported: {checks:?}"
            );
        }
    }

    /// Breaks the checks [`Broken`] cannot reach: undocumented element and register errors, a
    /// chord that stays taken after unregister, and a `next_event` that overruns its timeout.
    #[derive(Default)]
    struct Sticky {
        held: Vec<Hotkey>,
        released: Vec<Hotkey>,
    }

    impl ElementSource for Sticky {
        fn elements(&mut self, _scope: Scope) -> Result<Vec<Element>, PlatformError> {
            Err(PlatformError::Backend("tree exploded".to_owned()))
        }
    }

    impl Overlay for Sticky {
        fn show(&mut self, _hints: &[Hint], _typed: &str) -> Result<(), PlatformError> {
            Ok(())
        }

        fn hide(&mut self) -> Result<(), PlatformError> {
            Ok(())
        }
    }

    impl InputInjector for Sticky {
        fn perform(&mut self, _action: &Action) -> Result<(), PlatformError> {
            Ok(())
        }
    }

    impl HotkeySource for Sticky {
        fn register(&mut self, hotkey: Hotkey) -> Result<HotkeyId, PlatformError> {
            if hotkey.key == crate::hotkey::Key::F(23) {
                return Err(PlatformError::Backend("grab failed".to_owned()));
            }
            if self.held.contains(&hotkey) || self.released.contains(&hotkey) {
                return Err(PlatformError::HotkeyTaken(hotkey));
            }
            self.held.push(hotkey);
            Ok(HotkeyId(1))
        }

        fn unregister(&mut self, id: HotkeyId) -> Result<(), PlatformError> {
            if id == HotkeyId(1) {
                self.released.append(&mut self.held);
            }
            Ok(())
        }

        fn next_event(&mut self, timeout: Duration) -> Result<Option<InputEvent>, PlatformError> {
            std::thread::sleep(timeout * 2);
            Ok(None)
        }
    }

    impl Platform for Sticky {
        fn name(&self) -> &'static str {
            "sticky"
        }
    }

    #[test]
    fn the_remaining_checks_can_fail() {
        let probe = Probe {
            event_slack: Duration::from_millis(1),
            ..Probe::default()
        };
        let checks: HashSet<&str> = check_platform(&mut Sticky::default(), probe)
            .into_iter()
            .map(|v| v.check)
            .collect();
        for expected in [
            "elements: only documented errors",
            "hotkeys: register",
            "hotkeys: a chord is free again after unregister",
            "events: next_event honours its timeout",
        ] {
            assert!(
                checks.contains(expected),
                "{expected} not reported: {checks:?}"
            );
        }
    }

    #[test]
    fn documented_errors_are_allowed() {
        let mut fake = FakePlatform {
            taken: vec!["ctrl+alt+shift+f23".parse().expect("valid chord")],
            ..FakePlatform::default()
        };
        assert_eq!(check_platform(&mut fake, Probe::default()), Vec::new());
    }
}
