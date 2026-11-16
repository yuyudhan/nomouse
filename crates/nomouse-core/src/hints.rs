//! Hint label generation: short, prefix-free labels over a home-row alphabet.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::element::{Element, Rect, Role, reading_order};

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

/// Sorts `elements` into reading order and gives the shortest labels to the first ones, leaving
/// at least one label of the alphabet tree free so [`merge_hints`] can always label targets
/// that arrive later without touching these.
pub fn assign_hints(alphabet: &Alphabet, mut elements: Vec<Element>) -> Vec<Hint> {
    elements.sort_by(reading_order);
    // One label more than needed: the last one stays free. Only when `count` labels would
    // fill the tree exactly does that change a label (one short label splits).
    let labels = generate_labels(alphabet, elements.len() + 1);
    labels
        .into_iter()
        .zip(elements)
        .map(|(label, element)| Hint { label, element })
        .collect()
}

/// The hints for the targets of a fresh scan `elements` that `shown` does not label yet, in
/// reading order, to be shown alongside `shown`. The labels shown keep their letters, and no
/// new hint takes one of them.
///
/// A target is already labelled when a shown hint has its role, name and bounds; failing that,
/// when a shown hint of a named element has its role and name (it moved: a loading page pushed
/// content down, or the page was scrolled; [`follow_hints`] moves its hint along). Duplicates
/// pair up in reading order.
///
/// The new labels are the shortest ones free: neither a shown label, nor a prefix of one, nor
/// one of them as a prefix, so the whole set stays prefix-free. One label is always left free
/// for the next scan (see [`assign_hints`]); without one (`shown` filled the tree) nothing is
/// added.
pub fn merge_hints(alphabet: &Alphabet, shown: &[Hint], mut elements: Vec<Element>) -> Vec<Hint> {
    elements.sort_by(reading_order);
    let (labelled, _) = pair_up(shown, &elements);
    let new: Vec<Element> = elements
        .into_iter()
        .zip(labelled)
        .filter_map(|(element, labelled)| labelled.is_none().then_some(element))
        .collect();
    let taken = shown.iter().map(|hint| &hint.label);
    let Some(fresh) = free_labels(alphabet, taken, new.len() + 1) else {
        return Vec::new();
    };
    fresh
        .into_iter()
        .zip(new)
        .map(|(label, element)| Hint { label, element })
        .collect()
}

/// The indices of the `shown` hints whose target is not among `elements`, a complete fresh
/// scan (paired up as [`merge_hints`] does, so `elements` are sorted into reading order): the
/// element went away or out of view, so its hint can go and its label is free again.
pub fn gone_hints(shown: &[Hint], elements: &mut [Element]) -> Vec<usize> {
    elements.sort_by(reading_order);
    let (_, used) = pair_up(shown, elements);
    used.iter()
        .enumerate()
        .filter_map(|(index, used)| (!used).then_some(index))
        .collect()
}

/// Moves each `shown` hint whose target moved onto its target in `elements`, a fresh scan
/// (paired up as [`merge_hints`] does, so `elements` are sorted into reading order): a target
/// the page scrolled or pushed elsewhere keeps its label, which now sits on it and acts on it
/// there. Hints whose target stayed put are left as they are. Returns whether a hint moved.
pub fn follow_hints(shown: &mut [Hint], elements: &mut [Element]) -> bool {
    elements.sort_by(reading_order);
    let (labelled, _) = pair_up(shown, elements);
    let mut moved = false;
    for (element, index) in elements.iter().zip(labelled) {
        if let Some(hint) = index.and_then(|index| shown.get_mut(index))
            && hint.element.bounds != element.bounds
        {
            hint.element = element.clone();
            moved = true;
        }
    }
    moved
}

