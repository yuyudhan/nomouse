//! Drives the real settings window: sets values through its widgets and checks that
//! `config.toml` on disk changed accordingly.
//!
//! It needs a display. Without `DISPLAY`/`WAYLAND_DISPLAY` it prints why and passes, unless
//! `NOMOUSE_UI_TESTS=1` demands that it runs. Run it on a private Xvfb inside its own session
//! bus, never on your desktop:
//!
//! ```sh
//! Xvfb :91 -screen 0 1600x1000x24 &
//! NOMOUSE_UI_TESTS=1 DISPLAY=:91 dbus-run-session -- cargo test -p nomouse-settings --test ui
//! ```
//!
//! With `NOMOUSE_SHOTS=<dir>` it also saves a screenshot of every page there (with the
//! `import` command). A plain `main` (`harness = false`) because GTK must run on the main
//! thread.

use std::io::Write;
use std::process::ExitCode;

/// Reports progress on stderr (a harness-less test has no other channel).
fn note(message: &str) {
    // Nothing useful can be done when stderr is closed.
    writeln!(std::io::stderr(), "{message}").ok();
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::time::Duration;

    use adw::prelude::*;
    use nomouse_core::Modifiers;
    use nomouse_settings::args::Page;
    use nomouse_settings::controls::Control;
    use nomouse_settings::daemon::Daemon;
    use nomouse_settings::pages::advanced::confirm_reset;
    use nomouse_settings::recorder::PROMPT;
    use nomouse_settings::window::{SettingsWindow, iterate_until};

    use super::note;

    fn ensure(condition: bool, what: &str) -> Result<(), String> {
        if condition {
            Ok(())
        } else {
            Err(format!("failed: {what}"))
        }
    }

    fn file(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    fn settle() {
        iterate_until(Duration::from_millis(150), || false);
    }

    /// Waits until the file contains `needle`.
    fn saved(path: &Path, needle: &str) -> Result<(), String> {
        ensure(
            iterate_until(Duration::from_secs(3), || file(path).contains(needle)),
            &format!("config.toml contains {needle:?}; it is:\n{}", file(path)),
        )
    }

    fn control<'a>(ui: &'a SettingsWindow, key: &str) -> Result<&'a Control, String> {
        ui.pages
            .control(key)
            .ok_or_else(|| format!("no control for {key}"))
    }

    /// Set in the child process that drives the window.
    const CHILD: &str = "NOMOUSE_SETTINGS_UI_CHILD";

    /// Runs the test in a child process whose `XDG_CONFIG_HOME` (and `HOME`) is a temporary
    /// directory: the window, the config file and the autostart entry all stay in it. (Edition
    /// 2024 makes changing this process's environment unsafe.) The child claims a Sway session
    /// (`XDG_CURRENT_DESKTOP=sway`, no compositor sockets), so the General page must show the
    /// Sway config line under "Start at login".
    pub(super) fn run() -> Result<(), String> {
        if std::env::var_os(CHILD).is_some() {
            return drive();
        }
        let root = std::env::temp_dir().join(format!("nomouse-settings-ui-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut child = std::process::Command::new(exe);
        child
            .env(CHILD, "1")
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("HOME", &root)
            .env("XDG_CURRENT_DESKTOP", "sway")
            .env_remove("SWAYSOCK")
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("I3SOCK")
            .env_remove("DESKTOP_SESSION");
        // GTK tries `$XDG_RUNTIME_DIR/wayland-0` even when WAYLAND_DISPLAY is unset: with only
        // DISPLAY given (a private Xvfb), pin X11 so the window never opens on the desktop.
        if std::env::var_os("WAYLAND_DISPLAY").is_none_or(|value| value.is_empty()) {
            child.env("GDK_BACKEND", "x11");
        }
        let status = child
            .status()
            .map_err(|e| format!("cannot run the test child: {e}"))?;
        std::fs::remove_dir_all(&root).ok();
        ensure(status.success(), "the window test (see above)")
    }

    fn drive() -> Result<(), String> {
        let path = nomouse_core::config::default_path().ok_or("no config path")?;
        ensure(
            std::env::var_os("XDG_CONFIG_HOME").is_some_and(|home| path.starts_with(home)),
            "the config file is inside the temporary XDG_CONFIG_HOME",
        )?;
        gtk::init().map_err(|e| format!("gtk init: {e}"))?;
        adw::init().map_err(|e| format!("adw init: {e}"))?;
        let ui = SettingsWindow::new(None, path.clone(), Daemon::new())?;
        ui.window.present();
        settle();
        ensure(!path.exists(), "nothing is written before a change")?;

        hotkeys(&ui, &path)?;
        note("hotkeys: recorded, Shift+/ kept as slash, duplicate and plain keys refused");
        choices_and_numbers(&ui, &path)?;
        note("theme combo, font size, toggle: saved");
        alphabet(&ui, &path)?;
        note("alphabet: invalid shown inline and not saved, valid saved");
        apps_and_colours(&ui, &path)?;
        note("ignored apps and colours: saved, blanks and duplicates refused");
        external_edit(&ui, &path)?;
        note("an edit on disk refreshed the widgets");
        pages(&ui)?;
        note("every page shown");
        start_at_login_on_sway(&ui)?;
        note("start at login: the Sway config line is shown");
        reset(&ui, &path)?;
        note("reset dialog: Reset restored the defaults");
        let about = ui.show_about();
        settle();
        ensure(about.is_visible(), "the About dialog opens")?;
        about.close();
        ui.window.close();
        Ok(())
    }

    fn hotkeys(ui: &SettingsWindow, path: &Path) -> Result<(), String> {
        let Control::Hotkey(hints) = control(ui, "hotkeys.hints")? else {
            return Err("hotkeys.hints is not a recorder".to_owned());
        };
        let ctrl_alt = Modifiers {
            ctrl: true,
            alt: true,
            ..Modifiers::NONE
        };
        hints.button().emit_clicked();
        ensure(
            hints.is_recording(),
            "clicking the recorder starts recording",
        )?;
        ensure(
            hints.button().label().as_deref() == Some(PROMPT),
            "the recorder prompts",
        )?;
        ensure(
            hints.press("Control_L", Modifiers::NONE).is_none(),
            "a lone modifier waits",
        )?;
        ensure(
            matches!(hints.press("a", Modifiers::NONE), Some(Err(_))) && hints.is_recording(),
            "a key without a modifier is refused and recording goes on",
        )?;
        ensure(
            hints.press("h", ctrl_alt) == Some(Ok(())),
            "Ctrl+Alt+H is saved",
        )?;
        saved(path, "hints = \"ctrl+alt+h\"")?;
        ensure(
            hints.button().label().as_deref() == Some("Ctrl+Alt+H"),
            "the recorder shows the new chord",
        )?;

        let Control::Hotkey(scroll) = control(ui, "hotkeys.scroll")? else {
            return Err("hotkeys.scroll is not a recorder".to_owned());
        };
        scroll.start();
        let ctrl_shift = Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::NONE
        };
        ensure(
            scroll.press("slash", ctrl_shift) == Some(Ok(())),
            "Ctrl+Shift+/ is saved",
        )?;
        saved(path, "scroll = \"ctrl+shift+slash\"")?;

        let Control::Hotkey(continuous) = control(ui, "hotkeys.continuous")? else {
            return Err("hotkeys.continuous is not a recorder".to_owned());
        };
        let before = file(path);
        let clash = continuous.apply("ctrl+alt+h".parse().map_err(|e| format!("{e:?}"))?);
        ensure(
            clash
                .as_ref()
                .is_err_and(|error| error.contains("not saved")),
            &format!("two modes on one chord are refused: {clash:?}"),
        )?;
        ensure(file(path) == before, "a refused chord is not written")?;
        continuous.start();
        ensure(
            continuous.press("Escape", Modifiers::NONE) == Some(Ok(()))
                && !continuous.is_recording(),
            "Escape cancels recording",
        )?;
        // This build's own default (a test build is the dev flavour, `cargo test --release`
        // the release one): nothing has changed the continuous chord yet.
        let default = nomouse_core::config::HotkeysConfig::default()
            .continuous
            .human();
        ensure(
            continuous.button().label().as_deref() == Some(default.as_str()),
            &format!("a cancelled recording keeps the old chord {default}"),
        )
    }

    fn choices_and_numbers(ui: &SettingsWindow, path: &Path) -> Result<(), String> {
        let Control::Choice(theme) = control(ui, "overlay.theme")? else {
            return Err("overlay.theme is not a combo".to_owned());
        };
        theme.set_selected(2);
        saved(path, "theme = \"dark\"")?;
        let Control::Number(size) = control(ui, "overlay.font_size")? else {
            return Err("overlay.font_size is not a spin button".to_owned());
        };
        size.set_value(20.0);
        saved(path, "font_size = 20")?;
        let Control::Toggle(semantic) = control(ui, "hints.semantic_click")? else {
            return Err("hints.semantic_click is not a switch".to_owned());
        };
        semantic.set_active(true);
        saved(path, "semantic_click = true")?;
        ensure(
            ui.pages.control("macos.enhanced_ui_apps").is_none(),
            "the macOS-only list is not shown on Linux",
        )
    }

    fn alphabet(ui: &SettingsWindow, path: &Path) -> Result<(), String> {
        let Control::Text(entry) = control(ui, "hints.alphabet")? else {
            return Err("hints.alphabet is not an entry".to_owned());
        };
        entry.set_text("lo");
        settle();
        ensure(
            entry.has_css_class("error"),
            "an invalid alphabet is marked",
        )?;
        ensure(
            !file(path).contains("alphabet"),
            "an invalid alphabet is not saved",
        )?;
        entry.set_text("fjdksa");
        saved(path, "alphabet = \"fjdksa\"")?;
        ensure(
            !entry.has_css_class("error"),
            "a valid alphabet is not marked",
        )
    }

    fn apps_and_colours(ui: &SettingsWindow, path: &Path) -> Result<(), String> {
        let Control::Apps(apps) = control(ui, "ignore.apps")? else {
            return Err("ignore.apps is not a list".to_owned());
        };
        ensure(apps.add("  ").is_err(), "a blank app is refused")?;
        ensure(apps.add("kitty").is_ok(), "an app is added")?;
        ensure(apps.add("KITTY").is_err(), "a duplicate app is refused")?;
        saved(path, "apps = [\"kitty\"]")?;
        let Control::Colour(button, reset) = control(ui, "overlay.border")? else {
            return Err("overlay.border is not a colour".to_owned());
        };
        ensure(!reset.is_sensitive(), "an unset colour offers no reset")?;
        button.set_rgba(&gtk::gdk::RGBA::new(1.0, 0.0, 0.0, 1.0));
        button.emit_by_name::<()>("color-set", &[]);
        saved(path, "border = \"#ff0000\"")?;
        ensure(reset.is_sensitive(), "a set colour offers the reset")?;
        reset.emit_clicked();
        settle();
        ensure(!file(path).contains("border"), "Use theme colour unsets it")
    }

    fn external_edit(ui: &SettingsWindow, path: &Path) -> Result<(), String> {
        let Control::Number(step) = control(ui, "scroll.step")? else {
            return Err("scroll.step is not a spin button".to_owned());
        };
        let text = format!("{}\n[scroll]\nstep = 7\n", file(path));
        std::fs::write(path, text).map_err(|e| e.to_string())?;
        ensure(
            iterate_until(Duration::from_secs(5), || step.value_as_int() == 7),
            "the Step spin button follows the file",
        )
    }

    fn start_at_login_on_sway(ui: &SettingsWindow) -> Result<(), String> {
        let subtitle = ui
            .pages
            .general
            .autostart_row
            .subtitle()
            .unwrap_or_default();
        let program = nomouse_settings::launch::nomouse_program();
        let line = format!("exec {} daemon", program.display());
        ensure(
            subtitle.contains(&line) && subtitle.contains("~/.config/sway/config"),
            &format!("the Start at login row names `{line}`; it says {subtitle:?}"),
        )
    }

    fn pages(ui: &SettingsWindow) -> Result<(), String> {
        let shots = std::env::var_os("NOMOUSE_SHOTS").map(PathBuf::from);
        if let Some(dir) = &shots {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        for (index, page) in Page::ALL.into_iter().enumerate() {
            ui.show_page(page);
            iterate_until(
                Duration::from_millis(if page == Page::Doctor { 3000 } else { 400 }),
                || false,
            );
            ensure(
                ui.page() == Some(page),
                &format!("{} is shown", page.title()),
            )?;
            if let Some(dir) = &shots {
                let file = dir.join(format!("{}-{}.png", index + 1, page.id()));
                let status = std::process::Command::new("import")
                    .args(["-window", "root"])
                    .arg(&file)
                    .status()
                    .map_err(|e| format!("import: {e}"))?;
                ensure(status.success(), "import took the screenshot")?;
            }
        }
        Ok(())
    }

    fn reset(ui: &SettingsWindow, path: &Path) -> Result<(), String> {
        let dialog = confirm_reset(Some(ui.window.upcast_ref()), &Rc::clone(&ui.state));
        settle();
        dialog.response(gtk::ResponseType::Accept);
        settle();
        ensure(
            !file(path).contains("theme") && !file(path).contains("apps"),
            &format!("reset removed every setting:\n{}", file(path)),
        )?;
        let Control::Choice(theme) = control(ui, "overlay.theme")? else {
            return Err("overlay.theme is not a combo".to_owned());
        };
        ensure(
            theme.selected() == 0,
            "the theme combo shows the default again",
        )
    }
}

fn main() -> ExitCode {
    let has_display = ["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()));
    let required = std::env::var_os("NOMOUSE_UI_TESTS").is_some_and(|value| value == "1");
    if !cfg!(target_os = "linux") {
        note("skipped: the settings window is Linux only");
        return ExitCode::SUCCESS;
    }
    if !has_display {
        if required {
            note("NOMOUSE_UI_TESTS=1 but DISPLAY and WAYLAND_DISPLAY are unset");
            return ExitCode::FAILURE;
        }
        note("skipped: no display (set DISPLAY, or NOMOUSE_UI_TESTS=1 to require this test)");
        return ExitCode::SUCCESS;
    }
    #[cfg(target_os = "linux")]
    if let Err(error) = linux::run() {
        note(&error);
        return ExitCode::FAILURE;
    }
    note("ui: passed");
    ExitCode::SUCCESS
}
