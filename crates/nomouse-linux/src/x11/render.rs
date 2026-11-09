//! Label rendering: glyph outlines from the shared `nomouse_core::glyph` rasteriser (skrifa)
//! filled as tiny-skia paths, converted to X's premultiplied BGRA byte order.

use std::path::{Path, PathBuf};

use nomouse_core::config::OverlayConfig;
use nomouse_core::glyph::LabelFont;
use nomouse_core::search::search_box_anchor;
use nomouse_core::{Frame, PlatformError, Rect};
use tiny_skia::{Color, Paint, Pixmap, Transform};

/// Bold sans fonts tried first; any TrueType font under the system font directories after.
const FONT_CANDIDATES: [&str; 6] = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    "/usr/share/fonts/liberation-sans/LiberationSans-Bold.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
];
const FONT_DIRS: [&str; 2] = ["/usr/share/fonts", "/usr/local/share/fonts"];

/// Label colours and sizes, from `[overlay]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Style {
    /// Text size in pixels.
    pub font_px: f32,
    /// Background RGB.
    pub background: [u8; 3],
    /// Untyped text RGB.
    pub foreground: [u8; 3],
    /// Typed prefix RGB.
    pub typed: [u8; 3],
    /// Border RGB.
    pub border: [u8; 3],
    /// Opacity, 0 to 1.
    pub opacity: f32,
}

impl Style {
    /// The style of a validated `[overlay]` section; sizes are points at 96 dpi.
    ///
    /// # Errors
    /// [`PlatformError::Backend`] for an invalid colour.
    pub(crate) fn from_config(config: &OverlayConfig) -> Result<Self, PlatformError> {
        let [background, foreground, typed, border] = config
            .colours()
            .map_err(|e| PlatformError::Backend(e.to_string()))?;
        let points = f32::from(u16::try_from(config.font_size).unwrap_or(13));
        let percent = f32::from(u16::try_from(config.opacity.min(100)).unwrap_or(100));
        Ok(Self {
            font_px: points * 96.0 / 72.0,
            background,
            foreground,
            typed,
            border,
            opacity: percent / 100.0,
        })
    }
}

/// A rendered label at its screen position, premultiplied BGRA rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Label {
    /// Screen bounds.
    pub bounds: Rect,
    /// `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

/// Draws labels with one loaded font.
pub(crate) struct Renderer {
    font: LabelFont,
    style: Style,
}

fn find_font_in(dir: &Path, depth: usize) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    let is_ttf = |path: &PathBuf| path.extension().is_some_and(|ext| ext == "ttf");
    let is_bold = |path: &PathBuf| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().contains("Bold"))
    };
    entries
        .iter()
        .find(|p| is_ttf(p) && is_bold(p))
        .or_else(|| entries.iter().find(|p| is_ttf(p)))
        .cloned()
        .or_else(|| {
            let deeper = depth.checked_sub(1)?;
            entries
                .iter()
                .filter(|p| p.is_dir())
                .find_map(|p| find_font_in(p, deeper))
        })
}

/// Locates a usable font file.
pub(crate) fn find_font() -> Option<PathBuf> {
    FONT_CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .or_else(|| {
            FONT_DIRS
                .iter()
                .find_map(|dir| find_font_in(Path::new(dir), 4))
        })
}

fn paint(rgb: [u8; 3], opacity: f32) -> Paint<'static> {
    let mut colour = Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255);
    colour.set_alpha(opacity.clamp(0.0, 1.0));
    let mut paint = Paint::default();
    paint.set_color(colour);
    paint.anti_alias = true;
    paint
}

/// Places a span of `size` starting at `start` inside `lo..lo + span`.
fn place(start: i64, size: u32, lo: i32, span: u32) -> i32 {
    let max = i64::from(lo) + i64::from(span) - i64::from(size);
    i32::try_from(start.min(max).max(i64::from(lo))).unwrap_or(lo)
}

