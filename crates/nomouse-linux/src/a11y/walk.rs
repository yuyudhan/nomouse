//! The accessibility tree walk: limits, cycle detection and the "is this clickable" filter.
//!
//! The walk is pure logic over the [`Tree`] seam, so it is tested with an in-memory tree; the
//! AT-SPI implementation lives in [`super::bus`]. Requests are batched per tree level so the
//! D-Bus implementation can pipeline them.

use std::collections::HashSet;
use std::hash::Hash;
use std::time::Instant;

use atspi::Role as AtRole;
use nomouse_core::{Element, ElementId, Rect, Role};

/// What the walk needs to know about one accessible object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeInfo {
    /// The AT-SPI role.
    pub role: AtRole,
    /// Accessible name; empty when the tree reads names lazily through [`Tree::names`].
    pub name: String,
    /// `SHOWING` and `VISIBLE` are both set.
    pub showing: bool,
    /// `ENABLED` and `SENSITIVE` are both set.
    pub enabled: bool,
    /// `FOCUSABLE` is set.
    pub focusable: bool,
    /// The object implements the Action interface with at least one action.
    pub has_action: bool,
    /// Screen extents, `None` without the Component interface.
    pub extents: Option<Rect>,
}

/// A failure reading the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreeError(pub String);

/// Read access to an accessibility tree, batched per level.
pub(crate) trait Tree {
    /// A handle to one object.
    type Node: Clone + Eq + Hash;

    /// The children of each node, in the same order as `nodes`.
    fn children(&mut self, nodes: &[Self::Node]) -> Vec<Result<Vec<Self::Node>, TreeError>>;

    /// The info of each node, in the same order as `nodes`.
    fn info(&mut self, nodes: &[Self::Node]) -> Vec<Result<NodeInfo, TreeError>>;

    /// The texts of labelled objects (name, description and, for `ValueKind`s other than
    /// `None`, the value), in the same order; `None` keeps the name from [`Tree::info`] and
    /// leaves the description and value empty.
    fn texts(&mut self, nodes: &[(Self::Node, ValueKind)]) -> Vec<Option<Texts>>;

    /// The objects below `root` that may get a label (showing, with an interactive role, an
    /// action or the focusable state), found by one server-side query (AT-SPI Collection),
    /// at most `limit` per query. `Ok(None)` when the object does not support bulk matching.
    ///
    /// # Errors
    /// When the query itself fails.
    fn candidates(
        &mut self,
        root: &Self::Node,
        limit: usize,
    ) -> Result<Option<Vec<Self::Node>>, TreeError>;
}

/// Bounds on one walk.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// Deepest level visited below the roots.
    pub max_depth: usize,
    /// Most objects examined.
    pub max_nodes: usize,
    /// The walk stops (returning what it found) at this instant.
    pub deadline: Instant,
    /// Use bulk matching where the tree supports it.
    pub bulk: bool,
}

/// What search reads of a labelled object besides its role.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Texts {
    /// Accessible name.
    pub name: String,
    /// Accessible description (GTK and Qt put tooltips here).
    pub description: String,
    /// Current value (see [`ValueKind`]).
    pub value: String,
}

/// Where an object's value comes from, by its AT-SPI role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueKind {
    /// No value is read (password fields among them: their contents never leave the app).
    None,
    /// The Text interface: an entry's contents.
    Text,
    /// The Value interface: a slider's or spin button's number.
    Number,
}

impl ValueKind {
    /// The value an object of AT-SPI `role` has.
    pub(crate) const fn of(role: AtRole) -> Self {
        match role {
            AtRole::Entry | AtRole::Editbar => Self::Text,
            AtRole::Slider | AtRole::SpinButton => Self::Number,
            _ => Self::None,
        }
    }
}

/// A clickable object found by the walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found<N> {
    /// The tree handle, kept for semantic actions.
    pub node: N,
    /// The core element.
    pub element: Element,
    /// Whether the object has an Action interface.
    pub has_action: bool,
    /// Where its value is read from.
    pub value: ValueKind,
}

/// Why an object qualifies for a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    /// Its role is interactive.
    Role,
    /// Only because it has an action or is focusable; dropped when inside a [`Reason::Role`] one.
    Weak,
}

