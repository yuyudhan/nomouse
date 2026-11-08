//! Pointer actions that an [`crate::InputInjector`] performs.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::element::Point;

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// Primary button.
    Left,
    /// Secondary button.
    Right,
    /// Middle button.
    Middle,
}

/// A concrete pointer action at screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Move to `at` and click `button` `count` times (2 = double click).
    Click {
        /// Target point.
        at: Point,
        /// Button to press.
        button: MouseButton,
        /// Number of clicks, at least 1.
        count: u8,
    },
    /// Move the pointer to `at` without clicking.
    Hover {
        /// Target point.
        at: Point,
    },
    /// Press `button` at `from`, move to `to`, release.
    Drag {
        /// Start point.
        from: Point,
        /// End point.
        to: Point,
        /// Button held during the drag.
        button: MouseButton,
    },
    /// Scroll at `at` by whole wheel steps; positive `dy` scrolls down, positive `dx` right.
    ///
    /// The steps are final: the binary has already multiplied the direction by
    /// `scroll.step`, so backends send exactly `|dx|` and `|dy|` wheel clicks.
    Scroll {
        /// Point the pointer moves to before scrolling.
        at: Point,
        /// Horizontal steps.
        dx: i32,
        /// Vertical steps.
        dy: i32,
    },
}

impl Action {
    /// Left single click.
    pub const fn click(at: Point) -> Self {
        Self::Click {
            at,
            button: MouseButton::Left,
            count: 1,
        }
    }

    /// Right single click.
    pub const fn right_click(at: Point) -> Self {
        Self::Click {
            at,
            button: MouseButton::Right,
            count: 1,
        }
    }

    /// Left double click.
    pub const fn double_click(at: Point) -> Self {
        Self::Click {
            at,
            button: MouseButton::Left,
            count: 2,
        }
    }

    /// Hover.
    pub const fn hover(at: Point) -> Self {
        Self::Hover { at }
    }

    /// Left-button drag.
    pub const fn drag(from: Point, to: Point) -> Self {
        Self::Drag {
            from,
            to,
            button: MouseButton::Left,
        }
    }

    /// Scroll.
    pub const fn scroll(at: Point, dx: i32, dy: i32) -> Self {
        Self::Scroll { at, dx, dy }
    }

    /// The point the pointer ends at after the action.
    pub const fn end_point(&self) -> Point {
        match *self {
            Self::Click { at, .. } | Self::Hover { at } | Self::Scroll { at, .. } => at,
            Self::Drag { to, .. } => to,
        }
    }
}

/// What selecting a hint does; the user picks one per activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionKind {
    /// Left click.
    #[default]
    Click,
    /// Right click.
    RightClick,
    /// Double click.
    DoubleClick,
    /// Hover.
    Hover,
    /// Drag: two selections, start then end.
    Drag,
    /// Pick a scroll target, then scroll with keys.
    Scroll,
}

impl ActionKind {
    /// All kinds, in documentation order.
    pub const ALL: [Self; 6] = [
        Self::Click,
        Self::RightClick,
        Self::DoubleClick,
        Self::Hover,
        Self::Drag,
        Self::Scroll,
    ];

    /// The kebab-case name used in config and on the command line.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::RightClick => "right-click",
            Self::DoubleClick => "double-click",
            Self::Hover => "hover",
            Self::Drag => "drag",
            Self::Scroll => "scroll",
        }
    }

    /// The action for a single selected point, or `None` for the multi-step kinds (drag, scroll).
    pub const fn at(self, point: Point) -> Option<Action> {
        match self {
            Self::Click => Some(Action::click(point)),
            Self::RightClick => Some(Action::right_click(point)),
            Self::DoubleClick => Some(Action::double_click(point)),
            Self::Hover => Some(Action::hover(point)),
            Self::Drag | Self::Scroll => None,
        }
    }
}

impl fmt::Display for ActionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Error for an unknown action kind name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "unknown action {0:?}; expected one of click, right-click, double-click, hover, drag, scroll"
)]
pub struct UnknownActionKind(pub String);

impl FromStr for ActionKind {
    type Err = UnknownActionKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.name() == s)
            .ok_or_else(|| UnknownActionKind(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors() {
        let p = Point::new(3, 4);
        assert_eq!(
            Action::double_click(p),
            Action::Click {
                at: p,
                button: MouseButton::Left,
                count: 2
            }
        );
        assert_eq!(
            Action::drag(p, Point::new(9, 9)).end_point(),
            Point::new(9, 9)
        );
        assert_eq!(Action::scroll(p, 0, 3).end_point(), p);
    }

    #[test]
    fn kinds_round_trip_by_name() {
        for kind in ActionKind::ALL {
            assert_eq!(kind.name().parse::<ActionKind>(), Ok(kind));
        }
        assert!("tap".parse::<ActionKind>().is_err());
    }

    #[test]
    fn single_point_kinds() {
        let p = Point::new(1, 1);
        assert_eq!(ActionKind::RightClick.at(p), Some(Action::right_click(p)));
        assert_eq!(ActionKind::Drag.at(p), None);
        assert_eq!(ActionKind::Scroll.at(p), None);
    }
}
