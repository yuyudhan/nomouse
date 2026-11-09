//! One daemon per user: an exclusive lock on `$XDG_RUNTIME_DIR/nomouse.lock`.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Where the lock lives: `$XDG_RUNTIME_DIR`, else the temp dir with the user name.
pub(crate) fn lock_path(runtime_dir: Option<PathBuf>, user: Option<String>) -> PathBuf {
    match runtime_dir.filter(|dir| !dir.as_os_str().is_empty()) {
        Some(dir) => dir.join("nomouse.lock"),
        None => std::env::temp_dir().join(format!(
            "nomouse-{}.lock",
            user.unwrap_or_else(|| "user".to_owned())
        )),
    }
}

/// [`lock_path`] for this process's `XDG_RUNTIME_DIR` and `USER`.
pub(crate) fn current_path() -> PathBuf {
    lock_path(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(
            lock_path(Some(PathBuf::from("/run/user/1000")), None),
            PathBuf::from("/run/user/1000/nomouse.lock")
        );
        let fallback = lock_path(Some(PathBuf::new()), Some("ann".into()));
        assert!(fallback.ends_with("nomouse-ann.lock"));
    }

    #[test]
    fn second_lock_is_refused() {
        let path = std::env::temp_dir().join(format!("nomouse-test-{}.lock", std::process::id()));
        let first = acquire(&path).unwrap();
        assert!(matches!(acquire(&path), Err(LockError::Taken(_))));
        drop(first);
        assert!(acquire(&path).is_ok());
        std::fs::remove_file(&path).unwrap();
    }
}