/// Maps an AT-SPI role to the core role; `None` for containers that never get a label.
pub(crate) fn map_role(role: AtRole) -> Option<Role> {
    let mapped = match role {
        AtRole::Button | AtRole::PushButtonMenu | AtRole::ToggleButton => Role::Button,
        AtRole::Link => Role::Link,
        AtRole::CheckBox | AtRole::CheckMenuItem => Role::CheckBox,
        AtRole::RadioButton | AtRole::RadioMenuItem => Role::RadioButton,
        AtRole::MenuItem | AtRole::Menu | AtRole::TearoffMenuItem => Role::MenuItem,
        AtRole::PageTab => Role::Tab,
        AtRole::Entry | AtRole::PasswordText | AtRole::Editbar => Role::TextField,
        AtRole::ComboBox => Role::ComboBox,
        AtRole::ListItem => Role::ListItem,
        AtRole::TreeItem => Role::TreeItem,
        AtRole::Slider | AtRole::SpinButton => Role::Slider,
        AtRole::TableCell => Role::Cell,
        AtRole::ScrollPane => Role::ScrollArea,
        AtRole::Label | AtRole::Static | AtRole::Text | AtRole::Paragraph | AtRole::Heading => {
            Role::Text
        }
        AtRole::Application
        | AtRole::Frame
        | AtRole::Window
        | AtRole::Dialog
        | AtRole::DocumentWeb
        | AtRole::DocumentFrame
        | AtRole::DocumentText
        | AtRole::DocumentEmail
        | AtRole::DocumentPresentation
        | AtRole::DocumentSpreadsheet
        | AtRole::RootPane
        | AtRole::LayeredPane
        | AtRole::GlassPane
        | AtRole::Viewport
        | AtRole::Filler
        | AtRole::Panel
        | AtRole::MenuBar
        | AtRole::ToolBar
        | AtRole::PageTabList
        | AtRole::List
        | AtRole::ListBox
        | AtRole::Tree
        | AtRole::TreeTable
        | AtRole::Table
        | AtRole::Separator
        | AtRole::StatusBar
        | AtRole::ScrollBar
        | AtRole::Embedded
        | AtRole::InternalFrame
        | AtRole::Invalid => return None,
        _ => Role::Other,
    };
    Some(mapped)
}

/// The roles that always get a label (when showing, sized and on screen).
pub(crate) const INTERACTIVE_ROLES: &[AtRole] = &[
    AtRole::Button,
    AtRole::PushButtonMenu,
    AtRole::ToggleButton,
    AtRole::Link,
    AtRole::CheckBox,
    AtRole::CheckMenuItem,
    AtRole::RadioButton,
    AtRole::RadioMenuItem,
    AtRole::MenuItem,
    AtRole::Menu,
    AtRole::TearoffMenuItem,
    AtRole::PageTab,
    AtRole::Entry,
    AtRole::PasswordText,
    AtRole::ComboBox,
    AtRole::ListItem,
    AtRole::TreeItem,
    AtRole::Slider,
    AtRole::SpinButton,
];

/// The roles of scrollable containers, reported as [`Role::ScrollArea`] for scroll mode.
pub(crate) const SCROLL_ROLES: &[AtRole] = &[AtRole::ScrollPane];

fn is_interactive_role(role: AtRole) -> bool {
    INTERACTIVE_ROLES.contains(&role)
}

/// Whether the object gets a label, and why; `clip` is the visible area (window or screen).
fn qualifies(info: &NodeInfo, clip: Rect) -> Option<(Role, Reason)> {
    let bounds = info.extents?;
    if !info.showing || bounds.is_empty() || !bounds.intersects(&clip) {
        return None;
    }
    let role = map_role(info.role)?;
    if is_interactive_role(info.role) {
        Some((role, Reason::Role))
    } else if role == Role::ScrollArea {
        // Scroll mode's target. Weak, so the controls inside it keep their labels.
        Some((role, Reason::Weak))
    } else if info.has_action || info.focusable {
        Some((
            if role == Role::Text {
                Role::Other
            } else {
                role
            },
            Reason::Weak,
        ))
    } else {
        None
    }
}

