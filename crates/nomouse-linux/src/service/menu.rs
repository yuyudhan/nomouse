//! `com.canonical.dbusmenu`: the tray icon's menu, built from
//! [`nomouse_core::menu::status_menu`]. Item ids are stable: an action's id, or 1000 plus the
//! row for separators and status lines.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nomouse_core::menu::{ItemKind, MenuAction, MenuItem, status_menu};
use serde::{Deserialize, Serialize};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Type, Value};

use super::{Command, Shared};

/// A menu node: id, properties, children (each a variant holding a node).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Value, OwnedValue)]
pub(super) struct Layout {
    id: i32,
    properties: HashMap<String, OwnedValue>,
    children: Vec<OwnedValue>,
}

/// Id of the menu's root node.
const ROOT: i32 = 0;
/// First id of rows without an action.
const PLAIN: i32 = 1000;

/// Converts a basic value to an owned one (only file descriptors can fail, and none are used).
fn owned<'a>(value: impl Into<Value<'a>>) -> Option<OwnedValue> {
    value.into().try_to_owned().ok()
}

/// The dbusmenu id of the row at `index`.
fn item_id(index: usize, item: &MenuItem) -> i32 {
    item.action()
        .map_or_else(|| PLAIN + i32::try_from(index).unwrap_or(0), MenuAction::id)
}

/// An item's properties, as dbusmenu names them (`_` escaped: it marks mnemonics).
fn properties(item: &MenuItem) -> HashMap<String, OwnedValue> {
    let mut props = HashMap::new();
    let mut put = |name: &str, value: Option<OwnedValue>| {
        if let Some(value) = value {
            props.insert(name.to_owned(), value);
        }
    };
    match item.kind {
        ItemKind::Separator => put("type", owned("separator")),
        ItemKind::Info | ItemKind::Command(_) => {}
        ItemKind::Toggle(_, on) => {
            put("toggle-type", owned("checkmark"));
            put("toggle-state", owned(i32::from(on)));
        }
    }
    if !item.label.is_empty() {
        put("label", owned(item.label.replace('_', "__")));
    }
    put("enabled", owned(item.enabled));
    put("visible", owned(true));
    props
}

/// The whole menu for the current state.
fn layout(shared: &Shared) -> Layout {
    let items = status_menu(&shared.state());
    let children = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let node = Layout {
                id: item_id(index, item),
                properties: properties(item),
                children: Vec::new(),
            };
            OwnedValue::try_from(node).ok()
        })
        .collect();
    let mut root = HashMap::new();
    if let Some(value) = owned("submenu") {
        root.insert("children-display".to_owned(), value);
    }
    Layout {
        id: ROOT,
        properties: root,
        children,
    }
}

/// One node with its properties and no children, by id.
fn node(shared: &Shared, id: i32) -> Option<(i32, HashMap<String, OwnedValue>)> {
    if id == ROOT {
        let root = layout(shared);
        return Some((ROOT, root.properties));
    }
    status_menu(&shared.state())
        .iter()
        .enumerate()
        .find(|(index, item)| item_id(*index, item) == id)
        .map(|(_, item)| (id, properties(item)))
}

/// Keeps the properties named in `names` (all of them when `names` is empty).
fn only(
    mut properties: HashMap<String, OwnedValue>,
    names: &HashSet<String>,
) -> HashMap<String, OwnedValue> {
    if !names.is_empty() {
        properties.retain(|name, _| names.contains(name));
    }
    properties
}

/// The menu object.
pub(super) struct Menu {
    shared: Arc<Shared>,
    /// The dbusmenu protocol version, text direction and status it reports.
    version: u32,
    text_direction: &'static str,
    status: &'static str,
}

