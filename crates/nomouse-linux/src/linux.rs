//! The assembled Linux X11 platform, its constructor and `nomouse doctor` checks.

use std::time::Duration;

use nomouse_core::{
    Action, Config, Element, ElementSource, Hint, Hotkey, HotkeyId, HotkeySource, InputEvent,
    InputInjector, Modifiers, MouseButton, Overlay, Platform, PlatformError, Scope,
};

use crate::a11y::{AtspiSource, ScanLimits, bus};
use crate::session::DisplayServer;
use crate::x11::render::{Renderer, Style, find_font};
use crate::x11::{self, X11};

/// Timeout of one AT-SPI call: a hung application must not freeze hint mode.
const CALL_TIMEOUT: Duration = Duration::from_millis(400);

/// The Linux X11 backend: AT-SPI elements, X11 overlay, XTEST input, XGrabKey hotkeys.
pub struct LinuxPlatform {
    x: X11,
    source: Option<AtspiSource>,
    semantic_click: bool,
}

impl LinuxPlatform {
    fn source(&mut self) -> Result<&mut AtspiSource, PlatformError> {
        if self.source.is_none() {
            self.source = Some(AtspiSource::connect(ScanLimits::default(), CALL_TIMEOUT)?);
        }
        self.source
            .as_mut()
            .ok_or_else(|| PlatformError::Backend("AT-SPI source missing".to_owned()))
    }

    /// Performs `action` while holding `modifiers` (Ctrl-click, Shift-click, ...).
    ///
    /// # Errors
    /// When the X connection is lost.
    pub fn perform_with_modifiers(
        &mut self,
        action: &Action,
        modifiers: Modifiers,
    ) -> Result<(), PlatformError> {
        self.x.perform(action, modifiers)
    }
}

impl ElementSource for LinuxPlatform {
    fn elements(&mut self, scope: Scope) -> Result<Vec<Element>, PlatformError> {
        let pid = self.x.active_pid();
        let screen = self.x.screen();
        let result = self.source()?.elements(scope, pid, screen);
        if matches!(result, Err(PlatformError::AccessibilityUnavailable(_))) {
            // The bus may have restarted; reconnect on the next scan.
            self.source = None;
        }
        result
    }
}

impl Overlay for LinuxPlatform {
    fn show(&mut self, hints: &[Hint], typed: &str) -> Result<(), PlatformError> {
        self.x.show(hints, typed)
    }

    fn hide(&mut self) -> Result<(), PlatformError> {
        self.x.hide()
    }
}

impl InputInjector for LinuxPlatform {
    fn perform(&mut self, action: &Action) -> Result<(), PlatformError> {
        if let Action::Click {
            at,
            button: MouseButton::Left,
            count: 1,
        } = *action
            && self.semantic_click
            && self.source.as_ref().is_some_and(|s| s.activate_at(at))
        {
            return Ok(());
        }
        self.x.perform(action, Modifiers::default())
    }
}

impl HotkeySource for LinuxPlatform {
    fn register(&mut self, hotkey: Hotkey) -> Result<HotkeyId, PlatformError> {
        self.x.register(hotkey)
    }

    fn unregister(&mut self, id: HotkeyId) -> Result<(), PlatformError> {
        self.x.unregister(id)
    }

    fn next_event(&mut self, timeout: Duration) -> Result<Option<InputEvent>, PlatformError> {
        self.x.next_event(timeout)
    }
}

impl Platform for LinuxPlatform {
    fn name(&self) -> &'static str {
        "linux-x11"
    }
}

