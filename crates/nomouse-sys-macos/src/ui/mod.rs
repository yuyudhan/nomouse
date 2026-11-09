//! The app's own interface: the menu bar item and its menu, the settings window and its
//! controls, and the small panels around them (alerts, the about panel, the app picker).
//!
//! Controls report to one shared action target, which queues a [`UiEvent`] carrying the
//! control's tag; the caller takes the queue with [`take_events`] from its event loop and reads
//! the control's new value through the wrapper that made it. Everything here runs on the main
//! thread.

pub mod form;
pub mod notice;
pub mod panels;
mod recorder;
pub mod snapshot;
pub mod status;

/// Proof of being on the main thread, which every interface call takes.
pub use objc2::MainThreadMarker;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AnyThread, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSEventModifierFlags, NSFont, NSImage, NSMenu, NSMenuItem, NSTextField,
};
use objc2_foundation::{NSData, NSInteger, NSObjectProtocol, NSSize, NSString};

use crate::types::ModifierFlags;

/// Something the user did in the interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiEvent {
    /// The control or menu item with this tag was used (clicked, chosen, edited).
    Action(isize),
    /// A key was pressed in the shortcut recorder with this tag, while it was recording.
    Key {
        /// The recorder.
        tag: isize,
        /// macOS virtual key code (`kVK_*`).
        keycode: u16,
        /// What the key types without modifiers (`charactersIgnoringModifiers`).
        characters: String,
        /// The modifiers held.
        modifiers: ModifierFlags,
    },
}

thread_local! {
    static EVENTS: RefCell<VecDeque<UiEvent>> = const { RefCell::new(VecDeque::new()) };
    static TARGET: RefCell<Option<Retained<Target>>> = const { RefCell::new(None) };
    /// The tag of the shortcut recorder that is recording, if one is.
    static RECORDING: Cell<Option<isize>> = const { Cell::new(None) };
}

fn push(event: UiEvent) {
    EVENTS.with(|events| events.borrow_mut().push_back(event));
}

/// Every event since the last call, oldest first.
pub fn take_events() -> Vec<UiEvent> {
    EVENTS.with(|events| events.borrow_mut().drain(..).collect())
}

/// The tag of the recorder that is recording, if one is.
pub fn recording() -> Option<isize> {
    RECORDING.with(Cell::get)
}

fn set_recording(tag: Option<isize>) {
    RECORDING.with(|recording| recording.set(tag));
}

/// The modifiers in AppKit's flags.
pub(crate) fn modifiers(flags: NSEventModifierFlags) -> ModifierFlags {
    ModifierFlags {
        control: flags.contains(NSEventModifierFlags::Control),
        option: flags.contains(NSEventModifierFlags::Option),
        shift: flags.contains(NSEventModifierFlags::Shift),
        command: flags.contains(NSEventModifierFlags::Command),
    }
}

/// AppKit's flags for `modifiers`.
pub(crate) fn event_flags(modifiers: ModifierFlags) -> NSEventModifierFlags {
    let mut flags = NSEventModifierFlags::empty();
    for (held, flag) in [
        (modifiers.control, NSEventModifierFlags::Control),
        (modifiers.option, NSEventModifierFlags::Option),
        (modifiers.shift, NSEventModifierFlags::Shift),
        (modifiers.command, NSEventModifierFlags::Command),
    ] {
        if held {
            flags |= flag;
        }
    }
    flags
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and `Target` does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct Target;

    impl Target {
        /// Every control's action: queue the sender's tag.
        #[unsafe(method(act:))]
        fn act(&self, sender: Option<&AnyObject>) {
            let tag = sender.map_or(0, |sender| {
                // SAFETY: only `NSControl`s and `NSMenuItem`s are given this target, and both
                // answer `tag` with an NSInteger.
                let tag: NSInteger = unsafe { msg_send![sender, tag] };
                tag
            });
            push(UiEvent::Action(tag));
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's `init` on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

/// The shared action target every control sends to.
pub(crate) fn target(mtm: MainThreadMarker) -> Retained<NSObject> {
    TARGET.with(|cell| {
        let target = cell
            .borrow_mut()
            .get_or_insert_with(|| Target::new(mtm))
            .clone();
        Retained::into_super(target)
    })
}

/// The action selector of [`target`].
pub(crate) fn action() -> objc2::runtime::Sel {
    sel!(act:)
}

/// An image from PNG bytes at `points` size (bytes rendered at twice that size stay sharp on
/// Retina screens), optionally a template image that macOS tints for light and dark menus.
pub(crate) fn image(png: &[u8], points: f64, template: bool) -> Option<Retained<NSImage>> {
    let data = NSData::with_bytes(png);
    let image = NSImage::initWithData(NSImage::alloc(), &data)?;
    image.setSize(NSSize::new(points, points));
    image.setTemplate(template);
    Some(image)
}

/// A non-editable text label.
pub(crate) fn label(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    NSTextField::labelWithString(&NSString::from_str(text), mtm)
}

/// A wrapping label in the small, secondary style of help text, at most `width` points wide.
pub(crate) fn help(text: &str, width: f64, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&NSFont::systemFontOfSize(
        NSFont::smallSystemFontSize(),
    )));
    field.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
    field.setPreferredMaxLayoutWidth(width);
    field.setSelectable(false);
    field
}

/// Brings the app forward so its window takes the keyboard. An agent app (no Dock icon) is
/// not activated by showing a window; `activate` exists from macOS 14, and earlier systems
/// leave the window in front without focus.
pub fn activate(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    if app.respondsToSelector(sel!(activate)) {
        app.activate();
    }
}

/// Gives the app a main menu with the standard Edit and Window commands, so Cmd-C, Cmd-V,
/// Cmd-A, Cmd-Z and Cmd-W work in its windows. An agent app shows no menu bar of its own; the
/// menu only carries the key equivalents.
pub fn install_main_menu(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    if app.mainMenu().is_some() {
        return;
    }
    let main = NSMenu::new(mtm);
    let submenu = |title: &str, items: &[(&str, objc2::runtime::Sel, &str)]| {
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
        for (name, selector, key) in items {
            // SAFETY: the selectors are standard responder actions; a nil target sends them
            // along the responder chain, which ignores what no responder implements.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(name),
                    Some(*selector),
                    &NSString::from_str(key),
                )
            };
            menu.addItem(&item);
        }
        let holder = NSMenuItem::new(mtm);
        holder.setSubmenu(Some(&menu));
        main.addItem(&holder);
    };
    submenu("nomouse", &[("Hide nomouse", sel!(hide:), "h")]);
    submenu(
        "Edit",
        &[
            ("Undo", sel!(undo:), "z"),
            ("Redo", sel!(redo:), "Z"),
            ("Cut", sel!(cut:), "x"),
            ("Copy", sel!(copy:), "c"),
            ("Paste", sel!(paste:), "v"),
            ("Select All", sel!(selectAll:), "a"),
        ],
    );
    submenu(
        "Window",
        &[
            ("Close", sel!(performClose:), "w"),
            ("Minimize", sel!(performMiniaturize:), "m"),
        ],
    );
    app.setMainMenu(Some(&main));
}
