//! The guided first run inside the app (NM-APP-10): while Accessibility is missing, nomouse
//! shows its menu bar item and a Welcome window that explains the permission, opens its
//! settings pane and follows the grant, instead of a row of alerts.

use std::process::Command;
use std::time::{Duration, Instant};

use nomouse_core::setup::{Presenter, Step};
use nomouse_sys_macos::app::{App, NotMainThread};
use nomouse_sys_macos::ui::form::{Form, FormEvent, Page, Row, Spec, Tone};
use nomouse_sys_macos::ui::status::{MenuLine, StatusItem};
use nomouse_sys_macos::ui::{self, UiEvent, panels};

use super::MENU_ICON;
use crate::setup::{ACCESSIBILITY_SETTINGS, dialog};

/// "Open System Settings".
const OPEN: isize = 80;
/// "Quit".
const QUIT: isize = 81;
/// The explanation and progress.
const NOTE: isize = 82;

/// How often the window is serviced while the flow waits.
const SLICE: Duration = Duration::from_millis(50);

/// The first run's menu bar item and Welcome window.
#[derive(Debug)]
pub struct Onboarding {
    app: App,
    item: StatusItem,
    form: Form,
}

fn page() -> Page {
    Page {
        title: "Welcome".to_owned(),
        symbol: "hand.wave".to_owned(),
        summary: "nomouse labels every button, link and field so you click them from the \
                  keyboard. It needs one permission first."
            .to_owned(),
        rows: vec![
            Row {
                label: String::new(),
                spec: Spec::Note { tag: NOTE },
                help: String::new(),
                id: "welcome.note".to_owned(),
            },
            Row {
                label: String::new(),
                spec: Spec::Buttons(vec![
                    (OPEN, "Open System Settings".to_owned()),
                    (QUIT, "Quit".to_owned()),
                ]),
                help: String::new(),
                id: String::new(),
            },
        ],
    }
}

/// What the user picked.
enum Pick {
    Open,
    Quit,
}

impl Onboarding {
    /// The menu bar item and the (hidden) window.
    ///
    /// # Errors
    /// [`NotMainThread`] off the main thread.
    pub fn new() -> Result<Self, NotMainThread> {
        let app = App::new()?;
        let mtm = app.main_thread();
        ui::install_main_menu(mtm);
        let item = StatusItem::new(mtm, MENU_ICON, "nomouse");
        item.set_menu(&[
            MenuLine::Item {
                tag: 0,
                title: "nomouse needs Accessibility".to_owned(),
                enabled: false,
                checked: false,
                shortcut: None,
            },
            MenuLine::Item {
                tag: OPEN,
                title: "Open System Settings…".to_owned(),
                enabled: true,
                checked: false,
                shortcut: None,
            },
            MenuLine::Separator,
            MenuLine::Item {
                tag: QUIT,
                title: "Quit nomouse".to_owned(),
                enabled: true,
                checked: false,
                shortcut: None,
            },
        ]);
        item.set_dimmed(true);
        let form = Form::new(mtm, "Welcome to nomouse", &[page()]);
        Ok(Self { app, item, form })
    }

    /// Services the window for up to `duration`; returns the first button or menu pick.
    fn pump(&self, duration: Duration) -> Option<Pick> {
        let deadline = Instant::now() + duration;
        loop {
            self.app.pump(SLICE.as_secs_f64());
            for event in ui::take_events() {
                let tag = match self.form.translate(&event) {
                    Some(FormEvent::Pressed(tag)) => tag,
                    _ => match event {
                        UiEvent::Action(tag) => tag,
                        UiEvent::Key { .. } => continue,
                    },
                };
                match tag {
                    OPEN => return Some(Pick::Open),
                    QUIT => return Some(Pick::Quit),
                    _ => {}
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
        }
    }

    fn note(&self, text: &str, tone: Tone) {
        self.form.set_note(NOTE, text, tone);
    }
}

/// Quits the app from the Welcome window or its menu.
fn quit() -> ! {
    tracing::info!("quit during the first run");
    std::process::exit(0)
}

impl Presenter for Onboarding {
    fn show(&mut self, step: &Step) -> bool {
        match step {
            Step::Explain(permission) => {
                self.note(
                    &format!("{}\n\n{}", permission.why, permission.how),
                    Tone::Plain,
                );
                self.form.show(None);
                ui::activate(self.app.main_thread());
                loop {
                    match self.pump(Duration::from_secs(3600)) {
                        Some(Pick::Open) => return true,
                        Some(Pick::Quit) => return false,
                        None => {}
                    }
                }
            }
            Step::Requested(permission) | Step::Waiting(permission) => {
                self.note(
                    &format!(
                        "Waiting for {}: {}\n\nnomouse starts by itself once you allow it.",
                        permission.name, permission.how
                    ),
                    Tone::Warning,
                );
                true
            }
            Step::Granted(permission) => {
                self.note(
                    &format!("{} is allowed. nomouse is starting…", permission.name),
                    Tone::Plain,
                );
                // Long enough to read before the restart replaces the window.
                let _ = self.pump(Duration::from_millis(800));
                true
            }
            Step::TimedOut(_) | Step::RequestFailed(..) => {
                if let Some(dialog) = dialog(step) {
                    panels::alert(
                        self.app.main_thread(),
                        &dialog.title,
                        &dialog.message,
                        &dialog.buttons,
                        true,
                    );
                }
                true
            }
            Step::AlreadyGranted(_) | Step::Done { .. } => true,
        }
    }

    fn wait(&mut self, duration: Duration) {
        match self.pump(duration) {
            Some(Pick::Open) => {
                if let Err(error) = Command::new("/usr/bin/open")
                    .arg(ACCESSIBILITY_SETTINGS)
                    .status()
                {
                    tracing::warn!("cannot open System Settings: {error}");
                }
            }
            Some(Pick::Quit) => quit(),
            None => {}
        }
    }
}

impl Drop for Onboarding {
    fn drop(&mut self) {
        self.form.close();
        self.item.set_dimmed(false);
    }
}
