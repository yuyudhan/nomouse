//! The daemon's desktop presence. On Linux: the tray icon and its menu, the D-Bus control
//! interface (`nomouse_linux::service`), start at login, the update check, and opening the
//! settings window. On macOS: the menu bar item with its menu and the settings window in this
//! process (`nomouse_macos::menubar`). Other systems get a stand-in that does nothing.

use crate::Request;
use nomouse_core::Config;

#[cfg(target_os = "linux")]
pub(crate) use linux::Desktop;

#[cfg(target_os = "macos")]
pub(crate) use macos::Desktop;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) use other::Desktop;

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    use nomouse_core::ActionKind;
    use nomouse_core::menu::{MenuAction, MenuState, UpdateLine};
    use nomouse_core::update::{self, UpdateStatus, release_url};
    use nomouse_linux::autostart;
    use nomouse_linux::service::{self, Service, TrayHost};
    use nomouse_linux::shortcuts::Shortcuts;

    use super::{Config, Request};
    use crate::run::Mode;

    /// The tray and control interface, when the session bus is there.
    pub(crate) struct Desktop {
        service: Option<Service>,
        autostart: Option<PathBuf>,
        action: ActionKind,
    }

    impl Desktop {
        /// Puts nomouse on the session bus with `config`'s hotkeys; `problem` is why the
        /// backend did not start (the menu then only offers settings, doctor and quit).
        pub(crate) fn start(config: &Config, problem: Option<&str>) -> Self {
            let session = Shortcuts::current();
            let autostart = autostart::path();
            let state = MenuState {
                version: env!("CARGO_PKG_VERSION").to_owned(),
                paused: false,
                active: problem.is_none(),
                warning: session.warning().or_else(|| problem.map(ToOwned::to_owned)),
                hotkeys: session.grabs_keys().then(|| config.hotkeys.clone()),
                start_at_login: autostart.as_deref().map(autostart::is_enabled),
                update: UpdateLine::NotChecked,
            };
            let service = match Service::start(state) {
                Ok(service) => Some(service),
                Err(error) => {
                    tracing::warn!("no tray icon or D-Bus control: {error}");
                    None
                }
            };
            Self {
                service,
                autostart,
                action: config.hints.action,
            }
        }

        /// Whether the daemon is on the session bus (and can run without a backend, showing
        /// only its menu).
        pub(crate) const fn is_up(&self) -> bool {
            self.service.is_some()
        }

        /// Whether a tray host shows the icon.
        pub(crate) fn tray_shown(&self) -> bool {
            self.service
                .as_ref()
                .is_some_and(|service| service.tray_host() == TrayHost::Shown)
        }

        /// The next request for the daemon's loop. Menu picks that only concern the desktop
        /// (settings, doctor, start at login, the update check) are carried out here.
        pub(crate) fn next(&self) -> Option<Request> {
            let service = self.service.as_ref()?;
            while let Some(command) = service.next_command() {
                let action = match command {
                    service::Command::Reload => return Some(Request::Reload),
                    service::Command::Menu(action) => action,
                };
                let mode = |mode| Some(Request::Run(mode));
                let ask = match action {
                    MenuAction::Pause => Some(Request::Pause),
                    MenuAction::Resume => Some(Request::Resume),
                    MenuAction::Quit => Some(Request::Quit),
                    MenuAction::Hints => mode(Mode::Once(self.action)),
                    MenuAction::Continuous => mode(Mode::Continuous(self.action)),
                    MenuAction::Scroll => mode(Mode::Scroll),
                    MenuAction::Search => mode(Mode::Search),
                    MenuAction::Settings => self.open_settings(&[]),
                    MenuAction::Doctor | MenuAction::Warning => {
                        self.open_settings(&["--page", "doctor"])
                    }
                    MenuAction::About => self.open_settings(&["--about"]),
                    MenuAction::StartAtLogin => self.toggle_start_at_login(service),
                    MenuAction::CheckForUpdates => check_for_updates(service),
                    MenuAction::OpenRelease => open_release(service),
                };
                if ask.is_some() {
                    return ask;
                }
            }
            None
        }

        /// Starts the settings window without waiting for it, so the daemon never blocks; a
        /// thread reaps it when it exits.
        fn open_settings(&self, args: &[&str]) -> Option<Request> {
            match crate::launch::spawn_settings(args) {
                Ok((mut child, _, _)) => {
                    std::thread::spawn(move || child.wait());
                }
                Err(error) => {
                    tracing::warn!("settings window: {error}");
                    if let Some(service) = &self.service {
                        service.notify(
                            "nomouse settings could not open",
                            &format!(
                                "{error}. Reinstall nomouse with its installer or package to \
                                 get the settings window."
                            ),
                        );
                    }
                }
            }
            None
        }

        fn toggle_start_at_login(&self, service: &Service) -> Option<Request> {
            let path = self.autostart.as_deref()?;
            let on = !autostart::is_enabled(path);
            let program = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("nomouse"));
            match autostart::set(path, on, &program) {
                Ok(()) => service.update(|state| state.start_at_login = Some(on)),
                Err(error) => {
                    tracing::warn!("start at login: {}: {error}", path.display());
                    service.notify(
                        "Start at login failed",
                        &format!("{}: {error}", path.display()),
                    );
                }
            }
            None
        }

        /// Follows a running state change: paused, the backend up or down, new hotkeys.
        pub(crate) fn show(&mut self, config: &Config, paused: bool, problem: Option<&str>) {
            self.action = config.hints.action;
            let Some(service) = &self.service else {
                return;
            };
            let session = Shortcuts::current();
            let start_at_login = self.autostart.as_deref().map(autostart::is_enabled);
            service.update(|state| {
                state.paused = paused;
                state.active = problem.is_none();
                state.warning = session.warning().or_else(|| problem.map(ToOwned::to_owned));
                state.hotkeys = session.grabs_keys().then(|| config.hotkeys.clone());
                state.start_at_login = start_at_login;
            });
        }

        /// Picks up a start-at-login switch made elsewhere (the settings window).
        pub(crate) fn refresh(&self) {
            let (Some(service), Some(path)) = (&self.service, self.autostart.as_deref()) else {
                return;
            };
            let on = autostart::is_enabled(path);
            if service.state().start_at_login != Some(on) {
                service.update(|state| state.start_at_login = Some(on));
            }
        }
    }

    /// Runs the update check on its own thread; the menu shows its progress and answer, and a
    /// notification tells the result.
    fn check_for_updates(service: &Service) -> Option<Request> {
        service.update(|state| state.update = UpdateLine::Checking);
        let service = service.clone();
        std::thread::spawn(move || {
            let line = match update::check(env!("CARGO_PKG_VERSION")) {
                Ok(status) => {
                    let body = match &status {
                        UpdateStatus::Available { latest, .. } => format!(
                            "Pick the menu item to read what changed; run the installer again to update. {}",
                            release_url(&latest.to_string())
                        ),
                        UpdateStatus::UpToDate(_) => String::new(),
                    };
                    service.notify(&status.message(), &body);
                    UpdateLine::Checked(status)
                }
                Err(error) => {
                    service.notify("Update check failed", &error);
                    UpdateLine::Failed(error)
                }
            };
            service.update(|state| state.update = line);
        });
        None
    }

    /// Opens the newer release's page in the browser.
    fn open_release(service: &Service) -> Option<Request> {
        if let UpdateLine::Checked(UpdateStatus::Available { latest, .. }) = service.state().update
        {
            let url = release_url(&latest.to_string());
            let opened = Command::new("xdg-open")
                .arg(&url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            match opened {
                // Reap it when it exits, off the event loop.
                Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
                Err(error) => tracing::warn!("xdg-open {url}: {error}"),
            }
        }
        None
    }

    /// The doctor's tray line.
    pub(crate) fn doctor_line() -> (String, Result<String, String>) {
        ("tray icon".to_owned(), Ok(service::doctor_line()))
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::doctor_line;

#[cfg(target_os = "macos")]
mod macos {
    use std::time::Duration;

    use nomouse_core::ActionKind;
    use nomouse_core::menu::{MenuAction, MenuState, UpdateLine};
    use nomouse_macos::menubar::{Command, MenuBar};

    use super::{Config, Request};
    use crate::run::Mode;

    /// The menu bar item, when the daemon runs on the main thread.
    pub(crate) struct Desktop {
        menu_bar: Option<MenuBar>,
        action: ActionKind,
    }

    impl Desktop {
        /// Puts nomouse in the menu bar with `config`'s hotkeys; `problem` is why the backend
        /// did not start.
        pub(crate) fn start(config: &Config, problem: Option<&str>) -> Self {
            let state = MenuState {
                version: env!("CARGO_PKG_VERSION").to_owned(),
                paused: false,
                active: problem.is_none(),
                warning: problem.map(ToOwned::to_owned),
                hotkeys: Some(config.hotkeys.clone()),
                start_at_login: None,
                update: UpdateLine::NotChecked,
            };
            let menu_bar = match MenuBar::new(state) {
                Ok(menu_bar) => Some(menu_bar),
                Err(error) => {
                    tracing::warn!("no menu bar item: {error}");
                    None
                }
            };
            Self {
                menu_bar,
                action: config.hints.action,
            }
        }

        /// Whether the menu bar item is up (the daemon can then run without a backend).
        pub(crate) const fn is_up(&self) -> bool {
            self.menu_bar.is_some()
        }

        /// Whether the menu bar shows the item.
        pub(crate) const fn tray_shown(&self) -> bool {
            self.menu_bar.is_some()
        }

        /// The next request for the daemon's loop; the menu bar app carries out what concerns
        /// only itself (settings, About, start at login, the update check). It also services
        /// the menu and windows, which share the daemon's thread with the backend; without a
        /// backend this is all that runs them.
        pub(crate) fn next(&mut self) -> Option<Request> {
            let menu_bar = self.menu_bar.as_mut()?;
            menu_bar.pump(Duration::ZERO);
            while let Some(command) = menu_bar.next_command() {
                let mode = |mode| Some(Request::Run(mode));
                let ask = match command {
                    Command::Pause => Some(Request::Pause),
                    Command::Resume => Some(Request::Resume),
                    Command::Quit => Some(Request::Quit),
                    Command::Run(MenuAction::Continuous) => mode(Mode::Continuous(self.action)),
                    Command::Run(MenuAction::Scroll) => mode(Mode::Scroll),
                    Command::Run(MenuAction::Search) => mode(Mode::Search),
                    Command::Run(_) => mode(Mode::Once(self.action)),
                    Command::Doctor => {
                        match crate::doctor_report() {
                            Ok((report, failed)) => menu_bar.show_report(
                                if failed {
                                    "nomouse doctor found problems"
                                } else {
                                    "nomouse doctor: all checks passed"
                                },
                                &report,
                            ),
                            Err(error) => menu_bar.show_report("nomouse doctor failed", &error),
                        }
                        None
                    }
                };
                if ask.is_some() {
                    return ask;
                }
            }
            None
        }

        /// Follows a running state change: paused, the backend up or down, new hotkeys.
        pub(crate) fn show(&mut self, config: &Config, paused: bool, problem: Option<&str>) {
            self.action = config.hints.action;
            if let Some(menu_bar) = &mut self.menu_bar {
                menu_bar.update(|state| {
                    state.paused = paused;
                    state.active = problem.is_none();
                    state.warning = problem.map(ToOwned::to_owned);
                    state.hotkeys = Some(config.hotkeys.clone());
                });
            }
        }

        /// Picks up what changed outside nomouse: the Accessibility grant, the login item.
        pub(crate) fn refresh(&mut self) {
            if let Some(menu_bar) = &mut self.menu_bar {
                menu_bar.refresh();
            }
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod other {
    use std::convert::Infallible;

    use super::{Config, Request};

    /// No desktop integration on this system yet: there is never a service.
    pub(crate) struct Desktop {
        service: Option<Infallible>,
    }

    impl Desktop {
        pub(crate) const fn start(_config: &Config, _problem: Option<&str>) -> Self {
            Self { service: None }
        }

        pub(crate) const fn is_up(&self) -> bool {
            self.service.is_some()
        }

        pub(crate) const fn tray_shown(&self) -> bool {
            self.service.is_some()
        }

        pub(crate) fn next(&self) -> Option<Request> {
            self.service.map(|never| match never {})
        }

        pub(crate) fn show(&mut self, _config: &Config, _paused: bool, _problem: Option<&str>) {
            if let Some(never) = self.service {
                match never {}
            }
        }

        pub(crate) fn refresh(&self) {
            if let Some(never) = self.service {
                match never {}
            }
        }
    }
}
