//! [`MacPlatform`]: the core traits implemented on Accessibility, Quartz events and AppKit.

use std::collections::HashSet;
use std::thread;
use std::time::{Duration, Instant};

use nomouse_core::config::Config;
use nomouse_core::placement::label_bounds;
use nomouse_core::reveal::settle;
use nomouse_core::search::search_box_anchor;
use nomouse_core::{
    Action, Element, ElementId, ElementSource, Frame, Hint, Hotkey, HotkeyId, HotkeySource,
    InputEvent, InputInjector, MouseButton, Overlay, Platform, PlatformError, Point, Rect, Scope,
};
use nomouse_sys_macos::app::{self, App};
use nomouse_sys_macos::ax::{AxElement, is_process_trusted};
use nomouse_sys_macos::input::{self, MouseEventKind};
use nomouse_sys_macos::overlay::{Overlay as Panels, label_sizer};
use nomouse_sys_macos::tap::EventTap;
use nomouse_sys_macos::{
    Button, KeyLayout, Label, LabelStyle, ModifierFlags, Outline, TapEvent, layout,
};

use crate::ax_tree::{LiveTree, RootSet, enhanced_ui, frame_of, not_trusted, roots};
use crate::enhance::{is_web_app, missing_web_area, needs_enhancing};
use crate::geometry::{
    label_corner, lerp, screen_rect, to_panel, to_panel_rects, union, wheel_deltas, whole_points,
};
use crate::keys::{chord, held, key_press, modifier_flags};
use crate::roles::{PRESS, SHOW_MENU};
use crate::style::{Themed, appearance};
use crate::walk::{Limits, Walk, walk};

/// Global Accessibility messaging timeout, in seconds: how long one call waits for a busy app.
const AX_TIMEOUT: f32 = 0.25;
/// Longest single run-loop slice while waiting for events, in seconds.
const PUMP_SLICE: f64 = 0.05;
/// Pointer steps of a drag.
const DRAG_STEPS: i32 = 12;
/// Pause between drag steps and after moving before a click, so apps see distinct events.
const INPUT_PAUSE: Duration = Duration::from_millis(8);
/// Event flags for pointer input without modifier keys.
const NO_FLAGS: ModifierFlags = modifier_flags(nomouse_core::Modifiers::NONE);
/// How long a scroll into view may take before the action is given up.
const REVEAL_TIMEOUT: Duration = Duration::from_millis(2000);
/// How often the element's frame is re-read while it scrolls into view.
const REVEAL_POLL: Duration = Duration::from_millis(30);
/// Readings in a row that must agree before the scroll counts as finished: browsers report
/// the final frame while their smooth scroll still runs, and ignore a click meanwhile.
const REVEAL_STEADY: u32 = 8;
/// How long a scan waits for a web page's tree after switching `AXEnhancedUserInterface` on
/// (Chromium builds it a moment later).
const TREE_WAIT: Duration = Duration::from_millis(1500);
/// How often the window is walked again meanwhile.
const TREE_POLL: Duration = Duration::from_millis(100);

/// One element of the last snapshot, kept for semantic clicks and for scrolling off-screen
/// elements into view.
struct Snapshot {
    element: AxElement,
    bounds: Rect,
    /// Scrolled out of view (`hints.browser = "exhaustive"`).
    offscreen: bool,
    /// The region it is visible in once scrolled there.
    visible: Rect,
}