/// For each of `elements` (in reading order), the `shown` hint that already labels it; and for
/// each `shown` hint, whether one of `elements` is its target. When the page moved
/// ([`scroll_offset`]: it was scrolled, or content above it loaded), a target first pairs with
/// the hint of its role and name whose bounds were its bounds before that move, so unnamed
/// and same-named targets keep their letters (another target that moved into its old place
/// does not take it); then with one of its role, name and bounds (it stayed, as a toolbar
/// does); then with one of its role and non-empty name (it moved some other way).
fn pair_up(shown: &[Hint], elements: &[Element]) -> (Vec<Option<usize>>, Vec<bool>) {
    let mut exact: HashMap<(Role, &str, Rect), VecDeque<usize>> = HashMap::new();
    let mut named: HashMap<(Role, &str), VecDeque<usize>> = HashMap::new();
    let mut order: Vec<usize> = (0..shown.len()).collect();
    order.sort_by(|a, b| reading_order(&shown[*a].element, &shown[*b].element));
    for index in order {
        let element = &shown[index].element;
        exact
            .entry((element.role, element.name.as_str(), element.bounds))
            .or_default()
            .push_back(index);
        if !element.name.is_empty() {
            named
                .entry((element.role, element.name.as_str()))
                .or_default()
                .push_back(index);
        }
    }
    let mut used = vec![false; shown.len()];
    let take = |queue: Option<&mut VecDeque<usize>>, used: &mut [bool]| {
        let queue = queue?;
        while let Some(index) = queue.pop_front() {
            if !used[index] {
                used[index] = true;
                return Some(index);
            }
        }
        None
    };
    let mut labelled: Vec<Option<usize>> = vec![None; elements.len()];
    if let Some((dx, dy)) = scroll_offset(shown, elements) {
        for (labelled, element) in labelled.iter_mut().zip(elements) {
            let bounds = element.bounds;
            let before = Rect::new(
                bounds.x.saturating_sub(dx),
                bounds.y.saturating_sub(dy),
                bounds.width,
                bounds.height,
            );
            let key = (element.role, element.name.as_str(), before);
            *labelled = take(exact.get_mut(&key), &mut used);
        }
    }
    for (labelled, element) in labelled.iter_mut().zip(elements) {
        if labelled.is_none() {
            let key = (element.role, element.name.as_str(), element.bounds);
            *labelled = take(exact.get_mut(&key), &mut used);
        }
    }
    for (labelled, element) in labelled.iter_mut().zip(elements) {
        if labelled.is_none() && !element.name.is_empty() {
            let key = (element.role, element.name.as_str());
            *labelled = take(named.get_mut(&key), &mut used);
        }
    }
    (labelled, used)
}

/// How far the page moved since `shown` were labelled, when it did: the most common move of
/// the targets whose role, name and size are those of exactly one hint and exactly one
/// element (a named link; an unnamed file name cell, whose width follows its text), ties
/// going to the smallest move. `None` unless at least two of them moved that way: one target
/// that went and another like it that came elsewhere is no scroll.
fn scroll_offset(shown: &[Hint], elements: &[Element]) -> Option<(i32, i32)> {
    type Signature<'a> = (Role, &'a str, u32, u32);
    fn by_signature<'a>(
        elements: impl Iterator<Item = &'a Element>,
    ) -> HashMap<Signature<'a>, Vec<Rect>> {
        let mut map: HashMap<Signature<'a>, Vec<Rect>> = HashMap::new();
        for element in elements {
            let bounds = element.bounds;
            map.entry((element.role, &element.name, bounds.width, bounds.height))
                .or_default()
                .push(bounds);
        }
        map
    }
    let before = by_signature(shown.iter().map(|hint| &hint.element));
    let now = by_signature(elements.iter());
    let mut moves: HashMap<(i32, i32), usize> = HashMap::new();
    for (signature, now) in &now {
        let (Some([a]), [b]) = (before.get(signature).map(Vec::as_slice), now.as_slice()) else {
            continue;
        };
        let delta = (b.x.saturating_sub(a.x), b.y.saturating_sub(a.y));
        if delta != (0, 0) {
            *moves.entry(delta).or_default() += 1;
        }
    }
    moves
        .into_iter()
        .filter(|&(_, count)| count >= 2)
        .max_by_key(|&((dx, dy), count)| {
            let size = i64::from(dx).abs() + i64::from(dy).abs();
            (count, std::cmp::Reverse(size), std::cmp::Reverse((dx, dy)))
        })
        .map(|(delta, _)| delta)
}

