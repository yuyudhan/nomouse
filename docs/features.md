# Features

The complete feature inventory of nomouse, measured against [Homerow](https://www.homerow.app/)
(its [README](https://github.com/nchudleigh/homerow/blob/main/README.md) and
[changelog](https://www.homerow.app/changelog), up to 1.5.3) and the open-source prior art. Each
feature has a stable id (`NM-<area>-<nn>`; ids are never reused or renumbered), how it works on
each OS, its priority and its status in this repository.

- **Priority**: P0 is needed for a first public release that replaces Homerow for daily use;
  P1 is parity with Homerow; P2 goes beyond it or serves a narrow audience.
- **Status**: **done** (implemented and tested on that OS), **partial** (the note says what is
  missing), **missing**. Status reflects the `nightly` branch; [plan.md](plan.md) schedules every
  feature that is not done.
- **Windows** is a third backend that already exists (UI Automation, layered windows,
  `SendInput`); it is listed in the status column but not planned beyond keeping CI green.

## Prior art

| Project | OS | Model | What nomouse takes from it |
| --- | --- | --- | --- |
| [Homerow](https://www.homerow.app/) | macOS, closed source | AX labels, search, scroll mode, menu bar app | the product model and its key bindings |
| [Vimac](https://github.com/dexterleng/vimac) | macOS, archived | AX hint mode and scroll mode | AX tree walking and its role filters |
| [Shortcat](https://shortcat.app/) | macOS, closed source | search-first: type text, pick a match | search ranking and "match everything" queries |
| [warpd](https://github.com/rvaiya/warpd) | X11, Wayland, macOS | hint, grid and normal (keyboard cursor) modes without accessibility | the grid fallback for apps with no tree |
| [keynav](https://github.com/jordansissel/keynav) | X11 | grid bisection | grid narrowing keys |
| [hints](https://github.com/AlfredoSequeida/hints) | Linux (GTK, AT-SPI) | AT-SPI hint labels, scroll | AT-SPI quirks per toolkit |
| [wl-kbptr](https://github.com/moverest/wl-kbptr) | wlroots | layer-shell labels and floating grid | Wayland overlay and virtual-pointer use |
| rofi/dmenu pickers over AT-SPI | Linux | element list in a launcher | search as a list (not planned: overlay instead) |

## Mechanisms per OS

| Concern | macOS | Linux X11 | Linux Wayland |
| --- | --- | --- | --- |
| Elements | `AXUIElement` tree (Accessibility permission) | AT-SPI2 over D-Bus (`org.a11y.Bus`, Collection `GetMatches`) | AT-SPI2; native window origin from the compositor IPC (Sway `get_tree`, Hyprland `j/clients`) |
| Overlay | non-activating `NSPanel` at screen-saver level, click-through | override-redirect ARGB window per RandR monitor, shaped, empty input region | `zwlr_layer_shell_v1` overlay layer per output (wlroots, KDE); none on GNOME (needs a Shell extension) |
| Input | `CGEventPost`; `AXPress`/`AXShowMenu` for semantic click | XTest; AT-SPI `do_action` for semantic click | `zwlr_virtual_pointer_v1` + `zwp_virtual_keyboard_v1`, else `/dev/uinput`; libei/RemoteDesktop portal for GNOME and KDE (planned) |
| Hotkeys | `CGEventTap` (active tap, under Accessibility) | `XGrabKey` on the root window | compositor binds `nomouse hints`, which reaches the daemon socket; GlobalShortcuts portal where offered (planned) |
| Keys while shown | the same event tap swallows keys | `XGrabKeyboard` | layer surface with exclusive keyboard interactivity |
| Permissions | Accessibility (prompted); Input Monitoring not needed | AT-SPI enabled (`org.a11y.Status.IsEnabled`), Qt/Chromium opt-ins | as X11, plus `/dev/uinput` access without virtual pointer |

Compositor limits on Wayland: Sway and Hyprland support everything; KDE Plasma has layer-shell
but no virtual pointer (uinput) and no window positions (XWayland apps only); GNOME has
neither layer-shell, virtual pointer nor client hotkeys, so nomouse refuses there today.

## Inventory

### Hint mode (HINT)

| Id | Feature | macOS | Linux | Pri | Status (mac / X11 / Wayland / Windows) |
| --- | --- | --- | --- | --- | --- |
| NM-HINT-01 | Global hotkey shows labels on every clickable element of the focused window | AX walk of the frontmost app's focused window | AT-SPI app owning the active window | P0 | done / done / done / done |
| NM-HINT-02 | Typing a label clicks its element; labels narrow as you type; Backspace, Escape | event tap → `HintSession` | key grab → `HintSession` | P0 | done / done / done / done |
| NM-HINT-03 | Prefix-free, shortest-first labels from a configurable home-row alphabet; ambiguous letters rejected | core | core | P0 | done (all) |
| NM-HINT-04 | Labels in reading order, placed on the element without covering its neighbours | core + per-screen backing scale | core | P0 | done (all) |
| NM-HINT-05 | Scope `screen`: every visible window on every monitor | all regular apps' windows | all AT-SPI apps' visible frames | P0 | done (all) |
| NM-HINT-06 | Multi-monitor and mixed-DPI placement | `NSScreen` frames, backing scale | one overlay per RandR monitor | P0 | done / done / done (per output) / done (per-monitor DPI) |
| NM-HINT-07 | Menu bar items and menu bar extras (status items) get labels | `AXMenuBar`, `AXExtrasMenuBar` | panel apps via AT-SPI where exposed | P1 | done / partial (depends on the panel) / partial / done (taskbar, tray) |
| NM-HINT-08 | Open menus and popups get labels | AX menus | AT-SPI popups | P0 | done (all) |
| NM-HINT-09 | Continuous mode (Homerow "click chain"): re-label after every click until Escape | core loop | core loop | P1 | done (all) |
| NM-HINT-10 | Activation hotkey pressed again while shown dismisses (toggle) | event tap | key grab / socket | P1 | partial (core, X11 hotkey and Wayland socket done and e2e-tested; macOS and Windows deliver the hotkey but are untested) / done / done / partial |
| NM-HINT-11 | Focus-then-act: Tab/↓/Ctrl-N and Shift-Tab/↑/Ctrl-P move a highlight across labels; Return acts on it | core | core | P1 | partial (keys work everywhere; the highlight is drawn on Linux only) / done / done / partial |
| NM-HINT-12 | Tutor: `?` shows the focused element's role, name and searchable properties | AX attributes | AT-SPI attributes | P2 | missing |
| NM-HINT-13 | Mission Control: label window thumbnails while Mission Control is open; optional auto-activate | Dock AX tree | n/a | P2 | missing |
| NM-HINT-14 | Browser modes: fast (visible viewport) and exhaustive (whole document) | `AXEnhancedUserInterface`, `AXManualAccessibility` | Chromium `--force-renderer-accessibility` | P1 | partial (opt-in list `macos.enhanced_ui_apps`; no fast/exhaustive switch) |

### Actions (ACT)

| Id | Feature | macOS | Linux | Pri | Status (mac / X11 / Wayland / Windows) |
| --- | --- | --- | --- | --- | --- |
| NM-ACT-01 | Left click | `CGEvent` | XTest / virtual pointer / uinput | P0 | done (all) |
| NM-ACT-02 | Right click | `CGEvent` | same | P0 | done (all) |
| NM-ACT-03 | Double click | `CGEvent` with click state 2 | same | P0 | done (all) |
| NM-ACT-04 | Modifier click (shift, ctrl, alt, cmd/super), e.g. cmd+click to open a link in a new tab | flags on the event | XTest key events / virtual keyboard | P0 | done (digits `1`-`4`, and modifiers held with Return) |
| NM-ACT-05 | Hover (move the pointer only) | `CGEvent` move | pointer motion | P1 | done (all) |
| NM-ACT-06 | Drag: pick start, pick end | down, moves, up | same | P1 | done (all) |
| NM-ACT-07 | Semantic click without moving the pointer | `AXPress`, `AXShowMenu` | AT-SPI `do_action` | P1 | done (all; `hints.semantic_click`) |
| NM-ACT-08 | Middle click | `CGEvent` other button | button 2 | P2 | partial (backends inject `MouseButton::Middle`; no `ActionKind` binds it) |
| NM-ACT-09 | Homerow key bindings as an option: Return click, Return twice double click, Shift-Return right click, Cmd-Return cmd click | core | core | P1 | partial (`hints.keys = "homerow"`; macOS e2e pending; Windows reports Shift only, so no Cmd-Return) / done / done / partial |
| NM-ACT-10 | Focus a text field and leave the keyboard there (no chain click into inputs) | `AXFocused` | AT-SPI `grab_focus` | P1 | partial (core: continuous mode ends after a left click on a text field, on every backend; macOS e2e pending) / done / done / done |

### Search (SRCH)

| Id | Feature | macOS | Linux | Pri | Status (mac / X11 / Wayland / Windows) |
| --- | --- | --- | --- | --- | --- |
| NM-SRCH-01 | Search hotkey: type part of an element's name; labels only on matches, best first | core `search::rank` | same | P0 | done (all) |
| NM-SRCH-02 | Fuzzy, case-insensitive ranking (word starts, substrings, then subsequences) | core | core | P0 | done (all) |
| NM-SRCH-03 | Enter clicks the best match; Tab freezes the query to pick by label | core | core | P0 | done (all) |
| NM-SRCH-04 | Spaces ignored in queries; `*` and `;` match every element | core | core | P1 | done (all) |
| NM-SRCH-05 | Search over roles, descriptions and values, not only names | AX description/value/help | AT-SPI description, Text and Value interfaces | P1 | partial (roles everywhere; the macOS walk reads no AXDescription, AXHelp or AXValue yet) / done / done / partial (roles only) |
| NM-SRCH-06 | Search and hint mode in one activation (type letters to search, hold Shift to type a label) | core | core | P2 | missing |

### Scroll (SCRL)

| Id | Feature | macOS | Linux | Pri | Status (mac / X11 / Wayland / Windows) |
| --- | --- | --- | --- | --- | --- |
| NM-SCRL-01 | Scroll mode labels scrollable areas (digits) and scrolls with `h`/`j`/`k`/`l`; without one, pick an element to scroll | `AXScrollArea` + wheel `CGEvent` | AT-SPI scroll panes + wheel buttons / discrete axis | P0 | done (all) |
| NM-SCRL-02 | Page scroll `d`/`u` (and Space) | wheel × 5 | same | P0 | done (all) |
| NM-SCRL-03 | `gg` top, `G` bottom | many wheel steps or `AXScrollToVisible` on first/last child | 500 wheel steps (core `scroll::EDGE_STEPS`; X11 waits up to 1 s for a held Shift to be released, since wheel events carry it) | P1 | partial (core; not verified on macOS) / done / done / partial (core; not verified) |
| NM-SCRL-04 | Dash scroll: Shift-`hjkl` at a separate, configurable speed | wheel | wheel; `scroll.dash_step` | P1 | partial (core; not verified on macOS) / done / done / partial (core; not verified) |
| NM-SCRL-05 | Tab/Shift-Tab cycle scroll areas; digits `0`-`9` jump to one; the largest is preselected | core | core `ScrollSession` | P1 | partial (core; not verified on macOS) / done / done / partial (core; not verified) |
| NM-SCRL-06 | Smooth (pixel) scrolling instead of wheel notches | `kCGScrollEventUnitPixel` | `AXIS_VALUE120` / virtual pointer axis | P2 | missing |
| NM-SCRL-07 | Highlight border around the active scroll area | overlay | `Frame::area`, outlined by the shared renderer | P1 | partial (the macOS overlay draws no outline yet) / done / done / missing |

### Hotkeys and keys (KEY)

| Id | Feature | macOS | Linux | Pri | Status (mac / X11 / Wayland / Windows) |
| --- | --- | --- | --- | --- | --- |
| NM-KEY-01 | Configurable chords for hints, continuous, scroll, search; duplicates rejected | event tap | `XGrabKey` / compositor binding | P0 | done (all) |
| NM-KEY-02 | Wayland: compositor binds `nomouse hints` etc., which reach the daemon socket | n/a | socket in `$XDG_RUNTIME_DIR` | P0 | n/a / done / done / n/a |
| NM-KEY-03 | Layout-aware hotkeys and labels (Dvorak, Colemak, AZERTY, non-Latin layouts) | `UCKeyTranslate` per input source | xkb keysyms (done) | P1 | partial (macOS matches ANSI key positions) / done / done / done |
| NM-KEY-04 | Modifier-only shortcuts (e.g. double-tap Right Command) | flags-changed events in the tap | XInput2 raw modifier events | P2 | missing |
| NM-KEY-05 | Hyper key friendly (cmd+ctrl+alt+shift chords recorded and matched) | tap | key grab | P2 | partial (chords with all four modifiers parse; untested with Hyperkey/Karabiner) |
| NM-KEY-06 | GlobalShortcuts portal on Wayland (KDE, GNOME 48+) | n/a | `org.freedesktop.portal.GlobalShortcuts` | P2 | missing |

### Appearance (LOOK)

| Id | Feature | macOS | Linux | Pri | Status (mac / X11 / Wayland / Windows) |
| --- | --- | --- | --- | --- | --- |
| NM-LOOK-01 | Label colours, border, opacity, font size | `[overlay]` | `[overlay]` | P0 | done (all) |
| NM-LOOK-02 | Named themes (`light`, `dark`, `original`) and `auto` following the system appearance | `NSApp.effectiveAppearance` | `org.freedesktop.appearance color-scheme` portal setting | P1 | partial: `overlay.theme` and explicit-colour overrides in the core; Linux `auto` reads the portal (`ReadOne`, else `Read`) before each activation; macOS and Windows `auto` use the light colours until their appearance is read |
| NM-LOOK-03 | Search box showing the query | overlay | overlay | P0 | done (all) |
| NM-LOOK-04 | Sound feedback on click and on a rejected key (off by default) | `NSSound` | `canberra`-free: a bundled WAV through PipeWire/PulseAudio | P2 | missing |

### App behaviour (APP)

| Id | Feature | macOS | Linux | Pri | Status (mac / X11 / Wayland / Windows) |
| --- | --- | --- | --- | --- | --- |
| NM-APP-01 | Background daemon, single instance, command socket | daemon + socket | daemon + socket | P0 | done (all; Windows without socket) |
| NM-APP-02 | `nomouse doctor`: platform backend, permissions, display server, compositor abilities, config validity | AX trust, synthetic input, Input Monitoring | display server, AT-SPI bus and switch, compositor globals, uinput | P0 | done (all) |
| NM-APP-03 | Config in `~/.config/nomouse/config.toml` (`$XDG_CONFIG_HOME` honoured) with defaults for every key, validated, `config init|path|show|check` | XDG path | XDG path | P0 | done (Windows: `%APPDATA%`) |
| NM-APP-04 | Logging: `NOMOUSE_LOG=info|debug` to stderr | `tracing` | `tracing` | P0 | done (all) |
| NM-APP-05 | Ignored apps: no activation in listed apps (bundle id / app id / process name) | frontmost bundle id | `WM_CLASS` and `/proc/<pid>/comm` (X11); compositor app id, `WM_CLASS` (Sway, Hyprland) | P1 | partial (bundle id read; not verified on macOS) / done / done (Sway, Hyprland) / missing |
| NM-APP-06 | Per-app settings overrides (alphabet, action, scope, semantic click) | bundle id | app id | P2 | partial (only `macos.enhanced_ui_apps`) |
| NM-APP-07 | Config reload without restarting the daemon (file watch or `nomouse reload`) | mtime watch / socket | mtime watch / socket | P1 | partial (shared code; not run on macOS) / done / done / partial (file watch only, not run) |
| NM-APP-08 | Menu bar / tray icon: enabled state, open config, doctor, quit | `NSStatusItem` | StatusNotifierItem (KDE, GNOME with the AppIndicator extension) | P1 | missing |
| NM-APP-09 | Launch at login | `SMAppService` (needs the `.app`) | XDG autostart `.desktop`, systemd user unit | P1 | missing |
| NM-APP-10 | First-run onboarding: explain and request each permission, then verify with a live probe | AX prompt + System Settings deep link | enable `org.a11y.Status.IsEnabled`, explain Qt/Chromium flags and uinput | P0 | partial (prompt and doctor exist; no guided first run) |
| NM-APP-11 | Settings window (GUI over `config.toml`) | SwiftUI-free: AppKit via `objc2` | GTK 4 or a TOML-editing TUI | P2 | missing |
| NM-APP-12 | Update check (notify only; package managers update) | GitHub releases API | same | P2 | missing |

### Fallbacks for apps without an accessibility tree (FALL)

| Id | Feature | macOS | Linux | Pri | Status |
| --- | --- | --- | --- | --- | --- |
| NM-FALL-01 | Grid mode: split the screen into a labelled grid, narrow recursively, click (warpd, keynav) | overlay + `CGEvent` | overlay + XTest / virtual pointer | P2 | missing |
| NM-FALL-02 | Normal mode: move the pointer with `hjkl`, click with keys (warpd) | `CGEvent` | XTest / virtual pointer | P2 | missing |
| NM-FALL-03 | GNOME Wayland via a Shell extension (overlay, input, hotkeys inside Shell) | n/a | GJS extension talking to the daemon over D-Bus | P2 | missing |
| NM-FALL-04 | KDE/GNOME input through libei (RemoteDesktop portal) instead of uinput | n/a | `reis` crate, portal session | P2 | missing |

### Distribution (DIST)

| Id | Feature | macOS | Linux | Pri | Status |
| --- | --- | --- | --- | --- | --- |
| NM-DIST-01 | Release archives with checksums and a no-sudo installer | universal binary | x86_64, aarch64 tarballs | P0 | done |
| NM-DIST-02 | Signed and notarized `nomouse.app` (stable Accessibility grant across updates) in a `.dmg` | `codesign --options runtime`, `notarytool`, `stapler` | n/a | P0 | missing (needs the owner's Developer ID) |
| NM-DIST-03 | Homebrew: cask for the `.app` | `yuyudhan/homebrew-tap` | n/a | P0 | partial (a formula for the bare binary) |
| NM-DIST-04 | `.deb` | n/a | cargo-deb | P0 | done |
| NM-DIST-05 | `.rpm` | n/a | cargo-generate-rpm | P1 | missing |
| NM-DIST-06 | AUR `nomouse-bin` | n/a | PKGBUILD from the release tarball | P1 | missing |
| NM-DIST-07 | AppImage | n/a | linuxdeploy (bundles nothing: AT-SPI and X11 come from the host) | P2 | missing |
| NM-DIST-08 | udev rule and autostart files shipped in the Linux packages | n/a | `/usr/lib/udev/rules.d/70-nomouse-uinput.rules`, `/etc/xdg/autostart/nomouse.desktop` | P1 | missing |

## Counts

| Priority | Features | Done | Partial | Missing |
| --- | --- | --- | --- | --- |
| P0 | 29 | 26 | 2 | 1 |
| P1 | 25 | 5 | 15 | 5 |
| P2 | 17 | 0 | 3 | 14 |
| **All** | **71** | **31** | **20** | **20** |

A feature counts as done when it is done on macOS and Linux X11 (Wayland limits are listed per
row). The three open P0 items are all macOS distribution and onboarding: the signed and
notarized `.app` (NM-DIST-02), the Homebrew cask (NM-DIST-03) and the guided first run
(NM-APP-10).
