//! Who owns the global shortcuts in this session, how to bind nomouse's modes there, and
//! what the session means for nomouse (the warning menus and the settings window show).
//!
//! On X11 nomouse grabs the `[hotkeys]` chords itself. Wayland gives programs no global
//! hotkeys, so the compositor binds `nomouse hints` (and the other modes); GNOME on Wayland
//! cannot run nomouse at all.

use std::fmt::Write as _;

use nomouse_core::config::HotkeysConfig;
use nomouse_core::{Hotkey, Key};

use crate::session::DisplayServer;

/// The session's shortcut owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shortcuts {
    /// X11: nomouse grabs the keys.
    Grabbed,
    /// Sway: `bindsym` lines in `~/.config/sway/config`.
    Sway,
    /// Hyprland: `bind` lines in `~/.config/hypr/hyprland.conf`.
    Hyprland,
    /// KDE Plasma on Wayland: custom shortcuts in System Settings.
    Kde,
    /// GNOME on Wayland: unsupported. `ubuntu` names the Ubuntu session ("Ubuntu on Xorg").
    Gnome {
        /// Whether this is Ubuntu's GNOME session.
        ubuntu: bool,
    },
    /// Another Wayland compositor, by its `XDG_CURRENT_DESKTOP` (may be empty).
    Other(String),
    /// No graphical session.
    NoSession,
}

/// The command each mode's binding runs, with its `[hotkeys]` entry.
fn modes(hotkeys: &HotkeysConfig) -> [(&'static str, Hotkey); 4] {
    [
        ("nomouse hints", hotkeys.hints),
        ("nomouse continuous", hotkeys.continuous),
        ("nomouse scroll", hotkeys.scroll),
        ("nomouse search", hotkeys.search),
    ]
}

/// The xkb keysym name both Sway and Hyprland use for `key`.
fn keysym(key: Key) -> String {
    match key {
        Key::Char(c) => c.to_string(),
        Key::F(n) => format!("F{n}"),
        Key::Space => "space".to_owned(),
        Key::Tab => "Tab".to_owned(),
        Key::Enter => "Return".to_owned(),
        Key::Escape => "Escape".to_owned(),
        Key::Backspace => "BackSpace".to_owned(),
        Key::Semicolon => "semicolon".to_owned(),
        Key::Comma => "comma".to_owned(),
        Key::Period => "period".to_owned(),
        Key::Slash => "slash".to_owned(),
    }
}

/// `Ctrl+Mod1+space`: a Sway `bindsym` chord.
fn sway_chord(hotkey: Hotkey) -> String {
    let m = hotkey.modifiers;
    let mut parts: Vec<String> = [
        (m.ctrl, "Ctrl"),
        (m.shift, "Shift"),
        (m.alt, "Mod1"),
        (m.meta, "Mod4"),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, name)| name.to_owned())
    .collect();
    parts.push(keysym(hotkey.key));
    parts.join("+")
}

/// `CTRL ALT, space`: a Hyprland `bind` modifier list and key.
fn hyprland_chord(hotkey: Hotkey) -> String {
    let m = hotkey.modifiers;
    let mods: Vec<&str> = [
        (m.ctrl, "CTRL"),
        (m.shift, "SHIFT"),
        (m.alt, "ALT"),
        (m.meta, "SUPER"),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, name)| name)
    .collect();
    format!("{}, {}", mods.join(" "), keysym(hotkey.key))
}

impl Shortcuts {
    /// Classifies the session from the environment.
    pub fn detect(env: impl Fn(&str) -> Option<String>) -> Self {
        let get = |name: &str| env(name).filter(|value| !value.is_empty());
        match DisplayServer::detect(&env) {
            DisplayServer::X11(_) => return Self::Grabbed,
            DisplayServer::None => return Self::NoSession,
            DisplayServer::Wayland(_) => {}
        }
        if get("SWAYSOCK").is_some() {
            return Self::Sway;
        }
        if get("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            return Self::Hyprland;
        }
        let desktop = get("XDG_CURRENT_DESKTOP").unwrap_or_default();
        let parts: Vec<String> = desktop.split(':').map(str::to_ascii_uppercase).collect();
        if parts.iter().any(|part| part == "GNOME") {
            return Self::Gnome {
                ubuntu: parts.iter().any(|part| part == "UBUNTU"),
            };
        }
        if parts.iter().any(|part| part == "KDE") {
            return Self::Kde;
        }
        Self::Other(desktop)
    }

    /// Classifies this process's session.
    pub fn current() -> Self {
        Self::detect(|name| std::env::var(name).ok())
    }

    /// Whether nomouse grabs the `[hotkeys]` chords itself (X11).
    pub const fn grabs_keys(&self) -> bool {
        matches!(self, Self::Grabbed)
    }

