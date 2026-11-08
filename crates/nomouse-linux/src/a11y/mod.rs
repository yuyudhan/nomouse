//! AT-SPI element source: finds the focused window and collects its clickable elements.

pub(crate) mod bus;
pub(crate) mod walk;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use atspi::Role as AtRole;
use nomouse_core::{Element, ElementId, PlatformError, Point, Rect, Scope};

use bus::{App, Bus, Obj};
use walk::{Found, Limits};

/// Tunables for one element scan.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScanLimits {
    /// Deepest level below a window.
    pub max_depth: usize,
    /// Most objects examined per scan.
    pub max_nodes: usize,
    /// Whole-scan budget.
    pub budget: Duration,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_depth: 40,
            max_nodes: 5000,
            budget: Duration::from_millis(1500),
        }
    }
}

/// Whether a top-level object is a popup that belongs on screen with its window (menus,
/// combo-box lists, popovers).
const fn is_popup(role: AtRole) -> bool {
    matches!(
        role,
        AtRole::Menu | AtRole::PopupMenu | AtRole::Window | AtRole::ToolTip | AtRole::List
    )
}

/// Picks the windows to scan and the area to clip to.
///
/// For [`Scope::FocusedWindow`] the application is the one whose process owns the X active
/// window (`active_pid`), falling back to the application with an `ACTIVE` window; its active
/// window plus any showing popups (open menus) are scanned. For [`Scope::Screen`] every showing
/// top-level of every application is.
pub(crate) fn select_roots(
    apps: &[App],
    active_pid: Option<u32>,
    scope: Scope,
    screen: Rect,
) -> Result<(Vec<Obj>, Rect), PlatformError> {
    if scope == Scope::Screen {
        let roots = apps
            .iter()
            .flat_map(|app| &app.top_levels)
            .filter(|top| top.showing)
            .map(|top| top.obj.clone())
            .collect();
        return Ok((roots, screen));
    }
    let has_active = |app: &&App| app.top_levels.iter().any(|top| top.active);
    let app = active_pid
        .and_then(|pid| {
            apps.iter()
                .filter(|app| app.pid == Some(pid))
                .find(has_active)
                .or_else(|| apps.iter().find(|app| app.pid == Some(pid)))
        })
        .or_else(|| apps.iter().find(has_active))
        .ok_or(PlatformError::NoFocusedWindow)?;
    let active: Vec<_> = app.top_levels.iter().filter(|top| top.active).collect();
    let window = active
        .first()
        .copied()
        .or_else(|| {
            app.top_levels
                .iter()
                .find(|top| top.showing && !is_popup(top.role))
        })
        .ok_or(PlatformError::NoFocusedWindow)?;
    let mut roots = vec![window.obj.clone()];
    let mut clip = window.extents.unwrap_or(screen);
    for popup in app
        .top_levels
        .iter()
        .filter(|top| top.showing && top.obj != window.obj && (top.active || is_popup(top.role)))
    {
        roots.push(popup.obj.clone());
        if let Some(extents) = popup.extents {
            clip = union(clip, extents);
        }
    }
    Ok((roots, intersection(clip, screen).unwrap_or(screen)))
}

fn union(a: Rect, b: Rect) -> Rect {
    let left = a.x.min(b.x);
    let top = a.y.min(b.y);
    let right = (i64::from(a.x) + i64::from(a.width)).max(i64::from(b.x) + i64::from(b.width));
    let bottom = (i64::from(a.y) + i64::from(a.height)).max(i64::from(b.y) + i64::from(b.height));
    Rect::new(
        left,
        top,
        u32::try_from(right - i64::from(left)).unwrap_or(u32::MAX),
        u32::try_from(bottom - i64::from(top)).unwrap_or(u32::MAX),
    )
}

fn intersection(a: Rect, b: Rect) -> Option<Rect> {
    if !a.intersects(&b) {
        return None;
    }
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (i64::from(a.x) + i64::from(a.width)).min(i64::from(b.x) + i64::from(b.width));
    let bottom = (i64::from(a.y) + i64::from(a.height)).min(i64::from(b.y) + i64::from(b.height));
    Some(Rect::new(
        left,
        top,
        u32::try_from(right - i64::from(left)).ok()?,
        u32::try_from(bottom - i64::from(top)).ok()?,
    ))
}

/// Splits `roots` by the process of the application owning each, keeping first-seen order.
fn group_by_pid(apps: &[App], roots: Vec<Obj>) -> Vec<(Option<u32>, Vec<Obj>)> {
    let mut groups: Vec<(Option<u32>, Vec<Obj>)> = Vec::new();
    for root in roots {
        let pid = apps
            .iter()
            .find(|app| app.top_levels.iter().any(|top| top.obj == root))
            .and_then(|app| app.pid);
        match groups.iter_mut().find(|(known, _)| *known == pid) {
            Some((_, group)) => group.push(root),
            None => groups.push((pid, vec![root])),
        }
    }
    groups
}

/// The AT-SPI element source with the last snapshot kept for semantic clicks.
pub(crate) struct AtspiSource {
    bus: Bus,
    limits: ScanLimits,
    last: Vec<Found<Obj>>,
}

impl AtspiSource {
    /// Connects to the accessibility bus.
    ///
    /// # Errors
    /// [`PlatformError::AccessibilityUnavailable`] when the bus is unreachable.
    pub(crate) fn connect(
        limits: ScanLimits,
        call_timeout: Duration,
    ) -> Result<Self, PlatformError> {
        Ok(Self {
            bus: Bus::connect(call_timeout)?,
            limits,
            last: Vec::new(),
        })
    }

