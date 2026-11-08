//! User configuration: TOML with defaults for every field, validated after parsing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::action::ActionKind;
use crate::hints::{Alphabet, AlphabetError, DEFAULT_ALPHABET};
use crate::hotkey::{Hotkey, Key, Modifiers};
use crate::platform::Scope;

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
}

impl Default for HintsConfig {
    fn default() -> Self {
        Self {
            alphabet: DEFAULT_ALPHABET.to_owned(),
            scope: Scope::FocusedWindow,
            action: ActionKind::Click,
        }
    }
}

/// `[hotkeys]`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HotkeysConfig {
    /// Shows hints.
    pub hints: Hotkey,
    /// Shows hints for a scroll target.
    pub scroll: Hotkey,
    /// Search elements by text.
    pub search: Hotkey,
}

const fn ctrl_alt(key: Key) -> Hotkey {
    Hotkey::new(
        Modifiers {
            ctrl: true,
            alt: true,
            shift: false,
            meta: false,
        },
        key,
    )
}

impl Default for HotkeysConfig {
    fn default() -> Self {
        Self {
            hints: ctrl_alt(Key::Space),
            scroll: ctrl_alt(Key::Char('j')),
            search: ctrl_alt(Key::Slash),
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
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            font_size: 13,
            opacity: 95,
        }
    }
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
        check_range("overlay.opacity", self.overlay.opacity, 10, 100)
    }

    /// The validated label alphabet.
    ///
    /// # Errors
    /// [`ConfigError::Alphabet`] when `hints.alphabet` is invalid.
    pub fn alphabet(&self) -> Result<Alphabet, ConfigError> {
        Ok(Alphabet::new(&self.hints.alphabet)?)
    }
}

/// The conventional config file path for this OS, from the environment:
/// `$XDG_CONFIG_HOME/nomouse/config.toml` or `~/.config/...` on Linux,
/// `~/Library/Application Support/nomouse/config.toml` on macOS,
/// `%APPDATA%\nomouse\config.toml` on Windows. `None` when the variables are unset.
pub fn default_path() -> Option<PathBuf> {
    let env = |name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(target_os = "windows") {
        env("APPDATA")?
    } else if cfg!(target_os = "macos") {
        env("HOME")?.join("Library").join("Application Support")
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
        assert!(text.contains("hints = \"ctrl+alt+space\""));
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
            Config::from_toml("[hotkeys]\nscroll = \"ctrl+alt+space\"\n"),
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
    }

    #[test]
    fn load_reports_missing_file() {
        let missing = Path::new("/nonexistent/nomouse/config.toml");
        assert!(matches!(Config::load(missing), Err(ConfigError::Io { .. })));
    }
}
