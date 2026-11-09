//! Property tests of a hint session: whatever keys move the highlight or narrow the labels,
//! the highlight stays on a visible hint, and there is one whenever a hint is visible.

use nomouse_core::{
    ActionKind, Alphabet, Element, ElementId, HintSession, KeyBindings, KeyEvent, KeyPress,
    Modifiers, Rect, Role,
};
use proptest::prelude::*;

fn elements(count: usize) -> Vec<Element> {
    (0..count)
        .map(|i| Element {
            id: ElementId(u64::try_from(i).unwrap_or(u64::MAX)),
            role: Role::Button,
            name: String::new(),
            description: String::new(),
            value: String::new(),
            bounds: Rect::new(
                i32::try_from(i % 10).unwrap_or(0) * 50,
                i32::try_from(i / 10).unwrap_or(0) * 30,
                20,
                10,
            ),
            enabled: true,
            offscreen: false,
        })
        .collect()
}

/// Focus moves, label characters (some matching nothing) and Backspace.
fn key() -> impl Strategy<Value = KeyEvent> {
    prop_oneof![
        Just(KeyPress::Tab.into()),
        Just(KeyPress::Tab.with(Modifiers::SHIFT)),
        Just(KeyPress::Up.into()),
        Just(KeyPress::Down.into()),
        Just(KeyPress::Char('n').with(Modifiers::CTRL)),
        Just(KeyPress::Char('p').with(Modifiers::CTRL)),
        Just(KeyPress::Backspace.into()),
        prop::sample::select(vec!['a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'z'])
            .prop_map(|c| KeyPress::Char(c).into()),
    ]
}

proptest! {
    #[test]
    fn the_highlight_stays_on_a_visible_hint(
        count in 0usize..80,
        homerow in any::<bool>(),
        keys in prop::collection::vec(key(), 0..40),
    ) {
        let bindings = if homerow { KeyBindings::Homerow } else { KeyBindings::Nomouse };
        let mut session = HintSession::new(&Alphabet::default(), elements(count), ActionKind::Hover)
            .with_bindings(bindings);
        for key in keys {
            session.key(key);
            let visible = session.visible();
            match session.focused() {
                Some(focused) => prop_assert!(visible.contains(focused)),
                None => prop_assert!(visible.is_empty()),
            }
        }
    }
}
