//! Every configuration option, written to a real `config.toml` and read back through the
//! loader the daemon uses ([`ConfigFile`]), then driven through what the daemon does with it:
//! [`crate::settings`] and [`crate::register`] build the activation settings and hotkeys, and
//! [`activate`] runs a session against a scripted platform that records what it was asked.
//!
//! Options the backends read (`overlay.*`, `hints.semantic_click`, `hints.browser`,
//! `macos.enhanced_ui_apps`) are checked in each backend's own tests; here they only have to
//! load and validate.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use nomouse_core::config::{BrowserMode, Config, HotkeysConfig, Theme};
use nomouse_core::settings::{Change, Settings as ConfigFile, WATCH_INTERVAL};
use nomouse_core::{
    Action, ActionKind, AppIdentity, Element, ElementId, ElementSource, Hint, Hotkey, HotkeyId,
    HotkeySource, InputEvent, InputInjector, KeyBindings, KeyPress, Modifiers, Overlay, Platform,
    PlatformError, Point, Rect, Role, Scope,
};

use super::{Frame, Mode, Outcome, Settings, activate};

/// A directory of its own under the system temp dir, removed when dropped. Tests run in
/// parallel, so every one gets a fresh name.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nomouse-config-options-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // A directory left by an earlier, killed run of the same test.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("config.toml")
    }

    fn write(&self, text: &str) -> PathBuf {
        let path = self.file();
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The configuration the daemon would run with for a file holding `text`.
fn loaded(text: &str) -> Config {
    let scratch = Scratch::new("load");
    let file = ConfigFile::open(scratch.write(text)).unwrap();
    assert_eq!(file.problem(), None, "{text}");
    file.config().clone()
}

/// The daemon's activation settings for a file holding `text`.
fn settings_for(text: &str) -> Settings {
    crate::settings(&loaded(text)).unwrap()
}

/// Why the daemon refuses a file holding `text`: the loader's problem, which must agree with
/// `nomouse config check` ([`Config::load`]).
fn rejected(text: &str) -> String {
    let scratch = Scratch::new("reject");
    let path = scratch.write(text);
    let file = ConfigFile::open(&path).unwrap();
    let problem = file
        .problem()
        .unwrap_or_else(|| panic!("accepted: {text}"))
        .to_owned();
    assert_eq!(
        file.config(),
        &Config::default(),
        "the defaults stay in effect"
    );
    let check = Config::load(&path).unwrap_err().to_string();
    assert!(problem.contains(&check), "{problem} vs {check}");
    problem
}

/// A scripted platform recording every request a session makes.
#[derive(Default)]
struct Fake {
    elements: Vec<Element>,
    /// The script: `None` is a poll that times out; the session errors once it runs out.
    events: VecDeque<Option<InputEvent>>,
    app: Option<AppIdentity>,
    scopes: Vec<Scope>,
    /// The labels of each frame drawn, with their elements' centres.
    frames: Vec<Vec<(String, Point)>>,
    performed: Vec<Action>,
    registered: Vec<(HotkeyId, Hotkey)>,
    visible: bool,
}

impl Fake {
    /// `count` buttons in a row, 100 px apart.
    fn buttons(count: u64) -> Self {
        let elements = (0..count)
            .map(|i| {
                element(
                    i,
                    Role::Button,
                    Rect::new(i32::try_from(i).unwrap() * 100, 0, 10, 10),
                )
            })
            .collect();
        Self {
            elements,
            ..Self::default()
        }
    }

    fn keys(mut self, keys: impl IntoIterator<Item = Option<InputEvent>>) -> Self {
        self.events.extend(keys);
        self
    }

    /// The label the first frame drew on the element centred at `at`.
    fn label_at(&self, at: Point) -> String {
        self.frames[0]
            .iter()
            .find(|(_, centre)| *centre == at)
            .map_or_else(
                || panic!("no label at {at:?} in {:?}", self.frames[0]),
                |(label, _)| label.clone(),
            )
    }
}

fn element(id: u64, role: Role, bounds: Rect) -> Element {
    Element {
        id: ElementId(id),
        role,
        name: String::new(),
        description: String::new(),
        value: String::new(),
        bounds,
        enabled: true,
        offscreen: false,
    }
}

fn typed(text: &str) -> Vec<Option<InputEvent>> {
    text.chars()
        .map(|c| Some(InputEvent::Key(KeyPress::Char(c).into())))
        .collect()
}

fn key(press: KeyPress, modifiers: Modifiers) -> InputEvent {
    InputEvent::Key(press.with(modifiers))
}

impl ElementSource for Fake {
    fn elements(&mut self, scope: Scope) -> Result<Vec<Element>, PlatformError> {
        self.scopes.push(scope);
        Ok(self.elements.clone())
    }

    fn focused_app(&mut self) -> Result<Option<AppIdentity>, PlatformError> {
        Ok(self.app.clone())
    }
}

impl Overlay for Fake {
    fn show(&mut self, hints: &[Hint], _typed: &str) -> Result<(), PlatformError> {
        self.frames.push(
            hints
                .iter()
                .map(|hint| (hint.label.clone(), hint.element.bounds.center()))
                .collect(),
        );
        self.visible = true;
        Ok(())
    }

    fn show_frame(&mut self, frame: &Frame<'_>) -> Result<(), PlatformError> {
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
    fn register(&mut self, hotkey: Hotkey) -> Result<HotkeyId, PlatformError> {
        let id = HotkeyId(u32::try_from(self.registered.len()).unwrap() + 1);
        self.registered.push((id, hotkey));
        Ok(id)
    }

    fn unregister(&mut self, id: HotkeyId) -> Result<(), PlatformError> {
        self.registered.retain(|(known, _)| *known != id);
        Ok(())
    }

    fn next_event(&mut self, _timeout: Duration) -> Result<Option<InputEvent>, PlatformError> {
        self.events
            .pop_front()
            .ok_or_else(|| PlatformError::Backend("script exhausted".to_owned()))
    }
}

impl Platform for Fake {
    fn name(&self) -> &'static str {
        "config-options fake"
    }
}

fn run(platform: &mut Fake, settings: &Settings, mode: Mode) -> Outcome {
    let outcome = activate(platform, settings, mode, Instant::now(), &|| false).unwrap();
    assert!(!platform.visible, "the overlay is hidden afterwards");
    outcome
}

const FIRST: Point = Point::new(5, 5);
const SECOND: Point = Point::new(105, 5);
const THIRD: Point = Point::new(205, 5);

/// Runs one hint-mode activation over three buttons, typing the label of the second.
fn pick_second(settings: &Settings, mode: Mode) -> Fake {
    // Draw once to learn the labels, then type the second one.
    let mut probe = Fake::buttons(3).keys([Some(key(KeyPress::Escape, Modifiers::NONE))]);
    assert_eq!(run(&mut probe, settings, mode), Outcome::Cancelled);
    let label = probe.label_at(SECOND);
    let mut platform = Fake::buttons(3).keys(typed(&label));
    run(&mut platform, settings, mode);
    platform
}

#[test]
fn an_empty_or_missing_file_is_every_default() {
    assert_eq!(loaded(""), Config::default());
    assert_eq!(loaded("# only a comment\n\n"), Config::default());
    let scratch = Scratch::new("missing");
    let missing = scratch.0.join("absent").join("config.toml");
    let file = ConfigFile::open(&missing).unwrap();
    assert_eq!((file.config(), file.problem()), (&Config::default(), None));
    assert!(!missing.exists(), "opening never creates the file");
    let error = Config::load(&missing).unwrap_err().to_string();
    assert!(error.contains(&missing.display().to_string()), "{error}");
}

/// Bad files, each with what the refusal must say: the key (or its line) and why.
const BAD_VALUES: &[(&str, &[&str])] = &[
    (
        "[hints]\nalphabet = \"a\"\n",
        &["hints.alphabet", "at least 2"],
    ),
    ("[hints]\nalphabet = \"asl\"\n", &["hints.alphabet", "'l'"]),
    ("[hints]\nalphabet = \"asa\"\n", &["hints.alphabet", "'a'"]),
    ("[hints]\nalphabet = \"aS\"\n", &["hints.alphabet", "'S'"]),
    (
        "[hints]\nalphabet = 7\n",
        &["alphabet = 7", "expected a string"],
    ),
    (
        "[hints]\nscope = \"all\"\n",
        &["scope = \"all\"", "`focused-window` or `screen`"],
    ),
    (
        "[hints]\naction = \"tap\"\n",
        &["action = \"tap\"", "`right-click`"],
    ),
    (
        "[hints]\nsemantic_click = \"yes\"\n",
        &["semantic_click = \"yes\"", "boolean"],
    ),
    (
        "[hints]\nkeys = \"vim\"\n",
        &["keys = \"vim\"", "`homerow`"],
    ),
    (
        "[hints]\nbrowser = \"all\"\n",
        &["browser = \"all\"", "`exhaustive`"],
    ),
    (
        "[hotkeys]\nhints = \"space\"\n",
        &["hints = \"space\"", "at least one modifier"],
    ),
    (
        "[hotkeys]\ncontinuous = \"ctrl+mouse\"\n",
        &["continuous = ", "unknown key \"mouse\""],
    ),
    (
        "[hotkeys]\nscroll = \"hyper+j\"\n",
        &["scroll = ", "\"hyper\" is not a modifier"],
    ),
    (
        "[hotkeys]\nsearch = \"ctrl+ctrl+j\"\n",
        &["search = ", "repeated"],
    ),
    (
        "[hotkeys]\nsearch = \"\"\n",
        &["search = \"\"", "empty hotkey"],
    ),
    (
        "[hotkeys]\nhints = \"ctrl+j\"\nscroll = \"control+J\"\n",
        &["hotkeys.hints and hotkeys.scroll are both ctrl+j"],
    ),
    ("[scroll]\nstep = 0\n", &["scroll.step = 0", "1..=50"]),
    ("[scroll]\nstep = 51\n", &["scroll.step = 51", "1..=50"]),
    ("[scroll]\nstep = -1\n", &["step = -1", "u32"]),
    ("[scroll]\nstep = 1.5\n", &["step = 1.5", "u32"]),
    (
        "[scroll]\ndash_step = 0\n",
        &["scroll.dash_step = 0", "1..=100"],
    ),
    (
        "[scroll]\ndash_step = 101\n",
        &["scroll.dash_step = 101", "1..=100"],
    ),
    (
        "[overlay]\nfont_size = 5\n",
        &["overlay.font_size = 5", "6..=72"],
    ),
    (
        "[overlay]\nfont_size = 73\n",
        &["overlay.font_size = 73", "6..=72"],
    ),
    (
        "[overlay]\nfont_size = \"big\"\n",
        &["font_size = \"big\"", "u32"],
    ),
    (
        "[overlay]\nopacity = 9\n",
        &["overlay.opacity = 9", "10..=100"],
    ),
    (
        "[overlay]\nopacity = 101\n",
        &["overlay.opacity = 101", "10..=100"],
    ),
    (
        "[overlay]\ntheme = \"blue\"\n",
        &["theme = \"blue\"", "`auto`"],
    ),
    (
        "[overlay]\nbackground = \"red\"\n",
        &["overlay.background", "#rrggbb"],
    ),
    (
        "[overlay]\nforeground = \"#12345\"\n",
        &["overlay.foreground", "#rrggbb"],
    ),
    (
        "[overlay]\ntyped_foreground = \"#gggggg\"\n",
        &["overlay.typed_foreground", "#rrggbb"],
    ),
    (
        "[overlay]\nborder = \"112233\"\n",
        &["overlay.border", "#rrggbb"],
    ),
    (
        "[ignore]\napps = \"kitty\"\n",
        &["apps = \"kitty\"", "sequence"],
    ),
    (
        "[ignore]\napps = [\"kitty\", \" \"]\n",
        &["ignore.apps", "empty entry"],
    ),
    (
        "[macos]\nenhanced_ui_apps = \"com.google.Chrome\"\n",
        &["enhanced_ui_apps = ", "sequence"],
    ),
    (
        "[macos]\nenhanced_ui_apps = [\"\"]\n",
        &["macos.enhanced_ui_apps", "empty entry"],
    ),
    (
        "[hints]\nlate_rescan_ms = 10001\n",
        &["hints.late_rescan_ms", "10001"],
    ),
    (
        "[hints]\nearly_rescan_ms = 3000\nlate_rescan_ms = 1000\n",
        &["hints.late_rescan_ms", "later than hints.early_rescan_ms"],
    ),
];

#[test]
fn every_bad_value_is_refused_naming_its_key() {
    for (text, needles) in BAD_VALUES {
        let problem = rejected(text);
        for needle in *needles {
            assert!(
                problem.contains(needle),
                "{text:?}: {problem:?} lacks {needle:?}"
            );
        }
    }
}

#[test]
fn unknown_keys_and_sections_are_refused_with_the_valid_names() {
    for (text, needles) in [
        ("[bogus]\nx = 1\n", ["unknown field `bogus`", "`hints`"]),
        (
            "[hints]\nalphabett = \"fj\"\n",
            ["unknown field `alphabett`", "`alphabet`"],
        ),
        (
            "[hotkeys]\nhint = \"ctrl+j\"\n",
            ["unknown field `hint`", "`hints`"],
        ),
        (
            "[scroll]\ndash-step = 3\n",
            ["unknown field `dash-step`", "`dash_step`"],
        ),
        (
            "[overlay]\nfontsize = 3\n",
            ["unknown field `fontsize`", "`font_size`"],
        ),
        ("[ignore]\napp = []\n", ["unknown field `app`", "`apps`"]),
        (
            "[macos]\nenhanced = []\n",
            ["unknown field `enhanced`", "`enhanced_ui_apps`"],
        ),
        ("hints = 3\n", ["hints = 3", "HintsConfig"]),
    ] {
        let problem = rejected(text);
        for needle in needles {
            assert!(
                problem.contains(needle),
                "{text:?}: {problem:?} lacks {needle:?}"
            );
        }
    }
    assert!(rejected("[hints\n").contains("TOML parse error"));
}

#[test]
fn hints_alphabet_makes_the_labels() {
    let settings = settings_for("[hints]\nalphabet = \"fj\"\n");
    let platform = pick_second(&settings, Mode::Once(ActionKind::Click));
    assert_eq!(platform.performed, [Action::click(SECOND)]);
    let letters: String = platform.frames[0]
        .iter()
        .map(|(label, _)| label.as_str())
        .collect();
    assert!(letters.chars().all(|c| c == 'f' || c == 'j'), "{letters}");
    // The default alphabet's first letter is not a label any more.
    let mut stray = Fake::buttons(3).keys(
        typed("a")
            .into_iter()
            .chain([None, Some(key(KeyPress::Escape, Modifiers::NONE))]),
    );
    assert_eq!(
        run(&mut stray, &settings, Mode::Once(ActionKind::Click)),
        Outcome::Cancelled
    );
    assert_eq!(stray.performed, []);
}

#[test]
fn hints_scope_picks_where_elements_come_from() {
    for (text, scope) in [
        ("", Scope::FocusedWindow),
        (
            "[hints]\nscope = \"focused-window\"\n",
            Scope::FocusedWindow,
        ),
        ("[hints]\nscope = \"screen\"\n", Scope::Screen),
    ] {
        let settings = settings_for(text);
        for mode in [Mode::Once(ActionKind::Click), Mode::Search, Mode::Scroll] {
            let mut platform =
                Fake::buttons(1).keys([Some(key(KeyPress::Escape, Modifiers::NONE))]);
            run(&mut platform, &settings, mode);
            // The first scan and every re-scan read the configured scope.
            assert!(
                !platform.scopes.is_empty() && platform.scopes.iter().all(|s| *s == scope),
                "{text:?} {mode:?}: {:?}",
                platform.scopes
            );
        }
    }
}

#[test]
fn hints_action_is_what_the_hint_hotkeys_do() {
    for kind in [
        ActionKind::Click,
        ActionKind::RightClick,
        ActionKind::DoubleClick,
        ActionKind::Hover,
    ] {
        let config = loaded(&format!("[hints]\naction = \"{kind}\"\n"));
        let mut platform = Fake::default();
        let modes: Vec<Mode> = crate::register(&mut platform, &config)
            .into_iter()
            .map(|(_, mode)| mode)
            .collect();
        assert_eq!(
            modes,
            [
                Mode::Once(kind),
                Mode::Continuous(kind),
                Mode::Scroll,
                Mode::Search
            ]
        );
        let picked = pick_second(&crate::settings(&config).unwrap(), modes[0]);
        assert_eq!(picked.performed, [kind.at(SECOND).unwrap()], "{kind}");
    }
}

#[test]
fn hints_action_drag_takes_two_picks() {
    let config = loaded("[hints]\naction = \"drag\"\n");
    let settings = crate::settings(&config).unwrap();
    let mode = crate::register(&mut Fake::default(), &config)[0].1;
    assert_eq!(mode, Mode::Once(ActionKind::Drag));
    let mut probe = Fake::buttons(3).keys([Some(key(KeyPress::Escape, Modifiers::NONE))]);
    run(&mut probe, &settings, mode);
    let (from, to) = (probe.label_at(FIRST), probe.label_at(THIRD));
    let mut platform = Fake::buttons(3).keys(typed(&from).into_iter().chain(typed(&to)));
    assert_eq!(run(&mut platform, &settings, mode), Outcome::Performed(1));
    assert!(
        matches!(
            platform.performed[..],
            [Action::Drag {
                from: FIRST,
                to: THIRD,
                ..
            }]
        ),
        "{:?}",
        platform.performed
    );
}

#[test]
fn hints_action_scroll_scrolls_the_picked_element() {
    let config = loaded("[hints]\naction = \"scroll\"\n[scroll]\nstep = 4\n");
    let settings = crate::settings(&config).unwrap();
    let mode = crate::register(&mut Fake::default(), &config)[0].1;
    assert_eq!(mode, Mode::Once(ActionKind::Scroll));
    let mut probe = Fake::buttons(3).keys([Some(key(KeyPress::Escape, Modifiers::NONE))]);
    run(&mut probe, &settings, mode);
    let label = probe.label_at(SECOND);
    let script = typed(&label)
        .into_iter()
        .chain(typed("j"))
        .chain([Some(key(KeyPress::Escape, Modifiers::NONE))]);
    let mut platform = Fake::buttons(3).keys(script);
    run(&mut platform, &settings, mode);
    assert_eq!(
        platform.performed.last(),
        Some(&Action::scroll(SECOND, 0, 4))
    );
}

#[test]
fn hints_keys_nomouse_acts_at_once_and_homerow_waits_for_return() {
    let nomouse = settings_for("[hints]\nkeys = \"nomouse\"\n");
    assert_eq!(nomouse.keys, KeyBindings::Nomouse);
    let platform = pick_second(&nomouse, Mode::Once(ActionKind::Click));
    assert_eq!(platform.performed, [Action::click(SECOND)]);

    let homerow = settings_for("[hints]\nkeys = \"homerow\"\n");
    let mut probe = Fake::buttons(3).keys([Some(key(KeyPress::Escape, Modifiers::NONE))]);
    run(&mut probe, &homerow, Mode::Once(ActionKind::Click));
    let label = probe.label_at(SECOND);
    // The complete label only highlights: Escape then performs nothing.
    let mut highlighted = Fake::buttons(3).keys(
        typed(&label)
            .into_iter()
            .chain([None, Some(key(KeyPress::Escape, Modifiers::NONE))]),
    );
    assert_eq!(
        run(&mut highlighted, &homerow, Mode::Once(ActionKind::Click)),
        Outcome::Cancelled
    );
    assert_eq!(highlighted.performed, []);
    // Return, then the double-press time passing, clicks; Shift-Return right-clicks.
    for (enter, action) in [
        (Modifiers::NONE, Action::click(SECOND)),
        (Modifiers::SHIFT, Action::right_click(SECOND)),
    ] {
        let mut platform = Fake::buttons(3).keys(
            typed(&label)
                .into_iter()
                .chain([Some(key(KeyPress::Enter, enter)), None]),
        );
        run(&mut platform, &homerow, Mode::Once(ActionKind::Click));
        assert_eq!(platform.performed, [action]);
    }
}

#[test]
fn hotkeys_register_the_chords_in_the_file() {
    // Without a `[hotkeys]` section this build's defaults are registered.
    let mut platform = Fake::default();
    crate::register(&mut platform, &loaded(""));
    let defaults = HotkeysConfig::default();
    let chords: Vec<Hotkey> = platform.registered.iter().map(|(_, h)| *h).collect();
    assert_eq!(
        chords,
        [
            defaults.hints,
            defaults.continuous,
            defaults.scroll,
            defaults.search
        ]
    );

    let config = loaded(
        "[hotkeys]\nhints = \"Cmd+Shift+Space\"\ncontinuous = \"super+alt+f5\"\n\
         scroll = \"control+option+j\"\nsearch = \"ctrl+alt+/\"\n",
    );
    let mut platform = Fake::default();
    let modes = crate::register(&mut platform, &config);
    let registered: Vec<(String, Mode)> = platform
        .registered
        .iter()
        .zip(&modes)
        .map(|((id, hotkey), (mode_id, mode))| {
            assert_eq!(id, mode_id);
            (hotkey.to_string(), *mode)
        })
        .collect();
    assert_eq!(
        registered,
        [
            (
                "shift+super+space".to_owned(),
                Mode::Once(ActionKind::Click)
            ),
            (
                "alt+super+f5".to_owned(),
                Mode::Continuous(ActionKind::Click)
            ),
            ("ctrl+alt+j".to_owned(), Mode::Scroll),
            // `/` and `slash` are the same key.
            ("ctrl+alt+slash".to_owned(), Mode::Search),
        ]
    );
}

/// A button, then a large scroll area: scroll mode starts on the area.
fn scroll_area(keys: Vec<Option<InputEvent>>) -> Fake {
    let mut platform = Fake::buttons(1).keys(keys);
    platform
        .elements
        .push(element(9, Role::ScrollArea, Rect::new(0, 100, 400, 400)));
    platform
}

const AREA: Point = Point::new(200, 300);

#[test]
fn scroll_step_and_dash_step_set_the_wheel_steps() {
    for (text, step, dash) in [
        ("", 3, 15),
        ("[scroll]\nstep = 1\ndash_step = 100\n", 1, 100),
        ("[scroll]\nstep = 50\ndash_step = 1\n", 50, 1),
    ] {
        let settings = settings_for(text);
        let script = typed("jk")
            .into_iter()
            .chain([
                Some(key(KeyPress::Char('j'), Modifiers::SHIFT)),
                Some(key(KeyPress::Char('l'), Modifiers::SHIFT)),
            ])
            .chain(typed("d"))
            .chain([Some(key(KeyPress::Escape, Modifiers::NONE))]);
        let mut platform = scroll_area(script.collect());
        assert_eq!(
            run(&mut platform, &settings, Mode::Scroll),
            Outcome::Performed(5)
        );
        let scrolls: Vec<Action> = platform
            .performed
            .into_iter()
            .filter(|action| matches!(action, Action::Scroll { .. }))
            .collect();
        assert_eq!(
            scrolls,
            [
                Action::scroll(AREA, 0, step),
                Action::scroll(AREA, 0, -step),
                Action::scroll(AREA, 0, dash),
                Action::scroll(AREA, dash, 0),
                Action::scroll(AREA, 0, 5 * step),
            ],
            "{text:?}"
        );
    }
}

#[test]
fn ignore_apps_shows_no_labels_in_a_listed_app() {
    let settings = settings_for("[ignore]\napps = [\"KITTY\", \"com.apple.Terminal\"]\n");
    let kitty = AppIdentity {
        process: Some("kitty".to_owned()),
        ..AppIdentity::default()
    };
    for mode in [
        Mode::Once(ActionKind::Click),
        Mode::Continuous(ActionKind::Click),
        Mode::Scroll,
        Mode::Search,
    ] {
        let mut platform = Fake::buttons(3);
        platform.app = Some(kitty.clone());
        assert_eq!(
            run(&mut platform, &settings, mode),
            Outcome::Ignored("KITTY".to_owned())
        );
        assert!(
            platform.scopes.is_empty() && platform.frames.is_empty(),
            "{mode:?}"
        );
    }
    // Any other app is labelled as usual.
    let mut other = Fake::buttons(3).keys(typed("a"));
    other.app = Some(AppIdentity {
        app_id: Some("org.mozilla.firefox".to_owned()),
        ..AppIdentity::default()
    });
    assert_eq!(
        run(&mut other, &settings, Mode::Once(ActionKind::Click)),
        Outcome::Performed(1)
    );
}

#[test]
fn backend_options_load_with_every_value() {
    let config = loaded(
        "[hints]\nsemantic_click = true\nbrowser = \"exhaustive\"\n\
         [overlay]\nfont_size = 72\nopacity = 10\ntheme = \"auto\"\nbackground = \"#102030\"\n\
         foreground = \"#FFFFFF\"\ntyped_foreground = \"#00ff00\"\nborder = \"#000000\"\n\
         [macos]\nenhanced_ui_apps = [\"com.google.Chrome\", \"com.microsoft.VSCode\"]\n",
    );
    assert!(config.hints.semantic_click);
    assert_eq!(config.hints.browser, BrowserMode::Exhaustive);
    assert_eq!((config.overlay.font_size, config.overlay.opacity), (72, 10));
    assert_eq!(config.overlay.theme, Theme::Auto);
    assert_eq!(
        config.macos.enhanced_ui_apps,
        ["com.google.Chrome", "com.microsoft.VSCode"]
    );
    // The activation settings do not depend on them.
    let settings = crate::settings(&config).unwrap();
    let platform = pick_second(&settings, Mode::Once(ActionKind::Click));
    assert_eq!(platform.performed, [Action::click(SECOND)]);
}

/// The daemon's file watch: a valid edit is picked up within [`WATCH_INTERVAL`] and changes
/// what the next activation and the re-registered hotkeys do; an invalid one keeps the last
/// good configuration.
#[test]
fn edits_reload_into_the_running_settings_and_hotkeys() {
    let scratch = Scratch::new("reload");
    let path = scratch.write("[scroll]\nstep = 2\n");
    let mut file = ConfigFile::open(&path).unwrap();
    assert!(
        !crate::file_changed(&mut file),
        "the first look finds the file as opened"
    );

    std::fs::write(
        &path,
        "[hints]\nalphabet = \"fj\"\nscope = \"screen\"\n[scroll]\nstep = 9\n\
         [hotkeys]\nhints = \"ctrl+alt+h\"\n",
    )
    .unwrap();
    let deadline = Instant::now() + WATCH_INTERVAL * 3;
    while !crate::file_changed(&mut file) {
        assert!(Instant::now() < deadline, "the edit was never noticed");
        std::thread::sleep(Duration::from_millis(50));
    }
    let settings = crate::settings(file.config()).unwrap();
    assert_eq!((settings.scroll_step, settings.scope), (9, Scope::Screen));
    let platform = pick_second(&settings, Mode::Once(ActionKind::Click));
    assert!(
        platform.frames[0]
            .iter()
            .all(|(label, _)| label.chars().all(|c| "fj".contains(c)))
    );
    let mut hotkeys = Fake::default();
    crate::register(&mut hotkeys, file.config());
    assert_eq!(hotkeys.registered[0].1.to_string(), "ctrl+alt+h");

    std::fs::write(&path, "[scroll]\nstep = 0\n").unwrap();
    assert!(matches!(file.reload(), Change::Invalid(_)));
    assert!(
        file.problem()
            .is_some_and(|p| p.contains("scroll.step = 0"))
    );
    assert_eq!(
        file.config().scroll.step,
        9,
        "the last good configuration stays"
    );
    assert!(
        !crate::file_changed(&mut file),
        "an invalid edit is not a reload"
    );

    std::fs::remove_file(&path).unwrap();
    assert!(matches!(
        file.reload(),
        Change::Reloaded | Change::Unchanged
    ));
    assert_eq!(
        file.config(),
        &Config::default(),
        "a deleted file is the defaults"
    );
}

/// `macos.enhanced_ui_apps` follows the file: an edit is noticed by the watch and makes the
/// reloaded configuration differ from the running one, which is what makes [`crate::reload`]
/// start a new backend with the new list; deleting the key brings back the Chromium default.
#[test]
fn enhanced_ui_apps_edits_reload() {
    let scratch = Scratch::new("reload-enhanced");
    let path = scratch.write("[macos]\nenhanced_ui_apps = [\"com.google.Chrome\"]\n");
    let mut file = ConfigFile::open(&path).unwrap();
    let running = file.config().clone();
    assert_eq!(running.macos.enhanced_ui_apps, ["com.google.Chrome"]);

    std::fs::write(
        &path,
        "[macos]\nenhanced_ui_apps = [\"com.google.Chrome\", \"com.microsoft.VSCode\"]\n",
    )
    .unwrap();
    let deadline = Instant::now() + WATCH_INTERVAL * 3;
    while !crate::file_changed(&mut file) {
        assert!(Instant::now() < deadline, "the edit was never noticed");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        file.config().macos.enhanced_ui_apps,
        ["com.google.Chrome", "com.microsoft.VSCode"]
    );
    assert_ne!(
        file.config(),
        &running,
        "the backend restarts with the new list"
    );

    std::fs::write(&path, "[hints]\n").unwrap();
    assert!(matches!(file.reload(), Change::Reloaded));
    assert_eq!(
        file.config().macos.enhanced_ui_apps,
        nomouse_core::config::CHROMIUM_BROWSERS
    );
}

/// `hints.early_rescan_ms` and `hints.late_rescan_ms` set the label stages of the next
/// activation, also when edited while the daemon runs.
#[test]
fn rescan_stages_come_from_the_file_and_reload() {
    assert_eq!(
        settings_for("").rescans,
        nomouse_core::RescanSchedule::new(1000, 3000)
    );
    let scratch = Scratch::new("reload-rescans");
    let path = scratch.write("[hints]\nearly_rescan_ms = 500\nlate_rescan_ms = 2000\n");
    let mut file = ConfigFile::open(&path).unwrap();
    assert_eq!(
        crate::settings(file.config()).unwrap().rescans.times(),
        [
            Duration::ZERO,
            Duration::from_millis(500),
            Duration::from_millis(2000)
        ]
    );
    std::fs::write(
        &path,
        "[hints]\nearly_rescan_ms = 0\nlate_rescan_ms = 4000\n",
    )
    .unwrap();
    let deadline = Instant::now() + WATCH_INTERVAL * 3;
    while !crate::file_changed(&mut file) {
        assert!(Instant::now() < deadline, "the edit was never noticed");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        crate::settings(file.config()).unwrap().rescans.times(),
        [Duration::ZERO, Duration::from_millis(4000)]
    );
}
