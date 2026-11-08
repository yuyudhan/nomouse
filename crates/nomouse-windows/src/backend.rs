//! The core traits implemented over a small [`System`] seam, so all backend logic is testable
//! with a fake on any OS; [`crate::WindowsPlatform`] plugs in the real Win32 system.

use std::path::Path;
use std::thread;
use std::time::Duration;

use nomouse_core::{
    Action, Config, Element, ElementSource, Hint, Hotkey, HotkeyId, HotkeySource, InputEvent,
    InputInjector, Modifiers, MouseButton, Overlay, Platform, PlatformError, Point, Scope,
};
use nomouse_sys_windows::{
    Activation, Bitmap, Edges, HotkeyModifiers, InputStep, RawElement, SysError, SysErrorKind,
    SysEvent, WindowHandle, WindowInfo,
};

use crate::classify;
use crate::input_plan;
use crate::keys;
use crate::render::{Renderer, Style};
use crate::scope::{self, ScopeOptions, TASKBAR_CLASS, TRAY_CLASS};

/// Pause between input batches (drag press, motion, release) so targets see distinct events.
const BATCH_PAUSE: Duration = Duration::from_millis(30);
/// The DPI of a 100 % monitor.
const BASE_DPI: f32 = 96.0;

/// The OS operations the backend needs; implemented by the Win32 system and by test fakes.
pub trait System {
    /// The foreground window.
    fn foreground_window(&mut self) -> Option<WindowHandle>;
    /// Shown top-level windows, topmost first.
    fn top_level_windows(&mut self) -> Vec<WindowInfo>;
    /// The first top-level window of a class.
    fn find_window(&mut self, class: &str) -> Option<WindowHandle>;
    /// The first descendant window of a class.
    fn find_descendant(&mut self, parent: WindowHandle, class: &str) -> Option<WindowHandle>;
    /// Information about one window.
    ///
    /// # Errors
    /// When the window is gone.
    fn window_info(&mut self, window: WindowHandle) -> Result<WindowInfo, SysError>;
    /// Starts a new element snapshot, dropping retained elements.
    fn begin_snapshot(&mut self);
    /// Appends the matching elements of a window's subtree to the snapshot.
    ///
    /// # Errors
    /// When the subtree cannot be read.
    fn snapshot(&mut self, window: WindowHandle) -> Result<Vec<RawElement>, SysError>;
    /// Activates a snapshot element through a control pattern.
    ///
    /// # Errors
    /// When the element is gone or rejects the pattern.
    fn activate(&mut self, index: usize, activation: Activation) -> Result<(), SysError>;
    /// The virtual desktop bounds.
    fn virtual_screen(&mut self) -> Edges;
    /// The DPI of the monitor at a point.
    fn dpi_at(&mut self, x: i32, y: i32) -> u32;
    /// Injects one input batch.
    ///
    /// # Errors
    /// When the OS blocked the input.
    fn send_input(&mut self, steps: &[InputStep]) -> Result<(), SysError>;
    /// Shows `bitmap` in the overlay.
    ///
    /// # Errors
    /// When drawing fails.
    fn overlay_update(&mut self, bitmap: &Bitmap) -> Result<(), SysError>;
    /// Hides the overlay.
    fn overlay_hide(&mut self);
    /// The overlay window, so screen scope skips it.
    fn overlay_handle(&self) -> Option<WindowHandle>;
    /// Starts or stops the keyboard grab.
    ///
    /// # Errors
    /// When the hook cannot be installed.
    fn grab_keyboard(&mut self, grab: bool) -> Result<(), SysError>;
    /// Registers a global hotkey.
    ///
    /// # Errors
    /// When the chord is taken.
    fn register_hotkey(
        &mut self,
        id: i32,
        modifiers: HotkeyModifiers,
        vk: u32,
    ) -> Result<(), SysError>;
    /// Unregisters a global hotkey.
    ///
    /// # Errors
    /// When the OS rejects the call.
    fn unregister_hotkey(&mut self, id: i32) -> Result<(), SysError>;
    /// Pumps messages until an event or the timeout.
    fn next_event(&mut self, timeout: Duration) -> Option<SysEvent>;
}

fn backend_error(error: &SysError) -> PlatformError {
    match error.kind {
        SysErrorKind::AccessDenied => PlatformError::PermissionDenied(format!(
            "{error}; Windows blocks access to windows of elevated programs (UIPI) unless nomouse also runs elevated"
        )),
        _ => PlatformError::Backend(error.to_string()),
    }
}

/// One labelled element remembered for semantic clicks.
#[derive(Debug, Clone)]
struct Known {
    element: Element,
    index: usize,
    activation: Option<Activation>,
}

/// The Windows backend over a [`System`].
#[derive(Debug)]
pub struct Backend<S: System> {
    system: S,
    renderer: Renderer,
    options: ScopeOptions,
    semantic_click: bool,
    scroll_step: u32,
    snapshot: Vec<Known>,
    next_hotkey: u32,
    hotkeys: Vec<(HotkeyId, Hotkey)>,
    shown: bool,
}

