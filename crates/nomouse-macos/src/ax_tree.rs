//! [`AxTree`] over live `AXUIElement`s, plus the roots of a walk (focused window, menu bar,
//! open menus, menu bar extras).

use std::collections::HashSet;

use nomouse_core::{PlatformError, Rect};
use nomouse_sys_macos::AxError;
use nomouse_sys_macos::app::{RunningApp, running_apps};
use nomouse_sys_macos::ax::{AxElement, AxValue};

use crate::geometry::ax_rect;
use crate::roles::PRESS;
use crate::walk::{AxTree, ChildLists, NodeInfo, ReadError, Root, pick_children};

/// Scalar attributes read in one batch per node; indices below follow this order.
const SCALARS: [&str; 7] = [
    "AXRole",
    "AXSubrole",
    "AXTitle",
    "AXDescription",
    "AXEnabled",
    "AXPosition",
    "AXSize",
];

/// Roles that are clickable only when they list `AXPress`, so their actions are read.
const MAYBE_PRESSABLE: [&str; 7] = [
    "AXGroup",
    "AXImage",
    "AXStaticText",
    "AXCell",
    "AXUnknown",
    "AXLayoutItem",
    "AXWebArea",
];

/// Per-app messaging timeout for menu bar extras of background apps, in seconds.
const EXTRAS_TIMEOUT: f32 = 0.1;

/// Reads live accessibility nodes.
#[derive(Debug, Default)]
pub(crate) struct LiveTree;

fn fatal_or_skip(error: AxError) -> ReadError {
    match error {
        AxError::ApiDisabled => ReadError::Fatal(not_trusted()),
        _ => ReadError::Skip,
    }
}

/// The error for a process without Accessibility trust.
pub(crate) fn not_trusted() -> PlatformError {
    PlatformError::PermissionDenied(crate::ACCESSIBILITY_REQUIREMENT.to_owned())
}

