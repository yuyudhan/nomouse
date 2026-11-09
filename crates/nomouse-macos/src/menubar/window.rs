//! The settings window's behaviour: fills every control from `config.toml` through
//! [`Settings`], writes each change back at once, records shortcuts, and follows edits made to
//! the file elsewhere.

use std::path::Path;
use std::process::Command;

use num_traits::ToPrimitive as _;

use nomouse_core::Hotkey;
use nomouse_core::config::{Appearance, Theme};
use nomouse_core::menu::UpdateLine;
use nomouse_core::settings::{Change, Field, Kind, Settings, SettingsError, Value};
use nomouse_core::update::UpdateStatus;
use nomouse_sys_macos::login::LoginItem;
use nomouse_sys_macos::types::{Label, Rgba};
use nomouse_sys_macos::ui::MainThreadMarker;
use nomouse_sys_macos::ui::UiEvent;
use nomouse_sys_macos::ui::form::{Form, FormEvent, Tone};
use nomouse_sys_macos::ui::panels;

use super::pages::{self, field_at, field_tag};
use crate::keys;
use crate::shortcuts::{self, Recorded, SystemShortcut};
use crate::style;

/// The colour fields in the order `OverlayConfig::colours` returns them.
const COLOURS: [&str; 4] = [
    "overlay.background",
    "overlay.foreground",
    "overlay.typed_foreground",
    "overlay.border",
];

/// What the window asks of the menu bar app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    /// Release (`true`) the hotkeys while a shortcut is recorded, or take them again.
    Hold(bool),
    /// Run the doctor and show its report.
    Doctor,
    /// Check for a newer release.
    CheckForUpdates,
    /// Show the About panel.
    About,
    /// The start at login checkbox changed to this.
    StartAtLogin(bool),
    /// Ask for Accessibility and open its settings pane.
    OpenAccessibility,
}

/// Whether an interface event was the window's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Handled {
    /// It came from another control (the menu).
    NotMine,
    /// The window carried it out.
    Done,
    /// The window carried it out and asks the app for this.
    Outcome(Outcome),
}

/// What the General page shows besides settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct General {
    /// Whether nomouse has Accessibility.
    pub trusted: bool,
    /// Whether it starts at login.
    pub login: LoginItem,
    /// The update check's state.
    pub update: UpdateLine,
}

/// The settings window.
pub(super) struct SettingsWindow {
    form: Form,
    settings: Settings,
    /// macOS's own shortcuts, read when the window opens.
    system: Vec<SystemShortcut>,
    /// The field whose shortcut is being recorded.
    recording: Option<&'static Field>,
    mtm: MainThreadMarker,
}

impl std::fmt::Debug for SettingsWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsWindow")
            .field("path", &self.settings.path())
            .field("recording", &self.recording.map(|field| field.key))
            .finish_non_exhaustive()
    }
}

