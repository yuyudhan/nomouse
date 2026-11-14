//! The accessibility tree walk, generic over how nodes are read so it can be tested with a
//! fake tree.
//!
//! The walk is a depth-first pre-order traversal with three limits (depth, node count and a
//! time budget) and cycle detection (a node reached twice is visited once). Subtrees whose
//! frame lies entirely outside the visible region are pruned: scrolled-away rows and off-screen
//! content cannot be clicked, except in web documents under `hints.browser = "exhaustive"`,
//! where they are walked last and reported as off-screen (search finds them).

use std::collections::HashSet;
use std::hash::Hash;
use std::time::Duration;

use nomouse_core::config::BrowserMode;
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
    /// Description and help (tooltip) text the name does not already hold; search matches it.
    pub description: String,
    /// A text field's contents or a slider's position (empty elsewhere and for secure text
    /// fields); search matches it.
    pub value: String,
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
    /// [`BrowserMode::Exhaustive`] keeps the parts of web documents (below an `AXWebArea`)
    /// that are scrolled out of view, after everything visible; [`BrowserMode::Fast`] prunes
    /// them like any other off-screen subtree.
    pub browser: BrowserMode,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_depth: 48,
            max_nodes: 4000,
            time_budget: Duration::from_millis(600),
            browser: BrowserMode::Fast,
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
    /// Description and help text.
    pub description: String,
    /// Current value.
    pub value: String,
    /// Frame in global top-left coordinates (empty when the element reports none).
    pub bounds: Rect,
    /// Whether the element is enabled.
    pub enabled: bool,
    /// Whether the element supports `AXPress`.
    pub pressable: bool,
    /// Scrolled out of view: web content outside `visible`, kept by
    /// [`BrowserMode::Exhaustive`].
    pub offscreen: bool,
    /// The region the element's content is visible in (its window, narrowed by the scroll
    /// areas around it).
    pub visible: Rect,
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
    /// `AXWebArea` nodes read: zero in a Chromium window means its web content has no
    /// accessibility tree (`AXEnhancedUserInterface` is off).
    pub web_areas: usize,
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
    /// The frame of the innermost `AXWebArea` above it (empty when it reports none).
    web: Option<Rect>,
    /// Out of view, walked after everything visible: its subtree is not pruned.
    offscreen: bool,
}

/// Walks `roots` in order. `elapsed` reports time since the walk began. With
/// [`BrowserMode::Exhaustive`] the out-of-view subtrees of web documents are set aside and walked
/// once everything visible was, so they never use up the node or time budget first.
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
        web_areas: 0,
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
            web: None,
            offscreen: false,
        })
        .collect();
    let mut set_aside: Vec<Pending<T::Node>> = Vec::new();
    let exhaustive = limits.browser == BrowserMode::Exhaustive;
    let mut stand_ins: u32 = 0;
    loop {
        let Some(pending) = stack.pop() else {
            if set_aside.is_empty() {
                break;
            }
            stack = std::mem::take(&mut set_aside);
            stack.reverse();
            continue;
        };
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
        result.web_areas += usize::from(info.role == "AXWebArea");
        let out_of_view = is_out_of_view(info.frame, &pending, exhaustive);
        if out_of_view && !pending.offscreen {
            if pending.web.is_some() && exhaustive {
                // Read again in the second pass, so forget it was seen.
                seen.remove(&pending.node);
                set_aside.push(Pending {
                    offscreen: true,
                    ..pending
                });
            }
            continue;
        }
        let facts = RoleFacts {
            role: &info.role,
            subrole: info.subrole.as_deref(),
            parent_role: pending.parent_role.as_deref(),
            pressable: info.pressable,
        };
        let role = classify(facts);
        if !info.children.is_empty() {
            if pending.depth < pending.max_depth {
                push_children(&mut stack, &pending, &info.role, info.frame, info.children);
            } else if pending.max_depth == limits.max_depth {
                result.truncated.get_or_insert(Truncation::Depth);
            }
        }
        if let Some(role) = role {
            let bounds = match (info.frame, pending.web) {
                (Some(frame), Some(web)) if out_of_view && frame.is_empty() => {
                    stand_ins = stand_ins.saturating_add(1);
                    stand_in(frame, web, stand_ins - 1)
                }
                (frame, _) => frame.unwrap_or_default(),
            };
            result.found.push(Found {
                node: pending.node,
                role,
                name: info.name,
                description: info.description,
                value: info.value,
                bounds,
                enabled: info.enabled,
                pressable: info.pressable,
                offscreen: out_of_view,
                visible: pending.visible,
            });
        }
    }
    Ok(result)
}

