//! The settings window: one page per section behind a toolbar of tabs, as macOS's own
//! settings windows look, each page a grid of labelled controls with help text below them.
//!
//! The window is built from plain descriptions ([`Page`], [`Row`], [`Spec`]); each control
//! reports through [`super::take_events`] with its tag, and [`Form::translate`] turns those
//! events into [`FormEvent`]s. Values are read and written through the getters and setters
//! by tag.

use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{
    NSAccessibility, NSBackingStoreType, NSButton, NSColor, NSColorSpace, NSColorWell, NSControl,
    NSControlStateValueOff, NSControlStateValueOn, NSGridCell, NSGridCellPlacement, NSGridView,
    NSImage, NSImageView, NSLayoutAttribute, NSLineBreakMode, NSPopUpButton, NSScrollView,
    NSStackView, NSStepper, NSTabViewController, NSTabViewControllerTabStyle, NSTabViewItem,
    NSTextField, NSUserInterfaceItemIdentification, NSUserInterfaceLayoutOrientation, NSView,
    NSViewController, NSWindow, NSWindowStyleMask, NSWindowToolbarStyle,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSArray, NSEdgeInsets, NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{CALayer, CATransaction};

use super::panels::{AppInfo, icon};
use super::recorder::Recorder;
use super::{UiEvent, action, activate, help, label, set_recording, target};
use crate::overlay::{cg_color, label_layer};
use crate::types::{Label, LabelStyle, ModifierFlags, Rgba};

/// Width of a page, in points.
const PAGE_WIDTH: f64 = 620.0;
/// Width of help text and notes.
const TEXT_WIDTH: f64 = 400.0;
/// The size of the label preview.
const PREVIEW: NSSize = NSSize::new(400.0, 72.0);
/// The size of an app list.
const APP_LIST: NSSize = NSSize::new(400.0, 132.0);

/// Sub-tags of a control's parts, added to its tag.
const STEPPER: isize = 1 << 20;
const RESET: isize = 2 << 20;
const ADD: isize = 3 << 20;
const REMOVE: isize = 4 << 20;
const PART: isize = 0xf << 20;
/// A control's own tag fits below the part bits; a list entry's index sits above them.
const TAG: isize = (1 << 20) - 1;
const INDEX_SHIFT: u32 = 24;

/// What a page shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// The tab's title.
    pub title: String,
    /// The tab's SF Symbol (`keyboard`, `paintpalette`...).
    pub symbol: String,
    /// A sentence at the top of the page.
    pub summary: String,
    /// The settings, top to bottom.
    pub rows: Vec<Row>,
}

/// One labelled control, with help below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The text left of the control (empty for none).
    pub label: String,
    /// The control.
    pub spec: Spec,
    /// Help text below the control (empty for none).
    pub help: String,
    /// The accessibility identifier of the control (its config key), for UI tests.
    pub id: String,
}

/// A control and the tag it reports with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    /// A pop-up menu of `titles`.
    Choice {
        /// Reported tag.
        tag: isize,
        /// The entries.
        titles: Vec<String>,
    },
    /// A one-line text field; reports when Return is pressed or it loses focus.
    Text {
        /// Reported tag.
        tag: isize,
    },
    /// A number field with a stepper.
    Number {
        /// Reported tag.
        tag: isize,
        /// Smallest value.
        min: u32,
        /// Largest value.
        max: u32,
        /// The stepper's increment.
        step: u32,
    },
    /// A checkbox titled `title`.
    Toggle {
        /// Reported tag.
        tag: isize,
        /// Its text.
        title: String,
    },
    /// A colour well with a Reset button (reported as [`FormEvent::Reset`]).
    Colour {
        /// Reported tag.
        tag: isize,
    },
    /// A shortcut recorder with a note below it; a click is [`FormEvent::Pressed`], the keys
    /// typed while recording are [`FormEvent::Key`].
    Shortcut {
        /// Reported tag.
        tag: isize,
    },
    /// A list of apps with Add and Remove buttons.
    Apps {
        /// Reported tag.
        tag: isize,
    },
    /// A row of push buttons, each reported as [`FormEvent::Pressed`].
    Buttons(Vec<(isize, String)>),
    /// A preview of labels, drawn as the overlay draws them.
    Preview {
        /// Its tag.
        tag: isize,
    },
    /// A line of text set with [`Form::set_note`].
    Note {
        /// Its tag.
        tag: isize,
    },
}

