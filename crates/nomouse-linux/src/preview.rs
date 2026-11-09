//! A sample of hint labels drawn by the overlay's own renderer, for the settings window's live
//! appearance preview: what you see there is pixel for pixel what the overlay draws.

use nomouse_core::config::{Appearance, OverlayConfig};
use nomouse_core::{PlatformError, Rect};

use crate::x11::render::{Renderer, Style};

/// Premultiplied BGRA pixels, `width * 4` bytes per row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, premultiplied blue, green, red, alpha.
    pub pixels: Vec<u8>,
}

/// The sample labels: text, characters already typed, highlighted.
const SAMPLES: [(&str, usize, bool); 4] = [
    ("a", 0, false),
    ("sd", 1, false),
    ("f", 0, true),
    ("jk", 0, false),
];

/// Pixels between two labels.
const GAP: u32 = 10;

/// Draws [`SAMPLES`] side by side with the overlay's renderer: an untyped label, one with its
/// first letter typed, the highlighted label and another plain one.
///
/// # Errors
/// [`PlatformError::Backend`] for an invalid colour or when no TrueType font is installed.
pub fn render(
    config: &OverlayConfig,
    appearance: Appearance,
) -> Result<PreviewImage, PlatformError> {
    let renderer = Renderer::new(Style::from_config(config, appearance)?)?;
    let area = Rect::new(0, 0, 10_000, 10_000);
    let labels: Vec<_> = SAMPLES
        .iter()
        .filter_map(|(text, typed, focused)| {
            renderer.label(text, *typed, *focused, Rect::new(0, 0, 1, 200), area)
        })
        .collect();
    let height = labels.iter().map(|l| l.bounds.height).max().unwrap_or(0);
    let width = labels.iter().map(|l| l.bounds.width).sum::<u32>()
        + GAP * u32::try_from(labels.len().saturating_sub(1)).unwrap_or(0);
    let stride = width as usize * 4;
    let mut pixels = vec![0; stride * height as usize];
    let mut x = 0usize;
    for label in &labels {
        let row_bytes = label.bounds.width as usize * 4;
        let top = (height - label.bounds.height) as usize / 2;
        for (row, source) in label.pixels.chunks_exact(row_bytes).enumerate() {
            let start = (top + row) * stride + x * 4;
            pixels[start..start + row_bytes].copy_from_slice(source);
        }
        x += label.bounds.width as usize + GAP as usize;
    }
    Ok(PreviewImage {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(image: &PreviewImage, x: u32, y: u32) -> [u8; 4] {
        let at = (y * image.width + x) as usize * 4;
        let p = &image.pixels[at..at + 4];
        [p[0], p[1], p[2], p[3]]
    }

    #[test]
    fn draws_the_theme_and_follows_size_and_colour_overrides() {
        let config = OverlayConfig::default();
        let Ok(small) = render(&config, Appearance::Light) else {
            // No TrueType font on this machine: the overlay cannot draw either.
            assert!(crate::x11::render::find_font().is_none());
            return;
        };
        assert_eq!(
            small.pixels.len(),
            (small.width * small.height * 4) as usize
        );
        let big = render(
            &OverlayConfig {
                font_size: 30,
                ..OverlayConfig::default()
            },
            Appearance::Light,
        )
        .unwrap();
        assert!(big.width > small.width && big.height > small.height);

        // The first label's border pixel is the theme border, then an override replaces it.
        let [_, _, _, border] = config.colours(Appearance::Light).unwrap();
        let opaque = OverlayConfig {
            opacity: 100,
            ..OverlayConfig::default()
        };
        let image = render(&opaque, Appearance::Light).unwrap();
        // Every sample label has the same height, so row 0 is each label's top border.
        let top = 0;
        assert_eq!(
            pixel(&image, 0, top),
            [border[2], border[1], border[0], 255]
        );
        let red = render(
            &OverlayConfig {
                border: Some("#ff0000".to_owned()),
                ..opaque
            },
            Appearance::Light,
        )
        .unwrap();
        assert_eq!(pixel(&red, 0, top), [0, 0, 255, 255]);
    }

    #[test]
    fn a_bad_colour_is_an_error() {
        let config = OverlayConfig {
            background: Some("yellow".to_owned()),
            ..OverlayConfig::default()
        };
        assert!(render(&config, Appearance::Light).is_err());
    }
}
