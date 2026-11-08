//! Coordinate conversion between the two macOS screen spaces.
//!
//! Accessibility and Quartz events use *global top-left* coordinates: the origin is the top
//! left of the primary (menu bar) screen and y grows downwards. AppKit windows use *Cocoa*
//! coordinates: the origin is the bottom left of the primary screen and y grows upwards; each
//! `NSScreen` frame is given in that space. Both are in points, so nomouse's [`Rect`]s on
//! macOS are points too (the overlay draws at the screen's backing scale).

use nomouse_core::{Point, Rect};
use nomouse_sys_macos::ScreenFrame;
use num_traits::ToPrimitive;

/// Rounds a float coordinate to the nearest `i32`, saturating; NaN becomes 0.
fn to_i32(value: f64) -> i32 {
    let clamped = value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    clamped.to_i32().unwrap_or(0)
}

/// Rounds a float length to `u32`, saturating; negatives and NaN become 0.
fn to_u32(value: f64) -> u32 {
    value
        .round()
        .clamp(0.0, f64::from(u32::MAX))
        .to_u32()
        .unwrap_or(0)
}

/// Builds a [`Rect`] from an Accessibility `AXPosition` and `AXSize`.
pub fn ax_rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect::new(to_i32(x), to_i32(y), to_u32(width), to_u32(height))
}

/// A point in one panel's view coordinates (origin bottom-left of that screen, y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelPoint {
    /// Index of the screen (in the order the screens were given).
    pub screen: usize,
    /// Horizontal offset from the screen's left edge.
    pub x: f64,
    /// Vertical offset from the screen's bottom edge.
    pub y: f64,
}

/// Converts a global top-left `point` to the panel coordinates of the screen containing it.
///
/// `screens` lists Cocoa frames with the primary screen first (as `NSScreen.screens` does).
/// Returns `None` when no screen contains the point or `screens` is empty.
pub fn to_panel(point: Point, screens: &[ScreenFrame]) -> Option<PanelPoint> {
    let primary = screens.first()?;
    let x = f64::from(point.x);
    let cocoa_y = primary.y + primary.height - f64::from(point.y);
    screens.iter().enumerate().find_map(|(index, screen)| {
        let inside_x = x >= screen.x && x < screen.x + screen.width;
        // A top-left point lies on the row just below the top edge, so the top is inclusive
        // and the bottom exclusive in Cocoa's upward axis.
        let inside_y = cocoa_y > screen.y && cocoa_y <= screen.y + screen.height;
        (inside_x && inside_y).then_some(PanelPoint {
            screen: index,
            x: x - screen.x,
            y: cocoa_y - screen.y,
        })
    })
}

/// The global top-left bounds of a screen given in Cocoa coordinates.
pub fn screen_rect(screen: ScreenFrame, primary: ScreenFrame) -> Rect {
    let top = primary.y + primary.height - (screen.y + screen.height);
    ax_rect(screen.x, top, screen.width, screen.height)
}

/// Converts wheel steps in nomouse's convention (positive `dy` scrolls down, positive `dx`
/// right) to Quartz wheel values (vertical, horizontal), where positive scrolls up and left.
pub const fn wheel_deltas(dx: i32, dy: i32) -> (i32, i32) {
    (dy.saturating_neg(), dx.saturating_neg())
}

/// The smallest rectangle containing both `a` and `b`.
pub fn union(a: Rect, b: Rect) -> Rect {
    let left = a.x.min(b.x);
    let top = a.y.min(b.y);
    let right = (i64::from(a.x) + i64::from(a.width)).max(i64::from(b.x) + i64::from(b.width));
    let bottom = (i64::from(a.y) + i64::from(a.height)).max(i64::from(b.y) + i64::from(b.height));
    let width = u32::try_from(right - i64::from(left)).unwrap_or(u32::MAX);
    let height = u32::try_from(bottom - i64::from(top)).unwrap_or(u32::MAX);
    Rect::new(left, top, width, height)
}