/// `count` labels, shortest first, that are neither one of `taken` (a prefix-free set) nor a
/// prefix of one nor have one as a prefix; `None` when `count > 0` and `taken` leaves no label
/// free at all.
fn free_labels<'a>(
    alphabet: &Alphabet,
    taken: impl IntoIterator<Item = &'a String>,
    count: usize,
) -> Option<Vec<String>> {
    if count == 0 {
        return Some(Vec::new());
    }
    let taken: HashSet<&str> = taken.into_iter().map(String::as_str).collect();
    let inner: HashSet<&str> = taken
        .iter()
        .flat_map(|label| (1..label.len()).filter_map(|end| label.get(..end)))
        .collect();
    // Breadth first, so `free` is sorted by length.
    let mut free = Vec::new();
    let mut queue: VecDeque<String> = children(alphabet, "").collect();
    while let Some(node) = queue.pop_front() {
        if inner.contains(node.as_str()) {
            queue.extend(children(alphabet, &node));
        } else if !taken.contains(node.as_str()) {
            free.push(node);
        }
    }
    if free.is_empty() {
        return None;
    }
    // Like `generate_labels`: split the shortest free label until there are enough.
    while free.len() < count {
        let parent = free.remove(0);
        let at = free.partition_point(|label| label.len() <= parent.len() + 1);
        free.splice(at..at, children(alphabet, &parent));
    }
    free.truncate(count);
    Some(free)
}

