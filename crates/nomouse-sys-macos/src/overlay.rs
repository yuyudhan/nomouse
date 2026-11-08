//! The hint overlay: one borderless, transparent, click-through `NSPanel` per screen, above
//! every window (screen-saver level) and on every Space, with labels drawn as Core Animation
//! layers.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFont, NSPanel, NSScreenSaverWindowLevel,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CFRetained, CFString, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGColor;
use objc2_foundation::NSString;
use objc2_quartz_core::{CALayer, CATextLayer, CATransaction, kCAAlignmentCenter};

use crate::app::App;
use crate::types::{Label, LabelStyle, Rgba, ScreenFrame};

/// The monospaced font the labels use; Menlo ships with every macOS release.
const FONT_NAME: &str = "Menlo-Bold";
/// Horizontal padding inside a label box, in points.
const PAD_X: f64 = 3.0;
/// Vertical padding inside a label box, in points.
const PAD_Y: f64 = 1.0;

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
}

impl std::fmt::Debug for Overlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Overlay")
            .field("panels", &self.panels.len())
            .field("visible", &self.visible)
            .finish()
    }
}

fn cg_color(colour: Rgba) -> CFRetained<CGColor> {
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

fn label_layer(
    label: &Label,
    style: &LabelStyle,
    panel: &Panel,
    opacity: f32,
) -> Retained<CALayer> {
    let advance = char_width(style.font_size);
    let typed: String = label.text.chars().take(label.typed).collect();
    let rest: String = label.text.chars().skip(label.typed).collect();
    let typed_width = advance * text_units(&typed);
    let rest_width = advance * text_units(&rest);
    let line_height = (style.font_size * 1.25).ceil();
    let width = typed_width + rest_width + 2.0 * PAD_X;
    let height = line_height + 2.0 * PAD_Y;
    let x = label.x.clamp(0.0, (panel.frame.width - width).max(0.0));
    let y = (label.y - height).clamp(0.0, (panel.frame.height - height).max(0.0));

    let container = CALayer::new();
    container.setFrame(rect(x, y, width, height));
    container.setBackgroundColor(Some(&cg_color(style.background)));
    container.setBorderColor(Some(&cg_color(style.border)));
    container.setBorderWidth(1.0);
    container.setCornerRadius(3.0);
    container.setOpacity(opacity);
    container.setContentsScale(panel.scale);
    if !typed.is_empty() {
        let part = text_layer(&typed, style, style.typed_foreground, panel.scale);
        part.setFrame(rect(PAD_X, PAD_Y, typed_width, line_height));
        container.addSublayer(&part);
    }
    if !rest.is_empty() {
        let part = text_layer(&rest, style, style.foreground, panel.scale);
        part.setFrame(rect(PAD_X + typed_width, PAD_Y, rest_width, line_height));
        container.addSublayer(&part);
    }
    container
}

impl Overlay {
    /// An overlay with no panels yet.
    pub fn new() -> Self {
        Self::default()
    }

    fn sync_panels(&mut self, app: &App) {
        let frames = app.screens();
        let scales = app.screen_scales();
        let unchanged = self.panels.len() == frames.len()
            && self
                .panels
                .iter()
                .zip(&frames)
                .all(|(panel, frame)| panel.frame == *frame);
        if unchanged {
            return;
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
    }

    /// Shows `labels` (replacing any shown before) on the screens [`App::screens`] reports
    /// now, with `opacity` from 0.0 to 1.0. Labels whose screen index is out of range are
    /// skipped.
    pub fn show(&mut self, app: &App, labels: &[Label], style: &LabelStyle, opacity: f32) {
        self.sync_panels(app);
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        for panel in &self.panels {
            // SAFETY: clearing the sublayer array with `None` is always valid.
            unsafe { panel.root.setSublayers(None) };
        }
        for label in labels {
            if let Some(panel) = self.panels.get(label.screen) {
                panel
                    .root
                    .addSublayer(&label_layer(label, style, panel, opacity));
            }
        }
        CATransaction::commit();
        for panel in &self.panels {
            panel.window.orderFrontRegardless();
        }
        CATransaction::flush();
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
