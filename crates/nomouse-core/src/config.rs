//! User configuration: TOML with defaults for every field, validated after parsing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::action::ActionKind;
use crate::flavour::Flavour;
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
    /// Apps where the hotkeys show no labels.
    pub ignore: IgnoreConfig,
}

/// `[ignore]`
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IgnoreConfig {
    /// Apps where activation shows no labels, each named by its macOS bundle id, Wayland app
    /// id, X11 `WM_CLASS` (instance or class) or process name, matched ignoring case.
    pub apps: Vec<String>,
}

/// Bundle ids of the common Chromium browsers: the default `macos.enhanced_ui_apps`. Their web
/// content has no accessibility tree until `AXEnhancedUserInterface` is set.
pub const CHROMIUM_BROWSERS: &[&str] = &[
    "com.google.Chrome",
    "com.google.Chrome.beta",
    "com.google.Chrome.dev",
    "com.google.Chrome.canary",
    "org.chromium.Chromium",
    "company.thebrowser.Browser",
    "com.brave.Browser",
    "com.microsoft.edgemac",
    "com.vivaldi.Vivaldi",
    "com.operasoftware.Opera",
];

/// `[macos]`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MacosConfig {
    /// Bundle ids (such as `com.google.Chrome`) whose accessibility tree nomouse switches on
    /// with `AXEnhancedUserInterface`. Chromium and Electron apps build their tree only when
    /// asked; the attribute can slow window animations, so only listed apps get it. Defaults
    /// to [`CHROMIUM_BROWSERS`]; a list in the file replaces it.
    pub enhanced_ui_apps: Vec<String>,
}

impl Default for MacosConfig {
    fn default() -> Self {
        Self {
            enhanced_ui_apps: CHROMIUM_BROWSERS.iter().map(|&id| id.to_owned()).collect(),
        }
    }
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
    /// How much of a web page is collected: `fast` (what is in the viewport) or `exhaustive`
    /// (the whole document, so search finds links scrolled out of view).
    pub browser: BrowserMode,
    /// Milliseconds after the hotkey when labels are added for targets that appeared since
    /// the first scan (a page still loading); 0 turns this stage off.
    pub early_rescan_ms: u32,
    /// Milliseconds after the hotkey of the last stage, for slow targets (a browser that is
    /// building its page's accessibility tree); later than `early_rescan_ms`, 0 turns it off.
    pub late_rescan_ms: u32,
}

/// The largest `hints.early_rescan_ms` and `hints.late_rescan_ms`, in milliseconds.
pub const MAX_RESCAN_MS: u32 = 10_000;

/// How much of a web document (a browser page, an Electron app) a scan collects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserMode {
    /// Only the part of the page inside the viewport: the quickest scan.
    #[default]
    Fast,
    /// The whole document within the walk's limits: elements scrolled out of view are
    /// collected too ([`crate::Element::offscreen`]); search finds them and picking one scrolls
    /// it into view. Slower on long pages.
    Exhaustive,
}

