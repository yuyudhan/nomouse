//! The daemon's command socket: `nomouse hints|continuous|scroll|search` hand their mode to a
//! running daemon instead of starting a second overlay, and `nomouse reload` makes it re-read
//! its configuration. On Wayland this is how hotkeys work at all: the compositor binds the
//! command (Sway `bindsym ... exec nomouse hints`).
//!
//! The socket is `$XDG_RUNTIME_DIR/nomouse.sock` (owner-only, like the runtime dir). Without a
//! runtime dir it is `nomouse.sock` inside `<tmp>/nomouse-<uid>/`, a directory created with
//! mode 0700 and refused when it is a symlink, not ours, or open to group or others. A
//! development build uses `nomouse-dev.sock` and `<tmp>/nomouse-dev-<uid>/` instead
//! ([`nomouse_core::Flavour`]), so its commands never reach the installed release. A request
//! is one line of at most [`MAX_REQUEST`] bytes (`hints click`, `continuous right-click`,
//! `scroll`, `search`, `reload`); the daemon answers `ok` before it acts, or
//! `error: <reason>`. `ping` is answered `ok` and does nothing: `nomouse launch` uses it to see
//! that a daemon it started is up. `version` is answered `ok <pid> <version>` and does nothing:
//! `nomouse doctor` uses it to tell a daemon left running from before an update.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use nomouse_core::{ActionKind, Flavour};

use crate::Request;
use crate::run::Mode;

/// How long either side waits for the other's line.
const TIMEOUT: Duration = Duration::from_secs(2);
/// The longest request line accepted, newline included.
pub(crate) const MAX_REQUEST: usize = 4096;
/// Connections answered at once; more get `error: busy`, so silent clients cannot pile up threads.
const MAX_CLIENTS: usize = 8;
/// The request that only asks whether the daemon is up.
const PING: &str = "ping";
/// The request that asks the daemon who it is; answered `ok <pid> <version>`.
const VERSION: &str = "version";

