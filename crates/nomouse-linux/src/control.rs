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

/// The well-known name the daemon owns.
pub const BUS_NAME: &str = "io.github.yuyudhan.nomouse.Daemon";

/// The daemon object's path.
pub const OBJECT_PATH: &str = "/io/github/yuyudhan/nomouse/Daemon";

/// The daemon interface.
pub const INTERFACE: &str = "io.github.yuyudhan.nomouse.Daemon1";

/// The application id of the settings window (the launcher entry's desktop file name without
/// `.desktop`, and the single-instance D-Bus name).
pub const APP_ID: &str = "io.github.yuyudhan.nomouse";
