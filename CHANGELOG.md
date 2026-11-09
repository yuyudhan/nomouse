# Changelog

All notable changes to nomouse are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). `just release X.Y.Z` turns the Unreleased section into
the release's section; add entries under Unreleased as changes land.

## [Unreleased]

### Added

- Linux tray icon: the daemon shows a status icon over the freedesktop StatusNotifierItem
  protocol (KDE, Xfce, waybar's tray, and GNOME with the AppIndicator extension that Ubuntu
  enables). Its menu shows whether nomouse is on, the session warning (GNOME on Wayland: log
  in with "Ubuntu on Xorg"), Pause/Resume, each mode with its hotkey, Settings…, Run doctor…,
  Start at login, Check for updates…, About and Quit. Without a tray host nomouse runs as
  before and `nomouse doctor` says why there is no icon.
- `nomouse pause`, `nomouse resume` and `nomouse quit`, and the same over D-Bus
  (`io.github.yuyudhan.nomouse.Daemon1`: `Pause`, `Resume`, `Reload`, `Run`, `Quit`, and the
  `Paused`, `Active`, `Warning` and `Version` properties). While paused the hotkeys are
  released and mode commands are ignored.
- Start at login on Linux, from the tray menu or the settings window: an XDG autostart entry
  (`~/.config/autostart/nomouse.desktop`) that runs `nomouse daemon`.
- The update check (tray menu, settings window): reads the latest release from GitHub and says
  whether a newer one is out; installing stays with the installer or the package manager.