/// The labels one character longer than `parent`, in alphabet order.
fn children<'a>(alphabet: &'a Alphabet, parent: &'a str) -> impl Iterator<Item = String> + 'a {
    alphabet.chars().iter().map(move |c| {
        let mut label = parent.to_owned();
        label.push(*c);
        label
    })
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

    fn link(id: u64, name: &str, x: i32, y: i32) -> Element {
        Element {
            id: ElementId(id),
            role: Role::Link,
            name: name.to_owned(),
            description: String::new(),
            value: String::new(),
            bounds: Rect::new(x, y, 5, 5),
            enabled: true,
            offscreen: false,
        }
    }

    fn labels(hints: &[Hint]) -> Vec<(&str, &str)> {
        hints
            .iter()
            .map(|hint| (hint.element.name.as_str(), hint.label.as_str()))
            .collect()
    }

    fn assert_prefix_free(hints: &[Hint]) {
        for a in hints {
            for b in hints {
                if !std::ptr::eq(a, b) {
                    assert!(!b.label.starts_with(&a.label), "{} / {}", a.label, b.label);
                }
            }
        }
    }

    #[test]
    fn a_rescan_labels_only_new_targets_and_leaves_shown_hints_as_they_are() {
        let alphabet = Alphabet::default();
        let first = assign_hints(
            &alphabet,
            vec![link(1, "one", 0, 0), link(2, "two", 0, 100)],
        );
        assert_eq!(labels(&first), [("one", "a"), ("two", "s")]);
        // A new link between them in reading order; the scan gives every element a new id.
        let scan = vec![
            link(7, "two", 0, 100),
            link(8, "new", 0, 50),
            link(9, "one", 0, 0),
        ];
        let added = merge_hints(&alphabet, &first, scan);
        assert_eq!(labels(&added), [("new", "d")]);
        assert_eq!(added[0].element.id, ElementId(8));
        // "one" is gone and "two" came back: nothing to add, nothing shown is touched.
        let shown = [first, added].concat();
        let again = merge_hints(&alphabet, &shown, vec![link(4, "two", 0, 100)]);
        assert_eq!(again, Vec::new());
    }

    #[test]
    fn a_moved_named_target_is_already_labelled_and_duplicates_pair_in_reading_order() {
        let alphabet = Alphabet::default();
        let first = assign_hints(
            &alphabet,
            vec![
                link(1, "more", 0, 0),
                link(2, "more", 0, 10),
                link(3, "", 0, 20),
            ],
        );
        assert_eq!(labels(&first), [("more", "a"), ("more", "s"), ("", "d")]);
        // An image above loads and pushes everything down 200 px: the named links already have
        // their labels, a third "more" and the unnamed one are new targets (one unnamed link
        // alone tells nothing about how far the page moved).
        let moved = vec![
            link(4, "more", 0, 200),
            link(5, "more", 0, 210),
            link(6, "", 0, 220),
            link(7, "more", 0, 230),
        ];
        let added = merge_hints(&alphabet, &first, moved);
        assert_eq!(labels(&added), [("", "f"), ("more", "g")]);
        assert_eq!(added[1].element.bounds, Rect::new(0, 230, 5, 5));
    }

    #[test]
    fn hints_follow_their_targets_when_the_page_scrolls() {
        let alphabet = Alphabet::default();
        let mut shown = assign_hints(
            &alphabet,
            vec![
                link(1, "one", 0, 0),
                link(2, "two", 0, 100),
                link(3, "", 0, 200),
            ],
        );
        assert_eq!(labels(&shown), [("one", "a"), ("two", "s"), ("", "d")]);
        // Nothing moved: the hints stay as they are.
        let mut same = vec![
            link(7, "two", 0, 100),
            link(8, "one", 0, 0),
            link(9, "", 0, 200),
        ];
        let before = shown.clone();
        assert!(!follow_hints(&mut shown, &mut same));
        assert_eq!(shown, before);
        // Scrolled up by 100: every link keeps its letter and moves along, the unnamed one
        // too (it moved as far as the named ones).
        let mut scrolled = vec![
            link(4, "one", 0, -100),
            link(5, "two", 0, 0),
            link(6, "", 0, 100),
        ];
        assert!(follow_hints(&mut shown, &mut scrolled));
        assert_eq!(labels(&shown), [("one", "a"), ("two", "s"), ("", "d")]);
        assert_eq!(shown[0].element.bounds, Rect::new(0, -100, 5, 5));
        assert_eq!(shown[1].element.bounds, Rect::new(0, 0, 5, 5));
        assert_eq!(shown[1].element.id, ElementId(5));
        assert_eq!(shown[2].element.bounds, Rect::new(0, 100, 5, 5));
    }

    #[test]
    fn a_scroll_keeps_the_letters_of_same_named_and_unnamed_targets() {
        let alphabet = Alphabet::default();
        let mut shown = assign_hints(
            &alphabet,
            vec![
                link(1, "edit", 0, 0),
                link(2, "", 0, 100),
                link(3, "edit", 0, 200),
                link(4, "two", 0, 300),
                link(5, "three", 0, 400),
            ],
        );
        assert_eq!(
            labels(&shown),
            [
                ("edit", "a"),
                ("", "s"),
                ("edit", "d"),
                ("two", "f"),
                ("three", "g")
            ]
        );
        // Scrolled up by 200: the first "edit" and the unnamed link left, another unnamed link
        // and another "edit" came in. The "edit" still on screen keeps "d" (reading order
        // alone would have given it "a"), "two" and "three" keep theirs.
        let mut scrolled = vec![
            link(6, "edit", 0, 0),
            link(7, "two", 0, 100),
            link(8, "three", 0, 200),
            link(9, "", 0, 300),
            link(10, "edit", 0, 400),
        ];
        assert_eq!(gone_hints(&shown, &mut scrolled.clone()), [1]);
        assert!(follow_hints(&mut shown, &mut scrolled));
        let at = |label: &str| {
            shown
                .iter()
                .find(|hint| hint.label == label)
                .map(|hint| hint.element.bounds.y)
        };
        assert_eq!((at("d"), at("f"), at("g")), (Some(0), Some(100), Some(200)));
        // The scrolled-off "edit" pairs with the one that came in. The unnamed link that left
        // pairs with nothing ("s" stays where it was, and a complete scan takes it away); the
        // one that came in is new.
        assert_eq!(at("a"), Some(400));
        assert_eq!(at("s"), Some(100));
        // As the session does, the gone hint goes before new targets are labelled.
        shown.remove(1);
        let added = merge_hints(&alphabet, &shown, scrolled);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].element.id, ElementId(9));
    }

    #[test]
    fn the_first_labels_leave_room_so_later_ones_never_relabel() {
        let alphabet = Alphabet::new("fj").unwrap();
        // Two targets would fill the tree with f and j; one label stays free instead.
        let links = |count: u64| -> Vec<Element> {
            (0..count)
                .map(|i| link(i, &format!("link {i}"), 0, 10 * i32::try_from(i).unwrap()))
                .collect()
        };
        let first = assign_hints(&alphabet, links(2));
        assert_eq!(labels(&first), [("link 0", "j"), ("link 1", "ff")]);
        let mut shown = first;
        for count in 3..15 {
            let before = shown.clone();
            let added = merge_hints(&alphabet, &shown, links(count));
            assert_eq!(added.len(), 1);
            assert_eq!(added[0].element.name, format!("link {}", count - 1));
            shown.extend(added);
            assert_eq!(shown[..before.len()], before[..]);
            assert_prefix_free(&shown);
        }
    }

    #[test]
    fn eight_targets_keep_a_free_label_in_the_default_alphabet() {
        let alphabet = Alphabet::default();
        let eight: Vec<Element> = (0..8)
            .map(|i| link(i, "", 0, 10 * i32::try_from(i).unwrap()))
            .collect();
        let first = assign_hints(&alphabet, eight.clone());
        assert_prefix_free(&first);
        let mut scan = eight;
        scan.push(link(8, "", 0, 80));
        let added = merge_hints(&alphabet, &first, scan);
        assert_eq!(added.len(), 1);
        assert_prefix_free(&[first, added].concat());
    }

    #[test]
    fn free_labels_avoid_the_taken_ones_and_their_prefixes() {
        let alphabet = Alphabet::new("fj").unwrap();
        let taken = ["f".to_owned(), "jj".to_owned()];
        assert_eq!(
            free_labels(&alphabet, &taken, 1),
            Some(vec!["jf".to_owned()])
        );
        // One free label, "jf", splits until three are free.
        let three = ["jfj", "jfff", "jffj"].map(str::to_owned).to_vec();
        assert_eq!(free_labels(&alphabet, &taken, 3), Some(three));
        assert_eq!(
            free_labels(&alphabet, &["f".to_owned(), "j".to_owned()], 1),
            None
        );
        assert_eq!(free_labels(&alphabet, &taken, 0), Some(Vec::new()));
    }

    /// A Finder list: unnamed rows of one size, each with an unnamed file name cell as wide
    /// as its name; row `i` at `top + 20 * i`, ids from `first`.
    fn rows(widths: &[u32], first: u64, top: i32) -> Vec<Element> {
        let unnamed = |id: u64, role: Role, x: i32, y: i32, width: u32| Element {
            id: ElementId(id),
            role,
            name: String::new(),
            description: String::new(),
            value: String::new(),
            bounds: Rect::new(x, y, width, 20),
            enabled: true,
            offscreen: false,
        };
        let mut list = Vec::new();
        for ((i, y), width) in (0..).zip((top..).step_by(20)).zip(widths) {
            list.push(unnamed(first + 2 * i, Role::ListItem, 750, y, 600));
            list.push(unnamed(first + 2 * i + 1, Role::TextField, 800, y, *width));
        }
        list
    }

    #[test]
    fn unnamed_rows_keep_their_letters_when_the_list_scrolls() {
        let alphabet = Alphabet::default();
        let widths = [21, 44, 28, 39, 64, 50];
        let mut shown = assign_hints(&alphabet, rows(&widths[..4], 0, 100));
        let letters = |hints: &[Hint]| -> Vec<(i32, i32, String)> {
            let mut all: Vec<_> = hints
                .iter()
                .map(|hint| {
                    (
                        hint.element.bounds.y,
                        hint.element.bounds.x,
                        hint.label.clone(),
                    )
                })
                .collect();
            all.sort();
            all
        };
        let before = letters(&shown);
        // Scrolled by two rows: rows 2 and 3 are at the top now, rows 4 and 5 came in. Row 2
        // is where row 0 was, but keeps its own letters, and rows 0 and 1 are gone.
        let mut scrolled = rows(&widths[2..], 100, 100);
        assert_eq!(gone_hints(&shown, &mut scrolled.clone()), [0, 1, 2, 3]);
        assert!(follow_hints(&mut shown, &mut scrolled));
        let after = letters(&shown);
        for (y, x, label) in &before[4..] {
            assert!(
                after.contains(&(y - 40, *x, label.clone())),
                "{label}: {after:?}"
            );
        }
        assert_eq!(merge_hints(&alphabet, &shown, scrolled).len(), 4);
    }
}
