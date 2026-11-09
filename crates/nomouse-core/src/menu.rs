//! The status menu (the Linux tray icon's menu, the macOS menu-bar item's menu): what it lists
//! for the daemon's state, independent of how a backend draws it.

use crate::config::HotkeysConfig;
use crate::update::UpdateStatus;

/// What picking a menu item asks the daemon to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuAction {
    /// Release the hotkeys until resumed.
    Pause,
    /// Grab the hotkeys again.
    Resume,
    /// Show hints now.
    Hints,
    /// Continuous hints now.
    Continuous,
    /// Scroll mode now.
    Scroll,
    /// Search now.
    Search,
    /// Open the settings window.
    Settings,
    /// Open the settings window's doctor page.
    Doctor,
    /// Explain the session warning (opens the doctor page).
    Warning,
    /// Switch start at login.
    StartAtLogin,
    /// Check for a newer release.
    CheckForUpdates,
    /// Open the newer release's page.
    OpenRelease,
    /// Show the About window.
    About,
    /// Stop the daemon.
    Quit,
}

impl MenuAction {
    /// Every action, in a fixed order: a stable numeric id for menu protocols is its index.
    pub const ALL: [Self; 14] = [
        Self::Pause,
        Self::Resume,
        Self::Hints,
        Self::Continuous,
        Self::Scroll,
        Self::Search,
        Self::Settings,
        Self::Doctor,
        Self::Warning,
        Self::StartAtLogin,
        Self::CheckForUpdates,
        Self::OpenRelease,
        Self::About,
        Self::Quit,
    ];

    /// The action's stable id (1-based, so 0 stays free for a menu root).
    pub fn id(self) -> i32 {
        Self::ALL
            .iter()
            .position(|action| *action == self)
            .and_then(|index| i32::try_from(index + 1).ok())
            .unwrap_or(0)
    }

    /// The action with `id`.
    pub fn from_id(id: i32) -> Option<Self> {
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        Self::ALL.get(index).copied()
    }
}

/// The update check's state, for its menu line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateLine {
    /// Not checked since the daemon started.
    #[default]
    NotChecked,
    /// A check is running.
    Checking,
    /// The last check's answer.
    Checked(UpdateStatus),
    /// The last check failed.
    Failed(String),
}

/// The daemon state the menu shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuState {
    /// This build's version.
    pub version: String,
    /// Hotkeys released by Pause.
    pub paused: bool,
    /// The backend runs (false: the session cannot run nomouse; only the menu is up).
    pub active: bool,
    /// A one-line session problem (GNOME on Wayland and the like).
    pub warning: Option<String>,
    /// The chords nomouse grabs itself, or `None` where the compositor binds them.
    pub hotkeys: Option<HotkeysConfig>,
    /// Whether start at login is on, or `None` where nomouse cannot set it.
    pub start_at_login: Option<bool>,
    /// The update check.
    pub update: UpdateLine,
}

/// What kind of row a [`MenuItem`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A line between groups.
    Separator,
    /// Text that does nothing (a status line).
    Info,
    /// A command.
    Command(MenuAction),
    /// A command with a check mark showing `checked`.
    Toggle(MenuAction, bool),
}

/// One menu row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// The row's kind (and action).
    pub kind: ItemKind,
    /// Its text (empty for a separator).
    pub label: String,
    /// Whether it can be picked.
    pub enabled: bool,
}

impl MenuItem {
    fn separator() -> Self {
        Self {
            kind: ItemKind::Separator,
            label: String::new(),
            enabled: true,
        }
    }

    fn info(label: impl Into<String>) -> Self {
        Self {
            kind: ItemKind::Info,
            label: label.into(),
            enabled: false,
        }
    }

    fn command(action: MenuAction, label: impl Into<String>, enabled: bool) -> Self {
        Self {
            kind: ItemKind::Command(action),
            label: label.into(),
            enabled,
        }
    }

