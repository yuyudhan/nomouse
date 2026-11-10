//! The single-instance application: a second launch forwards its arguments to the first,
//! which presents its window.

use std::cell::RefCell;
use std::io::Write;
use std::process::ExitCode;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use nomouse_core::Flavour;
use nomouse_linux::control::{APP_ID, ICON};

use crate::args::{self, Args};
use crate::daemon::Daemon;
use crate::window::SettingsWindow;

/// Writes `message` to stderr.
fn note(message: &str) {
    // Nothing else to do when stderr is closed.
    writeln!(std::io::stderr(), "{message}").ok();
}

/// The arguments after the program name, as text.
fn strings(args: impl IntoIterator<Item = std::ffi::OsString>) -> Vec<String> {
    args.into_iter()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// The window, once built, shared by every command line the primary instance receives.
type Shared<T> = Rc<RefCell<Option<Rc<T>>>>;

/// Runs the app. The command line is parsed here first, so `--version`, `--help` and
/// mistakes are answered by the launch that asked, without GTK or a display; the rest is
/// handed to the primary instance.
pub fn run() -> ExitCode {
    let parsed = match args::parse(strings(std::env::args_os())) {
        Ok(parsed) => parsed,
        Err(error) => {
            note(&error);
            return ExitCode::from(2);
        }
    };
    if parsed.version || parsed.help {
        let text = if parsed.version {
            args::version_line()
        } else {
            args::USAGE.to_owned()
        };
        // A closed stdout leaves nothing to report to.
        writeln!(std::io::stdout(), "{text}").ok();
        return ExitCode::SUCCESS;
    }
    // WM_CLASS and the app name match the launcher entry (`APP_ID.desktop`), so docks group
    // the window under its icon. A development build's id is its own, so it never hands its
    // arguments to an open release window (or the other way round).
    glib::set_prgname(Some(APP_ID));
    glib::set_application_name(Flavour::CURRENT.display_name());
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|_| gtk::Window::set_default_icon_name(ICON));
    let window: Shared<SettingsWindow> = Rc::new(RefCell::new(None));
    let daemon: Shared<Daemon> = Rc::new(RefCell::new(None));
    app.connect_command_line(move |app, command_line| {
        let args = match args::parse(strings(command_line.arguments())) {
            Ok(args) => args,
            Err(error) => {
                note(&error);
                return glib::ExitCode::from(2);
            }
        };
        let daemon = Rc::clone(daemon.borrow_mut().get_or_insert_with(Daemon::new));
        open(app, &window, &daemon, &args);
        glib::ExitCode::SUCCESS
    });
    let code = app.run_with_args(&std::env::args().collect::<Vec<_>>());
    ExitCode::from(code.get())
}

/// Starts the daemon if needed, then shows the window as `args` ask.
fn open(app: &adw::Application, window: &Shared<SettingsWindow>, daemon: &Rc<Daemon>, args: &Args) {
    let existing = window.borrow().clone();
    let settings = if let Some(settings) = existing {
        settings
    } else {
        let Some(path) = nomouse_core::config::default_path() else {
            fail(app, "Cannot find the config file: HOME is not set");
            return;
        };
        match SettingsWindow::new(Some(app), path, Rc::clone(daemon)) {
            Ok(settings) => {
                window.replace(Some(Rc::clone(&settings)));
                settings
            }
            Err(error) => {
                fail(app, &error);
                return;
            }
        }
    };
    if !args.no_daemon {
        let state = Rc::clone(&settings.state);
        daemon.ensure_running(move |error| state.toast(&error));
    }
    if let Some(page) = args.page {
        settings.show_page(page);
    }
    settings.window.present();
    if args.about {
        settings.show_about();
    }
}

/// Reports why the window cannot open, on stderr and in a dialog.
fn fail(app: &adw::Application, message: &str) {
    note(message);
    let dialog = gtk::MessageDialog::builder()
        .application(app)
        .modal(true)
        .message_type(gtk::MessageType::Error)
        .buttons(gtk::ButtonsType::Close)
        .text("nomouse Settings cannot open")
        .secondary_text(message)
        .build();
    dialog.connect_response(|dialog, _| dialog.close());
    dialog.present();
}
