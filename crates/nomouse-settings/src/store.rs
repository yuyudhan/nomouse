//! The window's one handle on `config.toml`: a thin wrapper over the shared settings model
//! ([`nomouse_core::settings::Settings`]), which validates, writes only the keys a change
//! touches (keeping the user's comments), saves atomically and notices edits on disk.
//!
//! Nothing else in this crate reads or writes the file.

use std::path::{Path, PathBuf};

use nomouse_core::Config;
use nomouse_core::settings::{Change, Settings, SettingsError, Value, write_atomically};

/// Why a change was not saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The change would make the configuration invalid (two modes on one hotkey, a bad
    /// alphabet): shown next to the control, nothing written.
    Rejected(String),
    /// The file could not be changed (not TOML, not writable).
    Failed(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}

impl From<SettingsError> for StoreError {
    fn from(error: SettingsError) -> Self {
        match error {
            SettingsError::Rejected(source) => Self::Rejected(source.to_string()),
            other => Self::Failed(other.to_string()),
        }
    }
}

/// What a file that the window creates says: every key is optional.
const NEW_FILE: &str = "# nomouse settings. Every key is optional; the settings window writes the ones you\n\
                        # change. Reference: https://github.com/yuyudhan/nomouse/blob/nightly/docs/configuration.md\n";

/// The settings file.
#[derive(Debug)]
pub struct Store {
    settings: Settings,
}

impl Store {
    /// Reads `path`: a missing file means every default; an invalid one leaves the defaults
    /// in effect with [`Store::problem`] saying why.
    ///
    /// # Errors
    /// When the file exists but cannot be read.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        Ok(Self {
            settings: Settings::open(path).map_err(|error| error.to_string())?,
        })
    }

    /// The file.
    pub fn path(&self) -> &Path {
        self.settings.path()
    }

    /// The configuration in effect.
    pub fn config(&self) -> &Config {
        self.settings.config()
    }

    /// Why the file on disk is not in effect, while it is invalid.
    pub fn problem(&self) -> Option<&str> {
        self.settings.problem()
    }

    /// The value of `key` (`section.key`); [`Value::Unset`] for an unknown key.
    pub fn value(&self, key: &str) -> Value {
        self.settings.value(key).unwrap_or(Value::Unset)
    }

    /// Sets `key` and saves at once; returns whether the file changed.
    ///
    /// # Errors
    /// [`StoreError::Rejected`] when the change is invalid, else [`StoreError::Failed`].
    pub fn set(&mut self, key: &str, value: Value) -> Result<bool, StoreError> {
        Ok(self.settings.set(key, value)?)
    }

    /// Removes every setting from the file (comments stay), so the defaults apply.
    ///
    /// # Errors
    /// When the file is not TOML or cannot be written.
    pub fn reset(&mut self) -> Result<bool, String> {
        self.settings.reset().map_err(|error| error.to_string())
    }

    /// Looks at the file at most once a second; call it from a timer.
    pub fn watch(&mut self) -> Change {
        self.settings.watch()
    }

    /// Creates the file (a comment only, so every default applies) when it does not exist,
    /// so an editor can open it.
    ///
    /// # Errors
    /// When the directory or the file cannot be created.
    pub fn ensure_file(&mut self) -> Result<(), String> {
        let path = self.path().to_owned();
        if path.exists() {
            return Ok(());
        }
        let io = |error: std::io::Error| format!("cannot create {}: {error}", path.display());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        write_atomically(&path, NEW_FILE).map_err(io)?;
        self.settings.refresh();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomouse_core::config::Theme;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nomouse-settings-store-{name}-{}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn set_saves_keeps_comments_and_reads_back() {
        let dir = temp_dir("round-trip");
        let path = dir.join("config.toml");
        std::fs::write(&path, "# mine\n[scroll]\nstep = 4 # fast\n").unwrap();
        let mut store = Store::open(&path).unwrap();
        assert_eq!(store.config().scroll.step, 4);
        assert_eq!(
            store.set("overlay.theme", Value::Text("dark".to_owned())),
            Ok(true)
        );
        assert_eq!(store.config().overlay.theme, Theme::Dark);
        assert_eq!(store.value("overlay.theme"), Value::Text("dark".to_owned()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("# mine") && text.contains("step = 4 # fast"),
            "{text}"
        );
        assert!(text.contains("theme = \"dark\""), "{text}");
        assert_eq!(Store::open(&path).unwrap().config(), store.config());
        assert_eq!(store.value("overlay.border"), Value::Unset);
        assert_eq!(store.value("no.such"), Value::Unset);
        assert_eq!(store.reset(), Ok(true));
        assert_eq!(store.config(), &Config::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn invalid_changes_are_rejected_and_not_written() {
        let dir = temp_dir("invalid");
        let path = dir.join("config.toml");
        let mut store = Store::open(&path).unwrap();
        let clash = store.set("hotkeys.continuous", Value::Text("ctrl+space".to_owned()));
        assert!(
            matches!(&clash, Err(StoreError::Rejected(message)) if message.contains("hotkeys.hints")),
            "{clash:?}"
        );
        assert!(matches!(
            store.set("hints.alphabet", Value::Text("x".to_owned())),
            Err(StoreError::Rejected(_))
        ));
        assert!(matches!(
            store.set("scroll.step", Value::Bool(true)),
            Err(StoreError::Failed(_))
        ));
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ensure_file_creates_a_file_of_defaults_and_watch_sees_edits() {
        let dir = temp_dir("ensure");
        let path = dir.join("nomouse").join("config.toml");
        let mut store = Store::open(&path).unwrap();
        store.ensure_file().unwrap();
        assert!(path.exists());
        assert_eq!(Store::open(&path).unwrap().config(), &Config::default());
        std::fs::write(&path, "[scroll]\nstep = 9\n").unwrap();
        assert!(matches!(store.watch(), Change::Reloaded));
        assert_eq!(store.config().scroll.step, 9);
        std::fs::write(&path, "[scroll]\nstep = 0\n").unwrap();
        std::thread::sleep(nomouse_core::settings::WATCH_INTERVAL);
        assert!(matches!(store.watch(), Change::Invalid(_)));
        assert!(
            store
                .problem()
                .is_some_and(|problem| problem.contains("scroll.step"))
        );
        assert_eq!(store.config().scroll.step, 9);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
