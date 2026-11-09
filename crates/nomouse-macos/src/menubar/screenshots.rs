//! The documentation's screenshots of the menu and every settings page, light and dark,
//! drawn by the app itself (`nomouse screenshots <dir>`, `scripts/screenshots-macos.sh`), so
//! they need no Screen Recording permission and always show the current interface.

use std::path::{Path, PathBuf};
use std::time::Duration;

use nomouse_core::menu::{MenuState, UpdateLine};
use nomouse_core::settings::{Section, Settings};
use nomouse_sys_macos::ui::snapshot;

use super::{MenuBar, pages};

/// How long the app draws before a picture is taken.
const SETTLE: Duration = Duration::from_millis(400);

/// File name part of each settings page, in window order.
fn page_names() -> Vec<String> {
    std::iter::once("general".to_owned())
        .chain(
            Section::ALL
                .iter()
                .map(|section| section.title().to_lowercase().replace(' ', "-")),
        )
        .collect()
}

fn write(dir: &Path, name: &str, png: Option<Vec<u8>>) -> Result<PathBuf, String> {
    let png = png.ok_or_else(|| format!("{name}: nothing was drawn"))?;
    let path = dir.join(name);
    std::fs::write(&path, png).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

/// Writes `menu-light.png`, `menu-dark.png` and `settings-<page>-<light|dark>.png` into `dir`
/// from the configuration file in effect; returns the files written. Run on the main thread.
///
/// # Errors
/// Off the main thread, when the menu or a window draws nothing, or a file cannot be written.
pub fn screenshots(dir: &Path) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let config = Settings::open_default()
        .map_err(|error| error.to_string())?
        .config()
        .clone();
    let mut bar = MenuBar::new(MenuState {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        paused: false,
        active: true,
        warning: None,
        hotkeys: Some(config.hotkeys.clone()),
        start_at_login: super::start_at_login(),
        update: UpdateLine::NotChecked,
    })
    .map_err(|error| error.to_string())?;
    let mtm = bar.app.main_thread();
    let mut written = Vec::new();
    for (dark, look) in [(false, "light"), (true, "dark")] {
        snapshot::set_appearance(mtm, Some(dark));
        bar.pump(SETTLE);
        written.push(write(
            dir,
            &format!("menu-{look}.png"),
            bar.item.picture_menu(),
        )?);
        for (index, name) in page_names().iter().enumerate() {
            bar.open_settings(Some(index));
            bar.pump(SETTLE);
            let picture = bar.window.as_ref().and_then(super::SettingsWindow::picture);
            written.push(write(dir, &format!("settings-{name}-{look}.png"), picture)?);
        }
        bar.close_settings();
    }
    snapshot::set_appearance(mtm, None);
    debug_assert_eq!(page_names().len(), pages::pages().len());
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_a_file_name() {
        assert_eq!(
            page_names(),
            [
                "general",
                "hotkeys",
                "labels",
                "appearance",
                "scroll",
                "search",
                "ignored-apps",
                "advanced"
            ]
        );
        assert_eq!(page_names().len(), pages::pages().len());
    }
}