impl Menu {
    pub(super) const fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            version: 3,
            text_direction: "ltr",
            status: "normal",
        }
    }

    fn clicked(&self, id: i32, event: &str) {
        if event != "clicked" {
            return;
        }
        let Some(action) = MenuAction::from_id(id) else {
            return;
        };
        let enabled = status_menu(&self.shared.state())
            .iter()
            .any(|item| item.action() == Some(action) && item.enabled);
        if enabled {
            self.shared.send(Command::Menu(action));
        }
    }
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    /// The menu under `parent_id` (the root or one row: rows have no submenus), with
    /// children unless `recursion_depth` is 0, and only `property_names` (all when empty).
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Layout)> {
        let revision = self.shared.revision();
        let property_names: HashSet<String> = property_names.into_iter().collect();
        if parent_id == ROOT {
            let mut root = layout(&self.shared);
            root.properties = only(root.properties, &property_names);
            if recursion_depth == 0 {
                root.children.clear();
            } else if !property_names.is_empty() {
                root.children = root
                    .children
                    .into_iter()
                    .filter_map(|child| {
                        let mut node = Layout::try_from(child).ok()?;
                        node.properties = only(node.properties, &property_names);
                        OwnedValue::try_from(node).ok()
                    })
                    .collect();
            }
            return Ok((revision, root));
        }
        let (id, properties) = node(&self.shared, parent_id)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no menu item {parent_id}")))?;
        Ok((
            revision,
            Layout {
                id,
                properties: only(properties, &property_names),
                children: Vec::new(),
            },
        ))
    }

    /// `property_names` (all when empty) of several rows; unknown ids are left out.
    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let property_names: HashSet<String> = property_names.into_iter().collect();
        ids.into_iter()
            .filter_map(|id| node(&self.shared, id))
            .map(|(id, properties)| (id, only(properties, &property_names)))
            .collect()
    }

    /// One property of one row.
    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        node(&self.shared, id)
            .and_then(|(_, mut properties)| properties.remove(name))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no property {name} on {id}")))
    }

    /// A row was clicked (or hovered, opened, closed: only clicks act).
    fn event(&self, id: i32, event_id: &str, data: Value<'_>, timestamp: u32) {
        // The event's data (empty, or a host-specific value) carries nothing nomouse uses.
        drop(data);
        tracing::debug!("tray menu: {event_id} on {id} at {timestamp}");
        self.clicked(id, event_id);
    }

    /// Several events at once; returns the ids that are not in the menu.
    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        let mut unknown = Vec::new();
        for (id, event, _, _) in events {
            if node(&self.shared, id).is_none() {
                unknown.push(id);
            }
            self.clicked(id, &event);
        }
        unknown
    }

    /// The host is about to show `id`; the layout is always current, so nothing to update.
    fn about_to_show(&self, id: i32) -> bool {
        tracing::debug!(
            "tray menu: about to show {id} (revision {})",
            self.shared.revision()
        );
        false
    }

    /// [`Self::about_to_show`] for several ids: none needs an update; unknown ids are errors.
    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        let unknown = ids
            .into_iter()
            .filter(|id| node(&self.shared, *id).is_none())
            .collect();
        (Vec::new(), unknown)
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        self.version
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        self.text_direction
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        self.status
    }

    /// The layout under `parent` changed (the whole menu: `parent` 0).
    #[zbus(signal)]
    pub(super) async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomouse_core::menu::MenuState;
    use std::sync::Mutex;
    use std::sync::mpsc::channel;

    fn shared(state: MenuState) -> (Arc<Shared>, std::sync::mpsc::Receiver<Command>) {
        let (sender, receiver) = channel();
        (
            Arc::new(Shared {
                state: Mutex::new(state),
                revision: Mutex::new(7),
                commands: Mutex::new(sender),
            }),
            receiver,
        )
    }

    fn label(properties: &HashMap<String, OwnedValue>) -> String {
        properties
            .get("label")
            .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
            .unwrap_or_default()
    }

    #[test]
    fn layout_lists_the_status_menu_with_stable_ids() {
        let (shared, _) = shared(MenuState {
            version: "1.0.0".to_owned(),
            active: true,
            start_at_login: Some(true),
            ..MenuState::default()
        });
        let root = layout(&shared);
        assert_eq!(root.id, ROOT);
        let rows: Vec<Layout> = root
            .children
            .iter()
            .map(|child| Layout::try_from(child.try_clone().unwrap()).unwrap())
            .collect();
        assert_eq!(rows.len(), status_menu(&shared.state()).len());
        assert_eq!(label(&rows[0].properties), "nomouse 1.0.0: on");
        let quit = rows.last().unwrap();
        assert_eq!(quit.id, MenuAction::Quit.id());
        assert_eq!(label(&quit.properties), "Quit nomouse");
        let login = rows
            .iter()
            .find(|row| row.id == MenuAction::StartAtLogin.id())
            .unwrap();
        assert_eq!(
            i32::try_from(login.properties["toggle-state"].try_clone().unwrap()).unwrap(),
            1
        );
        let separator = &rows[1];
        assert_eq!(
            String::try_from(separator.properties["type"].try_clone().unwrap()).unwrap(),
            "separator"
        );
    }

    #[test]
    fn clicks_send_enabled_actions_only() {
        let (shared, commands) = shared(MenuState {
            active: true,
            paused: true,
            ..MenuState::default()
        });
        let menu = Menu::new(Arc::clone(&shared));
        menu.clicked(MenuAction::Settings.id(), "clicked");
        menu.clicked(MenuAction::Settings.id(), "hovered");
        // Paused: the modes are disabled and Pause is not in the menu.
        menu.clicked(MenuAction::Hints.id(), "clicked");
        menu.clicked(MenuAction::Pause.id(), "clicked");
        menu.clicked(MenuAction::Resume.id(), "clicked");
        let sent: Vec<Command> = commands.try_iter().collect();
        assert_eq!(
            sent,
            [
                Command::Menu(MenuAction::Settings),
                Command::Menu(MenuAction::Resume)
            ]
        );
    }

    #[test]
    fn underscores_are_not_mnemonics() {
        let item = MenuItem {
            kind: ItemKind::Info,
            label: "my_app".to_owned(),
            enabled: false,
        };
        assert_eq!(label(&properties(&item)), "my__app");
    }
}