/// The macOS backend. Must be created and used on the main thread.
pub struct MacPlatform {
    app: App,
    tap: EventTap,
    panels: Panels,
    style: LabelStyle,
    /// Re-resolves `style` when `overlay.theme = "auto"` and the system appearance changes.
    themed: Themed,
    opacity: f32,
    semantic_click: bool,
    enhanced_ui_apps: Vec<String>,
    /// Processes already sent `AXManualAccessibility`.
    prepared_pids: HashSet<i32>,
    /// Processes already warned that they show no web content.
    hinted_pids: HashSet<i32>,
    snapshot: Vec<Snapshot>,
    next_hotkey: u32,
    /// Registered chords by id; a chord registered twice is [`PlatformError::HotkeyTaken`].
    hotkeys: Vec<(u32, Hotkey)>,
    /// The keyboard layout hotkeys were matched against; `None` when the input source has no
    /// key layout data (hotkeys then match ANSI positions).
    layout: Option<KeyLayout>,
    limits: Limits,
}

impl std::fmt::Debug for MacPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacPlatform")
            .field("panels", &self.panels)
            .field("semantic_click", &self.semantic_click)
            .field("enhanced_ui_apps", &self.enhanced_ui_apps)
            .field("snapshot", &self.snapshot.len())
            .finish_non_exhaustive()
    }
}

impl MacPlatform {
    /// Creates the backend, showing macOS's Accessibility prompt when trust is missing.
    ///
    /// # Errors
    /// [`PlatformError::PermissionDenied`] without Accessibility trust or when the event tap is
    /// refused, [`PlatformError::Unsupported`] off the main thread, [`PlatformError::Backend`]
    /// for invalid overlay colours.
    pub fn new(config: &Config) -> Result<Self, PlatformError> {
        if !is_process_trusted(true) {
            return Err(not_trusted());
        }
        let app = App::new().map_err(|error| PlatformError::Unsupported(error.to_string()))?;
        let tap =
            EventTap::new().map_err(|error| PlatformError::PermissionDenied(error.to_string()))?;
        // Applies to every element of every app; failure only leaves the system default (6 s).
        AxElement::system_wide()
            .set_messaging_timeout(AX_TIMEOUT)
            .ok();
        let (themed, style) = Themed::new(&config.overlay, || appearance(app.prefers_dark()))
            .map_err(|error| PlatformError::Backend(error.to_string()))?;
        tracing::debug!(
            "labels use the {:?} colours (background {})",
            themed.appearance(),
            themed.background()
        );
        Ok(Self {
            app,
            tap,
            panels: Panels::new(),
            style,
            themed,
            opacity: crate::style::opacity(config.overlay.opacity),
            semantic_click: config.hints.semantic_click,
            enhanced_ui_apps: config.macos.enhanced_ui_apps.clone(),
            prepared_pids: HashSet::new(),
            hinted_pids: HashSet::new(),
            snapshot: Vec::new(),
            next_hotkey: 1,
            hotkeys: Vec::new(),
            layout: layout::current(),
            limits: Limits {
                browser: config.hints.browser,
                ..Limits::default()
            },
        })
    }

    /// Asks Chromium and Electron apps to build their accessibility tree: Electron's switch
    /// once per process, and `AXEnhancedUserInterface` for listed apps (`listed`) before every
    /// scan whenever the app does not report it on (a relaunched Chromium starts with it off).
    /// Returns whether the attribute was just switched on, judged by reading it back: Chrome
    /// 157 answers the set with `kAXErrorNotImplemented` although it takes effect.
    fn prepare_app(&mut self, pid: i32, listed: bool) -> bool {
        let element = AxElement::application(pid);
        if self.prepared_pids.insert(pid) {
            // Electron's switch; other apps reject the unknown attribute, which is harmless.
            element
                .set_bool_attribute("AXManualAccessibility", true)
                .ok();
        }
        let current = enhanced_ui(&element);
        if !needs_enhancing(listed, current) {
            return false;
        }
        let answer = element.set_bool_attribute("AXEnhancedUserInterface", true);
        let now = enhanced_ui(&element);
        tracing::debug!(
            "AXEnhancedUserInterface for pid {pid} was {current:?}: set answered {answer:?}, \
             now {now:?}"
        );
        now == Some(true)
    }

