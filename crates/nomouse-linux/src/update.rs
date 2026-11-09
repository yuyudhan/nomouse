//! The update check over the network: `curl` (which the installer needs anyway) reads where
//! the latest-release page redirects. Blocking: call it off the UI and event-loop threads.

use std::process::{Command, Stdio};

use nomouse_core::update::{LATEST_URL, UpdateStatus, latest_from_location};

/// Seconds `curl` may take before the check gives up.
const TIMEOUT_SECONDS: &str = "15";

/// Checks GitHub for a release newer than `current` (`env!("CARGO_PKG_VERSION")`).
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
