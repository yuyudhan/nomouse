//! The tray icon: a themed symbolic icon name (installed with the launcher entry, so panels
//! recolour it to match), plus the same drawing as pixmaps for hosts that cannot find the name.
//! The shapes are those of `assets/logo/nomouse-symbolic.svg` and
//! `nomouse-paused-symbolic.svg`: the logo's keycap with the pointer cut out; while paused or
//! unable to run only the keycap's outline, with a dimmed pointer.

use nomouse_core::menu::MenuState;
use tiny_skia::{Color, FillRule, Paint, Path, PathBuilder, Pixmap as Canvas, Transform};

use super::item::Pixmap;

/// The icon's name while nomouse is on.
const ON: &str = "io.github.yuyudhan.nomouse-symbolic";
/// Its name while paused or unable to run.
const OFF: &str = "io.github.yuyudhan.nomouse-paused-symbolic";
/// The pixmap sizes offered; hosts pick the closest.
const SIZES: [u32; 4] = [16, 22, 32, 48];
/// The symbolic icons' grey, readable on light and dark panels.
const GREY: u8 = 0xbe;

/// Whether `state` shows the "off" icon.
const fn off(state: &MenuState) -> bool {
    !state.active || state.paused
}

/// The themed icon name for `state`.
pub(super) const fn name(state: &MenuState) -> &'static str {
    if off(state) { OFF } else { ON }
}

/// The icon for `state` as ARGB32 pixmaps (network byte order, not premultiplied).
pub(super) fn pixmaps(state: &MenuState) -> Vec<Pixmap> {
    SIZES
        .iter()
        .filter_map(|size| draw(*size, off(state)))
        .collect()
}

/// A rounded rectangle from (`left`, `top`) to (`right`, `bottom`) with corner `radius`, on
/// the 16-unit grid of the SVG.
fn rounded(builder: &mut PathBuilder, [left, top, right, bottom]: [f32; 4], radius: f32) {
    builder.move_to(left + radius, top);
    builder.line_to(right - radius, top);
    builder.quad_to(right, top, right, top + radius);
    builder.line_to(right, bottom - radius);
    builder.quad_to(right, bottom, right - radius, bottom);
    builder.line_to(left + radius, bottom);
    builder.quad_to(left, bottom, left, bottom - radius);
    builder.line_to(left, top + radius);
    builder.quad_to(left, top, left + radius, top);
    builder.close();
}

/// A closed polygon through `points`.
fn polygon(builder: &mut PathBuilder, points: &[(f32, f32)]) {
    let mut points = points.iter();
    if let Some((x, y)) = points.next() {
        builder.move_to(*x, *y);
    }
    for (x, y) in points {
        builder.line_to(*x, *y);
    }
    builder.close();
}

/// The pointer cut out of the keycap.
fn pointer(builder: &mut PathBuilder) {
    polygon(
        builder,
        &[
            (5.3, 3.3),
            (5.3, 11.4),
            (7.25, 9.6),
            (8.65, 12.7),
            (10.1, 12.1),
            (8.7, 9.05),
            (11.45, 9.05),
        ],
    );
}

/// The keycap with the pointer cut out (off: the keycap's outline only).
fn keycap(off: bool) -> Option<Path> {
    let mut builder = PathBuilder::new();
    rounded(&mut builder, [1.0, 1.0, 15.0, 15.0], 3.2);
    if off {
        rounded(&mut builder, [2.2, 2.2, 13.8, 13.8], 2.0);
    } else {
        pointer(&mut builder);
    }
    builder.finish()
}

/// Draws the icon at `size` pixels square.
fn draw(size: u32, off: bool) -> Option<Pixmap> {
    let mut canvas = Canvas::new(size, size)?;
    let scale = f32::from(u16::try_from(size).ok()?) / 16.0;
    let transform = Transform::from_scale(scale, scale);
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(GREY, GREY, GREY, 255));
    paint.anti_alias = true;
    canvas.fill_path(&keycap(off)?, &paint, FillRule::EvenOdd, transform, None);
    if off {
        let mut builder = PathBuilder::new();
        pointer(&mut builder);
        paint.set_color(Color::from_rgba8(GREY, GREY, GREY, 153));
        canvas.fill_path(
            &builder.finish()?,
            &paint,
            FillRule::Winding,
            transform,
            None,
        );
    }
    let argb = canvas
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let colour = pixel.demultiply();
            [colour.alpha(), colour.red(), colour.green(), colour.blue()]
        })
        .collect();
    let pixels = i32::try_from(size).ok()?;
    Some((pixels, pixels, argb))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(pixmap: &Pixmap, x: i32, y: i32) -> u8 {
        let index = usize::try_from((y * pixmap.0 + x) * 4).unwrap();
        pixmap.2[index]
    }

    #[test]
    fn names_follow_the_state() {
        let on = MenuState {
            active: true,
            ..MenuState::default()
        };
        assert_eq!(name(&on), ON);
        let paused = MenuState {
            paused: true,
            ..on.clone()
        };
        assert_eq!(name(&paused), OFF);
        assert_eq!(name(&MenuState::default()), OFF);
    }

    #[test]
    fn pixmaps_cut_the_pointer_out_and_dim_it_when_off() {
        let on = pixmaps(&MenuState {
            active: true,
            ..MenuState::default()
        });
        assert_eq!(
            on.iter().map(|p| (p.0, p.1)).collect::<Vec<_>>(),
            [(16, 16), (22, 22), (32, 32), (48, 48)]
        );
        let big = &on[3];
        assert_eq!(big.2.len(), 48 * 48 * 4);
        // At 48 px a unit is 3 px. Inside the pointer (6.2, 7.2): cut out. In the keycap left
        // of the pointer (3, 8): solid grey. Left of the keycap (0.5, 8): empty.
        assert_eq!(alpha(big, 18, 21), 0);
        assert_eq!(alpha(big, 9, 24), 255);
        assert_eq!(
            &big.2[(24 * 48 + 9) * 4 + 1..(24 * 48 + 9) * 4 + 4],
            [GREY; 3]
        );
        assert_eq!(alpha(big, 1, 24), 0);
        let off = pixmaps(&MenuState::default());
        let big = &off[3];
        // Off: the outline (x 1 to 2.2) is solid, the keycap's inside empty, the pointer dimmed.
        assert_eq!(alpha(big, 4, 24), 255);
        assert_eq!(alpha(big, 9, 24), 0);
        assert_eq!(alpha(big, 18, 21), 153);
    }
}
