//! The daemon's D-Bus names on the session bus, shared by the daemon (which serves them) and
//! the settings window and tests (which call them).
//!
//! Interface [`INTERFACE`] at [`OBJECT_PATH`], owned as [`BUS_NAME`]:
//!
//! | Member | Kind | Meaning |
//! |--------|------|---------|
//! | `Pause()` | method | release the hotkeys and ignore mode requests until `Resume` |
//! | `Resume()` | method | grab the hotkeys again |
//! | `Reload()` | method | re-read `config.toml` now |
//! | `Run(s mode)` | method | start `hints`, `continuous`, `scroll` or `search` |
//! | `Quit()` | method | stop the daemon |
//! | `Paused` | property `b` | paused by `Pause` or the tray menu |
//! | `Active` | property `b` | the backend runs in this session (false: tray only) |
//! | `Warning` | property `s` | why the session is limited or unsupported, or empty |
//! | `Version` | property `s` | the daemon's version |
//!
//! A development build owns `io.github.yuyudhan.nomouse.dev.Daemon` and its settings window is
//! `io.github.yuyudhan.nomouse.dev` ([`Flavour`]), so it runs beside the installed release
//! without taking over its daemon or window.

use nomouse_core::Flavour;

/// The well-known name `flavour`'s daemon owns.
#[must_use]
pub const fn bus_name(flavour: Flavour) -> &'static str {
    match flavour {
        Flavour::Release => "io.github.yuyudhan.nomouse.Daemon",
        Flavour::Dev => "io.github.yuyudhan.nomouse.dev.Daemon",
    }
}

/// The well-known name this build's daemon owns.
pub const BUS_NAME: &str = bus_name(Flavour::CURRENT);

/// The daemon object's path.
pub const OBJECT_PATH: &str = "/io/github/yuyudhan/nomouse/Daemon";

/// The daemon interface.
pub const INTERFACE: &str = "io.github.yuyudhan.nomouse.Daemon1";

/// This build's application id of the settings window (the window's class and its
/// single-instance D-Bus name; for a release build also the launcher entry's desktop file name
/// without `.desktop`).
pub const APP_ID: &str = Flavour::CURRENT.linux_app_id();

/// The installed icon's name (`packaging/linux/share/icons`), which both flavours show.
pub const ICON: &str = "io.github.yuyudhan.nomouse";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_names_are_unchanged_and_dev_names_are_apart() {
        assert_eq!(
            bus_name(Flavour::Release),
            "io.github.yuyudhan.nomouse.Daemon"
        );
        assert_eq!(
            bus_name(Flavour::Dev),
            "io.github.yuyudhan.nomouse.dev.Daemon"
        );
        assert_eq!(BUS_NAME, bus_name(Flavour::CURRENT));
        assert_eq!(APP_ID, Flavour::CURRENT.linux_app_id());
        assert_eq!(ICON, Flavour::Release.linux_app_id());
    }
}
