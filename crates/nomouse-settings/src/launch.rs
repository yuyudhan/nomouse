//! Where the `nomouse` binary and the daemon's log are: pure lookups over an explicit
//! environment so they are testable anywhere.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use nomouse_core::Flavour;

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

/// Where `flavour`'s settings window may look for `nomouse` past its own folder: `path` for a
/// release build, nowhere for a development build (an installed `nomouse` is the release's,
/// with the release's configuration and daemon).
fn search_path(flavour: Flavour, path: Option<OsString>) -> Option<OsString> {
    match flavour {
        Flavour::Release => path,
        Flavour::Dev => None,
    }
}

/// [`find_program`] for this process: the `nomouse` binary to run. When there is none, plain
/// `nomouse` for a release build (left to the `PATH` lookup at spawn time, which then reports
/// it missing) and the absent sibling for a development build (reported missing at once).
pub fn nomouse_program() -> PathBuf {
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(Path::parent);
    let path = search_path(Flavour::CURRENT, std::env::var_os("PATH"));
    find_program(NOMOUSE, exe_dir, path.as_deref()).unwrap_or_else(|| match Flavour::CURRENT {
        Flavour::Release => PathBuf::from(NOMOUSE),
        Flavour::Dev => exe_dir.map_or_else(|| PathBuf::from(NOMOUSE), |dir| dir.join(NOMOUSE)),
    })
}

/// The daemon log `nomouse launch` writes too, for `flavour`: `$XDG_RUNTIME_DIR/<stem>.log`,
/// else `<temp dir>/<stem>-$USER.log` (`nomouse` or `nomouse-dev`).
pub fn log_path(flavour: Flavour, env: impl Fn(&str) -> Option<String>) -> PathBuf {
    let get = |name: &str| env(name).filter(|value| !value.is_empty());
    let stem = flavour.stem();
    match get("XDG_RUNTIME_DIR") {
        Some(dir) => Path::new(&dir).join(format!("{stem}.log")),
        None => std::env::temp_dir().join(format!(
            "{stem}-{}.log",
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
            log_path(
                Flavour::Release,
                env(&[("XDG_RUNTIME_DIR", "/run/user/1000"), ("USER", "u")])
            ),
            PathBuf::from("/run/user/1000/nomouse.log")
        );
        assert_eq!(
            log_path(
                Flavour::Dev,
                env(&[("XDG_RUNTIME_DIR", "/run/user/1000"), ("USER", "u")])
            ),
            PathBuf::from("/run/user/1000/nomouse-dev.log")
        );
        assert_eq!(
            log_path(
                Flavour::Release,
                env(&[("XDG_RUNTIME_DIR", ""), ("USER", "u")])
            ),
            std::env::temp_dir().join("nomouse-u.log")
        );
        assert_eq!(
            log_path(Flavour::Dev, env(&[("XDG_RUNTIME_DIR", ""), ("USER", "u")])),
            std::env::temp_dir().join("nomouse-dev-u.log")
        );
        assert_eq!(
            log_path(Flavour::Release, env(&[])),
            std::env::temp_dir().join("nomouse-user.log")
        );
    }

    #[test]
    fn a_development_build_never_runs_the_installed_nomouse() {
        let path = || Some(OsString::from("/usr/bin"));
        assert_eq!(search_path(Flavour::Dev, path()), None);
        assert_eq!(search_path(Flavour::Release, path()), path());
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
