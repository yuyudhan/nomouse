//! User configuration: TOML with defaults for every field, validated after parsing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::action::ActionKind;
use crate::hints::{Alphabet, AlphabetError, DEFAULT_ALPHABET};
use crate::hotkey::{Hotkey, Key, Modifiers};
use crate::platform::Scope;
use crate::session::KeyBindings;

/// The whole configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Hint labels.
    pub hints: HintsConfig,
    /// Global hotkeys.
    pub hotkeys: HotkeysConfig,
    /// Scroll mode.
    pub scroll: ScrollConfig,
    /// Overlay appearance.
    pub overlay: OverlayConfig,
    /// macOS-only settings; ignored on other systems.
    pub macos: MacosConfig,
}

/// `[macos]`
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MacosConfig {
    /// Bundle ids (such as `com.google.Chrome`) whose accessibility tree nomouse switches on
    /// with `AXEnhancedUserInterface`. Chromium and Electron apps build their tree only when
    /// asked; the attribute can slow window animations, so it is opt-in per app.
    pub enhanced_ui_apps: Vec<String>,
}

/// `[hints]`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HintsConfig {
    /// Label characters, most preferred first.
    pub alphabet: String,
    /// Where elements are collected from.
    pub scope: Scope,
    /// Action when a hint is picked after the main hotkey.
    pub action: ActionKind,
    /// Click through the accessibility action (AT-SPI `do_action`, UIA `Invoke`, `AXPress`)
    /// when the element offers one, instead of a synthetic pointer click.
    pub semantic_click: bool,
    /// Which keys act on a hint: `nomouse` (a typed label acts at once) or `homerow` (a typed
    /// label highlights; Return clicks, Return twice double clicks, Shift-Return right clicks,
    /// Cmd-Return cmd-clicks).
    pub keys: KeyBindings,
}

impl Default for HintsConfig {
    fn default() -> Self {
        Self {
            alphabet: DEFAULT_ALPHABET.to_owned(),
            scope: Scope::FocusedWindow,
            action: ActionKind::Click,
            semantic_click: false,
            keys: KeyBindings::Nomouse,
        }
    }
}

/// `[hotkeys]`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HotkeysConfig {
    /// Shows hints.
    pub hints: Hotkey,
    /// Shows hints in continuous mode: re-labels after every click until Escape.
    pub continuous: Hotkey,
    /// Shows hints for a scroll target.
    pub scroll: Hotkey,
    /// Search elements by text.
    pub search: Hotkey,
}

const fn chord(ctrl: bool, alt: bool, shift: bool, key: Key) -> Hotkey {
    Hotkey::new(
        Modifiers {
            ctrl,
            alt,
            shift,
            meta: false,
        },
        key,
    )
}

impl Default for HotkeysConfig {
    fn default() -> Self {
        Self {
            hints: chord(true, false, false, Key::Space),
            continuous: chord(true, false, true, Key::Space),
            scroll: chord(true, true, false, Key::Space),
            search: chord(true, true, false, Key::Slash),
        }
    }
}

/// `[scroll]`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScrollConfig {
    /// Wheel steps per key press, 1 to 50.
    pub step: u32,
}

impl Default for ScrollConfig {
    fn default() -> Self {
        Self { step: 3 }
    }
}

/// `[overlay]`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OverlayConfig {
    /// Label font size in points, 6 to 72.
    pub font_size: u32,
    /// Label opacity in percent, 10 to 100.
    pub opacity: u32,
    /// Label background, `#rrggbb`.
    pub background: String,
    /// Label text, `#rrggbb`.
    pub foreground: String,
    /// Already-typed label characters, `#rrggbb`.
    pub typed_foreground: String,
    /// Label border, `#rrggbb`.
    pub border: String,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            font_size: 13,
            opacity: 95,
            background: "#ffd75f".to_owned(),
            foreground: "#1c1c1c".to_owned(),
            typed_foreground: "#8a8a8a".to_owned(),
            border: "#1c1c1c".to_owned(),
        }
    }
}

impl OverlayConfig {
    /// The colours as RGB triples: background, foreground, typed foreground, border.
    ///
    /// # Errors
    /// [`ConfigError::Colour`] for the first field that is not `#rrggbb`.
    pub fn colours(&self) -> Result<[[u8; 3]; 4], ConfigError> {
        let parse = |field: &'static str, value: &str| {
            parse_colour(value).ok_or_else(|| ConfigError::Colour {
                field,
                value: value.to_owned(),
            })
        };
        Ok([
            parse("overlay.background", &self.background)?,
            parse("overlay.foreground", &self.foreground)?,
            parse("overlay.typed_foreground", &self.typed_foreground)?,
            parse("overlay.border", &self.border)?,
        ])
    }
}