/// The coordinate `step` of `steps` along the way from `from` to `to` (`step == steps` is
/// exactly `to`); `steps == 0` yields `to`.
pub fn lerp(from: i32, to: i32, step: i32, steps: i32) -> i32 {
    if steps == 0 {
        return to;
    }
    let value =
        i64::from(from) + (i64::from(to) - i64::from(from)) * i64::from(step) / i64::from(steps);
    i32::try_from(value).unwrap_or(to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_covers_both() {
        let a = Rect::new(0, 0, 100, 100);
        let b = Rect::new(-50, 20, 20, 200);
        assert_eq!(union(a, b), Rect::new(-50, 0, 150, 220));
    }

    #[test]
    fn interpolation() {
        assert_eq!(lerp(0, 120, 0, 12), 0);
        assert_eq!(lerp(0, 120, 6, 12), 60);
        assert_eq!(lerp(0, 120, 12, 12), 120);
        assert_eq!(lerp(100, -20, 12, 12), -20);
        assert_eq!(lerp(5, 9, 0, 0), 9);
    }

    const PRIMARY: ScreenFrame = ScreenFrame {
        x: 0.0,
        y: 0.0,
        width: 1440.0,
        height: 900.0,
    };
    /// A second screen to the right, 1080 tall, its bottom aligned 100 points below the
    /// primary's bottom.
    const RIGHT: ScreenFrame = ScreenFrame {
        x: 1440.0,
        y: -100.0,
        width: 1920.0,
        height: 1080.0,
    };
    /// A screen above the primary.
    const ABOVE: ScreenFrame = ScreenFrame {
        x: 0.0,
        y: 900.0,
        width: 1440.0,
        height: 900.0,
    };

    #[test]
    fn rounds_ax_rects() {
        assert_eq!(ax_rect(10.4, 20.6, 30.5, -2.0), Rect::new(10, 21, 31, 0));
    }

    #[test]
    fn primary_screen_flips_y() {
        let point = to_panel(Point::new(100, 50), &[PRIMARY]).unwrap();
        assert_eq!(
            point,
            PanelPoint {
                screen: 0,
                x: 100.0,
                y: 850.0
            }
        );
        let top_left = to_panel(Point::new(0, 0), &[PRIMARY]).unwrap();
        assert_eq!(top_left.y, 900.0);
        assert_eq!(to_panel(Point::new(0, 900), &[PRIMARY]), None);
    }

    #[test]
    fn secondary_screens() {
        let screens = [PRIMARY, RIGHT, ABOVE];
        // Global top-left y of RIGHT's top edge: 900 - (-100 + 1080) = -80.
        let right = to_panel(Point::new(1500, -80), &screens).unwrap();
        assert_eq!(
            right,
            PanelPoint {
                screen: 1,
                x: 60.0,
                y: 1080.0
            }
        );
        let above = to_panel(Point::new(10, -10), &screens).unwrap();
        assert_eq!(
            above,
            PanelPoint {
                screen: 2,
                x: 10.0,
                y: 10.0
            }
        );
        assert_eq!(to_panel(Point::new(-5, 10), &screens), None);
        assert_eq!(to_panel(Point::new(0, 0), &[]), None);
    }

    #[test]
    fn screen_rects_in_global_space() {
        assert_eq!(screen_rect(PRIMARY, PRIMARY), Rect::new(0, 0, 1440, 900));
        assert_eq!(
            screen_rect(RIGHT, PRIMARY),
            Rect::new(1440, -80, 1920, 1080)
        );
        assert_eq!(screen_rect(ABOVE, PRIMARY), Rect::new(0, -900, 1440, 900));
    }

    #[test]
    fn wheel_direction() {
        assert_eq!(wheel_deltas(0, 3), (-3, 0));
        assert_eq!(wheel_deltas(-2, 0), (0, 2));
        assert_eq!(wheel_deltas(0, i32::MIN), (i32::MAX, 0));
    }
}
