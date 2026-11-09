//! nomouse as a menu bar app, as Homerow is: the template icon in the menu bar, its menu
//! ([`nomouse_core::menu::status_menu`], the model the Linux tray draws too, plus the
//! Accessibility line), the settings window, start at login, the update check and About.
//!
//! The daemon owns one [`MenuBar`] on the main thread, tells it its state with
//! [`MenuBar::update`] and takes what the user asked for with [`MenuBar::next_command`] from its
//! event loop; menu picks that concern only the app (settings, About, start at login, the
//! update check) are carried out here.

mod pages;
mod screenshots;
mod welcome;
mod window;

use std::collections::VecDeque;
use std::process::Command as Process;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use nomouse_core::Hotkey;
use nomouse_core::menu::{ItemKind, MenuAction, MenuItem, MenuState, UpdateLine, status_menu};
use nomouse_core::settings::Section;
use nomouse_core::update::{self, UpdateStatus, release_url};
use nomouse_sys_macos::app::{App, NotMainThread};
use nomouse_sys_macos::ax;
use nomouse_sys_macos::login::{self, LoginItem};
use nomouse_sys_macos::ui::notice::Notice;
use nomouse_sys_macos::ui::status::{MenuLine, Shortcut, StatusItem};
use nomouse_sys_macos::ui::{self, UiEvent, panels};

pub use screenshots::screenshots;
pub use welcome::Onboarding;
use window::{General, Handled, Outcome, SettingsWindow};

use crate::keys;
use crate::setup::ACCESSIBILITY_SETTINGS;
use crate::shortcuts;

/// The menu bar template image: black on transparent, drawn at 36 px for 18 points.
const MENU_ICON: &[u8] = include_bytes!("../../../../assets/logo/png/nomouse-menubar-36.png");
/// The app icon, for the About panel.
const APP_ICON: &[u8] = include_bytes!("../../../../assets/logo/png/nomouse-256.png");

/// The menu's Accessibility line (menu action ids are 1 to 14).
const ACCESSIBILITY_TAG: isize = 50;

/// The tag of the first result window button, above every other control's tag.
const NOTICE_TAG: isize = 10_000;
/// The most buttons a result window has: window `n`'s button `i` reports
/// `NOTICE_TAG + n * NOTICE_BUTTONS + i`.
const NOTICE_BUTTONS: usize = 4;

/// What a result window's button does besides closing the window.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Click {
    /// Nothing more.
    Close,
    /// Put the text on the clipboard.
    Copy(String),
    /// Open the address in the browser.
    OpenUrl(String),
}

/// A result window on screen and what each of its buttons does.
#[derive(Debug)]
struct OpenNotice {
    id: usize,
    notice: Notice,
    clicks: Vec<Click>,
}

/// The tag of button `index` of result window `id`.
fn notice_tag(id: usize, index: usize) -> isize {
    let slot = id * NOTICE_BUTTONS + index;
    NOTICE_TAG + isize::try_from(slot).unwrap_or(0)
}

/// The result window and button a tag belongs to.
fn notice_at(tag: isize) -> Option<(usize, usize)> {
    let slot = usize::try_from(tag.checked_sub(NOTICE_TAG)?).ok()?;
    Some((slot / NOTICE_BUTTONS, slot % NOTICE_BUTTONS))
}

/// What the user asked of the daemon from the menu or the settings window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Release the hotkeys.
    Pause,
    /// Take them again.
    Resume,
    /// Run a mode now: [`MenuAction::Hints`], `Continuous`, `Scroll` or `Search`.
    Run(MenuAction),
    /// Run the doctor and show its report with [`MenuBar::show_report`].
    Doctor,
    /// Stop the daemon.
    Quit,
}

/// The menu bar item and the windows behind it.
#[derive(Debug)]
pub struct MenuBar {
    app: App,
    item: StatusItem,
    state: MenuState,
    trusted: bool,
    window: Option<SettingsWindow>,
    /// The result windows on screen (they never block the daemon's loop).
    notices: Vec<OpenNotice>,
    next_notice: usize,
    /// The update check running on its own thread, and whether to announce its answer.
    checking: Option<(Receiver<Result<UpdateStatus, String>>, bool)>,
    /// The hotkeys are released while a shortcut is recorded (the daemon was not paused).
    holding: bool,
    pending: VecDeque<Command>,
}

/// The menu bar's start at login state: `None` where nomouse cannot be a login item.
fn start_at_login() -> Option<bool> {
    match login::login_item() {
        LoginItem::Enabled | LoginItem::NeedsApproval => Some(true),
        LoginItem::Disabled => Some(false),
        LoginItem::Unavailable(_) => None,
    }
}