impl<S: System> Backend<S> {
    /// Creates a backend over `system` with a label font from the Windows font directory,
    /// reading `semantic_click`, `[scroll] step` and `[overlay]` from `config`.
    ///
    /// # Errors
    /// [`PlatformError::Backend`] for invalid `[overlay]` colours or when no font is installed.
    pub fn new(system: S, config: &Config, options: ScopeOptions) -> Result<Self, PlatformError> {
        let renderer = Renderer::system(Style::from_config(&config.overlay)?)?;
        Ok(Self::with_renderer(system, renderer, config, options))
    }

    /// As [`Backend::new`] with the label font read from `font` (a TrueType file).
    ///
    /// # Errors
    /// [`PlatformError::Backend`] for invalid colours or an unreadable font.
    pub fn with_font(
        system: S,
        font: &Path,
        config: &Config,
        options: ScopeOptions,
    ) -> Result<Self, PlatformError> {
        let renderer = Renderer::from_file(font, Style::from_config(&config.overlay)?)?;
        Ok(Self::with_renderer(system, renderer, config, options))
    }

    fn with_renderer(
        system: S,
        renderer: Renderer,
        config: &Config,
        options: ScopeOptions,
    ) -> Self {
        Self {
            system,
            renderer,
            options,
            semantic_click: config.hints.semantic_click,
            scroll_step: config.scroll.step.clamp(1, 50),
            snapshot: Vec::new(),
            next_hotkey: 1,
            hotkeys: Vec::new(),
            shown: false,
        }
    }

    /// The scope options in effect.
    pub const fn scope_options(&self) -> ScopeOptions {
        self.options
    }

    /// Changes which optional parts the screen scope includes.
    pub const fn set_scope_options(&mut self, options: ScopeOptions) {
        self.options = options;
    }

    /// Shown top-level windows, topmost first.
    pub fn windows(&mut self) -> Vec<WindowInfo> {
        self.system.top_level_windows()
    }

    /// A snapshot of one window's elements (replacing the previous snapshot), for tools and
    /// tests that target a specific window rather than a [`Scope`].
    ///
    /// # Errors
    /// As [`ElementSource::elements`].
    pub fn window_elements(&mut self, window: WindowHandle) -> Result<Vec<Element>, PlatformError> {
        let target = scope::Target {
            handle: window,
            blocked: Vec::new(),
        };
        self.collect(&[target], Some(window))
    }

    /// Clicks `button` at `at` with `modifiers` held (for example ctrl+click to open a link in
    /// a new tab).
    ///
    /// # Errors
    /// [`PlatformError::PermissionDenied`] when UIPI blocks the input.
    pub fn modifier_click(
        &mut self,
        at: Point,
        button: MouseButton,
        modifiers: Modifiers,
    ) -> Result<(), PlatformError> {
        let screen = self.system.virtual_screen();
        self.run(&input_plan::modifier_click(at, button, modifiers, screen))
    }