impl Default for HintsConfig {
    fn default() -> Self {
        Self {
            alphabet: DEFAULT_ALPHABET.to_owned(),
            scope: Scope::FocusedWindow,
            action: ActionKind::Click,
            semantic_click: false,
            keys: KeyBindings::Nomouse,
            browser: BrowserMode::Fast,
            early_rescan_ms: 1000,
            late_rescan_ms: 3000,
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

impl HotkeysConfig {
    /// The default chords of `flavour`. A release build binds `ctrl+space` (hints),
    /// `ctrl+shift+space` (continuous), `ctrl+alt+space` (scroll) and `ctrl+alt+/` (search). A
    /// development build adds `alt+shift` to `ctrl` and keeps the four apart by their key
    /// (`space`, `.`, `,`, `/`): a chord can be grabbed by only one program (X11 refuses the
    /// second grab, and on macOS the first event tap swallows the press), so a development
    /// build with the release defaults would lose its hotkeys, or take them from the
    /// installed nomouse.
    #[must_use]
    pub const fn defaults(flavour: Flavour) -> Self {
        match flavour {
            Flavour::Release => Self {
                hints: chord(true, false, false, Key::Space),
                continuous: chord(true, false, true, Key::Space),
                scroll: chord(true, true, false, Key::Space),
                search: chord(true, true, false, Key::Slash),
            },
            Flavour::Dev => Self {
                hints: chord(true, true, true, Key::Space),
                continuous: chord(true, true, true, Key::Period),
                scroll: chord(true, true, true, Key::Comma),
                search: chord(true, true, true, Key::Slash),
            },
        }
    }
}

impl Default for HotkeysConfig {
    /// [`HotkeysConfig::defaults`] of this build's [`Flavour::CURRENT`].
    fn default() -> Self {
        Self::defaults(Flavour::CURRENT)
    }
}

/// `[scroll]`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScrollConfig {
    /// Wheel steps per key press, 1 to 50.
    pub step: u32,
    /// Wheel steps per Shift-scroll key press (`H` `J` `K` `L`), 1 to 100.
    pub dash_step: u32,
}

impl Default for ScrollConfig {
    fn default() -> Self {
        Self {
            step: 3,
            dash_step: 15,
        }
    }
}

/// `overlay.theme`: the label colours before any explicit colour overrides them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    /// Yellow labels with dark text (nomouse's look before themes).
    #[default]
    Original,
    /// White labels with dark text.
    Light,
    /// Dark labels with light text.
    Dark,
    /// `light` or `dark`, following the system appearance.
    Auto,
}

/// The system's appearance, which `overlay.theme = "auto"` follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Appearance {
    /// Light (also when the system states no preference).
    #[default]
    Light,
    /// Dark.
    Dark,
}

impl Theme {
    /// The theme's colours under `appearance` (which only `Auto` looks at): background,
    /// foreground, typed foreground, border.
    pub const fn palette(self, appearance: Appearance) -> [[u8; 3]; 4] {
        match self {
            Self::Original => [
                [0xff, 0xd7, 0x5f],
                [0x1c, 0x1c, 0x1c],
                [0x8a, 0x8a, 0x8a],
                [0x1c, 0x1c, 0x1c],
            ],
            Self::Light => [
                [0xff, 0xff, 0xff],
                [0x1c, 0x1c, 0x1c],
                [0x9e, 0x9e, 0x9e],
                [0x8a, 0x8a, 0x8a],
            ],
            Self::Dark => [
                [0x2b, 0x2b, 0x2b],
                [0xf2, 0xf2, 0xf2],
                [0x80, 0x80, 0x80],
                [0x0a, 0x0a, 0x0a],
            ],
            Self::Auto => match appearance {
                Appearance::Light => Self::Light.palette(appearance),
                Appearance::Dark => Self::Dark.palette(appearance),
            },
        }
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
    /// Label colours: `original`, `light`, `dark` or `auto` (the system appearance).
    pub theme: Theme,
    /// Label background, `#rrggbb`; overrides the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Label text, `#rrggbb`; overrides the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    /// Already-typed label characters, `#rrggbb`; overrides the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typed_foreground: Option<String>,
    /// Label border, `#rrggbb`; overrides the theme's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            font_size: 13,
            opacity: 95,
            theme: Theme::Original,
            background: None,
            foreground: None,
            typed_foreground: None,
            border: None,
        }
    }
}