/// Asks for Accessibility with macOS's prompt (which lists nomouse) and opens its pane.
fn open_accessibility() {
    ax::is_process_trusted(true);
    if let Err(error) = Process::new("/usr/bin/open")
        .arg(ACCESSIBILITY_SETTINGS)
        .status()
    {
        tracing::warn!("cannot open System Settings: {error}");
    }
}

/// The macOS title of a mode item (its hotkey is drawn as the key equivalent).
const fn mode_title(action: MenuAction) -> Option<&'static str> {
    match action {
        MenuAction::Hints => Some("Show Hints"),
        MenuAction::Continuous => Some("Continuous Hints"),
        MenuAction::Scroll => Some("Scroll"),
        MenuAction::Search => Some("Search"),
        _ => None,
    }
}

/// The hotkey of a mode item, from the state.
fn mode_hotkey(state: &MenuState, action: MenuAction) -> Option<Hotkey> {
    let hotkeys = state.hotkeys.as_ref()?;
    match action {
        MenuAction::Hints => Some(hotkeys.hints),
        MenuAction::Continuous => Some(hotkeys.continuous),
        MenuAction::Scroll => Some(hotkeys.scroll),
        MenuAction::Search => Some(hotkeys.search),
        _ => None,
    }
}

fn shortcut(hotkey: Hotkey) -> Shortcut {
    Shortcut {
        key: shortcuts::menu_key(hotkey.key),
        modifiers: keys::modifier_flags(hotkey.modifiers),
    }
}

fn command_key(key: &str) -> Shortcut {
    Shortcut {
        key: key.to_owned(),
        modifiers: keys::modifier_flags(nomouse_core::Modifiers::META),
    }
}

/// The menu lines for `items` (from [`status_menu`]), with the Accessibility line after the
/// status line.
fn lines(state: &MenuState, items: &[MenuItem], trusted: bool) -> Vec<MenuLine> {
    let mut lines = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let action = item.action();
        lines.push(match item.kind {
            ItemKind::Separator => MenuLine::Separator,
            ItemKind::Info => MenuLine::Item {
                tag: 0,
                title: item.label.clone(),
                enabled: false,
                checked: false,
                shortcut: None,
            },
            ItemKind::Command(action) | ItemKind::Toggle(action, _) => MenuLine::Item {
                tag: isize::try_from(action.id()).unwrap_or(0),
                title: mode_title(action).map_or_else(|| item.label.clone(), ToOwned::to_owned),
                enabled: item.enabled,
                checked: matches!(item.kind, ItemKind::Toggle(_, true)),
                shortcut: match action {
                    MenuAction::Settings => Some(command_key(",")),
                    MenuAction::Quit => Some(command_key("q")),
                    _ => mode_hotkey(state, action).map(shortcut),
                },
            },
        });
        if index == 0 && action.is_none() {
            lines.push(MenuLine::Item {
                tag: if trusted { 0 } else { ACCESSIBILITY_TAG },
                title: if trusted {
                    "Accessibility: allowed".to_owned()
                } else {
                    "Accessibility: not allowed. Allow…".to_owned()
                },
                enabled: !trusted,
                checked: false,
                shortcut: None,
            });
        }
    }
    lines
}

impl MenuBar {
    /// Puts the item in the menu bar showing `state`.
    ///
    /// # Errors
    /// [`NotMainThread`] off the main thread.
    pub fn new(state: MenuState) -> Result<Self, NotMainThread> {
        let app = App::new()?;
        let mtm = app.main_thread();
        ui::install_main_menu(mtm);
        let item = StatusItem::new(mtm, MENU_ICON, "nomouse");
        let menu_bar = Self {
            app,
            item,
            state: MenuState {
                start_at_login: start_at_login(),
                ..state
            },
            trusted: ax::is_process_trusted(false),
            window: None,
            notices: Vec::new(),
            next_notice: 0,
            checking: None,
            holding: false,
            pending: VecDeque::new(),
        };
        menu_bar.redraw();
        Ok(menu_bar)
    }

    /// The state the menu shows.
    pub const fn state(&self) -> &MenuState {
        &self.state
    }

    /// Changes the state and redraws the menu.
    pub fn update(&mut self, change: impl FnOnce(&mut MenuState)) {
        change(&mut self.state);
        // The recorder's hold is the daemon's pause: the menu shows it as running.
        self.redraw();
    }

    fn redraw(&self) {
        let shown = MenuState {
            paused: self.state.paused && !self.holding,
            ..self.state.clone()
        };
        self.item
            .set_menu(&lines(&shown, &status_menu(&shown), self.trusted));
        self.item.set_dimmed(shown.paused || !shown.active);
        if let Some(window) = &self.window {
            window.set_general(&self.general());
        }
    }