    /// Walks the roots of `set` once.
    fn walk_once(
        &self,
        set: RootSet,
        frontmost: Option<i32>,
    ) -> Result<(bool, Walk<AxElement>), PlatformError> {
        let found_roots = roots(set, frontmost, self.desktop())?;
        let started = Instant::now();
        let result = walk(&mut LiveTree, found_roots.roots, self.limits, || {
            started.elapsed()
        })?;
        Ok((found_roots.has_window, result))
    }

    /// Logs once per process that the frontmost web app showed no web content.
    fn hint_missing_web(&mut self, pid: i32, bundle_id: &str, listed: bool) {
        if self.hinted_pids.insert(pid) {
            let enhanced = enhanced_ui(&AxElement::application(pid));
            tracing::warn!(
                "{}; `nomouse doctor` checks every open browser",
                missing_web_area(bundle_id, listed, enhanced)
            );
        }
    }

    /// The bounding box of every screen, in global top-left coordinates.
    fn desktop(&self) -> Rect {
        let screens = self.app.screens();
        let Some(primary) = screens.first().copied() else {
            return Rect::default();
        };
        screens
            .iter()
            .map(|screen| screen_rect(*screen, primary))
            .reduce(union)
            .unwrap_or_default()
    }

    /// Where an action aimed at `at` should land: `at` itself, unless it is the centre of an
    /// off-screen element of the last snapshot, which is first scrolled into view
    /// (`AXScrollToVisible`); its new centre once its frame is in view and has stopped moving
    /// (browsers scroll smoothly).
    ///
    /// # Errors
    /// [`PlatformError::Backend`] when the scroll fails or the element does not come to rest in
    /// view within [`REVEAL_TIMEOUT`] (its frame is stale or it still moves): acting at `at`,
    /// a stand-in point outside the page, or wherever it was last seen would click something
    /// else.
    fn reveal(&self, at: Point) -> Result<Point, PlatformError> {
        let Some(entry) = self
            .snapshot
            .iter()
            .find(|entry| entry.offscreen && entry.bounds.center() == at)
        else {
            return Ok(at);
        };
        entry
            .element
            .perform_action("AXScrollToVisible")
            .map_err(|error| {
                PlatformError::Backend(format!(
                    "scrolling an off-screen element into view: {error}"
                ))
            })?;
        let started = Instant::now();
        let frame = settle(
            || frame_of(&entry.element).filter(|frame| !frame.is_empty()),
            |frame| frame.intersects(&entry.visible),
            REVEAL_STEADY,
            || {
                thread::sleep(REVEAL_POLL);
                started.elapsed() < REVEAL_TIMEOUT
            },
        )
        .ok_or_else(|| {
            PlatformError::Backend(format!(
                "the element did not come to rest in view within {REVEAL_TIMEOUT:?}"
            ))
        })?;
        let landed = frame.center();
        tracing::debug!("scrolled into view: acting at {landed:?} instead of {at:?}");
        Ok(landed)
    }

    /// Tries the accessibility action for a click at `at` on the element last labelled there.
    fn semantic(&self, at: Point, action: &str) -> bool {
        self.snapshot
            .iter()
            .find(|entry| entry.bounds.center() == at)
            .is_some_and(|entry| entry.element.perform_action(action).is_ok())
    }

