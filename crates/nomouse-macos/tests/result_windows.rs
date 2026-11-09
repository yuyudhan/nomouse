//! The result windows (the doctor's report, the update check's answer) must not block the
//! daemon: while one is open, a hotkey's request and a configuration reload are still handled.
//!
//! A modal alert runs its own event loop inside the call that shows it, so the daemon's loop
//! (which asks the menu bar for its next command every few milliseconds) never comes back
//! until the user clicks OK. This test shows each result window, then works the loop as the
//! daemon does: a menu pick must come out of `next_command` and the configuration file must
//! reload, both with the windows still on screen. With a modal window the first call never
//! returns, so the parent kills the child after a deadline and the test fails.
//!
//! The windows are real AppKit on the main thread; no Accessibility grant, unlocked screen or
//! focus is needed. The test re-runs itself with `XDG_CONFIG_HOME` pointing at a fresh
//! directory, so the user's own `~/.config/nomouse` is never read or written.

use std::io::Write as _;
use std::process::ExitCode;

/// Reports progress on stderr (a harness-less test has no other channel).
fn note(message: &str) {
    // Nothing useful can be done when stderr is closed.
    writeln!(std::io::stderr(), "{message}").ok();
}

#[cfg(not(target_os = "macos"))]
fn main() -> ExitCode {
    note("SKIP: the result windows are macOS only");
    ExitCode::SUCCESS
}

#[cfg(target_os = "macos")]
fn main() -> ExitCode {
    macos::main()
}

#[cfg(target_os = "macos")]
mod macos {
    use std::process::{Command, ExitCode};
    use std::time::{Duration, Instant};

    use nomouse_core::config::Theme;
    use nomouse_core::menu::{MenuAction, MenuState};
    use nomouse_core::settings::{Change, Settings};
    use nomouse_core::update::{UpdateStatus, Version};
    use nomouse_macos::menubar::{Command as Asked, MenuBar};

    use super::note;

    /// Set in the re-run that does the work.
    const CHILD: &str = "NOMOUSE_RESULT_WINDOWS_TEST";
    /// How long the re-run may take before the loop counts as blocked.
    const BLOCKED_AFTER: Duration = Duration::from_secs(60);
    /// How long one click may take to be carried out.
    const DEADLINE: Duration = Duration::from_secs(5);
    /// The doctor's report, as the daemon would show it.
    const REPORT: &str = "nomouse 0.1.6 doctor\nok   accessibility\nok   config\n";

    pub(super) fn main() -> ExitCode {
        if std::env::var_os(CHILD).is_some() {
            return run();
        }
        let dir =
            std::env::temp_dir().join(format!("nomouse-result-windows-{}", std::process::id()));
        let result = std::fs::create_dir_all(&dir)
            .map_err(|error| error.to_string())
            .and_then(|()| wait_for_child(&dir));
        let _ = std::fs::remove_dir_all(&dir);
        match result {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => {
                note("FAIL: the result windows test failed");
                ExitCode::FAILURE
            }
            Err(error) => {
                note(&format!("FAIL: {error}"));
                ExitCode::FAILURE
            }
        }
    }

    /// Runs the test in a child and kills it when it is stuck; whether it passed.
    fn wait_for_child(dir: &std::path::Path) -> Result<bool, String> {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let mut child = Command::new(exe)
            .env(CHILD, "1")
            .env("XDG_CONFIG_HOME", dir)
            .spawn()
            .map_err(|error| error.to_string())?;
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                return Ok(status.success());
            }
            if start.elapsed() > BLOCKED_AFTER {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "the daemon's loop was still blocked after {BLOCKED_AFTER:?}: \
                     a result window is modal"
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Services the app for up to `DEADLINE` until `done` holds; whether it held.
    fn pump_until(bar: &mut MenuBar, mut done: impl FnMut(&MenuBar) -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < DEADLINE {
            bar.pump(Duration::from_millis(20));
            while bar.next_command().is_some() {}
            if done(bar) {
                return true;
            }
        }
        false
    }

    /// What the daemon's loop gets from the menu bar next, waiting up to `DEADLINE`.
    fn next_asked(bar: &mut MenuBar) -> Option<Asked> {
        let start = Instant::now();
        while start.elapsed() < DEADLINE {
            bar.pump(Duration::from_millis(20));
            if let Some(command) = bar.next_command() {
                return Some(command);
            }
        }
        None
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
        let Some(path) = nomouse_core::config::default_path() else {
            note("FAIL: no configuration path");
            return ExitCode::FAILURE;
        };
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
                    "FAIL: the daemon's model cannot open the file: {error}"
                ));
                return ExitCode::FAILURE;
            }
        };

        // Both windows open and stay open. A modal alert never returns from these calls.
        bar.show_report("nomouse doctor: all checks passed", REPORT);
        checks.check(
            bar.notice_count() == 1 && bar.notice_report().as_deref() == Some(REPORT),
            "the doctor's report is on screen and the call returned",
        );
        bar.announce_update(Ok(UpdateStatus::UpToDate(Version(0, 1, 6))));
        checks.check(
            bar.notice_count() == 2,
            "the up to date answer is on screen beside it",
        );

        // A hotkey's request reaches the daemon's loop (the loop asks the menu bar for it).
        bar.choose(MenuAction::Hints);
        checks.check(
            next_asked(&mut bar) == Some(Asked::Run(MenuAction::Hints)),
            "a Show Hints request is handled while both windows are open",
        );

        // A configuration reload: the file changes and the daemon's model picks it up.
        let edited = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, "[overlay]\ntheme = \"dark\"\n"));
        checks.check(edited.is_ok(), "config.toml is edited by hand");
        checks.check(
            matches!(daemon.refresh(), Change::Reloaded)
                && daemon.config().overlay.theme == Theme::Dark,
            "the configuration reloads while both windows are open",
        );
        checks.check(bar.notice_count() == 2, "both windows are still open");

        // OK closes the newest window, then the report's.
        checks.check(
            bar.press_notice_button("OK"),
            "the up to date window has an OK button",
        );
        let closed = pump_until(&mut bar, |bar| bar.notice_count() == 1);
        checks.check(
            closed,
            "OK closes the up to date window and keeps the report",
        );
        checks.check(
            bar.press_notice_button("OK"),
            "the report window has an OK button",
        );
        let closed = pump_until(&mut bar, |bar| bar.notice_count() == 0);
        checks.check(closed, "OK closes the report window");

        note(&format!("--- {} failed", checks.failed));
        if checks.failed == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }
}
