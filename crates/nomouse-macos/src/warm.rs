//! A browser's page: finding it under a window, and keeping its tree built while the browser
//! is frontmost.
//!
//! Chromium builds the accessibility tree of a page only while an assistive app uses it, and
//! drops it again after some seconds without use. A hotkey pressed then found only the
//! browser's own controls, and the page's links and buttons came a second or more later.
//! While a listed web app is frontmost, the daemon therefore reads its page's web area about
//! once a second, which keeps the tree ready for the first pass; and the first pass walks the
//! page before the toolbar, so its time budget goes to the page.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use nomouse_core::Rect;

use crate::walk::{AxTree, ReadError, Root, clips, intersection};

/// How often the frontmost web app's page is touched.
pub const WARM_EVERY: Duration = Duration::from_secs(1);

/// At most this many nodes are read looking for the page under a window: the toolbar and tab
/// strip come first, the page soon after.
pub const FIND_NODES: usize = 400;

/// When the next touch is due.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ticker {
    last: Option<Instant>,
}

impl Ticker {
    /// Whether a touch is due at `now` ([`WARM_EVERY`] after the last one, or the first);
    /// a due touch is counted as done.
    pub fn take_due(&mut self, now: Instant) -> bool {
        let due = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= WARM_EVERY);
        if due {
            self.last = Some(now);
        }
        due
    }
}

/// A page found under a window: its `AXWebArea` node, the region its content is visible in
/// (the window and the scroll areas around it, as the walk narrows it) and its depth below
/// the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<N> {
    /// The `AXWebArea`.
    pub node: N,
    /// Where its content is visible.
    pub visible: Rect,
    /// Levels below the window.
    pub depth: usize,
}

/// The first `AXWebArea` under `root` (visible within `visible`) in breadth-first order,
/// reading at most `max_nodes` nodes; `None` when there is none that close.
pub fn find_web_area<T: AxTree>(
    tree: &mut T,
    root: T::Node,
    visible: Rect,
    max_nodes: usize,
) -> Option<Page<T::Node>> {
    let mut queue = VecDeque::from([(root, visible, 0)]);
    let mut seen = HashSet::new();
    let mut read = 0;
    while let Some((node, visible, depth)) = queue.pop_front() {
        if read >= max_nodes {
            return None;
        }
        if !seen.insert(node.clone()) {
            continue;
        }
        read += 1;
        let info = match tree.read(&node) {
            Ok(info) => info,
            Err(ReadError::Skip | ReadError::Fatal(_)) => continue,
        };
        if info.role == "AXWebArea" {
            return Some(Page {
                node,
                visible,
                depth,
            });
        }
        let inner = match info.frame {
            Some(frame) if !frame.is_empty() && clips(&info.role) => intersection(frame, visible),
            _ => visible,
        };
        queue.extend(
            info.children
                .into_iter()
                .map(|child| (child, inner, depth + 1)),
        );
    }
    None
}

