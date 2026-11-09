# Architecture

## Crates

| Crate | Role | Unsafe |
|-------|------|--------|
| `nomouse-core` | element model, hint labels, label filter, actions, hotkeys, config, platform traits, hint session | forbidden |
| `nomouse-linux` | AT-SPI elements; X11 overlay, XTest input and XGrabKey hotkeys; Wayland (wlroots) layer-shell overlay and virtual-pointer/uinput input | forbidden |
| `nomouse-macos` | macOS backend built on `nomouse-sys-macos` | forbidden |
| `nomouse-sys-macos` | safe wrappers over AXUIElement, CGEvent, AppKit | allowed, audited |
| `nomouse-windows` | Windows backend built on `nomouse-sys-windows` | forbidden |
| `nomouse-sys-windows` | safe wrappers over UI Automation COM and SendInput | allowed, audited |
| `nomouse` | the binary: CLI, config loading, event loop | forbidden |

The binary depends on exactly one backend through target-specific dependencies
(`[target.'cfg(target_os = "...")'.dependencies]`). Each platform crate compiles on every OS:
OS-specific modules and dependencies are `cfg`-gated, so clippy checks all targets from one host.

## The platform traits

Defined in `crates/nomouse-core/src/platform.rs`, split by responsibility:

- `ElementSource::elements(scope)`: a snapshot of the accessibility tree for the focused window or
  the whole screen, as `Element { id, role, name, description, value, bounds, enabled }`
  (description and value feed search; Linux reads them over AT-SPI for labelled objects only,
  and never a password field's value).
- `Overlay::show(hints, typed)` / `show_frame(frame)` / `hide()`: draws labels above all windows
  and grabs the keyboard while shown, so typed keys reach nomouse, not the app. A `Frame` adds
  the search query box, the highlighted label and the outlined scroll area.
- `InputInjector::perform(action)`: click (any button, any count), hover, drag, scroll.
- `HotkeySource::register(hotkey)` / `unregister(id)` / `next_event(timeout)`: global hotkeys and
  the key presses captured while the overlay is up (`KeyEvent`: the key plus the modifiers
  held), as `InputEvent`. A registered hotkey pressed while the overlay is up still arrives as
  `InputEvent::Hotkey`.
- `Platform: all four + name()`.

Every method is blocking and runs on the event-loop thread; errors are `PlatformError`, whose
`AccessibilityUnavailable` and `PermissionDenied` variants carry user-facing instructions.

## Data flow

1. The binary loads `Config` (TOML, defaults for every field, validated) and registers the
   hotkeys from `[hotkeys]`.
2. `next_event` returns `InputEvent::Hotkey`. The binary asks `ElementSource` for elements.
3. `HintSession::new` drops non-clickable elements, sorts the rest into reading order (top to
   bottom, left to right, then id) and assigns labels: the leaves of a breadth-first expansion of
   the alphabet, so labels are as short as possible, shortest first, and prefix-free.
4. `Overlay::show_frame` draws `session.visible()` with the highlighted hint
   (`session.focused()`). Each `InputEvent::Key` goes to `session.key`, which narrows the
   labels (`LabelFilter`), moves the highlight (Tab/↓/Ctrl-N, Shift-Tab/↑/Ctrl-P), rejects keys
   that match nothing, or yields `SessionEvent::Perform(action)` (a complete label, or Return on
   the highlight; with `hints.keys = "homerow"` a label only highlights, and Return yields
   `Armed` until the double-press time passes or a second Return makes it a double click). Drag
   takes two picks; scroll mode (core `ScrollSession`) starts on the largest scroll area,
   outlined through `Frame::area` (without one it picks an element first). Escape cancels, and
   so does an activation hotkey or a command-socket request arriving while the labels are up
   (toggle).
5. `Overlay::hide`, then `InputInjector::perform(action)`.

Hint alphabet: `asdfghjk` by default, configurable; `i`, `l`, `o` are rejected as ambiguous.

## Backends

- **Linux X11**: `atspi`/`zbus` for the tree: the focused window is the AT-SPI application
  owning `_NET_ACTIVE_WINDOW`'s `_NET_WM_PID` (its active frame plus open menus/popups),
  read with three Collection `GetMatches` queries (interactive roles, Action, focusable) and a
  bounded level-by-level walk (depth, node count, deadline, cycle-safe) where Collection is
  missing; calls are pipelined per level with a 400 ms per-call timeout. `x11rb` for an
  override-redirect ARGB overlay per RandR monitor, shaped to the label boxes (so it looks right
  without a compositor) with an empty input region (click-through); labels are tiny-skia paths
  from skrifa outlines (the shared `nomouse_core::glyph` module, feature `glyph`). `XGrabKeyboard` while shown, `XGrabKey` on the root for hotkeys (all
  Caps/Num Lock variants, layout-aware keysyms), XTest for input; `semantic_click` uses the
  element's AT-SPI action instead. 100% safe Rust.
- **Linux Wayland (wlroots)**, feature `wayland` (default): the same AT-SPI tree. AT-SPI gives
  native Wayland windows' extents relative to the window, so the focused window's process and
  each native window's origin come from the compositor's IPC (Sway `get_tree` content rect,
  Hyprland `j/clients`) and are added to that process's elements; XWayland elements are global
  already. `wayland-client` (pure Rust backend) with one `zwlr_layer_shell_v1` overlay-layer
  surface per output (`xdg-output` logical geometry), anchored to all edges, empty input
  region, `keyboard_interactivity = exclusive` while shown and destroyed when hidden; frames are
  composed into ARGB8888 and written to a memfd with `pwrite` (no `mmap`). Keys come from
  `wl_keyboard` through `xkbcommon` with the compositor's keymap and the X11 keysym table.
  Input replays the XTest plan through `zwlr_virtual_pointer_v1` (absolute motion over the
  layout box, buttons, discrete wheel) and `zwp_virtual_keyboard_v1` (modifiers, keymap copied
  from the seat), else an absolute-pointer `/dev/uinput` device (`evdev`). Hotkeys: the
  compositor binds `nomouse hints|continuous|scroll|search`, which reach the daemon over its
  Unix socket (`crates/nomouse/src/ipc.rs`, also used on X11 and macOS). GNOME (no
  layer-shell, no virtual pointer, no client hotkeys) is refused with that reason; KDE works with
  uinput input and XWayland-only placement. 100% safe Rust; `xkbcommon` and `evdev` hold the
  FFI.
- **macOS**: `AXUIElementCopyAttributeValue` over the focused app's window, a non-activating
  `NSPanel` at screen-saver level, `CGEventPost` for input, a `CGEventTap` or Carbon hotkey.
  Requires the Accessibility permission.
- **Windows**: UI Automation (`IUIAutomation::ElementFromHandle` + cached tree walk), a
  `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST` overlay, `SendInput`, `RegisterHotKey`.

## Desktop integration (Linux)

The daemon puts itself on the session bus (`nomouse_linux::service`, zbus on its own executor
thread; the event loop reads its commands without blocking and pushes state changes):

- **Control interface**: `io.github.yuyudhan.nomouse.Daemon1` at
  `/io/github/yuyudhan/nomouse/Daemon`, owned as `io.github.yuyudhan.nomouse.Daemon`
  (`nomouse_linux::control`). Methods `Pause`, `Resume`, `Reload`, `Run(mode)`, `Quit`;
  properties `Paused`, `Active` (the backend runs), `Warning` (the session problem) and
  `Version`, with change signals. The settings window and the tests use it. The command socket
  takes the same `pause`, `resume` and `quit` lines (`nomouse pause|resume|quit`).
- **Tray icon**: a freedesktop `org.kde.StatusNotifierItem` at `/StatusNotifierItem` with a
  `com.canonical.dbusmenu` menu at `/MenuBar`, registered with `org.kde.StatusNotifierWatcher`
  by the connection's unique name, and again whenever a watcher appears (a panel restarting).
  KDE, Xfce, waybar's tray module and GNOME Shell with the AppIndicator extension (enabled by
  default on Ubuntu) are such hosts. The menu is `nomouse_core::menu::status_menu`, the same
  model the macOS menu-bar item draws: the state line, the session warning, Pause/Resume, each
  mode with its hotkey (X11 only; compositors own them on Wayland), Settings…, Run doctor…,
  Start at login (an XDG autostart entry, `nomouse_linux::autostart`), Check for updates… (a
  `curl` of the latest-release redirect, `nomouse_core::update`, answered in the menu and a
  desktop notification), About and Quit. Ids are stable per action, so a host clicking an item
  of an older layout still does the right thing; a click on a disabled item does nothing. The
  icon is the themed `io.github.yuyudhan.nomouse-symbolic` (paused:
  `io.github.yuyudhan.nomouse-paused-symbolic`; sources in `assets/logo`, copied into
  `packaging/linux/share/icons/hicolor/symbolic/apps` by `scripts/render-logo.sh`), plus the
  same drawing as ARGB pixmaps
  at 16 to 48 px (`service/icon.rs`, tiny-skia) for hosts that cannot find the name.
