//! Starting nomouse again once the first run's required permission is granted (NM-APP-10):
//! the grant applies to a process from its start.
//!
//! The new daemon is a new process, never this one replaced with `exec`. macOS's menu bar
//! knows a process by its pid and its pid version, and `exec` changes the version: the menu
//! bar then refuses every status item the replaced image asks for ("process has mismatched pid
//! version") and the icon never shows, though the daemon runs. So this daemon lets go of the
//! single-instance lock, starts the next one, and ends (or, in a terminal, waits for it), and
//! the next one takes the lock: one daemon, one icon, one set of hotkeys.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::lock::InstanceLock;

/// How the next daemon starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Plan {
    /// Opened as a new instance of the app bundle by `LaunchServices` (`open -n`), as when
    /// the user opened it; this process then ends. For an app started without a terminal.
    Open(PathBuf),
    /// Started as a child with this terminal, which waits for it and ends with its exit
    /// status, so the terminal keeps its foreground job and `Ctrl-C` still reaches it.
    Wait(PathBuf),
    /// Started as a child without a terminal; this process ends at once.
    Detach(PathBuf),
}

/// How to start again `exe` (this executable's real path): opened as the app when it is a
/// bundle's executable (`app`) started without a terminal, else as a child, waited for in a
/// terminal.
pub(crate) fn plan(exe: &Path, app: Option<&Path>, terminal: bool) -> Plan {
    match (terminal, app) {
        (true, _) => Plan::Wait(exe.to_owned()),
        (false, Some(app)) => Plan::Open(app.to_owned()),
        (false, None) => Plan::Detach(exe.to_owned()),
    }
}

/// The plan for this process: its executable, its bundle on macOS, and whether stderr is a
/// terminal.
///
/// # Errors
/// When the executable's path cannot be read.
pub(crate) fn current_plan(terminal: bool) -> Result<Plan, String> {
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| format!("cannot start nomouse again: {e}"))?;
    #[cfg(target_os = "macos")]
    let app = nomouse_macos::bundle::app_of(&exe).map(Path::to_owned);
    #[cfg(not(target_os = "macos"))]
    let app: Option<PathBuf> = None;
    Ok(plan(&exe, app.as_deref(), terminal))
}

/// Releases `lock`, then runs `start`: the next daemon must find the lock free, or it would
/// refuse to run beside this one.
pub(crate) fn hand_over<T>(lock: InstanceLock, start: impl FnOnce() -> T) -> T {
    drop(lock);
    start()
}

/// Starts the next daemon as `plan` says, with this process's arguments. Returns when this
/// process should end; for [`Plan::Wait`], after the child ended, and with its failure.
///
/// # Errors
/// When the next daemon cannot be started, or ([`Plan::Wait`]) ended with a failure.
pub(crate) fn start(plan: &Plan, args: &[OsString]) -> Result<String, String> {
    match plan {
        Plan::Open(app) => {
            tracing::info!("permissions granted; opening {} again", app.display());
            let status = Command::new("/usr/bin/open")
                .arg("-n")
                .arg(app)
                .arg("--args")
                .args(args)
                .stdin(Stdio::null())
                .status()
                .map_err(|e| format!("cannot open {}: {e}", app.display()))?;
            if status.success() {
                Ok(String::new())
            } else {
                Err(format!("cannot open {}: open {status}", app.display()))
            }
        }
        Plan::Wait(exe) => {
            tracing::info!("permissions granted; starting {} again", exe.display());
            let status = Command::new(exe)
                .args(args)
                .status()
                .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;
            if status.success() {
                Ok(String::new())
            } else {
                // The child said why on this terminal already.
                Err(format!("nomouse ended: {status}"))
            }
        }
        Plan::Detach(exe) => {
            tracing::info!("permissions granted; starting {} again", exe.display());
            Command::new(exe)
                .args(args)
                .stdin(Stdio::null())
                .spawn()
                .map(|_child| String::new())
                .map_err(|e| format!("cannot start {}: {e}", exe.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::{self, LockError};

    const EXE: &str = "/Applications/nomouse.app/Contents/MacOS/nomouse";
    const APP: &str = "/Applications/nomouse.app";

    #[test]
    fn the_app_without_a_terminal_is_opened_again_as_a_new_instance() {
        assert_eq!(
            plan(Path::new(EXE), Some(Path::new(APP)), false),
            Plan::Open(PathBuf::from(APP))
        );
    }

    #[test]
    fn in_a_terminal_the_next_daemon_is_a_child_it_waits_for() {
        assert_eq!(
            plan(Path::new(EXE), Some(Path::new(APP)), true),
            Plan::Wait(PathBuf::from(EXE))
        );
        assert_eq!(
            plan(Path::new("/usr/bin/nomouse"), None, true),
            Plan::Wait(PathBuf::from("/usr/bin/nomouse"))
        );
    }

    #[test]
    fn a_bare_binary_without_a_terminal_starts_a_detached_child() {
        assert_eq!(
            plan(Path::new("/usr/bin/nomouse"), None, false),
            Plan::Detach(PathBuf::from("/usr/bin/nomouse"))
        );
    }

    #[test]
    fn the_next_daemon_finds_the_lock_free_and_holds_it_alone() {
        let _alone = lock::children_guard();
        let path =
            std::env::temp_dir().join(format!("nomouse-relaunch-test-{}.lock", std::process::id()));
        let old = lock::acquire(&path).unwrap();
        assert!(matches!(lock::acquire(&path), Err(LockError::Taken(_))));
        let next = hand_over(old, || lock::acquire(&path)).unwrap();
        // The handed-over lock is the only one: a third start is refused.
        assert!(matches!(lock::acquire(&path), Err(LockError::Taken(_))));
        drop(next);
        std::fs::remove_file(&path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_waited_child_reports_its_failure() {
        let _alone = lock::children_guard();
        let plan = Plan::Wait(PathBuf::from("/bin/sh"));
        let args = ["-c", "exit 3"].map(OsString::from);
        let error = start(&plan, &args).unwrap_err();
        assert!(error.contains('3'), "{error}");
        let args = ["-c", "exit 0"].map(OsString::from);
        assert_eq!(start(&plan, &args), Ok(String::new()));
    }
}
