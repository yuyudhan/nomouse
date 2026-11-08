//! The element model: what a backend reports for each node of an accessibility tree.

use std::cmp::Ordering;

/// A backend-assigned identifier, unique within one element snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ElementId(pub u64);

/// A point in global screen coordinates (physical pixels, origin top-left).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: i32,
    /// Vertical coordinate.
    pub y: i32,
}

impl Point {
    /// Creates a point.
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle in global screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Rect {
    /// Creates a rectangle.
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Whether the rectangle has no area.
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The centre point, saturating at the coordinate limits.
    pub fn center(&self) -> Point {
        Point::new(
            offset(self.x, i64::from(self.width) / 2),
            offset(self.y, i64::from(self.height) / 2),
        )
    }

    /// Whether `point` lies inside the rectangle (right and bottom edges exclusive).
    pub fn contains(&self, point: Point) -> bool {
        let (px, py) = (i64::from(point.x), i64::from(point.y));
        let (x, y) = (i64::from(self.x), i64::from(self.y));
        px >= x && py >= y && px < x + i64::from(self.width) && py < y + i64::from(self.height)
    }

    /// Whether the two rectangles overlap with a non-empty area.
    pub fn intersects(&self, other: &Self) -> bool {
        let left = i64::from(self.x).max(i64::from(other.x));
        let top = i64::from(self.y).max(i64::from(other.y));
        let right = (i64::from(self.x) + i64::from(self.width))
            .min(i64::from(other.x) + i64::from(other.width));
        let bottom = (i64::from(self.y) + i64::from(self.height))
            .min(i64::from(other.y) + i64::from(other.height));
        left < right && top < bottom
    }
}

fn offset(base: i32, delta: i64) -> i32 {
    let value = i64::from(base) + delta;
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

/// The accessibility role of an element, normalised across platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// A push button.
    Button,
    /// A hyperlink.
    Link,
    /// A check box or toggle.
    CheckBox,
    /// A radio button.
    RadioButton,
    /// An item in a menu or menu bar.
    MenuItem,
    /// A tab in a tab list.
    Tab,
    /// An editable text field.
    TextField,
    /// A combo box or drop-down.
    ComboBox,
    /// An item in a list.
    ListItem,
    /// An item in a tree.
    TreeItem,
    /// A slider or spin control.
    Slider,
    /// A table or grid cell.
    Cell,
    /// A scrollable area (target of scroll mode).
    ScrollArea,
    /// Static text; shown only by search-by-text.
    Text,
    /// Anything else the backend reports as actionable.
    Other,
}

impl Role {
    /// Whether elements of this role get a hint label in normal hint mode.
    pub const fn is_interactive(self) -> bool {
        !matches!(self, Self::Text | Self::ScrollArea)
    }
}

/// One element of an accessibility tree, as reported by an [`crate::ElementSource`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Identifier, unique within one snapshot.
    pub id: ElementId,
    /// Normalised role.
    pub role: Role,
    /// Accessible name or label text (may be empty).
    pub name: String,
    /// On-screen bounds in global coordinates.
    pub bounds: Rect,
    /// Whether the element is enabled (disabled elements get no hint).
    pub enabled: bool,
}

impl Element {
    /// Whether the element should receive a hint label in normal hint mode.
    pub fn is_clickable(&self) -> bool {
        self.enabled && !self.bounds.is_empty() && self.role.is_interactive()
    }

    /// The point actions target: the centre of the bounds.
    pub fn target(&self) -> Point {
        self.bounds.center()
    }
}

/// Deterministic reading order: top to bottom, then left to right, then by id.
pub fn reading_order(a: &Element, b: &Element) -> Ordering {
    (a.bounds.y, a.bounds.x, a.id).cmp(&(b.bounds.y, b.bounds.x, b.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(id: u64, x: i32, y: i32) -> Element {
        Element {
            id: ElementId(id),
            role: Role::Button,
            name: String::new(),
            bounds: Rect::new(x, y, 10, 10),
            enabled: true,
        }
    }

    #[test]
    fn center_and_saturation() {
        assert_eq!(Rect::new(0, 0, 10, 20).center(), Point::new(5, 10));
        assert_eq!(Rect::new(i32::MAX, 0, u32::MAX, 2).center().x, i32::MAX);
    }

    #[test]
    fn contains_and_intersects() {
        let rect = Rect::new(0, 0, 10, 10);
        assert!(rect.contains(Point::new(0, 0)));
        assert!(!rect.contains(Point::new(10, 5)));
        assert!(rect.intersects(&Rect::new(5, 5, 10, 10)));
        assert!(!rect.intersects(&Rect::new(10, 0, 5, 5)));
    }

    #[test]
    fn clickable_rules() {
        let mut e = element(1, 0, 0);
        assert!(e.is_clickable());
        e.enabled = false;
        assert!(!e.is_clickable());
        e.enabled = true;
        e.role = Role::Text;
        assert!(!e.is_clickable());
        e.role = Role::Button;
        e.bounds.width = 0;
        assert!(!e.is_clickable());
    }

    #[test]
    fn reading_order_is_rows_then_columns() {
        let mut items = [element(3, 50, 10), element(2, 0, 10), element(1, 90, 0)];
        items.sort_by(reading_order);
        let ids: Vec<u64> = items.iter().map(|e| e.id.0).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }
}
