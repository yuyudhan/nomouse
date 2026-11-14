//! End-to-end check of the macOS backend against Finder.
//!
//! Runs only on macOS when this process is trusted for Accessibility; otherwise it prints why
//! it skipped and passes (GitHub's macOS runners have no Accessibility grant). It uses a plain
//! `main` (`harness = false`) because AppKit must run on the main thread.
//!
//! Finder is pointed at a scratch folder of its own, so the test never reuses or closes a
//! window the user opened; that window is closed again through Accessibility (also when the
//! test fails) and the test fails when it is still open afterwards.

use std::io::Write;

/// Reports progress on stderr (a harness-less test has no other channel).
fn note(message: &str) {
    // Nothing useful can be done when stderr is closed.
    writeln!(std::io::stderr(), "{message}").ok();
}

#[cfg(target_os = "macos")]
mod finder {
    use std::path::PathBuf;
    use std::process::Command;
    use std::thread::sleep;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use nomouse_sys_macos::app::{frontmost_pid, running_apps};
    use nomouse_sys_macos::ax::{AxElement, AxValue};

    const FINDER: &str = "com.apple.finder";
    const POLL: Duration = Duration::from_millis(100);

    /// Finder's process id, when it runs.
    fn finder_pid() -> Option<i32> {
        running_apps()
            .into_iter()
            .find(|app| app.bundle_id.as_deref() == Some(FINDER))
            .map(|app| app.pid)
    }

    fn title_is(element: &AxElement, title: &str) -> bool {
        matches!(element.attribute("AXTitle"), Ok(Some(AxValue::String(name))) if name == title)
    }

    /// Finder's windows titled `title`.
    fn windows(title: &str) -> Result<Vec<AxElement>, String> {
        let Some(pid) = finder_pid() else {
            return Ok(Vec::new());
        };
        let items = match AxElement::application(pid).attribute("AXWindows") {
            Ok(Some(AxValue::Array(items))) => items,
            Ok(_) => Vec::new(),
            Err(error) => return Err(format!("Finder windows: {error}")),
        };
        Ok(items
            .into_iter()
            .filter_map(|value| match value {
                AxValue::Element(window) => Some(window),
                _ => None,
            })
            .filter(|window| title_is(window, title))
            .collect())
    }

    /// Whether Finder is frontmost with its window titled `title` focused.
    fn focused(title: &str) -> bool {
        let Some(pid) = finder_pid() else {
            return false;
        };
        frontmost_pid() == Some(pid)
            && matches!(
                AxElement::application(pid).attribute("AXFocusedWindow"),
                Ok(Some(AxValue::Element(window))) if title_is(&window, title)
            )
    }

    /// Polls `done` every [`POLL`] until it holds or `limit` passes.
    fn wait_for(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        loop {
            if done() {
                return true;
            }
            if start.elapsed() >= limit {
                return false;
            }
            sleep(POLL);
        }
    }

    /// A scratch folder shown in a Finder window that only this test opened. Dropping it
    /// closes that window and deletes the folder.
    pub(crate) struct ScratchWindow {
        dir: PathBuf,
        title: String,
    }

    impl ScratchWindow {
        /// Creates a uniquely named folder with a few entries, opens it in Finder and waits
        /// until its window is the focused one.
        pub(crate) fn open() -> Result<Self, String> {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| format!("clock: {error}"))?
                .as_nanos();
            let title = format!("nomouse-finder-test-{}-{nanos}", std::process::id());
            let dir = std::env::temp_dir().join(&title);
            std::fs::create_dir(&dir)
                .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
            let scratch = Self { dir, title };
            std::fs::create_dir(scratch.dir.join("folder"))
                .map_err(|error| format!("cannot fill the scratch folder: {error}"))?;
            for name in ["one.txt", "two.txt", "three.txt"] {
                std::fs::write(scratch.dir.join(name), name)
                    .map_err(|error| format!("cannot fill the scratch folder: {error}"))?;
            }
            let status = Command::new("open")
                .arg("-a")
                .arg("Finder")
                .arg(&scratch.dir)
                .status()
                .map_err(|error| format!("cannot open Finder: {error}"))?;
            if !status.success() {
                return Err(format!("open -a Finder failed: {status}"));
            }
            if !wait_for(Duration::from_secs(10), || focused(&scratch.title)) {
                return Err(format!(
                    "Finder did not focus a window on {}",
                    scratch.title
                ));
            }
            // Let Finder fill the list before its elements are read.
            sleep(Duration::from_secs(1));
            Ok(scratch)
        }