/// Whether the walk descends into the object: hidden or fully clipped subtrees are skipped,
/// and so are the insides of controls (a button's label, a link's text), except list and tree
/// items, which can hold controls of their own.
fn descend(info: &NodeInfo, clip: Rect) -> bool {
    let control =
        is_interactive_role(info.role) && !matches!(info.role, AtRole::ListItem | AtRole::TreeItem);
    info.showing
        && !control
        && info
            .extents
            .is_none_or(|bounds| bounds.is_empty() || bounds.intersects(&clip))
}

fn contains(outer: Rect, inner: Rect) -> bool {
    let right = |r: Rect| i64::from(r.x) + i64::from(r.width);
    let bottom = |r: Rect| i64::from(r.y) + i64::from(r.height);
    outer.x <= inner.x
        && outer.y <= inner.y
        && right(outer) >= right(inner)
        && bottom(outer) >= bottom(inner)
}

struct Collector<N> {
    found: Vec<(Found<N>, Reason)>,
}

impl<N> Collector<N> {
    fn consider(&mut self, node: N, info: &NodeInfo, clip: Rect) {
        let Some((role, reason)) = qualifies(info, clip) else {
            return;
        };
        let Some(bounds) = info.extents else {
            return;
        };
        let id = ElementId(u64::try_from(self.found.len()).unwrap_or(u64::MAX));
        self.found.push((
            Found {
                node,
                element: Element {
                    id,
                    role,
                    name: info.name.clone(),
                    description: String::new(),
                    value: String::new(),
                    bounds,
                    enabled: info.enabled,
                },
                has_action: info.has_action,
                value: ValueKind::of(info.role),
            },
            reason,
        ));
    }

    /// Drops weak hits lying inside a role hit (link text inside a link) and duplicates.
    fn finish(self) -> Vec<Found<N>> {
        let strong: Vec<Rect> = self
            .found
            .iter()
            .filter(|(_, reason)| *reason == Reason::Role)
            .map(|(found, _)| found.element.bounds)
            .collect();
        let mut seen = HashSet::new();
        self.found
            .into_iter()
            .filter(|(found, reason)| {
                let bounds = found.element.bounds;
                (*reason == Reason::Role || !strong.iter().any(|outer| contains(*outer, bounds)))
                    && seen.insert((bounds, found.element.role))
            })
            .enumerate()
            .map(|(index, (mut found, _))| {
                found.element.id = ElementId(u64::try_from(index).unwrap_or(u64::MAX));
                found
            })
            .collect()
    }
}

/// Collects the clickable objects below `roots` that intersect `clip`.
///
/// Uses one bulk query per root when `limits.bulk` is set and the tree supports it, otherwise
/// a breadth-first walk bounded by depth, node count and deadline. Every object is visited at
/// most once, so reference cycles in a broken tree cannot loop. Per-object errors (objects that
/// vanished mid-walk) are skipped.
pub(crate) fn collect<T: Tree>(
    tree: &mut T,
    roots: &[T::Node],
    clip: Rect,
    limits: Limits,
) -> Vec<Found<T::Node>> {
    let mut collector = Collector { found: Vec::new() };
    let mut visited: HashSet<T::Node> = HashSet::new();
    let mut walk_roots = Vec::new();
    for root in roots {
        let bulk = if limits.bulk {
            tree.candidates(root, limits.max_nodes)
        } else {
            Ok(None)
        };
        match bulk {
            // An empty answer may be a toolkit whose Collection matching is broken; the walk
            // costs little when there really is nothing to label.
            Ok(Some(nodes)) if !nodes.is_empty() => {
                tracing::debug!("bulk match returned {} objects", nodes.len());
                let nodes: Vec<T::Node> = nodes
                    .into_iter()
                    .filter(|node| visited.insert(node.clone()))
                    .take(limits.max_nodes)
                    .collect();
                for (node, info) in nodes.iter().zip(tree.info(&nodes)) {
                    if let Ok(info) = info {
                        collector.consider(node.clone(), &info, clip);
                    }
                }
            }
            Ok(_) => walk_roots.push(root.clone()),
            Err(error) => {
                tracing::debug!("bulk match failed ({}); walking instead", error.0);
                walk_roots.push(root.clone());
            }
        }
    }
    walk(tree, walk_roots, clip, limits, &mut visited, &mut collector);
    let mut found = collector.finish();
    let nodes: Vec<(T::Node, ValueKind)> =
        found.iter().map(|f| (f.node.clone(), f.value)).collect();
    for (found, texts) in found.iter_mut().zip(tree.texts(&nodes)) {
        if let Some(texts) = texts {
            found.element.name = texts.name;
            found.element.description = texts.description;
            found.element.value = texts.value;
        }
    }
    found
}