    /// The action picking it runs, if any.
    pub const fn action(&self) -> Option<MenuAction> {
        match self.kind {
            ItemKind::Command(action) | ItemKind::Toggle(action, _) => Some(action),
            ItemKind::Separator | ItemKind::Info => None,
        }
    }
}

/// The menu for `state`: status, the session warning, pause or resume, the modes with their
/// hotkeys, settings and doctor, start at login, the update check, About and Quit.
pub fn status_menu(state: &MenuState) -> Vec<MenuItem> {
    let status = if !state.active {
        "not running in this session"
    } else if state.paused {
        "paused"
    } else {
        "on"
    };
    let mut items = vec![MenuItem::info(format!(
        "nomouse {}: {status}",
        state.version
    ))];
    if let Some(warning) = &state.warning {
        items.push(MenuItem::command(
            MenuAction::Warning,
            format!("⚠ {warning}"),
            true,
        ));
    }
    items.push(MenuItem::separator());
    if state.paused {
        items.push(MenuItem::command(
            MenuAction::Resume,
            "Resume nomouse",
            state.active,
        ));
    } else {
        items.push(MenuItem::command(
            MenuAction::Pause,
            "Pause nomouse",
            state.active,
        ));
    }
    let runnable = state.active && !state.paused;
    let modes = [
        (
            MenuAction::Hints,
            "Show hints",
            state.hotkeys.as_ref().map(|h| h.hints),
        ),
        (
            MenuAction::Continuous,
            "Continuous hints",
            state.hotkeys.as_ref().map(|h| h.continuous),
        ),
        (
            MenuAction::Scroll,
            "Scroll",
            state.hotkeys.as_ref().map(|h| h.scroll),
        ),
        (
            MenuAction::Search,
            "Search",
            state.hotkeys.as_ref().map(|h| h.search),
        ),
    ];
    for (action, name, hotkey) in modes {
        let label = match hotkey {
            Some(hotkey) => format!("{name}    {}", hotkey.human()),
            None => name.to_owned(),
        };
        items.push(MenuItem::command(action, label, runnable));
    }
    items.push(MenuItem::separator());
    items.push(MenuItem::command(MenuAction::Settings, "Settings…", true));
    items.push(MenuItem::command(MenuAction::Doctor, "Run doctor…", true));
    items.push(MenuItem::separator());
    if let Some(on) = state.start_at_login {
        items.push(MenuItem {
            kind: ItemKind::Toggle(MenuAction::StartAtLogin, on),
            label: "Start at login".to_owned(),
            enabled: true,
        });
    }
    items.push(match &state.update {
        UpdateLine::NotChecked => {
            MenuItem::command(MenuAction::CheckForUpdates, "Check for updates…", true)
        }
        UpdateLine::Checking => MenuItem::info("Checking for updates…"),
        UpdateLine::Checked(status @ UpdateStatus::Available { .. }) => {
            MenuItem::command(MenuAction::OpenRelease, status.message(), true)
        }
        UpdateLine::Checked(status @ UpdateStatus::UpToDate(_)) => {
            MenuItem::command(MenuAction::CheckForUpdates, status.message(), true)
        }
        UpdateLine::Failed(_) => MenuItem::command(
            MenuAction::CheckForUpdates,
            "Update check failed: try again",
            true,
        ),
    });
    items.push(MenuItem::command(MenuAction::About, "About nomouse", true));
    items.push(MenuItem::separator());
    items.push(MenuItem::command(MenuAction::Quit, "Quit nomouse", true));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::Version;

    fn state() -> MenuState {
        MenuState {
            version: "0.1.3".to_owned(),
            active: true,
            hotkeys: Some(HotkeysConfig::default()),
            start_at_login: Some(false),
            ..MenuState::default()
        }
    }

    fn labels(items: &[MenuItem]) -> Vec<&str> {
        items.iter().map(|item| item.label.as_str()).collect()
    }

    fn find(items: &[MenuItem], action: MenuAction) -> Option<&MenuItem> {
        items.iter().find(|item| item.action() == Some(action))
    }

    #[test]
    fn ids_round_trip_and_zero_is_free() {
        for action in MenuAction::ALL {
            assert!(action.id() > 0);
            assert_eq!(MenuAction::from_id(action.id()), Some(action));
        }
        assert_eq!(MenuAction::from_id(0), None);
        assert_eq!(MenuAction::from_id(-1), None);
        assert_eq!(MenuAction::from_id(99), None);
    }

    #[test]
    fn running_menu_lists_hotkeys_settings_and_quit() {
        let items = status_menu(&state());
        assert_eq!(
            labels(&items),
            [
                "nomouse 0.1.3: on",
                "",
                "Pause nomouse",
                "Show hints    Ctrl+Space",
                "Continuous hints    Ctrl+Shift+Space",
                "Scroll    Ctrl+Alt+Space",
                "Search    Ctrl+Alt+/",
                "",
                "Settings…",
                "Run doctor…",
                "",
                "Start at login",
                "Check for updates…",
                "About nomouse",
                "",
                "Quit nomouse",
            ]
        );
        assert!(find(&items, MenuAction::Hints).is_some_and(|item| item.enabled));
        assert_eq!(
            find(&items, MenuAction::StartAtLogin).map(|item| item.kind),
            Some(ItemKind::Toggle(MenuAction::StartAtLogin, false))
        );
    }

    #[test]
    fn paused_menu_offers_resume_and_disables_the_modes() {
        let items = status_menu(&MenuState {
            paused: true,
            ..state()
        });
        assert_eq!(items[0].label, "nomouse 0.1.3: paused");
        assert!(find(&items, MenuAction::Pause).is_none());
        assert!(find(&items, MenuAction::Resume).is_some_and(|item| item.enabled));
        assert!(find(&items, MenuAction::Search).is_some_and(|item| !item.enabled));
    }

    #[test]
    fn unsupported_session_shows_the_warning_and_keeps_settings() {
        let items = status_menu(&MenuState {
            active: false,
            warning: Some("GNOME on Wayland can't show labels".to_owned()),
            hotkeys: None,
            start_at_login: None,
            ..state()
        });
        assert_eq!(items[0].label, "nomouse 0.1.3: not running in this session");
        let warning = find(&items, MenuAction::Warning).unwrap();
        assert!(warning.enabled && warning.label.contains("GNOME on Wayland"));
        assert!(find(&items, MenuAction::Pause).is_some_and(|item| !item.enabled));
        assert_eq!(find(&items, MenuAction::Hints).unwrap().label, "Show hints");
        assert!(find(&items, MenuAction::Settings).is_some_and(|item| item.enabled));
        assert!(find(&items, MenuAction::StartAtLogin).is_none());
    }

    #[test]
    fn update_line_follows_the_check() {
        let line = |update: UpdateLine| {
            let items = status_menu(&MenuState { update, ..state() });
            items[items.len() - 4].clone()
        };
        assert_eq!(line(UpdateLine::Checking).kind, ItemKind::Info);
        let newer = line(UpdateLine::Checked(UpdateStatus::Available {
            current: Version(0, 1, 3),
            latest: Version(0, 1, 4),
        }));
        assert_eq!(newer.action(), Some(MenuAction::OpenRelease));
        assert_eq!(newer.label, "nomouse 0.1.4 is available (you have 0.1.3)");
        let current = line(UpdateLine::Checked(UpdateStatus::UpToDate(Version(
            0, 1, 3,
        ))));
        assert_eq!(current.action(), Some(MenuAction::CheckForUpdates));
        assert_eq!(
            line(UpdateLine::Failed("offline".to_owned())).action(),
            Some(MenuAction::CheckForUpdates)
        );
    }
}
