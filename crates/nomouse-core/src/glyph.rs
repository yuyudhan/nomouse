//! Text rasterisation for hint labels, shared by the overlay renderers (feature `glyph`).
//!
//! Glyph outlines come from an OpenType/TrueType font read with `skrifa` (Google's fontations,
//! pure safe Rust) and are filled as anti-aliased `tiny-skia` paths. Sizes are line heights:
//! a size of `px` makes ascent plus descent span `px` pixels, so labels keep the same height
//! whatever the font's units per em.

use std::path::Path;

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

/// Why a font could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    /// The file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// The font file.
        path: String,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The bytes are not a usable font.
    #[error("{0}")]
    Invalid(String),
}

/// Vertical metrics of one line of text, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineMetrics {
    /// Distance from the top of the line to the baseline.
    pub ascent: f32,
    /// Distance from the baseline to the bottom of the line (positive).
    pub descent: f32,
}

/// A loaded label font (the first face of a font file or collection).
#[derive(Clone)]
pub struct LabelFont {
    data: Vec<u8>,
    /// Font units of ascent + descent: the unscaled line height.
    units_height: f32,
    units_per_em: f32,
    units_ascent: f32,
    units_descent: f32,
}

impl std::fmt::Debug for LabelFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LabelFont")
            .field("bytes", &self.data.len())
            .field("units_per_em", &self.units_per_em)
            .finish_non_exhaustive()
    }
}

/// Feeds skrifa outline commands into a tiny-skia path, flipping y (fonts are y-up) and
/// placing the origin at `(x, baseline)`.
struct SkiaPen {
    builder: PathBuilder,
    x: f32,
    baseline: f32,
}

impl SkiaPen {
    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x, self.baseline - y)
    }
}

impl OutlinePen for SkiaPen {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.builder.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.builder.line_to(x, y);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (cx0, cy0) = self.point(cx0, cy0);
        let (x, y) = self.point(x, y);
        self.builder.quad_to(cx0, cy0, x, y);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (cx0, cy0) = self.point(cx0, cy0);
        let (cx1, cy1) = self.point(cx1, cy1);
        let (x, y) = self.point(x, y);
        self.builder.cubic_to(cx0, cy0, cx1, cy1, x, y);
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

impl LabelFont {
    /// Parses `data` as a font file or the first face of a collection.
    ///
    /// # Errors
    /// [`FontError::Invalid`] when the data is not a font with usable metrics.
    pub fn from_bytes(data: Vec<u8>) -> Result<Self, FontError> {
        let font = FontRef::from_index(&data, 0).map_err(|e| FontError::Invalid(e.to_string()))?;
        let metrics = font.metrics(Size::unscaled(), LocationRef::default());
        let units_per_em = f32::from(metrics.units_per_em);
        let units_ascent = metrics.ascent;
        let units_descent = -metrics.descent;
        let units_height = units_ascent + units_descent;
        if !(units_height > 0.0 && units_per_em > 0.0) {
            return Err(FontError::Invalid("font has no line metrics".to_owned()));
        }
        Ok(Self {
            data,
            units_height,
            units_per_em,
            units_ascent,
            units_descent,
        })
    }

