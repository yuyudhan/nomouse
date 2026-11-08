//! Property tests: generated labels are unique, prefix-free, minimal and typable.

use std::collections::HashSet;

use nomouse_core::{Alphabet, FilterState, LabelFilter, generate_labels};
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
}
