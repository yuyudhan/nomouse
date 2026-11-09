# nomouse

**Click anything on screen without touching the mouse.** A free, open-source take on
[Homerow](https://www.homerow.app/) for macOS and Linux, written in Rust.

Press a hotkey and every button, link, tab and field in the focused window gets a short label.
Type the label, and nomouse clicks it. Right-click, double-click, drag, scroll and search by text
work the same way.

## Demo

<!-- TODO: record docs/demo.gif (hint mode in Finder and Firefox, then search and scroll mode). -->
*A short screen recording goes here.*

## Why nomouse

| | nomouse | Homerow |
|---|---|---|
| Price and source | free, open source | paid, closed source |
| macOS | yes | yes |
| Linux (X11; Wayland on Sway, Hyprland, KDE) | yes | no |
| Labels on every clickable element | yes | yes |
| Search by text, best match first | yes | yes |
| Scroll mode with `h` `j` `k` `l`, `d` `u` | yes | yes |
| Right-click, double-click, cmd-click | yes | yes |
| Drag and hover | yes | not documented |
| Continuous mode (Homerow's click chain) | yes | yes |
| Click without moving the pointer (accessibility action) | yes, opt-in | not documented |
| Labels on every window of every monitor | yes, opt-in | not documented |
| Plain-text config you can keep in your dotfiles | `~/.config/nomouse/config.toml` | settings window |
| Menu bar icon, ignored apps, label themes | [planned](docs/plan.md) | yes |

Every feature, with how it works on each OS and what is still missing, is in
[docs/features.md](docs/features.md).

## Install

Release archives come with a `.sha256` and a no-sudo installer that puts `nomouse` in
`~/.local/bin`.

**macOS** (Apple silicon and Intel, one universal binary):

    curl -fsSL https://github.com/yuyudhan/nomouse/releases/latest/download/install.sh | bash

A signed `nomouse.app` and a Homebrew cask are [on the way](docs/plan.md).

**Linux** (x86_64, aarch64):

    curl -fsSL https://github.com/yuyudhan/nomouse/releases/latest/download/install.sh | bash

or the `.deb` from the [releases page](https://github.com/yuyudhan/nomouse/releases) on Debian
and Ubuntu. rpm, AUR and AppImage packages are planned.

Installer options go after `bash -s --`: `--version X.Y.Z`, `--uninstall`, `--uninstall --purge`
(also deletes the config). `NOMOUSE_INSTALL_DIR` changes the install folder. A Windows build and
`install.ps1` exist as well; see [CONTRIBUTING.md](CONTRIBUTING.md).

While the repository is private, release downloads need a GitHub login; use
`gh release download` until it is public.

Then run:

    nomouse setup      # walks you through the permissions nomouse needs
    nomouse doctor     # checks permissions and prints the platform backend
    nomouse            # starts the daemon with the default hotkeys

## Keys

| Hotkey (default) | Mode |
|------------------|------|
| `ctrl+space` | hints: label the focused window, type a label to click |
| `ctrl+shift+space` | continuous: keep labelling after every click until Escape or a click into a text field |
| `ctrl+alt+space` | scroll: label scrollable areas, then scroll with keys |
| `ctrl+alt+/` | search: type part of an element's text |

While labels are shown:

| Key | Effect |
|-----|--------|
| label letters | narrow the labels; a complete label acts on its element |
| Tab, ↓, Ctrl-N / Shift-Tab, ↑, Ctrl-P | move the highlight to the next / previous label |
| Return | act on the highlighted label (it starts on the first); modifiers held with Return are held for the click |
| `1` `2` `3` `4` | hold Shift / Ctrl / Alt / Command (Super) for the next click; `4` then a label is cmd+click |
| Backspace | remove the last typed letter |
| Escape, or the mode's hotkey again | cancel |

With `hints.keys = "homerow"` a complete label highlights its element instead of acting: then
Return clicks, Return twice double clicks, Shift-Return right clicks and Cmd-Return (Super)
cmd-clicks, as in Homerow.

In search mode, Enter clicks the highlighted match (the best one unless moved), Tab freezes the
query so letters pick a label, and Backspace with no label typed returns to the query. Search
matches names, tooltips and descriptions, text field contents and roles (`button`); spaces are
ignored, and `*` or `;` alone matches everything. Scroll mode starts on the largest scrollable
area, outlined, with every area numbered: `j` `k` `h` `l` (or the arrows) scroll, Shift with
them scrolls `dash_step` wheel steps at a time, `d` and `u` scroll five times as far, `gg`
goes to the top and `G` to the bottom, Tab and Shift-Tab (or a digit) switch areas, and
Escape or Enter ends it.

Every mode is also a command (`nomouse hints`, `continuous`, `scroll`, `search`), with
`--action right-click|double-click|hover|drag|scroll` for one-off actions. On Wayland the
compositor binds these commands and they reach the running daemon: see
[docs/wayland.md](docs/wayland.md).

## Configuration

`~/.config/nomouse/config.toml` on macOS and Linux (`$XDG_CONFIG_HOME` is honoured). Every key
is optional; `nomouse config init` writes the defaults, `nomouse config check` validates. The
running daemon picks up an edited file within two seconds, or at once with `nomouse reload`;
an invalid file is reported in its log and the previous settings stay.

```toml
[hints]
alphabet = "asdfghjk"        # label letters, most preferred first
scope = "focused-window"     # or "screen" for every window on every monitor
action = "click"             # click, right-click, double-click, hover, drag, scroll
semantic_click = false       # press via the accessibility API; the pointer stays put
keys = "nomouse"             # or "homerow": a label highlights, Return clicks

[hotkeys]                    # modifiers: ctrl, alt (Option), shift, meta (Command / Super)
hints = "ctrl+space"
continuous = "ctrl+shift+space"
scroll = "ctrl+alt+space"
search = "ctrl+alt+/"

[scroll]
step = 3
dash_step = 15

[overlay]
font_size = 13
opacity = 95
theme = "original"          # original (yellow), light, dark, or auto (the system appearance)
# background = "#ffd75f"    # each colour overrides the theme's when set:
# foreground = "#1c1c1c"    # background, foreground, typed_foreground, border

[ignore]
apps = ["org.wezfurlong.wezterm", "kitty"]   # no labels in these apps: bundle id, Wayland
                                             # app id, X11 WM_CLASS or process name

[macos]
enhanced_ui_apps = ["com.google.Chrome"]   # Chromium browsers build their tree only on request
```

The full schema with ranges is in [docs/architecture.md](docs/architecture.md#configuration).
Set `NOMOUSE_LOG=info` (or `debug`) for logs on stderr.

## Permissions

nomouse reads the screen through the accessibility tree, so it works with every app that exposes
one: native apps, GTK, Qt, Firefox, and Chromium or Electron apps with accessibility on.
Terminals and games without a tree are not supported yet.

| OS | Elements | What to enable |
|----|----------|----------------|
| macOS | AXUIElement | System Settings › Privacy & Security › Accessibility: allow `nomouse` (or the terminal you run it from). `nomouse setup`, or the first start of `nomouse.app`, opens that pane, waits for the switch and restarts nomouse |
| Linux X11 | AT-SPI over D-Bus | nothing on GNOME and KDE; `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1` for Qt apps, `--force-renderer-accessibility` for Chromium and Electron |
| Linux Wayland | AT-SPI | as X11; on compositors without a virtual pointer (KDE), write access to `/dev/uinput`. GNOME Wayland is not supported yet: use GNOME on Xorg |

`nomouse doctor` checks each of these and prints the fix for anything missing.

## Roadmap

Done: the core, Linux X11 and Wayland (wlroots), the macOS backend, release archives and the
`.deb`. Next, in order: a macOS end-to-end harness, a signed `nomouse.app` with a guided first
run and a Homebrew cask, layout-aware keys on macOS, then Homerow parity (focus-then-act,
richer search and scroll, ignored apps, themes, menu bar and tray, launch at login) and rpm,
AUR and AppImage packages. Further out: a grid mode for apps without a tree, and GNOME Wayland.
The slices and their acceptance tests are in [docs/plan.md](docs/plan.md).

## Contributing

    rustup show && cargo install just taplo-cli typos-cli cargo-deny --locked
    just install-hooks
    just check && just test

The rules (no unsafe outside the two FFI crates, clippy pedantic with `-D warnings`, tests with
every change) are in [CONTRIBUTING.md](CONTRIBUTING.md); the design is in
[docs/architecture.md](docs/architecture.md), the test layers in
[docs/testing.md](docs/testing.md), and the rules for coding agents in [AGENTS.md](AGENTS.md).

## License

Not decided yet. The repository stays private until it is.