/// Builds the backend for this session from `config`.
///
/// Turns on the session's accessibility switch (`org.a11y.Status.IsEnabled`) when it is off,
/// so Qt and Chromium apps started afterwards expose their trees.
///
/// # Errors
/// [`PlatformError::Unsupported`] on Wayland (until M4) or without a display or XTEST/SHAPE;
/// [`PlatformError::Backend`] for invalid overlay colours or a missing font.
pub fn platform(config: &Config) -> Result<Box<dyn Platform>, PlatformError> {
    DisplayServer::current().require_x11()?;
    let renderer = Renderer::new(Style::from_config(&config.overlay)?)?;
    let x = X11::connect(renderer)?;
    match bus::accessibility_enabled() {
        Ok(false) => match bus::enable_accessibility() {
            Ok(()) => tracing::info!("turned on the session accessibility switch"),
            Err(error) => tracing::warn!("cannot turn on accessibility: {error}"),
        },
        Ok(true) => {}
        Err(error) => tracing::warn!("{error}"),
    }
    let source = AtspiSource::connect(ScanLimits::default(), CALL_TIMEOUT)
        .inspect_err(|error| tracing::warn!("{error}"))
        .ok();
    Ok(Box::new(LinuxPlatform {
        x,
        source,
        semantic_click: config.hints.semantic_click,
    }))
}

fn check_display() -> Result<String, String> {
    let session = DisplayServer::current();
    session
        .require_x11()
        .map(|_| session.to_string())
        .map_err(|e| e.to_string())
}

fn check_x_server() -> Result<String, String> {
    let info = x11::probe().map_err(|e| e.to_string())?;
    let Some((major, minor)) = info.xtest else {
        return Err("XTEST is missing: synthetic clicks are impossible".to_owned());
    };
    if !info.shape {
        return Err("SHAPE is missing: the overlay cannot be click-through".to_owned());
    }
    let monitors = if info.monitors.is_empty() {
        "no RandR (one screen assumed)".to_owned()
    } else {
        let list: Vec<String> = info
            .monitors
            .iter()
            .map(|m| format!("{}x{}+{}+{}", m.width, m.height, m.x, m.y))
            .collect();
        format!("monitors {}", list.join(", "))
    };
    Ok(format!(
        "XTEST {major}.{minor}, SHAPE, {monitors}, {} visual",
        if info.argb { "ARGB" } else { "opaque" }
    ))
}

fn check_bus() -> Result<String, String> {
    let bus = bus::Bus::connect(CALL_TIMEOUT).map_err(|e| e.to_string())?;
    let apps = bus.applications().map_err(|e| e.to_string())?;
    Ok(format!(
        "reachable, {} accessible application(s) registered",
        apps.len()
    ))
}

fn check_switch() -> Result<String, String> {
    match bus::accessibility_enabled() {
        Ok(true) => Ok("on: GTK, Qt and Chromium apps expose their trees".to_owned()),
        Ok(false) => Err(
            "off: Qt and Chromium apps will not expose their trees; `nomouse daemon` \
             turns it on, or run `gsettings set org.gnome.desktop.interface toolkit-accessibility \
             true`"
                .to_owned(),
        ),
        Err(error) => Err(error.to_string()),
    }
}

fn check_toolkits() -> Result<String, String> {
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    if env("NO_AT_BRIDGE").is_some_and(|v| v != "0") {
        return Err("NO_AT_BRIDGE is set: GTK apps will not expose their trees".to_owned());
    }
    let qt = if env("QT_LINUX_ACCESSIBILITY_ALWAYS_ON").is_some() {
        "Qt forced on (QT_LINUX_ACCESSIBILITY_ALWAYS_ON)"
    } else {
        "Qt follows the accessibility switch"
    };
    Ok(format!(
        "GTK bridge allowed; {qt}; Chromium/Electron follow the switch or need \
         --force-renderer-accessibility"
    ))
}

fn check_font(config: &Config) -> Result<String, String> {
    Style::from_config(&config.overlay).map_err(|e| e.to_string())?;
    find_font()
        .map(|path| path.display().to_string())
        .ok_or_else(|| "no TrueType font under /usr/share/fonts".to_owned())
}

/// The `nomouse doctor` checks for this session: `(name, Ok(detail) | Err(problem))`.
pub fn doctor(config: &Config) -> Vec<(String, Result<String, String>)> {
    let display = check_display();
    let x_server = if display.is_ok() {
        check_x_server()
    } else {
        Err("skipped: no X11 session".to_owned())
    };
    vec![
        ("display server".to_owned(), display),
        ("X server".to_owned(), x_server),
        ("AT-SPI bus".to_owned(), check_bus()),
        ("accessibility switch".to_owned(), check_switch()),
        ("toolkits".to_owned(), check_toolkits()),
        ("label font".to_owned(), check_font(config)),
    ]
}