    /// Draws the frame's hint labels (the focused one highlighted), the outline of its scroll
    /// area, and the search box `/query` near the top centre of the main screen for a query,
    /// and grabs the keyboard.
    fn present(&mut self, frame: &Frame<'_>) {
        let screens = self.app.screens();
        let typed_len = frame.typed.chars().count();
        let size_of = label_sizer(&self.style);
        let mut labels: Vec<Label> = frame
            .hints
            .iter()
            .filter_map(|hint| {
                let text = hint.label.to_uppercase();
                let (width, height) = whole_points(size_of(&text));
                let mut target = hint.element.bounds;
                // An off-screen element's label is pinned to the nearest edge of its viewport.
                if let Some(entry) = usize::try_from(hint.element.id.0)
                    .ok()
                    .and_then(|index| self.snapshot.get(index))
                    .filter(|entry| entry.offscreen)
                {
                    target = label_bounds(target, width, height, entry.visible);
                }
                let corner = label_corner(target, (width, height), &screens)?;
                let anchor = to_panel(corner, &screens)?;
                Some(Label {
                    screen: anchor.screen,
                    x: anchor.x,
                    y: anchor.y,
                    text,
                    typed: typed_len.min(hint.label.chars().count()),
                    focused: frame.focused == Some(hint.label.as_str()),
                })
            })
            .collect();
        if let (Some(query), Some(primary)) = (frame.query, screens.first().copied()) {
            let box_at = search_box_anchor(screen_rect(primary, primary));
            if let Some(anchor) = to_panel(Point::new(box_at.x, box_at.y), &screens) {
                let text = format!("/{}", query.to_uppercase());
                labels.push(Label {
                    screen: anchor.screen,
                    x: anchor.x,
                    y: anchor.y,
                    typed: text.chars().count(),
                    text,
                    focused: false,
                });
            }
        }
        let outlines: Vec<Outline> = frame
            .area
            .map(|area| to_panel_rects(area, &screens))
            .unwrap_or_default()
            .into_iter()
            .map(|part| Outline {
                screen: part.screen,
                x: part.x,
                y: part.y,
                width: part.width,
                height: part.height,
            })
            .collect();
        self.panels
            .show(&self.app, &labels, &outlines, &self.style, self.opacity);
        self.tap.set_grab(true);
    }

    /// Re-matches every hotkey when the user switched keyboard layouts, so `ctrl+alt+/` stays
    /// on the key that types `/`.
    fn follow_layout(&mut self) {
        let current = layout::current_id();
        if current.as_deref() == self.layout.as_ref().map(|held| held.id.as_str()) {
            return;
        }
        self.layout = layout::current();
        tracing::info!(
            "keyboard layout {}: hotkeys re-matched",
            current.as_deref().unwrap_or("without key layout data")
        );
        for (id, hotkey) in &self.hotkeys {
            if let Some(chord) = chord(*hotkey, self.layout.as_ref()) {
                self.tap.register(*id, chord);
            }
        }
    }

    /// Follows a system appearance switch under `theme = "auto"` (read before each scan, so
    /// every activation starts in the current colours).
    fn follow_appearance(&mut self) {
        let app = &self.app;
        if let Some(style) = self.themed.changed(|| appearance(app.prefers_dark())) {
            self.style = style;
            tracing::debug!(
                "system appearance is now {:?}: labels use background {}",
                self.themed.appearance(),
                self.themed.background()
            );
        }
    }
}

