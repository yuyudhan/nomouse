//! Search-by-text: fuzzy, case-insensitive matching of a typed query against an element's
//! name, value, description and role.

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
/// Matching is case-insensitive and ignores whitespace on both sides (`open file` scores as
/// `openfile`, and `openfile` finds "Open File"). A contiguous substring beats a scattered
/// subsequence; among substrings, a match at the start of the name or of a word beats one
/// inside a word, and an earlier position beats a later one. Subsequence matches are ranked by
/// how spread out the matched characters are. An empty query matches nothing.
pub fn fuzzy_score(name: &str, query: &str) -> Option<u32> {
    // The name's non-space characters, each with whether a word starts there.
    let mut previous = None;
    let mut name_chars = Vec::new();
    let mut starts = Vec::new();
    for c in name.chars() {
        if !c.is_whitespace() {
            let start = !previous.is_some_and(char::is_alphanumeric);
            for lower in c.to_lowercase() {
                name_chars.push(lower);
                starts.push(start);
            }
        }
        previous = Some(c);
    }
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() || query.len() > name_chars.len() {
        return None;
    }
    let substring = (0..=name_chars.len() - query.len())
        .filter(|&start| name_chars[start..start + query.len()] == query[..])
        .min_by_key(|&start| (!starts[start], start));
    if let Some(start) = substring {
        let position = u32::try_from(start).unwrap_or(u32::MAX).min(999);
        return Some(if starts[start] {
            position
        } else {
            1_000 + position
        });
    }
    let mut chars = name_chars.iter().enumerate();
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

/// Whether `element` can be found by search: enabled, on screen, with some text (a name, value
/// or description), and either interactive or static text (scroll areas are left to scroll
/// mode).
pub fn is_searchable(element: &Element) -> bool {
    let has = |text: &str| !text.trim().is_empty();
    element.enabled
        && !element.bounds.is_empty()
        && element.role != Role::ScrollArea
        && (has(&element.name) || has(&element.value) || has(&element.description))
}

/// Whether `query` asks for every element: it is made of `*` and `;` only (spaces aside).
pub fn matches_everything(query: &str) -> bool {
    let mut chars = query.chars().filter(|c| !c.is_whitespace()).peekable();
    chars.peek().is_some() && chars.all(|c| c == '*' || c == ';')
}

/// Score bands: a name match beats a value or description match, which beats a role match.
const NAME: u32 = 0;
const TEXT: u32 = 100_000;
const ROLE: u32 = 200_000;
/// Fuzzy scores from this value up are scattered subsequences, not substrings.
const SUBSEQUENCE: u32 = 2_000;

/// How well `element` matches `query`: its best field, names first. Roles (`button`, `check
/// box`) match only as substrings, so a short query does not pick every control.
fn element_score(element: &Element, query: &str) -> Option<u32> {
    let in_band = |band: u32, text: &str| fuzzy_score(text, query).map(|score| band + score);
    let role = element
        .role
        .words()
        .and_then(|words| fuzzy_score(words, query))
        .filter(|score| *score < SUBSEQUENCE)
        .map(|score| ROLE + score);
    [
        in_band(NAME, &element.name),
        in_band(TEXT, &element.value),
        in_band(TEXT, &element.description),
        role,
    ]
    .into_iter()
    .flatten()
    .min()
}

/// The elements matching `query`, best match first (ties in reading order); a query of only
/// `*` and `;` ([`matches_everything`]) returns every element in reading order.
pub fn rank<'a>(elements: &'a [Element], query: &str) -> Vec<&'a Element> {
    let everything = matches_everything(query);
    let mut scored: Vec<(u32, &Element)> = elements
        .iter()
        .filter_map(|element| {
            let score = if everything {
                Some(0)
            } else {
                element_score(element, query)
            };
            score.map(|score| (score, element))
        })
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
            description: String::new(),
            value: String::new(),
            bounds: Rect::new(0, y, 10, 10),
            enabled: true,
            offscreen: false,
        }
    }

    #[test]
    fn substring_beats_subsequence_and_word_starts_win() {
        assert_eq!(fuzzy_score("Save", "sa"), Some(0));
        // Positions count non-space characters: "File" starts at the fifth, index 4.
        assert_eq!(fuzzy_score("Open File", "FILE"), Some(4));
        assert_eq!(fuzzy_score("Profile", "file"), Some(1_003));
        assert_eq!(fuzzy_score("Settings", "stg"), Some(2_004));
        assert_eq!(fuzzy_score("Save", "xyz"), None);
        assert_eq!(fuzzy_score("Save", ""), None);
        assert_eq!(fuzzy_score("ab", "abc"), None);
        assert_eq!(fuzzy_score("Open File", "open file"), Some(0));
        assert_eq!(fuzzy_score("Open File", "openfile"), Some(0));
        assert_eq!(fuzzy_score("Save", "  "), None);
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
        let mut tooltip_only = named(1, "", 0);
        "Save the file".clone_into(&mut tooltip_only.description);
        assert!(is_searchable(&tooltip_only));
        let mut filled = named(1, "", 0);
        "draft".clone_into(&mut filled.value);
        assert!(is_searchable(&filled));
    }

    #[test]
    fn star_and_semicolon_match_everything_in_reading_order() {
        let elements = vec![named(1, "B", 30), named(2, "A", 10), named(3, "C", 20)];
        for query in ["*", ";", " * ", ";;", "*;"] {
            assert!(matches_everything(query), "{query:?}");
            let ids: Vec<u64> = rank(&elements, query).iter().map(|e| e.id.0).collect();
            assert_eq!(ids, vec![2, 3, 1], "{query:?}");
        }
        for query in ["", "  ", "*a", "a;"] {
            assert!(!matches_everything(query), "{query:?}");
        }
    }

    #[test]
    fn names_beat_values_and_descriptions_which_beat_roles() {
        let mut by_description = named(1, "", 0);
        "Opens the button settings".clone_into(&mut by_description.description);
        let mut by_value = named(2, "", 10);
        "button text".clone_into(&mut by_value.value);
        let mut by_role = named(3, "Ok", 20);
        by_role.role = Role::Button;
        let by_name = named(4, "Button bar", 30);
        let elements = vec![by_description, by_value, by_role, by_name];
        let ids: Vec<u64> = rank(&elements, "button").iter().map(|e| e.id.0).collect();
        assert_eq!(ids, vec![4, 2, 1, 3]);
        // A role matches only as a substring: `bt` is a subsequence of "button".
        assert_eq!(rank(&elements[2..3], "bt"), Vec::<&Element>::new());
        assert_eq!(rank(&elements[2..3], "butt").len(), 1);
        // Role::Other names nothing.
        let mut other = named(5, "Ok", 0);
        other.role = Role::Other;
        assert_eq!(
            rank(std::slice::from_ref(&other), "other"),
            Vec::<&Element>::new()
        );
    }

    #[test]
    fn search_box_sits_near_the_top_centre() {
        let anchor = search_box_anchor(Rect::new(1920, 0, 1000, 800));
        assert_eq!(anchor, Rect::new(2320, 100, 1, 32));
        assert!(Rect::new(1920, 0, 1000, 800).contains(anchor.center()));
    }
}
