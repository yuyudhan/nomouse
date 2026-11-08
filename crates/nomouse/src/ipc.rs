//! The daemon's command socket: `nomouse hints|continuous|scroll|search` hand their mode to a
//! running daemon instead of starting a second overlay. On Wayland this is how hotkeys work at
//! all: the compositor binds the command (Sway `bindsym ... exec nomouse hints`).
//!
//! The socket is `$XDG_RUNTIME_DIR/nomouse.sock` (owner-only, like the runtime dir). Without a
//! runtime dir it is `nomouse.sock` inside `<tmp>/nomouse-<uid>/`, a directory created with
//! mode 0700 and refused when it is a symlink, not ours, or open to group or others. A request
//! is one line of at most [`MAX_REQUEST`] bytes (`hints click`, `continuous right-click`,
//! `scroll`, `search`); the daemon answers `ok` before it runs the mode, or `error: <reason>`.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use nomouse_core::ActionKind;

use crate::run::Mode;

/// How long either side waits for the other's line.
const TIMEOUT: Duration = Duration::from_secs(2);
/// The longest request line accepted, newline included.
pub(crate) const MAX_REQUEST: usize = 4096;
/// Connections answered at once; more get `error: busy`, so silent clients cannot pile up threads.
const MAX_CLIENTS: usize = 8;
/// The socket's file name.
const SOCKET: &str = "nomouse.sock";

/// Where the socket lives: `$XDG_RUNTIME_DIR`, else a private per-user directory in the temp
/// dir, created (mode 0700) and checked by [`private_dir`].
///
/// # Errors
/// When the fallback directory cannot be created or is unsafe to use.
pub(crate) fn socket_path(runtime_dir: Option<PathBuf>, uid: u32) -> std::io::Result<PathBuf> {
    if let Some(dir) = runtime_dir.filter(|dir| !dir.as_os_str().is_empty()) {
        return Ok(dir.join(SOCKET));
    }
    let dir = std::env::temp_dir().join(format!("nomouse-{uid}"));
    private_dir(&dir, uid)?;
    Ok(dir.join(SOCKET))
}

