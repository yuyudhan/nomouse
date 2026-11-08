//! The accessibility tree walk, generic over how nodes are read so it can be tested with a
//! fake tree.
//!
//! The walk is a depth-first pre-order traversal with three limits (depth, node count and a
//! time budget) and cycle detection (a node reached twice is visited once). Subtrees whose
//! frame lies entirely outside the visible region are pruned: scrolled-away rows and off-screen
//! content cannot be clicked.

use std::collections::HashSet;
use std::hash::Hash;
use std::time::Duration;

use nomouse_core::{PlatformError, Rect, Role};

use crate::roles::{RoleFacts, classify};

/// What the walk needs to know about one node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeInfo<N> {
    /// `AXRole`.
    pub role: String,
    /// `AXSubrole`.
    pub subrole: Option<String>,
    /// The accessible name (title, description or value).
    pub name: String,
    /// `AXEnabled`; elements that do not report it count as enabled.
    pub enabled: bool,
    /// The frame in global top-left coordinates, when the element reports one.
    pub frame: Option<Rect>,
    /// Whether `AXPress` is among the element's actions.
    pub pressable: bool,
    /// The children to descend into, already chosen by [`pick_children`].
    pub children: Vec<N>,
}

/// Why a node could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The node is gone or its app did not answer; skip it and its subtree.
    Skip,
    /// Reading cannot continue at all (for example, Accessibility was revoked).
    Fatal(PlatformError),
}

/// A source of nodes.
pub trait AxTree {
    /// A node handle; equal handles are the same UI element.
    type Node: Clone + Eq + Hash;

    /// Reads one node.
    ///
    /// # Errors
    /// [`ReadError::Skip`] for a vanished node, [`ReadError::Fatal`] to abort the walk.
    fn read(&mut self, node: &Self::Node) -> Result<NodeInfo<Self::Node>, ReadError>;
}

/// Bounds on one walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Nodes deeper than this (roots are depth 0) are not read.
    pub max_depth: usize,
    /// At most this many nodes are read per walk.
    pub max_nodes: usize,
    /// The walk stops once this much time has passed.
    pub time_budget: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 48,
            max_nodes: 4000,
            time_budget: Duration::from_millis(600),
        }
    }
}

/// Which limit ended a walk early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Truncation {
    /// Some nodes were deeper than [`Limits::max_depth`].
    Depth,
    /// [`Limits::max_nodes`] nodes were read.
    Nodes,
    /// [`Limits::time_budget`] ran out.
    Time,
}

/// One element that gets a hint (or is a scroll/text target).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found<N> {
    /// The node.
    pub node: N,
    /// Normalised role.
    pub role: Role,
    /// Accessible name.
    pub name: String,
    /// Frame in global top-left coordinates (empty when the element reports none).
    pub bounds: Rect,
    /// Whether the element is enabled.
    pub enabled: bool,
    /// Whether the element supports `AXPress`.
    pub pressable: bool,
}

/// The result of a walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Walk<N> {
    /// Elements found, in traversal order.
    pub found: Vec<Found<N>>,
    /// Nodes read.
    pub visited: usize,
    /// Nodes reached a second time and skipped (cycles or shared children).
    pub repeats: usize,
    /// The first limit hit, if any.
    pub truncated: Option<Truncation>,
}

/// One starting point: a node and the region its content is visible in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root<N> {
    /// The node to start from.
    pub node: N,
    /// Subtrees whose frame does not intersect this region are pruned.
    pub visible: Rect,
    /// Maximum depth below this root, capped by [`Limits::max_depth`] (the menu bar is
    /// walked one level deep so closed menus are not read).
    pub max_depth: usize,
}

struct Pending<N> {
    node: N,
    depth: usize,
    max_depth: usize,
    visible: Rect,
    parent_role: Option<String>,
}

