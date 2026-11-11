//! The menu bar item: a template icon that macOS tints for light and dark menu bars, and its
//! menu.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSControlStateValueOff, NSControlStateValueOn, NSImage, NSMenu, NSMenuItem,
    NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::NSString;

use super::{action, event_flags, image, target};
use crate::types::ModifierFlags;

/// The SF Symbol the item shows instead of its icon while it warns.
const WARNING_SYMBOL: &str = "exclamationmark.triangle";

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

/// The menu bar item. It keeps what it shows (icon, warning, dimming, menu), so
/// [`StatusItem::recreate`] can put an identical item in the menu bar.
#[derive(Debug)]
pub struct StatusItem {
    item: Retained<NSStatusItem>,
    /// Whether `item` is in the menu bar (it is taken out by [`StatusItem::remove`]).
    placed: bool,
    icon: Option<Retained<NSImage>>,
    name: String,
    warning: Option<String>,
    dimmed: bool,
    menu: Option<Retained<NSMenu>>,
    mtm: MainThreadMarker,
}

impl StatusItem {
    /// Adds the item to the right end of the menu bar with `icon` (a PNG drawn at 36 px for an
    /// 18 point template image) and `name` for `VoiceOver` and the tooltip.
    pub fn new(mtm: MainThreadMarker, icon: &[u8], name: &str) -> Self {
        let item = Self {
            item: NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength),
            placed: true,
            icon: image(icon, 18.0, true),
            name: name.to_owned(),
            warning: None,
            dimmed: false,
            menu: None,
            mtm,
        };
        item.dress();
        item
    }

    /// Applies the icon (or the warning sign), tooltip, dimming and menu to the item.
    fn dress(&self) {
        if let Some(button) = self.item.button(self.mtm) {
            let warning_sign = self.warning.as_ref().and_then(|_| {
                let sign = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(WARNING_SYMBOL),
                    None,
                )?;
                sign.setTemplate(true);
                Some(sign)
            });
            if let Some(shown) = warning_sign.as_ref().or(self.icon.as_ref()) {
                button.setImage(Some(shown));
                button.setTitle(&NSString::new());
            } else {
                button.setImage(None);
                button.setTitle(&NSString::from_str(&self.name));
            }
            let tip = NSString::from_str(&self.warning.as_ref().map_or_else(
                || self.name.clone(),
                |warning| format!("{}: {warning}", self.name),
            ));
            button.setToolTip(Some(&tip));
            button.setAccessibilityTitle(Some(&tip));
            button.setAppearsDisabled(self.dimmed);
        }
        self.item.setMenu(self.menu.as_deref());
    }

    /// Whether macOS shows the item: its button's window has the menu bar's height. When
    /// the menu bar refuses the item (its scene does not activate) the window keeps a zero
    /// height. An item in a menu bar that is hidden until the pointer reaches the top of the
    /// screen still counts as shown.
    #[must_use]
    pub fn is_on_screen(&self) -> bool {
        self.placed
            && self
                .item
                .button(self.mtm)
                .and_then(|button| button.window())
                .is_some_and(|window| window.frame().size.height > 0.0)
    }

    /// Takes the item out of the menu bar and puts a new one with the same icon, warning,
    /// dimming and menu in its place, as a fresh request to the menu bar.
    pub fn recreate(&mut self) {
        self.remove();
        self.item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        self.placed = true;
        self.dress();
    }

    /// Takes the item out of the menu bar, which also stops macOS asking the menu bar for it
    /// again and again when the menu bar refuses it.
    pub fn remove(&mut self) {
        if self.placed {
            NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
            self.placed = false;
        }
    }

    /// Shows a warning sign instead of the icon, with `warning` after the name in the
    /// tooltip, or (`None`) the icon again.
    pub fn set_warning(&mut self, warning: Option<&str>) {
        let warning = warning.map(ToOwned::to_owned);
        if warning != self.warning {
            self.warning = warning;
            self.dress();
        }
    }

    /// Replaces the menu with `lines`.
    pub fn set_menu(&mut self, lines: &[MenuLine]) {
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
        self.menu = Some(menu);
    }

    /// Dims the icon (while the hotkeys are paused) or shows it normally.
    pub fn set_dimmed(&mut self, dimmed: bool) {
        self.dimmed = dimmed;
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
        self.remove();
    }
}
