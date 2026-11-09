//! A result window that does not block: a titled window with a message, an optional scrollable
//! report and a row of buttons.
//!
//! An alert's `runModal` runs its own event loop until a button is clicked, so a daemon that
//! shows one stops serving its hotkeys and its configuration reloads. A [`Notice`] only shows
//! the window and returns; its buttons report through [`super::take_events`] like every other
//! control, and the caller closes it when it has carried the click out.

use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSBackingStoreType, NSBorderType, NSButton, NSColor, NSFont, NSLayoutAttribute, NSScrollView,
    NSStackView, NSTextView, NSUserInterfaceLayoutOrientation, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSEdgeInsets, NSPoint, NSRect, NSSize, NSString};

use super::{action, activate, help, label, target};

/// Width of the message and the report, in points.
const WIDTH: f64 = 520.0;
/// Height of the report box, in points.
const REPORT_HEIGHT: f64 = 240.0;
/// Space between the window's edge and its content, in points.
const MARGIN: f64 = 20.0;

/// A result window on screen. Dropping it closes it.
#[derive(Debug)]
pub struct Notice {
    window: Retained<NSWindow>,
    report: Option<Retained<NSTextView>>,
    /// The buttons by tag, for the test drivers.
    #[cfg(feature = "testing")]
    buttons: Vec<(isize, Retained<NSButton>)>,
}

impl Notice {
    /// Shows a window titled `title` with `message` (when not empty), `report` in a scrollable
    /// monospaced box (when given) and a button for each `(tag, title)`; the first answers
    /// Return. A click arrives as [`super::UiEvent::Action`] with the button's tag. `warning`
    /// draws the title in orange. The window is in front of other apps; this returns at once.
    pub fn show(
        mtm: MainThreadMarker,
        title: &str,
        message: &str,
        report: Option<&str>,
        buttons: &[(isize, &str)],
        warning: bool,
    ) -> Self {
        let stack = NSStackView::new(mtm);
        stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        stack.setAlignment(NSLayoutAttribute::Leading);
        stack.setSpacing(12.0);
        stack.setEdgeInsets(NSEdgeInsets {
            top: MARGIN,
            left: MARGIN,
            bottom: MARGIN,
            right: MARGIN,
        });

        let heading = label(title, mtm);
        heading.setFont(Some(&NSFont::boldSystemFontOfSize(
            NSFont::systemFontSize() + 1.0,
        )));
        if warning {
            heading.setTextColor(Some(&NSColor::systemOrangeColor()));
        }
        stack.addArrangedSubview(&heading);

        if !message.is_empty() {
            let text = help(message, WIDTH, mtm);
            text.setFont(Some(&NSFont::systemFontOfSize(NSFont::systemFontSize())));
            text.setTextColor(Some(&NSColor::labelColor()));
            stack.addArrangedSubview(&text);
        }

        let report = report.map(|text| {
            let (scroll, view) = report_box(mtm, text);
            stack.addArrangedSubview(&scroll);
            view
        });

        let row = NSStackView::new(mtm);
        row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        row.setSpacing(8.0);
        let mut made = Vec::with_capacity(buttons.len());
        // Return answers the first button; the row lists them right to left, as macOS alerts do.
        for (index, (tag, title)) in buttons.iter().enumerate() {
            let button = button(*tag, title, mtm);
            if index == 0 {
                button.setKeyEquivalent(&NSString::from_str("\r"));
            }
            made.push((*tag, button));
        }
        for (_, button) in made.iter().rev() {
            row.addArrangedSubview(button);
        }
        stack.addArrangedSubview(&row);

        let window = window(mtm, title);
        window.setContentView(Some(&stack));
        window.setContentSize(stack.fittingSize());
        window.center();
        activate(mtm);
        window.makeKeyAndOrderFront(None);
        Self {
            window,
            report,
            #[cfg(feature = "testing")]
            buttons: made,
        }
    }

    /// Whether the window is on screen (it is not once the user closed it).
    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }

    /// Closes the window.
    pub fn close(&self) {
        self.window.close();
    }

    /// The text of the report box, when the notice has one.
    pub fn report_text(&self) -> Option<String> {
        self.report.as_ref().map(|view| view.string().to_string())
    }

    /// Clicks the button with `tag` as a user would (tests). Whether there is such a button.
    #[cfg(feature = "testing")]
    pub fn click(&self, tag: isize) -> bool {
        self.buttons
            .iter()
            .find(|(button, _)| *button == tag)
            .is_some_and(|(_, button)| {
                // SAFETY: a nil sender is allowed; the button sends its action as for a click.
                unsafe { button.performClick(None) };
                true
            })
    }

    /// The titles of the buttons, in the order given to [`Notice::show`] (tests).
    #[cfg(feature = "testing")]
    pub fn button_titles(&self) -> Vec<String> {
        self.buttons
            .iter()
            .map(|(_, button)| button.title().to_string())
            .collect()
    }
}

impl Drop for Notice {
    fn drop(&mut self) {
        self.window.close();
    }
}

fn window(mtm: MainThreadMarker, title: &str) -> Retained<NSWindow> {
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, 120.0));
    // SAFETY: a plain titled window; `defer: true` creates its device when first shown.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            true,
        )
    };
    // SAFETY: the window is owned by `Notice`; AppKit must not free it on close.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str(title));
    window
}

/// A scroll view of `width` by `height` holding a read-only, selectable monospaced `text`.
fn report_box(mtm: MainThreadMarker, text: &str) -> (Retained<NSScrollView>, Retained<NSTextView>) {
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, REPORT_HEIGHT));
    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
    scroll.setHasVerticalScroller(true);
    scroll.setBorderType(NSBorderType::BezelBorder);
    scroll
        .widthAnchor()
        .constraintEqualToConstant(WIDTH)
        .setActive(true);
    scroll
        .heightAnchor()
        .constraintEqualToConstant(REPORT_HEIGHT)
        .setActive(true);
    let view = NSTextView::initWithFrame(NSTextView::alloc(mtm), frame);
    view.setEditable(false);
    view.setSelectable(true);
    if let Some(font) = NSFont::userFixedPitchFontOfSize(11.0) {
        view.setFont(Some(&font));
    }
    view.setString(&NSString::from_str(text));
    scroll.setDocumentView(Some(&view));
    (scroll, view)
}

fn button(tag: isize, title: &str, mtm: MainThreadMarker) -> Retained<NSButton> {
    let target = target(mtm);
    // SAFETY: the shared target implements `act:` and lives for the thread.
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(&target),
            Some(action()),
            mtm,
        )
    };
    button.setTag(tag);
    button
}