    /// Snapshots every target; a failure on `required` is fatal, others skip their window
    /// (it closed or runs elevated) unless every window failed.
    fn collect(
        &mut self,
        targets: &[scope::Target],
        required: Option<WindowHandle>,
    ) -> Result<Vec<Element>, PlatformError> {
        self.system.begin_snapshot();
        self.snapshot.clear();
        let screen = self.system.virtual_screen();
        let mut last_error = None;
        let mut read_any = false;
        for target in targets {
            let raws = match self.system.snapshot(target.handle) {
                Ok(raws) => raws,
                Err(error) if Some(target.handle) == required => {
                    return Err(match error.kind {
                        SysErrorKind::AccessDenied => backend_error(&error),
                        SysErrorKind::Gone => PlatformError::NoFocusedWindow,
                        _ => PlatformError::AccessibilityUnavailable(error.to_string()),
                    });
                }
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            read_any = true;
            self.snapshot.extend(
                raws.iter()
                    .filter(|raw| classify::is_candidate(raw) && target.shows(raw, screen))
                    .map(|raw| Known {
                        element: classify::to_element(raw),
                        index: raw.index,
                        activation: classify::activation(raw),
                    }),
            );
        }
        if let (false, Some(error)) = (read_any, last_error) {
            return Err(PlatformError::AccessibilityUnavailable(error.to_string()));
        }
        Ok(self
            .snapshot
            .iter()
            .map(|known| known.element.clone())
            .collect())
    }

    fn run(&mut self, plan: &input_plan::Plan) -> Result<(), PlatformError> {
        for (position, batch) in plan.iter().enumerate() {
            if position > 0 {
                thread::sleep(BATCH_PAUSE);
            }
            self.system
                .send_input(batch)
                .map_err(|error| backend_error(&error))?;
        }
        Ok(())
    }

    /// Activates the remembered element targeted by a plain left click, if it has a pattern.
    fn try_semantic(&mut self, action: &Action) -> bool {
        let Action::Click {
            at,
            button: MouseButton::Left,
            count: 1,
        } = *action
        else {
            return false;
        };
        let Some((index, activation)) = self.snapshot.iter().find_map(|known| {
            (known.element.target() == at)
                .then_some(known.activation.map(|activation| (known.index, activation)))
                .flatten()
        }) else {
            return false;
        };
        // A failed pattern call falls back to a pointer click.
        self.system.activate(index, activation).is_ok()
    }
}

impl<S: System> ElementSource for Backend<S> {
    fn elements(&mut self, scope: Scope) -> Result<Vec<Element>, PlatformError> {
        match scope {
            Scope::FocusedWindow => {
                let foreground = self
                    .system
                    .foreground_window()
                    .ok_or(PlatformError::NoFocusedWindow)?;
                let windows = self.system.top_level_windows();
                let targets = scope::focused(foreground, &windows);
                self.collect(&targets, Some(foreground))
            }
            Scope::Screen => {
                let windows = self.system.top_level_windows();
                let tray = if self.options.include_taskbar && !self.options.include_tray {
                    self.system
                        .find_window(TASKBAR_CLASS)
                        .and_then(|taskbar| self.system.find_descendant(taskbar, TRAY_CLASS))
                        .and_then(|tray| self.system.window_info(tray).ok())
                        .map(|info| info.bounds)
                } else {
                    None
                };
                let overlay = self.system.overlay_handle();
                let targets = scope::screen(&windows, overlay, self.options, tray);
                self.collect(&targets, None)
            }
        }
    }
}

impl<S: System> Overlay for Backend<S> {
    fn show(&mut self, hints: &[Hint], typed: &str) -> Result<(), PlatformError> {
        let area = self.system.virtual_screen();
        let system = &mut self.system;
        let bitmap = self.renderer.frame(hints, typed, area, |x, y| {
            f32::from(u16::try_from(system.dpi_at(x, y)).unwrap_or(96)) / BASE_DPI
        });
        self.system
            .overlay_update(&bitmap)
            .map_err(|error| backend_error(&error))?;
        if !self.shown {
            self.system
                .grab_keyboard(true)
                .map_err(|error| backend_error(&error))?;
            self.shown = true;
        }
        Ok(())
    }

    fn hide(&mut self) -> Result<(), PlatformError> {
        if self.shown {
            self.system.overlay_hide();
            self.shown = false;
            self.system
                .grab_keyboard(false)
                .map_err(|error| backend_error(&error))?;
        }
        Ok(())
    }
}

impl<S: System> InputInjector for Backend<S> {
    fn perform(&mut self, action: &Action) -> Result<(), PlatformError> {
        if self.semantic_click && self.try_semantic(action) {
            return Ok(());
        }
        let screen = self.system.virtual_screen();
        self.run(&input_plan::plan(action, screen, self.scroll_step))
    }
}

impl<S: System> HotkeySource for Backend<S> {
    fn register(&mut self, hotkey: Hotkey) -> Result<HotkeyId, PlatformError> {
        let id = HotkeyId(self.next_hotkey);
        let raw = i32::try_from(id.0)
            .map_err(|_| PlatformError::Backend("too many hotkeys".to_owned()))?;
        let (modifiers, vk) = keys::hotkey_args(hotkey);
        self.system
            .register_hotkey(raw, modifiers, vk)
            .map_err(|error| match error.kind {
                SysErrorKind::HotkeyTaken => PlatformError::HotkeyTaken(hotkey),
                _ => backend_error(&error),
            })?;
        self.next_hotkey += 1;
        self.hotkeys.push((id, hotkey));
        Ok(id)
    }

    fn unregister(&mut self, id: HotkeyId) -> Result<(), PlatformError> {
        let Some(position) = self.hotkeys.iter().position(|(known, _)| *known == id) else {
            return Ok(());
        };
        self.hotkeys.swap_remove(position);
        let raw = i32::try_from(id.0).unwrap_or(i32::MAX);
        self.system
            .unregister_hotkey(raw)
            .map_err(|error| backend_error(&error))
    }

    fn next_event(&mut self, timeout: Duration) -> Result<Option<InputEvent>, PlatformError> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let Some(event) = self.system.next_event(remaining) else {
                return Ok(None);
            };
            let event = match event {
                SysEvent::Hotkey(raw) => u32::try_from(raw)
                    .ok()
                    .map(HotkeyId)
                    .filter(|id| self.hotkeys.iter().any(|(known, _)| known == id))
                    .map(InputEvent::Hotkey),
                SysEvent::Key { vk, shift } if self.shown => {
                    keys::key_press(vk, shift).map(InputEvent::Key)
                }
                SysEvent::Key { .. } => None,
            };
            if event.is_some() {
                return Ok(event);
            }
            if remaining.is_zero() {
                return Ok(None);
            }
        }
    }
}

impl<S: System> Platform for Backend<S> {
    fn name(&self) -> &'static str {
        "windows-uia"
    }
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
