//! The AT-SPI implementation of [`Tree`]: async zbus proxies, pipelined per tree level.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_io::block_on;
use atspi::proxy::accessible::AccessibleProxy;
use atspi::proxy::action::ActionProxy;
use atspi::proxy::bus::{BusProxy, StatusProxy};
use atspi::proxy::collection::CollectionProxy;
use atspi::proxy::component::ComponentProxy;
use atspi::proxy::text::TextProxy;
use atspi::proxy::value::ValueProxy;
use atspi::{
    CoordType, Interface, InterfaceSet, MatchType, ObjectMatchRule, ObjectRefOwned, Role as AtRole,
    SortOrder, State, StateSet,
};
use futures_util::future::join_all;
use nomouse_core::{PlatformError, Rect};
use zbus::proxy::CacheProperties;

use super::walk::{self, NodeInfo, Texts, Tree, TreeError, ValueKind};

/// Longest value text read for search; a text field holding a whole document is cut here.
const MAX_VALUE_CHARS: i32 = 200;

fn node_info(
    role: AtRole,
    states: StateSet,
    interfaces: InterfaceSet,
    extents: Option<Rect>,
) -> NodeInfo {
    NodeInfo {
        role,
        name: String::new(),
        showing: states.contains(State::Showing) && states.contains(State::Visible),
        enabled: states.contains(State::Enabled) && states.contains(State::Sensitive),
        focusable: states.contains(State::Focusable),
        has_action: interfaces.contains(Interface::Action),
        extents,
    }
}

/// The registry's desktop object: its children are the applications.
const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT_PATH: &str = "/org/a11y/atspi/accessible/root";

/// One accessible object: its bus name and object path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Obj {
    /// Unique bus name of the owning application.
    pub bus: String,
    /// Object path within it.
    pub path: String,
}

impl Obj {
    fn from_ref(object: &ObjectRefOwned) -> Option<Self> {
        let bus = object.name_as_str()?;
        let path = object.path_as_str();
        (path != "/org/a11y/atspi/null").then(|| Self {
            bus: bus.to_owned(),
            path: path.to_owned(),
        })
    }
}

fn tree_error(error: &zbus::Error) -> TreeError {
    TreeError(error.to_string())
}

fn unavailable(error: &zbus::Error) -> PlatformError {
    PlatformError::AccessibilityUnavailable(format!(
        "{error}; {}",
        crate::ACCESSIBILITY_REQUIREMENT
    ))
}

/// A connection to the AT-SPI accessibility bus.
pub(crate) struct Bus {
    conn: zbus::Connection,
}

/// A top-level object of an application and the facts used to pick the focused window.
#[derive(Debug, Clone)]
pub(crate) struct TopLevel {
    /// The object.
    pub obj: Obj,
    /// Its role.
    pub role: AtRole,
    /// `ACTIVE` is set (the focused window).
    pub active: bool,
    /// `SHOWING` is set.
    pub showing: bool,
    /// Screen extents.
    pub extents: Option<Rect>,
}

/// An application on the bus.
#[derive(Debug, Clone)]
pub(crate) struct App {
    /// Process id of its bus connection.
    pub pid: Option<u32>,
    /// Its windows, menus and dialogs.
    pub top_levels: Vec<TopLevel>,
}

impl Bus {
    /// Connects to the accessibility bus, with `timeout` on every call.
    ///
    /// The address comes from `AT_SPI_BUS_ADDRESS`, else from `org.a11y.Bus` on the session bus.
    ///
    /// # Errors
    /// [`PlatformError::AccessibilityUnavailable`] when neither bus is reachable.
    pub(crate) fn connect(timeout: Duration) -> Result<Self, PlatformError> {
        block_on(async {
            let address = match std::env::var("AT_SPI_BUS_ADDRESS") {
                Ok(address) if !address.is_empty() => address,
                _ => {
                    let session = zbus::Connection::session()
                        .await
                        .map_err(|e| unavailable(&e))?;
                    BusProxy::new(&session)
                        .await
                        .map_err(|e| unavailable(&e))?
                        .get_address()
                        .await
                        .map_err(|e| unavailable(&e))?
                }
            };
            let conn = zbus::connection::Builder::address(address.as_str())
                .map_err(|e| unavailable(&e))?
                .method_timeout(timeout)
                .build()
                .await
                .map_err(|e| unavailable(&e))?;
            Ok(Self { conn })
        })
    }