        /// Presses the close button of every Finder window on the scratch folder.
        fn press_close(&self) -> Result<(), String> {
            for window in windows(&self.title)? {
                match window.attribute("AXCloseButton") {
                    Ok(Some(AxValue::Element(button))) => button
                        .perform_action("AXPress")
                        .map_err(|error| format!("close {}: {error}", self.title))?,
                    _ => return Err(format!("no close button on {}", self.title)),
                }
            }
            Ok(())
        }

        /// Closes the window and fails when a Finder window on the scratch folder is still
        /// open afterwards.
        pub(crate) fn close(self) -> Result<(), String> {
            self.press_close()?;
            let mut left = 0;
            let closed = wait_for(Duration::from_secs(5), || {
                left = windows(&self.title).map_or(usize::MAX, |open| open.len());
                left == 0
            });
            if closed {
                Ok(())
            } else {
                Err(format!(
                    "left {left} Finder window(s) open on {}",
                    self.title
                ))
            }
        }
    }

    impl Drop for ScratchWindow {
        fn drop(&mut self) {
            // Cleanup on a failed run: errors here cannot be reported any better.
            self.press_close().ok();
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }
}

#[cfg(target_os = "macos")]
fn run() -> Result<(), String> {
    use std::time::Duration;

    use nomouse_core::{Action, Config, Role, Scope, assign_hints};

    if !nomouse_sys_macos::ax::is_process_trusted(false) {
        note("skipped: this process is not trusted for Accessibility");
        return Ok(());
    }
    // The backend refuses to start untrusted, so the shared contract runs only here.
    let mut platform =
        nomouse_macos::platform(&Config::default()).map_err(|error| error.to_string())?;
    let violations = nomouse_core::contract::check_platform(
        platform.as_mut(),
        nomouse_core::contract::Probe::default(),
    );
    if !violations.is_empty() {
        let list: Vec<String> = violations.iter().map(ToString::to_string).collect();
        return Err(format!("backend contract:\n{}", list.join("\n")));
    }
    note("backend contract: passed");
    drop(platform);
    let scratch = finder::ScratchWindow::open()?;

    let config = Config::default();
    let mut platform = nomouse_macos::platform(&config).map_err(|error| error.to_string())?;
    let elements = platform
        .elements(Scope::FocusedWindow)
        .map_err(|error| format!("elements: {error}"))?;
    let clickable: Vec<_> = elements
        .into_iter()
        .filter(nomouse_core::Element::is_clickable)
        .collect();
    note(&format!("found {} clickable elements", clickable.len()));
    if !clickable.iter().any(|e| e.role == Role::MenuItem) {
        return Err("no menu bar items found".to_owned());
    }
    if !clickable.iter().any(|e| e.role != Role::MenuItem) {
        return Err("no window elements found in Finder".to_owned());
    }
    let alphabet = config.alphabet().map_err(|error| error.to_string())?;
    let hints = assign_hints(&alphabet, clickable.clone());
    platform
        .show(&hints, "")
        .map_err(|error| format!("show: {error}"))?;
    platform
        .next_event(Duration::from_millis(300))
        .map_err(|error| format!("next_event: {error}"))?;
    platform.hide().map_err(|error| format!("hide: {error}"))?;
    if let Some(first) = clickable.first() {
        platform
            .perform(&Action::hover(first.target()))
            .map_err(|error| format!("hover: {error}"))?;
    }
    drop(platform);
    scratch.close()?;
    note("Finder window closed");
    Ok(())
}

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(problem) => {
            note(&format!("macos_accessibility failed: {problem}"));
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    note("skipped: not macOS");
}
