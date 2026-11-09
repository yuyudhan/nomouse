//! The settings model shared by the daemon and the settings windows: `config.toml` loaded,
//! validated, edited, saved and watched.
//!
//! - Saves are atomic (a temporary file in the same directory, then a rename) and go through
//!   the user's own file with `toml_edit`: only the keys a change touches are rewritten, a key
//!   set back to its default is removed, and comments, blank lines, key order and keys this
//!   version does not know stay as they were.
//! - [`Settings::watch`] (at most once per [`WATCH_INTERVAL`]) and [`Settings::refresh`] notice
//!   edits made by anything else, a text editor or another window, by the file's content. An
//!   invalid edit is reported and the last good configuration stays in effect.
//! - [`fields`] describes every key for a window: section, title, help and kind of control.

mod document;
pub mod fields;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use toml_edit::DocumentMut;

use crate::config::{self, Config, ConfigError};
pub use fields::{Choice, FIELDS, Field, Kind, Section, field_for};

/// How often [`Settings::watch`] looks at the file.
pub const WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// Why settings could not be read, changed or saved.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// Neither `XDG_CONFIG_HOME`, `HOME` nor (on Windows) `APPDATA` is set.
    #[error("no config path: set HOME (or APPDATA on Windows)")]
    NoPath,
    /// The file exists but could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
    /// The file could not be written.
    #[error("cannot write {path}: {source}")]
    Write {
        /// The file.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
    /// The file is not TOML at all, so a change cannot be written into it without losing it.
    #[error("{path} is not valid TOML, so it was not changed: {source}")]
    Syntax {
        /// The file.
        path: PathBuf,
        /// Where the TOML breaks.
        source: toml_edit::TomlError,
    },
    /// The file is TOML but not a valid configuration.
    #[error("{path}: {source}")]
    Invalid {
        /// The file.
        path: PathBuf,
        /// The first problem.
        source: ConfigError,
    },
    /// A change would make the configuration invalid; nothing was written.
    #[error("not saved: {0}")]
    Rejected(#[source] ConfigError),
    /// No setting has this `section.key`.
    #[error("no setting is called {0}")]
    UnknownKey(String),
    /// The value has the wrong kind for the setting.
    #[error("{key} takes {expected}")]
    WrongKind {
        /// The setting.
        key: &'static str,
        /// What it takes.
        expected: &'static str,
    },
}

/// What [`Settings::refresh`], [`Settings::reload`] or [`Settings::watch`] found.
#[derive(Debug)]
pub enum Change {
    /// The configuration in effect is the same as before (the file may have changed in ways
    /// that do not matter, such as comments).
    Unchanged,
    /// The file now holds a different, valid configuration, now in effect.
    Reloaded,
    /// The file changed and is invalid (or unreadable); the last good configuration stays.
    Invalid(SettingsError),
}

/// A setting's value, as a window reads and writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Text, a choice's value, a hotkey (`ctrl+space`) or a colour (`#rrggbb`).
    Text(String),
    /// A number.
    Number(u32),
    /// A toggle.
    Bool(bool),
    /// A list of app ids.
    List(Vec<String>),
    /// An optional colour that is not set: the theme's applies.
    Unset,
}

/// `config.toml` and the configuration in effect.
#[derive(Debug)]
pub struct Settings {
    path: PathBuf,
    config: Config,
    /// The file's text as last read or written; `None` while there is no file.
    seen: Option<String>,
    /// Why the file on disk is not in effect, while it is not.
    problem: Option<String>,
    /// When [`Settings::watch`] last looked.
    checked: Option<Instant>,
}

