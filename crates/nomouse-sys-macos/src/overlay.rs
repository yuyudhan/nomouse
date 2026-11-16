//! The hint overlay: one borderless, transparent, click-through `NSPanel` per screen, above
//! every window (screen-saver level) and on every Space, with labels drawn as Core Animation
//! layers.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFont, NSPanel, NSScreenSaverWindowLevel,
    NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CFRetained, CFString, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGColor;
use objc2_foundation::NSString;
use objc2_quartz_core::{CALayer, CATextLayer, CATransaction, kCAAlignmentCenter};

use crate::app::App;
use crate::types::{Label, LabelStyle, Outline, Rgba, ScreenFrame};

/// The monospaced font the labels use; Menlo ships with every macOS release.
const FONT_NAME: &str = "Menlo-Bold";
/// Horizontal padding inside a label box, in points.
const PAD_X: f64 = 3.0;
/// Vertical padding inside a label box, in points.
const PAD_Y: f64 = 1.0;
/// Thickness of the outline around scroll mode's active area, in points.
const OUTLINE_WIDTH: f64 = 3.0;

struct Panel {
    window: Retained<NSPanel>,
    root: Retained<CALayer>,
    frame: ScreenFrame,
    scale: f64,
}

/// The overlay windows. Create with [`Overlay::new`]; panels are made lazily per screen.
#[derive(Default)]
pub struct Overlay {
    panels: Vec<Panel>,
    visible: bool,
    /// What the panels show now, so a frame that only adds labels draws just those.
    drawn: Option<Drawn>,
}

/// The contents of the panels, as last drawn.
#[derive(Debug, Clone)]
struct Drawn {
    labels: Vec<Label>,
    outlines: Vec<Outline>,
    style: LabelStyle,
    opacity: f32,
}

/// The labels of `now` that come after the ones `before` shows, when `now` is `before` plus
/// labels after it (with the same outlines, style and opacity); `None` when anything else
/// changed and the panels must be drawn again from scratch.
fn added<'a>(before: Option<&Drawn>, now: &'a Drawn) -> Option<&'a [Label]> {
    let before = before?;
    let same_rest = before.outlines == now.outlines
        && before.style == now.style
        && before.opacity.to_bits() == now.opacity.to_bits();
    (same_rest && now.labels.starts_with(&before.labels))
        .then(|| &now.labels[before.labels.len()..])
}

impl std::fmt::Debug for Overlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Overlay")
            .field("panels", &self.panels.len())
            .field("visible", &self.visible)
            .field("drawn", &self.drawn)
            .finish()
    }
}

pub(crate) fn cg_color(colour: Rgba) -> CFRetained<CGColor> {
    CGColor::new_srgb(colour.red, colour.green, colour.blue, colour.alpha)
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> CGRect {
    CGRect::new(CGPoint::new(x, y), CGSize::new(width, height))
}

fn new_panel(mtm: MainThreadMarker, frame: ScreenFrame, scale: f64) -> Panel {
    let content = rect(frame.x, frame.y, frame.width, frame.height);
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    // `defer: false` creates the window device immediately.
    let window = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        content,
        style,
        NSBackingStoreType::Buffered,
        false,
    );
    // SAFETY: the panel is owned by this struct; AppKit must not free it on close.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(false);
    window.setIgnoresMouseEvents(true);
    window.setFloatingPanel(true);
    window.setHidesOnDeactivate(false);
    window.setLevel(NSScreenSaverWindowLevel);
    // Panels fade out when ordered out; the labels must vanish at once, before the click
    // lands under them.
    window.setAnimationBehavior(NSWindowAnimationBehavior::None);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    let root = CALayer::new();
    root.setFrame(rect(0.0, 0.0, frame.width, frame.height));
    if let Some(view) = window.contentView() {
        view.setWantsLayer(true);
        view.setLayer(Some(&root));
    }
    window.setFrame_display(content, false);
    Panel {
        window,
        root,
        frame,
        scale,
    }
}

fn text_layer(text: &str, style: &LabelStyle, colour: Rgba, scale: f64) -> Retained<CATextLayer> {
    let layer = CATextLayer::new();
    let string = NSString::from_str(text);
    let font = CFString::from_static_str(FONT_NAME);
    // SAFETY: CATextLayer accepts an NSString for `string` and a font name (CFString) for
    // `font`; both objects are valid and retained by the layer.
    unsafe {
        layer.setString(Some(AsRef::<AnyObject>::as_ref(&*string)));
        layer.setFont(Some(&font));
    }
    layer.setFontSize(style.font_size);
    layer.setForegroundColor(Some(&cg_color(colour)));
    // SAFETY: kCAAlignmentCenter is an immutable NSString constant.
    layer.setAlignmentMode(unsafe { kCAAlignmentCenter });
    layer.setContentsScale(scale);
    layer
}

