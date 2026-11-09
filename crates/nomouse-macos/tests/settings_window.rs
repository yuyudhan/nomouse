//! The settings window over a scratch config file, worked as a user would: a shortcut recorded
//! with the keyboard and a theme picked from its pop-up button land in `config.toml` at once,
//! and the daemon's own settings model (a second reader of the same file, as the daemon is)
//! reloads each change.
//!
//! The window is real AppKit on the main thread (so the test has no harness), but it needs no
//! Accessibility grant, no unlocked screen and no focus: clicks and keys are sent to the
//! controls in this process. The test re-runs itself with `XDG_CONFIG_HOME` pointing at a
//! fresh directory, so the user's own `~/.config/nomouse` is never read or written.

use std::process::ExitCode;

use std::io::Write as _;

/// Reports progress on stderr (a harness-less test has no other channel).
fn note(message: &str) {
    // Nothing useful can be done when stderr is closed.
    writeln!(std::io::stderr(), "{message}").ok();
}

#[cfg(not(target_os = "macos"))]
fn main() -> ExitCode {
    note("SKIP: the settings window is macOS only");
    ExitCode::SUCCESS
}

#[cfg(target_os = "macos")]
fn main() -> ExitCode {
    macos::main()
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::{Path, PathBuf};
    use std::process::{Command, ExitCode};
    use std::time::{Duration, Instant};

    use nomouse_core::config::Theme;
    use nomouse_core::menu::MenuState;

    use super::note;
    use nomouse_core::settings::{Change, Section, Settings};
    use nomouse_macos::menubar::MenuBar;
    use nomouse_sys_macos::types::ModifierFlags;

    /// Set in the re-run that does the work.
    const CHILD: &str = "NOMOUSE_SETTINGS_WINDOW_TEST";
    /// How long one change may take to reach the file.
    const DEADLINE: Duration = Duration::from_secs(5);
    /// The ANSI `j` key.
    const KEY_J: u16 = 38;

    pub(super) fn main() -> ExitCode {
        if std::env::var_os(CHILD).is_some() {
            return run();
        }
        let dir =
            std::env::temp_dir().join(format!("nomouse-settings-window-{}", std::process::id()));
        let result = std::fs::create_dir_all(&dir)
            .map_err(|error| error.to_string())
            .and_then(|()| {
                let exe = std::env::current_exe().map_err(|error| error.to_string())?;
                Command::new(exe)
                    .env(CHILD, "1")
                    .env("XDG_CONFIG_HOME", &dir)
                    .status()
                    .map_err(|error| error.to_string())
            });
        let _ = std::fs::remove_dir_all(&dir);
        match result {
            Ok(status) if status.success() => ExitCode::SUCCESS,
            Ok(status) => {
                note(&format!(
                    "FAIL: the settings window test exited with {status}"
                ));
                ExitCode::FAILURE
            }
            Err(error) => {
                note(&format!(
                    "FAIL: cannot run the settings window test: {error}"
                ));
                ExitCode::FAILURE
            }
        }
    }

    fn config_path() -> PathBuf {
        nomouse_core::config::default_path().unwrap_or_default()
    }

    /// Services the window (and drains what the menu bar app asks of the daemon) until `done`
    /// holds or the deadline passes; whether it held.
    fn pump_until(bar: &mut MenuBar, mut done: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < DEADLINE {
            bar.pump(Duration::from_millis(20));
            while bar.next_command().is_some() {}
            if done() {
                return true;
            }
        }
        false
    }

    /// Services the window for a moment, so a click is carried out before the next step.
    fn settle(bar: &mut MenuBar) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(300) {
            bar.pump(Duration::from_millis(20));
            while bar.next_command().is_some() {}
        }
    }

    fn file_has(path: &Path, section: &str, line: &str) -> bool {
        let Ok(text) = std::fs::read_to_string(path) else {
            return false;
        };
        let mut in_section = false;
        for row in text.lines().map(str::trim) {
            if row.starts_with('[') {
                in_section = row == format!("[{section}]");
            } else if in_section && row == line {
                return true;
            }
        }
        false
    }

    struct Checks {
        failed: usize,
    }

    impl Checks {
        fn check(&mut self, ok: bool, what: &str) {
            if ok {
                note(&format!("PASS: {what}"));
            } else {
                note(&format!("FAIL: {what}"));
                self.failed += 1;
            }
        }
    }

    fn run() -> ExitCode {
        let path = config_path();
        let mut checks = Checks { failed: 0 };
        let mut bar = match MenuBar::new(MenuState {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            active: true,
            ..MenuState::default()
        }) {
            Ok(bar) => bar,
            Err(error) => {
                note(&format!("FAIL: no menu bar app: {error}"));
                return ExitCode::FAILURE;
            }
        };
        // The daemon reads the same file through its own model.
        let mut daemon = match Settings::open(&path) {
            Ok(daemon) => daemon,
            Err(error) => {
                note(&format!(
                    "FAIL: the daemon's model cannot open {}: {error}",
                    path.display()
                ));
                return ExitCode::FAILURE;
            }
        };

        // A shortcut, recorded: click the recorder, then press Control-Option-J.
        bar.open_section(Section::Hotkeys);
        checks.check(
            bar.settings_page().is_some(),
            "the settings window opens on Hotkeys",
        );
        let hints = MenuBar::settings_tag("hotkeys.hints");
        let clicked = match (bar.settings_form(), hints) {
            (Some(form), Some(tag)) => form.click(tag),
            _ => false,
        };
        checks.check(clicked, "the Hints shortcut recorder takes a click");
        settle(&mut bar);
        let chord = ModifierFlags {
            control: true,
            option: true,
            ..ModifierFlags::default()
        };
        let typed = bar
            .settings_form()
            .is_some_and(|form| form.press_key(KEY_J, "j", chord));
        checks.check(typed, "the recorder takes Control-Option-J");
        let saved = pump_until(&mut bar, || {
            file_has(&path, "hotkeys", r#"hints = "ctrl+alt+j""#)
        });
        checks.check(
            saved,
            "config.toml has hotkeys.hints = \"ctrl+alt+j\" at once",
        );
        let reloaded = matches!(daemon.refresh(), Change::Reloaded)
            && daemon.config().hotkeys.hints.to_string() == "ctrl+alt+j";
        checks.check(reloaded, "the daemon's model reloads the new hints hotkey");

        // A theme, picked from its pop-up button.
        bar.open_section(Section::Appearance);
        let theme = MenuBar::settings_tag("overlay.theme");
        let picked = match (bar.settings_form(), theme) {
            (Some(form), Some(tag)) => form.choose(tag, "Dark"),
            _ => false,
        };
        checks.check(picked, "the Theme pop-up button takes Dark");
        let saved = pump_until(&mut bar, || file_has(&path, "overlay", r#"theme = "dark""#));
        checks.check(saved, "config.toml has overlay.theme = \"dark\" at once");
        let reloaded = matches!(daemon.refresh(), Change::Reloaded)
            && daemon.config().overlay.theme == Theme::Dark;
        checks.check(reloaded, "the daemon's model reloads the dark theme");
        checks.check(
            file_has(&path, "hotkeys", r#"hints = "ctrl+alt+j""#),
            "the earlier hotkey stays in the file",
        );

        bar.close_settings();
        checks.check(bar.settings_page().is_none(), "the settings window closes");
        note(&format!("--- {} failed", checks.failed));
        if checks.failed == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }
}
