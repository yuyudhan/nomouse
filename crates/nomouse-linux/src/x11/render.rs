//! Label rendering: glyph outlines from the shared `nomouse_core::glyph` rasteriser (skrifa)
//! filled as tiny-skia paths, converted to X's premultiplied BGRA byte order.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use nomouse_core::config::{Appearance, OverlayConfig};
use nomouse_core::glyph::{LabelFont, find_font_file, is_font_file, system_font_dirs};
use nomouse_core::placement::label_bounds;
use nomouse_core::search::search_box_anchor;
use nomouse_core::{Frame, PlatformError, Rect};
use tiny_skia::{Color, Paint, Pixmap, Transform};

/// Bold sans fonts tried first; then fontconfig's choice; then any font under the font
/// directories ([`system_font_dirs`]).
const FONT_CANDIDATES: [&str; 6] = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    "/usr/share/fonts/liberation-sans/LiberationSans-Bold.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
];

/// Thickness in pixels of the outline drawn around the active scroll area.
const OUTLINE: u32 = 3;

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
    /// The style of a validated `[overlay]` section under the system `appearance` (which only
    /// `theme = "auto"` looks at); sizes are points at 96 dpi.
    ///
    /// # Errors
    /// [`PlatformError::Backend`] for an invalid colour.
    pub(crate) fn from_config(
        config: &OverlayConfig,
        appearance: Appearance,
    ) -> Result<Self, PlatformError> {
        let [background, foreground, typed, border] = config
            .colours(appearance)
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

/// The font fontconfig picks for bold sans (`fc-match -f '%{file}' sans:bold`), when
/// fontconfig is installed and names a TrueType/OpenType file that exists. Fontconfig knows
/// fonts in places no fixed list covers (NixOS profiles, `fonts.conf` additions).
fn fc_match() -> Option<PathBuf> {
    let output = Command::new("fc-match")
        .args(["-f", "%{file}", "sans:bold"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    (is_font_file(&path) && path.is_file()).then_some(path)
}

/// The label font: the first of `candidates` that exists, else `fontconfig`'s answer, else
/// the first font file under `dirs`.
fn find_font_with(
    candidates: &[PathBuf],
    fontconfig: impl FnOnce() -> Option<PathBuf>,
    dirs: &[PathBuf],
) -> Option<PathBuf> {
    candidates
        .iter()
        .find(|path| path.is_file())
        .cloned()
        .or_else(fontconfig)
        .or_else(|| find_font_file(dirs))
}

/// Locates a usable font file: [`FONT_CANDIDATES`], then `fc-match`, then
/// [`system_font_dirs`].
pub(crate) fn find_font() -> Option<PathBuf> {
    let candidates = FONT_CANDIDATES.map(PathBuf::from);
    find_font_with(&candidates, fc_match, &system_font_dirs())
}

/// How to install `DejaVu Sans` on the distribution an `/etc/os-release` text describes (its
/// `ID`, then each `ID_LIKE` entry).
pub(crate) fn font_install_hint(os_release: &str) -> &'static str {
    let field = |key: &str| {
        os_release.lines().find_map(|line| {
            let value = line.trim().strip_prefix(key)?.strip_prefix('=')?;
            Some(value.trim().trim_matches(['"', '\'']).to_ascii_lowercase())
        })
    };
    let id = field("ID").unwrap_or_default();
    let like = field("ID_LIKE").unwrap_or_default();
    std::iter::once(id.as_str())
        .chain(like.split_whitespace())
        .find_map(|distro| match distro {
            "debian" | "ubuntu" => {
                Some("install DejaVu Sans: `sudo apt install fonts-dejavu-core`")
            }
            "fedora" | "rhel" | "centos" => {
                Some("install DejaVu Sans: `sudo dnf install dejavu-sans-fonts`")
            }
            "arch" => Some("install DejaVu Sans: `sudo pacman -S ttf-dejavu`"),
            "opensuse" | "suse" => Some("install DejaVu Sans: `sudo zypper install dejavu-fonts`"),
            "alpine" => Some("install DejaVu Sans: `sudo apk add font-dejavu`"),
            "void" => Some("install DejaVu Sans: `sudo xbps-install dejavu-fonts-ttf`"),
            "gentoo" => Some("install DejaVu Sans: `sudo emerge media-fonts/dejavu`"),
            "nixos" => Some(
                "install DejaVu Sans: add `pkgs.dejavu_fonts` to `fonts.packages` in \
                 configuration.nix",
            ),
            _ => None,
        })
        .unwrap_or("install the DejaVu Sans font with your package manager")
}

/// What to say when [`find_font`] finds nothing: where it looked and how to install a font
/// on this distribution.
pub(crate) fn no_font_message() -> String {
    let os_release = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    let dirs: Vec<String> = system_font_dirs()
        .iter()
        .map(|dir| dir.display().to_string())
        .collect();
    format!(
        "no TrueType/OpenType font found (fontconfig's `fc-match sans:bold`, then {}); {}",
        dirs.join(", "),
        font_install_hint(&os_release)
    )
}

fn paint(rgb: [u8; 3], opacity: f32) -> Paint<'static> {
    let mut colour = Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255);
    colour.set_alpha(opacity.clamp(0.0, 1.0));
    let mut paint = Paint::default();
    paint.set_color(colour);
    paint.anti_alias = true;
    paint
}

