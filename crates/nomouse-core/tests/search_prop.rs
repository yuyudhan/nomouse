//! Property tests of search-by-text: spaces in a query change nothing, a query of `*` and `;`
//! returns every element, and an element is found by its description or value as by its name.

use nomouse_core::search::{fuzzy_score, rank};
use nomouse_core::{Element, ElementId, Rect, Role};
use proptest::prelude::*;

/// An element with no role words (`Role::Other`), so only its texts can match.
fn element(id: usize, name: &str, description: &str, value: &str) -> Element {
    let row = i32::try_from(id).unwrap_or(i32::MAX);
    Element {
        id: ElementId(u64::try_from(id).unwrap_or(u64::MAX)),
        role: Role::Other,
        name: name.to_owned(),
        description: description.to_owned(),
        value: value.to_owned(),
        bounds: Rect::new(0, row.saturating_mul(20), 50, 10),
        enabled: true,
    }
}

proptest! {
    #[test]
    fn spaces_in_the_query_score_as_without(
        name in "[a-zA-Z ]{0,30}",
        query in "[a-z]{1,8}",
        spaces in prop::collection::vec(0usize..3, 8),
    ) {
        let spaced: String = query
            .chars()
            .zip(&spaces)
            .flat_map(|(c, n)| std::iter::repeat_n(' ', *n).chain(std::iter::once(c)))
            .chain(std::iter::once(' '))
            .collect();
        prop_assert_eq!(fuzzy_score(&name, &spaced), fuzzy_score(&name, &query));
    }

    #[test]
    fn star_and_semicolon_return_every_element(
        names in prop::collection::vec("[a-z ]{0,12}", 0..30),
        query in "[*;]{1,3}",
        padding in " {0,2}",
    ) {
        let elements: Vec<Element> = names
            .iter()
            .enumerate()
            .map(|(i, name)| element(i, name, "", ""))
            .collect();
        let ranked = rank(&elements, &format!("{padding}{query}"));
        let ids: Vec<u64> = ranked.iter().map(|e| e.id.0).collect();
        let all: Vec<u64> = elements.iter().map(|e| e.id.0).collect();
        prop_assert_eq!(ids, all);
    }

    #[test]
    fn description_and_value_are_found_like_a_name(text in "[a-z]{3,20}", start in 0usize..20, len in 1usize..6) {
        prop_assume!(start + len <= text.len());
        let query = &text[start..start + len];
        let elements = vec![
            element(0, "", &text, ""),
            element(1, "", "", &text),
            element(2, "000", "", ""),
        ];
        let ids: Vec<u64> = rank(&elements, query).iter().map(|e| e.id.0).collect();
        prop_assert_eq!(ids, vec![0, 1]);
    }
}
