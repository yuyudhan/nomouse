//! Hint label rendering into one premultiplied ARGB [`Bitmap`] for the layered overlay.
//!
//! Same approach as the Linux backend: a system TrueType font rasterised by the shared
//! `nomouse_core::glyph` module (skrifa outlines filled with `tiny-skia`), all safe Rust.

use std::path::{Path, PathBuf};

use nomouse_core::config::OverlayConfig;
use nomouse_core::glyph::LabelFont;
use nomouse_core::search::search_box_anchor;
use nomouse_core::{Hint, PlatformError, Rect};
use nomouse_sys_windows::{Bitmap, Edges};
use tiny_skia::{Color, Paint, Pixmap, Transform};

/// Fonts tried first in the Windows font directory, boldest first.
const FONT_CANDIDATES: [&str; 5] = [
    "segoeuib.ttf",
    "seguisb.ttf",
    "arialbd.ttf",
    "segoeui.ttf",
    "arial.ttf",
];

/// Label colours and size, from `[overlay]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Style {
    /// Text size in pixels at 96 dpi.
    pub(crate) font_px: f32,
    /// Background RGB.
    pub(crate) background: [u8; 3],
    /// Untyped text RGB.
    pub(crate) foreground: [u8; 3],
    /// Typed prefix RGB.
    pub(crate) typed: [u8; 3],
    /// Border RGB.
    pub(crate) border: [u8; 3],
    /// Opacity, 0 to 1.
    pub(crate) opacity: f32,
}

