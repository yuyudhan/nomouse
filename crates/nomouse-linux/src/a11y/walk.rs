//! The accessibility tree walk: limits, cycle detection and the "is this clickable" filter.
//!
//! The walk is pure logic over the [`Tree`] seam, so it is tested with an in-memory tree; the
//! AT-SPI implementation lives in [`super::bus`]. Requests are batched per tree level so the
//! D-Bus implementation can pipeline them.
//!
//! With `hints.browser = "exhaustive"` the parts of web documents (below a `DOCUMENT_WEB`
//! object) that are scrolled out of view are kept too: their objects are not `SHOWING`, only
//! `VISIBLE`. They are walked after everything in view and reported as off-screen.

use std::collections::HashSet;
use std::hash::Hash;
use std::time::Instant;

use atspi::Role as AtRole;
use nomouse_core::config::BrowserMode;
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
    /// `VISIBLE` is set: web content scrolled out of view keeps it while losing `SHOWING`.
    pub visible: bool,
    /// `ENABLED` and `SENSITIVE` are both set.
    pub enabled: bool,
    /// `FOCUSABLE` is set.
    pub focusable: bool,
    /// `EDITABLE` is set (GTK 3 entries are `TEXT` objects with it).
    pub editable: bool,
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
    /// Use bulk matching where the tree supports it (never with [`BrowserMode::Exhaustive`]:
    /// the server-side match sees only showing objects).
    pub bulk: bool,
    /// Whether web content scrolled out of view is collected too.
    pub browser: BrowserMode,
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
    /// The extents as the toolkit reported them at the scan, before a native Wayland window's
    /// origin is added to the element's bounds: what a later extents reading compares to.
    pub extents: Rect,
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

/// Whether the object is on screen: showing and with its extents inside `clip`.
fn in_view(info: &NodeInfo, bounds: Rect, clip: Rect) -> bool {
    info.showing && bounds.intersects(&clip)
}