/// Creates `dir` with mode 0700, or accepts an existing one only when it is a real directory
/// (not a symlink) owned by `uid` with no group or other permissions.
///
/// # Errors
/// When it cannot be created or read, or fails one of those checks.
pub(crate) fn private_dir(dir: &Path, uid: u32) -> std::io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    let unsafe_dir = |why: String| {
        std::io::Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "refusing the command socket directory {}: {why}; remove it or set XDG_RUNTIME_DIR",
                dir.display()
            ),
        )
    };
    if meta.file_type().is_symlink() {
        return Err(unsafe_dir("it is a symlink".to_owned()));
    }
    if !meta.is_dir() {
        return Err(unsafe_dir("it is not a directory".to_owned()));
    }
    if meta.uid() != uid {
        return Err(unsafe_dir(format!(
            "it is owned by uid {}, not {uid}",
            meta.uid()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(unsafe_dir(format!(
            "its mode {:o} gives group or others access (must be 0700)",
            meta.mode() & 0o777
        )));
    }
    Ok(())
}

/// The socket path for this process's environment.
///
/// # Errors
/// As [`socket_path`].
pub(crate) fn current_path() -> std::io::Result<PathBuf> {
    socket_path(
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        rustix::process::getuid().as_raw(),
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

/// Accepts connections until the daemon's end of the channel is gone. Each connection is
/// answered on its own thread (at most [`MAX_CLIENTS`]), so a client that connects and sends
/// nothing delays nobody else.
fn serve_socket(listener: &UnixListener, sender: &Sender<Mode>) {
    let active = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicBool::new(false));
    for stream in listener.incoming() {
        if closed.load(Ordering::Relaxed) {
            return;
        }
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!("command socket: {error}");
                continue;
            }
        };
        if active.fetch_add(1, Ordering::AcqRel) >= MAX_CLIENTS {
            active.fetch_sub(1, Ordering::AcqRel);
            tracing::warn!("command socket: {MAX_CLIENTS} clients pending, refusing another");
            let mut writer = &stream;
            let _ = writer
                .set_write_timeout(Some(TIMEOUT))
                .and_then(|()| writeln!(writer, "error: busy"));
            continue;
        }
        let (sender, slot, stop) = (sender.clone(), Arc::clone(&active), Arc::clone(&closed));
        let spawned = std::thread::Builder::new()
            .name("nomouse-command".to_owned())
            .spawn(move || {
                if let Some(mode) = answer(&stream)
                    && sender.send(mode).is_err()
                {
                    stop.store(true, Ordering::Relaxed);
                }
                slot.fetch_sub(1, Ordering::AcqRel);
            });
        if let Err(error) = spawned {
            // The closure (and the stream) never ran, so its slot is still taken.
            active.fetch_sub(1, Ordering::AcqRel);
            tracing::warn!("command socket: {error}");
        }
    }
}

/// Reads one request (at most [`MAX_REQUEST`] bytes, within [`TIMEOUT`]) and acknowledges it,
/// or explains why it is invalid.
fn answer(stream: &UnixStream) -> Option<Mode> {
    let setup = stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)));
    if let Err(error) = setup {
        tracing::warn!("command socket: {error}");
        return None;
    }
    let mut line = String::new();
    let limit = u64::try_from(MAX_REQUEST).unwrap_or(u64::MAX);
    if let Err(error) = BufReader::new(stream.take(limit)).read_line(&mut line) {
        tracing::warn!("command socket: {error}");
        return None;
    }
    let mut writer = stream;
    if line.len() >= MAX_REQUEST && !line.ends_with('\n') {
        tracing::warn!("command socket: request longer than {MAX_REQUEST} bytes");
        let _ = writeln!(writer, "error: request too long");
        return None;
    }
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

    fn uid() -> u32 {
        rustix::process::getuid().as_raw()
    }

    /// A fresh scratch path under the temp dir, unique to this test process.
    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("nomouse-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_file(&path);
        path
    }

    /// Sends `bytes` raw and returns the daemon's reply line.
    fn request(path: &Path, bytes: &[u8]) -> String {
        let mut stream = UnixStream::connect(path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        // The daemon may hang up before taking all of an oversized request.
        let _ = stream.write_all(bytes);
        let mut reply = String::new();
        BufReader::new(&stream).read_line(&mut reply).unwrap();
        reply
    }

    #[test]
    fn paths() {
        assert_eq!(
            socket_path(Some(PathBuf::from("/run/user/1000")), 1000).unwrap(),
            PathBuf::from("/run/user/1000/nomouse.sock")
        );
        let fallback = socket_path(None, uid()).unwrap();
        assert_eq!(fallback.file_name().unwrap(), "nomouse.sock");
        let dir = fallback.parent().unwrap();
        assert!(dir.ends_with(format!("nomouse-{}", uid())));
        assert_eq!(std::fs::metadata(dir).unwrap().mode() & 0o777, 0o700);
    }

    #[test]
    fn private_dir_is_created_owner_only_and_socket_is_0600() {
        let dir = scratch("private");
        private_dir(&dir, uid()).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        // Accepted again once it exists.
        private_dir(&dir, uid()).unwrap();
        let path = dir.join(SOCKET);
        let server = Server::bind(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        drop(server);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unsafe_fallback_dirs_are_refused() {
        let open = scratch("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        let error = private_dir(&open, uid()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("group or others"), "{error}");

        // Owned by someone else: checked against another uid, since tests cannot chown.
        let foreign = scratch("foreign");
        private_dir(&foreign, uid()).unwrap();
        let error = private_dir(&foreign, uid().wrapping_add(1)).unwrap_err();
        assert!(error.to_string().contains("owned by uid"), "{error}");

        let link = scratch("link");
        std::os::unix::fs::symlink(&foreign, &link).unwrap();
        let error = private_dir(&link, uid()).unwrap_err();
        assert!(error.to_string().contains("symlink"), "{error}");

        std::fs::remove_file(&link).unwrap();
        std::fs::remove_dir_all(&foreign).unwrap();
        std::fs::remove_dir_all(&open).unwrap();
    }

    #[test]
    fn oversized_request_is_rejected_and_the_daemon_keeps_serving() {
        let path = scratch("long.sock");
        let server = Server::bind(&path).unwrap();
        let long = vec![b'a'; MAX_REQUEST * 4];
        assert_eq!(request(&path, &long), "error: request too long\n");
        assert_eq!(server.poll(), None);
        assert_eq!(request(&path, b"scroll\n"), "ok\n");
        assert_eq!(wait_poll(&server), Some(Mode::Scroll));
    }

    #[test]
    fn a_silent_client_does_not_block_the_next() {
        let path = scratch("silent.sock");
        let server = Server::bind(&path).unwrap();
        let silent = UnixStream::connect(&path).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(request(&path, b"search\n"), "ok\n");
        assert!(
            started.elapsed() < TIMEOUT,
            "the second client waited {:?} behind the silent one",
            started.elapsed()
        );
        assert_eq!(wait_poll(&server), Some(Mode::Search));
        drop(silent);
    }

    /// Polls `server` for up to 2 s.
    fn wait_poll(server: &Server) -> Option<Mode> {
        for _ in 0..200 {
            if let Some(mode) = server.poll() {
                return Some(mode);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
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
