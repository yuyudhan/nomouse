//! The nomouse settings window for Linux: a GTK 4 and libadwaita app that edits
//! `config.toml`, asks the daemon to reload it, and starts the daemon when it is not running.
//!
//! The pure parts (command line, key mapping, the doctor report, file lookups and the config
//! store) compile on every OS; the window itself only on Linux.
#![forbid(unsafe_code)]

pub mod args;
pub mod doctor;
pub mod keymap;
pub mod launch;
pub mod store;

#[cfg(target_os = "linux")]
pub mod app;
#[cfg(target_os = "linux")]
pub mod controls;
#[cfg(target_os = "linux")]
pub mod daemon;
#[cfg(target_os = "linux")]
pub mod list_editor;
#[cfg(target_os = "linux")]
pub mod pages;
#[cfg(target_os = "linux")]
pub mod recorder;
#[cfg(target_os = "linux")]
pub mod state;
#[cfg(target_os = "linux")]
pub mod widgets;
#[cfg(target_os = "linux")]
pub mod window;
