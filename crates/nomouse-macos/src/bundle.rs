//! Running as the app when started through a symlink to it.
//!
//! `install.sh` links `~/.local/bin/nomouse` to `nomouse.app/Contents/MacOS/nomouse`. macOS
//! decides which bundle a process belongs to from the path it was started by, not from where
//! that path leads, so a daemon started as `nomouse` has no bundle: no bundle identifier, and
//! with it no `SMAppService` (Start at login) and no app identity. Starting the daemon again
//! from the real path in the bundle gives it both.

use std::path::Path;

/// Whether `real` is the executable of an app bundle: `Name.app/Contents/MacOS/<executable>`.
#[must_use]
pub fn is_bundle_executable(real: &Path) -> bool {
    let Some(macos) = real.parent() else {
        return false;
    };
    let Some(contents) = macos.parent() else {
        return false;
    };
    macos.file_name().is_some_and(|name| name == "MacOS")
        && contents.file_name().is_some_and(|name| name == "Contents")
        && contents
            .parent()
            .and_then(Path::extension)
            .is_some_and(|extension| extension == "app")
}

/// The app bundle (`Name.app`) whose executable `real` is, if it is one.
#[must_use]
pub fn app_of(real: &Path) -> Option<&Path> {
    if !is_bundle_executable(real) {
        return None;
    }
    real.parent()?.parent()?.parent()
}

/// Replaces this process with the same command started from the real path of its executable,
/// when it was started through a symlink to an app bundle's executable. Returns when there is
/// nothing to do (started by its real path, or not from a bundle) or when starting it again
/// failed, which is logged: the daemon then runs without a bundle, as before.
///
/// The `exec` happens before this process has touched AppKit, so the menu bar has never seen
/// its old pid version and takes the new image's status item. That is why the restart after
/// the first run's grant, which comes after its Welcome item was up, starts a new process
/// instead (`relaunch` in the binary).
#[cfg(target_os = "macos")]
pub fn reexec_from_bundle() {
    use std::os::unix::process::CommandExt as _;

    let Ok(shown) = std::env::current_exe() else {
        return;
    };
    let Ok(real) = shown.canonicalize() else {
        return;
    };
    if shown == real || !is_bundle_executable(&real) {
        return;
    }
    tracing::debug!(
        "{} is {}: starting it again from there",
        shown.display(),
        real.display()
    );
    // `exec` only returns when it failed.
    let error = std::process::Command::new(&real)
        .args(std::env::args_os().skip(1))
        .exec();
    tracing::warn!(
        "cannot start {} as the app: {error}; Start at login is not available",
        real.display()
    );
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{app_of, is_bundle_executable};

    #[test]
    fn the_executable_of_an_app_bundle_is_recognised() {
        assert!(is_bundle_executable(Path::new(
            "/Applications/nomouse.app/Contents/MacOS/nomouse"
        )));
        assert!(is_bundle_executable(Path::new(
            "/Users/a/Applications/nomouse.app/Contents/MacOS/other"
        )));
    }

    #[test]
    fn the_app_of_a_bundle_executable_is_the_bundle() {
        assert_eq!(
            app_of(Path::new(
                "/Applications/nomouse Dev.app/Contents/MacOS/nomouse"
            )),
            Some(Path::new("/Applications/nomouse Dev.app"))
        );
        assert_eq!(app_of(Path::new("/usr/local/bin/nomouse")), None);
    }

    #[test]
    fn anything_else_is_not() {
        for path in [
            "/usr/local/bin/nomouse",
            "/Applications/nomouse.app/Contents/Resources/nomouse",
            "/Applications/nomouse.app/Contents/MacOS",
            "/Applications/nomouse/Contents/MacOS/nomouse",
            "/Contents/MacOS/nomouse",
            "nomouse",
            "",
        ] {
            assert!(!is_bundle_executable(Path::new(path)), "{path:?}");
        }
    }
}