fn walk<T: Tree>(
    tree: &mut T,
    roots: Vec<T::Node>,
    clip: Rect,
    limits: Limits,
    visited: &mut HashSet<T::Node>,
    collector: &mut Collector<T::Node>,
) {
    let mut level: Vec<T::Node> = roots
        .into_iter()
        .filter(|node| visited.insert(node.clone()))
        .collect();
    let mut depth = 0;
    while !level.is_empty() {
        if Instant::now() >= limits.deadline {
            tracing::debug!("tree walk hit its deadline at depth {depth}");
            return;
        }
        let level_started = Instant::now();
        let count = level.len();
        let infos = tree.info(&level);
        tracing::trace!(
            "level {depth}: {count} objects in {:?}",
            level_started.elapsed()
        );
        let parents: Vec<T::Node> = level
            .into_iter()
            .zip(infos)
            .filter_map(|(node, info)| {
                let info = info.ok()?;
                collector.consider(node.clone(), &info, clip);
                descend(&info, clip).then_some(node)
            })
            .collect();
        if depth >= limits.max_depth {
            tracing::debug!("tree walk hit its depth limit {depth}");
            return;
        }
        depth += 1;
        let mut next = Vec::new();
        for children in tree.children(&parents).into_iter().flatten() {
            for child in children {
                if visited.len() >= limits.max_nodes {
                    tracing::debug!("tree walk hit its node limit {}", limits.max_nodes);
                    level = next;
                    // Examine what was already fetched, then stop.
                    finish_level(tree, level, clip, collector);
                    return;
                }
                if visited.insert(child.clone()) {
                    next.push(child);
                }
            }
        }
        level = next;
    }
}

