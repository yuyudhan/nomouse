//! Narrowing the shown labels as the user types.

/// The result of typing one character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterState {
    /// Some labels still start with the typed prefix; none is complete yet.
    Pending,
    /// The typed prefix equals the label at this index.
    Matched(usize),
    /// No label starts with the prefix plus this character; the character was not added.
    Rejected,
}

/// Tracks the typed prefix over a fixed, prefix-free set of labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelFilter {
    labels: Vec<String>,
    typed: String,
}

impl LabelFilter {
    /// Creates a filter over `labels` (as produced by [`crate::generate_labels`]).
    pub fn new(labels: Vec<String>) -> Self {
        Self {
            labels,
            typed: String::new(),
        }
    }

    /// Types `c` (letters are lowercased).
    pub fn push(&mut self, c: char) -> FilterState {
        let mut candidate = self.typed.clone();
        candidate.push(c.to_ascii_lowercase());
        if !self
            .labels
            .iter()
            .any(|label| label.starts_with(&candidate))
        {
            return FilterState::Rejected;
        }
        self.typed = candidate;
        self.labels
            .iter()
            .position(|label| *label == self.typed)
            .map_or(FilterState::Pending, FilterState::Matched)
    }

    /// Removes the last typed character; returns whether there was one.
    pub fn pop(&mut self) -> bool {
        self.typed.pop().is_some()
    }

    /// Clears the typed prefix.
    pub fn reset(&mut self) {
        self.typed.clear();
    }

    /// The prefix typed so far.
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// Indices of the labels still matching the typed prefix, in label order.
    pub fn matching(&self) -> impl Iterator<Item = usize> + '_ {
        self.labels
            .iter()
            .enumerate()
            .filter(|(_, label)| label.starts_with(&self.typed))
            .map(|(index, _)| index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter() -> LabelFilter {
        LabelFilter::new(vec!["b".into(), "aa".into(), "ab".into()])
    }

    #[test]
    fn narrows_then_matches() {
        let mut f = filter();
        assert_eq!(f.matching().count(), 3);
        assert_eq!(f.push('A'), FilterState::Pending);
        assert_eq!(f.matching().collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(f.push('b'), FilterState::Matched(2));
    }

    #[test]
    fn rejects_without_changing_prefix() {
        let mut f = filter();
        assert_eq!(f.push('z'), FilterState::Rejected);
        assert_eq!(f.typed(), "");
        assert_eq!(f.push('b'), FilterState::Matched(0));
    }

    #[test]
    fn pop_and_reset() {
        let mut f = filter();
        assert!(!f.pop());
        f.push('a');
        assert!(f.pop());
        assert_eq!(f.typed(), "");
        f.push('a');
        f.reset();
        assert_eq!(f.matching().count(), 3);
    }
}