impl Renderer {
    /// Loads the font and keeps `style`.
    ///
    /// # Errors
    /// [`PlatformError::Backend`] when no TrueType font is installed.
    pub(crate) fn new(style: Style) -> Result<Self, PlatformError> {
        let path = find_font().ok_or_else(|| PlatformError::Backend(no_font_message()))?;
        let font =
            LabelFont::from_file(&path).map_err(|e| PlatformError::Backend(e.to_string()))?;
        tracing::debug!("label font {}", path.display());
        Ok(Self { font, style })
    }

    /// Draws with `style` from now on (the system appearance changed under `theme = "auto"`).
    pub(crate) const fn set_style(&mut self, style: Style) {
        self.style = style;
    }

    /// Renders `text` (shown uppercase) for an element at `anchor`, with the first `typed`
    /// characters in the typed colour, centred on the element and kept inside `area` (the
    /// monitor; [`label_bounds`]). A `focused` label swaps the background and text colours,
    /// so the highlighted hint stands out.
    pub(crate) fn label(
        &self,
        text: &str,
        typed: usize,
        focused: bool,
        anchor: Rect,
        area: Rect,
    ) -> Option<Label> {
        let (width, height, pixels) = self.draw(text, typed, focused)?;
        let bounds = label_bounds(anchor, width, height, area);
        Some(Label { bounds, pixels })
    }

    /// Renders the search box `text` (all of it in the typed colour) starting at the 1-pixel
    /// `marker` from [`search_box_anchor`], vertically centred on it and kept inside `area`:
    /// the box grows to the right as the query grows instead of staying centred.
    pub(crate) fn search_box(&self, text: &str, marker: Rect, area: Rect) -> Option<Label> {
        let (width, height, pixels) = self.draw(text, text.chars().count(), false)?;
        // Centring on a span as wide as the box that starts at the marker puts the box's left
        // edge on the marker.
        let start = Rect::new(marker.x, marker.y, width, marker.height);
        let bounds = label_bounds(start, width, height, area);
        Some(Label { bounds, pixels })
    }

    /// Draws `text` as a label: `(width, height, premultiplied BGRA pixels)`.
    fn draw(&self, text: &str, typed: usize, focused: bool) -> Option<(u32, u32, Vec<u8>)> {
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

        let pixels = pixmap
            .pixels()
            .iter()
            .flat_map(|p| [p.blue(), p.green(), p.red(), p.alpha()])
            .collect();
        Some((width, height, pixels))
    }

    /// Solid strips in the label background colour framing `rect`, clipped to `area` (one
    /// monitor): the outline of the active scroll area.
    pub(crate) fn outline(&self, rect: Rect, area: Rect) -> Vec<Label> {
        let rgb = self.style.background;
        let pixel = [rgb[2], rgb[1], rgb[0], u8::MAX];
        let width = rect.width.clamp(1, OUTLINE);
        let height = rect.height.clamp(1, OUTLINE);
        let right = i64::from(rect.x) + i64::from(rect.width) - i64::from(width);
        let bottom = i64::from(rect.y) + i64::from(rect.height) - i64::from(height);
        let (right, bottom) = (
            i32::try_from(right).unwrap_or(rect.x),
            i32::try_from(bottom).unwrap_or(rect.y),
        );
        [
            Rect::new(rect.x, rect.y, rect.width, height),
            Rect::new(rect.x, bottom, rect.width, height),
            Rect::new(rect.x, rect.y, width, rect.height),
            Rect::new(right, rect.y, width, rect.height),
        ]
        .iter()
        .filter_map(|strip| strip.intersection(&area))
        .map(|bounds| {
            let count =
                usize::try_from(u64::from(bounds.width) * u64::from(bounds.height)).unwrap_or(0);
            Label {
                bounds,
                pixels: pixel.repeat(count),
            }
        })
        .collect()
    }