- **No tray host** (GNOME without the extension, i3 without a tray): nothing changes for the
  hotkeys; the launcher entry opens the settings window, and `nomouse doctor` prints why there
  is no icon and how to get one.
- **Unsupported session** (GNOME on Wayland, no AT-SPI yet): when the backend cannot start but
  the bus is up, the daemon keeps running its menu and control interface only
  (`Active = false`, the reason as the warning, the icon's status `NeedsAttention`), retries
  every 30 seconds and on every configuration change, and starts the backend as soon as it
  can. Without the bus it exits with the error, as before.
- **Settings window**: `nomouse-settings` (crate `crates/nomouse-settings`, GTK 4 and
  libadwaita, [ADR 0004](adr/0004-linux-settings-window-and-tray.md)), a separate binary so the
  daemon never links GUI libraries. Its pages are a General page, the shared
  `settings::Section`s with one row per `settings::FIELDS` entry that applies on Linux, and a
  Doctor page (`nomouse doctor`'s report). Every change goes through
  `nomouse_core::settings::Settings` (saved at once, atomically, only the changed keys), then
  `Reload` on the control interface; edits of the file by hand show up in the window within a
  second (`Settings::watch`). The switch and the daemon row follow the control interface's
  `Paused` and `Active`. The hotkey recorder pauses the daemon while it listens, so the chord
  being recorded does not fire; on Wayland, where nomouse cannot grab keys
  (`nomouse_linux::shortcuts`), the page shows the compositor lines to add instead. The
  Appearance preview is drawn by the overlay's own renderer (`nomouse_linux::preview`). It is a
  single-instance `GtkApplication` (`io.github.yuyudhan.nomouse`): a second start, from the
  tray or the launcher, raises the open window on the asked `--page`.
- **Launcher entry**: `nomouse launch` (the desktop entry's `Exec`) starts the daemon unless
  it runs, then the settings window; when the window cannot open it shows the notification.
  The tray's Settings…, Run doctor… and About start `nomouse-settings` the same way
  (`launch::spawn_settings`: next to `nomouse`, else on `PATH`; errors in
  `nomouse-settings.log` beside the daemon's lock).

## Unsafe policy

The user requirement is `#![forbid(unsafe_code)]` in every crate. macOS and Windows APIs are
FFI, which Rust can only call through `unsafe`. So all such calls live in two small crates,
`nomouse-sys-macos` and `nomouse-sys-windows`, that:

- expose only safe functions and types;
- `#![deny(unsafe_op_in_unsafe_fn)]` and `#![deny(clippy::undocumented_unsafe_blocks)]`, with a
  `// SAFETY:` comment on every block;
- prefer safe wrappers (`objc2` safe APIs, `windows-rs`) and keep each unsafe block minimal.

`scripts/check-unsafe.sh` fails CI on any `unsafe` block, fn, impl, trait or extern outside these
two crates, and on any other crate root without `#![forbid(unsafe_code)]`. Linux needs no unsafe.

## App-support matrix

| App family | Linux | macOS | Windows | Note |
|------------|-------|-------|---------|------|
| Native (GTK / AppKit / Win32, WinUI) | yes | yes | yes | GTK 4 and 3 expose AT-SPI |
| Qt 5/6 | yes | yes | yes | Linux: `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1` if not detected |
| Chromium, Chrome, Edge, Electron | yes | yes | yes | Linux needs `--force-renderer-accessibility`; Electron apps may need it everywhere |
| Firefox, Thunderbird | yes | yes | yes | |
| Java (Swing/JavaFX) | with the ATK bridge | with the Java Access Bridge | with the Access Bridge | bridge must be enabled |
| Terminals, games, custom-drawn UIs | no tree | no tree | no tree | optional separate fallback, later |

Decisions behind this layout are recorded in [docs/adr/](adr/).

## Configuration

One TOML file, read at daemon start (and by every one-shot command):

| OS | Path |
|----|------|
| Linux, macOS | `$XDG_CONFIG_HOME/nomouse/config.toml`, else `~/.config/nomouse/config.toml` |
| Windows | `%APPDATA%\nomouse\config.toml` |

`--config <path>` overrides it. A missing file means all defaults; a present file is parsed
with `deny_unknown_fields` (typos are errors, not silently ignored), then validated
(`Config::validate`: alphabet, duplicate chords, numeric ranges, colours). `nomouse config
init` writes the defaults, `show` prints the effective config, `check` validates, `path`
prints the path. The schema, with every default:

```toml
[hints]
alphabet = "asdfghjk"        # label characters, most preferred first; i, l, o rejected
scope = "focused-window"     # or "screen": every visible window on every monitor
action = "click"             # click | right-click | double-click | hover | drag | scroll
semantic_click = false       # AXPress / AT-SPI action / UIA Invoke instead of a pointer click
keys = "nomouse"             # nomouse: a label acts at once | homerow: a label highlights,
                             # Return clicks, twice double clicks, Shift-Return right clicks
browser = "fast"             # fast: web pages read in view only | exhaustive: the whole page,
                             # so search finds links scrolled away (and scrolls them in)

[hotkeys]                    # modifiers: ctrl, alt (Option), shift, meta (Command / Super)
hints = "ctrl+space"
continuous = "ctrl+shift+space"
scroll = "ctrl+alt+space"
search = "ctrl+alt+/"

[scroll]
step = 3                     # wheel steps per key press, 1..=50; d/u scroll five times this
dash_step = 15               # wheel steps per Shift-h/j/k/l press, 1..=100

[overlay]
font_size = 13               # points, 6..=72
opacity = 95                 # percent, 10..=100
theme = "original"           # original (yellow), light, dark, or auto (system appearance)
# background = "#ffd75f"     # unset colours come from the theme; a set one overrides it
# foreground = "#1c1c1c"
# typed_foreground = "#8a8a8a" # already-typed characters
# border = "#1c1c1c"

[ignore]
apps = []                    # apps where activation shows nothing: macOS bundle id or
                             # executable name, Wayland app id, X11 WM_CLASS instance or
                             # class, or process name (ignoring case; a truncated 15-byte comm
                             # matches); no blanks

[macos]
enhanced_ui_apps = []        # bundle ids that get AXEnhancedUserInterface (Chromium browsers)
```

New keys land in `nomouse-core/src/config.rs` with a default, a validation rule when the type
alone does not constrain them, a unit test, an entry in `settings::FIELDS` (so the settings
windows show it), and an entry here and in the README.

The daemon holds the file as a `nomouse_core::Settings` (below) and re-reads it when its
content changes (looked at every second, whoever wrote it: a settings window or a text editor)
and on `nomouse reload` (a `reload` line on the command socket). An unreadable or invalid file
is logged once and the running configuration stays; an unchanged configuration (a comment
edited) does nothing. Otherwise the old platform is dropped first, releasing its hotkeys and
overlay, and a new one starts with the new configuration (or the previous one again if the
new one cannot start).
`ElementSource::focused_app` names the focused window's app (`AppIdentity`) for
`ignore.apps`; an ignored activation scans nothing and shows nothing.
`overlay.theme = "auto"` resolves to `light` or `dark` through `OverlayConfig::colours(appearance)`.
On Linux the appearance is the XDG desktop portal's `org.freedesktop.appearance color-scheme`
(1 is dark; anything else, or no portal within 250 ms, is light), read with `ReadOne` (or the
older `Read`) before each activation's scan, so a switch applies to the next labels. On macOS
it is `NSApp.effectiveAppearance` (Dark Aqua or not, which follows System Settings, its Auto
schedule included), read the same way before each scan. Windows uses the light colours for now.

### Settings model

`nomouse_core::settings` is the one way anything edits `config.toml`: the daemon, the macOS
settings window and menu bar, and the Linux tray and settings window all use it, so they agree
on the file and on validation. It has no threads and no OS code; callers poll it from their own
event loop.

```rust
use nomouse_core::settings::{Change, Settings, Value};

let mut file = Settings::open_default()?;        // or Settings::open(path)
file.config();                                   // &Config in effect
file.problem();                                  // Some(message) while the file is invalid
file.set("overlay.theme", Value::Text("dark".into()))?; // validate, then save at once
file.update(|config| config.scroll.step = 5)?;   // typed form of the same
file.value("hotkeys.hints")?;                    // Value::Text("ctrl+space")
file.reset()?;                                   // every setting back to its default
match file.watch() {                             // call every tick; looks once a second
    Change::Unchanged => {}
    Change::Reloaded => { /* file.config() changed: apply it */ }
    Change::Invalid(error) => { /* report it; the last good config stays */ }
}
```

| Item | Meaning |
|------|---------|
| `Settings::open(path)` / `open_default()` | Reads the file. Missing: every default. Invalid: the defaults stay in effect and `problem()` says why. Errors only when the file exists but cannot be read (or no path: `NoPath`). |
| `config()`, `problem()`, `path()` | The configuration in effect (the last good one while the file is invalid), why the file on disk is not in effect, the file. |
| `refresh()` | Re-reads the file when its content changed since the last read or write. `Change::Unchanged`, `Reloaded` (a different valid configuration is now in effect) or `Invalid(SettingsError)` (reported once per edit). |
| `watch()` | `refresh()` at most once per `WATCH_INTERVAL` (1 s); call it every loop. |
| `reload()` | Re-reads even when the content looks unchanged (`nomouse reload`), so an invalid file is reported again. |
| `update(fn(&mut Config))`, `replace(Config)` | Reads edits made meanwhile, applies the change, validates (`SettingsError::Rejected`, nothing written), then saves. Returns whether the file changed. |
| `value(key)`, `set(key, Value)` | One setting by its `section.key`, as `Value::Text` (text, a choice, a hotkey, a `#rrggbb` colour), `Number`, `Bool`, `List` (app ids) or `Unset` (an optional colour). `UnknownKey` and `WrongKind` errors. Hotkeys come back in canonical spelling (`shift+super+space`). |
| `reset()` | Removes every setting from the file; comments and unknown keys stay. |
| `settings::FIELDS`, `Section::ALL`, `Section::fields()` | Every key with its section (Hotkeys, Labels, Appearance, Scroll, Search, Ignored apps, Advanced), title, help and `Kind` (`Text`, `Choice(&[Choice])`, `Number { min, max }`, `Toggle`, `Hotkey`, `Colour`, `Apps`), plus `only_on` (`Some("macos")` for `macos.enhanced_ui_apps`). A test checks they list exactly the keys `Config` has, and that every choice and number limit matches validation. |

How a save writes the file:

- Only the keys whose value the change touched are written, through `toml_edit` into the
  user's own document: a value is replaced in place (its trailing comment stays), a value set
  back to its default is removed, and a table left empty goes with its comments moved to the
  next table. Untouched keys stay as written, even when they hold the default. Comments, blank
  lines, key order, inline and dotted tables, and keys this version does not know are kept.
- A file that is not TOML at all is never written (`SettingsError::Syntax`); an invalid but
  well-formed one (an unknown key, a value out of range) takes the change and keeps reporting
  its problem.
- Atomic: a temporary file in the same directory, flushed, given the old file's permissions,
  then renamed over it. A symbolic link (dotfiles) is followed, so the link stays.

## Logging

`tracing` to stderr, level from `NOMOUSE_LOG` (`error`, `warn` (default), `info`, `debug`,
`trace`), ANSI colours only on a terminal. The daemon run by launchd, systemd or XDG autostart
inherits stderr into that service manager's log (`log show`, `journalctl --user`). Rule for new
log lines: element names and typed keys only at `debug` or below, because they can be private.

## Permissions and onboarding

`nomouse doctor` is the single source of truth: every check is `(name, Ok(detail) |
Err(problem and fix))`, printed as `[ok  ]`/`[FAIL]` lines after `platform:`, and the command
exits non-zero when any check fails. When the daemon cannot start, the backend's
`PlatformError::AccessibilityUnavailable` or `PermissionDenied` carries the same fix text.

- **macOS**: Accessibility trust (`AXIsProcessTrustedWithOptions`, prompting once), whether
  synthetic input is allowed, Input Monitoring state (informational). The grant is tied to the
  code signature, so a stable signed `nomouse.app` is a prerequisite for a smooth first run
  (NM-DIST-02); the guided first run (NM-APP-10) opens the Accessibility pane
  (`x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility`), waits for
  the grant by polling trust, then restarts itself.
- **Linux**: display server and compositor, the AT-SPI bus, the `org.a11y.Status.IsEnabled`
  switch (the daemon turns it on), the compositor's Wayland globals, `/dev/uinput` access when
  needed. Fixes that need root (the uinput udev rule) are printed, never run.

The guided first run (NM-APP-10) is `nomouse_core::setup`: a step machine over the backend's
permission list and a `Probe` (is it granted, ask the OS for it) that explains each missing
permission, asks once, polls every 500 ms for up to 10 minutes, and reports whether a
*required* permission was granted on the way. `nomouse setup` walks through every permission
in the terminal. The daemon walks through the required ones before it starts, in a terminal
when it has one and otherwise in the backend's dialogs (macOS `NSAlert`, as when
`nomouse.app` is opened), then replaces itself with a fresh start (`exec`) once they are
granted. macOS: Accessibility (required; the request shows macOS's prompt, which lists nomouse
in the pane, and opens the pane). Linux: the AT-SPI switch (optional, the daemon's start also
turns it on). Windows: nothing to ask.

## Packaging

| Target | Artifact | State |
|--------|----------|-------|
| macOS | `install.sh` installs `nomouse.app` (from its zip) into `/Applications` with a `nomouse` command in `~/.local/bin`; the universal binary tarball ships as well | done |
| macOS | `nomouse.app` (LSUIElement agent, bundle id `com.yuyudhan.nomouse`, icon, version from `Cargo.toml`), ad-hoc signed, zipped: `scripts/release/app.sh` | done |
| macOS | `nomouse.app` in a `.dmg` with an `/Applications` link (`scripts/release/dmg.sh`, written by `hdiutil makehybrid`, so nothing is mounted) | done (ad-hoc signed) |
| macOS | Developer ID signed with hardened runtime, notarized and stapled (`signing.sh`, `notarize.sh` in `release.yml`) | wired; waits for the owner's certificate and App Store Connect key as secrets |
| macOS | Homebrew cask `nomouse-app.rb` rendered with every release, for `Casks/nomouse.rb` in `yuyudhan/homebrew-tap` (the formula stays for the CLI) | rendered; published once a release has the `.dmg` |
| Linux | x86_64/aarch64 tarballs + `install.sh`, `.deb` (cargo-deb) | done |
| Linux | `.rpm` (cargo-generate-rpm), AUR `nomouse-bin`, AppImage; each ships the uinput udev rule and an XDG autostart entry | planned |
| Windows | zip + `install.ps1` | done |

All artifacts come from `.github/workflows/release.yml` on a tag; nothing is built or signed on a
developer machine.