    async fn accessible(&self, obj: &Obj) -> zbus::Result<AccessibleProxy<'static>> {
        AccessibleProxy::builder(&self.conn)
            .destination(obj.bus.clone())?
            .path(obj.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    async fn component(&self, obj: &Obj) -> zbus::Result<ComponentProxy<'static>> {
        ComponentProxy::builder(&self.conn)
            .destination(obj.bus.clone())?
            .path(obj.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    async fn action(&self, obj: &Obj) -> zbus::Result<ActionProxy<'static>> {
        ActionProxy::builder(&self.conn)
            .destination(obj.bus.clone())?
            .path(obj.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    async fn text(&self, obj: &Obj) -> zbus::Result<TextProxy<'static>> {
        TextProxy::builder(&self.conn)
            .destination(obj.bus.clone())?
            .path(obj.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    async fn value(&self, obj: &Obj) -> zbus::Result<ValueProxy<'static>> {
        ValueProxy::builder(&self.conn)
            .destination(obj.bus.clone())?
            .path(obj.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    /// An object's value for search: a text field's first characters or a number; empty when
    /// there is none or it cannot be read.
    async fn value_of(&self, obj: &Obj, kind: ValueKind) -> String {
        match kind {
            ValueKind::None => String::new(),
            ValueKind::Text => match self.text(obj).await {
                Ok(text) => text.get_text(0, MAX_VALUE_CHARS).await.unwrap_or_default(),
                Err(_) => String::new(),
            },
            ValueKind::Number => match self.value(obj).await {
                Ok(value) => value
                    .current_value()
                    .await
                    .map(|number| number.to_string())
                    .unwrap_or_default(),
                Err(_) => String::new(),
            },
        }
    }

    async fn children_of(&self, obj: &Obj) -> zbus::Result<Vec<Obj>> {
        let children = self.accessible(obj).await?.get_children().await?;
        Ok(children.iter().filter_map(Obj::from_ref).collect())
    }

    async fn extents_of(&self, obj: &Obj) -> zbus::Result<Rect> {
        let (x, y, width, height) = self
            .component(obj)
            .await?
            .get_extents(CoordType::Screen)
            .await?;
        Ok(Rect::new(
            x,
            y,
            u32::try_from(width).unwrap_or(0),
            u32::try_from(height).unwrap_or(0),
        ))
    }

    async fn info_of(&self, obj: &Obj) -> zbus::Result<NodeInfo> {
        let proxy = self.accessible(obj).await?;
        // One round trip: the extents request fails harmlessly without a Component interface.
        // Names are read later, for labelled objects only (see `Tree::names`).
        let (role, states, interfaces, extents) = futures_util::join!(
            proxy.get_role(),
            proxy.get_state(),
            proxy.get_interfaces(),
            self.extents_of(obj)
        );
        Ok(node_info(role?, states?, interfaces?, extents.ok()))
    }

    async fn matches(
        &self,
        root: &Obj,
        rule: ObjectMatchRule,
        limit: usize,
    ) -> zbus::Result<Vec<Obj>> {
        let collection = CollectionProxy::builder(&self.conn)
            .destination(root.bus.clone())?
            .path(root.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let count = i32::try_from(limit).unwrap_or(i32::MAX);
        let found = collection
            .get_matches(rule, SortOrder::Canonical, count, true)
            .await?;
        Ok(found.iter().filter_map(Obj::from_ref).collect())
    }

    async fn pid_of(&self, bus: &str) -> Option<u32> {
        let dbus = zbus::fdo::DBusProxy::new(&self.conn).await.ok()?;
        let name = zbus::names::BusName::try_from(bus).ok()?;
        dbus.get_connection_unix_process_id(name).await.ok()
    }

    async fn top_level(&self, obj: Obj) -> Option<TopLevel> {
        let proxy = self.accessible(&obj).await.ok()?;
        let (role, states) = futures_util::join!(proxy.get_role(), proxy.get_state());
        let states = states.ok()?;
        Some(TopLevel {
            role: role.ok()?,
            active: states.contains(State::Active),
            showing: states.contains(State::Showing),
            extents: self.extents_of(&obj).await.ok(),
            obj,
        })
    }

    async fn app(&self, app: Obj) -> Option<App> {
        let (pid, children) = futures_util::join!(self.pid_of(&app.bus), self.children_of(&app));
        let top_levels = join_all(children.ok()?.into_iter().map(|obj| self.top_level(obj))).await;
        Some(App {
            pid,
            top_levels: top_levels.into_iter().flatten().collect(),
        })
    }

    /// Every application on the bus with its top-level objects.
    ///
    /// # Errors
    /// [`PlatformError::AccessibilityUnavailable`] when the registry does not answer.
    pub(crate) fn applications(&self) -> Result<Vec<App>, PlatformError> {
        block_on(async {
            let root = Obj {
                bus: REGISTRY.to_owned(),
                path: ROOT_PATH.to_owned(),
            };
            let apps = self.children_of(&root).await.map_err(|e| unavailable(&e))?;
            Ok(join_all(apps.into_iter().map(|app| self.app(app)))
                .await
                .into_iter()
                .flatten()
                .collect())
        })
    }

    /// Performs the object's first action named like a click (`click`, `press`, `activate`,
    /// `jump`, `toggle`), or its only action. Returns whether one ran.
    pub(crate) fn activate(&self, obj: &Obj) -> bool {
        block_on(async {
            let Ok(action) = self.action(obj).await else {
                return false;
            };
            // `GetActions` aborts GTK 3 apps (at-spi2-atk 2.38 marshals its a(sss) reply
            // wrongly), so read the count and each name instead.
            let Ok(count) = action.n_actions().await else {
                return false;
            };
            let names: Vec<String> = join_all((0..count.max(0)).map(|i| action.get_name(i)))
                .await
                .into_iter()
                .map(|name| name.unwrap_or_default().to_lowercase())
                .collect();
            let index = pick_action(&names);
            match index.and_then(|i| i32::try_from(i).ok()) {
                Some(index) => action.do_action(index).await.unwrap_or(false),
                None => false,
            }
        })
    }
}

/// Picks the click-like action among `names`.
pub(crate) fn pick_action(names: &[String]) -> Option<usize> {
    const PREFERRED: [&str; 5] = ["click", "press", "activate", "jump", "toggle"];
    PREFERRED
        .iter()
        .find_map(|wanted| names.iter().position(|name| name == wanted))
        .or((names.len() == 1).then_some(0))
}

/// Process id and connection status of the session's accessibility switch.
///
/// # Errors
/// When the session bus or `org.a11y.Bus` is unreachable.
pub(crate) fn accessibility_enabled() -> Result<bool, PlatformError> {
    block_on(async {
        let session = zbus::Connection::session()
            .await
            .map_err(|e| unavailable(&e))?;
        let status = StatusProxy::new(&session)
            .await
            .map_err(|e| unavailable(&e))?;
        status.is_enabled().await.map_err(|e| unavailable(&e))
    })
}

/// Turns on the session's accessibility switch so toolkits that check it (Qt, Chromium)
/// expose their trees to apps started afterwards.
///
/// # Errors
/// When the session bus or `org.a11y.Bus` is unreachable.
pub(crate) fn enable_accessibility() -> Result<(), PlatformError> {
    block_on(async {
        let session = zbus::Connection::session()
            .await
            .map_err(|e| unavailable(&e))?;
        let status = StatusProxy::new(&session)
            .await
            .map_err(|e| unavailable(&e))?;
        status
            .set_is_enabled(true)
            .await
            .map_err(|e| unavailable(&e))
    })
}

impl Tree for Bus {
    type Node = Obj;

    fn children(&mut self, nodes: &[Obj]) -> Vec<Result<Vec<Obj>, TreeError>> {
        block_on(join_all(nodes.iter().map(|node| async {
            self.children_of(node).await.map_err(|e| tree_error(&e))
        })))
    }

    fn texts(&mut self, nodes: &[(Obj, ValueKind)]) -> Vec<Option<Texts>> {
        let bus = &*self;
        block_on(join_all(nodes.iter().map(|(node, kind)| async move {
            let proxy = bus.accessible(node).await.ok()?;
            let (name, description, value) =
                futures_util::join!(proxy.name(), proxy.description(), bus.value_of(node, *kind));
            Some(Texts {
                name: name.ok()?,
                description: description.unwrap_or_default(),
                value,
            })
        })))
    }

    fn info(&mut self, nodes: &[Obj]) -> Vec<Result<NodeInfo, TreeError>> {
        block_on(join_all(nodes.iter().map(|node| async {
            self.info_of(node).await.map_err(|e| tree_error(&e))
        })))
    }

    fn candidates(&mut self, root: &Obj, limit: usize) -> Result<Option<Vec<Obj>>, TreeError> {
        block_on(async {
            let interfaces = self
                .accessible(root)
                .await
                .map_err(|e| tree_error(&e))?
                .get_interfaces()
                .await
                .map_err(|e| tree_error(&e))?;
            if !interfaces.contains(Interface::Collection) {
                return Ok(None);
            }
            // Four server-side matches, all for showing objects: interactive roles, scroll panes
            // (scroll mode's targets), objects with actions, focusable objects. Unused criteria
            // must be `NA` with empty sets (GTK treats `Invalid` as "never matches").
            let base = || {
                ObjectMatchRule::builder()
                    .states([State::Showing, State::Visible], MatchType::All)
                    .attributes(HashMap::new(), MatchType::NA)
                    .roles(&[], MatchType::NA)
                    .interfaces([] as [Interface; 0], MatchType::NA)
            };
            let by_role = base()
                .roles(walk::INTERACTIVE_ROLES, MatchType::Any)
                .build();
            let by_scroll = base().roles(walk::SCROLL_ROLES, MatchType::Any).build();
            let by_action = base()
                .interfaces([Interface::Action], MatchType::All)
                .build();
            let by_focus = base()
                .states(
                    [State::Showing, State::Visible, State::Focusable],
                    MatchType::All,
                )
                .build();
            let (a, b, c, d) = futures_util::join!(
                self.matches(root, by_role, limit),
                self.matches(root, by_action, limit),
                self.matches(root, by_focus, limit),
                self.matches(root, by_scroll, limit)
            );
            let mut seen = HashSet::new();
            let mut found = Vec::new();
            for obj in a
                .map_err(|e| tree_error(&e))?
                .into_iter()
                .chain(b.unwrap_or_default())
                .chain(c.unwrap_or_default())
                .chain(d.unwrap_or_default())
            {
                if seen.insert(obj.clone()) {
                    found.push(obj);
                }
            }
            Ok(Some(found))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn picks_click_like_actions() {
        assert_eq!(pick_action(&names(&["expand", "press"])), Some(1));
        assert_eq!(pick_action(&names(&["jump", "click"])), Some(1));
        assert_eq!(pick_action(&names(&["custom"])), Some(0));
        assert_eq!(pick_action(&names(&["a", "b"])), None);
        assert_eq!(pick_action(&[]), None);
    }
}
