//! The real Win32 [`System`], and the constructors the binary calls.

use std::time::Duration;

use nomouse_core::{Config, Platform, PlatformError};
use nomouse_sys_windows as sys;
use nomouse_sys_windows::{
    Activation, Automation, Bitmap, Edges, EventPump, HotkeyModifiers, InputStep, OverlayWindow,
    RawElement, SysError, SysEvent, WindowHandle, WindowInfo,
};

use crate::backend::{Backend, System};
use crate::render::{find_font, system_font_dir};
use crate::scope::ScopeOptions;

/// The Win32 system: UI Automation, the overlay window and this thread's event pump.
///
/// Thread-bound: create and use it on the thread running the event loop.
#[derive(Debug)]
pub struct NativeSystem {
    // Drop order: the overlay and pump first, the UIA client (and its COM apartment) last.
    overlay: OverlayWindow,
    pump: EventPump,
    automation: Automation,
}

impl NativeSystem {
    /// Enables per-monitor DPI awareness, then creates the UIA client, overlay and pump.
    ///
    /// # Errors
    /// [`PlatformError::AccessibilityUnavailable`] when UI Automation cannot be created,
    /// [`PlatformError::Backend`] when the overlay window cannot be created.
    pub fn new() -> Result<Self, PlatformError> {
        // Fails harmlessly when a manifest or an earlier call already chose the awareness.
        let _ = sys::enable_per_monitor_dpi();
        let automation = Automation::new()
            .map_err(|e| PlatformError::AccessibilityUnavailable(e.to_string()))?;
        let overlay = OverlayWindow::new().map_err(|e| PlatformError::Backend(e.to_string()))?;
        Ok(Self {
            overlay,
            pump: EventPump::new(),
            automation,
        })
    }
}

impl System for NativeSystem {
    fn foreground_window(&mut self) -> Option<WindowHandle> {
        sys::foreground_window()
    }

    fn top_level_windows(&mut self) -> Vec<WindowInfo> {
        sys::top_level_windows()
    }

    fn find_window(&mut self, class: &str) -> Option<WindowHandle> {
        sys::find_window(class)
    }

    fn find_descendant(&mut self, parent: WindowHandle, class: &str) -> Option<WindowHandle> {
        sys::find_descendant(parent, class)
    }

    fn window_info(&mut self, window: WindowHandle) -> Result<WindowInfo, SysError> {
        sys::window_info(window)
    }

    fn begin_snapshot(&mut self) {
        self.automation.clear();
    }

    fn snapshot(&mut self, window: WindowHandle) -> Result<Vec<RawElement>, SysError> {
        self.automation.snapshot(window)
    }

    fn activate(&mut self, index: usize, activation: Activation) -> Result<(), SysError> {
        self.automation.activate(index, activation)
    }

    fn virtual_screen(&mut self) -> Edges {
        sys::virtual_screen()
    }

    fn dpi_at(&mut self, x: i32, y: i32) -> u32 {
        sys::dpi_at(x, y)
    }

    fn send_input(&mut self, steps: &[InputStep]) -> Result<(), SysError> {
        sys::send_input(steps)
    }

    fn overlay_update(&mut self, bitmap: &Bitmap) -> Result<(), SysError> {
        self.overlay.update(bitmap)
    }

    fn overlay_hide(&mut self) {
        self.overlay.hide();
    }

    fn overlay_handle(&self) -> Option<WindowHandle> {
        Some(self.overlay.handle())
    }

    fn grab_keyboard(&mut self, grab: bool) -> Result<(), SysError> {
        self.pump.grab_keyboard(grab)
    }

    fn register_hotkey(
        &mut self,
        id: i32,
        modifiers: HotkeyModifiers,
        vk: u32,
    ) -> Result<(), SysError> {
        self.pump.register_hotkey(id, modifiers, vk)
    }

    fn unregister_hotkey(&mut self, id: i32) -> Result<(), SysError> {
        self.pump.unregister_hotkey(id)
    }

    fn next_event(&mut self, timeout: Duration) -> Option<SysEvent> {
        self.pump.next_event(timeout)
    }
}

/// The Windows backend on the real system.
pub type WindowsPlatform = Backend<NativeSystem>;

/// Creates the Windows backend with explicit screen-scope options.
///
/// # Errors
/// As [`NativeSystem::new`]; [`PlatformError::Backend`] for invalid `[overlay]` colours or a
/// missing font.
pub fn platform_with(
    config: &Config,
    options: ScopeOptions,
) -> Result<WindowsPlatform, PlatformError> {
    Backend::new(NativeSystem::new()?, config, options)
}

/// Creates the Windows backend for `config` (taskbar and tray included in screen scope).
///
/// # Errors
/// As [`platform_with`].
pub fn platform(config: &Config) -> Result<Box<dyn Platform>, PlatformError> {
    Ok(Box::new(platform_with(config, ScopeOptions::default())?))
}

/// Environment checks for `nomouse doctor`: `(check, Ok(detail) | Err(problem and fix))`.
pub fn doctor(config: &Config) -> Vec<(String, Result<String, String>)> {
    let mut checks = Vec::new();
    let _ = sys::enable_per_monitor_dpi();
    checks.push((
        "UI Automation".to_owned(),
        match Automation::new() {
            Ok(automation) if automation.uses_automation8() => {
                Ok("CUIAutomation8 client created".to_owned())
            }
            Ok(_) => Ok("CUIAutomation client created (CUIAutomation8 unavailable)".to_owned()),
            Err(error) => Err(format!(
                "{error}; UI Automation is part of Windows, so check that COM works for this user"
            )),
        },
    ));
    checks.push((
        "DPI awareness".to_owned(),
        match sys::dpi_awareness() {
            "per-monitor" => Ok("per-monitor (v2): coordinates are physical pixels".to_owned()),
            other => Err(format!(
                "{other}: labels and clicks may be offset on scaled monitors; remove any \
                 compatibility DPI override on nomouse.exe (Properties > Compatibility)"
            )),
        },
    ));
    checks.push((
        "Elevation".to_owned(),
        match sys::is_elevated() {
            Ok(true) => Ok("running elevated: every window is reachable".to_owned()),
            Ok(false) => Ok(
                "not elevated: windows of elevated programs (admin tools, installers) cannot be \
                 read or clicked (UIPI); run nomouse as administrator to reach them"
                    .to_owned(),
            ),
            Err(error) => Err(format!("cannot read the process token: {error}")),
        },
    ));
    let dir = system_font_dir();
    checks.push((
        "Label font".to_owned(),
        find_font(std::slice::from_ref(&dir)).map_or_else(
            || {
                Err(format!(
                    "no TrueType font in {}; reinstall Segoe UI or Arial",
                    dir.display()
                ))
            },
            |path| Ok(path.display().to_string()),
        ),
    ));
    checks.push((
        "Overlay colours".to_owned(),
        config
            .overlay
            .colours()
            .map(|_| "valid".to_owned())
            .map_err(|error| format!("{error}; use #rrggbb")),
    ));
    checks
}
