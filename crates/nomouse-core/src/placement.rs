//! Where a hint label goes on screen, shared by every backend: centred on its element's frame
//! on both axes, also when the label is larger than the element, then moved just enough to lie
//! fully inside the area it is drawn in (a monitor, a screen's overlay panel, a viewport).
//!
//! Labels of neighbouring elements keep the spacing of their elements' centres, so elements at
//! least as large as their labels get labels that do not overlap.

use crate::element::Rect;

/// The start of a span of `size` centred on `centre`, moved inside `lo..lo + span`; a span
/// larger than the area starts at `lo`.
pub fn centred_span(centre: i64, size: u32, lo: i32, span: u32) -> i32 {
    let start = centre - i64::from(size) / 2;
    let max = i64::from(lo) + i64::from(span) - i64::from(size);
    i32::try_from(start.min(max).max(i64::from(lo))).unwrap_or(lo)
}

/// The bounds of a `width` by `height` label for an element at `element`: centred on the
/// element's centre and kept inside `area`. A label larger than `area` starts at its top-left
/// corner.
pub fn label_bounds(element: Rect, width: u32, height: u32, area: Rect) -> Rect {
    let centre_x = i64::from(element.x) + i64::from(element.width) / 2;
    let centre_y = i64::from(element.y) + i64::from(element.height) / 2;
    Rect::new(
        centred_span(centre_x, width, area.x, area.width),
        centred_span(centre_y, height, area.y, area.height),
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 1920, 1080);

    #[test]
    fn a_label_is_centred_on_its_element() {
        let element = Rect::new(100, 200, 120, 40);
        let label = label_bounds(element, 30, 20, SCREEN);
        assert_eq!(label, Rect::new(145, 210, 30, 20));
        assert_eq!(label.center(), element.center());
    }

    #[test]
    fn a_label_larger_than_its_element_is_still_centred_on_it() {
        let element = Rect::new(500, 300, 10, 8);
        let label = label_bounds(element, 40, 24, SCREEN);
        assert_eq!(label, Rect::new(485, 292, 40, 24));
        assert_eq!(label.center(), element.center());
    }

    #[test]
    fn a_label_is_clamped_inside_the_screen() {
        // Elements on each edge (and one hanging past the corner) keep their labels on screen.
        assert_eq!(
            label_bounds(Rect::new(0, 0, 10, 10), 30, 20, SCREEN),
            Rect::new(0, 0, 30, 20)
        );
        assert_eq!(
            label_bounds(Rect::new(1910, 1070, 40, 40), 30, 20, SCREEN),
            Rect::new(1890, 1060, 30, 20)
        );
        assert_eq!(
            label_bounds(Rect::new(-50, 500, 60, 10), 30, 20, SCREEN),
            Rect::new(0, 495, 30, 20)
        );
        // On a monitor left of and above the primary one, in its own coordinates.
        let left = Rect::new(-1280, -200, 1280, 1024);
        assert_eq!(
            label_bounds(Rect::new(-1290, 100, 20, 20), 30, 20, left),
            Rect::new(-1280, 100, 30, 20)
        );
        // A label larger than the area starts at the area's top-left corner.
        let tiny = Rect::new(10, 10, 20, 10);
        assert_eq!(
            label_bounds(Rect::new(12, 12, 4, 4), 30, 20, tiny),
            Rect::new(10, 10, 30, 20)
        );
    }

    #[test]
    fn labels_of_adjacent_elements_do_not_collide() {
        // A row of toolbar buttons and a column of list rows, edge to edge.
        let buttons = [Rect::new(0, 40, 32, 28), Rect::new(32, 40, 32, 28)];
        let rows = [Rect::new(300, 100, 400, 22), Rect::new(300, 122, 400, 22)];
        for [first, second] in [buttons, rows] {
            let a = label_bounds(first, 26, 20, SCREEN);
            let b = label_bounds(second, 26, 20, SCREEN);
            assert!(!a.intersects(&b), "{a:?} overlaps {b:?}");
            assert!(first.contains(a.center()) && second.contains(b.center()));
        }
    }

    #[test]
    fn extreme_coordinates_saturate() {
        let far = Rect::new(i32::MAX - 5, i32::MIN, u32::MAX, 10);
        let area = Rect::new(i32::MAX - 100, i32::MIN, 100, 100);
        let label = label_bounds(far, 30, 20, area);
        assert_eq!(label.x, i32::MAX - 30);
        assert_eq!(label.y, i32::MIN);
    }
}
