//! Which windows a [`Scope`](nomouse_core::Scope) covers and which screen areas hide their
//! elements.

use nomouse_sys_windows::{Edges, RawElement, WindowHandle, WindowInfo};

/// Class of the primary taskbar.
pub(crate) const TASKBAR_CLASS: &str = "Shell_TrayWnd";
/// Class of taskbars on secondary monitors.
pub(crate) const SECONDARY_TASKBAR_CLASS: &str = "Shell_SecondaryTrayWnd";
/// Class of the notification area (system tray) inside the taskbar.
pub(crate) const TRAY_CLASS: &str = "TrayNotifyWnd";
/// Class of popup menus (`#32768`).
const MENU_CLASS: &str = "#32768";

/// Optional parts of [`Scope::Screen`](nomouse_core::Scope::Screen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeOptions {
    /// Label the taskbar (start button, pinned and running apps).
    pub include_taskbar: bool,
    /// Label the notification area (system tray) of the taskbar; needs `include_taskbar`.
    pub include_tray: bool,
}

impl Default for ScopeOptions {
    fn default() -> Self {
        Self {
            include_taskbar: true,
            include_tray: true,
        }
    }
}

/// One window to query and the screen areas where its elements are hidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    /// The window whose subtree is queried.
    pub(crate) handle: WindowHandle,
    /// Areas covered by windows above it or excluded by [`ScopeOptions`].
    pub(crate) blocked: Vec<Edges>,
}

impl Target {
    /// Whether the element's centre is visible: on screen and not blocked.
    pub(crate) fn shows(&self, raw: &RawElement, screen: Edges) -> bool {
        let x = i32::midpoint(raw.bounds.left, raw.bounds.right);
        let y = i32::midpoint(raw.bounds.top, raw.bounds.bottom);
        screen.contains(x, y) && !self.blocked.iter().any(|area| area.contains(x, y))
    }
}

fn is_taskbar(window: &WindowInfo) -> bool {
    window.class_name == TASKBAR_CLASS || window.class_name == SECONDARY_TASKBAR_CLASS
}

/// Whether a window hides what is below it. Tool windows are often invisible helpers, so only
/// regular windows, menus and taskbars occlude.
fn occludes(window: &WindowInfo) -> bool {
    !window.tool_window || window.class_name == MENU_CLASS || is_taskbar(window)
}

fn has_area(window: &WindowInfo) -> bool {
    window.bounds.width() > 0 && window.bounds.height() > 0
}

/// The focused window plus any popup menus above it (menus are separate top-level windows).
/// `windows` is in z-order, topmost first.
pub(crate) fn focused(foreground: WindowHandle, windows: &[WindowInfo]) -> Vec<Target> {
    let menus: Vec<&WindowInfo> = windows
        .iter()
        .take_while(|window| window.handle != foreground)
        .filter(|window| window.class_name == MENU_CLASS && has_area(window))
        .collect();
    let mut targets: Vec<Target> = menus
        .iter()
        .enumerate()
        .map(|(position, menu)| Target {
            handle: menu.handle,
            blocked: menus[..position].iter().map(|above| above.bounds).collect(),
        })
        .collect();
    targets.push(Target {
        handle: foreground,
        blocked: menus.iter().map(|menu| menu.bounds).collect(),
    });
    targets
}

/// Every shown window except `overlay`, each blocked by the occluding windows above it; the
/// taskbar and tray follow `options`. `tray` is the notification area's bounds, when found.
pub(crate) fn screen(
    windows: &[WindowInfo],
    overlay: Option<WindowHandle>,
    options: ScopeOptions,
    tray: Option<Edges>,
) -> Vec<Target> {
    let mut targets = Vec::new();
    let mut above: Vec<Edges> = Vec::new();
    for window in windows {
        if Some(window.handle) == overlay || !has_area(window) {
            continue;
        }
        let taskbar = is_taskbar(window);
        if !taskbar || options.include_taskbar {
            let mut blocked = above.clone();
            if taskbar
                && !options.include_tray
                && let Some(tray) = tray
            {
                blocked.push(tray);
            }
            targets.push(Target {
                handle: window.handle,
                blocked,
            });
        }
        if occludes(window) {
            above.push(window.bounds);
        }
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::tests::raw;

    fn window(id: isize, class: &str, bounds: Edges, tool: bool) -> WindowInfo {
        WindowInfo {
            handle: WindowHandle(id),
            bounds,
            class_name: class.to_owned(),
            title: format!("w{id}"),
            tool_window: tool,
        }
    }

    const SCREEN: Edges = Edges::new(0, 0, 1000, 800);

    #[test]
    fn screen_blocks_lower_windows_and_skips_overlay() {
        let windows = [
            window(9, "nomouse-overlay", SCREEN, true),
            window(1, "App", Edges::new(0, 0, 500, 500), false),
            window(2, "Helper", Edges::new(0, 0, 1000, 800), true),
            window(3, "App", Edges::new(0, 0, 1000, 800), false),
        ];
        let targets = screen(
            &windows,
            Some(WindowHandle(9)),
            ScopeOptions::default(),
            None,
        );
        let handles: Vec<isize> = targets.iter().map(|t| t.handle.0).collect();
        assert_eq!(handles, vec![1, 2, 3]);
        let bottom = &targets[2];
        assert_eq!(bottom.blocked, vec![Edges::new(0, 0, 500, 500)]);
        assert!(!bottom.shows(&raw(0, 50000, Edges::new(10, 10, 20, 20)), SCREEN));
        assert!(bottom.shows(&raw(0, 50000, Edges::new(600, 600, 620, 620)), SCREEN));
        assert!(!bottom.shows(&raw(0, 50000, Edges::new(990, 790, 1100, 900)), SCREEN));
    }

    #[test]
    fn taskbar_and_tray_options() {
        let windows = [
            window(1, TASKBAR_CLASS, Edges::new(0, 760, 1000, 800), true),
            window(2, "App", SCREEN, false),
        ];
        let tray = Some(Edges::new(800, 760, 1000, 800));
        let none = ScopeOptions {
            include_taskbar: false,
            include_tray: false,
        };
        let targets = screen(&windows, None, none, tray);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].blocked, vec![Edges::new(0, 760, 1000, 800)]);

        let no_tray = ScopeOptions {
            include_taskbar: true,
            include_tray: false,
        };
        let targets = screen(&windows, None, no_tray, tray);
        assert_eq!(targets[0].blocked, vec![Edges::new(800, 760, 1000, 800)]);
        assert!(!targets[0].shows(&raw(0, 50000, Edges::new(900, 770, 910, 780)), SCREEN));
        assert!(targets[0].shows(&raw(0, 50000, Edges::new(100, 770, 110, 780)), SCREEN));

        let all = screen(&windows, None, ScopeOptions::default(), tray);
        assert_eq!(all[0].blocked, [] as [nomouse_sys_windows::Edges; 0]);
    }

    #[test]
    fn focused_includes_menus_above() {
        let windows = [
            window(5, MENU_CLASS, Edges::new(10, 10, 100, 200), true),
            window(6, "Other", SCREEN, false),
            window(7, "App", SCREEN, false),
            window(8, MENU_CLASS, Edges::new(0, 0, 50, 50), true),
        ];
        let targets = focused(WindowHandle(7), &windows);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].handle, WindowHandle(5));
        assert_eq!(targets[0].blocked, [] as [nomouse_sys_windows::Edges; 0]);
        assert_eq!(targets[1].handle, WindowHandle(7));
        assert_eq!(targets[1].blocked, vec![Edges::new(10, 10, 100, 200)]);
    }
}
