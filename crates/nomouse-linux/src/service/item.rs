//! `org.kde.StatusNotifierItem`: the tray icon itself. A click shows the menu (the item is a
//! menu, like the macOS menu-bar item); hosts that activate instead open the settings window.

use std::sync::Arc;

use nomouse_core::menu::{MenuAction, MenuState};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedObjectPath;

use super::{Command, MENU_PATH, Shared, icon};

/// An ARGB32 image (network byte order): width, height, bytes.
pub(super) type Pixmap = (i32, i32, Vec<u8>);

/// The item's `Status` for `state`: `NeedsAttention` while the session cannot run nomouse.
pub(super) fn status(state: &MenuState) -> &'static str {
    if state.active {
        "Active"
    } else {
        "NeedsAttention"
    }
}

/// The tooltip's body for `state`.
fn tooltip(state: &MenuState) -> String {
    if !state.active {
        state
            .warning
            .clone()
            .unwrap_or_else(|| "not running in this session".to_owned())
    } else if state.paused {
        "Paused".to_owned()
    } else {
        state.hotkeys.as_ref().map_or_else(
            || "On".to_owned(),
            |hotkeys| format!("On: {} shows hints", hotkeys.hints.human()),
        )
    }
}

/// The tray item.
pub(super) struct Item {
    shared: Arc<Shared>,
    /// `Category`, `Id` and `Title`.
    category: &'static str,
    id: &'static str,
    title: &'static str,
    /// Where the menu lives.
    menu: OwnedObjectPath,
}

impl Item {
    pub(super) fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            category: "ApplicationStatus",
            id: "nomouse",
            title: "nomouse",
            menu: OwnedObjectPath::try_from(MENU_PATH).unwrap_or_default(),
        }
    }
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    /// Hosts call this for a primary click when the item is not a menu: open the settings.
    fn activate(&self, x: i32, y: i32) {
        tracing::debug!("tray: activated at {x},{y}");
        self.shared.send(Command::Menu(MenuAction::Settings));
    }

    /// Middle click: pause or resume.
    fn secondary_activate(&self, x: i32, y: i32) {
        tracing::debug!("tray: secondary activation at {x},{y}");
        let paused = self.shared.state().paused;
        self.shared.send(Command::Menu(if paused {
            MenuAction::Resume
        } else {
            MenuAction::Pause
        }));
    }

    /// A host that cannot draw the dbusmenu asks the app for a menu: open the settings,
    /// which hold everything the menu offers.
    fn context_menu(&self, x: i32, y: i32) {
        tracing::debug!("tray: context menu asked at {x},{y}");
        self.shared.send(Command::Menu(MenuAction::Settings));
    }

    #[zbus(property)]
    fn category(&self) -> &str {
        self.category
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        self.id
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        self.title
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        status(&self.shared.state())
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        icon::name(&self.shared.state())
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<Pixmap> {
        icon::pixmaps(&self.shared.state())
    }

    #[zbus(property)]
    fn attention_icon_name(&self) -> &str {
        icon::name(&self.shared.state())
    }

    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> Vec<Pixmap> {
        icon::pixmaps(&self.shared.state())
    }

    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<Pixmap>, String, String) {
        (
            String::new(),
            Vec::new(),
            self.title.to_owned(),
            tooltip(&self.shared.state()),
        )
    }

    /// A click shows the menu, as on the macOS menu bar.
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        self.menu.as_str() == MENU_PATH
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        self.menu.clone()
    }

    #[zbus(signal)]
    pub(super) async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub(super) async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    pub(super) async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomouse_core::config::HotkeysConfig;

    #[test]
    fn status_and_tooltip_follow_the_state() {
        let on = MenuState {
            active: true,
            hotkeys: Some(HotkeysConfig::default()),
            ..MenuState::default()
        };
        assert_eq!(status(&on), "Active");
        assert_eq!(tooltip(&on), "On: Ctrl+Space shows hints");
        let paused = MenuState {
            paused: true,
            ..on.clone()
        };
        assert_eq!(tooltip(&paused), "Paused");
        let unsupported = MenuState {
            active: false,
            warning: Some("GNOME on Wayland".to_owned()),
            ..on
        };
        assert_eq!(status(&unsupported), "NeedsAttention");
        assert_eq!(tooltip(&unsupported), "GNOME on Wayland");
    }
}