/// The advance of one character of the label font, in points.
fn char_width(font_size: f64) -> f64 {
    NSFont::fontWithName_size(&NSString::from_str(FONT_NAME), font_size)
        .map_or(font_size * 0.6, |font| font.maximumAdvancement().width)
}

/// The number of characters in `text` as a float (labels are a few characters long).
fn text_units(text: &str) -> CGFloat {
    CGFloat::from(u32::try_from(text.chars().count()).unwrap_or(u32::MAX))
}

/// Measures label boxes drawn with `style`: the returned function gives the width and height
/// of the box a label with a given text is drawn in, in points (the font is looked up once).
pub fn label_sizer(style: &LabelStyle) -> impl Fn(&str) -> (f64, f64) + use<> {
    let (style, advance) = (*style, char_width(style.font_size));
    move |text| box_size(advance, text, &style)
}

/// The box of a label with `text` whose characters advance by `advance` points.
fn box_size(advance: f64, text: &str, style: &LabelStyle) -> (f64, f64) {
    (
        advance * text_units(text) + 2.0 * PAD_X,
        line_height(style) + 2.0 * PAD_Y,
    )
}

/// The height of a label's text line, in points.
fn line_height(style: &LabelStyle) -> f64 {
    (style.font_size * 1.25).ceil()
}

/// One label as a layer, its box sized to the text and kept inside an area of `width` by
/// `height` points (a screen's panel, or a settings window's preview) drawn at `scale`.
pub(crate) fn label_layer(
    label: &Label,
    style: &LabelStyle,
    (width, height): (f64, f64),
    scale: f64,
    opacity: f32,
) -> Retained<CALayer> {
    let advance = char_width(style.font_size);
    let typed: String = label.text.chars().take(label.typed).collect();
    let rest: String = label.text.chars().skip(label.typed).collect();
    let typed_width = advance * text_units(&typed);
    let rest_width = advance * text_units(&rest);
    let line_height = line_height(style);
    let (box_width, box_height) = box_size(advance, &label.text, style);
    let x = label.x.clamp(0.0, (width - box_width).max(0.0));
    let y = (label.y - box_height).clamp(0.0, (height - box_height).max(0.0));

    let container = CALayer::new();
    container.setFrame(rect(x, y, box_width, box_height));
    // The highlighted label swaps background and text colours, so it stands out.
    let (fill, text) = if label.focused {
        (style.foreground, style.background)
    } else {
        (style.background, style.foreground)
    };
    container.setBackgroundColor(Some(&cg_color(fill)));
    container.setBorderColor(Some(&cg_color(style.border)));
    container.setBorderWidth(1.0);
    container.setCornerRadius(3.0);
    container.setOpacity(opacity);
    container.setContentsScale(scale);
    if !typed.is_empty() {
        let part = text_layer(&typed, style, style.typed_foreground, scale);
        part.setFrame(rect(PAD_X, PAD_Y, typed_width, line_height));
        container.addSublayer(&part);
    }
    if !rest.is_empty() {
        let part = text_layer(&rest, style, text, scale);
        part.setFrame(rect(PAD_X + typed_width, PAD_Y, rest_width, line_height));
        container.addSublayer(&part);
    }
    container
}

/// An unfilled frame in the label background colour around `outline`.
fn outline_layer(
    outline: &Outline,
    style: &LabelStyle,
    panel: &Panel,
    opacity: f32,
) -> Retained<CALayer> {
    let layer = CALayer::new();
    layer.setFrame(rect(outline.x, outline.y, outline.width, outline.height));
    layer.setBorderColor(Some(&cg_color(style.background)));
    layer.setBorderWidth(OUTLINE_WIDTH);
    layer.setOpacity(opacity);
    layer.setContentsScale(panel.scale);
    layer
}

