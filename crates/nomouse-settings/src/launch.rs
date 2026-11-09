//! Where the `nomouse` binary and the daemon's log are: pure lookups over an explicit
//! environment so they are testable anywhere.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The command line binary's name.
pub const NOMOUSE: &str = "nomouse";

/// `name` next to this program (`exe_dir`) when it exists there, else the first match on
/// `path` (a `PATH` value).
pub fn find_program(name: &str, exe_dir: Option<&Path>, path: Option<&OsStr>) -> Option<PathBuf> {
    let sibling = exe_dir
        .map(|dir| dir.join(name))
        .filter(|file| file.is_file());
    sibling.or_else(|| {
        std::env::split_paths(path?)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(name))
            .find(|file| file.is_file())
    })
}

/// [`find_program`] for this process: the `nomouse` binary to run, or plain `nomouse` (left to
/// the `PATH` lookup at spawn time, which then reports it missing).
pub fn nomouse_program() -> PathBuf {
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(Path::parent);
    find_program(NOMOUSE, exe_dir, std::env::var_os("PATH").as_deref())
        .unwrap_or_else(|| PathBuf::from(NOMOUSE))
}

/// The daemon log `nomouse launch` writes too: `$XDG_RUNTIME_DIR/nomouse.log`, else
/// `<temp dir>/nomouse-$USER.log`.
pub fn log_path(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    let get = |name: &str| env(name).filter(|value| !value.is_empty());
    match get("XDG_RUNTIME_DIR") {
        Some(dir) => Path::new(&dir).join("nomouse.log"),
        None => std::env::temp_dir().join(format!(
            "nomouse-{}.log",
            get("USER").unwrap_or_else(|| "user".to_owned())
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_path_follows_xdg_runtime_dir() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        assert_eq!(
            log_path(env(&[("XDG_RUNTIME_DIR", "/run/user/1000"), ("USER", "u")])),
            PathBuf::from("/run/user/1000/nomouse.log")
        );
        assert_eq!(
            log_path(env(&[("XDG_RUNTIME_DIR", ""), ("USER", "u")])),
            std::env::temp_dir().join("nomouse-u.log")
        );
        assert_eq!(
            log_path(env(&[])),
            std::env::temp_dir().join("nomouse-user.log")
        );
    }

    #[test]
    fn prefers_the_sibling_binary_then_path() {
        let root =
            std::env::temp_dir().join(format!("nomouse-settings-launch-{}", std::process::id()));
        let here = root.join("here");
        let bin = root.join("bin");
        std::fs::create_dir_all(&here).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(NOMOUSE), "").unwrap();
        let path = std::env::join_paths([Path::new("relative"), &bin]).unwrap();
        assert_eq!(
            find_program(NOMOUSE, Some(&here), Some(&path)),
            Some(bin.join(NOMOUSE))
        );
        std::fs::write(here.join(NOMOUSE), "").unwrap();
        assert_eq!(
            find_program(NOMOUSE, Some(&here), Some(&path)),
            Some(here.join(NOMOUSE))
        );
        assert_eq!(find_program("absent", Some(&here), Some(&path)), None);
        assert_eq!(find_program("absent", None, None), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
