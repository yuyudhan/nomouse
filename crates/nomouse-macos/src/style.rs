//! The overlay look, derived from `[overlay]` in the configuration.

use nomouse_sys_macos::{LabelStyle, Rgba};

fn rgba([red, green, blue]: [u8; 3]) -> Rgba {
    Rgba {
        red: f64::from(red) / 255.0,
        green: f64::from(green) / 255.0,
        blue: f64::from(blue) / 255.0,
        alpha: 1.0,
    }
}

/// Builds the label style from the font size in points and the colours as returned by
/// `OverlayConfig::colours` (background, foreground, typed foreground, border).
pub fn label_style(font_size: u32, colours: [[u8; 3]; 4]) -> LabelStyle {
    let [background, foreground, typed_foreground, border] = colours;
    LabelStyle {
        font_size: f64::from(font_size),
        background: rgba(background),
        foreground: rgba(foreground),
        typed_foreground: rgba(typed_foreground),
        border: rgba(border),
    }
}

/// Converts an opacity percentage (clamped to 0 to 100) to Core Animation's 0.0 to 1.0.
pub fn opacity(percent: u32) -> f32 {
    f32::from(u8::try_from(percent.min(100)).unwrap_or(100)) / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_scale_to_unit_range() {
        let style = label_style(13, [[255, 0, 0], [0, 255, 0], [0, 0, 255], [51, 51, 51]]);
        assert!((style.font_size - 13.0).abs() < f64::EPSILON);
        assert!((style.background.red - 1.0).abs() < f64::EPSILON);
        assert!(style.background.green.abs() < f64::EPSILON);
        assert!((style.typed_foreground.blue - 1.0).abs() < f64::EPSILON);
        assert!((style.border.red - 0.2).abs() < 1e-9);
        assert!((style.border.alpha - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn opacity_percent() {
        assert!((opacity(95) - 0.95).abs() < 1e-6);
        assert!((opacity(250) - 1.0).abs() < f32::EPSILON);
        assert!(opacity(0).abs() < f32::EPSILON);
    }
}
