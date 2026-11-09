//! `nomouse launch`: what the desktop entry `io.github.yuyudhan.nomouse.desktop` runs when
//! nomouse is picked in the app launcher (GNOME: Super, then type "nomouse"). It starts the
//! daemon unless one is running and then always shows a desktop notification with the daemon's
//! state and the hotkeys, so picking nomouse has a visible result. On GNOME Wayland, where
//! nomouse cannot run, the notification says to log in with "Ubuntu on Xorg".
//!
//! `nomouse launch --doctor` is the entry's "nomouse doctor" action: it opens `nomouse doctor`
//! in a terminal, or shows the report as a notification when no terminal is installed.

use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{ErrorKind, Read as _, Seek as _, SeekFrom};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use nomouse_core::config::{Config, HotkeysConfig};

use crate::ipc;
use crate::lock::{self, LockError};

/// The desktop entry's id, which is also the icon's name.
pub(crate) const APP_ID: &str = "io.github.yuyudhan.nomouse";
/// How long a started daemon gets to answer on its command socket.
const STARTUP: Duration = Duration::from_secs(5);
/// How often the start is checked within [`STARTUP`].
const POLL: Duration = Duration::from_millis(100);
/// The most of the daemon's log (its last bytes) a failure notification quotes.
const LOG_TAIL: u64 = 600;
/// The most of the doctor report a notification shows.
const REPORT_LIMIT: usize = 1500;

/// What became of the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Daemon {
    /// One was running already.
    AlreadyRunning,
    /// This launch started it and it answers on its command socket.
    Started,
    /// This launch started it; it is still running but has not answered yet (it may be waiting
    /// on a permission prompt, or have no command socket).
    Starting,
    /// It could not be started, or exited at once; the reason.
    Failed(String),
}

/// A desktop notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Note {
    /// The title line.
    pub(crate) summary: String,
    /// The text below it, plain (escaped for notification markup when sent).
    pub(crate) body: String,
}

/// Whether `env` describes a GNOME session on Wayland, where nomouse cannot run (Mutter has no
/// overlay layer, virtual pointer or app hotkeys). Uses the installer's rule: a GNOME part in
/// `XDG_CURRENT_DESKTOP` and `XDG_SESSION_TYPE=wayland`.
pub(crate) fn gnome_wayland(env: impl Fn(&str) -> Option<String>) -> bool {
    let wayland = env("XDG_SESSION_TYPE").is_some_and(|kind| kind.eq_ignore_ascii_case("wayland"));
    let gnome = env("XDG_CURRENT_DESKTOP").is_some_and(|desktop| {
        desktop
            .split(':')
            .any(|part| part.eq_ignore_ascii_case("gnome"))
    });
    wayland && gnome
}

/// What GNOME Wayland users are told to do.
const XORG_HINT: &str = "GNOME on Wayland cannot run nomouse. Log out, click the gear on the \
                         login screen, choose \"Ubuntu on Xorg\" (\"GNOME on Xorg\" elsewhere) \
                         and log in again.";

/// The notification for `daemon`, listing `hotkeys` (absent when the configuration did not
/// load) and the Xorg hint when `gnome_wayland`.
pub(crate) fn note(daemon: &Daemon, hotkeys: Option<&HotkeysConfig>, gnome_wayland: bool) -> Note {
    let summary = match daemon {
        Daemon::AlreadyRunning => "nomouse is running",
        Daemon::Started => "nomouse started",
        Daemon::Starting => "nomouse is starting",
        Daemon::Failed(_) => "nomouse could not start",
    };
    let mut lines = Vec::new();
    match daemon {
        Daemon::Failed(reason) => lines.push(reason.trim().to_owned()),
        Daemon::Starting => lines.push(
            "The daemon runs but has not answered yet; it may be waiting for a permission."
                .to_owned(),
        ),
        Daemon::AlreadyRunning | Daemon::Started => {}
    }
    if gnome_wayland {
        lines.push(XORG_HINT.to_owned());
    }
    if let Some(keys) = hotkeys {
        lines.push(format!(
            "{}: hints\n{}: continuous hints\n{}: scroll\n{}: search",
            keys.hints, keys.continuous, keys.scroll, keys.search
        ));
    }
    lines.push(
        "Check the setup: right-click nomouse in the app launcher, then nomouse doctor.".to_owned(),
    );
    Note {
        summary: summary.to_owned(),
        body: lines.join("\n\n"),
    }
}