fn string(value: Option<&AxValue>) -> Option<String> {
    match value {
        Some(AxValue::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn elements(value: Option<AxValue>) -> Option<Vec<AxElement>> {
    match value {
        Some(AxValue::Array(items)) => Some(
            items
                .into_iter()
                .filter_map(|item| match item {
                    AxValue::Element(element) => Some(element),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    }
}

fn frame(position: Option<&AxValue>, size: Option<&AxValue>) -> Option<Rect> {
    match (position, size) {
        (Some(&AxValue::Point { x, y }), Some(&AxValue::Size { width, height })) => {
            Some(ax_rect(x, y, width, height))
        }
        _ => None,
    }
}

fn read_list(element: &AxElement, attribute: &str) -> Option<Vec<AxElement>> {
    elements(element.attribute(attribute).ok().flatten())
}

impl AxTree for LiveTree {
    type Node = AxElement;

    fn read(&mut self, node: &AxElement) -> Result<NodeInfo<AxElement>, ReadError> {
        let values = node.attributes(&SCALARS).map_err(fatal_or_skip)?;
        let get = |index: usize| values.get(index).and_then(Option::as_ref);
        let role = string(get(0)).unwrap_or_default();
        let subrole = string(get(1));
        let name = string(get(2))
            .or_else(|| string(get(3)))
            .unwrap_or_default();
        let enabled = !matches!(get(4), Some(AxValue::Bool(false)));
        let frame = frame(get(5), get(6));
        let pressable = if MAYBE_PRESSABLE.contains(&role.as_str()) {
            node.action_names()
                .is_ok_and(|actions| actions.iter().any(|action| action == PRESS))
        } else {
            false
        };
        let table_like = matches!(
            role.as_str(),
            "AXTable" | "AXOutline" | "AXList" | "AXBrowser"
        );
        let lists = if table_like {
            ChildLists {
                visible_rows: read_list(node, "AXVisibleRows"),
                // Column headers and scroll bars sit outside the row list.
                children: read_list(node, "AXChildren")
                    .map(|children| children.into_iter().take(64).collect()),
                ..ChildLists::default()
            }
        } else if matches!(role.as_str(), "AXWindow" | "AXWebArea") {
            ChildLists {
                navigation_order: read_list(node, "AXChildrenInNavigationOrder"),
                children: read_list(node, "AXChildren"),
                ..ChildLists::default()
            }
        } else {
            ChildLists {
                children: read_list(node, "AXChildren"),
                ..ChildLists::default()
            }
        };
        let mut children = pick_children(&role, lists);
        if table_like {
            // Rows come first; drop any rows the truncated child list repeated.
            let mut seen = HashSet::new();
            children.retain(|child| seen.insert(child.clone()));
        }
        Ok(NodeInfo {
            role,
            subrole,
            name,
            enabled,
            frame,
            pressable,
            children,
        })
    }
}

/// Where to look for elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootSet {
    /// The frontmost app's focused window.
    FocusedWindow,
    /// Every visible window of every regular app.
    AllWindows,
}

/// Roots of one walk, and whether a focused window was found.
#[derive(Debug)]
pub(crate) struct Roots {
    /// The walk roots, windows first.
    pub(crate) roots: Vec<Root<AxElement>>,
    /// Whether any window root was found.
    pub(crate) has_window: bool,
}

fn role_of(element: &AxElement) -> Option<String> {
    string(element.attribute("AXRole").ok().flatten().as_ref())
}

fn is_minimized(window: &AxElement) -> bool {
    matches!(
        window.attribute("AXMinimized"),
        Ok(Some(AxValue::Bool(true)))
    )
}

/// Collects the walk roots.
///
/// # Errors
/// [`PlatformError::PermissionDenied`] when Accessibility trust is missing.
pub(crate) fn roots(
    set: RootSet,
    frontmost: Option<i32>,
    visible: Rect,
) -> Result<Roots, PlatformError> {
    let deep = |node: AxElement| Root {
        node,
        visible,
        max_depth: usize::MAX,
    };
    let shallow = |node: AxElement| Root {
        node,
        visible,
        max_depth: 1,
    };
    let apps = running_apps();
    let mut roots = Vec::new();
    let mut has_window = false;
    match set {
        RootSet::FocusedWindow => {
            if let Some(pid) = frontmost {
                let app = AxElement::application(pid);
                match app.attribute("AXFocusedWindow") {
                    Ok(Some(AxValue::Element(window))) => {
                        roots.push(deep(window));
                        has_window = true;
                    }
                    Err(AxError::ApiDisabled) => return Err(not_trusted()),
                    _ => {}
                }
            }
        }
        RootSet::AllWindows => {
            for app in apps.iter().filter(|app| app.regular) {
                let element = AxElement::application(app.pid);
                match element.attribute("AXWindows") {
                    Ok(value) => {
                        for window in elements(value).unwrap_or_default() {
                            if !is_minimized(&window) {
                                roots.push(deep(window));
                                has_window = true;
                            }
                        }
                    }
                    Err(AxError::ApiDisabled) => return Err(not_trusted()),
                    Err(_) => {}
                }
            }
        }
    }
    if let Some(pid) = frontmost {
        let app = AxElement::application(pid);
        // Open context and pop-up menus are children of the application element.
        for child in read_list(&app, "AXChildren").unwrap_or_default() {
            if role_of(&child).as_deref() == Some("AXMenu") {
                roots.push(deep(child));
            }
        }
        if let Ok(Some(AxValue::Element(menu_bar))) = app.attribute("AXMenuBar") {
            roots.push(shallow(menu_bar));
        }
    }
    roots.extend(menu_extras(&apps).into_iter().map(shallow));
    Ok(Roots { roots, has_window })
}

/// The menu bar extras (status items) of every app; most live in background agents such as
/// Control Center and `SystemUIServer`.
fn menu_extras(apps: &[RunningApp]) -> Vec<AxElement> {
    apps.iter()
        .filter_map(|app| {
            let element = AxElement::application(app.pid);
            // Ignore failures: a slow agent only loses its extras.
            let _ = element.set_messaging_timeout(EXTRAS_TIMEOUT);
            match element.attribute("AXExtrasMenuBar") {
                Ok(Some(AxValue::Element(bar))) => Some(bar),
                _ => None,
            }
        })
        .collect()
}