impl Overlay {
    /// An overlay with no panels yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes one panel per screen, unless the screens are the ones the panels cover; returns
    /// whether the panels were made again.
    fn sync_panels(&mut self, app: &App) -> bool {
        let frames = app.screens();
        let scales = app.screen_scales();
        let unchanged = self.panels.len() == frames.len()
            && self
                .panels
                .iter()
                .zip(&frames)
                .all(|(panel, frame)| panel.frame == *frame);
        if unchanged {
            return false;
        }
        for panel in self.panels.drain(..) {
            panel.window.orderOut(None);
            panel.window.close();
        }
        self.panels = frames
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                let scale = scales.get(index).copied().unwrap_or(1.0);
                new_panel(app.main_thread(), *frame, scale)
            })
            .collect();
        true
    }

    /// Adds `label`'s layer to the panel of its screen; one out of range is skipped.
    fn add_label(&self, label: &Label, style: &LabelStyle, opacity: f32) {
        if let Some(panel) = self.panels.get(label.screen) {
            let area = (panel.frame.width, panel.frame.height);
            panel
                .root
                .addSublayer(&label_layer(label, style, area, panel.scale, opacity));
        }
    }

    /// Shows `labels` above `outlines` on the screens [`App::screens`] reports now, with
    /// `opacity` from 0.0 to 1.0. Labels and outlines whose screen index is out of range are
    /// skipped. When the labels shown are the first of `labels` and nothing else changed, only
    /// the labels after them are drawn: the ones on screen are not touched at all. Anything
    /// else replaces what was shown.
    pub fn show(
        &mut self,
        app: &App,
        labels: &[Label],
        outlines: &[Outline],
        style: &LabelStyle,
        opacity: f32,
    ) {
        let remade = self.sync_panels(app);
        let now = Drawn {
            labels: labels.to_vec(),
            outlines: outlines.to_vec(),
            style: *style,
            opacity,
        };
        let before = if remade || !self.visible {
            None
        } else {
            self.drawn.as_ref()
        };
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        if let Some(new) = added(before, &now) {
            for label in new {
                self.add_label(label, style, opacity);
            }
        } else {
            for panel in &self.panels {
                // SAFETY: clearing the sublayer array with `None` is always valid.
                unsafe { panel.root.setSublayers(None) };
            }
            for outline in outlines {
                if let Some(panel) = self.panels.get(outline.screen) {
                    panel
                        .root
                        .addSublayer(&outline_layer(outline, style, panel, opacity));
                }
            }
            for label in labels {
                self.add_label(label, style, opacity);
            }
        }
        CATransaction::commit();
        for panel in &self.panels {
            panel.window.orderFrontRegardless();
        }
        CATransaction::flush();
        self.drawn = Some(now);
        self.visible = true;
    }

    /// Hides every panel and drops their labels. A no-op while hidden.
    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        for panel in &self.panels {
            // SAFETY: clearing the sublayer array with `None` is always valid.
            unsafe { panel.root.setSublayers(None) };
            panel.window.orderOut(None);
        }
        CATransaction::commit();
        CATransaction::flush();
        self.drawn = None;
        self.visible = false;
    }

    /// Whether labels are currently shown.
    pub const fn is_visible(&self) -> bool {
        self.visible
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        for panel in self.panels.drain(..) {
            panel.window.orderOut(None);
            panel.window.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Drawn, added};
    use crate::types::{Label, LabelStyle, Outline, Rgba};

    fn label(text: &str, x: f64) -> Label {
        Label {
            screen: 0,
            x,
            y: 10.0,
            text: text.to_owned(),
            typed: 0,
            focused: false,
        }
    }

    fn drawn(labels: &[Label]) -> Drawn {
        let black = Rgba {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        };
        Drawn {
            labels: labels.to_vec(),
            outlines: Vec::new(),
            style: LabelStyle {
                font_size: 12.0,
                background: black,
                foreground: black,
                typed_foreground: black,
                border: black,
            },
            opacity: 1.0,
        }
    }

    #[test]
    fn a_frame_that_only_adds_labels_draws_only_those() {
        let shown = drawn(&[label("a", 0.0), label("s", 50.0)]);
        let grown = drawn(&[label("a", 0.0), label("s", 50.0), label("d", 90.0)]);
        assert_eq!(added(Some(&shown), &grown), Some(&grown.labels[2..]));
        assert_eq!(added(Some(&shown), &shown), Some(&shown.labels[2..]));
    }

    #[test]
    fn any_other_change_draws_everything_again() {
        let shown = drawn(&[label("a", 0.0), label("s", 50.0)]);
        assert_eq!(added(None, &shown), None);
        // A typed prefix, a highlight, a dropped or moved label, or a new outline.
        let mut typed = shown.clone();
        typed.labels[0].typed = 1;
        assert_eq!(added(Some(&shown), &typed), None);
        let mut focused = shown.clone();
        focused.labels[1].focused = true;
        assert_eq!(added(Some(&shown), &focused), None);
        assert_eq!(added(Some(&shown), &drawn(&[label("s", 50.0)])), None);
        assert_eq!(
            added(Some(&shown), &drawn(&[label("a", 1.0), label("s", 50.0)])),
            None
        );
        let mut outlined = shown.clone();
        outlined.outlines.push(Outline {
            screen: 0,
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });
        assert_eq!(added(Some(&shown), &outlined), None);
    }
}