- Linux settings window, `nomouse-settings` (GTK 4 and libadwaita; GNOME 42 / Ubuntu 22.04 and
  later): a General page (on/off, start at login, the update check, the session), then Hotkeys,
  Labels, Appearance (with a live preview drawn by the overlay's own renderer), Scroll, Search,
  Ignored apps and Advanced, the same sections as the macOS window, and a Doctor page. Hotkeys
  are recorded by pressing them on X11; on Wayland the page shows the lines to add to the
  compositor's config. Every change is saved at once into `config.toml`, which the daemon
  reloads; "Open config file" and "Reset to defaults…" (asks first) are on the Advanced page.
  One window per session: opening it again raises it on the asked page
  (`nomouse-settings --page hotkeys`).
- The tray icon is the logo's keycap as a symbolic icon that panels recolour
  (`io.github.yuyudhan.nomouse-symbolic`), and its outline while paused or unable to run
  (`io.github.yuyudhan.nomouse-paused-symbolic`), installed with the launcher icons.
- macOS: nomouse.app is a menu bar app, like Homerow: no Dock icon, a keycap template icon in
  the menu bar that follows light and dark menu bars (dimmed while paused). Its menu shows the
  state, the Accessibility status with a button to fix it, Pause/Resume, each mode with its
  hotkey, Settings… (`cmd+,`), Run doctor… (the report in a window, with Copy), Start at login
  (`SMAppService`, listed under System Settings › General › Login Items), Check for updates…
  (opens the newer release's page), About and Quit.
- macOS settings window: a General page, then Hotkeys (shortcut recorders that flag a chord
  macOS itself uses, read from its keyboard shortcuts), Labels, Appearance (with a live
  preview of the labels), Scroll, Search, Ignored apps (an app picker) and Advanced (Open
  Config File, Show in Finder, Reset to Defaults… with a confirmation). Every change is saved
  into `config.toml` at once and the daemon reloads it; a hand edit shows in the open window.
- A new logo: a hint-label keycap with a mouse pointer pressed into it, as the app icon
  (`.icns` from 16 to 1024 px), the macOS menu bar template, the Linux launcher icons and
  symbolic tray icon, and the README header. The SVG sources are in `assets/logo/`
  (`scripts/render-logo.sh` renders the PNG files).
- Screenshots of the macOS menu and every settings page, light and dark, in
  `docs/media/macos/` (`scripts/screenshots-macos.sh`: the app draws its own windows, so no
  Screen Recording permission is needed).

### Changed

- Picking nomouse in the app launcher (`nomouse launch`) now also opens the settings window;
  without it (not installed, or GTK 4 and libadwaita missing) the notification stays.
- `install.sh` installs `nomouse-settings` beside `nomouse` and, when GTK 4 or libadwaita is
  missing, prints the package manager's command to add them (it never runs `sudo`);
  `--uninstall` also removes the settings window, the tray icons and the start-at-login entry.
  The `.deb` ships `nomouse-settings` and recommends `libgtk-4-1` and `libadwaita-1-0`.
- On Linux, when the session cannot run nomouse (GNOME on Wayland, no accessibility bus yet),
  a daemon started from the desktop now stays up with only its tray menu and D-Bus interface,
  showing why, and tries again every 30 seconds and on every config change, instead of
  exiting.
- The guided first run on macOS happens in the app: while Accessibility is missing, nomouse's
  menu bar item (dimmed) and a Welcome window explain the permission, open its pane and follow
  the grant, instead of a row of alerts.
- `just check` from a Mac or Windows cross-checks Linux without `nomouse-settings`, whose GTK
  build scripts need Linux's libraries (Linux hosts and CI still lint it); it failed on every
  Mac since the Linux settings window landed.

## [0.1.3] - 2026-11-09

### Added

- `hints.browser = "exhaustive"`: a scan reads the whole web page (browser tabs, Electron
  windows), not only what is in view, so search also finds links and buttons scrolled out of
  view; picking one scrolls it into view first (`AXScrollToVisible` on macOS,
  `Component.ScrollTo` on Linux) and acts where it lands. Hint mode still labels only what is
  in view. The default, `"fast"`, keeps reading only the viewport. The click waits until the
  scrolled element has stopped moving (browsers scroll smoothly and ignore a click meanwhile)
  and fails with an error, instead of clicking elsewhere, when it never settles in view.
- A shared settings model (`nomouse_core::settings`) for the settings windows and the daemon:
  changes are saved at once and atomically into the user's own `config.toml`, writing only the
  keys that changed and keeping comments, blank lines, key order and unknown keys. The daemon
  now notices a changed file by its content, so a save within the same second as the last one
  is not missed, and reports an invalid edit once while the last good settings stay.
- Linux: nomouse is in the app launcher. `install.sh` installs the desktop entry
  `io.github.yuyudhan.nomouse.desktop` and its icons (hicolor 16 to 512 px and a scalable SVG)
  into `~/.local/share`, refreshes the desktop and icon caches, and `--uninstall` removes them;
  the `.deb` and the tarball ship the same files. On GNOME, press Super and type `nomouse`.
- `nomouse launch`, what the entry runs: it starts the daemon unless one runs and shows a
  notification with its state and the hotkeys, and on GNOME Wayland says to log in with
  "Ubuntu on Xorg". The entry's "nomouse doctor" action (`nomouse launch --doctor`) opens
  `nomouse doctor` in a terminal, or shows the report as a notification without one.
- The command socket answers `ping`, which `nomouse launch` uses to see that a daemon it
  started is up.

## [0.1.2] - 2026-11-09

### Fixed

- The install failed on networks that cut some connections to GitHub's download host with
  `curl: (35) error:0A000126:SSL routines::unexpected eof while reading` (seen on Ubuntu 22.04,
  a third of connections): `--retry` alone never retries that. `install.sh` now retries every
  download up to five times with `--retry-all-errors` (curl 7.71+, plain `--retry` on older
  curl), the README and install guide one-liner does the same for the installer itself, and
  `scripts/test-installer.sh` installs through a server that hangs up on the first two requests
  for every file.
- On GNOME Wayland the installer printed Sway and Hyprland bindings; it now says to log in with
  "GNOME on Xorg" ("Ubuntu on Xorg" on Ubuntu) first, as `nomouse doctor` does. A failed
  download names the network instead of the old private-repository hint.

## [0.1.1] - 2026-11-09

### Fixed

- `install.sh` with `GH_TOKEN` or `GITHUB_TOKEN` stopped right after "installing the latest
  nomouse", without an error: on a release as large as a real one, its asset lookup died of
  SIGPIPE under `pipefail`. The lookup now reads the file directly, and
  `scripts/test-installer.sh` checks it against a release of that size.

## [0.1.0] - 2026-11-09

### Added

- macOS reaches the Milestone B features (4e3a367, b08a84a): the `auto` theme follows the
  system appearance, the focus highlight and the scroll-area outline are drawn, search reads
  tooltips and values, `ignore.apps` matches executable names, and the desktop end-to-end test
  covers all of it.
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
- Release pipeline: archives for Linux (x86_64, aarch64), macOS (universal, plus `nomouse.app`
  zipped and in a `.dmg`) and Windows (x86_64) with SHA-256 checksums, `.deb` packages, a
  Homebrew formula, and no-sudo installers (`install.sh`, `install.ps1`).
- `install.sh` installs `nomouse.app` into `/Applications` on macOS (quarantine removed, a
  `nomouse` command in `~/.local/bin` pointing into it) and prints each system's next step:
  the Accessibility grant, Wayland compositor bindings, the uinput rule, start at login.
  `--from DIR` installs from a folder of release assets; with `GH_TOKEN` or `GITHUB_TOKEN` it
  downloads through the GitHub API, which also works while the repository is private.
  `scripts/test-installer.sh` (in `just test` and CI) tests it against a fake release.
- User docs in `docs/`: `install.md`, `usage.md`, `configuration.md` (every key and default,
  checked against the code by a test), `troubleshooting.md` and `platform-support.md`, indexed
  in `docs/README.md`; the README starts with the install and a 30-second first use, with
  recordings from macOS and Linux.

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