/// `path` with the home folder written as `~`.
fn tilde(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home.and_then(|home| path.strip_prefix(home).ok().map(Path::to_owned)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

fn rgba([red, green, blue]: [u8; 3]) -> Rgba {
    Rgba {
        red: f64::from(red) / 255.0,
        green: f64::from(green) / 255.0,
        blue: f64::from(blue) / 255.0,
        alpha: 1.0,
    }
}

fn hex(colour: Rgba) -> String {
    let byte = |channel: f64| {
        (channel.clamp(0.0, 1.0) * 255.0)
            .round()
            .to_u8()
            .unwrap_or(0)
    };
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(colour.red),
        byte(colour.green),
        byte(colour.blue)
    )
}

/// macOS's keyboard shortcuts as `com.apple.symbolichotkeys` sets them.
fn read_system_shortcuts() -> Vec<SystemShortcut> {
    let path = std::env::var_os("HOME")
        .map(|home| Path::new(&home).join("Library/Preferences/com.apple.symbolichotkeys.plist"));
    let json = path.and_then(|path| {
        let output = Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-"])
            .arg(path)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    });
    shortcuts::system_shortcuts(json.as_deref())
}

/// The note below a shortcut: which macOS shortcut it collides with, if any.
fn conflict_note(hotkey: Hotkey, system: &[SystemShortcut]) -> (String, Tone) {
    match shortcuts::conflict(hotkey, system) {
        Some(shortcut) => (
            format!(
                "macOS uses this for \u{201c}{}\u{201d}. Turn that off in System Settings › \
                 Keyboard › Keyboard Shortcuts, or record another.",
                shortcut.name
            ),
            Tone::Warning,
        ),
        None => (String::new(), Tone::Plain),
    }
}

impl SettingsWindow {
    /// Builds the window (hidden) over the config file.
    ///
    /// # Errors
    /// When the config file has no path or cannot be read.
    pub(super) fn new(mtm: MainThreadMarker) -> Result<Self, SettingsError> {
        Ok(Self {
            form: Form::new(mtm, "nomouse Settings", &pages::pages()),
            settings: Settings::open_default()?,
            system: read_system_shortcuts(),
            recording: None,
            mtm,
        })
    }

    /// Shows the window in front on `page` (the last one when `None`), filled from the file.
    pub(super) fn show(&mut self, page: Option<usize>, dark: bool) {
        self.system = read_system_shortcuts();
        self.settings.refresh();
        self.load(dark);
        self.form.show(page);
    }

    /// Whether the window is on screen.
    pub(super) fn is_visible(&self) -> bool {
        self.form.is_visible()
    }

    /// The selected page.
    pub(super) fn page_index(&self) -> usize {
        self.form.page_index()
    }

    /// Closes the window.
    pub(super) fn close(&self) {
        self.form.close();
    }

    /// The window as drawn now, as PNG bytes.
    pub(super) fn picture(&self) -> Option<Vec<u8>> {
        self.form.picture()
    }

    /// The window's controls, to drive them as a user would.
    #[cfg(feature = "testing")]
    pub(super) const fn form(&self) -> &Form {
        &self.form
    }

    /// Fills the General page.
    pub(super) fn set_general(&self, general: &General) {
        let (access, tone) = if general.trusted {
            ("Allowed", Tone::Plain)
        } else {
            (
                "Not allowed: the hotkeys do nothing until you allow nomouse.",
                Tone::Error,
            )
        };
        self.form.set_note(pages::ACCESS_NOTE, access, tone);
        let (on, note, tone) = match &general.login {
            LoginItem::Enabled => (true, String::new(), Tone::Plain),
            LoginItem::Disabled => (false, String::new(), Tone::Plain),
            LoginItem::NeedsApproval => (
                true,
                "Waiting for your approval in System Settings › General › Login Items.".to_owned(),
                Tone::Warning,
            ),
            LoginItem::Unavailable(reason) => {
                (false, format!("Unavailable: {reason}."), Tone::Plain)
            }
        };
        self.form.set_toggle(pages::LOGIN, on);
        self.form.set_note(pages::LOGIN_NOTE, &note, tone);
        let (update, tone) = match &general.update {
            UpdateLine::NotChecked => (
                format!("nomouse {}", env!("CARGO_PKG_VERSION")),
                Tone::Plain,
            ),
            UpdateLine::Checking => ("Checking…".to_owned(), Tone::Plain),
            UpdateLine::Checked(status @ UpdateStatus::UpToDate(_)) => {
                (status.message(), Tone::Plain)
            }
            UpdateLine::Checked(status @ UpdateStatus::Available { .. }) => {
                (status.message(), Tone::Warning)
            }
            UpdateLine::Failed(error) => (format!("The check failed: {error}"), Tone::Error),
        };
        self.form.set_note(pages::UPDATE_NOTE, &update, tone);
    }

    /// Puts the settings in effect into every control, and the file's state into the status
    /// line. `dark` is the system appearance, for the preview of the `auto` theme.
    fn load(&self, dark: bool) {
        let config = self.settings.config();
        for field in pages::shown_fields() {
            let tag = field_tag(field);
            let Ok(value) = self.settings.value(field.key) else {
                continue;
            };
            match (field.kind, value) {
                (Kind::Hotkey, Value::Text(text)) => {
                    if self.recording.map(|recording| recording.key) == Some(field.key) {
                        continue;
                    }
                    if let Ok(hotkey) = text.parse::<Hotkey>() {
                        let (note, tone) = conflict_note(hotkey, &self.system);
                        self.form
                            .set_shortcut(tag, &shortcuts::symbols(hotkey), &note, tone);
                    }
                }
                (Kind::Text, Value::Text(text)) => self.form.set_text(tag, &text),
                (Kind::Choice(choices), Value::Text(text)) => {
                    if let Some(index) = choices.iter().position(|choice| choice.value == text) {
                        self.form.set_choice(tag, index);
                    }
                }
                (Kind::Number { .. }, Value::Number(number)) => self.form.set_number(tag, number),
                (Kind::Toggle, Value::Bool(on)) => self.form.set_toggle(tag, on),
                (Kind::Apps, Value::List(ids)) => {
                    let apps: Vec<panels::AppInfo> =
                        ids.iter().map(|id| panels::app_for(id)).collect();
                    self.form.set_apps(tag, &apps);
                }
                // Colours are filled below, from the theme when unset.
                _ => {}
            }
        }
        let appearance = if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        let colours = config
            .overlay
            .colours(appearance)
            .unwrap_or_else(|_| config.overlay.theme.palette(appearance));
        for (key, colour) in COLOURS.into_iter().zip(colours) {
            if let Some(field) = nomouse_core::settings::field_for(key) {
                let from_theme = matches!(self.settings.value(key), Ok(Value::Unset));
                self.form
                    .set_colour(field_tag(field), rgba(colour), from_theme);
            }
        }
        self.preview(dark);
        self.form.set_note(
            pages::CONFIG_NOTE,
            &tilde(self.settings.path()),
            Tone::Plain,
        );
        match self.settings.problem() {
            Some(problem) => self.form.set_status(
                &format!("{problem}. The last good settings stay in effect."),
                Tone::Error,
            ),
            None => self.form.set_status(
                &format!("Saved in {} as you change it.", tilde(self.settings.path())),
                Tone::Plain,
            ),
        }
    }

    /// Draws sample labels as the overlay will.
    fn preview(&self, dark: bool) {
        let overlay = &self.settings.config().overlay;
        let dark = match overlay.theme {
            Theme::Auto => dark,
            Theme::Dark => true,
            Theme::Original | Theme::Light => false,
        };
        let appearance = if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        let colours = overlay
            .colours(appearance)
            .unwrap_or_else(|_| overlay.theme.palette(appearance));
        let style = style::label_style(overlay.font_size, colours);
        let samples = [
            ("SA", 0, false),
            ("SD", 1, false),
            ("DF", 0, true),
            ("FG", 0, false),
        ];
        let labels: Vec<Label> = samples
            .iter()
            .zip(0_u32..)
            .map(|((text, typed, focused), index)| Label {
                screen: 0,
                x: 20.0 + f64::from(index) * 92.0,
                y: 18.0,
                text: (*text).to_owned(),
                typed: *typed,
                focused: *focused,
            })
            .collect();
        self.form.set_preview(
            pages::PREVIEW,
            &labels,
            &style,
            style::opacity(overlay.opacity),
            dark,
        );
    }

    /// Saves `value` for `field` and shows the outcome; a refused value puts the control back.
    fn save(&mut self, field: &Field, value: Value, dark: bool) {
        match self.settings.set(field.key, value) {
            Ok(_) => self.load(dark),
            Err(error) => {
                self.load(dark);
                self.form.set_status(&error.to_string(), Tone::Error);
            }
        }
    }

    /// The value the user gave `field`'s control.
    fn value_of(&self, field: &Field) -> Option<Value> {
        let tag = field_tag(field);
        Some(match field.kind {
            Kind::Text => Value::Text(self.form.text(tag).trim().to_owned()),
            Kind::Choice(choices) => {
                Value::Text(choices.get(self.form.choice(tag)?)?.value.to_owned())
            }
            Kind::Number { .. } => Value::Number(self.form.number(tag)),
            Kind::Toggle => Value::Bool(self.form.toggle(tag)),
            Kind::Colour => Value::Text(hex(self.form.colour(tag)?)),
            Kind::Hotkey | Kind::Apps => return None,
        })
    }

    fn apps(&self, field: &Field) -> Vec<String> {
        match self.settings.value(field.key) {
            Ok(Value::List(ids)) => ids,
            _ => Vec::new(),
        }
    }

    /// Handles `event` when it belongs to the window.
    pub(super) fn handle(&mut self, event: &UiEvent, dark: bool) -> Handled {
        match self.form.translate(event) {
            Some(event) => self
                .act(event, dark)
                .map_or(Handled::Done, Handled::Outcome),
            None => Handled::NotMine,
        }
    }

    /// Carries out a window event; returns what the menu bar app has to do about it.
    fn act(&mut self, event: FormEvent, dark: bool) -> Option<Outcome> {
        match event {
            FormEvent::Changed(pages::LOGIN) => {
                Some(Outcome::StartAtLogin(self.form.toggle(pages::LOGIN)))
            }
            FormEvent::Changed(tag) => {
                let field = field_at(tag)?;
                if let Some(value) = self.value_of(field) {
                    self.save(field, value, dark);
                }
                None
            }
            FormEvent::Pressed(tag) => self.pressed(tag, dark),
            FormEvent::Reset(tag) => {
                let field = field_at(tag)?;
                self.save(field, Value::Unset, dark);
                None
            }
            FormEvent::AddApp(tag) => {
                let field = field_at(tag)?;
                let mut ids = self.apps(field);
                for app in panels::choose_apps(self.mtm, "Choose apps") {
                    if !ids.contains(&app.id) {
                        ids.push(app.id);
                    }
                }
                self.save(field, Value::List(ids), dark);
                None
            }
            FormEvent::RemoveApp(tag, index) => {
                let field = field_at(tag)?;
                let mut ids = self.apps(field);
                if index < ids.len() {
                    ids.remove(index);
                }
                self.save(field, Value::List(ids), dark);
                None
            }
            FormEvent::Key {
                tag,
                keycode,
                characters,
                modifiers,
            } => self.key(tag, keycode, &characters, keys::held(modifiers), dark),
        }
    }

    fn pressed(&mut self, tag: isize, dark: bool) -> Option<Outcome> {
        match tag {
            pages::ACCESS_BUTTON => Some(Outcome::OpenAccessibility),
            pages::UPDATE_BUTTON => Some(Outcome::CheckForUpdates),
            pages::DOCTOR => Some(Outcome::Doctor),
            pages::ABOUT => Some(Outcome::About),
            pages::OPEN_CONFIG => {
                self.open_config(&["-t"]);
                None
            }
            pages::REVEAL_CONFIG => {
                self.open_config(&["-R"]);
                None
            }
            pages::RESET_ALL => {
                self.reset(dark);
                None
            }
            _ => {
                let field = field_at(tag)?;
                if field.kind != Kind::Hotkey {
                    return None;
                }
                let held = self.recording.is_none();
                self.recording = Some(field);
                self.load(dark);
                self.form.record(tag, "Type the shortcut…");
                self.form.set_shortcut(
                    tag,
                    "Type the shortcut…",
                    "Press the keys together; Escape keeps the old one.",
                    Tone::Plain,
                );
                held.then_some(Outcome::Hold(true))
            }
        }
    }

    fn key(
        &mut self,
        tag: isize,
        keycode: u16,
        characters: &str,
        modifiers: nomouse_core::Modifiers,
        dark: bool,
    ) -> Option<Outcome> {
        let field = self.recording.filter(|field| field_tag(field) == tag)?;
        match shortcuts::recorded(keycode, characters, modifiers) {
            Recorded::Invalid(why) => {
                self.form
                    .set_shortcut(tag, "Type the shortcut…", why, Tone::Warning);
                None
            }
            Recorded::Cancel => {
                self.stop_recording(dark);
                Some(Outcome::Hold(false))
            }
            Recorded::Hotkey(hotkey) => {
                self.stop_recording(dark);
                self.save(field, Value::Text(hotkey.to_string()), dark);
                Some(Outcome::Hold(false))
            }
        }
    }

    fn stop_recording(&mut self, dark: bool) {
        self.recording = None;
        self.form.stop_recording();
        self.load(dark);
    }

    /// Opens the config file with `open` and `flags` (`-t`: the default text editor, `-R`:
    /// Finder), writing a commented file first when there is none.
    fn open_config(&self, flags: &[&str]) {
        let path = self.settings.path();
        if !path.exists() {
            let header = "# nomouse settings. Every key is optional; the settings window writes \
                          only what differs from the defaults.\n# Reference: \
                          https://github.com/yuyudhan/nomouse/blob/main/docs/configuration.md\n";
            if let Err(error) = nomouse_core::settings::write_atomically(path, header) {
                self.form.set_status(
                    &format!("cannot create {}: {error}", path.display()),
                    Tone::Error,
                );
                return;
            }
        }
        let opened = Command::new("/usr/bin/open").args(flags).arg(path).status();
        if !opened.is_ok_and(|status| status.success()) {
            self.form
                .set_status(&format!("cannot open {}", path.display()), Tone::Error);
        }
    }

    /// Asks, then removes every setting from the file.
    fn reset(&mut self, dark: bool) {
        let choice = panels::alert(
            self.mtm,
            "Reset every setting to its default?",
            "nomouse removes your settings from config.toml. Comments and keys it does not \
             know stay. This cannot be undone.",
            &["Reset", "Cancel"],
            true,
        );
        if choice != 0 {
            return;
        }
        match self.settings.reset() {
            Ok(_) => self.load(dark),
            Err(error) => self.form.set_status(&error.to_string(), Tone::Error),
        }
    }

    /// Follows the file (edited in a text editor) and the window being closed while a shortcut
    /// was recorded; call it from the event loop.
    pub(super) fn tick(&mut self, dark: bool) -> Option<Outcome> {
        if self.recording.is_some() && !self.form.is_visible() {
            self.recording = None;
            self.form.stop_recording();
            return Some(Outcome::Hold(false));
        }
        if !self.form.is_visible() {
            return None;
        }
        // A toolbar tab clicked, or a note shown, since the last tick.
        self.form.fit();
        match self.settings.watch() {
            Change::Unchanged => {}
            Change::Reloaded | Change::Invalid(_) => self.load(dark),
        }
        None
    }
}