/// Walks `roots` in order. `elapsed` reports time since the walk began.
///
/// # Errors
/// The [`PlatformError`] of the first [`ReadError::Fatal`].
pub fn walk<T: AxTree>(
    tree: &mut T,
    roots: Vec<Root<T::Node>>,
    limits: Limits,
    mut elapsed: impl FnMut() -> Duration,
) -> Result<Walk<T::Node>, PlatformError> {
    let mut result = Walk {
        found: Vec::new(),
        visited: 0,
        repeats: 0,
        truncated: None,
    };
    let mut seen: HashSet<T::Node> = HashSet::new();
    let mut stack: Vec<Pending<T::Node>> = roots
        .into_iter()
        .rev()
        .map(|root| Pending {
            node: root.node,
            depth: 0,
            max_depth: root.max_depth.min(limits.max_depth),
            visible: root.visible,
            parent_role: None,
        })
        .collect();
    while let Some(pending) = stack.pop() {
        if result.visited >= limits.max_nodes {
            result.truncated.get_or_insert(Truncation::Nodes);
            break;
        }
        if elapsed() >= limits.time_budget {
            result.truncated.get_or_insert(Truncation::Time);
            break;
        }
        if !seen.insert(pending.node.clone()) {
            result.repeats += 1;
            continue;
        }
        result.visited += 1;
        let info = match tree.read(&pending.node) {
            Ok(info) => info,
            Err(ReadError::Skip) => continue,
            Err(ReadError::Fatal(error)) => return Err(error),
        };
        if let Some(frame) = info.frame {
            if !frame.is_empty() && !frame.intersects(&pending.visible) {
                continue;
            }
        }
        let facts = RoleFacts {
            role: &info.role,
            subrole: info.subrole.as_deref(),
            parent_role: pending.parent_role.as_deref(),
            pressable: info.pressable,
        };
        if let Some(role) = classify(facts) {
            result.found.push(Found {
                node: pending.node,
                role,
                name: info.name,
                bounds: info.frame.unwrap_or_default(),
                enabled: info.enabled,
                pressable: info.pressable,
            });
        }
        if info.children.is_empty() {
            continue;
        }
        if pending.depth >= pending.max_depth {
            if pending.max_depth == limits.max_depth {
                result.truncated.get_or_insert(Truncation::Depth);
            }
            continue;
        }
        // A scroll area or a clipping container narrows what is visible below it.
        let visible = match info.frame {
            Some(frame) if !frame.is_empty() && clips(&info.role) => {
                intersection(frame, pending.visible)
            }
            _ => pending.visible,
        };
        for child in info.children.into_iter().rev() {
            stack.push(Pending {
                node: child,
                depth: pending.depth + 1,
                max_depth: pending.max_depth,
                visible,
                parent_role: Some(info.role.clone()),
            });
        }
    }
    Ok(result)
}

/// Roles whose frame clips their content.
fn clips(role: &str) -> bool {
    matches!(role, "AXScrollArea" | "AXWindow" | "AXSheet" | "AXDrawer")
}

fn intersection(a: Rect, b: Rect) -> Rect {
    let left = i64::from(a.x).max(i64::from(b.x));
    let top = i64::from(a.y).max(i64::from(b.y));
    let right = (i64::from(a.x) + i64::from(a.width)).min(i64::from(b.x) + i64::from(b.width));
    let bottom = (i64::from(a.y) + i64::from(a.height)).min(i64::from(b.y) + i64::from(b.height));
    let to_coordinate =
        |v: i64| i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX });
    let to_length = |v: i64| u32::try_from(v.max(0)).unwrap_or(u32::MAX);
    Rect::new(
        to_coordinate(left),
        to_coordinate(top),
        to_length(right - left),
        to_length(bottom - top),
    )
}

/// Child lists an element may report, as read from the attributes of the same names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildLists<N> {
    /// `AXVisibleRows` (tables and outlines).
    pub visible_rows: Option<Vec<N>>,
    /// `AXVisibleChildren`.
    pub visible_children: Option<Vec<N>>,
    /// `AXChildrenInNavigationOrder`.
    pub navigation_order: Option<Vec<N>>,
    /// `AXChildren`.
    pub children: Option<Vec<N>>,
}

impl<N> Default for ChildLists<N> {
    fn default() -> Self {
        Self {
            visible_rows: None,
            visible_children: None,
            navigation_order: None,
            children: None,
        }
    }
}