/// Queues the `children` of `parent` (role `role`, frame `frame`) so they pop in order.
fn push_children<N>(
    stack: &mut Vec<Pending<N>>,
    parent: &Pending<N>,
    role: &str,
    frame: Option<Rect>,
    children: Vec<N>,
) {
    // A scroll area or a clipping container narrows what is visible below it (not out of
    // view, where the region stays the one the content would be scrolled into).
    let visible = match frame {
        Some(frame) if !frame.is_empty() && clips(role) && !parent.offscreen => {
            intersection(frame, parent.visible)
        }
        _ => parent.visible,
    };
    let web = if role == "AXWebArea" {
        Some(frame.unwrap_or_default())
    } else {
        parent.web
    };
    for child in children.into_iter().rev() {
        stack.push(Pending {
            node: child,
            depth: parent.depth + 1,
            max_depth: parent.max_depth,
            visible,
            parent_role: Some(role.to_owned()),
            web,
            offscreen: parent.offscreen,
        });
    }
}

/// Roles whose frame clips their content.
fn clips(role: &str) -> bool {
    matches!(role, "AXScrollArea" | "AXWindow" | "AXSheet" | "AXDrawer")
}

/// Whether a node with `frame` lies outside the region `pending` is visible in. In exhaustive
/// mode an empty frame in a web document counts too: Chromium reports content scrolled out of
/// view zero pixels high, pinned to the edge it went past (and leaves that stale when the
/// window moves), so the frame says nothing about where the element is.
fn is_out_of_view<N>(frame: Option<Rect>, pending: &Pending<N>, exhaustive: bool) -> bool {
    frame.is_some_and(|frame| {
        if frame.is_empty() {
            exhaustive && pending.web.is_some()
        } else {
            !frame.intersects(&pending.visible)
        }
    })
}

/// A one-point-high frame standing in for the `index`-th element with an empty `reported`
/// frame in the web area `web`: just past the edge it is nearer to, one point further out per
/// element, so every such element keeps a target of its own (until it is scrolled into view
/// and read again).
fn stand_in(reported: Rect, web: Rect, index: u32) -> Rect {
    let offset = i64::from(index);
    let below = i64::from(reported.y) > i64::from(web.y) + i64::from(web.height / 2);
    let y = if below {
        i64::from(web.y) + i64::from(web.height) + offset
    } else {
        i64::from(web.y) - 1 - offset
    };
    let y = i32::try_from(y).unwrap_or(if y < 0 { i32::MIN } else { i32::MAX });
    Rect::new(reported.x, y, reported.width.max(1), 1)
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

/// The raw text attributes of one node.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RawTexts<'a> {
    /// `AXTitle`.
    pub title: Option<&'a str>,
    /// `AXDescription`.
    pub description: Option<&'a str>,
    /// `AXHelp` (tooltips).
    pub help: Option<&'a str>,
}

/// The name and description of a node: the name is the title, else the description; the
/// description holds whatever of the description and help text the name does not.
pub fn texts(raw: RawTexts<'_>) -> (String, String) {
    fn present(text: Option<&str>) -> Option<&str> {
        text.map(str::trim).filter(|text| !text.is_empty())
    }
    let title = present(raw.title);
    let description = present(raw.description);
    let name = title.or(description).unwrap_or_default();
    let mut extra: Vec<&str> = Vec::new();
    for text in [description, present(raw.help)].into_iter().flatten() {
        if text != name && !extra.contains(&text) {
            extra.push(text);
        }
    }
    (name.to_owned(), extra.join(" "))
}

