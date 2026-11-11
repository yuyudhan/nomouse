//! Property tests: generated labels are unique, prefix-free, minimal and typable, and a re-scan
//! keeps the labels of the targets still there.

use std::collections::{HashMap, HashSet};

use nomouse_core::{
    Alphabet, Element, ElementId, FilterState, Hint, LabelFilter, Rect, Role, assign_hints,
    generate_labels, reassign_hints,
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
    fn a_rescan_keeps_the_labels_of_targets_still_there(
        alphabet in alphabet(),
        before in prop::collection::vec(any::<bool>(), 0..120),
        after in prop::collection::vec(any::<bool>(), 0..120),
    ) {
        let first = assign_hints(&alphabet, grid(&before));
        let scan = grid(&after);
        let second = reassign_hints(&alphabet, &first, scan.clone());
        // Every target of the new scan has exactly one label, and the labels stay typable.
        prop_assert_eq!(second.len(), scan.len());
        let targets: HashSet<Rect> = second.iter().map(|hint| hint.element.bounds).collect();
        prop_assert_eq!(targets, scan.iter().map(|e| e.bounds).collect::<HashSet<_>>());
        assert_prefix_free(&second)?;
        // Targets in both scans keep their labels; only a full label tree gives one up.
        let old: HashMap<Rect, &str> =
            first.iter().map(|hint| (hint.element.bounds, hint.label.as_str())).collect();
        let changed = second
            .iter()
            .filter(|hint| old.get(&hint.element.bounds).is_some_and(|label| *label != hint.label))
            .count();
        prop_assert!(changed <= 1, "{changed} kept targets changed their label");
    }
}
