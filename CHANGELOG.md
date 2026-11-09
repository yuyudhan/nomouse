# Changelog

All notable changes to nomouse are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). `just release X.Y.Z` turns the Unreleased section into
the release's section; add entries under Unreleased as changes land.

## [Unreleased]

### Added

- Hint mode: a hotkey labels every clickable element of the focused window from the
  accessibility tree; typing a label clicks it (left, right, double, middle, move, semantic
  click), with a continuous mode that keeps hinting after each click.
- Scroll mode and configurable keys, alphabet, colours and per-app settings in `config.toml`
  (`nomouse config init|path|show`).
- Search mode (`hotkeys.search`, default `ctrl+alt+/`; `nomouse search`): type to filter
  elements by name (fuzzy, case-insensitive), Enter clicks the best match, Tab then a label
  picks another; the overlay shows the query on Linux, macOS and Windows.
- Modifier clicks: `1`-`4` toggle Shift, Ctrl, Alt, Super for the next click (cmd/ctrl+click to
  open a link in a new tab), on every backend.
- Scroll mode targets scrollable areas (AT-SPI scroll panes, AX scroll areas, UIA Scroll
  pattern) instead of a control inside them.
- Linux backend: AT-SPI elements, X11 overlay, XTEST input and XGrabKey hotkeys.
- Wayland backend (Linux feature `wayland`, on by default) for wlroots compositors (Sway,
  Hyprland): layer-shell overlay that takes the keyboard while labels are up, virtual pointer
  and keyboard input (or `/dev/uinput`), native Wayland windows placed with the compositor's
  IPC; `nomouse doctor` explains what GNOME and KDE Wayland lack.
- `nomouse continuous`, and the daemon's command socket (`$XDG_RUNTIME_DIR/nomouse.sock`, else
  inside a private 0700 `<tmp>/nomouse-<uid>/`, refused when unsafe; requests capped at 4 KiB,
  each client answered on its own thread with a read timeout):
  `nomouse hints|continuous|scroll|search` hand the mode to a running daemon (how Wayland
  compositors bind hotkeys); `--standalone` runs it in-process.
- `scripts/e2e-wayland.sh`: end-to-end test on a private headless Sway with native and
  XWayland GTK windows.
- `scripts/e2e-macos.sh` (`just e2e-macos`): end-to-end test on the Mac desktop against a Swift
  fixture app, asserting clicks, right click, double click, drag, scroll, search and the daemon
  through Accessibility.
- macOS backend: AX tree walk, NSPanel overlay, CGEvent input and event-tap hotkeys.
- Windows backend: UI Automation elements, layered overlay, SendInput and hotkeys.
- `nomouse doctor` reports whether the platform's accessibility and input access work.
- Release pipeline: archives for Linux (x86_64, aarch64), macOS (universal) and Windows
  (x86_64) with SHA-256 checksums, `.deb` packages, a Homebrew formula, and no-sudo installers
  (`install.sh`, `install.ps1`).

### Changed

- The config file is `$XDG_CONFIG_HOME/nomouse/config.toml` (else
  `~/.config/nomouse/config.toml`) on macOS as on Linux, instead of
  `~/Library/Application Support/nomouse/`; move an existing file there. Windows keeps
  `%APPDATA%\nomouse\config.toml`.
- Core API: `Action::Click` has a `modifiers` field (`Action::with_modifiers`,
  `Action::modifiers`, `Modifiers::NONE`); `Overlay::show_search` draws the search box (default:
  labels only); `HintSession::search`, `HintSession::query`, `SessionEvent::ModifiersChanged`;
  new `nomouse_core::search` module. `LinuxPlatform::perform_with_modifiers` and the Windows
  `Backend::modifier_click` are removed: `perform` holds the action's modifiers.

### Fixed

- macOS: after a scan that took longer than about a second (a cold or busy app), the first key
  typed went to the app instead of nomouse, because macOS had switched the event tap off. The
  tap is now replaced before nomouse waits for keys whenever macOS disabled it.