impl Settings {
    /// Reads the file at `path`. A missing file means every default; an invalid one leaves
    /// the defaults in effect with [`Settings::problem`] saying why.
    ///
    /// # Errors
    /// [`SettingsError::Read`] when the file exists but cannot be read.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, SettingsError> {
        let mut settings = Self {
            path: path.into(),
            config: Config::default(),
            seen: None,
            problem: None,
            checked: None,
        };
        settings.seen = settings.read()?;
        // An invalid file stays recorded in `problem`; the defaults are in effect meanwhile.
        settings.evaluate();
        Ok(settings)
    }

    /// [`Settings::open`] on the conventional path ([`config::default_path`]).
    ///
    /// # Errors
    /// [`SettingsError::NoPath`] when the environment names no path, else as
    /// [`Settings::open`].
    pub fn open_default() -> Result<Self, SettingsError> {
        Self::open(config::default_path().ok_or(SettingsError::NoPath)?)
    }

    /// The file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The configuration in effect: the file's, or the last good one while the file is
    /// invalid, or the defaults.
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Why the file on disk is not in effect, while it is invalid.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Re-reads the file when its content changed since it was last read or written.
    pub fn refresh(&mut self) -> Change {
        match self.read() {
            Ok(text) if text == self.seen => Change::Unchanged,
            Ok(text) => {
                self.seen = text;
                self.evaluate()
            }
            Err(error) => {
                self.problem = Some(error.to_string());
                Change::Invalid(error)
            }
        }
    }

    /// Re-reads the file even when its content looks unchanged (`nomouse reload`), so an
    /// invalid file is reported again.
    pub fn reload(&mut self) -> Change {
        match self.read() {
            Ok(text) => {
                self.seen = text;
                self.evaluate()
            }
            Err(error) => {
                self.problem = Some(error.to_string());
                Change::Invalid(error)
            }
        }
    }

    /// [`Settings::refresh`] at most once per [`WATCH_INTERVAL`]; [`Change::Unchanged`] in
    /// between. Call it from an event loop.
    pub fn watch(&mut self) -> Change {
        if self
            .checked
            .is_some_and(|checked| checked.elapsed() < WATCH_INTERVAL)
        {
            return Change::Unchanged;
        }
        self.checked = Some(Instant::now());
        self.refresh()
    }

    /// Changes the configuration and saves it at once. Edits made to the file meanwhile are
    /// read first, so they are kept. Returns whether the file changed.
    ///
    /// # Errors
    /// [`SettingsError::Rejected`] when the changed configuration is invalid (nothing is
    /// written), [`SettingsError::Syntax`] when the file is not TOML, and
    /// [`SettingsError::Write`].
    pub fn update(&mut self, change: impl FnOnce(&mut Config)) -> Result<bool, SettingsError> {
        self.refresh();
        let mut after = self.config.clone();
        change(&mut after);
        after.validate().map_err(SettingsError::Rejected)?;
        self.commit(after)
    }

    /// Replaces the whole configuration (as [`Settings::update`]).
    ///
    /// # Errors
    /// As [`Settings::update`].
    pub fn replace(&mut self, config: Config) -> Result<bool, SettingsError> {
        self.update(|current| *current = config)
    }

    /// The value of `key` (`section.key`, see [`FIELDS`]) in effect.
    ///
    /// # Errors
    /// [`SettingsError::UnknownKey`].
    pub fn value(&self, key: &str) -> Result<Value, SettingsError> {
        let field = field_for(key).ok_or_else(|| SettingsError::UnknownKey(key.to_owned()))?;
        Ok(match document::flatten(&self.config).remove(field.key) {
            None => Value::Unset,
            Some(toml::Value::Boolean(flag)) => Value::Bool(flag),
            Some(toml::Value::Integer(number)) => {
                Value::Number(u32::try_from(number).unwrap_or_default())
            }
            Some(toml::Value::Array(items)) => Value::List(
                items
                    .into_iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect(),
            ),
            Some(toml::Value::String(text)) => Value::Text(text),
            Some(other) => Value::Text(other.to_string()),
        })
    }

    /// Sets `key` (`section.key`, see [`FIELDS`]) and saves at once, as [`Settings::update`].
    /// [`Value::Unset`] clears an optional colour.
    ///
    /// # Errors
    /// [`SettingsError::UnknownKey`], [`SettingsError::WrongKind`] when the value does not
    /// fit the field's kind, otherwise as [`Settings::update`].
    pub fn set(&mut self, key: &str, value: Value) -> Result<bool, SettingsError> {
        let field = field_for(key).ok_or_else(|| SettingsError::UnknownKey(key.to_owned()))?;
        let value = toml_value(field, value)?;
        self.refresh();
        let mut keys = document::flatten(&self.config);
        match value {
            Some(value) => keys.insert(field.key.to_owned(), value),
            None => keys.remove(field.key),
        };
        let after = document::unflatten(&keys).map_err(SettingsError::Rejected)?;
        self.commit(after)
    }

    /// Removes every setting from the file, so the defaults apply; comments and keys this
    /// version does not know stay. Returns whether the file changed.
    ///
    /// # Errors
    /// [`SettingsError::Syntax`] when the file is not TOML, and [`SettingsError::Write`].
    pub fn reset(&mut self) -> Result<bool, SettingsError> {
        self.refresh();
        let mut document = self.document()?;
        if !document::clear(&mut document) {
            return Ok(false);
        }
        self.write(&document.to_string())?;
        self.evaluate();
        Ok(true)
    }

    /// Writes the keys that differ between the configuration in effect and `after` into the
    /// file, then puts `after` (or what the file now says) in effect.
    fn commit(&mut self, after: Config) -> Result<bool, SettingsError> {
        let mut document = self.document()?;
        let changed = document::patch(&mut document, &self.config, &after);
        if changed {
            self.write(&document.to_string())?;
        }
        if let Change::Invalid(_) = self.evaluate() {
            // The rest of the file is still invalid (a mistake made elsewhere); what was asked
            // for is written and stays what a window shows.
            self.config = after;
        }
        Ok(changed)
    }

    /// The file as an editable document (empty when there is no file).
    fn document(&self) -> Result<DocumentMut, SettingsError> {
        self.seen
            .as_deref()
            .unwrap_or_default()
            .parse()
            .map_err(|source| SettingsError::Syntax {
                path: self.path.clone(),
                source,
            })
    }

    /// The file's text, `None` when it does not exist.
    fn read(&self) -> Result<Option<String>, SettingsError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(SettingsError::Read {
                path: self.path.clone(),
                source,
            }),
        }
    }

    /// Saves `text` atomically and remembers it as the file's content.
    fn write(&mut self, text: &str) -> Result<(), SettingsError> {
        write_atomically(&self.path, text).map_err(|source| SettingsError::Write {
            path: self.path.clone(),
            source,
        })?;
        self.seen = Some(text.to_owned());
        Ok(())
    }

    /// Puts the last-read text in effect when it is valid; records the problem otherwise.
    fn evaluate(&mut self) -> Change {
        let parsed = self
            .seen
            .as_deref()
            .map_or_else(|| Ok(Config::default()), Config::from_toml);
        match parsed {
            Ok(config) => {
                self.problem = None;
                if config == self.config {
                    Change::Unchanged
                } else {
                    self.config = config;
                    Change::Reloaded
                }
            }
            Err(source) => {
                let error = SettingsError::Invalid {
                    path: self.path.clone(),
                    source,
                };
                self.problem = Some(error.to_string());
                Change::Invalid(error)
            }
        }
    }
}

