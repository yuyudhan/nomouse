//! The guided first run (NM-APP-10): `nomouse setup`, and the daemon's start when a required
//! permission is missing. The step machine is `nomouse_core::setup`; this module shows it in a
//! terminal (or the backend's dialogs when there is no terminal) and restarts the daemon once
//! a required permission is granted.

use std::io::{IsTerminal, Write};
use std::time::Duration;

use nomouse_core::setup::{Outcome, Permission, Presenter, Setup, Step, guide};

use crate::backend;

/// How long the flow waits for each grant after asking.
const GRANT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The text a terminal shows for `step`, or `None` for a repeated wait.
pub(crate) fn describe(step: &Step) -> Option<String> {
    Some(match step {
        Step::AlreadyGranted(permission) | Step::Granted(permission) => {
            format!("[ok  ] {}: granted", permission.name)
        }
        Step::Explain(permission) => format!(
            "[need] {}: {}\n       {}",
            permission.name, permission.why, permission.how
        ),
        Step::Requested(permission) => format!(
            "       asked for {}; waiting until it is granted (Ctrl-C to stop)",
            permission.name
        ),
        Step::Waiting(_) => return None,
        Step::RequestFailed(permission, reason) => {
            format!(
                "[FAIL] {}: {reason}\n       {}",
                permission.name, permission.how
            )
        }
        Step::TimedOut(permission) => format!(
            "[FAIL] {}: still not granted after {} minutes. {}",
            permission.name,
            GRANT_TIMEOUT.as_secs() / 60,
            permission.how
        ),
        Step::Done { restart: true } => {
            "nomouse has every permission it needs; restart nomouse to use them.".to_owned()
        }
        Step::Done { restart: false } => "nomouse has every permission it needs.".to_owned(),
    })
}

/// Shows the flow as lines of text.
pub(crate) struct Terminal<W: Write> {
    out: W,
}

impl<W: Write> Terminal<W> {
    pub(crate) const fn new(out: W) -> Self {
        Self { out }
    }
}

impl<W: Write> Presenter for Terminal<W> {
    fn show(&mut self, step: &Step) -> bool {
        if let Some(text) = describe(step) {
            // A closed terminal must not stop the flow; the outcome is still returned.
            writeln!(self.out, "{text}").ok();
        }
        true
    }

    fn wait(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// `nomouse setup`: walks through every permission of this OS, required or not.
pub(crate) fn setup_command() -> Result<String, String> {
    let mut probe = backend::first_run_probe();
    let mut setup = Setup::new(backend::first_run_permissions(), GRANT_TIMEOUT);
    let mut terminal = Terminal::new(std::io::stdout().lock());
    match guide(&mut setup, probe.as_mut(), &mut terminal) {
        Outcome::Ready { .. } => Ok(String::new()),
        Outcome::Stopped(_) => Err("setup stopped; see above".to_owned()),
        Outcome::Quit(permission) => Err(format!("{} is still missing", permission.name)),
    }
}

/// The daemon's first run: when a required permission is missing, explains it (in dialogs
/// when nomouse has no terminal, as when `nomouse.app` is opened), asks for it, waits, then
/// restarts the daemon in place. Returns at once when everything is granted.
///
/// # Errors
/// When the user quits, the request fails or the grant does not come in time.
pub(crate) fn before_daemon() -> Result<(), String> {
    let required: Vec<Permission> = backend::first_run_permissions()
        .into_iter()
        .filter(|permission| permission.required)
        .collect();
    let mut probe = backend::first_run_probe();
    if Setup::all_granted(&required, probe.as_mut()) {
        return Ok(());
    }
    let mut presenter: Box<dyn Presenter> = if std::io::stderr().is_terminal() {
        Box::new(Terminal::new(std::io::stderr()))
    } else {
        backend::first_run_dialogs().unwrap_or_else(|| Box::new(Terminal::new(std::io::stderr())))
    };
    let mut setup = Setup::new(required, GRANT_TIMEOUT);
    match guide(&mut setup, probe.as_mut(), presenter.as_mut()) {
        Outcome::Ready { restart: true } => restart(),
        Outcome::Ready { restart: false } => Ok(()),
        Outcome::Stopped(step) => Err(describe(&step).unwrap_or_default()),
        Outcome::Quit(permission) => Err(format!("{} not granted; quitting", permission.name)),
    }
}

/// Replaces this process with a fresh start of the same command, so permissions granted while
/// it ran apply from the beginning. The single-instance lock is released by the exec (it is
/// close-on-exec) and taken again by the new image.
#[cfg(unix)]
fn restart() -> Result<(), String> {
    use std::os::unix::process::CommandExt;

    let exe = std::env::current_exe().map_err(|e| format!("cannot restart: {e}"))?;
    tracing::info!("permissions granted; restarting {}", exe.display());
    let error = std::process::Command::new(&exe)
        .args(std::env::args_os().skip(1))
        .exec();
    Err(format!("cannot restart {}: {error}", exe.display()))
}

/// No OS without exec needs a restart for a permission yet.
#[cfg(not(unix))]
fn restart() -> Result<(), String> {
    Err("permissions granted; start nomouse again".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCESSIBILITY: Permission = Permission {
        id: "accessibility",
        name: "Accessibility",
        why: "read other apps",
        how: "turn nomouse on",
        required: true,
    };

    #[test]
    fn explaining_says_why_and_how() {
        let text = describe(&Step::Explain(ACCESSIBILITY)).unwrap_or_default();
        assert!(text.contains("read other apps") && text.contains("turn nomouse on"));
    }

    #[test]
    fn waiting_prints_nothing_so_polls_do_not_flood_the_terminal() {
        let mut out = Vec::new();
        let mut terminal = Terminal::new(&mut out);
        for _ in 0..5 {
            assert!(terminal.show(&Step::Waiting(ACCESSIBILITY)));
        }
        assert_eq!(out, Vec::<u8>::new());
    }

    #[test]
    fn a_stop_tells_the_user_how_to_grant() {
        for step in [
            Step::TimedOut(ACCESSIBILITY),
            Step::RequestFailed(ACCESSIBILITY, "no settings app".to_owned()),
        ] {
            let text = describe(&step).unwrap_or_default();
            assert!(text.starts_with("[FAIL]") && text.contains("turn nomouse on"));
        }
    }
}