/// `roots` with the page under the first one (the focused window) put in front, so a walk
/// with a tight budget spends it on the page's links and buttons before the browser's toolbar
/// and tab strip. The page keeps the region and the depth limit it has under the window, and
/// the window's walk skips it as already seen. Without a page the roots stay as they are.
pub fn page_first<T: AxTree>(tree: &mut T, mut roots: Vec<Root<T::Node>>) -> Vec<Root<T::Node>> {
    let Some(window) = roots.first() else {
        return roots;
    };
    let max_depth = window.max_depth;
    if let Some(page) = find_web_area(tree, window.node.clone(), window.visible, FIND_NODES) {
        roots.insert(
            0,
            Root {
                node: page.node,
                visible: page.visible,
                max_depth: max_depth.saturating_sub(page.depth),
            },
        );
    }
    roots
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::walk::{Limits, NodeInfo, Walk, walk};

    #[derive(Default)]
    struct Tree {
        nodes: HashMap<u32, (&'static str, Rect, Vec<u32>)>,
        reads: usize,
    }

    impl AxTree for Tree {
        type Node = u32;

        fn read(&mut self, node: &u32) -> Result<NodeInfo<u32>, ReadError> {
            self.reads += 1;
            let (role, frame, children) = self.nodes.get(node).cloned().ok_or(ReadError::Skip)?;
            Ok(NodeInfo {
                role: role.to_owned(),
                subrole: None,
                name: String::new(),
                description: String::new(),
                value: String::new(),
                enabled: true,
                frame: Some(frame),
                pressable: false,
                children,
            })
        }
    }

    const SCREEN: Rect = Rect::new(0, 0, 2000, 2000);
    const WINDOW: Rect = Rect::new(100, 100, 1000, 800);
    const VIEWPORT: Rect = Rect::new(100, 200, 1000, 700);

    /// A browser window: a tab strip with many tabs, then the page in a scroll area.
    fn browser() -> Tree {
        let mut tree = Tree::default();
        let tabs: Vec<u32> = (100..130).collect();
        let tab = Rect::new(100, 100, 30, 20);
        tree.nodes.insert(1, ("AXWindow", WINDOW, vec![2, 3]));
        tree.nodes.insert(2, ("AXTabGroup", WINDOW, tabs.clone()));
        for id in tabs {
            tree.nodes.insert(id, ("AXRadioButton", tab, Vec::new()));
        }
        tree.nodes.insert(3, ("AXScrollArea", VIEWPORT, vec![4]));
        // The document is taller than the viewport.
        let document = Rect::new(100, 200, 1000, 5000);
        tree.nodes.insert(4, ("AXWebArea", document, vec![5]));
        tree.nodes
            .insert(5, ("AXLink", Rect::new(110, 210, 50, 20), Vec::new()));
        tree
    }

    #[test]
    fn the_page_is_found_under_the_window_within_the_budget() {
        let mut tree = browser();
        let page = find_web_area(&mut tree, 1, SCREEN, FIND_NODES);
        // Visible where the walk would see it: the window narrowed by the scroll area.
        let expected = Page {
            node: 4,
            visible: VIEWPORT,
            depth: 2,
        };
        assert_eq!(page, Some(expected));
        // Breadth first: the page sits two levels down, before the tabs' children.
        assert!(tree.reads < 40, "{} reads", tree.reads);
        assert_eq!(find_web_area(&mut browser(), 1, SCREEN, 3), None);
        assert_eq!(find_web_area(&mut browser(), 5, SCREEN, FIND_NODES), None);
    }

    #[test]
    fn a_short_first_pass_reaches_the_page_when_it_goes_first() {
        let window = Root {
            node: 1,
            visible: SCREEN,
            max_depth: 48,
        };
        let limits = Limits {
            max_nodes: 5,
            ..Limits::default()
        };
        let no_time = || Duration::ZERO;
        // Window first: the budget goes to the tab strip.
        let toolbar_first = walk(&mut browser(), vec![window.clone()], limits, no_time).unwrap();
        assert!(toolbar_first.found.iter().all(|found| found.node != 5));
        // Page first: the link is found first, inside the viewport.
        let mut tree = browser();
        let roots = page_first(&mut tree, vec![window.clone()]);
        assert_eq!(roots[0].node, 4);
        assert_eq!(roots[0].visible, VIEWPORT);
        assert_eq!(roots[0].max_depth, 46);
        assert_eq!(roots[1], window);
        let page = walk(&mut tree, roots, limits, no_time).unwrap();
        assert_eq!(page.found.first().map(|found| found.node), Some(5));
        assert_eq!(page.web_areas, 1);
        // The whole walk finds the same elements either way.
        let unlimited = Limits::default();
        let all = walk(&mut browser(), vec![window.clone()], unlimited, no_time).unwrap();
        let mut tree = browser();
        let roots = page_first(&mut tree, vec![window]);
        let reordered = walk(&mut tree, roots, unlimited, no_time).unwrap();
        let nodes = |walk: &Walk<u32>| {
            let mut nodes: Vec<u32> = walk.found.iter().map(|found| found.node).collect();
            nodes.sort_unstable();
            nodes
        };
        assert_eq!(nodes(&reordered), nodes(&all));
    }

    #[test]
    fn without_a_page_the_roots_stay() {
        let mut tree = Tree::default();
        tree.nodes.insert(1, ("AXWindow", WINDOW, vec![2]));
        tree.nodes
            .insert(2, ("AXButton", Rect::new(110, 110, 20, 20), Vec::new()));
        let window = Root {
            node: 1,
            visible: SCREEN,
            max_depth: 48,
        };
        assert_eq!(page_first(&mut tree, vec![window.clone()]), vec![window]);
        assert_eq!(page_first(&mut tree, Vec::new()), Vec::new());
    }

    #[test]
    fn touches_come_once_a_second() {
        let start = Instant::now();
        let mut ticker = Ticker::default();
        assert!(ticker.take_due(start));
        assert!(!ticker.take_due(start + Duration::from_millis(999)));
        assert!(ticker.take_due(start + Duration::from_millis(1000)));
        assert!(!ticker.take_due(start + Duration::from_millis(1500)));
    }
}
