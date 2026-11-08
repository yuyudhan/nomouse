# Architecture

## Crates

| Crate | Role | Unsafe |
|-------|------|--------|
| `nomouse-core` | element model, hint labels, label filter, actions, hotkeys, config, platform traits, hint session | forbidden |
| `nomouse-linux` | AT-SPI elements; X11 overlay, XTest input and XGrabKey hotkeys (Wayland: M4) | forbidden |
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
  the whole screen, as `Element { id, role, name, bounds, enabled }`.
- `Overlay::show(hints, typed)` / `hide()`: draws labels above all windows and grabs the keyboard
  while shown, so typed keys reach nomouse, not the app.
- `InputInjector::perform(action)`: click (any button, any count), hover, drag, scroll.
- `HotkeySource::register(hotkey)` / `unregister(id)` / `next_event(timeout)`: global hotkeys and
  the key presses captured while the overlay is up, as `InputEvent`.
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
4. `Overlay::show` draws `session.visible()`. Each `InputEvent::Key` goes to `session.key`,
   which narrows the labels (`LabelFilter`), rejects keys that match nothing, or yields
   `SessionEvent::Perform(action)`. Drag takes two picks; scroll picks a target then enters scroll
   mode. Escape cancels.
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