/// The TOML value for `value` in `field`, or `None` to remove an optional key.
fn toml_value(field: &'static Field, value: Value) -> Result<Option<toml::Value>, SettingsError> {
    let wrong = |expected| SettingsError::WrongKind {
        key: field.key,
        expected,
    };
    Ok(Some(match (field.kind, value) {
        (Kind::Text | Kind::Choice(_) | Kind::Hotkey | Kind::Colour, Value::Text(text)) => {
            toml::Value::String(text)
        }
        (Kind::Colour, Value::Unset) => return Ok(None),
        (Kind::Number { .. }, Value::Number(number)) => toml::Value::Integer(i64::from(number)),
        (Kind::Toggle, Value::Bool(flag)) => toml::Value::Boolean(flag),
        (Kind::Apps, Value::List(items)) => {
            toml::Value::Array(items.into_iter().map(toml::Value::String).collect())
        }
        (Kind::Number { .. }, _) => return Err(wrong("a number")),
        (Kind::Toggle, _) => return Err(wrong("true or false")),
        (Kind::Apps, _) => return Err(wrong("a list of app ids")),
        (Kind::Colour, _) => return Err(wrong("a #rrggbb colour or nothing")),
        (Kind::Text | Kind::Choice(_) | Kind::Hotkey, _) => return Err(wrong("text")),
    }))
}

/// Writes `text` to `path` so that readers see the old file or the new one, never a part:
/// a temporary file next to it (in the same directory, so the rename cannot cross file
/// systems), flushed to disk, then renamed over it. A symbolic link (a dotfiles checkout) is
/// followed, so the link stays and its target changes. The file keeps its permissions.
///
/// # Errors
/// The OS error of creating, writing or renaming.
pub fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let target = match std::fs::read_link(path) {
        Ok(link) if link.is_absolute() => link,
        Ok(link) => path.parent().unwrap_or_else(|| Path::new("")).join(link),
        Err(_) => path.to_owned(),
    };
    let dir = target
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = target
        .file_name()
        .map_or_else(|| "config".into(), |name| name.to_string_lossy());
    let temporary = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if let Ok(meta) = std::fs::metadata(&target) {
            std::fs::set_permissions(&temporary, meta.permissions())?;
        }
        std::fs::rename(&temporary, &target)
    })();
    if written.is_err() {
        // The temporary file is ours; a failed save must not leave it behind.
        std::fs::remove_file(&temporary).ok();
    }
    written
}

#[cfg(test)]
mod tests;