    /// Reads and parses the font at `path`.
    ///
    /// # Errors
    /// [`FontError::Io`] when unreadable, else as [`LabelFont::from_bytes`].
    pub fn from_file(path: &Path) -> Result<Self, FontError> {
        let data = std::fs::read(path).map_err(|source| FontError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_bytes(data).map_err(|error| match error {
            FontError::Invalid(message) => {
                FontError::Invalid(format!("{}: {message}", path.display()))
            }
            io @ FontError::Io { .. } => io,
        })
    }

    fn font(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(&self.data, 0).ok()
    }

    /// Pixels per em for a line height of `px`.
    fn ppem(&self, px: f32) -> f32 {
        px * self.units_per_em / self.units_height
    }

    /// Line metrics at line height `px`.
    pub fn line(&self, px: f32) -> LineMetrics {
        let scale = px / self.units_height;
        LineMetrics {
            ascent: self.units_ascent * scale,
            descent: self.units_descent * scale,
        }
    }

    fn glyphs(font: &FontRef<'_>, text: &[char]) -> Vec<GlyphId> {
        let charmap = font.charmap();
        text.iter()
            .map(|&c| charmap.map(c).unwrap_or(GlyphId::NOTDEF))
            .collect()
    }

    /// The advance width of `text` at line height `px`.
    pub fn text_width(&self, text: &[char], px: f32) -> f32 {
        let Some(font) = self.font() else {
            return 0.0;
        };
        let metrics = font.glyph_metrics(Size::new(self.ppem(px)), LocationRef::default());
        Self::glyphs(&font, text)
            .into_iter()
            .map(|id| metrics.advance_width(id).unwrap_or(0.0))
            .sum()
    }

    /// Fills `text` into `pixmap` at line height `px`, starting at `x` on `baseline`, with
    /// `paint(index)` for the character at `index`; returns the pen position after the text.
    pub fn draw(
        &self,
        pixmap: &mut Pixmap,
        text: &[char],
        px: f32,
        x: f32,
        baseline: f32,
        mut paint: impl FnMut(usize) -> Paint<'static>,
    ) -> f32 {
        let Some(font) = self.font() else {
            return x;
        };
        let size = Size::new(self.ppem(px));
        let metrics = font.glyph_metrics(size, LocationRef::default());
        let outlines = font.outline_glyphs();
        let mut pen_x = x;
        for (index, id) in Self::glyphs(&font, text).into_iter().enumerate() {
            if let Some(glyph) = outlines.get(id) {
                let mut pen = SkiaPen {
                    builder: PathBuilder::new(),
                    x: pen_x,
                    baseline,
                };
                let settings = DrawSettings::unhinted(size, LocationRef::default());
                if glyph.draw(settings, &mut pen).is_ok()
                    && let Some(path) = pen.builder.finish()
                {
                    pixmap.fill_path(
                        &path,
                        &paint(index),
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
            }
            pen_x += metrics.advance_width(id).unwrap_or(0.0);
        }
        pen_x
    }
}

/// System font directories searched for a label font on this OS.
pub fn system_font_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(windows) {
        let windir = std::env::var_os("WINDIR")
            .filter(|dir| !dir.is_empty())
            .map_or_else(|| "C:\\Windows".into(), std::path::PathBuf::from);
        dirs.push(windir.join("Fonts"));
    } else if cfg!(target_os = "macos") {
        dirs.push("/System/Library/Fonts".into());
        dirs.push("/Library/Fonts".into());
    } else {
        dirs.push("/usr/share/fonts".into());
        dirs.push("/usr/local/share/fonts".into());
    }
    dirs
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tiny_skia::Color;

    use super::*;

    /// Any TrueType font on this machine, for tests (skipped when none is installed).
    fn any_font() -> Option<PathBuf> {
        fn find(dir: &Path, depth: usize) -> Option<PathBuf> {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .collect();
            entries.sort();
            entries
                .iter()
                .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ttf")))
                .cloned()
                .or_else(|| {
                    let deeper = depth.checked_sub(1)?;
                    entries
                        .iter()
                        .filter(|p| p.is_dir())
                        .find_map(|p| find(p, deeper))
                })
        }
        system_font_dirs().iter().find_map(|dir| find(dir, 4))
    }

    #[test]
    fn rejects_non_fonts() {
        assert!(matches!(
            LabelFont::from_bytes(b"not a font".to_vec()),
            Err(FontError::Invalid(_))
        ));
        assert!(matches!(
            LabelFont::from_file(Path::new("/nonexistent/font.ttf")),
            Err(FontError::Io { .. })
        ));
    }

    #[test]
    fn metrics_scale_with_line_height() {
        let Some(path) = any_font() else { return };
        let font = LabelFont::from_file(&path).unwrap();
        let line = font.line(20.0);
        assert!((line.ascent + line.descent - 20.0).abs() < 1e-3);
        let text: Vec<char> = "AS".chars().collect();
        let narrow = font.text_width(&text, 10.0);
        let wide = font.text_width(&text, 20.0);
        assert!(narrow > 0.0);
        assert!((wide - 2.0 * narrow).abs() < 0.01);
    }

    #[test]
    fn draws_each_character_in_its_paint() {
        let Some(path) = any_font() else { return };
        let font = LabelFont::from_file(&path).unwrap();
        let text: Vec<char> = "HH".chars().collect();
        let px = 40.0;
        let width = font.text_width(&text, px).ceil();
        let mut pixmap = Pixmap::new(200, 50).unwrap();
        assert!(width + 4.0 < 200.0);
        let line = font.line(px);
        let end = font.draw(&mut pixmap, &text, px, 2.0, 2.0 + line.ascent, |index| {
            let mut paint = Paint::default();
            paint.set_color(if index == 0 {
                Color::from_rgba8(255, 0, 0, 255)
            } else {
                Color::from_rgba8(0, 0, 255, 255)
            });
            paint
        });
        assert!((end - (2.0 + width)).abs() <= 1.0);
        let pixels = pixmap.pixels();
        let solid = |r: u8, b: u8| {
            pixels
                .iter()
                .any(|p| p.alpha() == 255 && p.red() == r && p.blue() == b && p.green() == 0)
        };
        assert!(solid(255, 0), "first glyph red");
        assert!(solid(0, 255), "second glyph blue");
    }
}
