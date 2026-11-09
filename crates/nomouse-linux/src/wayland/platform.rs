//! The assembled Wayland platform and its `nomouse doctor` checks.

use std::collections::HashMap;
use std::time::Duration;

use nomouse_core::{
    Action, Element, ElementSource, Frame, Hint, Hotkey, HotkeyId, HotkeySource, InputEvent,
    InputInjector, MouseButton, Overlay, Platform, PlatformError, Scope,
};

use super::Wayland;
use super::compositor::{Compositor, Windows};
use crate::a11y::AtspiSource;
use crate::appearance::Themed;

/// The Wayland backend: AT-SPI elements placed with the compositor's window positions, a
/// layer-shell overlay, virtual pointer or uinput input, and hotkeys bound by the compositor.
pub struct WaylandPlatform {
    wayland: Wayland,
    compositor: Compositor,
    source: Option<AtspiSource>,
    semantic_click: bool,
    themed: Themed,
}

impl WaylandPlatform {
    pub(crate) fn new(
        wayland: Wayland,
        compositor: Compositor,
        source: Option<AtspiSource>,
        semantic_click: bool,
        themed: Themed,
    ) -> Self {
        tracing::info!(
            "Wayland backend on {} with {}",
            compositor.name(),
            wayland.input_name()
        );
        Self {
            wayland,
            compositor,
            source,
            semantic_click,
            themed,
        }
    }

    fn windows(&self) -> Windows {
        if !self.compositor.locates_windows() {
            return Windows::default();
        }
        self.compositor.windows().unwrap_or_else(|error| {
            tracing::warn!("window positions: {error}");
            Windows::default()
        })
    }
}

/// The hotkey error on Wayland: clients cannot grab keys, the compositor binds the command.
pub(crate) fn hotkey_unsupported(hotkey: Hotkey) -> PlatformError {
    PlatformError::Unsupported(format!(
        "Wayland gives clients no global hotkeys: bind {hotkey} to `nomouse hints` (or \
         `continuous`, `scroll`, `search`) in the compositor, e.g. Sway: `bindsym {} exec \
         nomouse hints`",
        sway_chord(hotkey)
    ))
}

/// A hotkey in Sway's `bindsym` syntax (`ctrl+space` → `Ctrl+space`).
pub(crate) fn sway_chord(hotkey: Hotkey) -> String {
    let modifiers = hotkey.modifiers;
    let mut parts: Vec<String> = Vec::new();
    if modifiers.meta {
        parts.push("Mod4".to_owned());
    }
    if modifiers.ctrl {
        parts.push("Ctrl".to_owned());
    }
    if modifiers.alt {
        parts.push("Mod1".to_owned());
    }
    if modifiers.shift {
        parts.push("Shift".to_owned());
    }
    parts.push(keysym_name(hotkey.key));
    parts.join("+")
}

/// The XKB keysym name compositors use in bindings (`space`, `Tab`, `Return`, `F5`, `a`).
fn keysym_name(key: nomouse_core::Key) -> String {
    match key.to_string().as_str() {
        "tab" => "Tab".to_owned(),
        "enter" => "Return".to_owned(),
        "escape" => "Escape".to_owned(),
        name if name.len() > 1
            && name.starts_with('f')
            && name[1..].bytes().all(|b| b.is_ascii_digit()) =>
        {
            name.to_ascii_uppercase()
        }
        name => name.to_owned(),
    }
}

impl ElementSource for WaylandPlatform {
    fn elements(&mut self, scope: Scope) -> Result<Vec<Element>, PlatformError> {
        // Each activation starts with a scan: the moment to follow the system appearance.
        if let Some(style) = self.themed.changed() {
            self.wayland.set_style(style);
        }
        let windows = self.windows();
        let pid = windows.focused_pid();
        let offsets: HashMap<u32, nomouse_core::Point> = windows.native_offsets();
        tracing::debug!(
            "focused pid {pid:?}, {} window(s), native offsets {offsets:?}",
            windows.all.len()
        );
        let screen = self.wayland.screen();
        if self.source.is_none() {
            self.source = Some(AtspiSource::connect(
                crate::a11y::ScanLimits::default(),
                crate::linux::CALL_TIMEOUT,
            )?);
        }
        let Some(source) = self.source.as_mut() else {
            return Err(PlatformError::Backend("AT-SPI source missing".to_owned()));
        };
        let result = source.elements(scope, pid, screen, &offsets);
        if matches!(result, Err(PlatformError::AccessibilityUnavailable(_))) {
            // The bus may have restarted; reconnect on the next scan.
            self.source = None;
        }
        result
    }

