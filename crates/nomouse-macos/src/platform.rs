//! [`MacPlatform`]: the core traits implemented on Accessibility, Quartz events and AppKit.

use std::collections::HashSet;
use std::thread;
use std::time::{Duration, Instant};

use nomouse_core::config::Config;
use nomouse_core::search::search_box_anchor;
use nomouse_core::{
    Action, Element, ElementId, ElementSource, Hint, Hotkey, HotkeyId, HotkeySource, InputEvent,
    InputInjector, MouseButton, Overlay, Platform, PlatformError, Point, Rect, Scope,
};
use nomouse_sys_macos::app::{self, App};
use nomouse_sys_macos::ax::{AxElement, is_process_trusted};
use nomouse_sys_macos::input::{self, MouseEventKind};
use nomouse_sys_macos::overlay::Overlay as Panels;
use nomouse_sys_macos::tap::EventTap;
use nomouse_sys_macos::{Button, Label, LabelStyle, ModifierFlags, TapEvent};

use crate::ax_tree::{LiveTree, RootSet, not_trusted, roots};
use crate::geometry::{lerp, screen_rect, to_panel, union, wheel_deltas};
use crate::keys::{chord, key_press, modifier_flags};
use crate::roles::{PRESS, SHOW_MENU};
use crate::style::label_style;
use crate::walk::{Limits, walk};

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

/// One element of the last snapshot, kept for semantic clicks.
struct Snapshot {
    element: AxElement,
    bounds: Rect,
}

