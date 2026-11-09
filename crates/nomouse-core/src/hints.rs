//! Hint label generation: short, prefix-free labels over a home-row alphabet.

use crate::element::{Element, reading_order};

/// The default alphabet: the home row, without `l` (easily confused with `1`/`I`).
pub const DEFAULT_ALPHABET: &str = "asdfghjk";

/// Characters never allowed in labels because they are easily misread.
pub const AMBIGUOUS: [char; 3] = ['i', 'l', 'o'];

/// A validated label alphabet: at least two distinct, unambiguous lowercase ASCII letters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alphabet(Vec<char>);

/// Why an alphabet was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AlphabetError {
    /// Fewer than two characters.
    #[error("alphabet needs at least 2 characters, got {0}")]
    TooShort(usize),
    /// A character appears twice.
    #[error("alphabet repeats {0:?}")]
    Duplicate(char),
    /// A character is in [`AMBIGUOUS`].
    #[error("alphabet contains ambiguous {0:?} (i, l and o are not allowed)")]
    Ambiguous(char),
    /// A character is not a lowercase ASCII letter.
    #[error("alphabet character {0:?} is not a lowercase ASCII letter")]
    Invalid(char),
}

impl Alphabet {
    /// Validates `chars` as an alphabet.
    ///
    /// # Errors
    /// Returns an [`AlphabetError`] naming the first offending character or the short length.
    pub fn new(chars: &str) -> Result<Self, AlphabetError> {
        let mut seen = Vec::new();
        for c in chars.chars() {
            if !c.is_ascii_lowercase() {
                return Err(AlphabetError::Invalid(c));
            }
            if AMBIGUOUS.contains(&c) {
                return Err(AlphabetError::Ambiguous(c));
            }
            if seen.contains(&c) {
                return Err(AlphabetError::Duplicate(c));
            }
            seen.push(c);
        }
        if seen.len() < 2 {
            return Err(AlphabetError::TooShort(seen.len()));
        }
        Ok(Self(seen))
    }

    /// The characters, in preference order.
    pub fn chars(&self) -> &[char] {
        &self.0
    }
}

impl Default for Alphabet {
    fn default() -> Self {
        Self(DEFAULT_ALPHABET.chars().collect())
    }
}

/// Generates `count` distinct, prefix-free labels, shortest first.
///
/// Labels are the leaves of a breadth-first expansion of the alphabet tree, so every label is
/// as short as the count allows and no label is a prefix of another: typing a full label is
/// always unambiguous.
pub fn generate_labels(alphabet: &Alphabet, count: usize) -> Vec<String> {
    if count == 0 {
        return Vec::new();
    }
    let mut nodes = vec![String::new()];
    let mut expanded = 0;
    while expanded == 0 || nodes.len() - expanded < count {
        let parent = nodes[expanded].clone();
        expanded += 1;
        nodes.extend(alphabet.chars().iter().map(|c| {
            let mut label = parent.clone();
            label.push(*c);
            label
        }));
    }
    nodes.drain(..expanded);
    nodes.truncate(count);
    nodes
}

/// A label assigned to an element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    /// The label the user types.
    pub label: String,
    /// The labelled element.
    pub element: Element,
}

/// Sorts `elements` into reading order and gives the shortest labels to the first ones.
pub fn assign_hints(alphabet: &Alphabet, mut elements: Vec<Element>) -> Vec<Hint> {
    elements.sort_by(reading_order);
    let labels = generate_labels(alphabet, elements.len());
    labels
        .into_iter()
        .zip(elements)
        .map(|(label, element)| Hint { label, element })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementId, Rect, Role};

    #[test]
    fn alphabet_validation() {
        assert!(Alphabet::new("asdf").is_ok());
        assert_eq!(Alphabet::new("a"), Err(AlphabetError::TooShort(1)));
        assert_eq!(Alphabet::new("asa"), Err(AlphabetError::Duplicate('a')));
        assert_eq!(Alphabet::new("asl"), Err(AlphabetError::Ambiguous('l')));
        assert_eq!(Alphabet::new("aS"), Err(AlphabetError::Invalid('S')));
        assert_eq!(Alphabet::new("a1"), Err(AlphabetError::Invalid('1')));
    }

    #[test]
    fn small_counts_use_single_characters() {
        let alphabet = Alphabet::default();
        assert_eq!(generate_labels(&alphabet, 0), Vec::<String>::new());
        assert_eq!(generate_labels(&alphabet, 1), vec!["a"]);
        assert_eq!(generate_labels(&alphabet, 8).concat(), "asdfghjk");
    }

    #[test]
    fn grows_to_two_characters_minimally() {
        let alphabet = Alphabet::new("ab").unwrap();
        assert_eq!(generate_labels(&alphabet, 3), vec!["b", "aa", "ab"]);
        let labels = generate_labels(&Alphabet::default(), 9);
        assert_eq!(labels.iter().filter(|l| l.len() == 1).count(), 7);
        assert_eq!(labels.len(), 9);
    }

    #[test]
    fn assignment_follows_reading_order() {
        let make = |id, x, y| Element {
            id: ElementId(id),
            role: Role::Button,
            name: String::new(),
            description: String::new(),
            value: String::new(),
            bounds: Rect::new(x, y, 5, 5),
            enabled: true,
            offscreen: false,
        };
        let hints = assign_hints(&Alphabet::default(), vec![make(1, 10, 50), make(2, 0, 0)]);
        assert_eq!(hints[0].label, "a");
        assert_eq!(hints[0].element.id, ElementId(2));
        assert_eq!(hints[1].element.id, ElementId(1));
    }
}
