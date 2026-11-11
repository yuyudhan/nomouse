//! One daemon per user and flavour: an exclusive lock on `$XDG_RUNTIME_DIR/nomouse.lock`
//! (`nomouse-dev.lock` for a development build, so it runs beside the installed release).

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

use nomouse_core::Flavour;

/// Where `flavour`'s lock lives: `$XDG_RUNTIME_DIR/<stem>.lock`, else
/// `<stem>-<user>.lock` in the temp dir.
pub(crate) fn lock_path(
    flavour: Flavour,
    runtime_dir: Option<PathBuf>,
    user: Option<String>,
) -> PathBuf {
    let stem = flavour.stem();
    match runtime_dir.filter(|dir| !dir.as_os_str().is_empty()) {
        Some(dir) => dir.join(format!("{stem}.lock")),
        None => std::env::temp_dir().join(format!(
            "{stem}-{}.lock",
            user.unwrap_or_else(|| "user".to_owned())
        )),
    }
}

/// [`lock_path`] for this build's flavour and this process's `XDG_RUNTIME_DIR` and `USER`.
pub(crate) fn current_path() -> PathBuf {
    lock_path(
        Flavour::CURRENT,
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var("USER").ok(),
    )
}

/// Holds the lock until dropped (the OS releases it when the process exits).
#[derive(Debug)]
pub(crate) struct InstanceLock {
    _file: File,
}

/// Why the lock was not taken.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LockError {
    /// Another daemon holds it.
    #[error("another nomouse daemon is already running (lock {0})")]
    Taken(PathBuf),
    /// The file could not be opened or locked.
    #[error("cannot lock {path}: {source}")]
    Io {
        /// The lock file.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
}

/// Takes the exclusive lock at `path` and records this process id in it.
///
/// # Errors
/// [`LockError::Taken`] when another process holds it.
pub(crate) fn acquire(path: &Path) -> Result<InstanceLock, LockError> {
    let io = |source| LockError::Io {
        path: path.to_owned(),
        source,
    };
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(io)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(LockError::Taken(path.to_owned())),
        Err(TryLockError::Error(error)) => return Err(io(error)),
    }
    file.set_len(0).map_err(io)?;
    writeln!(file, "{}", std::process::id()).map_err(io)?;
    Ok(InstanceLock { _file: file })
}

/// Serialises the tests that start a child process and the tests that release a lock and
/// expect it free: a child forked while a lock file is open shares that file's lock until it
/// execs, so a release in that window would not be seen. Every such test, in any module,
/// holds this guard for its whole body.
#[cfg(test)]
pub(crate) fn children_guard() -> std::sync::MutexGuard<'static, ()> {
    static CHILDREN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    CHILDREN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let run = || Some(PathBuf::from("/run/user/1000"));
        assert_eq!(
            lock_path(Flavour::Release, run(), None),
            PathBuf::from("/run/user/1000/nomouse.lock")
        );
        assert_eq!(
            lock_path(Flavour::Dev, run(), None),
            PathBuf::from("/run/user/1000/nomouse-dev.lock")
        );
        let fallback = lock_path(Flavour::Release, Some(PathBuf::new()), Some("ann".into()));
        assert!(fallback.ends_with("nomouse-ann.lock"));
        let fallback = lock_path(Flavour::Dev, None, Some("ann".into()));
        assert!(fallback.ends_with("nomouse-dev-ann.lock"));
        assert!(
            current_path()
                .to_string_lossy()
                .contains(Flavour::CURRENT.stem())
        );
    }

    #[test]
    fn second_lock_is_refused() {
        let _alone = children_guard();
        let path = std::env::temp_dir().join(format!("nomouse-test-{}.lock", std::process::id()));
        let first = acquire(&path).unwrap();
        assert!(matches!(acquire(&path), Err(LockError::Taken(_))));
        drop(first);
        assert!(acquire(&path).is_ok());
        std::fs::remove_file(&path).unwrap();
    }
}