impl OverlayConfig {
    /// The colours as RGB triples (background, foreground, typed foreground, border): the
    /// theme's under `appearance`, each replaced by its explicit colour when one is set.
    ///
    /// # Errors
    /// [`ConfigError::Colour`] for the first explicit colour that is not `#rrggbb`.
    pub fn colours(&self, appearance: Appearance) -> Result<[[u8; 3]; 4], ConfigError> {
        let theme = self.theme.palette(appearance);
        let pick = |field: &'static str, value: Option<&String>, fallback: [u8; 3]| {
            value.map_or(Ok(fallback), |value| {
                parse_colour(value).ok_or_else(|| ConfigError::Colour {
                    field,
                    value: value.clone(),
                })
            })
        };
        Ok([
            pick("overlay.background", self.background.as_ref(), theme[0])?,
            pick("overlay.foreground", self.foreground.as_ref(), theme[1])?,
            pick(
                "overlay.typed_foreground",
                self.typed_foreground.as_ref(),
                theme[2],
            )?,
            pick("overlay.border", self.border.as_ref(), theme[3])?,
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
    /// A list holds an empty (or blank) entry.
    #[error("{field} has an empty entry")]
    EmptyEntry {
        /// The field.
        field: &'static str,
    },
    /// `hints.late_rescan_ms` is not later than `hints.early_rescan_ms` (both non-zero).
    #[error(
        "hints.late_rescan_ms = {late} must be later than hints.early_rescan_ms = {early} \
         (or 0 to turn that stage off)"
    )]
    RescanOrder {
        /// `hints.early_rescan_ms`.
        early: u32,
        /// `hints.late_rescan_ms`.
        late: u32,
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
        check_range("scroll.dash_step", self.scroll.dash_step, 1, 100)?;
        check_range("overlay.font_size", self.overlay.font_size, 6, 72)?;
        check_range("overlay.opacity", self.overlay.opacity, 10, 100)?;
        self.overlay.colours(Appearance::Light).map(|_| ())?;
        check_range(
            "hints.early_rescan_ms",
            self.hints.early_rescan_ms,
            0,
            MAX_RESCAN_MS,
        )?;
        check_range(
            "hints.late_rescan_ms",
            self.hints.late_rescan_ms,
            0,
            MAX_RESCAN_MS,
        )?;
        let (early, late) = (self.hints.early_rescan_ms, self.hints.late_rescan_ms);
        if early > 0 && late > 0 && late <= early {
            return Err(ConfigError::RescanOrder { early, late });
        }
        if self.ignore.apps.iter().any(|app| app.trim().is_empty()) {
            return Err(ConfigError::EmptyEntry {
                field: "ignore.apps",
            });
        }
        if self
            .macos
            .enhanced_ui_apps
            .iter()
            .any(|app| app.trim().is_empty())
        {
            return Err(ConfigError::EmptyEntry {
                field: "macos.enhanced_ui_apps",
            });
        }
        Ok(())
    }

    /// The validated label alphabet.
    ///
    /// # Errors
    /// [`ConfigError::Alphabet`] when `hints.alphabet` is invalid.
    pub fn alphabet(&self) -> Result<Alphabet, ConfigError> {
        Ok(Alphabet::new(&self.hints.alphabet)?)
    }
}

/// The conventional config file path, from the environment, for this build's
/// [`Flavour::CURRENT`]: on Linux and macOS `$XDG_CONFIG_HOME/<stem>/config.toml`, else
/// `~/.config/<stem>/config.toml`; on Windows `%APPDATA%\<stem>\config.toml`, where the stem is
/// `nomouse` for a release build and `nomouse-dev` for a development one. `None` when the
/// variables are unset.
pub fn default_path() -> Option<PathBuf> {
    path_from_env(Flavour::CURRENT, cfg!(target_os = "windows"), |name| {
        std::env::var_os(name)
    })
}

