# nomouse

Keyboard-driven clicking for Linux, macOS and Windows, in the spirit of
[Homerow](https://www.homerow.app/): press a global hotkey, hint labels appear on every clickable
element of the focused window (or the whole screen), type a label to click it. Right-click,
double-click, hover, drag, modifier clicks, a scroll mode and search by element text are built in.

Elements come from each OS's accessibility tree, so nomouse works with every app that exposes
one: native apps, GTK, Qt, Chromium and Electron (with accessibility on), Firefox, and Java apps
with the accessibility bridge. Apps without an accessibility tree (some terminals, games) are not
supported; a clearly separate fallback may come later.

## Status

Pre-alpha. Done: the platform-independent core (`nomouse-core`) and the Linux X11 backend with
the full CLI: `nomouse daemon` (the default; global hotkeys `ctrl+space` hints,
`ctrl+shift+space` continuous, `ctrl+alt+space` scroll mode, `ctrl+alt+/` search),
`nomouse hints [--continuous] [--action <kind>]`, `nomouse scroll`, `nomouse search`,
`nomouse doctor`, `nomouse config path|show|init|check` and
`nomouse labels <n>`. Set `NOMOUSE_LOG=info` (or `debug`) for logs. The Linux backend also runs
on wlroots Wayland compositors (Sway, Hyprland; see [Wayland](#wayland)). macOS and Windows
backends are in progress. See [docs/roadmap.md](docs/roadmap.md). `scripts/e2e-linux.sh` runs
the Linux end-to-end test on a private Xvfb display, `scripts/e2e-wayland.sh` on a private
headless Sway.

## Keys while labels are shown

| Key | Effect |
|-----|--------|
| label letters | narrow the labels; a full label acts on its element |
| Backspace | remove the last typed character |
| Escape | cancel |
| `1` / `2` / `3` / `4` | toggle Shift / Ctrl / Alt / Super (Command on macOS, Windows key on Windows) for the next click, e.g. `2` then a label is ctrl+click (cmd+click is `4`): open a link in a new tab |

**Search mode** (`ctrl+alt+/`, `nomouse search`): type part of an element's name (fuzzy,
case-insensitive); labels appear only on matches, best match first, with the query in a box near
the top of the screen. Enter clicks the best match; Tab freezes the query so label letters (and
the modifier digits) pick another match; Backspace with no label typed returns to the query.

**Scroll mode** labels the scrollable areas (AT-SPI scroll panes, AX scroll areas, UI Automation
elements with the Scroll pattern) rather than the controls inside them, falling back to the
clickable elements when an app reports none; after picking one, `j`/`k`/`h`/`l` scroll by
`scroll.step` wheel steps, `d`/`u` by five times that, Escape or Enter ends it.

## Platforms and the accessibility requirement

| OS | Elements | Overlay | Input | What you must enable |
|----|----------|---------|-------|----------------------|
| Linux X11 | AT-SPI (D-Bus) | override-redirect windows | XTest | AT-SPI running (GNOME/KDE do this); `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1` for Qt; `--force-renderer-accessibility` for Chromium/Electron |
| Linux Wayland (wlroots) | AT-SPI | layer-shell | virtual-pointer or uinput | as X11, plus `uinput` access when the compositor lacks virtual-pointer |
| macOS | AXUIElement | NSPanel | CGEvent | System Settings > Privacy & Security > Accessibility: allow nomouse |
| Windows | UI Automation | layered windows | SendInput | nothing; run elevated to reach elevated apps |

## Wayland

nomouse runs natively on wlroots compositors (Sway 1.7+, Hyprland). `nomouse doctor` lists what
the compositor offers and what does not work, and why.

- **Overlay**: one `zwlr_layer_shell_v1` surface per output on the overlay layer, click-through
  (empty input region) and keyboard-exclusive while labels are up.
- **Input**: `zwlr_virtual_pointer_v1` plus `zwp_virtual_keyboard_v1` (for modifier clicks) when
  the compositor offers them; otherwise an absolute pointer on `/dev/uinput`. Your user needs
  write access to `/dev/uinput` for that; check with `ls -l /dev/uinput` (a `+` means an ACL is
  set, as `uaccess` does) and, if needed, add the udev rule (needs root, once):

      echo 'KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"' \
          | sudo tee /etc/udev/rules.d/70-nomouse-uinput.rules \
          && sudo udevadm control --reload && sudo udevadm trigger

- **Hotkeys**: Wayland gives clients no global hotkeys, so the compositor binds the commands, and
  `nomouse hints|continuous|scroll|search` hand the mode to the running `nomouse daemon` over
  `$XDG_RUNTIME_DIR/nomouse.sock` (without a daemon they run in-process). Sway:

      exec nomouse daemon
      bindsym Ctrl+space exec nomouse hints
      bindsym Ctrl+Shift+space exec nomouse continuous
      bindsym Ctrl+Mod1+space exec nomouse scroll
      bindsym Ctrl+Mod1+slash exec nomouse search

  Hyprland: `exec-once = nomouse daemon` and `bind = CTRL, space, exec, nomouse hints`. The same
  commands work on X11, where the daemon also grabs its own hotkeys; `--standalone` runs a mode
  in the calling process even when a daemon runs.
- **Coordinates**: AT-SPI reports native Wayland windows' elements relative to the window (a
  Wayland client does not know where its window is). XWayland apps report global coordinates
  and work fully. For native GTK/Qt windows nomouse adds the window's position from the
  compositor's IPC (Sway `get_tree`, Hyprland `j/clients`). Limits: a native app's popup menus
  are placed relative to their own surface, and with several windows of one native app on
  screen the focused one is the one placed correctly.
- **GNOME Wayland**: Mutter implements neither layer-shell nor virtual pointer, and Shell offers
  clients no hotkeys: nomouse refuses with that explanation; use "GNOME on Xorg".
  **KDE Plasma Wayland**: layer-shell works, input goes through `/dev/uinput`, the hotkey is a
  custom shortcut running `nomouse hints`; KWin has no window-position IPC, so only XWayland
  apps are placed correctly (doctor reports it).

`scripts/e2e-wayland.sh` runs the Wayland end-to-end test on a private headless Sway.

## Install

Each release ships archives for Linux (x86_64, aarch64), macOS (one universal binary) and
Windows (x86_64), each with a `.sha256`, plus `.deb` packages and a Homebrew formula. The
installers need no administrator rights; they verify the checksum and install for the current
user only.

Linux and macOS (into `~/.local/bin`):

    curl -fsSL https://github.com/yuyudhan/nomouse/releases/latest/download/install.sh | bash

Windows (PowerShell, into `%LOCALAPPDATA%\Programs\nomouse`, added to the user PATH):

    irm https://github.com/yuyudhan/nomouse/releases/latest/download/install.ps1 | iex

Options go after `bash -s --` (or run `install.ps1` as a file): `--version X.Y.Z`,
`--uninstall`, `--uninstall --purge` (also deletes the config); on Windows `-Version`,
`-Uninstall`, `-Purge`. `NOMOUSE_INSTALL_DIR` changes the install folder. Then run
`nomouse doctor`.

While the repository is private, release downloads need a GitHub login, so these one-liners work
only once it is public; until then download the assets with `gh release download`.

## Building

Rust (toolchain pinned in `rust-toolchain.toml`) and [just](https://github.com/casey/just):

    just verify      # format check, guards, typos, clippy for linux/windows/macos targets, tests, cargo-deny
    cargo run -p nomouse -- doctor

See [CONTRIBUTING.md](CONTRIBUTING.md) for the quality rules and
[docs/architecture.md](docs/architecture.md) for the design.

## Licence

Not yet decided; the repository is private until it is.
