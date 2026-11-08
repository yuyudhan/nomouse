//! Which Accessibility roles get hints, and how they map to [`Role`].

use nomouse_core::Role;

/// `AXPress`, the default action of buttons, links and most clickable things.
pub const PRESS: &str = "AXPress";
/// `AXShowMenu`, offered by elements with a context or pop-up menu.
pub const SHOW_MENU: &str = "AXShowMenu";

/// What the classifier knows about one element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleFacts<'a> {
    /// `AXRole`, such as `AXButton`.
    pub role: &'a str,
    /// `AXSubrole`, such as `AXTabButton` or `AXOutlineRow`.
    pub subrole: Option<&'a str>,
    /// The parent's `AXRole`.
    pub parent_role: Option<&'a str>,
    /// Whether the element lists `AXPress` among its actions.
    pub pressable: bool,
}

/// The normalised role of an element, or `None` when it gets no hint and is only walked
/// through (groups, windows, images without actions, ...).
pub fn classify(facts: RoleFacts<'_>) -> Option<Role> {
    let RoleFacts {
        role,
        subrole,
        parent_role,
        pressable,
    } = facts;
    let mapped = match role {
        "AXButton" | "AXMenuButton" | "AXDisclosureTriangle" | "AXColorWell" | "AXDockItem" => {
            Some(Role::Button)
        }
        "AXLink" => Some(Role::Link),
        "AXCheckBox" => Some(Role::CheckBox),
        "AXRadioButton" if subrole == Some("AXTabButton") || parent_role == Some("AXTabGroup") => {
            Some(Role::Tab)
        }
        "AXRadioButton" => Some(Role::RadioButton),
        "AXTab" => Some(Role::Tab),
        "AXPopUpButton" | "AXComboBox" => Some(Role::ComboBox),
        "AXMenuItem" | "AXMenuBarItem" => Some(Role::MenuItem),
        "AXTextField" | "AXTextArea" | "AXSearchField" | "AXSecureTextField" => {
            Some(Role::TextField)
        }
        "AXSlider" | "AXIncrementor" | "AXStepper" => Some(Role::Slider),
        "AXRow" if subrole == Some("AXOutlineRow") => Some(Role::TreeItem),
        "AXRow" => Some(Role::ListItem),
        // Cells inside a row are covered by the row's hint; standalone cells (collection
        // views, grids without row objects) get their own.
        "AXCell" if parent_role != Some("AXRow") => Some(Role::Cell),
        "AXScrollArea" => Some(Role::ScrollArea),
        "AXStaticText" => Some(Role::Text),
        _ => None,
    };
    mapped.or_else(|| pressable.then_some(Role::Other))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(role: &str) -> RoleFacts<'_> {
        RoleFacts {
            role,
            subrole: None,
            parent_role: None,
            pressable: false,
        }
    }

    #[test]
    fn common_controls() {
        assert_eq!(classify(facts("AXButton")), Some(Role::Button));
        assert_eq!(classify(facts("AXLink")), Some(Role::Link));
        assert_eq!(classify(facts("AXCheckBox")), Some(Role::CheckBox));
        assert_eq!(classify(facts("AXPopUpButton")), Some(Role::ComboBox));
        assert_eq!(classify(facts("AXComboBox")), Some(Role::ComboBox));
        assert_eq!(classify(facts("AXMenuBarItem")), Some(Role::MenuItem));
        assert_eq!(classify(facts("AXMenuItem")), Some(Role::MenuItem));
        assert_eq!(classify(facts("AXTextArea")), Some(Role::TextField));
        assert_eq!(classify(facts("AXTextField")), Some(Role::TextField));
        assert_eq!(classify(facts("AXSlider")), Some(Role::Slider));
        assert_eq!(classify(facts("AXDisclosureTriangle")), Some(Role::Button));
        assert_eq!(classify(facts("AXScrollArea")), Some(Role::ScrollArea));
        assert_eq!(classify(facts("AXStaticText")), Some(Role::Text));
    }

    #[test]
    fn radio_buttons_in_tab_groups_are_tabs() {
        assert_eq!(classify(facts("AXRadioButton")), Some(Role::RadioButton));
        let in_tabs = RoleFacts {
            parent_role: Some("AXTabGroup"),
            ..facts("AXRadioButton")
        };
        assert_eq!(classify(in_tabs), Some(Role::Tab));
        let tab_button = RoleFacts {
            subrole: Some("AXTabButton"),
            ..facts("AXRadioButton")
        };
        assert_eq!(classify(tab_button), Some(Role::Tab));
    }

    #[test]
    fn rows_and_cells() {
        assert_eq!(classify(facts("AXRow")), Some(Role::ListItem));
        let outline = RoleFacts {
            subrole: Some("AXOutlineRow"),
            ..facts("AXRow")
        };
        assert_eq!(classify(outline), Some(Role::TreeItem));
        assert_eq!(classify(facts("AXCell")), Some(Role::Cell));
        let in_row = RoleFacts {
            parent_role: Some("AXRow"),
            ..facts("AXCell")
        };
        assert_eq!(classify(in_row), None);
    }

    #[test]
    fn anything_pressable_counts() {
        assert_eq!(classify(facts("AXImage")), None);
        assert_eq!(classify(facts("AXGroup")), None);
        let pressable = RoleFacts {
            pressable: true,
            ..facts("AXImage")
        };
        assert_eq!(classify(pressable), Some(Role::Other));
    }
}