impl ElementSource for MacPlatform {
    fn elements(&mut self, scope: Scope) -> Result<Vec<Element>, PlatformError> {
        self.follow_appearance();
        let frontmost = app::frontmost_pid();
        // The frontmost app's pid and bundle id, and whether it is listed for
        // `AXEnhancedUserInterface`.
        let target = frontmost.map(|pid| {
            let bundle_id = app::bundle_id(pid);
            let listed = bundle_id
                .as_ref()
                .is_some_and(|id| self.enhanced_ui_apps.contains(id));
            (pid, bundle_id, listed)
        });
        let switched_on = target
            .as_ref()
            .is_some_and(|&(pid, _, listed)| self.prepare_app(pid, listed));
        let set = match scope {
            Scope::FocusedWindow => RootSet::FocusedWindow,
            Scope::Screen => RootSet::AllWindows,
        };
        let started = Instant::now();
        let (mut has_window, mut result) = self.walk_once(set, frontmost)?;
        // Chromium builds a page's tree a moment after the attribute is switched on: walk
        // again until a web area shows up, so the first activation after a relaunch still
        // labels the page.
        while switched_on
            && has_window
            && result.web_areas == 0
            && started.elapsed() + TREE_POLL < TREE_WAIT
        {
            thread::sleep(TREE_POLL);
            (has_window, result) = self.walk_once(set, frontmost)?;
        }
        if scope == Scope::FocusedWindow && !has_window && result.found.is_empty() {
            return Err(PlatformError::NoFocusedWindow);
        }
        if let Some((pid, Some(bundle_id), listed)) = &target
            && has_window
            && result.web_areas == 0
            && is_web_app(bundle_id, &self.enhanced_ui_apps)
        {
            self.hint_missing_web(*pid, bundle_id, *listed);
        }
        tracing::debug!(
            "walked {} nodes in {:?}: {} elements, {} of them off-screen{}",
            result.visited,
            started.elapsed(),
            result.found.len(),
            result.found.iter().filter(|found| found.offscreen).count(),
            result.truncated.map_or_else(String::new, |limit| format!(
                " (stopped by the {limit:?} limit)"
            ))
        );
        for found in result.found.iter().filter(|found| found.offscreen) {
            tracing::debug!(
                "off-screen {:?} {:?} at {:?}",
                found.role,
                found.name,
                found.bounds
            );
        }
        self.snapshot.clear();
        let mut elements = Vec::with_capacity(result.found.len());
        for (index, found) in result.found.into_iter().enumerate() {
            elements.push(Element {
                id: ElementId(u64::try_from(index).unwrap_or(u64::MAX)),
                role: found.role,
                name: found.name,
                description: found.description,
                value: found.value,
                bounds: found.bounds,
                enabled: found.enabled,
                offscreen: found.offscreen,
            });
            self.snapshot.push(Snapshot {
                element: found.node,
                bounds: found.bounds,
                offscreen: found.offscreen,
                visible: found.visible,
            });
        }
        Ok(elements)
    }

    fn focused_app(&mut self) -> Result<Option<nomouse_core::AppIdentity>, PlatformError> {
        // The frontmost app, by bundle id and by executable name (unbundled programs have
        // only the latter).
        Ok(app::frontmost_pid().map(|pid| nomouse_core::AppIdentity {
            app_id: app::bundle_id(pid),
            process: app::process_name(pid),
            ..nomouse_core::AppIdentity::default()
        }))
    }
}

impl Overlay for MacPlatform {
    fn show(&mut self, hints: &[Hint], typed: &str) -> Result<(), PlatformError> {
        self.present(&Frame {
            hints,
            typed,
            ..Frame::default()
        });
        Ok(())
    }

    fn show_frame(&mut self, frame: &Frame<'_>) -> Result<(), PlatformError> {
        self.present(frame);
        Ok(())
    }

    fn hide(&mut self) -> Result<(), PlatformError> {
        self.tap.set_grab(false);
        self.panels.hide();
        Ok(())
    }
}

const fn button(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
    }
}

fn post(
    kind: MouseEventKind,
    at: Point,
    click_state: i64,
    modifiers: ModifierFlags,
) -> Result<(), PlatformError> {
    input::post_mouse(
        kind,
        f64::from(at.x),
        f64::from(at.y),
        click_state,
        modifiers,
    )
    .map_err(|error| PlatformError::Backend(error.to_string()))
}