/// Where `flavour`'s socket lives: `$XDG_RUNTIME_DIR/<stem>.sock`, else `<stem>.sock` in a
/// private per-user directory `<stem>-<uid>` in the temp dir, created (mode 0700) and checked
/// by [`private_dir`].
///
/// # Errors
/// When the fallback directory cannot be created or is unsafe to use.
pub(crate) fn socket_path(
    flavour: Flavour,
    runtime_dir: Option<PathBuf>,
    uid: u32,
) -> std::io::Result<PathBuf> {
    let stem = flavour.stem();
    let socket = format!("{stem}.sock");
    if let Some(dir) = runtime_dir.filter(|dir| !dir.as_os_str().is_empty()) {
        return Ok(dir.join(socket));
    }
    let dir = std::env::temp_dir().join(format!("{stem}-{uid}"));
    private_dir(&dir, uid)?;
    Ok(dir.join(socket))
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

/// The socket path for this build's flavour and this process's environment.
///
/// # Errors
/// As [`socket_path`].
pub(crate) fn current_path() -> std::io::Result<PathBuf> {
    socket_path(
        Flavour::CURRENT,
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        rustix::process::getuid().as_raw(),
    )
}

/// The request line for `request`.
pub(crate) fn encode(request: Request) -> String {
    match request {
        Request::Run(Mode::Once(kind)) => format!("hints {kind}"),
        Request::Run(Mode::Continuous(kind)) => format!("continuous {kind}"),
        Request::Run(Mode::Scroll) => "scroll".to_owned(),
        Request::Run(Mode::Search) => "search".to_owned(),
        Request::Reload => "reload".to_owned(),
        Request::Pause => "pause".to_owned(),
        Request::Resume => "resume".to_owned(),
        Request::Quit => "quit".to_owned(),
    }
}

/// The request of a request line.
///
/// # Errors
/// The reason, for the `error:` reply.
pub(crate) fn decode(line: &str) -> Result<Request, String> {
    let mut words = line.split_whitespace();
    let command = words.next().unwrap_or_default();
    let kind = |word: Option<&str>| -> Result<ActionKind, String> {
        word.map_or(Ok(ActionKind::Click), |w| {
            w.parse::<ActionKind>().map_err(|e| e.to_string())
        })
    };
    let request = match command {
        "hints" => Request::Run(match kind(words.next())? {
            ActionKind::Scroll => Mode::Scroll,
            kind => Mode::Once(kind),
        }),
        "continuous" => Request::Run(Mode::Continuous(kind(words.next())?)),
        "scroll" => Request::Run(Mode::Scroll),
        "search" => Request::Run(Mode::Search),
        "reload" => Request::Reload,
        "pause" => Request::Pause,
        "resume" => Request::Resume,
        "quit" => Request::Quit,
        other => return Err(format!("unknown request `{other}`")),
    };
    if words.next().is_some() {
        return Err(format!("trailing words in `{line}`"));
    }
    Ok(request)
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

/// Sends `request` to the daemon at `path`.
///
/// # Errors
/// [`SendError::NoDaemon`] when nothing listens there (the caller then runs the mode itself).
pub(crate) fn send(path: &Path, request: Request) -> Result<(), SendError> {
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
    writeln!(stream, "{}", encode(request)).map_err(|e| failed(e.to_string()))?;
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

/// Whether a daemon answers `ping` on the socket at `path`.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn ping(path: &Path) -> bool {
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    if stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .and_then(|()| writeln!(stream, "{PING}"))
        .is_err()
    {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).is_ok() && reply.trim_end() == "ok"
}

/// What the daemon on a socket says about itself, for `nomouse doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DaemonInfo {
    /// Nothing listens on the socket.
    NotRunning,
    /// A daemon answered `version`.
    Running {
        /// Its process id.
        pid: u32,
        /// Its version (`CARGO_PKG_VERSION`).
        version: String,
    },
    /// Something listens but gave no version: a daemon from before the `version` request
    /// (it answers `error: unknown request`), or one that does not answer at all.
    Unversioned(String),
}

/// The `pid version` of a `version` reply line (`ok 4242 0.1.8`).
fn parse_version_reply(reply: &str) -> Option<(u32, String)> {
    let mut words = reply.trim_end().strip_prefix("ok ")?.split_whitespace();
    let pid = words.next()?.parse().ok()?;
    let version = words.next()?.to_owned();
    words.next().is_none().then_some((pid, version))
}

/// Asks the daemon at `path` for its process id and version.
pub(crate) fn daemon_info(path: &Path) -> DaemonInfo {
    let mut stream = match UnixStream::connect(path) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) =>
        {
            return DaemonInfo::NotRunning;
        }
        Err(error) => return DaemonInfo::Unversioned(format!("{}: {error}", path.display())),
    };
    let mut reply = String::new();
    let exchanged = stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .and_then(|()| writeln!(stream, "{VERSION}"))
        .and_then(|()| BufReader::new(&stream).read_line(&mut reply));
    match exchanged {
        Ok(_) => parse_version_reply(&reply).map_or_else(
            || {
                DaemonInfo::Unversioned(format!(
                    "it answered {:?} instead of its version",
                    reply.trim_end()
                ))
            },
            |(pid, version)| DaemonInfo::Running { pid, version },
        ),
        Err(error) => DaemonInfo::Unversioned(format!("it did not answer: {error}")),
    }
}

/// The daemon's listening socket, served by a thread that acknowledges each request at once
/// (so a bound key never waits on a running session); removed when dropped.
#[derive(Debug)]
pub(crate) struct Server {
    requests: Receiver<Request>,
    path: PathBuf,
}

impl Server {
    /// Listens on `path`. Call only while holding the single-instance lock: a leftover socket
    /// from a crashed daemon, or a symlink (the link only, never its target), is replaced;
    /// anything else at `path` is refused and left in place.
    ///
    /// # Errors
    /// When something other than a socket or symlink is at `path`, or binding fails.
    pub(crate) fn bind(path: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::FileTypeExt;
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_socket() || meta.file_type().is_symlink() => {
                std::fs::remove_file(path)?;
            }
            Ok(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::AlreadyExists,
                    format!(
                        "{} exists and is not a socket; refusing to replace it (remove it \
                         yourself if it is stale)",
                        path.display()
                    ),
                ));
            }
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
    pub(crate) fn poll(&self) -> Option<Request> {
        self.requests.try_recv().ok()
    }

    /// Takes the requests that arrived while a session ran: activations are dropped (a second
    /// press of the bound key during a session must not start another one afterwards).
    /// Returns how many were dropped, and the other requests (reload, pause, resume, quit) in
    /// order, for the daemon to carry out now.
    pub(crate) fn discard_pending(&self) -> (usize, Vec<Request>) {
        let mut dropped = 0;
        let mut kept = Vec::new();
        for request in self.requests.try_iter() {
            match request {
                Request::Run(_) => dropped += 1,
                other => kept.push(other),
            }
        }
        (dropped, kept)
    }
}