impl Renderer {
    /// Loads the font and keeps `style`.
    ///
    /// # Errors
    /// [`PlatformError::Backend`] when no TrueType font is installed.
    pub(crate) fn new(style: Style) -> Result<Self, PlatformError> {
        let path = find_font().ok_or_else(|| {
            PlatformError::Backend(
                "no TrueType font found under /usr/share/fonts (install fonts-dejavu-core)"
                    .to_owned(),
            )
        })?;
        let font =
            LabelFont::from_file(&path).map_err(|e| PlatformError::Backend(e.to_string()))?;
        tracing::debug!("label font {}", path.display());
        Ok(Self { font, style })
    }

    /// Renders `text` (shown uppercase) for an element at `anchor`, with the first `typed`
    /// characters in the typed colour, placed at the element's left edge, vertically centred,
    /// and kept inside `area` (the monitor). A `focused` label swaps the background and text
    /// colours, so the highlighted hint stands out.
    pub(crate) fn label(
        &self,
        text: &str,
        typed: usize,
        focused: bool,
        anchor: Rect,
        area: Rect,
    ) -> Option<Label> {
        let px = self.style.font_px;
        let metrics = self.font.line(px);
        let upper: Vec<char> = text.chars().flat_map(char::to_uppercase).collect();
        let text_width = self.font.text_width(&upper, px);
        let pad_x = (px * 0.3).ceil();
        let pad_y = (px * 0.15).ceil();
        let line = (metrics.ascent + metrics.descent).ceil();
        let size =
            tiny_skia::Rect::from_xywh(0.0, 0.0, text_width + pad_x * 2.0, line + pad_y * 2.0)?
                .round_out()?;
        let (width, height) = (size.width(), size.height());
        let mut pixmap = Pixmap::new(width, height)?;
        let outer = size.to_rect();
        let opacity = self.style.opacity;
        let (fill, text_colour) = if focused {
            (self.style.foreground, self.style.background)
        } else {
            (self.style.background, self.style.foreground)
        };
        pixmap.fill_rect(
            outer,
            &paint(self.style.border, opacity),
            Transform::identity(),
            None,
        );
        let inner =
            tiny_skia::Rect::from_ltrb(1.0, 1.0, outer.right() - 1.0, outer.bottom() - 1.0)?;
        pixmap.fill_rect(inner, &paint(fill, opacity), Transform::identity(), None);

        let baseline = pad_y + metrics.ascent;
        self.font
            .draw(&mut pixmap, &upper, px, pad_x, baseline, |index| {
                let rgb = if index < typed {
                    self.style.typed
                } else {
                    text_colour
                };
                paint(rgb, opacity)
            });

        let center_y = i64::from(anchor.y) + i64::from(anchor.height) / 2;
        let bounds = Rect::new(
            place(i64::from(anchor.x), width, area.x, area.width),
            place(
                center_y - i64::from(height) / 2,
                height,
                area.y,
                area.height,
            ),
            width,
            height,
        );
        let pixels = pixmap
            .pixels()
            .iter()
            .flat_map(|p| [p.blue(), p.green(), p.red(), p.alpha()])
            .collect();
        Some(Label { bounds, pixels })
    }