impl Style {
    /// The style of an `[overlay]` section; the font size is in points.
    ///
    /// # Errors
    /// [`PlatformError::Backend`] for an invalid colour.
    pub(crate) fn from_config(config: &OverlayConfig) -> Result<Self, PlatformError> {
        let [background, foreground, typed, border] = config
            .colours()
            .map_err(|e| PlatformError::Backend(e.to_string()))?;
        let points = f32::from(u16::try_from(config.font_size.clamp(6, 72)).unwrap_or(13));
        let percent = f32::from(u16::try_from(config.opacity.clamp(10, 100)).unwrap_or(100));
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

/// The Windows font directory (`%WINDIR%\Fonts`).
pub(crate) fn system_font_dir() -> PathBuf {
    std::env::var_os("WINDIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join("Fonts")
}

/// The first candidate font in `dirs`, else any `.ttf` found up to three levels deep.
pub(crate) fn find_font(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .flat_map(|dir| FONT_CANDIDATES.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file())
        .or_else(|| dirs.iter().find_map(|dir| any_ttf(dir, 3)))
}

fn any_ttf(dir: &Path, depth: usize) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    entries
        .iter()
        .find(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf"))
        })
        .cloned()
        .or_else(|| {
            let deeper = depth.checked_sub(1)?;
            entries
                .iter()
                .filter(|path| path.is_dir())
                .find_map(|path| any_ttf(path, deeper))
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

/// Places a span of `size` centred on `center` inside `lo..lo + span`.
fn place(center: i64, size: u32, lo: i32, span: u32) -> i32 {
    let start = center - i64::from(size) / 2;
    let max = i64::from(lo) + i64::from(span) - i64::from(size);
    i32::try_from(start.min(max).max(i64::from(lo))).unwrap_or(lo)
}

/// One rendered label: screen bounds and premultiplied pixels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Label {
    pub(crate) bounds: Rect,
    pub(crate) pixmap: Pixmap,
}

/// Draws labels with one loaded font.
pub(crate) struct Renderer {
    font: LabelFont,
    style: Style,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("style", &self.style)
            .finish_non_exhaustive()
    }
}

impl Renderer {
    /// Loads the font at `path`.
    ///
    /// # Errors
    /// [`PlatformError::Backend`] when the file is unreadable or not a font.
    pub(crate) fn from_file(path: &Path, style: Style) -> Result<Self, PlatformError> {
        let font = LabelFont::from_file(path).map_err(|e| PlatformError::Backend(e.to_string()))?;
        Ok(Self { font, style })
    }

    /// Loads the first usable font from the Windows font directory.
    ///
    /// # Errors
    /// [`PlatformError::Backend`] when no TrueType font is installed.
    pub(crate) fn system(style: Style) -> Result<Self, PlatformError> {
        let dir = system_font_dir();
        let path = find_font(std::slice::from_ref(&dir)).ok_or_else(|| {
            PlatformError::Backend(format!("no TrueType font found in {}", dir.display()))
        })?;
        Self::from_file(&path, style)
    }

    /// Renders `text` (uppercase) at `scale` (monitor DPI / 96) centred on `anchor`'s centre and
    /// kept inside `area`; the first `typed` characters use the typed colour.
    pub(crate) fn label(
        &self,
        text: &str,
        typed: usize,
        anchor: Rect,
        area: Edges,
        scale: f32,
    ) -> Option<Label> {
        let px = self.style.font_px * scale.max(0.5);
        let metrics = self.font.line(px);
        let upper: Vec<char> = text.chars().flat_map(char::to_uppercase).collect();
        let text_width = self.font.text_width(&upper, px);
        let pad_x = (px * 0.3).ceil();
        let pad_y = (px * 0.15).ceil();
        let border = scale.round().max(1.0);
        let line = (metrics.ascent + metrics.descent).ceil();
        let size =
            tiny_skia::Rect::from_xywh(0.0, 0.0, text_width + pad_x * 2.0, line + pad_y * 2.0)?
                .round_out()?;
        let (width, height) = (size.width(), size.height());
        let mut pixmap = Pixmap::new(width, height)?;
        let outer = size.to_rect();
        let opacity = self.style.opacity;
        pixmap.fill_rect(
            outer,
            &paint(self.style.border, opacity),
            Transform::identity(),
            None,
        );
        let inner = tiny_skia::Rect::from_ltrb(
            border,
            border,
            outer.right() - border,
            outer.bottom() - border,
        )?;
        pixmap.fill_rect(
            inner,
            &paint(self.style.background, opacity),
            Transform::identity(),
            None,
        );

        let baseline = pad_y + metrics.ascent;
        self.font
            .draw(&mut pixmap, &upper, px, pad_x, baseline, |index| {
                let rgb = if index < typed {
                    self.style.typed
                } else {
                    self.style.foreground
                };
                paint(rgb, opacity)
            });
        let center = anchor.center();
        let bounds = Rect::new(
            place(i64::from(center.x), width, area.left, area.width()),
            place(i64::from(center.y), height, area.top, area.height()),
            width,
            height,
        );
        Some(Label { bounds, pixmap })
    }

    /// Renders every hint into one bitmap covering their union; `scale_at` gives the DPI scale
    /// at a point. With a `query` (search-by-text) a search box `/query` is drawn near the top
    /// centre of `area`. Returns an empty bitmap when there is nothing to draw.
    pub(crate) fn frame(
        &self,
        hints: &[Hint],
        typed: &str,
        query: Option<&str>,
        area: Edges,
        mut scale_at: impl FnMut(i32, i32) -> f32,
    ) -> Bitmap {
        let typed_len = typed.chars().count();
        let mut labels: Vec<Label> = hints
            .iter()
            .filter_map(|hint| {
                let at = hint.element.target();
                let typed = if hint.label.starts_with(typed) {
                    typed_len
                } else {
                    0
                };
                self.label(
                    &hint.label,
                    typed,
                    hint.element.bounds,
                    area,
                    scale_at(at.x, at.y),
                )
            })
            .collect();
        if let Some(query) = query {
            let anchor =
                search_box_anchor(Rect::new(area.left, area.top, area.width(), area.height()));
            let at = anchor.center();
            let text = format!("/{query}");
            labels.extend(self.label(
                &text,
                text.chars().count(),
                anchor,
                area,
                scale_at(at.x, at.y),
            ));
        }
        compose(&labels)
    }
}

/// Blends `labels` (later ones on top) into one bitmap covering their union.
pub(crate) fn compose(labels: &[Label]) -> Bitmap {
    let Some(first) = labels.first() else {
        return Bitmap::default();
    };
    let edges = |r: &Rect| {
        (
            i64::from(r.x),
            i64::from(r.y),
            i64::from(r.x) + i64::from(r.width),
            i64::from(r.y) + i64::from(r.height),
        )
    };
    let (mut left, mut top, mut right, mut bottom) = edges(&first.bounds);
    for label in &labels[1..] {
        let (l, t, r, b) = edges(&label.bounds);
        left = left.min(l);
        top = top.min(t);
        right = right.max(r);
        bottom = bottom.max(b);
    }
    let width = usize::try_from(right - left).unwrap_or(0);
    let height = usize::try_from(bottom - top).unwrap_or(0);
    let mut pixels = vec![0u32; width * height];
    for label in labels {
        let (l, t, _, _) = edges(&label.bounds);
        let (dx, dy) = (
            usize::try_from(l - left).unwrap_or(0),
            usize::try_from(t - top).unwrap_or(0),
        );
        let label_width = usize::try_from(label.pixmap.width()).unwrap_or(0);
        for (row, line) in label.pixmap.pixels().chunks(label_width.max(1)).enumerate() {
            let start = (dy + row) * width + dx;
            for (target, source) in pixels[start..start + line.len()].iter_mut().zip(line) {
                *target = over(
                    [source.alpha(), source.red(), source.green(), source.blue()],
                    *target,
                );
            }
        }
    }
    Bitmap {
        x: i32::try_from(left).unwrap_or(0),
        y: i32::try_from(top).unwrap_or(0),
        width: u32::try_from(width).unwrap_or(0),
        height: u32::try_from(height).unwrap_or(0),
        pixels,
    }
}

/// Premultiplied source-over of `[a, r, g, b]` onto `0xAARRGGBB`.
fn over(source: [u8; 4], destination: u32) -> u32 {
    let inverse = 255 - u32::from(source[0]);
    let [da, dr, dg, db] = destination.to_be_bytes();
    let mix = |s: u8, d: u8| (u32::from(s) + (u32::from(d) * inverse + 127) / 255).min(255);
    (mix(source[0], da) << 24)
        | (mix(source[1], dr) << 16)
        | (mix(source[2], dg) << 8)
        | mix(source[3], db)
}

#[cfg(test)]
mod tests {
    use nomouse_core::{Element, ElementId, Role};

    use super::*;

    fn style() -> Style {
        Style::from_config(&OverlayConfig::default()).unwrap()
    }

    fn test_font() -> Option<PathBuf> {
        find_font(&[
            system_font_dir(),
            PathBuf::from("/usr/share/fonts"),
            PathBuf::from("/System/Library/Fonts"),
            PathBuf::from("/Library/Fonts"),
        ])
    }

    fn hint(label: &str, bounds: Rect) -> Hint {
        Hint {
            label: label.to_owned(),
            element: Element {
                id: ElementId(0),
                role: Role::Button,
                name: String::new(),
                description: String::new(),
                value: String::new(),
                bounds,
                enabled: true,
            },
        }
    }

    #[test]
    fn style_from_config() {
        let style = style();
        assert_eq!(style.background, [0xff, 0xd7, 0x5f]);
        assert!((style.opacity - 0.95).abs() < 1e-6);
        assert!(style.font_px > 17.0);
    }

    #[test]
    fn placement_centres_and_clamps() {
        assert_eq!(place(100, 30, 0, 800), 85);
        assert_eq!(place(790, 30, 0, 800), 770);
        assert_eq!(place(-5, 30, -1920, 800), -1150);
        assert_eq!(place(-5000, 30, -1920, 800), -1920);
    }

    #[test]
    fn over_blends_premultiplied() {
        assert_eq!(over([255, 1, 2, 3], 0xFF00_0000), 0xFF01_0203);
        assert_eq!(over([0, 0, 0, 0], 0x8040_2010), 0x8040_2010);
        assert_eq!(over([128, 128, 0, 0], 0), 0x8080_0000);
    }

    #[test]
    fn compose_covers_the_union() {
        let mut a = Pixmap::new(2, 1).unwrap();
        a.fill(Color::from_rgba8(255, 0, 0, 255));
        let mut b = Pixmap::new(1, 2).unwrap();
        b.fill(Color::from_rgba8(0, 0, 255, 255));
        let bitmap = compose(&[
            Label {
                bounds: Rect::new(-2, 5, 2, 1),
                pixmap: a,
            },
            Label {
                bounds: Rect::new(-1, 5, 1, 2),
                pixmap: b,
            },
        ]);
        assert_eq!(
            (bitmap.x, bitmap.y, bitmap.width, bitmap.height),
            (-2, 5, 2, 2)
        );
        assert_eq!(
            bitmap.pixels,
            vec![0xFFFF_0000, 0xFF00_00FF, 0, 0xFF00_00FF]
        );
        assert_eq!(compose(&[]), Bitmap::default());
    }

    #[test]
    fn renders_hints_with_dpi_scaling() {
        let Some(path) = test_font() else {
            return;
        };
        let mut opaque = style();
        opaque.opacity = 1.0;
        let renderer = Renderer::from_file(&path, opaque).unwrap();
        let area = Edges::new(0, 0, 800, 600);
        let small = renderer
            .label("as", 1, Rect::new(100, 100, 40, 20), area, 1.0)
            .unwrap();
        let large = renderer
            .label("as", 1, Rect::new(100, 100, 40, 20), area, 2.0)
            .unwrap();
        assert!(large.bounds.height >= small.bounds.height * 2 - 2);
        let frame = renderer.frame(
            &[
                hint("as", Rect::new(790, 10, 5, 20)),
                hint("d", Rect::new(0, 590, 5, 5)),
            ],
            "a",
            None,
            area,
            |_, _| 1.0,
        );
        assert_eq!(
            frame.pixels.len(),
            usize::try_from(frame.width * frame.height).unwrap()
        );
        assert!(frame.x >= 0 && i64::from(frame.x) + i64::from(frame.width) <= 800);
        // Glyph strokes are anti-aliased, so text colours are matched with a tolerance.
        let near = |pixel: u32, rgb: [u8; 3]| {
            let [a, r, g, b] = pixel.to_be_bytes();
            a == 255 && [r, g, b].iter().zip(rgb).all(|(&c, t)| c.abs_diff(t) <= 40)
        };
        for colour in [
            opaque.background,
            opaque.typed,
            opaque.foreground,
            opaque.border,
        ] {
            assert!(frame.pixels.iter().any(|&p| near(p, colour)), "{colour:?}");
        }
    }

    #[test]
    fn search_frame_adds_the_query_box_near_the_top() {
        let Some(path) = test_font() else {
            return;
        };
        let renderer = Renderer::from_file(&path, style()).unwrap();
        let area = Edges::new(0, 0, 800, 600);
        let empty = renderer.frame(&[], "", None, area, |_, _| 1.0);
        assert_eq!(empty, Bitmap::default());
        let search = renderer.frame(&[], "", Some("save"), area, |_, _| 1.0);
        assert!(search.width > 0 && search.height > 0);
        assert!(search.y < 150 && search.x > 200 && search.x < 400);
    }
}
