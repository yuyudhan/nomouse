//! Start at login, for real: the menu's "Start at login" item registers the app with
//! `SMAppService`, the login item appears in the system's list, and picking the item again
//! removes it.
//!
//! `SMAppService.mainAppService` only works for an app bundle, so the test builds one: it
//! copies its own executable into a signed `nomouse.app` under a scratch directory (with
//! `scripts/release/app.sh`, as the release does, but with its own bundle identifier so the
//! installed nomouse's login item is never touched) and runs that copy, which works the real
//! [`MenuBar`] menu path and reads the state back from the system. The run ends with the item
//! removed, whatever happened.
//!
//! It needs the user's login session (the login item belongs to the user, not to a build
//! directory), so it skips on GitHub Actions' macOS runners, which have none.

use std::io::Write as _;
use std::process::ExitCode;

/// Reports progress on stderr (a harness-less test has no other channel).
fn note(message: &str) {
    // Nothing useful can be done when stderr is closed.
    writeln!(std::io::stderr(), "{message}").ok();
}

#[cfg(not(target_os = "macos"))]
fn main() -> ExitCode {
    note("SKIP: start at login is macOS only");
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

    use nomouse_core::menu::{MenuAction, MenuState};
    use nomouse_macos::menubar::MenuBar;
    use nomouse_sys_macos::login::{self, LoginItem};

    use super::note;

    /// Set in the re-run inside the app bundle.
    const CHILD: &str = "NOMOUSE_START_AT_LOGIN_TEST";
    /// The bundle identifier of the test app (not the installed nomouse's).
    const BUNDLE_ID: &str = "com.yuyudhan.nomouse.logintest";
    /// How long the system may take to show a change.
    const DEADLINE: Duration = Duration::from_secs(10);

    pub(super) fn main() -> ExitCode {
        if std::env::var_os(CHILD).is_some() {
            // As the daemon starts: through a symlink, it starts again from the bundle.
            nomouse_macos::bundle::reexec_from_bundle();
            return run();
        }
        if std::env::var_os("GITHUB_ACTIONS").is_some() {
            note("SKIP: a login item needs the user's login session; GitHub Actions has none");
            return ExitCode::SUCCESS;
        }
        let dir =
            std::env::temp_dir().join(format!("nomouse-start-at-login-{}", std::process::id()));
        let result = std::fs::create_dir_all(&dir)
            .map_err(|error| error.to_string())
            .and_then(|()| run_in_bundle(&dir));
        let _ = std::fs::remove_dir_all(&dir);
        match result {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => {
                note("FAIL: the start at login test failed");
                ExitCode::FAILURE
            }
            Err(error) => {
                note(&format!("FAIL: {error}"));
                ExitCode::FAILURE
            }
        }
    }

    /// Wraps this executable in a signed app bundle under `dir` and runs it twice, by its path
    /// in the bundle and through a symlink to it as `install.sh` makes (`~/.local/bin/nomouse`,
    /// where `nomouse daemon` is usually started from); whether both passed.
    fn run_in_bundle(dir: &Path) -> Result<bool, String> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let built = Command::new(root.join("scripts/release/app.sh"))
            .arg(&exe)
            .arg(dir)
            .env("NOMOUSE_BUNDLE_ID", BUNDLE_ID)
            .output()
            .map_err(|error| format!("cannot run app.sh: {error}"))?;
        if !built.status.success() {
            return Err(format!(
                "app.sh failed: {}",
                String::from_utf8_lossy(&built.stderr)
            ));
        }
        let app = dir.join("nomouse.app");
        let link = dir.join("bin/nomouse");
        std::fs::create_dir_all(dir.join("bin")).map_err(|error| error.to_string())?;
        std::os::unix::fs::symlink(app.join("Contents/MacOS/nomouse"), &link)
            .map_err(|error| error.to_string())?;
        let mut passed = true;
        for launched in [app.join("Contents/MacOS/nomouse"), link] {
            note(&format!("--- run from {}", launched.display()));
            let status = Command::new(&launched)
                .env(CHILD, "1")
                .status()
                .map_err(|error| error.to_string())?;
            passed &= status.success();
        }
        Ok(passed)
    }

    /// The paths of the login items System Events lists, or why it cannot say.
    fn listed_login_items() -> Result<Vec<PathBuf>, String> {
        let output = Command::new("/usr/bin/osascript")
            .args([
                "-e",
                "tell application \"System Events\" to get the path of every login item",
            ])
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim()
            .split(", ")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect())
    }

    /// Whether the bundle this process runs from is among the listed login items.
    fn listed(app: &Path) -> Result<bool, String> {
        let app = app.canonicalize().map_err(|error| error.to_string())?;
        Ok(listed_login_items()?
            .iter()
            .any(|path| path.canonicalize().is_ok_and(|path| path == app)))
    }

    /// Services the app until `done` holds or the deadline passes; whether it held.
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

    /// The test, inside the bundle.
    fn run() -> ExitCode {
        let Some(app) = std::env::current_exe()
            .and_then(|exe| exe.canonicalize())
            .ok()
            .and_then(|exe| exe.ancestors().nth(3).map(Path::to_path_buf))
        else {
            note("FAIL: no app bundle around the executable");
            return ExitCode::FAILURE;
        };
        let mut checks = Checks { failed: 0 };
        // Start from "off" even after an earlier run died with the item on.
        let _ = login::set_login_item(false);
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
        checks.check(
            login::login_item() == LoginItem::Disabled && bar.state().start_at_login == Some(false),
            "it starts with the login item off and the menu shows it unchecked",
        );
        let listing = listed(&app);

        bar.choose(MenuAction::StartAtLogin);
        checks.check(
            login::login_item() == LoginItem::Enabled,
            "Start at login registers the app: SMAppService status is enabled",
        );
        checks.check(
            bar.state().start_at_login == Some(true),
            "the menu shows Start at login checked",
        );
        if let Err(reason) = &listing {
            note(&format!(
                "NOTE: System Events cannot list login items here ({reason}); \
                 the listing is not checked"
            ));
        } else {
            let appears = pump_until(&mut bar, |_| listed(&app) == Ok(true));
            checks.check(appears, "the app appears in the system's login items");
        }

        bar.choose(MenuAction::StartAtLogin);
        checks.check(
            login::login_item() == LoginItem::Disabled,
            "Start at login again unregisters it: SMAppService status is off",
        );
        checks.check(
            bar.state().start_at_login == Some(false),
            "the menu shows Start at login unchecked",
        );
        if listing.is_ok() {
            let gone = pump_until(&mut bar, |_| listed(&app) == Ok(false));
            checks.check(gone, "the app is gone from the system's login items");
        }

        // Whatever happened, leave nothing registered.
        let _ = login::set_login_item(false);
        note(&format!("--- {} failed", checks.failed));
        if checks.failed == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }
}
