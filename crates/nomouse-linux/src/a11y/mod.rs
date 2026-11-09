//! AT-SPI element source: finds the focused window and collects its clickable elements.

pub(crate) mod bus;
pub(crate) mod walk;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use atspi::Role as AtRole;
use nomouse_core::config::{BrowserMode, Config};
use nomouse_core::reveal::settle;
use nomouse_core::{Action, Element, ElementId, PlatformError, Point, Rect, Scope};

use bus::{App, Bus, Obj};
use walk::{Found, Limits};

/// How long a scroll into view may take before the action is given up.
const REVEAL_TIMEOUT: Duration = Duration::from_millis(1500);
/// How often the element's extents are re-read while it scrolls into view.
const REVEAL_POLL: Duration = Duration::from_millis(30);
/// Readings in a row that must agree before the scroll counts as finished: browsers report
/// the final place while their smooth scroll still runs, and ignore a click meanwhile.
const REVEAL_STEADY: u32 = 4;

/// Tunables for one element scan.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScanLimits {
    /// Deepest level below a window.
    pub max_depth: usize,
    /// Most objects examined per scan.
    pub max_nodes: usize,
    /// Whole-scan budget.
    pub budget: Duration,
    /// Whether web content scrolled out of view is collected too (`hints.browser`).
    pub browser: BrowserMode,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_depth: 40,
            max_nodes: 5000,
            budget: Duration::from_millis(1500),
            browser: BrowserMode::Fast,
        }
    }
}