/// Accepts connections until the daemon's end of the channel is gone. Each connection is
/// answered on its own thread (at most [`MAX_CLIENTS`]), so a client that connects and sends
/// nothing delays nobody else.
fn serve_socket(listener: &UnixListener, sender: &Sender<Request>) {
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
                if let Some(request) = answer(&stream)
                    && sender.send(request).is_err()
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
fn answer(stream: &UnixStream) -> Option<Request> {
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
    if line.trim() == PING {
        let _ = writeln!(writer, "ok");
        return None;
    }
    if line.trim() == VERSION {
        let _ = writeln!(
            writer,
            "ok {} {}",
            std::process::id(),
            env!("CARGO_PKG_VERSION")
        );
        return None;
    }
    match decode(line.trim()) {
        Ok(request) => {
            // The client only waits for the acknowledgement, not for the request to finish.
            if let Err(error) = writeln!(writer, "ok") {
                tracing::warn!("command socket: {error}");
            }
            Some(request)
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
        let run = || Some(PathBuf::from("/run/user/1000"));
        assert_eq!(
            socket_path(Flavour::Release, run(), 1000).unwrap(),
            PathBuf::from("/run/user/1000/nomouse.sock")
        );
        assert_eq!(
            socket_path(Flavour::Dev, run(), 1000).unwrap(),
            PathBuf::from("/run/user/1000/nomouse-dev.sock")
        );
        for (flavour, name) in [(Flavour::Release, "nomouse"), (Flavour::Dev, "nomouse-dev")] {
            let fallback = socket_path(flavour, None, uid()).unwrap();
            assert_eq!(
                fallback.file_name().unwrap(),
                format!("{name}.sock").as_str()
            );
            let dir = fallback.parent().unwrap();
            assert!(dir.ends_with(format!("{name}-{}", uid())));
            assert_eq!(std::fs::metadata(dir).unwrap().mode() & 0o777, 0o700);
        }
    }

    #[test]
    fn private_dir_is_created_owner_only_and_socket_is_0600() {
        let dir = scratch("private");
        private_dir(&dir, uid()).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        // Accepted again once it exists.
        private_dir(&dir, uid()).unwrap();
        let path = dir.join("nomouse.sock");
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
        assert_eq!(wait_poll(&server), Some(Request::Run(Mode::Scroll)));
    }

    #[test]
    fn bind_refuses_a_regular_file_and_leaves_it() {
        let path = scratch("file.sock");
        std::fs::write(&path, b"keep me").unwrap();
        let error = Server::bind(&path).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("not a socket"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn bind_replaces_a_stale_socket() {
        let path = scratch("stale.sock");
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists(), "the stale socket file stays behind");
        let server = Server::bind(&path).unwrap();
        assert_eq!(request(&path, b"scroll\n"), "ok\n");
        assert_eq!(wait_poll(&server), Some(Request::Run(Mode::Scroll)));
    }

    #[test]
    fn ping_is_answered_and_queues_nothing() {
        let path = scratch("ping.sock");
        assert!(!ping(&path), "nothing listens yet");
        let server = Server::bind(&path).unwrap();
        assert!(ping(&path));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(server.poll(), None);
    }

    #[test]
    fn version_is_answered_with_pid_and_version_and_queues_nothing() {
        let path = scratch("version.sock");
        assert_eq!(daemon_info(&path), DaemonInfo::NotRunning);
        let server = Server::bind(&path).unwrap();
        assert_eq!(
            daemon_info(&path),
            DaemonInfo::Running {
                pid: std::process::id(),
                version: env!("CARGO_PKG_VERSION").to_owned()
            }
        );
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(server.poll(), None);
    }

    #[test]
    fn a_daemon_without_the_version_request_is_unversioned() {
        // What nomouse 0.1.7 and older answer: `version` is an unknown request there.
        let path = scratch("old.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let old = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            writeln!(&stream, "error: unknown request `{}`", line.trim()).unwrap();
        });
        assert_eq!(
            daemon_info(&path),
            DaemonInfo::Unversioned(
                "it answered \"error: unknown request `version`\" instead of its version"
                    .to_owned()
            )
        );
        old.join().unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            parse_version_reply("ok 42 0.1.8\n"),
            Some((42, "0.1.8".to_owned()))
        );
        assert_eq!(parse_version_reply("ok"), None);
        assert_eq!(parse_version_reply("ok x 0.1.8"), None);
        assert_eq!(parse_version_reply("ok 42 0.1.8 extra"), None);
    }

    #[test]
    fn bind_removes_a_symlink_but_not_its_target() {
        let target = scratch("target");
        std::fs::write(&target, b"target").unwrap();
        let path = scratch("link.sock");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let server = Server::bind(&path).unwrap();
        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert!(std::os::unix::fs::FileTypeExt::is_socket(&meta.file_type()));
        assert_eq!(std::fs::read(&target).unwrap(), b"target");
        drop(server);
        std::fs::remove_file(&target).unwrap();
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
        assert_eq!(wait_poll(&server), Some(Request::Run(Mode::Search)));
        drop(silent);
    }

    /// Polls `server` for up to 2 s.
    fn wait_poll(server: &Server) -> Option<Request> {
        for _ in 0..200 {
            if let Some(request) = server.poll() {
                return Some(request);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn requests_round_trip() {
        for request in [
            Request::Run(Mode::Once(ActionKind::Click)),
            Request::Run(Mode::Once(ActionKind::RightClick)),
            Request::Run(Mode::Continuous(ActionKind::DoubleClick)),
            Request::Run(Mode::Scroll),
            Request::Run(Mode::Search),
            Request::Reload,
            Request::Pause,
            Request::Resume,
            Request::Quit,
        ] {
            assert_eq!(decode(&encode(request)), Ok(request), "{}", encode(request));
        }
        assert_eq!(
            decode("hints"),
            Ok(Request::Run(Mode::Once(ActionKind::Click)))
        );
        assert_eq!(decode("hints scroll"), Ok(Request::Run(Mode::Scroll)));
        assert!(decode("explode").is_err());
        assert!(decode("hints nonsense").is_err());
        assert!(decode("scroll now").is_err());
        assert!(decode("reload now").is_err());
        assert!(decode("pause now").is_err());
    }

    #[test]
    fn pending_activations_are_dropped_but_other_requests_are_kept_in_order() {
        let path = scratch("pending.sock");
        let server = Server::bind(&path).unwrap();
        for line in [&b"hints\n"[..], b"reload\n", b"search\n", b"pause\n"] {
            assert_eq!(request(&path, line), "ok\n");
        }
        // A request is queued just after its reply: collect for up to 2 s.
        let (mut dropped, mut kept) = (0, Vec::new());
        for _ in 0..200 {
            let (more, others) = server.discard_pending();
            dropped += more;
            kept.extend(others);
            if dropped == 2 && kept.len() == 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!((dropped, kept), (2, vec![Request::Reload, Request::Pause]));
        assert_eq!(server.discard_pending(), (0, Vec::new()));
    }

    #[test]
    fn daemon_and_client_talk() {
        let path = std::env::temp_dir().join(format!("nomouse-ipc-{}.sock", std::process::id()));
        assert!(matches!(
            send(&path, Request::Run(Mode::Scroll)),
            Err(SendError::NoDaemon(_))
        ));
        let server = Server::bind(&path).unwrap();
        assert_eq!(server.poll(), None);
        let client = std::thread::spawn({
            let path = path.clone();
            move || send(&path, Request::Run(Mode::Continuous(ActionKind::Hover)))
        });
        let mut received = None;
        for _ in 0..200 {
            received = server.poll();
            if received.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            received,
            Some(Request::Run(Mode::Continuous(ActionKind::Hover)))
        );
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
