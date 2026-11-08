//! End-to-end check of the macOS backend against Finder.
//!
//! Runs only on macOS when this process is trusted for Accessibility; otherwise it prints why
//! it skipped and passes (GitHub's macOS runners have no Accessibility grant). It uses a plain
//! `main` (`harness = false`) because AppKit must run on the main thread.

use std::io::Write;

/// Reports progress on stderr (a harness-less test has no other channel).
fn note(message: &str) {
    // Nothing useful can be done when stderr is closed.
    writeln!(std::io::stderr(), "{message}").ok();
}

#[cfg(target_os = "macos")]
fn run() -> Result<(), String> {
    use std::process::Command;
    use std::thread::sleep;
    use std::time::Duration;

    use nomouse_core::{Action, Config, Role, Scope, assign_hints};

    if !nomouse_sys_macos::ax::is_process_trusted(false) {
        note("skipped: this process is not trusted for Accessibility");
        return Ok(());
    }
    Command::new("open")
        .args(["-a", "Finder", "/Applications"])
        .status()
        .map_err(|error| format!("cannot open Finder: {error}"))?;
    sleep(Duration::from_secs(2));

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