/// [`default_path`] for `flavour` over an explicit environment lookup, so every OS's rule and
/// both flavours are testable on any host. Empty values count as unset, as the XDG Base
/// Directory specification requires.
fn path_from_env(
    flavour: Flavour,
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
    Some(base.join(flavour.stem()).join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rescan_stages_default_to_one_and_three_seconds_and_are_validated() {
        let hints = Config::default().hints;
        assert_eq!((hints.early_rescan_ms, hints.late_rescan_ms), (1000, 3000));
        let config =
            Config::from_toml("[hints]\nearly_rescan_ms = 500\nlate_rescan_ms = 10000\n").unwrap();
        assert_eq!(
            (config.hints.early_rescan_ms, config.hints.late_rescan_ms),
            (500, 10_000)
        );
        // 0 turns a stage off, in any combination.
        for text in [
            "early_rescan_ms = 0",
            "late_rescan_ms = 0",
            "early_rescan_ms = 0\nlate_rescan_ms = 0",
            "early_rescan_ms = 5000\nlate_rescan_ms = 0",
        ] {
            assert!(
                Config::from_toml(&format!("[hints]\n{text}\n")).is_ok(),
                "{text}"
            );
        }
        for (field, text) in [
            ("hints.early_rescan_ms", "early_rescan_ms = 10001"),
            ("hints.late_rescan_ms", "late_rescan_ms = 60000"),
        ] {
            assert!(
                matches!(
                    Config::from_toml(&format!("[hints]\n{text}\n")),
                    Err(ConfigError::OutOfRange { field: f, max: MAX_RESCAN_MS, .. }) if f == field
                ),
                "{text}"
            );
        }
        for (early, late) in [(3000, 1000), (2000, 2000), (1000, 4000)] {
            let result = Config::from_toml(&format!(
                "[hints]\nearly_rescan_ms = {early}\nlate_rescan_ms = {late}\n"
            ));
            if early < late {
                assert!(result.is_ok());
            } else {
                assert!(matches!(result, Err(ConfigError::RescanOrder { .. })));
            }
        }
    }

    #[test]
    fn empty_file_is_defaults() {
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
    }

    #[test]
    fn defaults_round_trip() {
        let text = Config::default().to_toml();
        let hotkeys = HotkeysConfig::default();
        assert!(text.contains(&format!("hints = \"{}\"", hotkeys.hints)));
        assert!(text.contains(&format!("continuous = \"{}\"", hotkeys.continuous)));
        assert!(text.contains("semantic_click = false"));
        assert!(text.contains("keys = \"nomouse\""));
        assert_eq!(Config::from_toml(&text).unwrap(), Config::default());
    }

    #[test]
    fn each_flavour_has_its_own_valid_default_hotkeys() {
        let release = HotkeysConfig::defaults(Flavour::Release);
        let dev = HotkeysConfig::defaults(Flavour::Dev);
        assert_eq!(
            HotkeysConfig::default(),
            HotkeysConfig::defaults(Flavour::CURRENT)
        );
        let names = |keys: &HotkeysConfig| {
            [keys.hints, keys.continuous, keys.scroll, keys.search].map(|key| key.to_string())
        };
        assert_eq!(
            names(&release),
            [
                "ctrl+space",
                "ctrl+shift+space",
                "ctrl+alt+space",
                "ctrl+alt+slash"
            ]
        );
        assert_eq!(
            names(&dev),
            [
                "ctrl+alt+shift+space",
                "ctrl+alt+shift+period",
                "ctrl+alt+shift+comma",
                "ctrl+alt+shift+slash"
            ]
        );
        // No development chord is a release chord, so both builds can run at once.
        for chord in names(&dev) {
            assert!(!names(&release).contains(&chord), "{chord}");
        }
        for keys in [release, dev] {
            let config = Config {
                hotkeys: keys,
                ..Config::default()
            };
            assert!(config.validate().is_ok(), "{:?}", config.hotkeys);
            assert_eq!(Config::from_toml(&config.to_toml()).unwrap(), config);
        }
    }

    #[test]
    fn partial_override() {
        let config = Config::from_toml(
            "[hints]\nalphabet = \"jkfd\"\naction = \"right-click\"\nscope = \"screen\"\n[scroll]\nstep = 5\ndash_step = 40\n",
        )
        .unwrap();
        assert_eq!(config.hints.action, ActionKind::RightClick);
        assert_eq!(config.hints.scope, Scope::Screen);
        assert_eq!(config.scroll.step, 5);
        assert_eq!(config.scroll.dash_step, 40);
        assert_eq!(Config::default().scroll.dash_step, 15);
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
    fn ignore_apps_parse_and_reject_blank_entries() {
        assert_eq!(Config::default().ignore.apps, Vec::<String>::new());
        let config =
            Config::from_toml("[ignore]\napps = [\"org.gnome.Terminal\", \"kitty\"]\n").unwrap();
        assert_eq!(config.ignore.apps, ["org.gnome.Terminal", "kitty"]);
        assert!(matches!(
            Config::from_toml("[ignore]\napps = [\"kitty\", \" \"]\n"),
            Err(ConfigError::EmptyEntry {
                field: "ignore.apps"
            })
        ));
        assert!(matches!(
            Config::from_toml("[ignore]\nprograms = []\n"),
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
            Config::from_toml(&format!(
                "[hotkeys]\nscroll = \"{}\"\n",
                HotkeysConfig::default().hints
            )),
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
        for dash_step in [0, 101] {
            assert!(matches!(
                Config::from_toml(&format!("[scroll]\ndash_step = {dash_step}\n")),
                Err(ConfigError::OutOfRange {
                    field: "scroll.dash_step",
                    ..
                })
            ));
        }
        assert!(Config::from_toml("[scroll]\ndash_step = 100\n").is_ok());
        assert!(Config::from_toml("[scroll]\ndash_step = 1\n").is_ok());
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
            OverlayConfig::default().colours(Appearance::Dark).unwrap()[0],
            [255, 215, 95]
        );
    }

    #[test]
    fn each_theme_parses_and_sets_its_colours() {
        let colours = |text: &str, appearance| {
            Config::from_toml(text)
                .unwrap()
                .overlay
                .colours(appearance)
                .unwrap()
        };
        let light = Appearance::Light;
        assert_eq!(Config::default().overlay.theme, Theme::Original);
        assert_eq!(colours("", light)[0], [0xff, 0xd7, 0x5f]);
        assert_eq!(
            colours("[overlay]\ntheme = \"light\"\n", Appearance::Dark),
            Theme::Light.palette(light)
        );
        let dark = colours("[overlay]\ntheme = \"dark\"\n", light);
        assert_eq!(dark, Theme::Dark.palette(light));
        // Dark labels: a dark background under light text.
        assert!(dark[0].iter().all(|c| *c < 0x40) && dark[1].iter().all(|c| *c > 0xc0));
        assert!(matches!(
            Config::from_toml("[overlay]\ntheme = \"solarized\"\n"),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn auto_follows_the_appearance() {
        let auto = Config::from_toml("[overlay]\ntheme = \"auto\"\n")
            .unwrap()
            .overlay;
        assert_eq!(
            auto.colours(Appearance::Light).unwrap(),
            Theme::Light.palette(Appearance::Light)
        );
        assert_eq!(
            auto.colours(Appearance::Dark).unwrap(),
            Theme::Dark.palette(Appearance::Dark)
        );
    }

    #[test]
    fn explicit_colours_override_the_theme() {
        let overlay = Config::from_toml(
            "[overlay]\ntheme = \"dark\"\nbackground = \"#102030\"\nborder = \"#ff0000\"\n",
        )
        .unwrap()
        .overlay;
        let dark = Theme::Dark.palette(Appearance::Light);
        assert_eq!(
            overlay.colours(Appearance::Light).unwrap(),
            [[0x10, 0x20, 0x30], dark[1], dark[2], [0xff, 0, 0]]
        );
        // An override stays whatever the appearance under `auto`.
        let auto = Config::from_toml("[overlay]\ntheme = \"auto\"\nforeground = \"#00ff00\"\n")
            .unwrap()
            .overlay;
        assert_eq!(auto.colours(Appearance::Dark).unwrap()[1], [0, 0xff, 0]);
        assert_eq!(auto.colours(Appearance::Light).unwrap()[1], [0, 0xff, 0]);
        // The defaults serialise without colours, so `config init` keeps following the theme.
        assert!(!Config::default().to_toml().contains("background"));
    }

    #[test]
    fn browser_mode_defaults_to_fast_and_parses_exhaustive() {
        assert_eq!(Config::default().hints.browser, BrowserMode::Fast);
        let config = Config::from_toml("[hints]\nbrowser = \"exhaustive\"\n").unwrap();
        assert_eq!(config.hints.browser, BrowserMode::Exhaustive);
        assert!(Config::from_toml("[hints]\nbrowser = \"thorough\"\n").is_err());
        assert!(Config::default().to_toml().contains("browser = \"fast\""));
    }

    #[test]
    fn macos_enhanced_ui_apps() {
        assert_eq!(Config::default().macos, MacosConfig::default());
        assert_eq!(
            Config::default().macos.enhanced_ui_apps,
            CHROMIUM_BROWSERS,
            "Chromium browsers get web labels without any configuration"
        );
        assert!(CHROMIUM_BROWSERS.contains(&"com.google.Chrome"));
        assert_eq!(
            Config::from_toml("[hints]\n")
                .unwrap()
                .macos
                .enhanced_ui_apps,
            CHROMIUM_BROWSERS,
            "a file without [macos] keeps the default"
        );
        assert!(
            Config::from_toml("[macos]\nenhanced_ui_apps = []\n")
                .unwrap()
                .macos
                .enhanced_ui_apps
                .is_empty(),
            "an empty list in the file turns it off everywhere"
        );
        let config =
            Config::from_toml("[macos]\nenhanced_ui_apps = [\"com.google.Chrome\"]\n").unwrap();
        assert_eq!(config.macos.enhanced_ui_apps, ["com.google.Chrome"]);
        assert!(Config::from_toml("[macos]\nunknown = 1\n").is_err());
        assert!(matches!(
            Config::from_toml("[macos]\nenhanced_ui_apps = [\"com.google.Chrome\", \" \"]\n"),
            Err(ConfigError::EmptyEntry {
                field: "macos.enhanced_ui_apps"
            })
        ));
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

    // macOS and Linux share one rule (`$XDG_CONFIG_HOME`, else `~/.config`: config lives under
    // `~/.config` on both), so each case below is the path on either system.
    #[test]
    fn unix_path_prefers_xdg_config_home() {
        let vars = [("XDG_CONFIG_HOME", "/cfg"), ("HOME", "/home/u")];
        assert_eq!(
            path_from_env(Flavour::Release, false, lookup(&vars)),
            Some(PathBuf::from("/cfg/nomouse/config.toml"))
        );
        assert_eq!(
            path_from_env(Flavour::Dev, false, lookup(&vars)),
            Some(PathBuf::from("/cfg/nomouse-dev/config.toml"))
        );
    }

    #[test]
    fn unix_path_falls_back_to_dot_config_when_xdg_unset_or_empty() {
        for (flavour, expected) in [
            (Flavour::Release, "/home/u/.config/nomouse/config.toml"),
            (Flavour::Dev, "/home/u/.config/nomouse-dev/config.toml"),
        ] {
            let expected = Some(PathBuf::from(expected));
            assert_eq!(
                path_from_env(flavour, false, lookup(&[("HOME", "/home/u")])),
                expected
            );
            let empty = [("XDG_CONFIG_HOME", ""), ("HOME", "/home/u")];
            assert_eq!(path_from_env(flavour, false, lookup(&empty)), expected);
            assert_eq!(path_from_env(flavour, false, lookup(&[])), None);
        }
    }

    #[test]
    fn macos_home_paths_stay_under_dot_config() {
        let home = [("HOME", "/Users/u")];
        assert_eq!(
            path_from_env(Flavour::Release, false, lookup(&home)),
            Some(PathBuf::from("/Users/u/.config/nomouse/config.toml"))
        );
        assert_eq!(
            path_from_env(Flavour::Dev, false, lookup(&home)),
            Some(PathBuf::from("/Users/u/.config/nomouse-dev/config.toml"))
        );
    }

    #[test]
    fn windows_path_uses_appdata_only() {
        let vars = [("APPDATA", "/appdata"), ("XDG_CONFIG_HOME", "/cfg")];
        assert_eq!(
            path_from_env(Flavour::Release, true, lookup(&vars)),
            Some(PathBuf::from("/appdata/nomouse/config.toml"))
        );
        assert_eq!(
            path_from_env(Flavour::Dev, true, lookup(&vars)),
            Some(PathBuf::from("/appdata/nomouse-dev/config.toml"))
        );
        assert_eq!(
            path_from_env(Flavour::Release, true, lookup(&[("HOME", "/home/u")])),
            None
        );
    }

    #[test]
    fn the_default_path_is_this_builds_flavour() {
        let windows = cfg!(target_os = "windows");
        let real = |name: &str| std::env::var_os(name);
        assert_eq!(
            default_path(),
            path_from_env(Flavour::CURRENT, windows, real)
        );
        if let Some(path) = default_path() {
            let dir = path.parent().and_then(Path::file_name);
            assert_eq!(dir, Some(std::ffi::OsStr::new(Flavour::CURRENT.stem())));
        }
    }
}