/// Whether the object gets a label, and why; `clip` is the visible area (window or screen).
/// `out_of_view` also accepts web content scrolled out of it (`VISIBLE` but not `SHOWING`, or
/// outside `clip`).
fn qualifies(info: &NodeInfo, clip: Rect, out_of_view: bool) -> Option<(Role, Reason)> {
    let bounds = info.extents?;
    if bounds.is_empty() || !(in_view(info, bounds, clip) || (out_of_view && info.visible)) {
        return None;
    }
    let role = match map_role(info.role)? {
        Role::Text if info.editable => Role::TextField,
        role => role,
    };
    if is_interactive_role(info.role) || role == Role::TextField {
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

/// When the walk reads an object's children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Descend {
    /// In the next level.
    Now,
    /// After everything in view: web content scrolled out of view, under exhaustive mode.
    Later,
    /// Never.
    No,
}

/// Whether the walk descends into the object: hidden or fully clipped subtrees are skipped
/// (unless `out_of_view` keeps web content scrolled away, for later), and so are the insides
/// of controls (a button's label, a link's text), except list and tree items, which can hold
/// controls of their own.
fn descend(info: &NodeInfo, clip: Rect, out_of_view: bool) -> Descend {
    let control =
        is_interactive_role(info.role) && !matches!(info.role, AtRole::ListItem | AtRole::TreeItem);
    let shown = info.showing
        && info
            .extents
            .is_none_or(|bounds| bounds.is_empty() || bounds.intersects(&clip));
    if control {
        Descend::No
    } else if shown {
        Descend::Now
    } else if out_of_view && info.visible {
        Descend::Later
    } else {
        Descend::No
    }
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
    fn consider(&mut self, node: N, info: &NodeInfo, clip: Rect, out_of_view: bool) {
        let Some((role, reason)) = qualifies(info, clip, out_of_view) else {
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
                    offscreen: !in_view(info, bounds, clip),
                },
                has_action: info.has_action,
                value: ValueKind::of(info.role),
                extents: bounds,
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
/// vanished mid-walk) are skipped. With [`BrowserMode::Exhaustive`] every root is walked, and
/// the out-of-view subtrees of web documents are walked last, so they never use up the node
/// budget or the deadline before what is in view.
pub(crate) fn collect<T: Tree>(
    tree: &mut T,
    roots: &[T::Node],
    clip: Rect,
    limits: Limits,
) -> Vec<Found<T::Node>> {
    let mut walker = Walker {
        clip,
        limits,
        visited: HashSet::new(),
        collector: Collector { found: Vec::new() },
    };
    let bulk = limits.bulk && limits.browser == BrowserMode::Fast;
    let mut walk_roots = Vec::new();
    for root in roots {
        let candidates = if bulk {
            tree.candidates(root, limits.max_nodes)
        } else {
            Ok(None)
        };
        match candidates {
            // An empty answer may be a toolkit whose Collection matching is broken; the walk
            // costs little when there really is nothing to label.
            Ok(Some(nodes)) if !nodes.is_empty() => {
                tracing::debug!("bulk match returned {} objects", nodes.len());
                let nodes: Vec<T::Node> = nodes
                    .into_iter()
                    .filter(|node| walker.visited.insert(node.clone()))
                    .take(limits.max_nodes)
                    .collect();
                for (node, info) in nodes.iter().zip(tree.info(&nodes)) {
                    if let Ok(info) = info {
                        walker.collector.consider(node.clone(), &info, clip, false);
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
    let level: Vec<T::Node> = walk_roots
        .into_iter()
        .filter(|node| walker.visited.insert(node.clone()))
        .collect();
    let webs = vec![false; level.len()];
    let (later, later_webs) = walker.walk(tree, level, webs, Pass::InView);
    if !later.is_empty() {
        tracing::debug!("walking {} web subtrees out of view", later.len());
        let (level, webs, full) = walker.expand(tree, &later, &later_webs);
        if full {
            walker.finish_level(tree, level, &webs, Pass::OutOfView);
        } else {
            walker.walk(tree, level, webs, Pass::OutOfView);
        }
    }
    let mut found = walker.collector.finish();
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

/// Which objects a walk pass reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Everything in view; out-of-view web subtrees are set aside.
    InView,
    /// The set-aside web subtrees.
    OutOfView,
}

/// The state of one scan's breadth-first walks.
struct Walker<N> {
    clip: Rect,
    limits: Limits,
    visited: HashSet<N>,
    collector: Collector<N>,
}

impl<N: Clone + Eq + Hash> Walker<N> {
    /// Whether an object may be collected (and walked) while out of view.
    fn out_of_view(&self, pass: Pass, web: bool) -> bool {
        pass == Pass::OutOfView || (web && self.limits.browser == BrowserMode::Exhaustive)
    }

    /// Walks level by level from `level` (`webs[i]`: whether `level[i]` is inside a web
    /// document). Returns the objects whose children were set aside for [`Pass::OutOfView`],
    /// with the web flag of those children.
    fn walk<T: Tree<Node = N>>(
        &mut self,
        tree: &mut T,
        mut level: Vec<N>,
        mut webs: Vec<bool>,
        pass: Pass,
    ) -> (Vec<N>, Vec<bool>) {
        let (mut later, mut later_webs) = (Vec::new(), Vec::new());
        let mut depth = 0;
        while !level.is_empty() {
            if Instant::now() >= self.limits.deadline {
                tracing::debug!("tree walk hit its deadline at depth {depth}");
                break;
            }
            let level_started = Instant::now();
            let count = level.len();
            let infos = tree.info(&level);
            tracing::trace!(
                "level {depth}: {count} objects in {:?}",
                level_started.elapsed()
            );
            let (mut parents, mut parent_webs) = (Vec::new(), Vec::new());
            for ((node, web), info) in level.into_iter().zip(webs).zip(infos) {
                let Ok(info) = info else {
                    continue;
                };
                let out_of_view = self.out_of_view(pass, web);
                self.collector
                    .consider(node.clone(), &info, self.clip, out_of_view);
                let child_web = web || info.role == AtRole::DocumentWeb;
                match descend(&info, self.clip, out_of_view) {
                    Descend::Later if pass == Pass::InView => {
                        later.push(node);
                        later_webs.push(child_web);
                    }
                    Descend::Now | Descend::Later => {
                        parents.push(node);
                        parent_webs.push(child_web);
                    }
                    Descend::No => {}
                }
            }
            if depth >= self.limits.max_depth {
                tracing::debug!("tree walk hit its depth limit {depth}");
                break;
            }
            depth += 1;
            let (next, next_webs, full) = self.expand(tree, &parents, &parent_webs);
            if full {
                // Examine what was already fetched, then stop.
                self.finish_level(tree, next, &next_webs, pass);
                break;
            }
            level = next;
            webs = next_webs;
        }
        (later, later_webs)
    }

    /// The unvisited children of `parents`, each with its parent's entry of `webs`; `true`
    /// when the node limit cut the list short.
    fn expand<T: Tree<Node = N>>(
        &mut self,
        tree: &mut T,
        parents: &[N],
        webs: &[bool],
    ) -> (Vec<N>, Vec<bool>, bool) {
        let (mut next, mut next_webs) = (Vec::new(), Vec::new());
        for (children, web) in tree.children(parents).into_iter().zip(webs) {
            let Ok(children) = children else {
                continue;
            };
            for child in children {
                if self.visited.len() >= self.limits.max_nodes {
                    tracing::debug!("tree walk hit its node limit {}", self.limits.max_nodes);
                    return (next, next_webs, true);
                }
                if self.visited.insert(child.clone()) {
                    next.push(child);
                    next_webs.push(*web);
                }
            }
        }
        (next, next_webs, false)
    }

    /// Considers `level` without descending (the node limit was reached).
    fn finish_level<T: Tree<Node = N>>(
        &mut self,
        tree: &mut T,
        level: Vec<N>,
        webs: &[bool],
        pass: Pass,
    ) {
        let infos = tree.info(&level);
        for ((node, web), info) in level.into_iter().zip(webs).zip(infos) {
            if let Ok(info) = info {
                let out_of_view = self.out_of_view(pass, *web);
                self.collector.consider(node, &info, self.clip, out_of_view);
            }
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
            visible: true,
            enabled: true,
            focusable: false,
            editable: false,
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
            browser: BrowserMode::Fast,
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

    /// frame(20) > [document(21) > [link in view(22), link scrolled below(23), section below(24)
    /// > link(25), hidden link(26)], native button off screen(27)]
    fn long_page() -> FakeTree {
        let scrolled = |role, rect| NodeInfo {
            showing: false,
            ..node(role, rect)
        };
        let mut tree = FakeTree::default();
        tree.add(20, None, node(AtRole::Frame, CLIP));
        tree.add(21, Some(20), node(AtRole::DocumentWeb, CLIP));
        tree.add(22, Some(21), node(AtRole::Link, Rect::new(10, 10, 80, 20)));
        tree.add(
            23,
            Some(21),
            scrolled(AtRole::Link, Rect::new(10, 3000, 80, 20)),
        );
        tree.add(
            24,
            Some(21),
            scrolled(AtRole::Section, Rect::new(0, 4000, 1000, 500)),
        );
        tree.add(
            25,
            Some(24),
            scrolled(AtRole::Link, Rect::new(10, 4100, 80, 20)),
        );
        let mut hidden = scrolled(AtRole::Link, Rect::new(10, 20, 80, 20));
        hidden.visible = false;
        tree.add(26, Some(21), hidden);
        tree.add(
            27,
            Some(20),
            node(AtRole::Button, Rect::new(2000, 0, 10, 10)),
        );
        tree.bulk = Some(vec![22]);
        tree
    }

    fn offscreen_flags(found: &[Found<u32>]) -> Vec<(u32, bool)> {
        found
            .iter()
            .map(|f| (f.node, f.element.offscreen))
            .collect()
    }

    #[test]
    fn fast_collects_only_the_part_of_the_page_in_view() {
        for bulk in [false, true] {
            let fast = Limits { bulk, ..limits() };
            let found = collect(&mut long_page(), &[20], CLIP, fast);
            assert_eq!(offscreen_flags(&found), [(22, false)]);
        }
    }

    #[test]
    fn exhaustive_collects_the_page_scrolled_out_of_view_last() {
        let exhaustive = Limits {
            bulk: true,
            browser: BrowserMode::Exhaustive,
            ..limits()
        };
        // The bulk match sees showing objects only, so exhaustive mode walks instead.
        let found = collect(&mut long_page(), &[20], CLIP, exhaustive);
        assert_eq!(
            offscreen_flags(&found),
            [(22, false), (23, true), (25, true)]
        );
        // A node budget used up by what is in view leaves the set-aside subtrees unwalked.
        let tight = Limits {
            max_nodes: 7,
            ..exhaustive
        };
        let found = collect(&mut long_page(), &[20], CLIP, tight);
        assert_eq!(offscreen_flags(&found), [(22, false), (23, true)]);
    }

    #[test]
    fn editable_text_is_a_text_field() {
        let mut tree = sample();
        // A GTK 3 entry: role TEXT, focusable and editable, no Action interface.
        let mut entry = node(AtRole::Text, Rect::new(500, 10, 120, 30));
        entry.focusable = true;
        entry.editable = true;
        tree.add(11, Some(1), entry);
        let mut read_only = node(AtRole::Text, Rect::new(500, 50, 120, 30));
        read_only.focusable = true;
        tree.add(12, Some(1), read_only);
        let found = collect(&mut tree, &[0], CLIP, limits());
        let role = |id| found.iter().find(|f| f.node == id).map(|f| f.element.role);
        assert_eq!(role(11), Some(Role::TextField));
        assert_eq!(role(12), Some(Role::Other));
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