    /// What is wrong with this session for nomouse, in one line for a menu or a banner, or
    /// `None` when it is fully supported.
    pub fn warning(&self) -> Option<String> {
        match self {
            Self::Grabbed | Self::Sway | Self::Hyprland | Self::Other(_) => None,
            Self::Gnome { ubuntu } => Some(format!(
                "GNOME on Wayland can't show labels: log out, and pick “{}” under the gear on \
                 the login screen",
                if *ubuntu {
                    "Ubuntu on Xorg"
                } else {
                    "GNOME on Xorg"
                }
            )),
            Self::Kde => Some(
                "KDE on Wayland: clicks need /dev/uinput access, and only XWayland apps are \
                 placed exactly"
                    .to_owned(),
            ),
            Self::NoSession => {
                Some("No graphical session: DISPLAY and WAYLAND_DISPLAY are unset".to_owned())
            }
        }
    }

    /// How to bind nomouse's modes in this session's compositor, using the chords in
    /// `hotkeys`; `None` on X11 (nomouse grabs them) and where nomouse cannot run.
    pub fn instructions(&self, hotkeys: &HotkeysConfig) -> Option<String> {
        let modes = modes(hotkeys);
        match self {
            Self::Grabbed | Self::Gnome { .. } | Self::NoSession => None,
            Self::Sway => {
                let mut text = String::from("Add to ~/.config/sway/config, then reload Sway:\n\n");
                text.push_str("exec nomouse daemon\n");
                for (command, hotkey) in modes {
                    let _ = writeln!(text, "bindsym {} exec {command}", sway_chord(hotkey));
                }
                Some(text)
            }
            Self::Hyprland => {
                let mut text = String::from("Add to ~/.config/hypr/hyprland.conf:\n\n");
                text.push_str("exec-once = nomouse daemon\n");
                for (command, hotkey) in modes {
                    let _ = writeln!(text, "bind = {}, exec, {command}", hyprland_chord(hotkey));
                }
                Some(text)
            }
            Self::Kde | Self::Other(_) => {
                let mut text = String::from(
                    "Wayland gives apps no global hotkeys. Add a custom shortcut in your \
                     desktop's keyboard settings for each command:\n\n",
                );
                for (command, hotkey) in modes {
                    let _ = writeln!(text, "{}  →  {command}", hotkey.human());
                }
                Some(text)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| {
            owned
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn classifies_sessions() {
        assert_eq!(
            Shortcuts::detect(env(&[("DISPLAY", ":0")])),
            Shortcuts::Grabbed
        );
        assert_eq!(Shortcuts::detect(env(&[])), Shortcuts::NoSession);
        let wayland = |extra: (&str, &str)| {
            Shortcuts::detect(env(&[
                ("WAYLAND_DISPLAY", "wayland-1"),
                ("DISPLAY", ":0"),
                extra,
            ]))
        };
        assert_eq!(wayland(("SWAYSOCK", "/run/sway")), Shortcuts::Sway);
        assert_eq!(
            wayland(("HYPRLAND_INSTANCE_SIGNATURE", "x")),
            Shortcuts::Hyprland
        );
        assert_eq!(
            wayland(("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")),
            Shortcuts::Gnome { ubuntu: true }
        );
        assert_eq!(
            wayland(("XDG_CURRENT_DESKTOP", "GNOME")),
            Shortcuts::Gnome { ubuntu: false }
        );
        assert_eq!(wayland(("XDG_CURRENT_DESKTOP", "KDE")), Shortcuts::Kde);
        assert_eq!(
            wayland(("XDG_CURRENT_DESKTOP", "river")),
            Shortcuts::Other("river".to_owned())
        );
    }

    #[test]
    fn only_x11_grabs_and_gnome_wayland_names_the_xorg_session() {
        assert!(Shortcuts::Grabbed.grabs_keys());
        assert!(!Shortcuts::Sway.grabs_keys());
        assert_eq!(Shortcuts::Grabbed.warning(), None);
        assert_eq!(Shortcuts::Sway.warning(), None);
        assert!(
            Shortcuts::Gnome { ubuntu: true }
                .warning()
                .is_some_and(|w| w.contains("Ubuntu on Xorg"))
        );
        assert!(
            Shortcuts::Gnome { ubuntu: false }
                .warning()
                .is_some_and(|w| w.contains("GNOME on Xorg"))
        );
    }

    #[test]
    fn compositor_bindings_use_the_configured_chords() {
        let hotkeys = HotkeysConfig {
            hints: "super+shift+j".parse().unwrap(),
            ..HotkeysConfig::defaults(nomouse_core::Flavour::Release)
        };
        let sway = Shortcuts::Sway.instructions(&hotkeys).unwrap();
        assert!(sway.contains("bindsym Shift+Mod4+j exec nomouse hints\n"));
        assert!(sway.contains("bindsym Ctrl+Mod1+slash exec nomouse search\n"));
        let hyprland = Shortcuts::Hyprland.instructions(&hotkeys).unwrap();
        assert!(hyprland.contains("bind = SHIFT SUPER, j, exec, nomouse hints\n"));
        assert!(hyprland.contains("bind = CTRL SHIFT, space, exec, nomouse continuous\n"));
        let kde = Shortcuts::Kde.instructions(&hotkeys).unwrap();
        assert!(kde.contains("Shift+Super+J  →  nomouse hints"));
        assert_eq!(Shortcuts::Grabbed.instructions(&hotkeys), None);
        assert_eq!(
            Shortcuts::Gnome { ubuntu: true }.instructions(&hotkeys),
            None
        );
    }
}