    /// Renders a whole frame onto `areas` (monitors or outputs): `result[i]` holds the labels
    /// on `areas[i]`. Each hint goes to the area containing its centre (else the first it
    /// overlaps, else the first area); the hint labelled `frame.focused` is highlighted. With
    /// a query, a search box `/query` is drawn near the top centre of the area of the best
    /// match (the first hint), or of the first area when nothing matches. `frame.area` is
    /// outlined on every area it overlaps, below the labels.
    pub(crate) fn frame(&self, frame: &Frame<'_>, areas: &[Rect]) -> Vec<Vec<Label>> {
        let mut per_area: Vec<Vec<Label>> = vec![Vec::new(); areas.len()];
        let Some(first_area) = areas.first() else {
            return per_area;
        };
        if let Some(outlined) = frame.area {
            for (labels, area) in per_area.iter_mut().zip(areas) {
                labels.extend(self.outline(outlined, *area));
            }
        }
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
            let marker = search_box_anchor(*area);
            if let Some(label) = self.search_box(&text, marker, *area) {
                per_area[index].push(label);
            }
        }
        per_area
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_search_order_is_candidates_fontconfig_then_xdg_and_user_dirs() {
        let root = std::env::temp_dir().join(format!("nomouse-render-font-{}", std::process::id()));
        let (home, system) = (root.join("home"), root.join("system"));
        let user_fonts = home.join(".local/share/fonts");
        std::fs::create_dir_all(&user_fonts).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        let user_font = user_fonts.join("Inter-Bold.otf");
        std::fs::write(&user_font, "").unwrap();
        // The machine's own /usr/share/fonts stays out: only the temporary tree counts.
        let dirs: Vec<PathBuf> = nomouse_core::glyph::freedesktop_font_dirs(|name| match name {
            "HOME" => Some(home.display().to_string()),
            "XDG_DATA_DIRS" => Some(system.display().to_string()),
            _ => None,
        })
        .into_iter()
        .filter(|dir| dir.starts_with(&root))
        .collect();
        assert_eq!(
            dirs,
            [
                system.join("fonts"),
                user_fonts.clone(),
                home.join(".fonts")
            ]
        );
        let missing = [root.join("missing/DejaVuSans-Bold.ttf")];
        // Only in ~/.local/share/fonts, no fontconfig: still found.
        assert_eq!(
            find_font_with(&missing, || None, &dirs),
            Some(user_font.clone())
        );
        // A font in an XDG_DATA_DIRS entry comes before the user's.
        std::fs::create_dir_all(system.join("fonts")).unwrap();
        let system_font = system.join("fonts").join("Sans.ttf");
        std::fs::write(&system_font, "").unwrap();
        assert_eq!(
            find_font_with(&missing, || None, &dirs),
            Some(system_font.clone())
        );
        // fontconfig's answer comes before the directory walk; a fixed candidate before both.
        let chosen = root.join("fc.ttf");
        assert_eq!(
            find_font_with(&missing, || Some(chosen.clone()), &dirs),
            Some(chosen)
        );
        assert_eq!(
            find_font_with(std::slice::from_ref(&system_font), || None, &[]),
            Some(system_font)
        );
        assert_eq!(find_font_with(&missing, || None, &[]), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_font_hint_names_the_distributions_package() {
        let hint = |text: &str| font_install_hint(text);
        assert!(hint("ID=ubuntu\nID_LIKE=debian\n").contains("fonts-dejavu-core"));
        assert!(hint("ID=fedora\n").contains("dejavu-sans-fonts"));
        assert!(hint("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n").contains("dnf"));
        assert!(hint("ID=endeavouros\nID_LIKE=arch\n").contains("ttf-dejavu"));
        assert!(hint("ID=arch\n").contains("pacman"));
        assert!(hint("ID=nixos\n").contains("fonts.packages"));
        assert!(hint("ID=opensuse-tumbleweed\nID_LIKE=\"opensuse suse\"\n").contains("zypper"));
        assert!(hint("ID=alpine\n").contains("apk add font-dejavu"));
        assert_eq!(
            hint(""),
            "install the DejaVu Sans font with your package manager"
        );
        // VERSION_ID and ID_LIKE are not ID.
        assert!(hint("VERSION_ID=12\nID_LIKE=debian\n").contains("apt"));
    }

    fn style() -> Style {
        Style::from_config(&OverlayConfig::default(), Appearance::Light).unwrap()
    }

    #[test]
    fn style_from_config() {
        let style = style();
        assert_eq!(style.background, [0xff, 0xd7, 0x5f]);
        assert!((style.opacity - 0.95).abs() < 1e-6);
        assert!(style.font_px > 17.0);
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
                description: String::new(),
                value: String::new(),
                bounds: Rect::new(x, 10, 40, 20),
                enabled: true,
                offscreen: false,
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
            area: None,
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

    #[test]
    fn hints_are_centred_and_the_search_box_starts_at_its_marker() {
        if find_font().is_none() {
            return;
        }
        let renderer = Renderer::new(style()).unwrap();
        let area = Rect::new(0, 0, 800, 600);
        let element = Rect::new(100, 100, 200, 50);
        let hint = renderer.label("as", 0, false, element, area).unwrap();
        let (width, height) = (hint.bounds.width, hint.bounds.height);
        assert_eq!(hint.bounds, label_bounds(element, width, height, area));
        let centre = |start: i32, size: u32| i64::from(start) * 2 + i64::from(size);
        assert!((centre(hint.bounds.x, width) - centre(100, 200)).abs() <= 1);
        assert!((centre(hint.bounds.y, height) - centre(100, 50)).abs() <= 1);
        // A hint at the monitor's right edge stays inside it.
        let edge = renderer
            .label("as", 0, false, Rect::new(795, 10, 5, 20), area)
            .unwrap();
        assert_eq!(i64::from(edge.bounds.x) + i64::from(edge.bounds.width), 800);

        let marker = search_box_anchor(area);
        let short = renderer.search_box("/a", marker, area).unwrap();
        let long = renderer
            .search_box("/a longer query", marker, area)
            .unwrap();
        assert_eq!(
            short.bounds.x, marker.x,
            "left edge on the marker, not centred"
        );
        assert_eq!(long.bounds.x, marker.x, "it grows to the right");
        assert!(long.bounds.width > short.bounds.width);
        let marker_centre = centre(marker.y, marker.height);
        assert!((centre(short.bounds.y, short.bounds.height) - marker_centre).abs() <= 1);
    }

    #[test]
    fn frame_outlines_the_scroll_area_on_each_monitor_it_spans() {
        if find_font().is_none() {
            return;
        }
        let renderer = Renderer::new(style()).unwrap();
        let monitors = [Rect::new(0, 0, 800, 600), Rect::new(800, 0, 800, 600)];
        // Spans both monitors: top, bottom and left strips on the first; top, bottom and
        // right strips on the second.
        let area = Rect::new(700, 100, 200, 100);
        let frame = Frame {
            area: Some(area),
            ..Frame::default()
        };
        let per_monitor = renderer.frame(&frame, &monitors);
        let bounds = |labels: &[Label]| labels.iter().map(|l| l.bounds).collect::<Vec<_>>();
        assert_eq!(
            bounds(&per_monitor[0]),
            [
                Rect::new(700, 100, 100, OUTLINE),
                Rect::new(700, 197, 100, OUTLINE),
                Rect::new(700, 100, OUTLINE, 100),
            ]
        );
        assert_eq!(
            bounds(&per_monitor[1]),
            [
                Rect::new(800, 100, 100, OUTLINE),
                Rect::new(800, 197, 100, OUTLINE),
                Rect::new(897, 100, OUTLINE, 100),
            ]
        );
        let strip = &per_monitor[0][0];
        let rgb = style().background;
        assert_eq!(strip.pixels.len(), 100 * 3 * 4);
        assert_eq!(strip.pixels[..4], [rgb[2], rgb[1], rgb[0], 255]);
    }
}
