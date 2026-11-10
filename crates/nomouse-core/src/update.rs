//! The update check: which release is the newest, and whether it is newer than this build.
//!
//! The check reads where `https://github.com/yuyudhan/nomouse/releases/latest` redirects
//! (`.../releases/tag/vX.Y.Z`), so it needs no API token, no JSON and no rate-limited API.
//! It only tells: installing stays with the installer or the package manager. [`check`] asks
//! over the network with `curl`, which the installer needs anyway, on Linux and macOS alike.
//! [`INSTALL_COMMAND`] is what menus and dialogs show for the update itself.

use std::process::{Command, Stdio};

/// The page that redirects to the newest release's tag.
pub const LATEST_URL: &str = "https://github.com/yuyudhan/nomouse/releases/latest";

/// The installer one-liner (macOS and Linux) that installs or updates to the newest release.
/// Until releases are Developer-ID signed, this is the update path to offer on macOS: the
/// installer clears the download quarantine that makes Gatekeeper refuse a browser-downloaded
/// `.dmg` or `.app.zip`.
pub const INSTALL_COMMAND: &str = "curl -fsSL --retry 5 --retry-all-errors \
     https://github.com/yuyudhan/nomouse/releases/latest/download/install.sh | bash";

/// Where the release notes of `version` (`0.1.3`) are.
pub fn release_url(version: &str) -> String {
    format!("https://github.com/yuyudhan/nomouse/releases/tag/v{version}")
}

/// A `major.minor.patch` version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    /// Parses `1.2.3` or `v1.2.3`; anything after a `-` or `+` (pre-release, build) is
    /// ignored.
    pub fn parse(text: &str) -> Option<Self> {
        let core = text.trim().trim_start_matches('v');
        let core = core.split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
        let version = Self(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(version)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The release version in an HTTP response's `Location` header (or the final URL) pointing
/// at `.../releases/tag/vX.Y.Z`, from the response headers or a bare URL.
pub fn latest_from_location(response: &str) -> Option<Version> {
    response.lines().rev().find_map(|line| {
        let line = line.trim();
        let url = match line.split_once(':') {
            Some((name, value)) if name.eq_ignore_ascii_case("location") => value.trim(),
            _ => line,
        };
        let tag = url.split("/releases/tag/").nth(1)?;
        Version::parse(tag.split(['/', '?', '#']).next()?)
    })
}

/// The outcome of a check, as menus and the settings window say it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// This build is the newest release (or newer).
    UpToDate(Version),
    /// A newer release exists.
    Available {
        /// This build.
        current: Version,
        /// The newest release.
        latest: Version,
    },
}

impl UpdateStatus {
    /// Compares this build's `current` version (`CARGO_PKG_VERSION`) with `latest`.
    ///
    /// # Errors
    /// When `current` is not a version.
    pub fn new(current: &str, latest: Version) -> Result<Self, String> {
        let current =
            Version::parse(current).ok_or_else(|| format!("{current:?} is not a version"))?;
        Ok(if latest > current {
            Self::Available { current, latest }
        } else {
            Self::UpToDate(current)
        })
    }

    /// One line for a menu item or a notification.
    pub fn message(&self) -> String {
        match self {
            Self::UpToDate(current) => format!("nomouse {current} is up to date"),
            Self::Available { current, latest } => {
                format!("nomouse {latest} is available (you have {current})")
            }
        }
    }
}

/// Seconds `curl` may take before the check gives up.
const TIMEOUT_SECONDS: &str = "15";

/// Checks GitHub for a release newer than `current` (`env!("CARGO_PKG_VERSION")`). Blocking:
/// call it off the UI and event-loop threads.
///
/// # Errors
/// When `curl` is missing or fails (offline, timeout), or the answer names no release.
pub fn check(current: &str) -> Result<UpdateStatus, String> {
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--head",
            "--max-time",
            TIMEOUT_SECONDS,
            LATEST_URL,
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("could not reach GitHub: {}", error.trim()));
    }
    let headers = String::from_utf8_lossy(&output.stdout);
    let latest =
        latest_from_location(&headers).ok_or_else(|| "GitHub named no release".to_owned())?;
    UpdateStatus::new(current, latest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_and_order() {
        assert_eq!(Version::parse("v0.1.3"), Some(Version(0, 1, 3)));
        assert_eq!(Version::parse("1.10.0-rc.1"), Some(Version(1, 10, 0)));
        assert_eq!(Version::parse("1.2"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert_eq!(Version::parse("x.1.2"), None);
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert_eq!(Version(1, 2, 3).to_string(), "1.2.3");
    }

    #[test]
    fn reads_the_tag_from_the_redirect() {
        let headers = "HTTP/2 302 \r\nserver: GitHub.com\r\nlocation: \
                       https://github.com/yuyudhan/nomouse/releases/tag/v0.1.4\r\n\r\n";
        assert_eq!(latest_from_location(headers), Some(Version(0, 1, 4)));
        assert_eq!(
            latest_from_location("https://github.com/yuyudhan/nomouse/releases/tag/v2.0.1"),
            Some(Version(2, 0, 1))
        );
        // No release yet: GitHub redirects to the releases list.
        assert_eq!(
            latest_from_location("location: https://github.com/yuyudhan/nomouse/releases"),
            None
        );
    }

    #[test]
    fn compares_with_this_build() {
        assert_eq!(
            UpdateStatus::new("0.1.3", Version(0, 1, 4)).unwrap(),
            UpdateStatus::Available {
                current: Version(0, 1, 3),
                latest: Version(0, 1, 4)
            }
        );
        let same = UpdateStatus::new("0.1.4", Version(0, 1, 4)).unwrap();
        assert_eq!(same.message(), "nomouse 0.1.4 is up to date");
        assert_eq!(
            UpdateStatus::new("0.2.0", Version(0, 1, 9)).unwrap(),
            UpdateStatus::UpToDate(Version(0, 2, 0))
        );
        assert!(UpdateStatus::new("dev", Version(0, 1, 0)).is_err());
    }

    #[test]
    fn the_install_command_fetches_the_latest_installer() {
        assert_eq!(
            INSTALL_COMMAND,
            "curl -fsSL --retry 5 --retry-all-errors \
             https://github.com/yuyudhan/nomouse/releases/latest/download/install.sh | bash"
        );
        assert!(INSTALL_COMMAND.contains(&format!("{LATEST_URL}/download/install.sh")));
    }
}
