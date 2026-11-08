//! The daemon's command socket: `nomouse hints|continuous|scroll|search` hand their mode to a
//! running daemon instead of starting a second overlay. On Wayland this is how hotkeys work at
//! all: the compositor binds the command (Sway `bindsym ... exec nomouse hints`).
//!
//! The socket is `$XDG_RUNTIME_DIR/nomouse.sock` (owner-only, like the runtime dir). A request
//! is one line (`hints click`, `continuous right-click`, `scroll`, `search`); the daemon
//! answers `ok` before it runs the mode, or `error: <reason>`.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use nomouse_core::ActionKind;

use crate::run::Mode;

/// How long either side waits for the other's line.
const TIMEOUT: Duration = Duration::from_secs(2);

/// Where the socket lives: `$XDG_RUNTIME_DIR`, else the temp dir with the user name.
pub(crate) fn socket_path(runtime_dir: Option<PathBuf>, user: Option<String>) -> PathBuf {
    match runtime_dir.filter(|dir| !dir.as_os_str().is_empty()) {
        Some(dir) => dir.join("nomouse.sock"),
        None => std::env::temp_dir().join(format!(
            "nomouse-{}.sock",
            user.unwrap_or_else(|| "user".to_owned())
        )),
    }
}

/// The socket path for this process's environment.
pub(crate) fn current_path() -> PathBuf {
    socket_path(
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var("USER").ok(),
    )
}

/// The request line for `mode`.
pub(crate) fn encode(mode: Mode) -> String {
    match mode {
        Mode::Once(kind) => format!("hints {kind}"),
        Mode::Continuous(kind) => format!("continuous {kind}"),
        Mode::Scroll => "scroll".to_owned(),
        Mode::Search => "search".to_owned(),
    }
}

/// The mode of a request line.
///
/// # Errors
/// The reason, for the `error:` reply.
pub(crate) fn decode(line: &str) -> Result<Mode, String> {
    let mut words = line.split_whitespace();
    let command = words.next().unwrap_or_default();
    let kind = |word: Option<&str>| -> Result<ActionKind, String> {
        word.map_or(Ok(ActionKind::Click), |w| {
            w.parse::<ActionKind>().map_err(|e| e.to_string())
        })
    };
    let mode = match command {
        "hints" => match kind(words.next())? {
            ActionKind::Scroll => Mode::Scroll,
            kind => Mode::Once(kind),
        },
        "continuous" => Mode::Continuous(kind(words.next())?),
        "scroll" => Mode::Scroll,
        "search" => Mode::Search,
        other => return Err(format!("unknown request `{other}`")),
    };
    if words.next().is_some() {
        return Err(format!("trailing words in `{line}`"));
    }
    Ok(mode)
}

/// Why a request could not be delivered.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SendError {
    /// No daemon listens on the socket.
    #[error("no daemon is listening on {0}")]
    NoDaemon(PathBuf),
    /// The daemon refused or the exchange failed.
    #[error("daemon at {path}: {reason}")]
    Failed {
        /// The socket.
        path: PathBuf,
        /// What went wrong.
        reason: String,
    },
}

/// Asks the daemon at `path` to run `mode`.
///
/// # Errors
/// [`SendError::NoDaemon`] when nothing listens there (the caller then runs the mode itself).
pub(crate) fn send(path: &Path, mode: Mode) -> Result<(), SendError> {
    let failed = |reason: String| SendError::Failed {
        path: path.to_owned(),
        reason,
    };
    let mut stream = match UnixStream::connect(path) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(SendError::NoDaemon(path.to_owned()));
        }
        Err(error) => return Err(failed(error.to_string())),
    };
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|e| failed(e.to_string()))?;
    writeln!(stream, "{}", encode(mode)).map_err(|e| failed(e.to_string()))?;
    let mut reply = String::new();
    BufReader::new(&stream)
        .read_line(&mut reply)
        .map_err(|e| failed(e.to_string()))?;
    match reply.trim_end() {
        "ok" => Ok(()),
        other => Err(failed(
            other.strip_prefix("error: ").unwrap_or(other).to_owned(),
        )),
    }
}

/// The daemon's listening socket, served by a thread that acknowledges each request at once
/// (so a bound key never waits on a running session); removed when dropped.
#[derive(Debug)]
pub(crate) struct Server {
    requests: Receiver<Mode>,
    path: PathBuf,
}

