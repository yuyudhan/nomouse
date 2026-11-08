//! Which UI Automation elements get hints, with what role, and how to activate them.

use nomouse_core::{Element, ElementId, Rect, Role};
use nomouse_sys_windows::{Activation, RawElement, control_type as ct};

/// The normalised role of a UIA control type, or `None` for container and decoration types.
pub(crate) fn role(control_type: i32) -> Option<Role> {
    let role = match control_type {
        ct::BUTTON | ct::SPLIT_BUTTON | ct::HEADER_ITEM => Role::Button,
        ct::HYPERLINK => Role::Link,
        ct::CHECK_BOX => Role::CheckBox,
        ct::RADIO_BUTTON => Role::RadioButton,
        ct::MENU_ITEM => Role::MenuItem,
        ct::TAB_ITEM => Role::Tab,
        ct::EDIT => Role::TextField,
        ct::COMBO_BOX => Role::ComboBox,
        ct::LIST_ITEM => Role::ListItem,
        ct::TREE_ITEM => Role::TreeItem,
        ct::SLIDER | ct::SPINNER => Role::Slider,
        ct::DATA_ITEM => Role::Cell,
        ct::TEXT => Role::Text,
        _ => return None,
    };
    Some(role)
}

/// Whether the element gets a hint: an actionable control type, a scroll container, or any
/// element offering an activation pattern or keyboard focus; never offscreen or zero-sized ones.
pub(crate) fn is_candidate(raw: &RawElement) -> bool {
    if raw.offscreen || raw.bounds.width() == 0 || raw.bounds.height() == 0 {
        return false;
    }
    let actionable = ct::ACTIONABLE.contains(&raw.control_type);
    // Windows, panes and documents are often focusable as a whole; labelling them adds noise.
    let container = matches!(raw.control_type, ct::WINDOW | ct::PANE | ct::DOCUMENT);
    actionable
        || is_scroll_area(raw)
        || (!container && (raw.patterns.any() || raw.keyboard_focusable))
}

/// A container (pane, document, list, tree, grid, ...) that scrolls its own content with
/// `ScrollPattern`: scroll mode targets it rather than a control or scroll bar inside it.
/// Controls with their own role (a multi-line edit, a combo box) keep that role.
fn is_scroll_area(raw: &RawElement) -> bool {
    raw.patterns.scroll && role(raw.control_type).is_none()
}

/// The core element for a raw UIA element; `id` is the snapshot index.
pub(crate) fn to_element(raw: &RawElement) -> Element {
    let role = if is_scroll_area(raw) {
        Role::ScrollArea
    } else {
        role(raw.control_type).unwrap_or(Role::Other)
    };
    Element {
        id: ElementId(u64::try_from(raw.index).unwrap_or(u64::MAX)),
        role: if role == Role::Text && raw.patterns.any() {
            Role::Other
        } else {
            role
        },
        name: raw.name.clone(),
        bounds: Rect::new(
            raw.bounds.left,
            raw.bounds.top,
            raw.bounds.width(),
            raw.bounds.height(),
        ),
        enabled: raw.enabled,
    }
}

/// The pattern a semantic click uses, or `None` when only a pointer click will do (text fields
/// need the caret placed; sliders need a position).
pub(crate) fn activation(raw: &RawElement) -> Option<Activation> {
    if matches!(raw.control_type, ct::EDIT | ct::SLIDER | ct::SCROLL_BAR) {
        return None;
    }
    let patterns = raw.patterns;
    if patterns.invoke {
        Some(Activation::Invoke)
    } else if patterns.toggle {
        Some(Activation::Toggle)
    } else if patterns.selection_item {
        Some(Activation::Select)
    } else if patterns.expand_collapse {
        Some(Activation::ExpandCollapse)
    } else if patterns.legacy_default_action {
        Some(Activation::DefaultAction)
    } else {
        None
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use nomouse_sys_windows::{Edges, Patterns};

    use super::*;

    pub(crate) fn raw(index: usize, control_type: i32, bounds: Edges) -> RawElement {
        RawElement {
            index,
            control_type,
            name: format!("e{index}"),
            bounds,
            offscreen: false,
            enabled: true,
            keyboard_focusable: false,
            patterns: Patterns::default(),
        }
    }

    const BOX: Edges = Edges::new(0, 0, 20, 10);

    #[test]
    fn actionable_types_are_candidates() {
        for kind in ct::ACTIONABLE {
            assert!(is_candidate(&raw(0, kind, BOX)), "{kind}");
            assert!(role(kind).is_some(), "{kind}");
        }
    }

    #[test]
    fn offscreen_and_empty_are_dropped() {
        let mut button = raw(0, ct::BUTTON, BOX);
        button.offscreen = true;
        assert!(!is_candidate(&button));
        assert!(!is_candidate(&raw(0, ct::BUTTON, Edges::new(5, 5, 5, 30))));
    }

    #[test]
    fn patterns_or_focus_make_other_types_candidates() {
        let mut text = raw(0, ct::TEXT, BOX);
        assert!(!is_candidate(&text));
        text.patterns.invoke = true;
        assert!(is_candidate(&text));
        assert_eq!(to_element(&text).role, Role::Other);
        let mut custom = raw(0, 50025, BOX);
        custom.keyboard_focusable = true;
        assert!(is_candidate(&custom));
        assert_eq!(to_element(&custom).role, Role::Other);
        let mut pane = raw(0, ct::PANE, BOX);
        pane.keyboard_focusable = true;
        assert!(!is_candidate(&pane));
    }

    #[test]
    fn scroll_containers_are_scroll_areas_but_scroll_bars_are_not() {
        let mut pane = raw(0, ct::PANE, BOX);
        pane.patterns.scroll = true;
        assert!(is_candidate(&pane));
        let element = to_element(&pane);
        assert_eq!(element.role, Role::ScrollArea);
        assert!(!element.is_clickable());
        let mut edit = raw(0, ct::EDIT, BOX);
        edit.patterns.scroll = true;
        assert_eq!(to_element(&edit).role, Role::TextField);
        let mut bar = raw(0, ct::SCROLL_BAR, BOX);
        assert!(!is_candidate(&bar));
        bar.keyboard_focusable = true;
        assert_ne!(to_element(&bar).role, Role::ScrollArea);
    }

    #[test]
    fn element_conversion() {
        let element = to_element(&raw(7, ct::HYPERLINK, Edges::new(10, 20, 40, 30)));
        assert_eq!(element.id, ElementId(7));
        assert_eq!(element.role, Role::Link);
        assert_eq!(element.bounds, Rect::new(10, 20, 30, 10));
        assert!(element.is_clickable());
    }

    #[test]
    fn activation_preference() {
        let mut item = raw(0, ct::LIST_ITEM, BOX);
        assert_eq!(activation(&item), None);
        item.patterns.legacy_default_action = true;
        assert_eq!(activation(&item), Some(Activation::DefaultAction));
        item.patterns.selection_item = true;
        assert_eq!(activation(&item), Some(Activation::Select));
        item.patterns.invoke = true;
        assert_eq!(activation(&item), Some(Activation::Invoke));
        let mut edit = raw(0, ct::EDIT, BOX);
        edit.patterns.invoke = true;
        assert_eq!(activation(&edit), None);
    }
}