    fn general(&self) -> General {
        General {
            trusted: self.trusted,
            login: login::login_item(),
            update: self.state.update.clone(),
        }
    }

    /// Re-reads what can change outside nomouse (the Accessibility grant, the login item).
    pub fn refresh(&mut self) {
        let trusted = ax::is_process_trusted(false);
        let login = start_at_login();
        if trusted != self.trusted || login != self.state.start_at_login {
            self.trusted = trusted;
            self.state.start_at_login = login;
            self.redraw();
        }
    }

    /// Runs the app's windows and menu for up to `timeout`, for a daemon with no backend to
    /// wait on.
    pub fn pump(&self, timeout: Duration) {
        self.app.pump(timeout.as_secs_f64());
    }

    /// Whether the system appearance is dark.
    fn dark(&self) -> bool {
        self.app.prefers_dark()
    }

    /// Opens the settings window on `page`, in front.
    pub fn open_settings(&mut self, page: Option<usize>) {
        if self.window.is_none() {
            match SettingsWindow::new(self.app.main_thread()) {
                Ok(window) => self.window = Some(window),
                Err(error) => {
                    self.notify(
                        "nomouse cannot open its settings",
                        &error.to_string(),
                        None,
                        vec![("OK", Click::Close)],
                        true,
                    );
                    return;
                }
            }
        }
        let dark = self.dark();
        let general = self.general();
        if let Some(window) = &mut self.window {
            window.set_general(&general);
            window.show(page, dark);
        }
    }

    /// Opens the settings window on the page of `section`.
    pub fn open_section(&mut self, section: Section) {
        self.open_settings(Some(pages::page_of(section)));
    }

    /// The settings window's selected page and whether it is on screen.
    pub fn settings_page(&self) -> Option<usize> {
        self.window
            .as_ref()
            .filter(|window| window.is_visible())
            .map(SettingsWindow::page_index)
    }

    /// Closes the settings window.
    pub fn close_settings(&self) {
        if let Some(window) = &self.window {
            window.close();
        }
    }

    /// The open settings window's controls, to drive them as a user would (tests).
    #[cfg(feature = "testing")]
    pub fn settings_form(&self) -> Option<&nomouse_sys_macos::ui::form::Form> {
        self.window.as_ref().map(SettingsWindow::form)
    }

    /// The tag of the settings window's control for configuration key `key` (tests).
    #[cfg(feature = "testing")]
    pub fn settings_tag(key: &str) -> Option<isize> {
        nomouse_core::settings::field_for(key).map(pages::field_tag)
    }

    /// Shows the doctor's report in a window of its own. The window does not block: this
    /// returns at once and the daemon goes on serving its hotkeys and configuration reloads
    /// while the report is open.
    pub fn show_report(&mut self, title: &str, text: &str) {
        self.notify(
            title,
            "",
            Some(text),
            vec![("OK", Click::Close), ("Copy", Click::Copy(text.to_owned()))],
            false,
        );
    }

    /// Shows a result window with `message`, an optional monospaced `report` and a button per
    /// `(title, click)`, and returns at once.
    fn notify(
        &mut self,
        title: &str,
        message: &str,
        report: Option<&str>,
        buttons: Vec<(&str, Click)>,
        warning: bool,
    ) {
        let id = self.next_notice;
        self.next_notice += 1;
        let (titles, clicks): (Vec<&str>, Vec<Click>) =
            buttons.into_iter().take(NOTICE_BUTTONS).unzip();
        let shown: Vec<(isize, &str)> = titles
            .iter()
            .enumerate()
            .map(|(index, title)| (notice_tag(id, index), *title))
            .collect();
        let notice = Notice::show(
            self.app.main_thread(),
            title,
            message,
            report,
            &shown,
            warning,
        );
        self.notices.push(OpenNotice { id, notice, clicks });
    }

    /// Carries out a click on a result window's button and closes the window; whether `tag`
    /// belongs to one.
    fn carry_out(&mut self, tag: isize) -> bool {
        let Some((id, index)) = notice_at(tag) else {
            return false;
        };
        let Some(position) = self.notices.iter().position(|open| open.id == id) else {
            // The window was closed by then.
            return true;
        };
        let open = self.notices.remove(position);
        match open.clicks.get(index) {
            Some(Click::Copy(text)) => panels::copy(text),
            Some(Click::OpenUrl(url)) => {
                panels::open_url(url);
            }
            Some(Click::Close) | None => {}
        }
        true
    }

