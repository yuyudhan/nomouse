//! Property tests: generated labels are unique, prefix-free, minimal and typable, and a re-scan
//! only adds labels: the ones shown never change and the new ones never collide with them.

use std::collections::{HashMap, HashSet};

use nomouse_core::{
    Alphabet, Element, ElementId, FilterState, Hint, LabelFilter, Rect, Role, assign_hints,
    generate_labels, merge_hints,
};
use proptest::prelude::*;

fn alphabet() -> impl Strategy<Value = Alphabet> {
    proptest::sample::subsequence(
        "abcdefghjkmnpqrstuvwxyz".chars().collect::<Vec<_>>(),
        2..=12,
    )
    .prop_shuffle()
    .prop_filter_map("valid alphabet", |chars| {
        Alphabet::new(&chars.into_iter().collect::<String>()).ok()
    })
}

/// The buttons of a 10-column grid at the positions `present` marks.
fn grid(present: &[bool]) -> Vec<Element> {
    (0..present.len())
        .filter(|index| present[*index])
        .map(|index| {
            let i = i32::try_from(index).unwrap_or(0);
            Element {
                id: ElementId(u64::try_from(index).unwrap_or(0)),
                role: Role::Button,
                name: String::new(),
                description: String::new(),
                value: String::new(),
                bounds: Rect::new(i % 10 * 50, i / 10 * 30, 20, 10),
                enabled: true,
                offscreen: false,
            }
        })
        .collect()
}

fn assert_prefix_free(hints: &[Hint]) -> Result<(), TestCaseError> {
    let mut labels: Vec<&str> = hints.iter().map(|hint| hint.label.as_str()).collect();
    labels.sort_unstable();
    for pair in labels.windows(2) {
        prop_assert!(
            !pair[1].starts_with(pair[0]),
            "{} prefixes {}",
            pair[0],
            pair[1]
        );
    }
    Ok(())
}

proptest! {
    #[test]
    fn unique_and_prefix_free(alphabet in alphabet(), count in 0usize..2000) {
        let labels = generate_labels(&alphabet, count);
        prop_assert_eq!(labels.len(), count);
        let set: HashSet<&String> = labels.iter().collect();
        prop_assert_eq!(set.len(), count);
        let mut sorted = labels.clone();
        sorted.sort();
        for pair in sorted.windows(2) {
            prop_assert!(!pair[1].starts_with(&pair[0]), "{} prefixes {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn only_alphabet_characters_and_shortest_first(alphabet in alphabet(), count in 1usize..2000) {
        let labels = generate_labels(&alphabet, count);
        prop_assert!(labels.iter().all(|l| l.chars().all(|c| alphabet.chars().contains(&c))));
        prop_assert!(labels.windows(2).all(|p| p[0].len() <= p[1].len()));
        let n = alphabet.chars().len();
        let max = labels.iter().map(String::len).max().unwrap_or(0);
        // Minimal length: count labels never need more than ceil(log_n(count)) + 1 characters.
        let mut capacity = 1usize;
        let mut needed = 0usize;
        while capacity < count {
            capacity = capacity.saturating_mul(n);
            needed += 1;
        }
        prop_assert!(max <= needed.max(1));
    }

    #[test]
    fn every_label_types_to_its_own_match(alphabet in alphabet(), count in 1usize..300, pick in any::<prop::sample::Index>()) {
        let labels = generate_labels(&alphabet, count);
        let index = pick.index(count);
        let mut filter = LabelFilter::new(labels.clone());
        let mut last = FilterState::Rejected;
        for c in labels[index].chars() {
            last = filter.push(c);
        }
        prop_assert_eq!(last, FilterState::Matched(index));
    }

    #[test]
    fn rescans_only_add_labels_and_never_collide(
        alphabet in alphabet(),
        scans in prop::collection::vec(prop::collection::vec(any::<bool>(), 0..120), 1..5),
    ) {
        let mut shown = assign_hints(&alphabet, grid(&scans[0]));
        assert_prefix_free(&shown)?;
        for scan in &scans[1..] {
            let before = shown.clone();
            let elements = grid(scan);
            let added = merge_hints(&alphabet, &shown, elements.clone());
            shown.extend(added);
            // The labels already shown are untouched, in place, with their targets.
            prop_assert_eq!(&shown[..before.len()], &before[..]);
            // Every target of the scan is labelled exactly once, and all labels stay typable.
            let mut labelled: HashMap<Rect, usize> = HashMap::new();
            for hint in &shown {
                *labelled.entry(hint.element.bounds).or_default() += 1;
            }
            for element in &elements {
                prop_assert_eq!(labelled.get(&element.bounds), Some(&1));
            }
            let unique: HashSet<&str> = shown.iter().map(|hint| hint.label.as_str()).collect();
            prop_assert_eq!(unique.len(), shown.len());
            assert_prefix_free(&shown)?;
        }
    }
}