impl ScanLimits {
    /// The default limits with `config`'s browser mode.
    pub(crate) fn for_config(config: &Config) -> Self {
        Self {
            browser: config.hints.browser,
            ..Self::default()
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
    /// The area the last scan was clipped to.
    last_clip: Rect,
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
            last_clip: Rect::default(),
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
        self.last_clip = clip;
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
            browser: self.limits.browser,
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

    /// `action` aimed where it lands once its target is in view: an off-screen element of the
    /// last snapshot ([`Element::offscreen`]) is first scrolled into view (`Component.ScrollTo`);
    /// the action then follows it by as much as its extents moved since the scan (so
    /// window-relative Wayland extents and a window moved meanwhile work too), once they came to
    /// rest. Any other action is returned as it is.
    ///
    /// # Errors
    /// [`PlatformError::Backend`] when the toolkit does not scroll the element into view or it
    /// never comes to rest there: the action would land somewhere else.
    pub(crate) fn reveal(&self, action: Action) -> Result<Action, PlatformError> {
        let at = match action {
            Action::Click { at, .. } | Action::Hover { at } => at,
            Action::Drag { from, .. } => from,
            Action::Scroll { .. } => return Ok(action),
        };
        let Some(found) = self
            .last
            .iter()
            .find(|found| found.element.offscreen && found.element.target() == at)
        else {
            return Ok(action);
        };
        let started = Instant::now();
        reveal_with(
            action,
            found.extents,
            self.last_clip,
            || self.bus.scroll_into_view(&found.node),
            || self.bus.extents(&found.node),
            || {
                std::thread::sleep(REVEAL_POLL);
                started.elapsed() < REVEAL_TIMEOUT
            },
        )
    }
}

/// [`AtspiSource::reveal`] over `scroll` (asks the toolkit to scroll the element, whose extents
/// were `scanned` by the scan, into view), `read` (its extents now) and `wait` (pauses between
/// readings, `false` at the deadline); `clip` is the visible area in the extents' coordinates
/// (window-relative for native Wayland clients, like `scanned`).
fn reveal_with(
    action: Action,
    scanned: Rect,
    clip: Rect,
    scroll: impl FnOnce() -> bool,
    read: impl FnMut() -> Option<Rect>,
    wait: impl FnMut() -> bool,
) -> Result<Action, PlatformError> {
    let (Action::Click { at, .. } | Action::Hover { at } | Action::Drag { from: at, .. }) = action
    else {
        return Ok(action);
    };
    if !scroll() {
        return Err(PlatformError::Backend(
            "the toolkit did not scroll the element into view".to_owned(),
        ));
    }
    let settled = settle(
        read,
        |after| clip.contains(after.center()),
        REVEAL_STEADY,
        wait,
    )
    .ok_or_else(|| {
        PlatformError::Backend(format!(
            "the element did not come to rest in view within {REVEAL_TIMEOUT:?}"
        ))
    })?;
    let moved = follow(at, scanned, settled);
    tracing::debug!("scrolled into view: acting at {moved:?} instead of {at:?}");
    Ok(match action {
        Action::Click {
            button,
            count,
            modifiers,
            ..
        } => Action::Click {
            at: moved,
            button,
            count,
            modifiers,
        },
        Action::Hover { .. } => Action::Hover { at: moved },
        Action::Drag { to, button, .. } => Action::Drag {
            from: moved,
            to,
            button,
        },
        Action::Scroll { .. } => action,
    })
}

/// `at` moved by as much as an element's extents moved from `before` to `after`.
fn follow(at: Point, before: Rect, after: Rect) -> Point {
    let shift = |value: i32, from: i32, to: i32| {
        let moved = i64::from(value) + i64::from(to) - i64::from(from);
        i32::try_from(moved).unwrap_or(if moved < 0 { i32::MIN } else { i32::MAX })
    };
    Point::new(
        shift(at.x, before.x, after.x),
        shift(at.y, before.y, after.y),
    )
}

#[cfg(test)]
mod tests {
    use super::bus::TopLevel;
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 1920, 1080);

    #[test]
    fn actions_follow_an_element_scrolled_into_view() {
        // A link 3000 px below the viewport scrolled up to y = 400; the pointer target moves
        // by as much, whatever the extents' origin (Wayland reports window-relative ones).
        let before = Rect::new(10, 3000, 80, 20);
        let after = Rect::new(10, 400, 80, 20);
        assert_eq!(
            follow(Point::new(150, 3110), before, after),
            Point::new(150, 510)
        );
        assert_eq!(follow(Point::new(5, 5), before, before), Point::new(5, 5));
        let far = Rect::new(0, i32::MIN, 1, 1);
        assert_eq!(
            follow(Point::new(0, 0), far, Rect::new(0, i32::MAX, 1, 1)).y,
            i32::MAX
        );
    }

    fn click(at: Point) -> Action {
        Action::Click {
            at,
            button: nomouse_core::MouseButton::Left,
            count: 1,
            modifiers: nomouse_core::Modifiers::NONE,
        }
    }

    /// `reveal_with` for a link scanned at `scanned` that reads `readings` in turn (then fails
    /// to read), with `polls` polls available.
    fn reveal_readings(
        scanned: Rect,
        readings: &[Rect],
        polls: usize,
    ) -> Result<Action, PlatformError> {
        let mut next = readings.iter().copied();
        let mut left = polls;
        reveal_with(
            click(scanned.center()),
            scanned,
            SCREEN,
            || true,
            || next.next(),
            || {
                let more = left > 0;
                left = left.saturating_sub(1);
                more
            },
        )
    }

    #[test]
    fn a_smooth_scroll_is_waited_out_before_the_click() {
        // Chromium reports the link's extents on every poll while it scrolls from 6000 px below
        // up to y = 400; the click must go where it stops, not where it passed.
        let scanned = Rect::new(10, 6000, 80, 20);
        let at = |y| Rect::new(10, y, 80, 20);
        let readings = [
            at(5200),
            at(3000),
            at(900),
            at(500),
            at(400),
            at(400),
            at(400),
            at(400),
            at(400),
        ];
        assert_eq!(
            reveal_readings(scanned, &readings, 20).unwrap(),
            click(Point::new(50, 410))
        );
    }

    #[test]
    fn a_window_moved_since_the_scan_does_not_misplace_the_click() {
        // The window moved 100 px right and 30 px down after the scan, then the link scrolled
        // to y = 400 (window moved: extents are screen coordinates).
        let scanned = Rect::new(10, 6000, 80, 20);
        let settled = Rect::new(110, 430, 80, 20);
        assert_eq!(
            reveal_readings(scanned, &[settled; 8], 20).unwrap(),
            click(Point::new(150, 440))
        );
    }

    #[test]
    fn native_wayland_extents_are_window_relative() {
        // The element's bounds carry the window's origin (2, 25), its extents do not: the click
        // keeps the origin and moves by the scroll alone.
        let extents = Rect::new(10, 6000, 80, 20);
        let origin = Point::new(2, 25);
        let at = Point::new(50 + origin.x, 6010 + origin.y);
        let settled = Rect::new(10, 949, 80, 20);
        let mut next = std::iter::repeat(settled);
        // The window's clip is window-relative too: its bottom (975) is above the click's
        // screen y (984), which must not count as out of view.
        let clip = Rect::new(0, 0, 1598, 975);
        let result = reveal_with(click(at), extents, clip, || true, || next.next(), || true);
        // The link's raw extents settle at y = 949; on screen its centre sits 25 px lower.
        assert_eq!(result.unwrap(), click(Point::new(52, 984)));
    }

    #[test]
    fn an_element_that_never_comes_to_rest_in_view_is_not_clicked() {
        let scanned = Rect::new(10, 6000, 80, 20);
        // Still below the viewport: the toolkit's extents did not update.
        assert!(reveal_readings(scanned, &[scanned; 20], 20).is_err());
        // Moving on every poll.
        let moving: Vec<Rect> = (0..20)
            .map(|i| Rect::new(10, 900 - i * 7, 80, 20))
            .collect();
        assert!(reveal_readings(scanned, &moving, 20).is_err());
        // The extents cannot be read at all.
        assert!(reveal_readings(scanned, &[], 20).is_err());
    }

    #[test]
    fn a_toolkit_that_refuses_to_scroll_gets_no_click() {
        let scanned = Rect::new(10, 6000, 80, 20);
        let result = reveal_with(
            click(scanned.center()),
            scanned,
            SCREEN,
            || false,
            || Some(Rect::new(10, 400, 80, 20)),
            || true,
        );
        assert!(matches!(result, Err(PlatformError::Backend(_))));
    }

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
