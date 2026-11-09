//! The menu bar item: a template icon that macOS tints for light and dark menu bars, and its
//! menu.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSControlStateValueOff, NSControlStateValueOn, NSMenu, NSMenuItem,
    NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::NSString;

use super::{action, event_flags, image, target};
use crate::types::ModifierFlags;

/// A shortcut shown at the right edge of a menu item, as macOS draws key equivalents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// The key: a lowercase character, `" "` for Space, or a function key's private-use
    /// character (`NSF1FunctionKey`...).
    pub key: String,
    /// The modifiers.
    pub modifiers: ModifierFlags,
}

/// One line of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuLine {
    /// A clickable (or greyed-out) item; clicking queues `UiEvent::Action(tag)`.
    Item {
        /// Reported when clicked.
        tag: isize,
        /// The text.
        title: String,
        /// Whether it can be clicked; a disabled item is greyed out and only informs.
        enabled: bool,
        /// Whether it shows a check mark.
        checked: bool,
        /// A shortcut drawn at the right edge.
        shortcut: Option<Shortcut>,
    },
    /// A thin line between groups.
    Separator,
}

/// The menu bar item.
#[derive(Debug)]
pub struct StatusItem {
    item: Retained<NSStatusItem>,
    mtm: MainThreadMarker,
}

impl StatusItem {
    /// Adds the item to the right end of the menu bar with `icon` (a PNG drawn at 36 px for an
    /// 18 point template image) and `name` for `VoiceOver` and the tooltip.
    pub fn new(mtm: MainThreadMarker, icon: &[u8], name: &str) -> Self {
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = item.button(mtm) {
            match image(icon, 18.0, true) {
                Some(icon) => button.setImage(Some(&icon)),
                None => button.setTitle(&NSString::from_str(name)),
            }
            let name = NSString::from_str(name);
            button.setToolTip(Some(&name));
            button.setAccessibilityTitle(Some(&name));
        }
        Self { item, mtm }
    }

    /// Replaces the menu with `lines`.
    pub fn set_menu(&self, lines: &[MenuLine]) {
        let menu = NSMenu::new(self.mtm);
        // Items are enabled or not as `lines` say, not by AppKit's validation.
        menu.setAutoenablesItems(false);
        let target = target(self.mtm);
        for line in lines {
            let item = match line {
                MenuLine::Separator => NSMenuItem::separatorItem(self.mtm),
                MenuLine::Item {
                    tag,
                    title,
                    enabled,
                    checked,
                    shortcut,
                } => {
                    let key = shortcut.as_ref().map_or("", |shortcut| &shortcut.key);
                    // SAFETY: `act:` is implemented by the target set below, which outlives
                    // the menu (it lives for the thread).
                    let item = unsafe {
                        NSMenuItem::initWithTitle_action_keyEquivalent(
                            NSMenuItem::alloc(self.mtm),
                            &NSString::from_str(title),
                            Some(action()),
                            &NSString::from_str(key),
                        )
                    };
                    if let Some(shortcut) = shortcut {
                        item.setKeyEquivalentModifierMask(event_flags(shortcut.modifiers));
                    }
                    // SAFETY: the target is the shared action target, alive for the thread.
                    unsafe { item.setTarget(Some(&target)) };
                    item.setTag(*tag);
                    item.setEnabled(*enabled);
                    item.setState(if *checked {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                    item
                }
            };
            menu.addItem(&item);
        }
        self.item.setMenu(Some(&menu));
    }

    /// Dims the icon (while the hotkeys are paused) or shows it normally.
    pub fn set_dimmed(&self, dimmed: bool) {
        if let Some(button) = self.item.button(self.mtm) {
            button.setAppearsDisabled(dimmed);
        }
    }

    /// Opens the menu, pictures it as PNG bytes, and closes it again; `None` when the menu
    /// did not open.
    pub fn picture_menu(&self) -> Option<Vec<u8>> {
        let menu = self.item.menu(self.mtm)?;
        let button = self.item.button(self.mtm)?;
        super::snapshot::picture_menu(self.mtm, &menu, || {
            // SAFETY: a nil sender is allowed; the click opens the item's menu, as the mouse
            // would.
            unsafe { button.performClick(None) };
        })
    }
}

impl Drop for StatusItem {
    fn drop(&mut self) {
        NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
    }
}
