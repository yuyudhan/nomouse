//! The tray icon: a themed icon name (installed with the launcher entry), plus the same logo
//! as pixmaps for hosts that cannot find the name.

use nomouse_core::menu::MenuState;

use super::item::Pixmap;

/// The icon's name while nomouse is on.
const ON: &str = "io.github.yuyudhan.nomouse-symbolic";
/// Its name while paused or unable to run.
const OFF: &str = "io.github.yuyudhan.nomouse-paused-symbolic";

/// The themed icon name for `state`.
pub(super) const fn name(state: &MenuState) -> &'static str {
    if state.active && !state.paused {
        ON
    } else {
        OFF
    }
}

/// The icon as ARGB32 pixmaps for `state`.
pub(super) fn pixmaps(_state: &MenuState) -> Vec<Pixmap> {
    Vec::new()
}
