//! End-to-end test on a real Windows desktop: launch a tiny `WinForms` window, read its elements
//! through UI Automation, click its button through the backend, and watch the handler run.
#![cfg(windows)]

use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

use nomouse_core::{
    Action, Alphabet, Config, HotkeySource, InputInjector, Overlay, Role, assign_hints,
};
use nomouse_windows::{ScopeOptions, WindowHandle, WindowsPlatform, platform_with};

/// Kills the form when the test ends, pass or fail.
struct Form(Child);

impl Drop for Form {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn launch(title: &str) -> std::io::Result<Form> {
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         $form = New-Object System.Windows.Forms.Form; \
         $form.Text = '{title}'; $form.Width = 400; $form.Height = 200; \
         $button = New-Object System.Windows.Forms.Button; \
         $button.Text = 'Press me'; $button.Width = 200; $button.Height = 60; \
         $button.Add_Click({{ $form.Text = '{title}-clicked' }}); \
         $form.Controls.Add($button); \
         [System.Windows.Forms.Application]::Run($form)"
    );
    let child = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .spawn()?;
    Ok(Form(child))
}

fn wait_for_window(
    platform: &mut WindowsPlatform,
    title: &str,
    timeout: Duration,
) -> Option<WindowHandle> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(window) = platform.windows().into_iter().find(|w| w.title == title) {
            return Some(window.handle);
        }
        thread::sleep(Duration::from_millis(250));
    }
    None
}

#[test]
fn invokes_a_winforms_button() {
    let title = format!("nomouse-test-{}", std::process::id());
    let _form = launch(&title).expect("powershell starts");

    let mut config = Config::default();
    config.hints.semantic_click = true;
    let mut platform = platform_with(&config, ScopeOptions::default()).expect("backend starts");

    let window = wait_for_window(&mut platform, &title, Duration::from_secs(60))
        .expect("the WinForms window appears");

    // The button may take a moment to be exposed after the window maps.
    let deadline = Instant::now() + Duration::from_secs(20);
    let button = loop {
        let elements = platform
            .window_elements(window)
            .expect("elements are readable");
        if let Some(button) = elements
            .into_iter()
            .find(|e| e.role == Role::Button && e.name == "Press me")
        {
            break button;
        }
        assert!(Instant::now() < deadline, "the button is never exposed");
        thread::sleep(Duration::from_millis(250));
    };
    assert!(button.is_clickable());

    // The overlay draws and hides without errors.
    let hints = assign_hints(&Alphabet::default(), vec![button.clone()]);
    platform.show(&hints, "").expect("overlay shows");
    platform.show(&hints, "a").expect("overlay updates");
    platform.hide().expect("overlay hides");

    // Hotkeys register and unregister (a chord nobody else uses).
    let id = platform
        .register("ctrl+alt+shift+f24".parse().expect("valid chord"))
        .expect("hotkey registers");
    assert_eq!(
        platform
            .next_event(Duration::from_millis(50))
            .expect("pump"),
        None
    );
    platform.unregister(id).expect("hotkey unregisters");

    platform
        .perform(&Action::click(button.target()))
        .expect("click");
    let clicked = format!("{title}-clicked");
    assert!(
        wait_for_window(&mut platform, &clicked, Duration::from_secs(20)).is_some(),
        "the click handler ran"
    );
}