    fn focused_app(&mut self) -> Result<Option<nomouse_core::AppIdentity>, PlatformError> {
        Ok(self.windows().focused().map(|window| {
            let mut app = window.app.clone();
            app.process = crate::linux::process_name(window.pid);
            app
        }))
    }
}

impl Overlay for WaylandPlatform {
    fn show(&mut self, hints: &[Hint], typed: &str) -> Result<(), PlatformError> {
        self.wayland.show(&Frame {
            hints,
            typed,
            ..Frame::default()
        })
    }

    fn show_frame(&mut self, frame: &Frame<'_>) -> Result<(), PlatformError> {
        self.wayland.show(frame)
    }

    fn hide(&mut self) -> Result<(), PlatformError> {
        self.wayland.hide()
    }
}

impl InputInjector for WaylandPlatform {
    /// Clicks hold [`Action::modifiers`] with virtual keyboard (or uinput) key presses; the
    /// semantic AT-SPI click is used only for a plain left click.
    fn perform(&mut self, action: &Action) -> Result<(), PlatformError> {
        if let Action::Click {
            at,
            button: MouseButton::Left,
            count: 1,
            modifiers,
        } = *action
            && modifiers.is_empty()
            && self.semantic_click
            && self.source.as_ref().is_some_and(|s| s.activate_at(at))
        {
            return Ok(());
        }
        tracing::debug!("injecting {action:?}");
        self.wayland.perform(action)
    }
}

impl HotkeySource for WaylandPlatform {
    fn register(&mut self, hotkey: Hotkey) -> Result<HotkeyId, PlatformError> {
        Err(hotkey_unsupported(hotkey))
    }

    fn unregister(&mut self, _id: HotkeyId) -> Result<(), PlatformError> {
        Ok(())
    }

    fn next_event(&mut self, timeout: Duration) -> Result<Option<InputEvent>, PlatformError> {
        self.wayland.next_event(timeout)
    }
}

impl Platform for WaylandPlatform {
    fn name(&self) -> &'static str {
        "linux-wayland"
    }
}

/// The compositor-specific checks of `nomouse doctor` on Wayland.
pub(crate) fn doctor(hints_hotkey: Hotkey) -> Vec<(String, Result<String, String>)> {
    let compositor = Compositor::current();
    let globals = super::probe().map_err(|e| e.to_string());
    vec![
        ("compositor".to_owned(), check_compositor(&compositor)),
        ("overlay".to_owned(), check_overlay(&compositor, &globals)),
        ("input".to_owned(), check_input(&globals)),
        ("window positions".to_owned(), window_positions(&compositor)),
        (
            "hotkeys".to_owned(),
            Ok(hotkey_advice(&compositor, hints_hotkey)),
        ),
    ]
}

fn check_compositor(compositor: &Compositor) -> Result<String, String> {
    match compositor {
        Compositor::Gnome => Err("GNOME (Mutter) implements no wlr-layer-shell, virtual \
                                  pointer or client hotkeys, so nomouse cannot draw labels or \
                                  click on GNOME Wayland; log out and choose \"GNOME on Xorg\" \
                                  (Ubuntu: \"Ubuntu on Xorg\") under the gear on the login \
                                  screen"
            .to_owned()),
        other => Ok(format!(
            "{}{}",
            other.name(),
            other.version().map(|v| format!(" {v}")).unwrap_or_default()
        )),
    }
}

fn check_overlay(
    compositor: &Compositor,
    globals: &Result<super::Globals, String>,
) -> Result<String, String> {
    let g = globals.as_ref().map_err(Clone::clone)?;
    let Some(version) = g.layer_shell else {
        return Err(format!(
            "{} has no zwlr_layer_shell_v1: the overlay cannot be drawn",
            compositor.name()
        ));
    };
    let outputs: Vec<String> = g
        .outputs
        .iter()
        .map(|(name, r)| format!("{name} {}x{}+{}+{}", r.width, r.height, r.x, r.y))
        .collect();
    Ok(format!(
        "zwlr_layer_shell_v1 v{version} (keyboard-exclusive while labels are up); outputs {}{}",
        outputs.join(", "),
        if g.xdg_output.is_some() {
            ""
        } else {
            " (no xdg-output: positions from wl_output)"
        }
    ))
}

