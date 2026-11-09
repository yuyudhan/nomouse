# ADR 0004: the Linux settings window is GTK 4 and libadwaita; the tray is the daemon's

- Status: accepted
- Date: 2026-11-09

## Context

nomouse should open like an app on Linux, as `nomouse.app` does on macOS: picking it in the
app launcher opens its settings, a status icon sits in the panel, and every setting in
`config.toml` can be changed without a text editor. The settings model
(`nomouse_core::settings`, documented in [architecture.md](../architecture.md)) is
shared with the macOS window, so the sections and validation are the same on both systems.

Ubuntu (GNOME) is the main Linux target; KDE, Xfce and tiling compositors (Sway, Hyprland,
i3) must work too. The oldest supported system is Ubuntu 22.04: GTK 4.6, libadwaita 1.1,
glibc 2.35. Every crate keeps `#![forbid(unsafe_code)]`, and `just check` runs clippy for Linux,
Windows and macOS from one host.

Options for the window:

1. **GTK 4 + libadwaita through gtk-rs.** Native on GNOME (the HIG look, dark style, the
   accessibility bus that our UI tests drive), good on other desktops, packaged everywhere
   (`libgtk-4-1`, `libadwaita-1-0`), safe Rust bindings without subclassing. Costs: two system
   libraries at runtime and their `-dev` packages at build time; libadwaita 1.1 lacks newer
   widgets (`AdwEntryRow`, `AdwSwitchRow`, `AdwMessageDialog`, `AdwBanner`, `AdwAboutWindow`).
2. **Qt 6 (cxx-qt) or a Rust-native toolkit (iced, egui, slint).** Qt needs a C++ toolchain
   and looks foreign on GNOME; the Rust-native toolkits draw their own widgets, so the window
   looks like no desktop and their AT-SPI support is partial or missing, which the UI tests
   and nomouse's own users (keyboard and accessibility first) need.
3. **A web view (Tauri, WebKitGTK).** Pulls a browser engine for a dozen forms.
4. **A TUI.** Not what "opens like an app" means.

For the tray: GTK 4 has no status icon API; the freedesktop `StatusNotifierItem` protocol with
a `com.canonical.dbusmenu` menu is what KDE, Xfce, waybar and GNOME's AppIndicator extension
(enabled by default on Ubuntu) display. `ksni` implements it but is Unlicense, outside our
`deny.toml` licence list; the protocol is two small D-Bus interfaces over zbus, which the Linux
backend already uses.

## Decision

- The settings window is a separate binary, `nomouse-settings` (crate
  `crates/nomouse-settings`), on GTK 4 and libadwaita via gtk-rs (`gtk4` 0.11 with
  `gnome_42`, `libadwaita` 0.9 with `v1_1`), composed without GObject subclassing so the crate
  keeps `#![forbid(unsafe_code)]`. On other systems the crate compiles to a stub. Its pages are
  the shared `settings::Section`s in order (Hotkeys, Labels, Appearance, Scroll, Search,
  Ignored apps, Advanced), with a General page first (on/off, start at login, updates, the
  session) and a Doctor page last; every row comes from `settings::FIELDS`, so a new key shows
  up on both systems. It is a single instance (`GtkApplication` id
  `io.github.yuyudhan.nomouse`, the launcher entry's id).
- The daemon (`nomouse`, no GUI libraries) owns the tray icon and its menu
  (`nomouse_linux::service`), and a D-Bus control interface the window uses. The daemon keeps
  running without GTK, so the hotkeys work on minimal systems and over SSH-started sessions.
- `nomouse launch` (the launcher entry) starts the daemon and opens the window; without the
  window (not installed, libraries missing) it shows a notification as before.
- Release builds link against Ubuntu 22.04's GTK and libadwaita on native x86_64 and aarch64
  runners. The `.deb` recommends `libgtk-4-1` and `libadwaita-1-0`; `install.sh` checks them
  with `ldd` and prints the package manager's command when they are missing (it never runs
  `sudo`).

## Consequences

- One more binary in the tarball and the `.deb`; the daemon's dependencies are unchanged.
- The window's widgets are limited to libadwaita 1.1: confirm dialogs are `GtkMessageDialog`,
  the About window `GtkAboutDialog`, the session warning a `GtkInfoBar`. Raise the floor (and
  use the newer widgets) when Ubuntu 22.04 support ends.
- CI and release jobs install `libgtk-4-dev` and `libadwaita-1-dev`; `deny.toml` skips the
  duplicate `system-deps` build dependency of libadwaita-sys.
- On a desktop without a tray host the window is the way in; `nomouse doctor` names the missing
  host and how to get one.