impl InputInjector for MacPlatform {
    fn perform(&mut self, action: &Action) -> Result<(), PlatformError> {
        if !input::can_post_events() {
            return Err(not_trusted());
        }
        match *action {
            Action::Click {
                at,
                button: b,
                count,
                modifiers,
            } => {
                if self.semantic_click && count == 1 && modifiers.is_empty() {
                    let semantic = match b {
                        MouseButton::Left => Some(PRESS),
                        MouseButton::Right => Some(SHOW_MENU),
                        MouseButton::Middle => None,
                    };
                    if semantic.is_some_and(|name| self.semantic(at, name)) {
                        return Ok(());
                    }
                }
                // The flags on the mouse events are what apps read for cmd/shift/option+click.
                let flags = modifier_flags(modifiers);
                let at = self.reveal(at)?;
                post(MouseEventKind::Move, at, 0, flags)?;
                thread::sleep(INPUT_PAUSE);
                for click in 1..=count.max(1) {
                    post(MouseEventKind::Down(button(b)), at, i64::from(click), flags)?;
                    post(MouseEventKind::Up(button(b)), at, i64::from(click), flags)?;
                }
            }
            Action::Hover { at } => post(MouseEventKind::Move, self.reveal(at)?, 0, NO_FLAGS)?,
            Action::Drag {
                from,
                to,
                button: b,
            } => {
                let from = self.reveal(from)?;
                post(MouseEventKind::Move, from, 0, NO_FLAGS)?;
                thread::sleep(INPUT_PAUSE);
                post(MouseEventKind::Down(button(b)), from, 1, NO_FLAGS)?;
                for step in 1..=DRAG_STEPS {
                    thread::sleep(INPUT_PAUSE);
                    let point = Point::new(
                        lerp(from.x, to.x, step, DRAG_STEPS),
                        lerp(from.y, to.y, step, DRAG_STEPS),
                    );
                    post(MouseEventKind::Drag(button(b)), point, 1, NO_FLAGS)?;
                }
                post(MouseEventKind::Up(button(b)), to, 1, NO_FLAGS)?;
            }
            Action::Scroll { at, dx, dy } => {
                post(MouseEventKind::Move, at, 0, NO_FLAGS)?;
                let (vertical, horizontal) = wheel_deltas(dx, dy);
                input::post_scroll(vertical, horizontal)
                    .map_err(|error| PlatformError::Backend(error.to_string()))?;
            }
        }
        Ok(())
    }
}

impl HotkeySource for MacPlatform {
    fn register(&mut self, hotkey: Hotkey) -> Result<HotkeyId, PlatformError> {
        if self.hotkeys.iter().any(|(_, held)| *held == hotkey) {
            return Err(PlatformError::HotkeyTaken(hotkey));
        }
        let chord = chord(hotkey, self.layout.as_ref()).ok_or_else(|| {
            PlatformError::Unsupported(format!("{hotkey}: macOS keyboards have no such key"))
        })?;
        let id = self.next_hotkey;
        self.next_hotkey = self.next_hotkey.wrapping_add(1).max(1);
        self.tap.register(id, chord);
        self.hotkeys.push((id, hotkey));
        Ok(HotkeyId(id))
    }

    fn unregister(&mut self, id: HotkeyId) -> Result<(), PlatformError> {
        self.tap.unregister(id.0);
        self.hotkeys.retain(|(held, _)| *held != id.0);
        Ok(())
    }

    fn next_event(&mut self, timeout: Duration) -> Result<Option<InputEvent>, PlatformError> {
        // A scan longer than about a second gets the tap disabled; without this the next key
        // would slip past it to the app.
        self.tap
            .ensure_enabled()
            .map_err(|error| PlatformError::PermissionDenied(error.to_string()))?;
        self.follow_layout();
        let deadline = Instant::now() + timeout;
        loop {
            while let Some(event) = self.tap.pop() {
                match event {
                    TapEvent::Hotkey(id) => return Ok(Some(InputEvent::Hotkey(HotkeyId(id)))),
                    TapEvent::Key {
                        keycode,
                        character,
                        modifiers,
                    } => {
                        if let Some(key) = key_press(keycode, character) {
                            return Ok(Some(InputEvent::Key(key.with(held(modifiers)))));
                        }
                    }
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let slice = (deadline - now).as_secs_f64().min(PUMP_SLICE);
            self.app.pump(slice);
        }
    }
}

impl Platform for MacPlatform {
    fn name(&self) -> &'static str {
        "macos"
    }
}