    fn about(&self) {
        panels::about(
            self.app.main_thread(),
            "nomouse",
            env!("CARGO_PKG_VERSION"),
            APP_ICON,
            "Click anything on screen without touching the mouse.\nhttps://github.com/yuyudhan/nomouse\nMIT or Apache-2.0",
        );
    }

    fn set_start_at_login(&mut self, on: bool) {
        if let Err(error) = login::set_login_item(on) {
            self.notify(
                "Start at login could not be changed",
                &error,
                None,
                vec![("OK", Click::Close)],
                true,
            );
        }
        if login::login_item() == LoginItem::NeedsApproval {
            login::open_login_items_settings();
        }
        self.state.start_at_login = start_at_login();
        self.redraw();
    }

    /// Starts the update check on its own thread; `announce` shows its answer in an alert.
    fn check_for_updates(&mut self, announce: bool) {
        if self.checking.is_some() {
            return;
        }
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            // The receiver is gone only when the app quit meanwhile.
            let _ = send.send(update::check(env!("CARGO_PKG_VERSION")));
        });
        self.checking = Some((receive, announce));
        self.state.update = UpdateLine::Checking;
        self.redraw();
    }

    fn poll_update(&mut self) {
        let Some((receive, announce)) = &self.checking else {
            return;
        };
        let answer = match receive.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the check stopped".to_owned()),
        };
        let announce = *announce;
        self.checking = None;
        self.state.update = match &answer {
            Ok(status) => UpdateLine::Checked(status.clone()),
            Err(error) => UpdateLine::Failed(error.clone()),
        };
        self.redraw();
        if announce {
            self.announce(answer);
        }
    }

    /// Shows the update check's answer in a result window.
    fn announce(&mut self, answer: Result<UpdateStatus, String>) {
        match answer {
            Ok(status @ UpdateStatus::Available { latest, .. }) => self.notify(
                &status.message(),
                "Download it from its release page, or run the installer again.",
                None,
                vec![
                    (
                        "Open Release Page",
                        Click::OpenUrl(release_url(&latest.to_string())),
                    ),
                    ("Later", Click::Close),
                ],
                false,
            ),
            Ok(status @ UpdateStatus::UpToDate(_)) => {
                self.notify(
                    &status.message(),
                    "",
                    None,
                    vec![("OK", Click::Close)],
                    false,
                );
            }
            Err(error) => self.notify(
                "The update check failed",
                &error,
                None,
                vec![("OK", Click::Close)],
                true,
            ),
        }
    }

    /// Releases the hotkeys while a shortcut is recorded (unless paused), or takes them back.
    fn hold(&mut self, on: bool) {
        if on && !self.holding && !self.state.paused {
            self.holding = true;
            self.pending.push_back(Command::Pause);
        } else if !on && self.holding {
            self.holding = false;
            self.pending.push_back(Command::Resume);
        }
    }

    fn outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Hold(on) => self.hold(on),
            Outcome::Doctor => self.pending.push_back(Command::Doctor),
            Outcome::CheckForUpdates => self.check_for_updates(true),
            Outcome::About => self.about(),
            Outcome::StartAtLogin(on) => self.set_start_at_login(on),
            Outcome::OpenAccessibility => open_accessibility(),
        }
    }

    fn menu(&mut self, action: MenuAction) {
        tracing::debug!("menu: {action:?}");
        match action {
            MenuAction::Pause => self.pending.push_back(Command::Pause),
            MenuAction::Resume => self.pending.push_back(Command::Resume),
            MenuAction::Hints
            | MenuAction::Continuous
            | MenuAction::Scroll
            | MenuAction::Search => {
                self.pending.push_back(Command::Run(action));
            }
            MenuAction::Settings => self.open_settings(None),
            MenuAction::Warning => self.open_settings(Some(pages::GENERAL_PAGE)),
            MenuAction::Doctor => self.pending.push_back(Command::Doctor),
            MenuAction::StartAtLogin => {
                let on = self.state.start_at_login != Some(true);
                self.set_start_at_login(on);
            }
            MenuAction::CheckForUpdates => self.check_for_updates(true),
            MenuAction::OpenRelease => {
                if let UpdateLine::Checked(UpdateStatus::Available { latest, .. }) =
                    &self.state.update
                {
                    panels::open_url(&release_url(&latest.to_string()));
                }
            }
            MenuAction::About => self.about(),
            MenuAction::Quit => self.pending.push_back(Command::Quit),
        }
    }

    /// The oldest thing the user asked of the daemon, if any. Also carries out what concerns
    /// only the app and follows the config file for the settings window; call it every few
    /// milliseconds.
    pub fn next_command(&mut self) -> Option<Command> {
        self.poll_update();
        self.notices.retain(|open| open.notice.is_visible());
        let dark = self.dark();
        if let Some(outcome) = self.window.as_mut().and_then(|window| window.tick(dark)) {
            self.outcome(outcome);
        }
        for event in ui::take_events() {
            let handled = self
                .window
                .as_mut()
                .map_or(Handled::NotMine, |window| window.handle(&event, dark));
            match handled {
                Handled::Done => continue,
                Handled::Outcome(outcome) => {
                    self.outcome(outcome);
                    continue;
                }
                Handled::NotMine => {}
            }
            match event {
                UiEvent::Action(tag) if self.carry_out(tag) => {}
                UiEvent::Action(ACCESSIBILITY_TAG) => open_accessibility(),
                UiEvent::Action(tag) => {
                    if let Some(action) = i32::try_from(tag).ok().and_then(MenuAction::from_id) {
                        self.menu(action);
                    }
                }
                UiEvent::Key { .. } => {}
            }
        }
        self.pending.pop_front()
    }
}