/// How a note reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Secondary, informative text.
    Plain,
    /// A caution, in orange.
    Warning,
    /// A problem, in red.
    Error,
}

/// What the user did in the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormEvent {
    /// The value of the control with this tag changed.
    Changed(isize),
    /// The button (or shortcut recorder) with this tag was clicked.
    Pressed(isize),
    /// A colour's Reset button.
    Reset(isize),
    /// An app list's Add button.
    AddApp(isize),
    /// An app list's Remove button for the entry at the index.
    RemoveApp(isize, usize),
    /// A key pressed in a recording shortcut recorder.
    Key {
        /// The recorder.
        tag: isize,
        /// macOS virtual key code.
        keycode: u16,
        /// What the key types without modifiers.
        characters: String,
        /// The modifiers held.
        modifiers: ModifierFlags,
    },
}

enum Control {
    Choice(Retained<NSPopUpButton>),
    Text(Retained<NSTextField>),
    Number {
        field: Retained<NSTextField>,
        stepper: Retained<NSStepper>,
    },
    Toggle(Retained<NSButton>),
    Colour {
        well: Retained<NSColorWell>,
        note: Retained<NSTextField>,
    },
    Shortcut {
        button: Retained<Recorder>,
        note: Retained<NSTextField>,
    },
    Apps(Retained<NSScrollView>),
    Preview(Retained<NSView>),
    Note(Retained<NSTextField>),
    Button,
}

/// The settings window.
pub struct Form {
    window: Retained<NSWindow>,
    tabs: Retained<NSTabViewController>,
    controls: HashMap<isize, Control>,
    status: Vec<Retained<NSTextField>>,
    mtm: MainThreadMarker,
}

impl std::fmt::Debug for Form {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Form")
            .field("controls", &self.controls.len())
            .finish_non_exhaustive()
    }
}

fn as_view<T: AsRef<NSView>>(view: &T) -> &NSView {
    view.as_ref()
}

fn stack(
    views: &[&NSView],
    vertical: bool,
    spacing: f64,
    mtm: MainThreadMarker,
) -> Retained<NSStackView> {
    let stack = NSStackView::stackViewWithViews(&NSArray::from_slice(views), mtm);
    stack.setOrientation(if vertical {
        NSUserInterfaceLayoutOrientation::Vertical
    } else {
        NSUserInterfaceLayoutOrientation::Horizontal
    });
    stack.setAlignment(if vertical {
        NSLayoutAttribute::Leading
    } else {
        NSLayoutAttribute::CenterY
    });
    stack.setSpacing(spacing);
    stack
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

fn set_tone(field: &NSTextField, text: &str, tone: Tone) {
    field.setStringValue(&NSString::from_str(text));
    let colour = match tone {
        Tone::Plain => NSColor::secondaryLabelColor(),
        Tone::Warning => NSColor::systemOrangeColor(),
        Tone::Error => NSColor::systemRedColor(),
    };
    field.setTextColor(Some(&colour));
}

fn ns_colour(colour: Rgba) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(colour.red, colour.green, colour.blue, colour.alpha)
}

fn identify(view: &NSView, id: &str) {
    if !id.is_empty() {
        view.setAccessibilityIdentifier(Some(&NSString::from_str(id)));
    }
}

/// Points `control` at the shared target with `tag`, and names it `id` for accessibility.
fn wire(control: &NSControl, tag: isize, id: &str, mtm: MainThreadMarker) {
    let target = target(mtm);
    // SAFETY: the shared target implements `act:` and lives for the thread.
    unsafe {
        control.setTarget(Some(&target));
        control.setAction(Some(action()));
    }
    control.setTag(tag);
    identify(control, id);
}

/// An editable text field `width` points wide that reports when editing ends.
fn text_field(width: f64, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let field = NSTextField::textFieldWithString(&NSString::new(), mtm);
    fix_size(&field, Some(width), None);
    if let Some(cell) = field.cell() {
        cell.setSendsActionOnEndEditing(true);
    }
    field
}