/// Parses `#rrggbb` (case-insensitive) into an RGB triple.
pub fn parse_colour(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// Why a configuration could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
    /// The TOML is malformed, has unknown keys, or a value of the wrong type.
    #[error("invalid config: {0}")]
    Parse(#[from] toml::de::Error),
    /// `hints.alphabet` is not a valid alphabet.
    #[error("hints.alphabet: {0}")]
    Alphabet(#[from] AlphabetError),
    /// Two hotkeys use the same chord.
    #[error("hotkeys.{first} and hotkeys.{second} are both {hotkey}")]
    DuplicateHotkey {
        /// First field.
        first: &'static str,
        /// Second field.
        second: &'static str,
        /// The shared chord.
        hotkey: Hotkey,
    },
    /// A number is outside its range.
    #[error("{field} = {value} is outside {min}..={max}")]
    OutOfRange {
        /// The field.
        field: &'static str,
        /// The given value.
        value: u32,
        /// Smallest allowed.
        min: u32,
        /// Largest allowed.
        max: u32,
    },
    /// A colour is not `#rrggbb`.
    #[error("{field} = {value:?} is not a #rrggbb colour")]
    Colour {
        /// The field.
        field: &'static str,
        /// The given value.
        value: String,
    },
}

fn check_range(field: &'static str, value: u32, min: u32, max: u32) -> Result<(), ConfigError> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::OutOfRange {
            field,
            value,
            min,
            max,
        })
    }
}

impl Config {
    /// Parses and validates TOML; missing fields take their defaults.
    ///
    /// # Errors
    /// [`ConfigError::Parse`] for malformed TOML, otherwise the first [`Config::validate`] error.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    /// Reads and validates the file at `path`.
    ///
    /// # Errors
    /// [`ConfigError::Io`] when unreadable, otherwise as [`Config::from_toml`].
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_owned(),
            source,
        })?;
        Self::from_toml(&text)
    }

    /// Serialises the configuration as TOML.
    pub fn to_toml(&self) -> String {
        // Every field is a string, enum or integer, so serialisation cannot fail.
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Checks values that the types alone do not constrain.
    ///
    /// # Errors
    /// The first invalid field.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.alphabet()?;
        let keys = [
            ("hints", self.hotkeys.hints),
            ("continuous", self.hotkeys.continuous),
            ("scroll", self.hotkeys.scroll),
            ("search", self.hotkeys.search),
        ];
        for (i, (first, hotkey)) in keys.iter().enumerate() {
            if let Some((second, _)) = keys[i + 1..].iter().find(|(_, other)| other == hotkey) {
                return Err(ConfigError::DuplicateHotkey {
                    first,
                    second,
                    hotkey: *hotkey,
                });
            }
        }
        check_range("scroll.step", self.scroll.step, 1, 50)?;
        check_range("overlay.font_size", self.overlay.font_size, 6, 72)?;
        check_range("overlay.opacity", self.overlay.opacity, 10, 100)?;
        self.overlay.colours().map(|_| ())
    }

    /// The validated label alphabet.
    ///
    /// # Errors
    /// [`ConfigError::Alphabet`] when `hints.alphabet` is invalid.
    pub fn alphabet(&self) -> Result<Alphabet, ConfigError> {
        Ok(Alphabet::new(&self.hints.alphabet)?)
    }
}

/// The conventional config file path, from the environment: on Linux and macOS
/// `$XDG_CONFIG_HOME/nomouse/config.toml`, else `~/.config/nomouse/config.toml`; on Windows
/// `%APPDATA%\nomouse\config.toml`. `None` when the variables are unset.
pub fn default_path() -> Option<PathBuf> {
    path_from_env(cfg!(target_os = "windows"), |name| std::env::var_os(name))
}

