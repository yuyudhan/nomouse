//! Search-by-text: fuzzy, case-insensitive matching of a typed query against element names.

use std::cmp::Ordering;

use crate::element::{Element, Rect, Role, reading_order};

/// Where overlays draw the search box on a monitor `area`: a thin anchor rectangle a little
/// left of the horizontal centre, an eighth of the way down, so the box (drawn like a label at
/// the anchor's left edge) sits near the top centre without covering the window title.
pub fn search_box_anchor(area: Rect) -> Rect {
    let x = i64::from(area.x) + i64::from(area.width) * 2 / 5;
    let y = i64::from(area.y) + i64::from(area.height) / 8;
    let clamp = |value: i64| i32::try_from(value).unwrap_or(i32::MAX);
    Rect::new(clamp(x), clamp(y), 1, 32)
}

/// Scores how well `name` matches `query`; lower is better, `None` is no match.
///
/// Matching is case-insensitive. A contiguous substring beats a scattered subsequence; among
/// substrings, a match at the start of the name or of a word beats one inside a word, and an
/// earlier position beats a later one. Subsequence matches are ranked by how spread out the
/// matched characters are. An empty query matches nothing.
pub fn fuzzy_score(name: &str, query: &str) -> Option<u32> {
    let name: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if query.is_empty() || query.len() > name.len() {
        return None;
    }
    let substring = (0..=name.len() - query.len())
        .filter(|&start| name[start..start + query.len()] == query[..])
        .min_by_key(|&start| (!word_start(&name, start), start));
    if let Some(start) = substring {
        let position = u32::try_from(start).unwrap_or(u32::MAX).min(999);
        return Some(if word_start(&name, start) {
            position
        } else {
            1_000 + position
        });
    }
    let mut chars = name.iter().enumerate();
    let mut first = None;
    let mut last = 0;
    for wanted in &query {
        let (index, _) = chars.find(|(_, c)| *c == wanted)?;
        first.get_or_insert(index);
        last = index;
    }
    let spread = last - first.unwrap_or(0) + 1 - query.len();
    Some(2_000u32.saturating_add(u32::try_from(spread).unwrap_or(u32::MAX)))
}

fn word_start(name: &[char], index: usize) -> bool {
    index == 0 || !name[index - 1].is_alphanumeric()
}

/// Whether `element` can be found by search: enabled, on screen, named, and either
/// interactive or static text (scroll areas are left to scroll mode).
pub fn is_searchable(element: &Element) -> bool {
    element.enabled
        && !element.bounds.is_empty()
        && element.role != Role::ScrollArea
        && !element.name.trim().is_empty()
}

/// The elements whose names match `query`, best match first (ties in reading order).
pub fn rank<'a>(elements: &'a [Element], query: &str) -> Vec<&'a Element> {
    let mut scored: Vec<(u32, &Element)> = elements
        .iter()
        .filter_map(|element| fuzzy_score(&element.name, query).map(|score| (score, element)))
        .collect();
    scored.sort_by(|(a_score, a), (b_score, b)| match a_score.cmp(b_score) {
        Ordering::Equal => reading_order(a, b),
        other => other,
    });
    scored.into_iter().map(|(_, element)| element).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementId, Rect};

    fn named(id: u64, name: &str, y: i32) -> Element {
        Element {
            id: ElementId(id),
            role: Role::Link,
            name: name.to_owned(),
            bounds: Rect::new(0, y, 10, 10),
            enabled: true,
        }
    }

    #[test]
    fn substring_beats_subsequence_and_word_starts_win() {
        assert_eq!(fuzzy_score("Save", "sa"), Some(0));
        assert_eq!(fuzzy_score("Open File", "FILE"), Some(5));
        assert_eq!(fuzzy_score("Profile", "file"), Some(1_003));
        assert_eq!(fuzzy_score("Settings", "stg"), Some(2_004));
        assert_eq!(fuzzy_score("Save", "xyz"), None);
        assert_eq!(fuzzy_score("Save", ""), None);
        assert_eq!(fuzzy_score("ab", "abc"), None);
    }

    #[test]
    fn ranking_orders_by_score_then_reading_order() {
        let elements = vec![
            named(1, "Profile", 0),
            named(2, "File", 50),
            named(3, "Open file", 20),
            named(4, "Help", 10),
        ];
        let ids: Vec<u64> = rank(&elements, "file").iter().map(|e| e.id.0).collect();
        assert_eq!(ids, vec![2, 3, 1]);
    }

    #[test]
    fn searchable_needs_a_name_and_no_scroll_role() {
        assert!(is_searchable(&named(1, "Go", 0)));
        assert!(!is_searchable(&named(1, "  ", 0)));
        let mut area = named(1, "List", 0);
        area.role = Role::ScrollArea;
        assert!(!is_searchable(&area));
    }

    #[test]
    fn search_box_sits_near_the_top_centre() {
        let anchor = search_box_anchor(Rect::new(1920, 0, 1000, 800));
        assert_eq!(anchor, Rect::new(2320, 100, 1, 32));
        assert!(Rect::new(1920, 0, 1000, 800).contains(anchor.center()));
    }
}