/// Chooses which children to descend into.
///
/// Tables and outlines expose every row in `AXChildren` (thousands in a large Finder folder)
/// but only on-screen rows in `AXVisibleRows`; those rows plus the non-row children (headers,
/// scroll bars) are used. Otherwise the most specific non-empty list wins: visible children,
/// then navigation order (which also includes web content some apps omit from `AXChildren`),
/// then plain children.
pub fn pick_children<N: PartialEq>(role: &str, lists: ChildLists<N>) -> Vec<N> {
    let ChildLists {
        visible_rows,
        visible_children,
        navigation_order,
        children,
    } = lists;
    if matches!(role, "AXTable" | "AXOutline" | "AXList" | "AXBrowser") {
        if let Some(rows) = visible_rows.filter(|rows| !rows.is_empty()) {
            return rows;
        }
    }
    [visible_children, navigation_order, children]
        .into_iter()
        .flatten()
        .find(|list| !list.is_empty())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct FakeTree {
        nodes: HashMap<u32, NodeInfo<u32>>,
        reads: usize,
    }

    impl FakeTree {
        fn add(&mut self, id: u32, role: &str, frame: Rect, children: &[u32]) {
            self.nodes.insert(
                id,
                NodeInfo {
                    role: role.to_owned(),
                    subrole: None,
                    name: format!("n{id}"),
                    enabled: true,
                    frame: Some(frame),
                    pressable: false,
                    children: children.to_vec(),
                },
            );
        }
    }

    impl AxTree for FakeTree {
        type Node = u32;

        fn read(&mut self, node: &u32) -> Result<NodeInfo<u32>, ReadError> {
            self.reads += 1;
            if *node == 666 {
                return Err(ReadError::Fatal(PlatformError::PermissionDenied(
                    "revoked".into(),
                )));
            }
            self.nodes.get(node).cloned().ok_or(ReadError::Skip)
        }
    }

    const SCREEN: Rect = Rect::new(0, 0, 1000, 1000);
    const SMALL: Rect = Rect::new(10, 10, 20, 20);

    fn root(node: u32) -> Root<u32> {
        Root {
            node,
            visible: SCREEN,
            max_depth: usize::MAX,
        }
    }

    fn no_time() -> impl FnMut() -> Duration {
        || Duration::ZERO
    }

    fn ids(walk: &Walk<u32>) -> Vec<u32> {
        walk.found.iter().map(|found| found.node).collect()
    }

    #[test]
    fn finds_buttons_in_preorder() {
        let mut tree = FakeTree::default();
        tree.add(1, "AXWindow", SCREEN, &[2, 3]);
        tree.add(2, "AXGroup", SCREEN, &[4]);
        tree.add(3, "AXButton", SMALL, &[]);
        tree.add(4, "AXLink", SMALL, &[]);
        let walk = walk(&mut tree, vec![root(1)], Limits::default(), no_time()).unwrap();
        assert_eq!(ids(&walk), [4, 3]);
        assert_eq!(walk.found[0].role, Role::Link);
        assert_eq!(walk.visited, 4);
        assert_eq!(walk.truncated, None);
    }

    #[test]
    fn cycles_are_visited_once() {
        let mut tree = FakeTree::default();
        tree.add(1, "AXGroup", SCREEN, &[2]);
        tree.add(2, "AXButton", SMALL, &[1, 2]);
        let walk = walk(&mut tree, vec![root(1)], Limits::default(), no_time()).unwrap();
        assert_eq!(ids(&walk), [2]);
        assert_eq!(walk.repeats, 2);
        assert_eq!(tree.reads, 2);
    }

    #[test]
    fn depth_limit() {
        let mut tree = FakeTree::default();
        for id in 1..10 {
            tree.add(id, "AXButton", SMALL, &[id + 1]);
        }
        let limits = Limits {
            max_depth: 2,
            ..Limits::default()
        };
        let walk = walk(&mut tree, vec![root(1)], limits, no_time()).unwrap();
        assert_eq!(ids(&walk), [1, 2, 3]);
        assert_eq!(walk.truncated, Some(Truncation::Depth));
    }

    #[test]
    fn per_root_depth_is_not_truncation() {
        let mut tree = FakeTree::default();
        tree.add(1, "AXMenuBar", SCREEN, &[2]);
        tree.add(2, "AXMenuBarItem", SMALL, &[3]);
        tree.add(3, "AXMenu", SCREEN, &[4]);
        tree.add(4, "AXMenuItem", SMALL, &[]);
        let menu_bar = Root {
            max_depth: 1,
            ..root(1)
        };
        let walk = walk(&mut tree, vec![menu_bar], Limits::default(), no_time()).unwrap();
        assert_eq!(ids(&walk), [2]);
        assert_eq!(walk.truncated, None);
    }

    #[test]
    fn node_limit() {
        let mut tree = FakeTree::default();
        tree.add(1, "AXGroup", SCREEN, &[2, 3, 4, 5]);
        for id in 2..6 {
            tree.add(id, "AXButton", SMALL, &[]);
        }
        let limits = Limits {
            max_nodes: 3,
            ..Limits::default()
        };
        let walk = walk(&mut tree, vec![root(1)], limits, no_time()).unwrap();
        assert_eq!(ids(&walk), [2, 3]);
        assert_eq!(walk.truncated, Some(Truncation::Nodes));
    }

    #[test]
    fn time_limit() {
        let mut tree = FakeTree::default();
        tree.add(1, "AXGroup", SCREEN, &[2, 3, 4]);
        for id in 2..5 {
            tree.add(id, "AXButton", SMALL, &[]);
        }
        let ticks = Cell::new(0_u64);
        let clock = || {
            ticks.set(ticks.get() + 1);
            Duration::from_millis(ticks.get() * 100)
        };
        let limits = Limits {
            time_budget: Duration::from_millis(250),
            ..Limits::default()
        };
        let walk = walk(&mut tree, vec![root(1)], limits, clock).unwrap();
        assert_eq!(ids(&walk), [2]);
        assert_eq!(walk.truncated, Some(Truncation::Time));
    }

    #[test]
    fn offscreen_subtrees_are_pruned_and_scroll_areas_clip() {
        let mut tree = FakeTree::default();
        tree.add(1, "AXWindow", SCREEN, &[2, 3]);
        tree.add(2, "AXButton", Rect::new(2000, 10, 20, 20), &[]);
        tree.add(3, "AXScrollArea", Rect::new(0, 0, 100, 100), &[4, 5]);
        tree.add(4, "AXButton", Rect::new(10, 50, 20, 20), &[]);
        tree.add(5, "AXButton", Rect::new(10, 500, 20, 20), &[]);
        let walk = walk(&mut tree, vec![root(1)], Limits::default(), no_time()).unwrap();
        assert_eq!(ids(&walk), [3, 4]);
    }

    #[test]
    fn zero_sized_containers_are_still_walked() {
        let mut tree = FakeTree::default();
        tree.add(1, "AXGroup", Rect::new(5000, 5000, 0, 0), &[2]);
        tree.add(2, "AXButton", SMALL, &[]);
        let walk = walk(&mut tree, vec![root(1)], Limits::default(), no_time()).unwrap();
        assert_eq!(ids(&walk), [2]);
    }

    #[test]
    fn vanished_nodes_are_skipped_and_fatal_errors_abort() {
        let mut tree = FakeTree::default();
        tree.add(1, "AXGroup", SCREEN, &[99, 2]);
        tree.add(2, "AXButton", SMALL, &[]);
        let walk = walk(&mut tree, vec![root(1)], Limits::default(), no_time()).unwrap();
        assert_eq!(ids(&walk), [2]);

        tree.add(3, "AXGroup", SCREEN, &[666]);
        let error = walk_err(&mut tree, 3);
        assert_eq!(error, PlatformError::PermissionDenied("revoked".into()));
    }

    fn walk_err(tree: &mut FakeTree, start: u32) -> PlatformError {
        match walk(tree, vec![root(start)], Limits::default(), no_time()) {
            Ok(_) => PlatformError::Backend("walk unexpectedly succeeded".into()),
            Err(error) => error,
        }
    }

    #[test]
    fn tab_groups_give_tabs() {
        let mut tree = FakeTree::default();
        tree.add(1, "AXTabGroup", SCREEN, &[2]);
        tree.add(2, "AXRadioButton", SMALL, &[]);
        let walk = walk(&mut tree, vec![root(1)], Limits::default(), no_time()).unwrap();
        assert_eq!(walk.found[0].role, Role::Tab);
    }

    #[test]
    fn child_selection() {
        let lists = ChildLists {
            visible_rows: Some(vec![1, 2]),
            visible_children: None,
            navigation_order: None,
            children: Some(vec![1, 2, 3, 4]),
        };
        assert_eq!(pick_children("AXOutline", lists.clone()), [1, 2]);
        assert_eq!(pick_children("AXGroup", lists), [1, 2, 3, 4]);
        let nav = ChildLists {
            visible_children: Some(vec![]),
            navigation_order: Some(vec![7]),
            children: Some(vec![8]),
            ..ChildLists::default()
        };
        assert_eq!(pick_children("AXWebArea", nav), [7]);
        assert_eq!(
            pick_children::<u32>("AXGroup", ChildLists::default()),
            Vec::<u32>::new()
        );
    }

    #[test]
    fn intersections() {
        assert_eq!(
            intersection(Rect::new(0, 0, 10, 10), Rect::new(5, 5, 10, 10)),
            Rect::new(5, 5, 5, 5)
        );
        assert!(intersection(Rect::new(0, 0, 10, 10), Rect::new(20, 20, 5, 5)).is_empty());
    }
}