/// [`default_path`] over an explicit environment lookup, so every OS's rule is testable on any
/// host. Empty values count as unset, as the XDG Base Directory specification requires.
fn path_from_env(
    windows: bool,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    let env = |name| {
        lookup(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let base = if windows {
        env("APPDATA")?
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|home| home.join(".config")))?
    };
    Some(base.join("nomouse").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_defaults() {
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
    }

    #[test]
    fn defaults_round_trip() {
        let text = Config::default().to_toml();
        assert!(text.contains("hints = \"ctrl+space\""));
        assert!(text.contains("continuous = \"ctrl+shift+space\""));
        assert!(text.contains("semantic_click = false"));
        assert!(text.contains("keys = \"nomouse\""));
        assert_eq!(Config::from_toml(&text).unwrap(), Config::default());
    }

    #[test]
    fn partial_override() {
        let config = Config::from_toml(
            "[hints]\nalphabet = \"jkfd\"\naction = \"right-click\"\nscope = \"screen\"\n[scroll]\nstep = 5\n",
        )
        .unwrap();
        assert_eq!(config.hints.action, ActionKind::RightClick);
        assert_eq!(config.hints.scope, Scope::Screen);
        assert_eq!(config.scroll.step, 5);
        assert_eq!(config.overlay, OverlayConfig::default());
    }

    #[test]
    fn homerow_key_bindings() {
        let config = Config::from_toml("[hints]\nkeys = \"homerow\"\n").unwrap();
        assert_eq!(config.hints.keys, KeyBindings::Homerow);
        assert!(matches!(
            Config::from_toml("[hints]\nkeys = \"vim\"\n"),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn rejects_invalid() {
        assert!(matches!(
            Config::from_toml("[hints]\nalphabet = \"al\"\n"),
            Err(ConfigError::Alphabet(_))
        ));
        assert!(matches!(
            Config::from_toml("[hints]\ncolour = 1\n"),
            Err(ConfigError::Parse(_))
        ));
        assert!(matches!(
            Config::from_toml("[hotkeys]\nhints = \"space\"\n"),
            Err(ConfigError::Parse(_))
        ));
        assert!(matches!(
            Config::from_toml("[hotkeys]\nscroll = \"ctrl+space\"\n"),
            Err(ConfigError::DuplicateHotkey {
                first: "hints",
                second: "scroll",
                ..
            })
        ));
        assert!(matches!(
            Config::from_toml("[overlay]\nopacity = 5\n"),
            Err(ConfigError::OutOfRange {
                field: "overlay.opacity",
                ..
            })
        ));
        assert!(matches!(
            Config::from_toml("[scroll]\nstep = 0\n"),
            Err(ConfigError::OutOfRange { .. })
        ));
        assert!(matches!(
            Config::from_toml("[overlay]\nborder = \"red\"\n"),
            Err(ConfigError::Colour {
                field: "overlay.border",
                ..
            })
        ));
    }

    #[test]
    fn colours_parse() {
        assert_eq!(parse_colour("#FFd75f"), Some([255, 215, 95]));
        assert_eq!(parse_colour("ffd75f"), None);
        assert_eq!(parse_colour("#ffd75"), None);
        assert_eq!(parse_colour("#gggggg"), None);
        assert_eq!(
            OverlayConfig::default().colours().unwrap()[0],
            [255, 215, 95]
        );
    }

    #[test]
    fn macos_enhanced_ui_apps() {
        assert_eq!(Config::default().macos, MacosConfig::default());
        assert_eq!(Config::default().macos.enhanced_ui_apps.len(), 0);
        let config =
            Config::from_toml("[macos]\nenhanced_ui_apps = [\"com.google.Chrome\"]\n").unwrap();
        assert_eq!(config.macos.enhanced_ui_apps, ["com.google.Chrome"]);
        assert!(Config::from_toml("[macos]\nunknown = 1\n").is_err());
    }

    #[test]
    fn load_reports_missing_file() {
        let missing = Path::new("/nonexistent/nomouse/config.toml");
        assert!(matches!(Config::load(missing), Err(ConfigError::Io { .. })));
    }

    fn lookup<'a>(
        vars: &'a [(&'a str, &'a str)],
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
        move |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.into())
        }
    }

    #[test]
    fn unix_path_prefers_xdg_config_home() {
        let vars = [("XDG_CONFIG_HOME", "/cfg"), ("HOME", "/home/u")];
        assert_eq!(
            path_from_env(false, lookup(&vars)),
            Some(PathBuf::from("/cfg/nomouse/config.toml"))
        );
    }

    #[test]
    fn unix_path_falls_back_to_dot_config_when_xdg_unset_or_empty() {
        let expected = Some(PathBuf::from("/home/u/.config/nomouse/config.toml"));
        assert_eq!(
            path_from_env(false, lookup(&[("HOME", "/home/u")])),
            expected
        );
        let empty = [("XDG_CONFIG_HOME", ""), ("HOME", "/home/u")];
        assert_eq!(path_from_env(false, lookup(&empty)), expected);
        assert_eq!(path_from_env(false, lookup(&[])), None);
    }

    #[test]
    fn windows_path_uses_appdata_only() {
        let vars = [("APPDATA", "/appdata"), ("XDG_CONFIG_HOME", "/cfg")];
        assert_eq!(
            path_from_env(true, lookup(&vars)),
            Some(PathBuf::from("/appdata/nomouse/config.toml"))
        );
        assert_eq!(path_from_env(true, lookup(&[("HOME", "/home/u")])), None);
    }
}
