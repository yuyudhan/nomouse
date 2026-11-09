//! The overlay look, derived from `[overlay]` in the configuration.

use nomouse_core::config::{Appearance, ConfigError, OverlayConfig, Theme};
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

/// The label style, re-resolved before each activation when `overlay.theme = "auto"` follows
/// the system appearance, so a switch in System Settings needs no restart.
#[derive(Debug, Clone)]
pub struct Themed {
    overlay: OverlayConfig,
    appearance: Appearance,
}

impl Themed {
    /// The style for `overlay` now, and what keeps it current. `system` reads the system
    /// appearance; it is called only for `theme = "auto"`.
    ///
    /// # Errors
    /// [`ConfigError::Colour`] for an explicit colour that is not `#rrggbb`.
    pub fn new(
        overlay: &OverlayConfig,
        system: impl FnOnce() -> Appearance,
    ) -> Result<(Self, LabelStyle), ConfigError> {
        let appearance = if overlay.theme == Theme::Auto {
            system()
        } else {
            Appearance::Light
        };
        let style = label_style(overlay.font_size, overlay.colours(appearance)?);
        let themed = Self {
            overlay: overlay.clone(),
            appearance,
        };
        Ok((themed, style))
    }

    /// The appearance the current style was resolved for.
    pub const fn appearance(&self) -> Appearance {
        self.appearance
    }

    /// The current label background as `#rrggbb`, for logs.
    pub fn background(&self) -> String {
        self.overlay.colours(self.appearance).map_or_else(
            |_| "?".to_owned(),
            |[[red, green, blue], ..]| format!("#{red:02x}{green:02x}{blue:02x}"),
        )
    }

    /// The new style when the theme is `auto` and `system` reports another appearance than
    /// the last look; `None` otherwise (`system` is not called for other themes).
    pub fn changed(&mut self, system: impl FnOnce() -> Appearance) -> Option<LabelStyle> {
        if self.overlay.theme != Theme::Auto {
            return None;
        }
        let appearance = system();
        if appearance == self.appearance {
            return None;
        }
        self.appearance = appearance;
        // The colours were validated by `new`; only the theme's palette changes.
        let colours = self.overlay.colours(appearance).ok()?;
        Some(label_style(self.overlay.font_size, colours))
    }
}

/// The appearance macOS reports: dark for Dark Aqua, light otherwise.
pub const fn appearance(prefers_dark: bool) -> Appearance {
    if prefers_dark {
        Appearance::Dark
    } else {
        Appearance::Light
    }
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

    fn overlay(toml: &str) -> OverlayConfig {
        nomouse_core::config::Config::from_toml(toml)
            .unwrap()
            .overlay
    }

    fn background(theme: Theme, appearance: Appearance) -> Rgba {
        label_style(13, theme.palette(appearance)).background
    }

    #[test]
    fn auto_follows_a_dark_system() {
        let (themed, style) = Themed::new(&overlay("[overlay]\ntheme = \"auto\"\n"), || {
            Appearance::Dark
        })
        .unwrap();
        assert_eq!(themed.appearance(), Appearance::Dark);
        assert_eq!(style.background, background(Theme::Dark, Appearance::Dark));
        assert_ne!(
            style.background,
            background(Theme::Light, Appearance::Light)
        );
    }

    #[test]
    fn auto_switches_with_the_system_and_reports_only_changes() {
        let (mut themed, _) = Themed::new(&overlay("[overlay]\ntheme = \"auto\"\n"), || {
            Appearance::Light
        })
        .unwrap();
        assert_eq!(themed.changed(|| Appearance::Light), None);
        let dark = themed.changed(|| Appearance::Dark).unwrap();
        assert_eq!(dark.background, background(Theme::Dark, Appearance::Dark));
        assert_eq!(themed.changed(|| Appearance::Dark), None);
        let light = themed.changed(|| Appearance::Light).unwrap();
        assert_eq!(
            light.background,
            background(Theme::Light, Appearance::Light)
        );
    }

    #[test]
    fn fixed_themes_never_read_the_system_and_overrides_win() {
        let unread = || -> Appearance { unreachable!("the system appearance was read") };
        let (mut themed, style) =
            Themed::new(&overlay("[overlay]\ntheme = \"light\"\n"), unread).unwrap();
        assert_eq!(
            style.background,
            background(Theme::Light, Appearance::Light)
        );
        assert_eq!(themed.changed(unread), None);
        let (mut themed, style) = Themed::new(
            &overlay("[overlay]\ntheme = \"auto\"\nbackground = \"#102030\"\n"),
            || Appearance::Dark,
        )
        .unwrap();
        assert_eq!(style.background, rgba([0x10, 0x20, 0x30]));
        assert_eq!(themed.background(), "#102030");
        let light = themed.changed(|| Appearance::Light).unwrap();
        assert_eq!(light.background, rgba([0x10, 0x20, 0x30]));
        assert_ne!(light.foreground, style.foreground);
    }

    #[test]
    fn macos_appearance() {
        assert_eq!(appearance(true), Appearance::Dark);
        assert_eq!(appearance(false), Appearance::Light);
    }
}
