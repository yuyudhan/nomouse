//! Text rasterisation for hint labels, shared by the overlay renderers (feature `glyph`).
//!
//! Glyph outlines come from an OpenType/TrueType font read with `skrifa` (Google's fontations,
//! pure safe Rust) and are filled as anti-aliased `tiny-skia` paths. Sizes are line heights:
//! a size of `px` makes ascent plus descent span `px` pixels, so labels keep the same height
//! whatever the font's units per em.

use std::path::{Path, PathBuf};

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
pub fn system_font_dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        let windir = std::env::var_os("WINDIR")
            .filter(|dir| !dir.is_empty())
            .map_or_else(|| "C:\\Windows".into(), PathBuf::from);
        vec![windir.join("Fonts")]
    } else if cfg!(target_os = "macos") {
        vec!["/System/Library/Fonts".into(), "/Library/Fonts".into()]
    } else {
        freedesktop_font_dirs(|name| std::env::var(name).ok())
    }
}

/// The font directories of a freedesktop (Linux, BSD) system for the environment `env`, in
/// search order: `fonts` under every `$XDG_DATA_DIRS` entry (default `/usr/local/share` and
/// `/usr/share`), then `/usr/share/fonts` and `/usr/local/share/fonts` when that list left them
/// out, then the user's `$XDG_DATA_HOME/fonts` (default `~/.local/share/fonts`) and `~/.fonts`.
/// Relative entries are ignored, as the XDG base directory spec says; duplicates appear once.
pub fn freedesktop_font_dirs(env: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let get = |name: &str| env(name).filter(|value| !value.is_empty());
    let home = get("HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute());
    let data_dirs =
        get("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    let data_home = get("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.as_ref().map(|home| home.join(".local").join("share")));
    let mut dirs: Vec<PathBuf> = Vec::new();
    let candidates = data_dirs
        .split(':')
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("fonts"))
        .chain(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from))
        .chain(data_home.map(|dir| dir.join("fonts")))
        .chain(home.map(|home| home.join(".fonts")));
    for dir in candidates {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Whether `path` names a font file the label renderer reads (`.ttf` or `.otf`).
pub fn is_font_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ttf") || ext.eq_ignore_ascii_case("otf"))
}

/// The first font file under `dirs`, searched in order and up to four levels deep: in each
/// directory (entries sorted by name) a file whose name contains `Bold`, else any font file,
/// else the subdirectories.
pub fn find_font_file(dirs: &[PathBuf]) -> Option<PathBuf> {
    fn find_in(dir: &Path, depth: usize) -> Option<PathBuf> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .collect();
        entries.sort();
        let is_bold = |path: &&PathBuf| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().contains("Bold"))
        };
        let fonts = || entries.iter().filter(|path| is_font_file(path));
        fonts()
            .find(is_bold)
            .or_else(|| fonts().next())
            .cloned()
            .or_else(|| {
                let deeper = depth.checked_sub(1)?;
                entries
                    .iter()
                    .filter(|path| path.is_dir())
                    .find_map(|path| find_in(path, deeper))
            })
    }
    dirs.iter().find_map(|dir| find_in(dir, 4))
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
        // Fonts flagged "integer ppem" truncate the scale slightly (~0.2 % on macOS CI).
        assert!((wide / narrow - 2.0).abs() < 0.02, "{narrow} {wide}");
        // Two capitals at a 20 px line height span roughly one line height; a wrong scale
        // (ppem = px, or unscaled font units) falls far outside this range.
        assert!(
            (12.0..36.0).contains(&wide),
            "AS at 20 px is {wide} px wide"
        );
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

    /// An environment lookup over `pairs`.
    #[cfg(unix)]
    fn env(pairs: &[(&str, String)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, value)| value.clone())
        }
    }

    // `:`-separated absolute paths: freedesktop systems only.
    #[cfg(unix)]
    #[test]
    fn freedesktop_dirs_follow_xdg_data_dirs_then_the_user_dirs() {
        let root = std::env::temp_dir().join(format!("nomouse-fontdirs-{}", std::process::id()));
        let (nix, share, home) = (root.join("nix"), root.join("share"), root.join("home"));
        let data_dirs = format!("{}:relative/dir:{}", nix.display(), share.display());
        let dirs = freedesktop_font_dirs(env(&[
            ("XDG_DATA_DIRS", data_dirs),
            ("HOME", home.display().to_string()),
        ]));
        assert_eq!(
            dirs,
            vec![
                nix.join("fonts"),
                share.join("fonts"),
                PathBuf::from("/usr/share/fonts"),
                PathBuf::from("/usr/local/share/fonts"),
                home.join(".local/share/fonts"),
                home.join(".fonts"),
            ]
        );
        // Defaults: the spec's XDG_DATA_DIRS, no duplicates; XDG_DATA_HOME replaces ~/.local/share.
        let data_home = root.join("data");
        assert_eq!(
            freedesktop_font_dirs(env(&[
                ("HOME", home.display().to_string()),
                ("XDG_DATA_HOME", data_home.display().to_string()),
            ])),
            vec![
                PathBuf::from("/usr/local/share/fonts"),
                PathBuf::from("/usr/share/fonts"),
                data_home.join("fonts"),
                home.join(".fonts"),
            ]
        );
        assert_eq!(
            freedesktop_font_dirs(env(&[])),
            vec![
                PathBuf::from("/usr/local/share/fonts"),
                PathBuf::from("/usr/share/fonts"),
            ]
        );
    }

    #[test]
    fn find_font_file_takes_the_first_dir_and_prefers_bold() {
        let root = std::env::temp_dir().join(format!("nomouse-findfont-{}", std::process::id()));
        let (empty, user, system) = (root.join("empty"), root.join("user"), root.join("system"));
        let nested = user.join("dejavu");
        for dir in [&empty, &nested, &system] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(empty.join("readme.txt"), "").unwrap();
        std::fs::write(nested.join("DejaVuSans.ttf"), "").unwrap();
        std::fs::write(nested.join("DejaVuSans-Bold.ttf"), "").unwrap();
        std::fs::write(system.join("Other.otf"), "").unwrap();
        let found = find_font_file(&[root.join("missing"), empty.clone(), user, system.clone()]);
        assert_eq!(found, Some(nested.join("DejaVuSans-Bold.ttf")));
        assert_eq!(
            find_font_file(&[empty, system.clone()]),
            Some(system.join("Other.otf"))
        );
        assert_eq!(find_font_file(&[root.join("missing")]), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
