# Changelog

All notable changes to nomouse are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). `just release X.Y.Z` turns the Unreleased section into
the release's section; add entries under Unreleased as changes land.

## [Unreleased]

### Added

- Focus-then-act: a highlight starts on the first label; Tab, ↓ and Ctrl-N move it forward,
  Shift-Tab, ↑ and Ctrl-P back, and Return acts on it (holding any modifier pressed with
  Return). The highlighted label is drawn in inverted colours on Linux.
- `hints.keys = "homerow"`: Homerow's bindings; a typed label highlights, Return clicks,
  Return twice double clicks, Shift-Return right clicks, Cmd-Return cmd-clicks.
- Pressing the activation hotkey again (or running `nomouse hints` again on Wayland) while the
  labels are shown dismisses them.
- Search ignores spaces (in the query and the text), a query of only `*` or `;` labels every
  element, and matching covers an element's description (tooltips), value (text field
  contents, slider positions; never password fields) and role (`button`, `check box`), names
  first. Linux reads descriptions and values over AT-SPI.
- Scroll mode starts on the largest scrollable area and outlines it; areas are numbered, and
  Tab, Shift-Tab or a digit switch between them. `gg` and `G` go to the top and bottom, and
  Shift with `h` `j` `k` `l` scrolls faster (`scroll.dash_step`, default 15 wheel steps).
- `ignore.apps`: apps (by macOS bundle id, Wayland app id, X11 `WM_CLASS` or process name)
  where the hotkeys and commands show no labels; `nomouse doctor` lists them.
- The daemon re-reads `config.toml` when it changes, or at once on `nomouse reload`; an
  invalid file is logged and the running settings stay.
- `overlay.theme`: `original` (the yellow labels, default), `light`, `dark`, or `auto`, which
  follows the desktop's light/dark preference (the XDG portal `color-scheme` on Linux, read
  before each activation). `background`, `foreground`, `typed_foreground` and `border` now
  override the theme's colour only when set.
- Continuous mode ends after a left click on a text field, so the keyboard stays there for
  typing. On Linux, editable text objects (GTK 3 entries) now count as text fields.
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
