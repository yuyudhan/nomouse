//! General: nomouse on/off, the daemon's status, start at login, updates.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use nomouse_core::update::{UpdateStatus, release_url};
use nomouse_linux::autostart::{self, TilingSession};

use crate::args::Page;
use crate::daemon::{self, DaemonStatus};
use crate::launch;
use crate::state::State;
use crate::widgets;

/// This build's version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The General page.
pub struct GeneralPage {
    /// The page.
    pub page: adw::PreferencesPage,
    /// "nomouse is on".
    pub enabled: gtk::Switch,
    /// "Start at login".
    pub autostart: gtk::Switch,
    /// The "Start at login" row; on Sway, Hyprland and i3 its subtitle is the config line
    /// that starts the daemon, since they ignore the autostart entry.
    pub autostart_row: adw::ActionRow,
    status_row: adw::ActionRow,
    start: gtk::Button,
    update_row: adw::ActionRow,
    update_button: gtk::Button,
    release_link: gtk::LinkButton,
    syncing: Cell<bool>,
}

impl GeneralPage {
    /// Builds the page.
    pub fn new(state: &Rc<State>) -> Rc<Self> {
        let page = widgets::page(Page::General);
        let daemon_group = widgets::group(
            "nomouse",
            "The daemon listens for the hotkeys and draws the labels.",
        );
        let (enabled_row, enabled) = widgets::switch_row(
            "nomouse is on",
            "Off releases the hotkeys until you switch it on again",
        );
        let status_row = widgets::row("Daemon", "Checking…");
        let start = widgets::button("Start", "Start");
        start.add_css_class("suggested-action");
        start.set_visible(false);
        status_row.add_suffix(&start);
        daemon_group.add(&enabled_row);
        daemon_group.add(&status_row);
        page.add(&daemon_group);

        let session_group = widgets::group("Session", "");
        let (autostart_row, autostart) = widgets::switch_row(
            "Start at login",
            &autostart_subtitle(TilingSession::current()),
        );
        session_group.add(&autostart_row);
        page.add(&session_group);

        let update_group = widgets::group("Updates", "");
        let update_row = widgets::row("Check for updates", &format!("nomouse {VERSION}"));
        let update_button = widgets::button("Check", "Check for updates");
        let release_link = gtk::LinkButton::with_label("", "Release notes");
        release_link.set_valign(gtk::Align::Center);
        release_link.set_visible(false);
        update_row.add_suffix(&release_link);
        update_row.add_suffix(&update_button);
        update_group.add(&update_row);
        page.add(&update_group);

        let general = Rc::new(Self {
            page,
            enabled,
            autostart,
            autostart_row,
            status_row,
            start,
            update_row,
            update_button,
            release_link,
            syncing: Cell::new(false),
        });
        general.connect(state);
        general.show_status(state.daemon.status());
        general.show_autostart();
        general
    }

    fn connect(self: &Rc<Self>, state: &Rc<State>) {
        let weak = Rc::downgrade(self);
        state.daemon.connect_status(move |status| {
            if let Some(page) = weak.upgrade() {
                page.show_status(status);
            }
        });
        let daemon = Rc::clone(&state.daemon);
        let weak = Rc::downgrade(self);
        self.enabled.connect_active_notify(move |switch| {
            if weak.upgrade().is_some_and(|page| !page.syncing.get()) {
                if switch.is_active() {
                    daemon.resume();
                } else {
                    daemon.pause();
                }
            }
        });
        let toast_state = Rc::clone(state);
        self.start.connect_clicked(move |button| {
            button.set_sensitive(false);
            if let Err(error) = daemon::start() {
                button.set_sensitive(true);
                toast_state.toast(&error);
            }
        });
        let weak = Rc::downgrade(self);
        let toast_state = Rc::clone(state);
        self.autostart.connect_active_notify(move |switch| {
            let Some(page) = weak.upgrade() else {
                return;
            };
            if page.syncing.get() {
                return;
            }
            let result = autostart::path()
                .ok_or_else(|| "HOME is not set".to_owned())
                .and_then(|path| {
                    autostart::set(&path, switch.is_active(), &launch::nomouse_program())
                        .map_err(|error| format!("Cannot change {}: {error}", path.display()))
                });
            if let Err(error) = result {
                toast_state.toast(&error);
            }
            page.show_autostart();
        });
        // The tray menu (or a file manager) can switch it too: follow the file.
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(nomouse_core::settings::WATCH_INTERVAL, move || {
            let Some(page) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            page.show_autostart();
            glib::ControlFlow::Continue
        });
        let weak = Rc::downgrade(self);
        self.update_button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.check_updates();
            }
        });
    }

    fn show_status(&self, status: DaemonStatus) {
        self.syncing.set(true);
        let (text, running, paused) = match status {
            DaemonStatus::Unknown => ("Checking…", false, false),
            DaemonStatus::NotRunning => ("Not running", false, false),
            DaemonStatus::Running { paused: false } => ("Running", true, false),
            DaemonStatus::Running { paused: true } => ("Paused", true, true),
        };
        self.status_row.set_subtitle(text);
        self.enabled.set_active(running && !paused);
        self.enabled.set_sensitive(running);
        self.start.set_visible(status == DaemonStatus::NotRunning);
        self.start.set_sensitive(true);
        self.syncing.set(false);
    }

    fn show_autostart(&self) {
        self.syncing.set(true);
        let on = autostart::path().is_some_and(|path| autostart::is_enabled(&path));
        self.autostart.set_active(on);
        self.syncing.set(false);
    }

    /// Asks GitHub for a newer release (off the main thread) and shows the answer.
    pub fn check_updates(self: &Rc<Self>) {
        self.update_button.set_sensitive(false);
        self.release_link.set_visible(false);
        self.update_row.set_subtitle("Checking…");
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(|| nomouse_core::update::check(VERSION))
                .await
                .unwrap_or_else(|_| Err("the update check stopped".to_owned()));
            let Some(page) = weak.upgrade() else {
                return;
            };
            page.update_button.set_sensitive(true);
            match result {
                Ok(status) => {
                    widgets::set_subtitle(&page.update_row, &status.message());
                    if let UpdateStatus::Available { latest, .. } = status {
                        page.release_link.set_uri(&release_url(&latest.to_string()));
                        page.release_link.set_visible(true);
                    }
                }
                Err(error) => {
                    widgets::set_subtitle(&page.update_row, &format!("Could not check: {error}"));
                }
            }
        });
    }
}

/// The "Start at login" row's subtitle: what the switch does, or on a tiling session that
/// ignores autostart entries, the line to put in its config instead.
fn autostart_subtitle(session: Option<TilingSession>) -> String {
    session.map_or_else(
        || "Run the daemon when you log in to the desktop".to_owned(),
        |session| session.advice(&launch::nomouse_program()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiling_sessions_show_the_config_line_under_start_at_login() {
        assert_eq!(
            autostart_subtitle(None),
            "Run the daemon when you log in to the desktop"
        );
        let program = launch::nomouse_program();
        assert_eq!(
            autostart_subtitle(Some(TilingSession::Sway)),
            TilingSession::Sway.advice(&program)
        );
        assert!(autostart_subtitle(Some(TilingSession::Hyprland)).contains("exec-once = "));
    }
}