/// What kind of value a node has for search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// None is read (secure text fields among them: their contents never leave the app).
    None,
    /// A single-line text field's or combo box's text.
    Text,
    /// A slider's or stepper's number.
    Number,
}

impl ValueKind {
    /// The value kind of an element with `AXRole` `role` and `AXSubrole` `subrole`.
    pub fn of(role: &str, subrole: Option<&str>) -> Self {
        if subrole == Some("AXSecureTextField") {
            return Self::None;
        }
        match role {
            "AXTextField" | "AXComboBox" => Self::Text,
            "AXSlider" | "AXIncrementor" => Self::Number,
            _ => Self::None,
        }
    }
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
    if matches!(role, "AXTable" | "AXOutline" | "AXList" | "AXBrowser")
        && let Some(rows) = visible_rows.filter(|rows| !rows.is_empty())
    {
        return rows;
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
                    description: String::new(),
                    value: String::new(),
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

    /// window(1) > [web area(2) > [link above the viewport(3), group below(4) > link(5),
    /// visible link(6)], native button off screen(7)]
    fn long_page() -> FakeTree {
        let mut tree = FakeTree::default();
        tree.add(1, "AXWindow", SCREEN, &[2, 7]);
        tree.add(2, "AXWebArea", SCREEN, &[3, 4, 6]);
        tree.add(3, "AXLink", Rect::new(10, -500, 20, 20), &[]);
        tree.add(4, "AXGroup", Rect::new(0, 2000, 1000, 900), &[5]);
        tree.add(5, "AXLink", Rect::new(10, 2500, 20, 20), &[]);
        tree.add(6, "AXLink", SMALL, &[]);
        tree.add(7, "AXButton", Rect::new(3000, 10, 20, 20), &[]);
        tree
    }

    #[test]
    fn fast_prunes_the_page_outside_the_viewport() {
        let walk = walk(
            &mut long_page(),
            vec![root(1)],
            Limits::default(),
            no_time(),
        )
        .unwrap();
        assert_eq!(ids(&walk), [6]);
        assert!(!walk.found[0].offscreen);
    }

    #[test]
    fn counts_web_areas() {
        let page = walk(
            &mut long_page(),
            vec![root(1)],
            Limits::default(),
            no_time(),
        )
        .unwrap();
        assert_eq!(page.web_areas, 1);
        // A Chromium window whose web content has no accessibility tree: only its toolbar.
        let mut tree = FakeTree::default();
        tree.add(1, "AXWindow", SCREEN, &[2]);
        tree.add(2, "AXToolbar", SCREEN, &[3]);
        tree.add(3, "AXButton", SMALL, &[]);
        let bare = walk(&mut tree, vec![root(1)], Limits::default(), no_time()).unwrap();
        assert_eq!(ids(&bare), [3]);
        assert_eq!(bare.web_areas, 0);
    }

    #[test]
    fn empty_web_frames_get_stand_ins_in_exhaustive_mode() {
        // Chromium reports content scrolled out of view zero pixels high on the web area's
        // edge it went past (stale once the window moved): two links below on the same spot,
        // one above, one left mid-area. A native zero-size button keeps its empty frame.
        let web = Rect::new(0, 100, 1000, 500);
        let mut tree = FakeTree::default();
        tree.add(1, "AXWindow", SCREEN, &[2, 7]);
        tree.add(2, "AXWebArea", web, &[3, 4, 5, 6]);
        tree.add(3, "AXLink", Rect::new(10, 600, 40, 0), &[]);
        tree.add(4, "AXLink", Rect::new(10, 600, 40, 0), &[]);
        tree.add(5, "AXLink", Rect::new(10, 100, 40, 0), &[]);
        tree.add(6, "AXLink", Rect::new(10, 300, 40, 0), &[]);
        tree.add(7, "AXButton", Rect::new(10, 700, 0, 0), &[]);
        let exhaustive = Limits {
            browser: BrowserMode::Exhaustive,
            ..Limits::default()
        };
        let found = walk(&mut tree, vec![root(1)], exhaustive, no_time()).unwrap();
        assert_eq!(ids(&found), [7, 3, 4, 5, 6]);
        let bounds: Vec<(Rect, bool)> = found
            .found
            .iter()
            .map(|f| (f.bounds, f.offscreen))
            .collect();
        assert_eq!(
            bounds,
            [
                (Rect::new(10, 700, 0, 0), false),
                (Rect::new(10, 600, 40, 1), true),
                (Rect::new(10, 601, 40, 1), true),
                (Rect::new(10, 97, 40, 1), true),
                (Rect::new(10, 96, 40, 1), true),
            ]
        );
        // Fast mode leaves them as they are: empty, so they get no label.
        let found = walk(&mut tree, vec![root(1)], Limits::default(), no_time()).unwrap();
        assert_eq!(ids(&found), [3, 4, 5, 6, 7]);
        assert!(
            found
                .found
                .iter()
                .all(|f| !f.offscreen && f.bounds.is_empty())
        );
    }

    #[test]
    fn exhaustive_walks_the_rest_of_the_page_last() {
        let exhaustive = Limits {
            browser: BrowserMode::Exhaustive,
            ..Limits::default()
        };
        let mut tree = long_page();
        let found = walk(&mut tree, vec![root(1)], exhaustive, no_time()).unwrap();
        // The visible link first; then the page above and below it; never native windows'
        // off-screen controls.
        assert_eq!(ids(&found), [6, 3, 5]);
        let offscreen: Vec<bool> = found.found.iter().map(|f| f.offscreen).collect();
        assert_eq!(offscreen, [false, true, true]);
        assert!(found.found.iter().all(|f| f.visible == SCREEN));
        assert_eq!(found.truncated, None);

        // A budget that runs out in the second pass still covers what is visible.
        let tight = Limits {
            max_nodes: 5,
            ..exhaustive
        };
        let found = walk(&mut long_page(), vec![root(1)], tight, no_time()).unwrap();
        assert_eq!(ids(&found), [6]);
        assert_eq!(found.truncated, Some(Truncation::Nodes));
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

    #[test]
    fn names_descriptions_and_tooltips() {
        let raw = |title, description, help| RawTexts {
            title,
            description,
            help,
        };
        // A tooltip-only button: no name, the tooltip is its description.
        assert_eq!(
            texts(raw(None, None, Some("Keep a copy"))),
            (String::new(), "Keep a copy".to_owned())
        );
        assert_eq!(
            texts(raw(
                Some("Save"),
                Some("Saves the file"),
                Some("Saves the file")
            )),
            ("Save".to_owned(), "Saves the file".to_owned())
        );
        // The description becomes the name when there is no title, and is not repeated.
        assert_eq!(
            texts(raw(Some(" "), Some("Close"), Some("Close the window"))),
            ("Close".to_owned(), "Close the window".to_owned())
        );
        assert_eq!(texts(RawTexts::default()), (String::new(), String::new()));
    }

    #[test]
    fn values_are_read_from_fields_and_sliders_but_never_secure_fields() {
        assert_eq!(ValueKind::of("AXTextField", None), ValueKind::Text);
        assert_eq!(
            ValueKind::of("AXTextField", Some("AXSearchField")),
            ValueKind::Text
        );
        assert_eq!(
            ValueKind::of("AXTextField", Some("AXSecureTextField")),
            ValueKind::None
        );
        assert_eq!(ValueKind::of("AXSlider", None), ValueKind::Number);
        assert_eq!(ValueKind::of("AXTextArea", None), ValueKind::None);
        assert_eq!(ValueKind::of("AXButton", None), ValueKind::None);
    }
}