fn check_input(globals: &Result<super::Globals, String>) -> Result<String, String> {
    let Ok(g) = globals else {
        return Err("skipped: no Wayland connection".to_owned());
    };
    match (g.virtual_pointer, g.virtual_keyboard) {
        (Some(p), Some(k)) => Ok(format!(
            "zwlr_virtual_pointer_v1 v{p}, zwp_virtual_keyboard_v1 v{k}"
        )),
        (Some(p), None) => Ok(format!(
            "zwlr_virtual_pointer_v1 v{p}; no virtual keyboard, so modifier clicks \
             (ctrl+click) are plain clicks"
        )),
        (None, _) => super::uinput_writable()
            .map(|()| {
                "no wlr virtual pointer; /dev/uinput is writable and is used instead".to_owned()
            })
            .map_err(|e| {
                format!(
                    "no wlr virtual pointer and {e}: install the udev rule `KERNEL==\"uinput\", \
                     SUBSYSTEM==\"misc\", TAG+=\"uaccess\", OPTIONS+=\"static_node=uinput\"` as \
                     /etc/udev/rules.d/70-nomouse-uinput.rules, then `sudo udevadm control \
                     --reload && sudo udevadm trigger` and log in again"
                )
            }),
    }
}

fn hotkey_advice(compositor: &Compositor, hints_hotkey: Hotkey) -> String {
    match compositor {
        Compositor::Sway(_) => format!(
            "bound by Sway: add `bindsym {} exec nomouse hints` (and `continuous`, `scroll`, \
             `search`) to ~/.config/sway/config; the commands reach the running daemon",
            sway_chord(hints_hotkey)
        ),
        Compositor::Hyprland(_) => format!(
            "bound by Hyprland: add `bind = {}, exec, nomouse hints` to hyprland.conf",
            hyprland_chord(hints_hotkey)
        ),
        Compositor::Kde => {
            "bound by KDE: System Settings > Shortcuts > Add New > Command: `nomouse hints`"
                .to_owned()
        }
        other => format!(
            "Wayland has no client hotkeys: bind `nomouse hints` in {}",
            other.name()
        ),
    }
}

fn window_positions(compositor: &Compositor) -> Result<String, String> {
    let limitation = "AT-SPI gives native Wayland windows' elements relative to the window, \
                      not the screen";
    if !compositor.locates_windows() {
        return Err(format!(
            "{limitation}, and {} exposes no window positions to clients: only XWayland apps \
             are labelled at the right place",
            compositor.name()
        ));
    }
    match compositor.windows() {
        Ok(windows) => {
            let native = windows.all.iter().filter(|w| w.native).count();
            Ok(format!(
                "{limitation}; nomouse adds each native window's position from {} IPC \
                 ({} windows, {native} native, {} XWayland)",
                compositor.name(),
                windows.all.len(),
                windows.all.len() - native
            ))
        }
        Err(error) => Err(format!("{limitation}; the compositor IPC failed: {error}")),
    }
}

/// A hotkey in Hyprland's `bind` syntax (`CTRL, space`).
fn hyprland_chord(hotkey: Hotkey) -> String {
    let modifiers = hotkey.modifiers;
    let mut mods: Vec<&str> = Vec::new();
    if modifiers.meta {
        mods.push("SUPER");
    }
    if modifiers.ctrl {
        mods.push("CTRL");
    }
    if modifiers.alt {
        mods.push("ALT");
    }
    if modifiers.shift {
        mods.push("SHIFT");
    }
    format!("{}, {}", mods.join(" "), keysym_name(hotkey.key))
}

#[cfg(test)]
mod tests {
    use nomouse_core::{Key, Modifiers};

    use super::*;

    fn chord(ctrl: bool, alt: bool, shift: bool, key: Key) -> Hotkey {
        Hotkey {
            modifiers: Modifiers {
                ctrl,
                alt,
                shift,
                meta: false,
            },
            key,
        }
    }

    #[test]
    fn compositor_chords() {
        assert_eq!(
            sway_chord(chord(true, false, false, Key::Space)),
            "Ctrl+space"
        );
        assert_eq!(
            sway_chord(chord(true, true, false, Key::Slash)),
            "Ctrl+Mod1+slash"
        );
        assert_eq!(
            hyprland_chord(chord(true, false, true, Key::Space)),
            "CTRL SHIFT, space"
        );
        let error = hotkey_unsupported(chord(true, false, false, Key::Space));
        assert!(
            matches!(&error, PlatformError::Unsupported(m) if m.contains("bindsym Ctrl+space exec nomouse hints"))
        );
    }
}