    /// Scans `scope`; `active_pid` is the focused window's process, `screen` the desktop bounds.
    ///
    /// `offsets` maps processes whose AT-SPI extents are window-relative (native Wayland
    /// clients) to their window's origin; their elements are moved by it. X11 passes none.
    ///
    /// # Errors
    /// As [`nomouse_core::ElementSource::elements`].
    pub(crate) fn elements(
        &mut self,
        scope: Scope,
        active_pid: Option<u32>,
        screen: Rect,
        offsets: &HashMap<u32, Point>,
    ) -> Result<Vec<Element>, PlatformError> {
        let started = Instant::now();
        let apps = self.bus.applications()?;
        let (roots, clip) = select_roots(&apps, active_pid, scope, screen)?;
        tracing::debug!(
            "found {} applications in {:?}",
            apps.len(),
            started.elapsed()
        );
        let limits = Limits {
            max_depth: self.limits.max_depth,
            max_nodes: self.limits.max_nodes,
            deadline: started + self.limits.budget,
            bulk: true,
        };
        let root_count = roots.len();
        self.last = if offsets.is_empty() {
            walk::collect(&mut self.bus, &roots, clip, limits)
        } else {
            let mut found = Vec::new();
            for (pid, group) in group_by_pid(&apps, roots) {
                let mut part = walk::collect(&mut self.bus, &group, clip, limits);
                if let Some(offset) = pid.and_then(|pid| offsets.get(&pid)) {
                    for item in &mut part {
                        let bounds = &mut item.element.bounds;
                        bounds.x = bounds.x.saturating_add(offset.x);
                        bounds.y = bounds.y.saturating_add(offset.y);
                    }
                }
                found.append(&mut part);
            }
            for (index, item) in found.iter_mut().enumerate() {
                item.element.id = ElementId(u64::try_from(index).unwrap_or(u64::MAX));
            }
            found
        };
        tracing::debug!(
            "scanned {} roots in {:?}: {} elements",
            root_count,
            started.elapsed(),
            self.last.len()
        );
        Ok(self
            .last
            .iter()
            .map(|found| found.element.clone())
            .collect())
    }

    /// Runs the accessibility action of the last snapshot's element targeted at `point`.
    /// Returns `false` when there is none, so the caller falls back to a pointer click.
    pub(crate) fn activate_at(&self, point: Point) -> bool {
        self.last
            .iter()
            .find(|found| found.has_action && found.element.target() == point)
            .is_some_and(|found| self.bus.activate(&found.node))
    }
}

#[cfg(test)]
mod tests {
    use super::bus::TopLevel;
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 1920, 1080);

    fn top(path: &str, role: AtRole, active: bool, extents: Rect) -> TopLevel {
        TopLevel {
            obj: Obj {
                bus: ":1.1".to_owned(),
                path: path.to_owned(),
            },
            role,
            active,
            showing: true,
            extents: Some(extents),
        }
    }

    fn apps() -> Vec<App> {
        vec![
            App {
                pid: Some(10),
                top_levels: vec![top("/a", AtRole::Frame, false, Rect::new(0, 0, 100, 100))],
            },
            App {
                pid: Some(20),
                top_levels: vec![
                    top("/b", AtRole::Frame, true, Rect::new(100, 100, 400, 300)),
                    top(
                        "/menu",
                        AtRole::Window,
                        false,
                        Rect::new(450, 120, 200, 200),
                    ),
                    top("/other", AtRole::Frame, false, Rect::new(0, 0, 10, 10)),
                ],
            },
        ]
    }

    fn paths(roots: &[Obj]) -> Vec<&str> {
        roots.iter().map(|obj| obj.path.as_str()).collect()
    }

    #[test]
    fn focused_window_by_pid_with_menus() {
        let (roots, clip) = select_roots(&apps(), Some(20), Scope::FocusedWindow, SCREEN).unwrap();
        assert_eq!(paths(&roots), vec!["/b", "/menu"]);
        assert_eq!(clip, Rect::new(100, 100, 550, 300));
    }

    #[test]
    fn pid_without_active_window_uses_its_first_window() {
        let (roots, _) = select_roots(&apps(), Some(10), Scope::FocusedWindow, SCREEN).unwrap();
        assert_eq!(paths(&roots), vec!["/a"]);
    }

    #[test]
    fn falls_back_to_active_state() {
        let (roots, _) = select_roots(&apps(), None, Scope::FocusedWindow, SCREEN).unwrap();
        assert_eq!(roots[0].path, "/b");
        let (roots, _) = select_roots(&apps(), Some(99), Scope::FocusedWindow, SCREEN).unwrap();
        assert_eq!(roots[0].path, "/b");
    }

    #[test]
    fn no_focus_is_an_error() {
        let mut list = apps();
        list.remove(1);
        assert_eq!(
            select_roots(&list, None, Scope::FocusedWindow, SCREEN).unwrap_err(),
            PlatformError::NoFocusedWindow
        );
    }

    #[test]
    fn screen_scope_takes_everything_showing() {
        let (roots, clip) = select_roots(&apps(), None, Scope::Screen, SCREEN).unwrap();
        assert_eq!(roots.len(), 4);
        assert_eq!(clip, SCREEN);
    }

    #[test]
    fn rect_helpers() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(union(a, Rect::new(5, 5, 10, 10)), Rect::new(0, 0, 15, 15));
        assert_eq!(
            intersection(a, Rect::new(5, 5, 10, 10)),
            Some(Rect::new(5, 5, 5, 5))
        );
        assert_eq!(intersection(a, Rect::new(20, 20, 1, 1)), None);
    }
}