fn finish_level<T: Tree>(
    tree: &mut T,
    level: Vec<T::Node>,
    clip: Rect,
    collector: &mut Collector<T::Node>,
) {
    let infos = tree.info(&level);
    for (node, info) in level.into_iter().zip(infos) {
        if let Ok(info) = info {
            collector.consider(node, &info, clip);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use super::*;

    #[derive(Default)]
    struct FakeTree {
        children: HashMap<u32, Vec<u32>>,
        info: HashMap<u32, NodeInfo>,
        texts: HashMap<u32, Texts>,
        /// What `texts` was asked for, in order.
        asked: Vec<(u32, ValueKind)>,
        bulk: Option<Vec<u32>>,
        info_calls: usize,
    }

    impl FakeTree {
        fn add(&mut self, id: u32, parent: Option<u32>, info: NodeInfo) {
            self.info.insert(id, info);
            if let Some(parent) = parent {
                self.children.entry(parent).or_default().push(id);
            }
        }
    }

    impl Tree for FakeTree {
        type Node = u32;

        fn children(&mut self, nodes: &[u32]) -> Vec<Result<Vec<u32>, TreeError>> {
            nodes
                .iter()
                .map(|n| Ok(self.children.get(n).cloned().unwrap_or_default()))
                .collect()
        }

        fn info(&mut self, nodes: &[u32]) -> Vec<Result<NodeInfo, TreeError>> {
            self.info_calls += nodes.len();
            nodes
                .iter()
                .map(|n| {
                    self.info
                        .get(n)
                        .cloned()
                        .ok_or_else(|| TreeError(format!("gone {n}")))
                })
                .collect()
        }

        fn texts(&mut self, nodes: &[(u32, ValueKind)]) -> Vec<Option<Texts>> {
            self.asked.extend_from_slice(nodes);
            nodes
                .iter()
                .map(|(n, _)| self.texts.get(n).cloned())
                .collect()
        }

        fn candidates(
            &mut self,
            _root: &u32,
            _limit: usize,
        ) -> Result<Option<Vec<u32>>, TreeError> {
            Ok(self.bulk.clone())
        }
    }

    fn node(role: AtRole, rect: Rect) -> NodeInfo {
        NodeInfo {
            role,
            name: format!("{role:?}"),
            showing: true,
            enabled: true,
            focusable: false,
            has_action: false,
            extents: Some(rect),
        }
    }

    fn limits() -> Limits {
        Limits {
            max_depth: 50,
            max_nodes: 10_000,
            deadline: Instant::now() + Duration::from_secs(5),
            bulk: false,
        }
    }

    const CLIP: Rect = Rect::new(0, 0, 1000, 1000);

    /// frame(0) > panel(1) > [button(2), label(3), hidden panel(4) > button(5), offscreen(6),
    /// link(7) > text-with-action(8), zero-size button(9), focusable custom(10)]
    fn sample() -> FakeTree {
        let mut tree = FakeTree::default();
        tree.add(0, None, node(AtRole::Frame, CLIP));
        tree.add(1, Some(0), node(AtRole::Panel, CLIP));
        tree.add(2, Some(1), node(AtRole::Button, Rect::new(10, 10, 50, 20)));
        tree.add(3, Some(1), node(AtRole::Label, Rect::new(10, 40, 50, 20)));
        let mut hidden = node(AtRole::Panel, Rect::new(0, 100, 100, 100));
        hidden.showing = false;
        tree.add(4, Some(1), hidden);
        tree.add(5, Some(4), node(AtRole::Button, Rect::new(0, 100, 10, 10)));
        tree.add(6, Some(1), node(AtRole::Button, Rect::new(2000, 0, 10, 10)));
        tree.add(7, Some(1), node(AtRole::Link, Rect::new(100, 10, 80, 20)));
        let mut text = node(AtRole::Static, Rect::new(105, 12, 40, 16));
        text.has_action = true;
        tree.add(8, Some(7), text);
        tree.add(9, Some(1), node(AtRole::Button, Rect::new(300, 300, 0, 10)));
        let mut custom = node(AtRole::Section, Rect::new(400, 400, 30, 30));
        custom.focusable = true;
        tree.add(10, Some(1), custom);
        tree
    }

    fn ids(found: &[Found<u32>]) -> Vec<u32> {
        let mut nodes: Vec<u32> = found.iter().map(|f| f.node).collect();
        nodes.sort_unstable();
        nodes
    }

    #[test]
    fn keeps_visible_actionable_only() {
        let found = collect(&mut sample(), &[0], CLIP, limits());
        assert_eq!(ids(&found), vec![2, 7, 10]);
        let custom = found.iter().find(|f| f.node == 10).map(|f| f.element.role);
        assert_eq!(custom, Some(Role::Other));
        let element_ids: HashSet<ElementId> = found.iter().map(|f| f.element.id).collect();
        assert_eq!(element_ids.len(), found.len());
    }

    #[test]
    fn scroll_panes_are_reported_without_hiding_their_contents() {
        let mut tree = sample();
        tree.add(
            11,
            Some(1),
            node(AtRole::ScrollPane, Rect::new(0, 500, 600, 400)),
        );
        tree.add(
            12,
            Some(11),
            node(AtRole::Button, Rect::new(10, 510, 80, 20)),
        );
        let found = collect(&mut tree, &[0], CLIP, limits());
        assert_eq!(ids(&found), vec![2, 7, 10, 11, 12]);
        let pane = found.iter().find(|f| f.node == 11).map(|f| f.element.role);
        assert_eq!(pane, Some(Role::ScrollArea));
        assert!(
            SCROLL_ROLES
                .iter()
                .all(|role| map_role(*role) == Some(Role::ScrollArea))
        );
    }

    #[test]
    fn hidden_subtrees_are_not_visited() {
        let mut tree = sample();
        collect(&mut tree, &[0], CLIP, limits());
        // Node 5 lives under the hidden panel and node 8 inside a link: never requested.
        assert_eq!(tree.info_calls, 9);
    }

    #[test]
    fn disabled_elements_are_reported_disabled() {
        let mut tree = sample();
        if let Some(info) = tree.info.get_mut(&2) {
            info.enabled = false;
        }
        let found = collect(&mut tree, &[0], CLIP, limits());
        let button = found.iter().find(|f| f.node == 2);
        assert_eq!(button.map(|f| f.element.enabled), Some(false));
    }

    #[test]
    fn depth_and_node_limits() {
        let mut chain = FakeTree::default();
        chain.add(0, None, node(AtRole::Frame, CLIP));
        for id in 1..100 {
            chain.add(id, Some(id - 1), node(AtRole::Panel, Rect::new(0, 0, 5, 5)));
        }
        let shallow = Limits {
            max_depth: 3,
            ..limits()
        };
        // Identical bounds collapse to one, so count visits instead.
        collect(&mut chain, &[0], CLIP, shallow);
        assert_eq!(chain.info_calls, 4);
        let mut wide = FakeTree::default();
        wide.add(0, None, node(AtRole::Frame, CLIP));
        for id in 1..500 {
            let x = i32::try_from(id).unwrap_or_default();
            wide.add(id, Some(0), node(AtRole::Button, Rect::new(x, 0, 1, 1)));
        }
        let few = Limits {
            max_nodes: 50,
            ..limits()
        };
        let found = collect(&mut wide, &[0], CLIP, few);
        assert_eq!(found.len(), 49);
    }

    #[test]
    fn deadline_stops_the_walk() {
        let past = Limits {
            deadline: Instant::now(),
            ..limits()
        };
        assert_eq!(collect(&mut sample(), &[0], CLIP, past), Vec::new());
    }

    #[test]
    fn cycles_terminate() {
        let mut tree = sample();
        tree.children.entry(2).or_default().push(0);
        tree.children.entry(10).or_default().push(1);
        assert_eq!(
            ids(&collect(&mut tree, &[0], CLIP, limits())),
            vec![2, 7, 10]
        );
    }

    #[test]
    fn vanished_nodes_are_skipped() {
        let mut tree = sample();
        tree.info.remove(&7);
        assert_eq!(ids(&collect(&mut tree, &[0], CLIP, limits())), vec![2, 10]);
    }

    #[test]
    fn bulk_match_skips_the_walk() {
        let mut tree = sample();
        tree.bulk = Some(vec![1, 2, 3, 7, 8, 10, 2]);
        let bulk = Limits {
            bulk: true,
            ..limits()
        };
        assert_eq!(ids(&collect(&mut tree, &[0], CLIP, bulk)), vec![2, 7, 10]);
        assert_eq!(tree.info_calls, 6);
    }

    #[test]
    fn names_descriptions_and_values_reach_the_elements() {
        let mut tree = sample();
        tree.add(11, Some(1), node(AtRole::Entry, Rect::new(10, 600, 80, 20)));
        tree.add(
            12,
            Some(1),
            node(AtRole::PasswordText, Rect::new(10, 640, 80, 20)),
        );
        tree.add(
            13,
            Some(1),
            node(AtRole::Slider, Rect::new(10, 680, 80, 20)),
        );
        tree.texts.insert(
            2,
            Texts {
                name: String::new(),
                description: "Save the file".to_owned(),
                value: String::new(),
            },
        );
        tree.texts.insert(
            11,
            Texts {
                name: "Subject".to_owned(),
                description: String::new(),
                value: "draft".to_owned(),
            },
        );
        let found = collect(&mut tree, &[0], CLIP, limits());
        let element = |node: u32| &found.iter().find(|f| f.node == node).unwrap().element;
        assert_eq!(element(2).name, "");
        assert_eq!(element(2).description, "Save the file");
        assert_eq!(element(11).value, "draft");
        assert_eq!(element(11).name, "Subject");
        // Without texts the name from the tree info stays.
        assert_eq!(element(7).name, "Link");
        let kind = |node: u32| tree.asked.iter().find(|(n, _)| *n == node).map(|(_, k)| *k);
        assert_eq!(kind(11), Some(ValueKind::Text));
        assert_eq!(kind(12), Some(ValueKind::None));
        assert_eq!(kind(13), Some(ValueKind::Number));
        assert_eq!(kind(2), Some(ValueKind::None));
    }

    #[test]
    fn role_mapping() {
        assert_eq!(map_role(AtRole::ToggleButton), Some(Role::Button));
        assert_eq!(map_role(AtRole::PageTab), Some(Role::Tab));
        assert_eq!(map_role(AtRole::Frame), None);
        assert_eq!(map_role(AtRole::Section), Some(Role::Other));
        assert!(!is_interactive_role(AtRole::Label));
    }
}