/// Pins `view`'s width and height (when given) with layout constraints.
fn fix_size(view: &NSView, width: Option<f64>, height: Option<f64>) {
    if let Some(width) = width {
        view.widthAnchor()
            .constraintEqualToConstant(width)
            .setActive(true);
    }
    if let Some(height) = height {
        view.heightAnchor()
            .constraintEqualToConstant(height)
            .setActive(true);
    }
}

impl Form {
    /// Builds the window (hidden) titled `title` with `pages`.
    pub fn new(mtm: MainThreadMarker, title: &str, pages: &[Page]) -> Self {
        let tabs = NSTabViewController::new(mtm);
        tabs.setTabStyle(NSTabViewControllerTabStyle::Toolbar);
        let mut form = Self {
            window: Self::window(mtm, title),
            tabs,
            controls: HashMap::new(),
            status: Vec::new(),
            mtm,
        };
        for page in pages {
            let view = form.page(page);
            let controller = NSViewController::new(mtm);
            controller.setView(&view);
            controller.setTitle(Some(&NSString::from_str(&page.title)));
            controller.setPreferredContentSize(view.fittingSize());
            let item = NSTabViewItem::tabViewItemWithViewController(&controller);
            item.setLabel(&NSString::from_str(&page.title));
            if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(&page.symbol),
                Some(&NSString::from_str(&page.title)),
            ) {
                item.setImage(Some(&image));
            }
            form.tabs.addTabViewItem(&item);
        }
        form.window.setContentViewController(Some(&form.tabs));
        form.window.center();
        form
    }

    fn window(mtm: MainThreadMarker, title: &str) -> Retained<NSWindow> {
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable;
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(PAGE_WIDTH, 400.0));
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
        // SAFETY: the window is owned by `Form`; AppKit must not free it on close.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(title));
        window.setToolbarStyle(NSWindowToolbarStyle::Preference);
        window.setIdentifier(Some(&NSString::from_str("nomouse-settings")));
        window
    }

    fn page(&mut self, page: &Page) -> Retained<NSStackView> {
        let mtm = self.mtm;
        let summary = help(&page.summary, PAGE_WIDTH - 40.0, mtm);
        let status = label("", mtm);
        status.setFont(Some(&objc2_app_kit::NSFont::systemFontOfSize(
            objc2_app_kit::NSFont::smallSystemFontSize(),
        )));
        status.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        status.setMaximumNumberOfLines(1);
        status.setPreferredMaxLayoutWidth(PAGE_WIDTH - 40.0);
        identify(&status, "settings.status");
        self.status.push(status.clone());

        let mut rows: Vec<Retained<NSArray<NSView>>> = Vec::new();
        let mut help_rows = Vec::new();
        for row in &page.rows {
            let control = self.control(&row.spec, &row.id);
            let title = label(&row.label, mtm);
            rows.push(NSArray::from_slice(&[as_view(&title), &*control]));
            if !row.help.is_empty() {
                help_rows.push(rows.len());
                let text = help(&row.help, TEXT_WIDTH, mtm);
                let empty = NSGridCell::emptyContentView(mtm);
                rows.push(NSArray::from_slice(&[&*empty, as_view(&text)]));
            }
        }
        let grid = NSGridView::gridViewWithViews(&NSArray::from_retained_slice(&rows), mtm);
        grid.setRowSpacing(10.0);
        grid.setColumnSpacing(12.0);
        grid.setYPlacement(NSGridCellPlacement::Center);
        if grid.numberOfColumns() > 0 {
            grid.columnAtIndex(0)
                .setXPlacement(NSGridCellPlacement::Trailing);
        }
        for index in help_rows {
            if let Ok(index) = isize::try_from(index) {
                grid.rowAtIndex(index).setTopPadding(-6.0);
            }
        }

        let page_view = stack(
            &[as_view(&summary), as_view(&status), as_view(&grid)],
            true,
            14.0,
            mtm,
        );
        page_view.setEdgeInsets(NSEdgeInsets {
            top: 20.0,
            left: 20.0,
            bottom: 20.0,
            right: 20.0,
        });
        page_view
            .widthAnchor()
            .constraintEqualToConstant(PAGE_WIDTH)
            .setActive(true);
        page_view.layoutSubtreeIfNeeded();
        page_view.setFrameSize(page_view.fittingSize());
        page_view
    }

    /// Makes the control for `spec` and records it under its tag; returns the view to place.
    fn control(&mut self, spec: &Spec, id: &str) -> Retained<NSView> {
        let mtm = self.mtm;
        match spec {
            Spec::Choice { tag, titles } => {
                let popup = NSPopUpButton::initWithFrame_pullsDown(
                    NSPopUpButton::alloc(mtm),
                    NSRect::ZERO,
                    false,
                );
                for title in titles {
                    popup.addItemWithTitle(&NSString::from_str(title));
                }
                wire(&popup, *tag, id, mtm);
                self.controls.insert(*tag, Control::Choice(popup.clone()));
                as_view(&popup).retain()
            }
            Spec::Text { tag } => {
                let field = text_field(220.0, mtm);
                wire(&field, *tag, id, mtm);
                self.controls.insert(*tag, Control::Text(field.clone()));
                as_view(&field).retain()
            }
            Spec::Number {
                tag,
                min,
                max,
                step,
            } => self.number_field(*tag, (*min, *max, *step), id),
            Spec::Toggle { tag, title } => {
                let target = target(mtm);
                // SAFETY: the shared target implements `act:` and lives for the thread.
                let check = unsafe {
                    NSButton::checkboxWithTitle_target_action(
                        &NSString::from_str(title),
                        Some(&target),
                        Some(action()),
                        mtm,
                    )
                };
                check.setTag(*tag);
                identify(&check, id);
                self.controls.insert(*tag, Control::Toggle(check.clone()));
                as_view(&check).retain()
            }
            Spec::Colour { tag } => self.colour_well(*tag, id),
            Spec::Shortcut { tag } => {
                let recorder = Recorder::new(*tag, mtm);
                wire(&recorder, *tag, id, mtm);
                fix_size(&recorder, Some(180.0), None);
                let note = help("", TEXT_WIDTH, mtm);
                let column = stack(&[as_view(&recorder), as_view(&note)], true, 4.0, mtm);
                self.controls.insert(
                    *tag,
                    Control::Shortcut {
                        button: recorder,
                        note,
                    },
                );
                as_view(&column).retain()
            }
            Spec::Apps { tag } => self.app_list(*tag, id),
            Spec::Buttons(buttons) => {
                let made: Vec<Retained<NSButton>> = buttons
                    .iter()
                    .map(|(tag, title)| {
                        self.controls.insert(*tag, Control::Button);
                        let made = button(*tag, title, mtm);
                        identify(&made, title);
                        made
                    })
                    .collect();
                let views: Vec<&NSView> = made.iter().map(as_view).collect();
                as_view(&stack(&views, false, 8.0, mtm)).retain()
            }
            Spec::Preview { tag } => {
                let frame = NSRect::new(NSPoint::new(0.0, 0.0), PREVIEW);
                let view = NSView::initWithFrame(NSView::alloc(mtm), frame);
                view.setWantsLayer(true);
                fix_size(&view, Some(PREVIEW.width), Some(PREVIEW.height));
                identify(&view, id);
                self.controls.insert(*tag, Control::Preview(view.clone()));
                view
            }
            Spec::Note { tag } => {
                let note = help("", TEXT_WIDTH, mtm);
                identify(&note, id);
                self.controls.insert(*tag, Control::Note(note.clone()));
                as_view(&note).retain()
            }
        }
    }

    /// A number field with a stepper beside it, the limits being (min, max, step).
    fn number_field(
        &mut self,
        tag: isize,
        (min, max, step): (u32, u32, u32),
        id: &str,
    ) -> Retained<NSView> {
        let mtm = self.mtm;
        let field = text_field(56.0, mtm);
        let stepper = NSStepper::new(mtm);
        stepper.setMinValue(f64::from(min));
        stepper.setMaxValue(f64::from(max));
        stepper.setIncrement(f64::from(step));
        stepper.setValueWraps(false);
        wire(&field, tag, id, mtm);
        wire(&stepper, tag + STEPPER, "", mtm);
        let row = stack(&[as_view(&field), as_view(&stepper)], false, 4.0, mtm);
        self.controls
            .insert(tag, Control::Number { field, stepper });
        as_view(&row).retain()
    }

    /// A colour well, a button back to the theme's colour and a note saying which applies.
    fn colour_well(&mut self, tag: isize, id: &str) -> Retained<NSView> {
        let mtm = self.mtm;
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(44.0, 24.0));
        let well = NSColorWell::initWithFrame(NSColorWell::alloc(mtm), frame);
        fix_size(&well, Some(44.0), None);
        wire(&well, tag, id, mtm);
        let reset = button(tag + RESET, "Use theme", mtm);
        let note = label("", mtm);
        let row = stack(
            &[as_view(&well), as_view(&reset), as_view(&note)],
            false,
            8.0,
            mtm,
        );
        self.controls.insert(tag, Control::Colour { well, note });
        as_view(&row).retain()
    }

    /// A scrolling list of apps with an Add button below it.
    fn app_list(&mut self, tag: isize, id: &str) -> Retained<NSView> {
        let mtm = self.mtm;
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), APP_LIST);
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
        scroll.setHasVerticalScroller(true);
        scroll.setBorderType(objc2_app_kit::NSBorderType::BezelBorder);
        fix_size(&scroll, Some(APP_LIST.width), Some(APP_LIST.height));
        identify(&scroll, id);
        let add = button(tag + ADD, "Add App…", mtm);
        let column = stack(&[as_view(&scroll), as_view(&add)], true, 6.0, mtm);
        self.controls.insert(tag, Control::Apps(scroll));
        self.set_apps(tag, &[]);
        as_view(&column).retain()
    }

    /// Turns an interface event into what it means for this window, or `None` when it
    /// belongs to something else. Number fields and steppers are kept in step here.
    pub fn translate(&self, event: &UiEvent) -> Option<FormEvent> {
        match event {
            UiEvent::Key {
                tag,
                keycode,
                characters,
                modifiers,
            } => self.controls.contains_key(tag).then(|| FormEvent::Key {
                tag: *tag,
                keycode: *keycode,
                characters: characters.clone(),
                modifiers: *modifiers,
            }),
            UiEvent::Action(raw) => {
                let part = raw & PART;
                let tag = raw & TAG;
                let control = self.controls.get(&tag)?;
                Some(match (part, control) {
                    (STEPPER, Control::Number { field, stepper }) => {
                        field.setIntegerValue(stepper.integerValue());
                        FormEvent::Changed(tag)
                    }
                    (0, Control::Number { field, stepper }) => {
                        let typed = field.stringValue().to_string();
                        let value = typed
                            .trim()
                            .parse::<f64>()
                            .unwrap_or_else(|_| stepper.doubleValue())
                            .clamp(stepper.minValue(), stepper.maxValue());
                        stepper.setDoubleValue(value);
                        field.setIntegerValue(stepper.integerValue());
                        FormEvent::Changed(tag)
                    }
                    (RESET, Control::Colour { .. }) => FormEvent::Reset(tag),
                    (ADD, Control::Apps(_)) => FormEvent::AddApp(tag),
                    (REMOVE, Control::Apps(_)) => {
                        FormEvent::RemoveApp(tag, usize::try_from(raw >> INDEX_SHIFT).ok()?)
                    }
                    (0, Control::Button | Control::Shortcut { .. }) => FormEvent::Pressed(tag),
                    (0, _) => FormEvent::Changed(tag),
                    _ => return None,
                })
            }
        }
    }

    /// Shows the window in front, on the page at `page` when given.
    pub fn show(&self, page: Option<usize>) {
        if let Some(index) = page.and_then(|page| isize::try_from(page).ok()) {
            self.tabs.setSelectedTabViewItemIndex(index);
        }
        activate(self.mtm);
        self.window.makeKeyAndOrderFront(None);
    }

    /// Whether the window is on screen.
    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }

    /// Closes the window (it can be shown again).
    pub fn close(&self) {
        self.window.close();
    }

    /// The selected page.
    pub fn page_index(&self) -> usize {
        usize::try_from(self.tabs.selectedTabViewItemIndex()).unwrap_or(0)
    }

    /// The line at the top of every page: where settings are saved, or why they are not in
    /// effect.
    pub fn set_status(&self, text: &str, tone: Tone) {
        for status in &self.status {
            set_tone(status, text, tone);
            status.setToolTip(Some(&NSString::from_str(text)));
        }
    }

    /// Selects entry `index` of a [`Spec::Choice`].
    pub fn set_choice(&self, tag: isize, index: usize) {
        if let (Some(Control::Choice(popup)), Ok(index)) =
            (self.controls.get(&tag), isize::try_from(index))
        {
            popup.selectItemAtIndex(index);
        }
    }

    /// The selected entry of a [`Spec::Choice`].
    pub fn choice(&self, tag: isize) -> Option<usize> {
        match self.controls.get(&tag) {
            Some(Control::Choice(popup)) => usize::try_from(popup.indexOfSelectedItem()).ok(),
            _ => None,
        }
    }

    /// Sets the text of a [`Spec::Text`], unless the user is typing in it.
    pub fn set_text(&self, tag: isize, text: &str) {
        if let Some(Control::Text(field)) = self.controls.get(&tag)
            && field.currentEditor().is_none()
        {
            field.setStringValue(&NSString::from_str(text));
        }
    }

    /// The text of a [`Spec::Text`].
    pub fn text(&self, tag: isize) -> String {
        match self.controls.get(&tag) {
            Some(Control::Text(field)) => field.stringValue().to_string(),
            _ => String::new(),
        }
    }

    /// Sets a [`Spec::Number`].
    pub fn set_number(&self, tag: isize, value: u32) {
        if let Some(Control::Number { field, stepper }) = self.controls.get(&tag) {
            stepper.setDoubleValue(f64::from(value));
            if field.currentEditor().is_none() {
                field.setIntegerValue(stepper.integerValue());
            }
        }
    }

    /// The value of a [`Spec::Number`].
    pub fn number(&self, tag: isize) -> u32 {
        match self.controls.get(&tag) {
            Some(Control::Number { stepper, .. }) => {
                u32::try_from(stepper.integerValue()).unwrap_or(0)
            }
            _ => 0,
        }
    }

    /// Ticks or clears a [`Spec::Toggle`].
    pub fn set_toggle(&self, tag: isize, on: bool) {
        if let Some(Control::Toggle(check)) = self.controls.get(&tag) {
            check.setState(if on {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
    }

    /// Whether a [`Spec::Toggle`] is ticked.
    pub fn toggle(&self, tag: isize) -> bool {
        match self.controls.get(&tag) {
            Some(Control::Toggle(check)) => check.state() == NSControlStateValueOn,
            _ => false,
        }
    }

    /// Shows `colour` in a [`Spec::Colour`], noting whether it is the theme's.
    pub fn set_colour(&self, tag: isize, colour: Rgba, from_theme: bool) {
        if let Some(Control::Colour { well, note }) = self.controls.get(&tag) {
            if !well.isActive() {
                well.setColor(&ns_colour(colour));
            }
            set_tone(
                note,
                if from_theme { "theme" } else { "custom" },
                Tone::Plain,
            );
        }
    }

    /// The colour of a [`Spec::Colour`], in sRGB.
    pub fn colour(&self, tag: isize) -> Option<Rgba> {
        let Some(Control::Colour { well, .. }) = self.controls.get(&tag) else {
            return None;
        };
        let colour = well
            .color()
            .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        Some(Rgba {
            red: colour.redComponent(),
            green: colour.greenComponent(),
            blue: colour.blueComponent(),
            alpha: colour.alphaComponent(),
        })
    }

    /// Shows `title` on a [`Spec::Shortcut`] and `note` (with its tone) below it.
    pub fn set_shortcut(&self, tag: isize, title: &str, note: &str, tone: Tone) {
        if let Some(Control::Shortcut { button, note: line }) = self.controls.get(&tag) {
            button.setTitle(&NSString::from_str(title));
            set_tone(line, note, tone);
        }
    }

    /// Starts recording in a [`Spec::Shortcut`]: it shows `prompt` and takes the next key
    /// press. Any other recording stops.
    pub fn record(&self, tag: isize, prompt: &str) {
        if let Some(Control::Shortcut { button, .. }) = self.controls.get(&tag) {
            set_recording(Some(tag));
            button.setTitle(&NSString::from_str(prompt));
            self.window.makeFirstResponder(Some(button));
        }
    }

    /// Stops recording.
    pub fn stop_recording(&self) {
        set_recording(None);
    }

    /// Fills a [`Spec::Apps`] list.
    pub fn set_apps(&self, tag: isize, apps: &[AppInfo]) {
        let Some(Control::Apps(scroll)) = self.controls.get(&tag) else {
            return;
        };
        let mtm = self.mtm;
        let mut rows: Vec<Retained<NSStackView>> = Vec::new();
        for (index, app) in apps.iter().enumerate() {
            let name = label(&app.name, mtm);
            let mut views: Vec<Retained<NSView>> = Vec::new();
            if let Some(path) = &app.path {
                let image = NSImageView::imageViewWithImage(&icon(path, 18.0), mtm);
                views.push(as_view(&image).retain());
            }
            views.push(as_view(&name).retain());
            if app.name != app.id {
                let id = label(&app.id, mtm);
                id.setTextColor(Some(&NSColor::secondaryLabelColor()));
                views.push(as_view(&id).retain());
            }
            let remove = button(
                tag + REMOVE + (isize::try_from(index).unwrap_or(0) << INDEX_SHIFT),
                "Remove",
                mtm,
            );
            remove.setControlSize(objc2_app_kit::NSControlSize::Small);
            remove
                .setAccessibilityLabel(Some(&NSString::from_str(&format!("Remove {}", app.name))));
            let spacer = NSView::new(mtm);
            spacer.setContentHuggingPriority_forOrientation(
                1.0,
                objc2_app_kit::NSLayoutConstraintOrientation::Horizontal,
            );
            views.push(spacer);
            views.push(as_view(&remove).retain());
            let refs: Vec<&NSView> = views.iter().map(|view| &**view).collect();
            let row = stack(&refs, false, 8.0, mtm);
            row.widthAnchor()
                .constraintEqualToConstant(APP_LIST.width - 24.0)
                .setActive(true);
            rows.push(row);
        }
        let empty;
        let refs: Vec<&NSView> = if rows.is_empty() {
            empty = help("No apps yet: Add App… picks one.", TEXT_WIDTH, mtm);
            vec![as_view(&empty)]
        } else {
            rows.iter().map(as_view).collect()
        };
        let list = stack(&refs, true, 6.0, mtm);
        list.setEdgeInsets(NSEdgeInsets {
            top: 8.0,
            left: 8.0,
            bottom: 8.0,
            right: 8.0,
        });
        list.layoutSubtreeIfNeeded();
        let size = list.fittingSize();
        list.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(APP_LIST.width - 4.0, size.height.max(APP_LIST.height - 4.0)),
        ));
        scroll.setDocumentView(Some(&list));
    }

    /// Draws `labels` in a [`Spec::Preview`] with `style` at `opacity` (0.0 to 1.0) over a
    /// light or dark backdrop. Label positions are from the preview's top-left corner.
    pub fn set_preview(
        &self,
        tag: isize,
        labels: &[Label],
        style: &LabelStyle,
        opacity: f32,
        dark: bool,
    ) {
        let Some(Control::Preview(view)) = self.controls.get(&tag) else {
            return;
        };
        let root = CALayer::new();
        root.setFrame(CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(PREVIEW.width, PREVIEW.height),
        ));
        let backdrop = if dark {
            Rgba {
                red: 0.16,
                green: 0.17,
                blue: 0.2,
                alpha: 1.0,
            }
        } else {
            Rgba {
                red: 0.93,
                green: 0.94,
                blue: 0.96,
                alpha: 1.0,
            }
        };
        root.setBackgroundColor(Some(&cg_color(backdrop)));
        root.setCornerRadius(8.0);
        let scale = self.window.backingScaleFactor();
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        for label in labels {
            let flipped = Label {
                y: PREVIEW.height - label.y,
                ..label.clone()
            };
            root.addSublayer(&label_layer(
                &flipped,
                style,
                (PREVIEW.width, PREVIEW.height),
                scale,
                opacity,
            ));
        }
        view.setLayer(Some(&root));
        CATransaction::commit();
    }

    /// Sets a [`Spec::Note`].
    pub fn set_note(&self, tag: isize, text: &str, tone: Tone) {
        if let Some(Control::Note(note)) = self.controls.get(&tag) {
            set_tone(note, text, tone);
        }
    }
}

impl Drop for Form {
    fn drop(&mut self) {
        set_recording(None);
        self.window.close();
    }
}