/// `text` with the characters notification markup gives a meaning escaped.
fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// `text` as a `GVariant` text-format string literal, for `gdbus call`.
fn gvariant_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The commands that can show `note`, in the order to try them: `notify-send`, else the
/// notification service over D-Bus with `gdbus`.
pub(crate) fn notify_commands(note: &Note) -> [(&'static str, Vec<String>); 2] {
    let body = escape_markup(&note.body);
    [
        (
            "notify-send",
            vec![
                "--app-name=nomouse".to_owned(),
                format!("--icon={APP_ID}"),
                format!("--hint=string:desktop-entry:{APP_ID}"),
                "--".to_owned(),
                note.summary.clone(),
                body.clone(),
            ],
        ),
        (
            "gdbus",
            vec![
                "call".to_owned(),
                "--session".to_owned(),
                "--dest=org.freedesktop.Notifications".to_owned(),
                "--object-path=/org/freedesktop/Notifications".to_owned(),
                "--method=org.freedesktop.Notifications.Notify".to_owned(),
                "nomouse".to_owned(),
                "0".to_owned(),
                APP_ID.to_owned(),
                gvariant_string(&note.summary),
                gvariant_string(&body),
                "[]".to_owned(),
                format!("{{\"desktop-entry\": <\"{APP_ID}\">}}"),
                "-1".to_owned(),
            ],
        ),
    ]
}

/// Shows `note` with the first of [`notify_commands`] that works.
///
/// # Errors
/// When none does; the reasons.
fn notify(note: &Note) -> Result<(), String> {
    let mut problems = Vec::new();
    for (program, args) in notify_commands(note) {
        match Command::new(program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
        {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => problems.push(format!(
                "{program}: {} {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                problems.push(format!("{program}: not installed"));
            }
            Err(error) => problems.push(format!("{program}: {error}")),
        }
    }
    Err(format!(
        "cannot show a notification ({})",
        problems.join("; ")
    ))
}

/// The executable `name` on the search path `path` (a `PATH` value), if any.
pub(crate) fn find_program(name: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .map(|dir| dir.join(name))
        .find(|candidate| {
            std::fs::metadata(candidate)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

/// Terminals and the arguments that make them run a command, in the order they are tried.
const TERMINALS: [(&str, &[&str]); 6] = [
    ("gnome-terminal", &["--"]),
    ("kgx", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("x-terminal-emulator", &["-e"]),
    ("xterm", &["-e"]),
];

/// The shell script the terminal runs: the doctor report, then a wait so the window stays
/// until the user has read it. `$0` is the nomouse executable.
const DOCTOR_SCRIPT: &str =
    "\"$0\" doctor; printf '\\nPress Enter to close this window. '; read -r _";

/// The terminal command that shows `nomouse doctor` for the executable `exe`: the first of
/// [`TERMINALS`] that `find` locates, with its arguments.
pub(crate) fn terminal_command(
    find: impl Fn(&str) -> Option<PathBuf>,
    exe: &Path,
) -> Option<(PathBuf, Vec<OsString>)> {
    TERMINALS.iter().find_map(|(name, flags)| {
        let terminal = find(name)?;
        let mut args: Vec<OsString> = flags.iter().map(OsString::from).collect();
        args.extend(["sh", "-c", DOCTOR_SCRIPT].map(OsString::from));
        args.push(exe.as_os_str().to_owned());
        Some((terminal, args))
    })
}

/// `text` cut to at most `limit` bytes on a character boundary, marked when cut.
fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// The last [`LOG_TAIL`] bytes of the file at `path`, trimmed, or nothing.
fn log_tail(path: &Path) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let len = file.metadata().map_or(0, |meta| meta.len());
    if file
        .seek(SeekFrom::Start(len.saturating_sub(LOG_TAIL)))
        .is_err()
    {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&bytes).trim().to_owned()
}

/// Starts `nomouse daemon` in its own process group, logging to `log`, and waits up to
/// [`STARTUP`] for it to answer on the command socket.
fn start_daemon(exe: &Path, log: &Path) -> Daemon {
    let stderr = match File::create(log) {
        Ok(file) => file,
        Err(error) => return Daemon::Failed(format!("cannot write {}: {error}", log.display())),
    };
    let child = Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .process_group(0)
        .spawn();
    match child {
        Ok(child) => wait_for_daemon(child, log, ipc::current_path().ok().as_deref()),
        Err(error) => Daemon::Failed(format!("cannot run {} daemon: {error}", exe.display())),
    }
}

/// Watches the started daemon `child` until it answers on `socket`, exits or [`STARTUP`]
/// passes.
fn wait_for_daemon(mut child: Child, log: &Path, socket: Option<&Path>) -> Daemon {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let tail = log_tail(log);
                return Daemon::Failed(if tail.is_empty() {
                    format!("the daemon exited ({status})")
                } else {
                    tail
                });
            }
            Ok(None) => {}
            Err(error) => return Daemon::Failed(format!("cannot watch the daemon: {error}")),
        }
        if socket.is_some_and(ipc::ping) {
            return Daemon::Started;
        }
        if started.elapsed() >= STARTUP {
            return Daemon::Starting;
        }
        std::thread::sleep(POLL);
    }
}

/// Starts the daemon unless one holds the single-instance lock.
fn ensure_daemon() -> Daemon {
    let lock_path = lock::current_path();
    match lock::acquire(&lock_path) {
        Ok(free) => drop(free),
        Err(LockError::Taken(_)) => return Daemon::AlreadyRunning,
        Err(error) => return Daemon::Failed(error.to_string()),
    }
    match std::env::current_exe() {
        Ok(exe) => start_daemon(&exe, &lock_path.with_extension("log")),
        Err(error) => Daemon::Failed(format!("cannot find the nomouse executable: {error}")),
    }
}

/// Shows `note` and returns its text for the terminal, which also gets a notification failure.
fn show(note: &Note) -> String {
    let mut out = format!("{}\n\n{}\n", note.summary, note.body);
    if let Err(problem) = notify(note) {
        let _ = write!(out, "\nnomouse: {problem}\n");
    }
    out
}

/// `nomouse launch`: starts the daemon unless it runs, then notifies its state and hotkeys.
/// `config` is the configuration as loaded; when it failed to load, the daemon is not started
/// and the notification says why.
///
/// # Errors
/// When the daemon could not start (after notifying), so a terminal run exits non-zero.
pub(crate) fn launch(config: Result<Config, String>) -> Result<String, String> {
    let wayland_gnome = gnome_wayland(|name| std::env::var(name).ok());
    let (daemon, hotkeys) = match config {
        Ok(config) => (ensure_daemon(), Some(config.hotkeys)),
        Err(error) => (
            Daemon::Failed(format!("the configuration is invalid: {error}")),
            None,
        ),
    };
    let out = show(&note(&daemon, hotkeys.as_ref(), wayland_gnome));
    match daemon {
        Daemon::Failed(_) => Err(out.trim_end().to_owned()),
        Daemon::AlreadyRunning | Daemon::Started | Daemon::Starting => Ok(out),
    }
}

/// `nomouse launch --doctor`: `nomouse doctor` in a new terminal window, or the report as a
/// notification when no terminal is installed.
///
/// # Errors
/// When nomouse cannot find itself or the report cannot be made.
pub(crate) fn doctor() -> Result<String, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("cannot find the nomouse executable: {error}"))?;
    let path = std::env::var_os("PATH");
    let find = |name: &str| find_program(name, path.as_deref());
    if let Some((terminal, args)) = terminal_command(find, &exe) {
        match Command::new(&terminal)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
        {
            Ok(_) => return Ok(format!("opened nomouse doctor in {}\n", terminal.display())),
            Err(error) => tracing::warn!("{}: {error}", terminal.display()),
        }
    }
    let (report, failed) = crate::doctor_report()?;
    let note = Note {
        summary: if failed {
            "nomouse doctor: some checks failed".to_owned()
        } else {
            "nomouse doctor: all checks pass".to_owned()
        },
        body: truncate(&report, REPORT_LIMIT),
    };
    Ok(show(&note))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn gnome_wayland_needs_both_gnome_and_wayland() {
        let ubuntu = [
            ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"),
            ("XDG_SESSION_TYPE", "wayland"),
        ];
        assert!(gnome_wayland(env(&ubuntu)));
        assert!(!gnome_wayland(env(&[
            ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"),
            ("XDG_SESSION_TYPE", "x11")
        ])));
        assert!(!gnome_wayland(env(&[
            ("XDG_CURRENT_DESKTOP", "sway"),
            ("XDG_SESSION_TYPE", "wayland")
        ])));
        assert!(!gnome_wayland(env(&[])));
    }

    #[test]
    fn note_lists_state_hotkeys_and_the_xorg_hint() {
        let keys = Config::default().hotkeys;
        let running = note(&Daemon::AlreadyRunning, Some(&keys), false);
        assert_eq!(running.summary, "nomouse is running");
        assert!(
            running.body.contains(&format!("{}: hints", keys.hints)),
            "{}",
            running.body
        );
        assert!(running.body.contains(&format!("{}: search", keys.search)));
        assert!(!running.body.contains("Xorg"));
        assert!(running.body.contains("nomouse doctor"));

        let failed = note(
            &Daemon::Failed("no display\n".to_owned()),
            Some(&keys),
            true,
        );
        assert_eq!(failed.summary, "nomouse could not start");
        assert!(failed.body.starts_with("no display\n\n"), "{}", failed.body);
        assert!(failed.body.contains("\"Ubuntu on Xorg\""));
        assert!(failed.body.contains(&format!("{}: scroll", keys.scroll)));

        assert_eq!(
            note(&Daemon::Started, None, false).summary,
            "nomouse started"
        );
        let starting = note(&Daemon::Starting, None, false);
        assert_eq!(starting.summary, "nomouse is starting");
        assert!(!starting.body.contains(": hints"));
    }

    #[test]
    fn notify_commands_escape_their_text() {
        let note = Note {
            summary: "nomouse".to_owned(),
            body: "a <b> & \"c\"\nd\\e".to_owned(),
        };
        let [(send, send_args), (gdbus, gdbus_args)] = notify_commands(&note);
        assert_eq!(send, "notify-send");
        assert_eq!(
            send_args[send_args.len() - 1],
            "a &lt;b&gt; &amp; \"c\"\nd\\e"
        );
        assert!(send_args.contains(&format!("--icon={APP_ID}")));
        assert_eq!(gdbus, "gdbus");
        assert!(gdbus_args.contains(&"\"a &lt;b&gt; &amp; \\\"c\\\"\\nd\\\\e\"".to_owned()));
        assert_eq!(gdbus_args.last().map(String::as_str), Some("-1"));
    }

    #[test]
    fn terminal_command_uses_the_first_terminal_found() {
        let exe = Path::new("/opt/nomouse");
        assert_eq!(terminal_command(|_| None, exe), None);
        let (terminal, args) = terminal_command(
            |name| (name == "xterm").then(|| PathBuf::from("/usr/bin/xterm")),
            exe,
        )
        .unwrap();
        assert_eq!(terminal, PathBuf::from("/usr/bin/xterm"));
        assert_eq!(
            args,
            ["-e", "sh", "-c", DOCTOR_SCRIPT, "/opt/nomouse"].map(OsString::from)
        );
        let (terminal, args) = terminal_command(|name| Some(PathBuf::from(name)), exe).unwrap();
        assert_eq!(terminal, PathBuf::from("gnome-terminal"));
        assert_eq!(args[0], "--");
    }

    #[test]
    fn find_program_needs_an_executable_file() {
        let dir = std::env::temp_dir().join(format!("nomouse-launch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("plain"), b"").unwrap();
        std::fs::write(dir.join("tool"), b"").unwrap();
        std::fs::set_permissions(dir.join("tool"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([Path::new("/nonexistent"), &dir]).unwrap();
        assert_eq!(find_program("tool", Some(&path)), Some(dir.join("tool")));
        assert_eq!(find_program("plain", Some(&path)), None);
        assert_eq!(find_program("sub", Some(&path)), None);
        assert_eq!(find_program("tool", None), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn truncate_cuts_on_a_character_boundary() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("aé", 2), "a…");
    }

    #[test]
    fn a_daemon_that_exits_at_once_fails_with_its_log() {
        let log = std::env::temp_dir().join(format!("nomouse-launch-{}.log", std::process::id()));
        std::fs::write(&log, b"\nnomouse: no display\n").unwrap();
        let child = Command::new("true").spawn().unwrap();
        assert_eq!(
            wait_for_daemon(child, &log, None),
            Daemon::Failed("nomouse: no display".to_owned())
        );
        std::fs::write(&log, b"").unwrap();
        let child = Command::new("false").spawn().unwrap();
        assert!(matches!(
            wait_for_daemon(child, &log, None),
            Daemon::Failed(reason) if reason.contains("exited")
        ));
        std::fs::remove_file(&log).unwrap();
    }
}