/// Drivers that work the result windows as a user would, for the UI test.
#[cfg(feature = "testing")]
impl MenuBar {
    /// How many result windows are on screen.
    pub fn notice_count(&self) -> usize {
        self.notices
            .iter()
            .filter(|open| open.notice.is_visible())
            .count()
    }

    /// The report text of the newest result window that has one.
    pub fn notice_report(&self) -> Option<String> {
        self.notices
            .iter()
            .rev()
            .find_map(|open| open.notice.report_text())
    }

    /// Clicks the button titled `title` of the newest result window that has one, as a user
    /// would; whether there was one. [`MenuBar::next_command`] carries the click out.
    pub fn press_notice_button(&self, title: &str) -> bool {
        self.notices.iter().rev().any(|open| {
            open.notice
                .button_titles()
                .iter()
                .position(|button| button == title)
                .is_some_and(|index| open.notice.click(notice_tag(open.id, index)))
        })
    }

    /// Picks `action` from the menu, as the menu item does.
    pub fn choose(&mut self, action: MenuAction) {
        self.menu(action);
    }

    /// Shows the update check's `answer`, as when the check finishes.
    pub fn announce_update(&mut self, answer: Result<UpdateStatus, String>) {
        self.announce(answer);
    }
}

#[cfg(test)]
mod tests {
    use nomouse_core::config::HotkeysConfig;

    use super::*;

    fn titles(lines: &[MenuLine]) -> Vec<&str> {
        lines
            .iter()
            .map(|line| match line {
                MenuLine::Item { title, .. } => title.as_str(),
                MenuLine::Separator => "-",
            })
            .collect()
    }

    fn state() -> MenuState {
        MenuState {
            version: "0.1.4".to_owned(),
            active: true,
            hotkeys: Some(HotkeysConfig::default()),
            start_at_login: Some(true),
            ..MenuState::default()
        }
    }

    #[test]
    fn the_menu_has_the_accessibility_line_and_macos_key_equivalents() {
        let state = state();
        let lines = lines(&state, &status_menu(&state), false);
        assert_eq!(
            titles(&lines),
            [
                "nomouse 0.1.4: on",
                "Accessibility: not allowed. Allow…",
                "-",
                "Pause nomouse",
                "Show Hints",
                "Continuous Hints",
                "Scroll",
                "Search",
                "-",
                "Settings…",
                "Run doctor…",
                "-",
                "Start at login",
                "Check for updates…",
                "About nomouse",
                "-",
                "Quit nomouse",
            ]
        );
        let MenuLine::Item {
            tag,
            enabled,
            shortcut,
            ..
        } = &lines[4]
        else {
            panic!("not an item");
        };
        assert_eq!(*tag, isize::try_from(MenuAction::Hints.id()).unwrap());
        assert!(enabled);
        let hints = HotkeysConfig::default().hints;
        assert_eq!(shortcut.as_ref(), Some(&super::shortcut(hints)));
        assert!(matches!(
            &lines[1],
            MenuLine::Item {
                tag: ACCESSIBILITY_TAG,
                enabled: true,
                ..
            }
        ));
        assert!(matches!(&lines[12], MenuLine::Item { checked: true, .. }));
    }

    #[test]
    fn a_granted_permission_is_an_informative_line() {
        let state = state();
        let lines = lines(&state, &status_menu(&state), true);
        assert!(matches!(
            &lines[1],
            MenuLine::Item { tag: 0, enabled: false, title, .. } if title == "Accessibility: allowed"
        ));
    }
}
