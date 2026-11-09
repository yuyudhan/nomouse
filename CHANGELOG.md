# Changelog

All notable changes to nomouse are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). `just release X.Y.Z` turns the Unreleased section into
the release's section; add entries under Unreleased as changes land.

## [Unreleased]

### Added

- Licence: MIT OR Apache-2.0, at your option (`LICENSE-MIT`, `LICENSE-APACHE`); release archives
  carry both files and the Homebrew formula declares it.
- Focus-then-act: a highlight starts on the first label; Tab, ↓ and Ctrl-N move it forward,
  Shift-Tab, ↑ and Ctrl-P back, and Return acts on it (holding any modifier pressed with
  Return). The highlighted label is drawn in inverted colours on Linux and macOS.
- `hints.keys = "homerow"`: Homerow's bindings; a typed label highlights, Return clicks,
  Return twice double clicks, Shift-Return right clicks, Cmd-Return cmd-clicks.
- Pressing the activation hotkey again (or running `nomouse hints` again on Wayland) while the
  labels are shown dismisses them.
- Search ignores spaces (in the query and the text), a query of only `*` or `;` labels every
  element, and matching covers an element's description (tooltips), value (text field
  contents, slider positions; never password fields) and role (`button`, `check box`), names
  first. Linux reads descriptions and values over AT-SPI, macOS reads `AXDescription`,
  `AXHelp` (tooltips) and the `AXValue` of text fields, combo boxes and sliders.
- Scroll mode starts on the largest scrollable area and outlines it (Linux and macOS); areas
  are numbered, and Tab, Shift-Tab or a digit switch between them. `gg` and `G` go to the top
  and bottom, and Shift with `h` `j` `k` `l` scrolls faster (`scroll.dash_step`, default 15
  wheel steps).
- `ignore.apps`: apps (by macOS bundle id or executable name, Wayland app id, X11 `WM_CLASS`
  or process name) where the hotkeys and commands show no labels; `nomouse doctor` lists them.
- The daemon re-reads `config.toml` when it changes, or at once on `nomouse reload`; an
  invalid file is logged and the running settings stay.
- `overlay.theme`: `original` (the yellow labels, default), `light`, `dark`, or `auto`, which
  follows the desktop's light/dark preference (the XDG portal `color-scheme` on Linux, the
  app's effective appearance on macOS, Dark Aqua or not; read before each activation).
  `background`, `foreground`, `typed_foreground` and `border` now override the theme's colour
  only when set.
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
- `scripts/e2e-macos.sh` (`just e2e-macos`): end-to-end test on the Mac desktop against a Swift
  fixture app, asserting through Accessibility clicks, right click, double click, drag, scroll
  mode (largest list first, `j`, `G`, `gg`, Shift-`j`, Tab to a second list), search by name
  and by tooltip, focus-then-act, the Homerow bindings, continuous mode ending at a text field,
  the `auto` theme, the daemon (hotkey click, socket, toggle), `ignore.apps` with
  `nomouse reload`, and Dvorak.
- `nomouse.app` (`scripts/release/app.sh`, `just app`): the macOS binary in an ad-hoc signed
  agent bundle (no Dock icon, bundle id `com.yuyudhan.nomouse`, icon), released as
  `nomouse-universal-apple-darwin.app.zip`; `scripts/test-app.sh` checks it.
- `nomouse-universal-apple-darwin.dmg` (`scripts/release/dmg.sh`, built without mounting) and
  the Homebrew cask `nomouse-app.rb` for it. With the owner's Developer ID certificate and App
  Store Connect key as repository secrets (`scripts/release/signing.sh`), the release signs
  `nomouse.app` with the hardened runtime and notarizes and staples the app and the disk image
  (`scripts/release/notarize.sh`); without them both stay ad-hoc signed.
- Guided first run (`nomouse setup`, and the daemon's start when Accessibility is missing):
  explains each permission, asks macOS for it and opens the Accessibility pane, waits for the
  switch, then restarts the daemon. Without a terminal (`nomouse.app`) it uses dialogs. On
  Linux `nomouse setup` turns the AT-SPI switch on.
- Layout-aware hotkeys on macOS: a hotkey on a letter, digit or `; , / .` matches the key that
  types that character on the current keyboard layout (Dvorak, Colemak, AZERTY, German, ...)
  and follows layout switches while the daemon runs; characters no key types alone (digits on
  AZERTY, letters on non-Latin layouts) keep their ANSI position, as macOS shortcuts do.
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
- macOS: `G` and Shift-`h` `j` `k` `l` in scroll mode did not scroll vertically: the wheel
  events carried the Shift key still held, which macOS turns into horizontal scrolling. Wheel
  events now carry no modifiers.
- macOS: the labels faded out for a moment after a pick instead of vanishing (AppKit's panel
  animation); they now disappear before the click lands.