    /// Renders a whole frame onto `areas` (monitors or outputs): `result[i]` holds the labels
    /// on `areas[i]`. Each hint goes to the area containing its centre (else the first it
    /// overlaps, else the first area); the hint labelled `frame.focused` is highlighted. With
    /// a query, a search box `/query` is drawn near the top centre of the area of the best
    /// match (the first hint), or of the first area when nothing matches.
    pub(crate) fn frame(&self, frame: &Frame<'_>, areas: &[Rect]) -> Vec<Vec<Label>> {
        let mut per_area: Vec<Vec<Label>> = vec![Vec::new(); areas.len()];
        let Some(first_area) = areas.first() else {
            return per_area;
        };
        let typed = frame.typed.chars().count();
        let mut best_area = None;
        for hint in frame.hints {
            let bounds = hint.element.bounds;
            let index = areas
                .iter()
                .position(|a| a.contains(bounds.center()))
                .or_else(|| areas.iter().position(|a| a.intersects(&bounds)))
                .unwrap_or(0);
            best_area.get_or_insert(index);
            let focused = frame.focused == Some(hint.label.as_str());
            if let Some(label) = self.label(&hint.label, typed, focused, bounds, areas[index]) {
                per_area[index].push(label);
            }
        }
        if let Some(query) = frame.query {
            let index = best_area.unwrap_or(0);
            let area = areas.get(index).unwrap_or(first_area);
            let text = format!("/{query}");
            let anchor = search_box_anchor(*area);
            if let Some(label) = self.label(&text, text.chars().count(), false, anchor, *area) {
                per_area[index].push(label);
            }
        }
        per_area
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> Style {
        Style::from_config(&OverlayConfig::default()).unwrap()
    }

    #[test]
    fn style_from_config() {
        let style = style();
        assert_eq!(style.background, [0xff, 0xd7, 0x5f]);
        assert!((style.opacity - 0.95).abs() < 1e-6);
        assert!(style.font_px > 17.0);
    }

    #[test]
    fn placement_clamps_to_area() {
        assert_eq!(place(790, 30, 0, 800), 770);
        assert_eq!(place(-5, 30, 0, 800), 0);
        assert_eq!(place(100, 30, 1920, 800), 1920);
    }

    #[test]
    fn renders_labels_in_the_configured_colours() {
        if find_font().is_none() {
            return;
        }
        let mut opaque = style();
        opaque.opacity = 1.0;
        let renderer = Renderer::new(opaque).unwrap();
        let area = Rect::new(0, 0, 800, 600);
        let label = renderer
            .label("as", 1, false, Rect::new(790, 10, 5, 20), area)
            .unwrap();
        let Rect {
            x, width, height, ..
        } = label.bounds;
        assert_eq!(
            label.pixels.len(),
            usize::try_from(width * height * 4).unwrap()
        );
        assert!(i64::from(x) + i64::from(width) <= 800);
        let has = |rgb: [u8; 3]| {
            label
                .pixels
                .chunks(4)
                .any(|p| p == [rgb[2], rgb[1], rgb[0], 255])
        };
        assert_eq!(label.pixels[..4], [0x1c, 0x1c, 0x1c, 255]);
        assert!(has(opaque.background));
        assert!(has(opaque.typed));
        assert!(has(opaque.foreground));
    }

    fn hint(label: &str, x: i32) -> nomouse_core::Hint {
        nomouse_core::Hint {
            label: label.to_owned(),
            element: nomouse_core::Element {
                id: nomouse_core::ElementId(0),
                role: nomouse_core::Role::Button,
                name: String::new(),
                bounds: Rect::new(x, 10, 40, 20),
                enabled: true,
            },
        }
    }

    /// The colour just inside the label's 1-pixel border, as RGB.
    fn fill(label: &Label) -> [u8; 3] {
        let width = usize::try_from(label.bounds.width).unwrap();
        let p = &label.pixels[(width + 1) * 4..(width + 1) * 4 + 4];
        [p[2], p[1], p[0]]
    }

    #[test]
    fn frame_puts_hints_on_their_area_and_inverts_the_focused_one() {
        if find_font().is_none() {
            return;
        }
        let mut opaque = style();
        opaque.opacity = 1.0;
        let renderer = Renderer::new(opaque).unwrap();
        let areas = [Rect::new(0, 0, 800, 600), Rect::new(800, 0, 800, 600)];
        let hints = [hint("a", 100), hint("s", 900), hint("d", 1000)];
        let frame = Frame {
            hints: &hints,
            typed: "",
            query: Some("x"),
            focused: Some("s"),
        };
        let per_area = renderer.frame(&frame, &areas);
        // The first area holds `a` and the search box (it follows the best match, `a`).
        assert_eq!(per_area[0].len(), 2);
        assert_eq!(per_area[1].len(), 2);
        assert_eq!(fill(&per_area[0][0]), opaque.background);
        assert_eq!(fill(&per_area[1][0]), opaque.foreground);
        assert_eq!(fill(&per_area[1][1]), opaque.background);
        assert_eq!(renderer.frame(&frame, &[]), Vec::<Vec<Label>>::new());
    }
}