/// The macOS backend. Must be created and used on the main thread.
pub struct MacPlatform {
    app: App,
    tap: EventTap,
    panels: Panels,
    style: LabelStyle,
    opacity: f32,
    semantic_click: bool,
    enhanced_ui_apps: Vec<String>,
    prepared_pids: HashSet<i32>,
    snapshot: Vec<Snapshot>,
    next_hotkey: u32,
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
        let colours = config
            .overlay
            .colours()
            .map_err(|error| PlatformError::Backend(error.to_string()))?;
        Ok(Self {
            app,
            tap,
            panels: Panels::new(),
            style: label_style(config.overlay.font_size, colours),
            opacity: crate::style::opacity(config.overlay.opacity),
            semantic_click: config.hints.semantic_click,
            enhanced_ui_apps: config.macos.enhanced_ui_apps.clone(),
            prepared_pids: HashSet::new(),
            snapshot: Vec::new(),
            next_hotkey: 1,
            limits: Limits::default(),
        })
    }

    /// Asks Chromium and Electron apps to build their accessibility tree, once per process.
    fn prepare_app(&mut self, pid: i32) {
        if !self.prepared_pids.insert(pid) {
            return;
        }
        let element = AxElement::application(pid);
        // Electron's switch; other apps reject the unknown attribute, which is harmless.
        element
            .set_bool_attribute("AXManualAccessibility", true)
            .ok();
        let opted_in = app::bundle_id(pid).is_some_and(|id| self.enhanced_ui_apps.contains(&id));
        if opted_in {
            element
                .set_bool_attribute("AXEnhancedUserInterface", true)
                .ok();
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

    /// Tries the accessibility action for a click at `at` on the element last labelled there.
    fn semantic(&self, at: Point, action: &str) -> bool {
        self.snapshot
            .iter()
            .find(|entry| entry.bounds.center() == at)
            .is_some_and(|entry| entry.element.perform_action(action).is_ok())
    }

    /// Draws the hint labels, plus the search box `/query` near the top centre of the main
    /// screen for a `query`, and grabs the keyboard.
    fn present(&mut self, hints: &[Hint], typed: &str, query: Option<&str>) {
        let screens = self.app.screens();
        let typed_len = typed.chars().count();
        let mut labels: Vec<Label> = hints
            .iter()
            .filter_map(|hint| {
                let bounds = hint.element.bounds;
                let anchor = to_panel(Point::new(bounds.x, bounds.y), &screens)?;
                Some(Label {
                    screen: anchor.screen,
                    x: anchor.x,
                    y: anchor.y,
                    text: hint.label.to_uppercase(),
                    typed: typed_len.min(hint.label.chars().count()),
                })
            })
            .collect();
        if let (Some(query), Some(primary)) = (query, screens.first().copied()) {
            let box_at = search_box_anchor(screen_rect(primary, primary));
            if let Some(anchor) = to_panel(Point::new(box_at.x, box_at.y), &screens) {
                let text = format!("/{}", query.to_uppercase());
                labels.push(Label {
                    screen: anchor.screen,
                    x: anchor.x,
                    y: anchor.y,
                    typed: text.chars().count(),
                    text,
                });
            }
        }
        self.panels
            .show(&self.app, &labels, &self.style, self.opacity);
        self.tap.set_grab(true);
    }
}

impl ElementSource for MacPlatform {
    fn elements(&mut self, scope: Scope) -> Result<Vec<Element>, PlatformError> {
        let frontmost = app::frontmost_pid();
        if let Some(pid) = frontmost {
            self.prepare_app(pid);
        }
        let set = match scope {
            Scope::FocusedWindow => RootSet::FocusedWindow,
            Scope::Screen => RootSet::AllWindows,
        };
        let found_roots = roots(set, frontmost, self.desktop())?;
        let started = Instant::now();
        let result = walk(&mut LiveTree, found_roots.roots, self.limits, || {
            started.elapsed()
        })?;
        if scope == Scope::FocusedWindow && !found_roots.has_window && result.found.is_empty() {
            return Err(PlatformError::NoFocusedWindow);
        }
        self.snapshot.clear();
        let mut elements = Vec::with_capacity(result.found.len());
        for (index, found) in result.found.into_iter().enumerate() {
            elements.push(Element {
                id: ElementId(u64::try_from(index).unwrap_or(u64::MAX)),
                role: found.role,
                name: found.name,
                bounds: found.bounds,
                enabled: found.enabled,
            });
            self.snapshot.push(Snapshot {
                element: found.node,
                bounds: found.bounds,
            });
        }
        Ok(elements)
    }
}

impl Overlay for MacPlatform {
    fn show(&mut self, hints: &[Hint], typed: &str) -> Result<(), PlatformError> {
        self.present(hints, typed, None);
        Ok(())
    }

    fn show_search(
        &mut self,
        hints: &[Hint],
        typed: &str,
        query: &str,
    ) -> Result<(), PlatformError> {
        self.present(hints, typed, Some(query));
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
                post(MouseEventKind::Move, at, 0, flags)?;
                thread::sleep(INPUT_PAUSE);
                for click in 1..=count.max(1) {
                    post(MouseEventKind::Down(button(b)), at, i64::from(click), flags)?;
                    post(MouseEventKind::Up(button(b)), at, i64::from(click), flags)?;
                }
            }
            Action::Hover { at } => post(MouseEventKind::Move, at, 0, NO_FLAGS)?,
            Action::Drag {
                from,
                to,
                button: b,
            } => {
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
        let chord = chord(hotkey).ok_or_else(|| {
            PlatformError::Unsupported(format!("{hotkey}: macOS keyboards have no such key"))
        })?;
        let id = self.next_hotkey;
        self.next_hotkey = self.next_hotkey.wrapping_add(1).max(1);
        self.tap.register(id, chord);
        Ok(HotkeyId(id))
    }

    fn unregister(&mut self, id: HotkeyId) -> Result<(), PlatformError> {
        self.tap.unregister(id.0);
        Ok(())
    }

    fn next_event(&mut self, timeout: Duration) -> Result<Option<InputEvent>, PlatformError> {
        let deadline = Instant::now() + timeout;
        loop {
            while let Some(event) = self.tap.pop() {
                match event {
                    TapEvent::Hotkey(id) => return Ok(Some(InputEvent::Hotkey(HotkeyId(id)))),
                    TapEvent::Key {
                        keycode, character, ..
                    } => {
                        if let Some(key) = key_press(keycode, character) {
                            return Ok(Some(InputEvent::Key(key)));
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
