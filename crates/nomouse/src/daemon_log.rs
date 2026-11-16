//! The daemon's log file: what the daemon logs (timings and events at `info` by default, never
//! typed keys) goes to a size-capped file that is rotated, so the next problem can be read
//! after the fact. macOS: `~/Library/Logs/<stem>/<stem>.log`; Linux:
//! `$XDG_STATE_HOME/<stem>/<stem>.log` (else `~/.local/state/...`); Windows:
//! `%LOCALAPPDATA%\<stem>\logs\<stem>.log`. The stem is `nomouse`, or `nomouse-dev` for a
//! development build.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use nomouse_core::Flavour;

/// The size at which the log file is rotated.
pub(crate) const MAX_BYTES: u64 = 1024 * 1024;
/// How many rotated files (`<stem>.log.1` newest to `<stem>.log.<KEEP>` oldest) are kept.
pub(crate) const KEEP: u32 = 3;

/// Which desktop OS's convention [`path_from_env`] follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Os {
    Mac,
    Linux,
    Windows,
}

impl Os {
    const CURRENT: Self = if cfg!(target_os = "macos") {
        Self::Mac
    } else if cfg!(target_os = "windows") {
        Self::Windows
    } else {
        Self::Linux
    };
}

/// The daemon's log file for this build, from the environment; `None` when the variables it
/// needs are unset.
pub(crate) fn path() -> Option<PathBuf> {
    path_from_env(Flavour::CURRENT, Os::CURRENT, |name| std::env::var_os(name))
}

/// [`path`] for `flavour` and `os` over an explicit environment lookup. Empty values count as
/// unset.
pub(crate) fn path_from_env(
    flavour: Flavour,
    os: Os,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let env = |name| {
        lookup(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let stem = flavour.stem();
    let dir = match os {
        Os::Mac => env("HOME")?.join("Library/Logs").join(stem),
        Os::Linux => env("XDG_STATE_HOME")
            .or_else(|| env("HOME").map(|home| home.join(".local/state")))?
            .join(stem),
        Os::Windows => env("LOCALAPPDATA")?.join(stem).join("logs"),
    };
    Some(dir.join(format!("{stem}.log")))
}

/// A log file that is moved to `<name>.1` (and the older ones one further, the oldest
/// dropped) before a write would take it past `max_bytes`.
#[derive(Debug)]
pub(crate) struct RotatingFile {
    path: PathBuf,
    max_bytes: u64,
    keep: u32,
    file: File,
    written: u64,
}

impl RotatingFile {
    /// Opens (creating its directory) `path` for appending.
    ///
    /// # Errors
    /// When the directory or the file cannot be created.
    pub(crate) fn open(path: PathBuf, max_bytes: u64, keep: u32) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let file = append(&path)?;
        let written = file.metadata()?.len();
        Ok(Self {
            path,
            max_bytes,
            keep,
            file,
            written,
        })
    }

    fn rotated(&self, index: u32) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(format!(".{index}"));
        PathBuf::from(name)
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        if self.keep == 0 {
            self.file = File::create(&self.path)?;
        } else {
            for index in (1..self.keep).rev() {
                let from = self.rotated(index);
                if from.exists() {
                    fs::rename(&from, self.rotated(index + 1))?;
                }
            }
            fs::rename(&self.path, self.rotated(1))?;
            self.file = append(&self.path)?;
        }
        self.written = 0;
        Ok(())
    }
}

fn append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        if self.written > 0 && self.written.saturating_add(len) > self.max_bytes {
            self.rotate()?;
        }
        let count = self.file.write(buf)?;
        self.written = self
            .written
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nomouse-daemon-log-{name}-{}", std::process::id()));
        drop(fs::remove_dir_all(&dir));
        dir
    }

    #[test]
    fn the_log_lives_where_each_os_keeps_logs() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            }
        };
        let home = env(&[("HOME", "/Users/m")]);
        assert_eq!(
            path_from_env(Flavour::Release, Os::Mac, home),
            Some(PathBuf::from("/Users/m/Library/Logs/nomouse/nomouse.log"))
        );
        assert_eq!(
            path_from_env(Flavour::Dev, Os::Mac, home),
            Some(PathBuf::from(
                "/Users/m/Library/Logs/nomouse-dev/nomouse-dev.log"
            ))
        );
        assert_eq!(
            path_from_env(Flavour::Release, Os::Linux, home),
            Some(PathBuf::from("/Users/m/.local/state/nomouse/nomouse.log"))
        );
        assert_eq!(
            path_from_env(
                Flavour::Release,
                Os::Linux,
                env(&[("HOME", "/h"), ("XDG_STATE_HOME", "/s")])
            ),
            Some(PathBuf::from("/s/nomouse/nomouse.log"))
        );
        assert_eq!(
            path_from_env(
                Flavour::Release,
                Os::Windows,
                env(&[("LOCALAPPDATA", "C:/L")])
            ),
            Some(PathBuf::from("C:/L/nomouse/logs/nomouse.log"))
        );
        assert_eq!(
            path_from_env(Flavour::Release, Os::Mac, env(&[("HOME", "")])),
            None
        );
    }

    #[test]
    fn the_file_is_rotated_before_it_passes_the_cap_and_the_oldest_is_dropped() {
        let dir = scratch("rotate");
        let path = dir.join("nomouse.log");
        let mut log = RotatingFile::open(path.clone(), 10, 2).unwrap();
        for line in [
            "aaaa\n", "bbbb\n", "cccc\n", "dddd\n", "eeee\n", "ffff\n", "gggg\n",
        ] {
            log.write_all(line.as_bytes()).unwrap();
        }
        log.flush().unwrap();
        let read = |name: &str| fs::read_to_string(dir.join(name)).unwrap();
        assert_eq!(read("nomouse.log"), "gggg\n");
        assert_eq!(read("nomouse.log.1"), "eeee\nffff\n");
        assert_eq!(read("nomouse.log.2"), "cccc\ndddd\n");
        assert!(!dir.join("nomouse.log.3").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reopening_appends_and_counts_what_is_there() {
        let dir = scratch("reopen");
        let path = dir.join("nomouse.log");
        RotatingFile::open(path.clone(), 10, 1)
            .unwrap()
            .write_all(b"12345678\n")
            .unwrap();
        let mut again = RotatingFile::open(path.clone(), 10, 1).unwrap();
        again.write_all(b"next\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "next\n");
        assert_eq!(
            fs::read_to_string(dir.join("nomouse.log.1")).unwrap(),
            "12345678\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_line_longer_than_the_cap_is_still_written_whole() {
        let dir = scratch("long");
        let path = dir.join("nomouse.log");
        let mut log = RotatingFile::open(path.clone(), 4, 1).unwrap();
        log.write_all(b"a long line\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "a long line\n");
        fs::remove_dir_all(&dir).unwrap();
    }
}