impl Server {
    /// Listens on `path`. Call only while holding the single-instance lock: a leftover socket
    /// from a crashed daemon is replaced.
    ///
    /// # Errors
    /// When the socket cannot be bound.
    pub(crate) fn bind(path: &Path) -> std::io::Result<Self> {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let (sender, requests) = mpsc::channel();
        std::thread::Builder::new()
            .name("nomouse-commands".to_owned())
            .spawn(move || serve_socket(&listener, &sender))?;
        Ok(Self {
            requests,
            path: path.to_owned(),
        })
    }

    /// The oldest request not yet taken.
    pub(crate) fn poll(&self) -> Option<Mode> {
        self.requests.try_recv().ok()
    }

    /// Drops the requests that arrived while a session ran (a second press of the bound key
    /// during a session must not start another one afterwards); returns how many.
    pub(crate) fn discard_pending(&self) -> usize {
        self.requests.try_iter().count()
    }
}

/// Accepts connections until the daemon's end of the channel is gone.
fn serve_socket(listener: &UnixListener, sender: &Sender<Mode>) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Some(mode) = answer(&stream)
                    && sender.send(mode).is_err()
                {
                    return;
                }
            }
            Err(error) => tracing::warn!("command socket: {error}"),
        }
    }
}

/// Reads one request and acknowledges it (or explains why it is invalid).
fn answer(stream: &UnixStream) -> Option<Mode> {
    let setup = stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)));
    if let Err(error) = setup {
        tracing::warn!("command socket: {error}");
        return None;
    }
    let mut line = String::new();
    if let Err(error) = BufReader::new(stream).read_line(&mut line) {
        tracing::warn!("command socket: {error}");
        return None;
    }
    let mut writer = stream;
    match decode(line.trim()) {
        Ok(mode) => {
            // The client only waits for the acknowledgement, not for the mode to finish.
            if let Err(error) = writeln!(writer, "ok") {
                tracing::warn!("command socket: {error}");
            }
            Some(mode)
        }
        Err(reason) => {
            tracing::warn!("command socket: {reason}");
            // The client gets the reason when it is still listening.
            let _ = writeln!(writer, "error: {reason}");
            None
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Best effort: a leftover socket is replaced by the next daemon anyway.
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(
            socket_path(Some(PathBuf::from("/run/user/1000")), None),
            PathBuf::from("/run/user/1000/nomouse.sock")
        );
        assert!(socket_path(None, Some("ann".into())).ends_with("nomouse-ann.sock"));
    }

    #[test]
    fn requests_round_trip() {
        for mode in [
            Mode::Once(ActionKind::Click),
            Mode::Once(ActionKind::RightClick),
            Mode::Continuous(ActionKind::DoubleClick),
            Mode::Scroll,
            Mode::Search,
        ] {
            assert_eq!(decode(&encode(mode)), Ok(mode), "{}", encode(mode));
        }
        assert_eq!(decode("hints"), Ok(Mode::Once(ActionKind::Click)));
        assert_eq!(decode("hints scroll"), Ok(Mode::Scroll));
        assert!(decode("explode").is_err());
        assert!(decode("hints nonsense").is_err());
        assert!(decode("scroll now").is_err());
    }

    #[test]
    fn daemon_and_client_talk() {
        let path = std::env::temp_dir().join(format!("nomouse-ipc-{}.sock", std::process::id()));
        assert!(matches!(
            send(&path, Mode::Scroll),
            Err(SendError::NoDaemon(_))
        ));
        let server = Server::bind(&path).unwrap();
        assert_eq!(server.poll(), None);
        let client = std::thread::spawn({
            let path = path.clone();
            move || send(&path, Mode::Continuous(ActionKind::Hover))
        });
        let mut received = None;
        for _ in 0..200 {
            received = server.poll();
            if received.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(received, Some(Mode::Continuous(ActionKind::Hover)));
        assert!(client.join().unwrap().is_ok());
        drop(server);
        assert!(!path.exists(), "the socket is removed with the server");
    }

    #[test]
    fn invalid_requests_get_an_error_reply() {
        let path =
            std::env::temp_dir().join(format!("nomouse-ipc-bad-{}.sock", std::process::id()));
        let server = Server::bind(&path).unwrap();
        let client = std::thread::spawn({
            let path = path.clone();
            move || {
                let mut stream = UnixStream::connect(&path).unwrap();
                writeln!(stream, "explode").unwrap();
                let mut reply = String::new();
                BufReader::new(&stream).read_line(&mut reply).unwrap();
                reply
            }
        });
        let mut reply = None;
        for _ in 0..200 {
            assert_eq!(server.poll(), None);
            if client.is_finished() {
                reply = Some(client.join().unwrap());
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(reply.as_deref(), Some("error: unknown request `explode`\n"));
    }
}
