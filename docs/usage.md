# Using nomouse

nomouse runs as a small background daemon. A hotkey starts a **mode**; the mode draws labels
over the elements of the focused window, you type, and it acts. Escape always cancels.

On macOS, nomouse.app runs in the menu bar: its keycap icon's menu starts each mode, pauses
nomouse and opens the settings ([Menu bar and settings](#menu-bar-and-settings)).

On Linux, nomouse is in the app launcher: press Super (the Windows key), type `nomouse` (or
`mouse`, `click`, `hints`) and pick it. That starts the daemon unless it already runs and opens
the settings window, which shows its state, the hotkeys and, on GNOME Wayland, to log in with
**Ubuntu on Xorg** instead. Right-click the entry for **nomouse doctor**. The tray icon's menu
pauses and resumes nomouse, starts each mode and opens the settings.

- [Hints: click anything](#hints-click-anything)
- [Other clicks: right, double, hover, drag](#other-clicks-right-double-hover-drag)
- [Modifier clicks: cmd-click, shift-click](#modifier-clicks-cmd-click-shift-click)
- [Continuous mode](#continuous-mode)
- [Scroll mode](#scroll-mode)
- [Search mode](#search-mode)
- [Homerow key bindings](#homerow-key-bindings)
- [Menu bar and settings](#menu-bar-and-settings)
- [Commands](#commands)

## Hints: click anything

Press `ctrl+space`. Every clickable element in the focused window (buttons, links, tabs, menu
items, text fields, checkboxes) gets a label of one or two letters from the home row, in the
middle of the element (kept on screen when the element reaches past its edge).

| Key | Effect |
|-----|--------|
| label letters | narrow the labels to those starting with what you typed; a complete label clicks its element and the labels go away |
| Backspace | remove the last typed letter |
| Tab, ↓, Ctrl-N | move the highlight to the next label (the first, when none is highlighted) |
| Shift-Tab, ↑, Ctrl-P | move the highlight to the previous label (the last, when none is highlighted) |
| Return | click the highlighted label; nothing is highlighted when the labels appear, so a stray Return does nothing until Tab, an arrow or a typed letter picks one |
| `1` `2` `3` `4` | hold Shift, Ctrl, Alt (Option) or Super (Command) for the next click |
| Escape, or `ctrl+space` again | cancel |

Labels never use `i`, `l` or `o`, which look like digits, and stay as short as they can: with
the default alphabet `asdfghjk`, up to eight elements get one letter each, and twelve get
`s d f g h j k aa as ad af ag` (`nomouse labels 12` prints them).

With `hints.scope = "screen"` in the [config](configuration.md), labels cover every visible
window on every monitor, not only the focused one. With `hints.semantic_click = true`, nomouse
presses elements through the accessibility API where they allow it, so the pointer stays where
it is.

## Other clicks: right, double, hover, drag

A mode can do something other than a left click. Run it from a hotkey your compositor or a
launcher binds, or from a terminal:

```sh
nomouse hints --action right-click
nomouse hints --action double-click
nomouse hints --action hover          # move the pointer there, no click
nomouse hints --action drag           # pick the start, then the end
```

Set `hints.action` in the config to change what `ctrl+space` does.

## Modifier clicks: cmd-click, shift-click

While the labels are up, the digits `1` to `4` toggle a modifier held during the next click:

| Digit | Modifier | Typical use |
|-------|----------|-------------|
| `1` | Shift | extend a selection |
| `2` | Ctrl | Linux and Windows: open a link in a new tab |
| `3` | Alt (Option) | download a link in a browser |
| `4` | Super (Command) | macOS: open a link in a new tab |

So `ctrl+space`, `4`, then a link's label opens the link in a background tab on macOS. Pressing a
digit again turns its modifier off. Modifiers you hold while pressing Return are held for the
click too.

## Continuous mode

`ctrl+shift+space` labels the window, clicks, then labels it again, until you press Escape or
click into a text field (so you can type there). Use it to work through a dialog or a form
without pressing the hotkey for every click.

## Scroll mode

`ctrl+alt+space` outlines the largest scrollable area and numbers every other one.

| Key | Effect |
|-----|--------|
| `j` `k` (or ↓ ↑) | scroll down and up `scroll.step` wheel steps (default 3) |
| `h` `l` (or ← →) | scroll left and right |
| `J` `K` `H` `L` (Shift) | scroll `scroll.dash_step` steps at a time (default 15) |
| `d` `u` | scroll down and up five times as far |
| `gg` | go to the top |
| `G` | go to the bottom |
| Tab, Shift-Tab, or a digit | switch to another area |
| Escape or Enter | end scroll mode |

## Search mode

`ctrl+alt+/` opens a search: type part of an element's text and the labels narrow to the
matching elements, best match first.

| Key | Effect |
|-----|--------|
| letters, digits, spaces | the query; spaces are ignored and matching is fuzzy and case-insensitive |
| `*` or `;` alone | match every element |
| Enter | click the highlighted match (the best one unless you moved the highlight) |
| Tab | freeze the query; letters now type labels, to pick another match |
| Backspace | edit the query (with no label typed after Tab, go back to the query) |
| Escape | cancel |

Search matches an element's name, its tooltip or description, a text field's contents, a
slider's value, and its role, so `button` or `check box` find those. Password fields are never
searched.

On a long web page, search normally sees only what is in view. With
`hints.browser = "exhaustive"` it reads the whole page (Linux and macOS), so it also finds a
link or button you would have to scroll to; picking it scrolls it into view and clicks it
there. On macOS, Chromium browsers need their bundle id in `macos.enhanced_ui_apps`, which lists
the common ones by default.

## Homerow key bindings

If your fingers know [Homerow](https://www.homerow.app/), set `hints.keys = "homerow"`: a
complete label then only highlights its element, and

| Key | Effect |
|-----|--------|
| Return | click |
| Return twice | double click |
| Shift-Return | right click |
| Cmd-Return (Super-Return) | cmd-click |

## Menu bar and settings

On macOS, nomouse.app is a menu bar app: no Dock icon, a keycap icon in the menu bar (dimmed
while paused or unable to run). Its menu:

| Item | Does |
|------|------|
| `nomouse 0.1.4: on` | the state: on, paused, or why it cannot run |
| `Accessibility: allowed` | the permission; while it is missing the icon is a warning sign and `Accessibility: not allowed: click to allow` removes a stale entry left by an older version, asks for it and opens its Settings pane (Privacy & Security › Accessibility, Device Control and Data Access on macOS 27) |
| Pause nomouse / Resume nomouse | release the hotkeys until resumed |
| Show Hints, Continuous Hints, Scroll, Search | start the mode; the hotkey in effect is shown at the right |
| Settings… (`cmd+,`) | the settings window |
| Run doctor… | `nomouse doctor`'s report in a window, with a Copy button |
| Start at login | registers nomouse.app as a login item (System Settings › General › Login Items) |
| Check for updates… | asks GitHub for the latest release; when it is newer, its window shows the [one-line install](install.md#the-one-line-install) (`curl -fsSL https://yuyudhan.github.io/nomouse/install \| sh`): **Copy Install Command** puts it on the clipboard to paste into Terminal, **Open Release Page** opens the release. Use the command rather than the `.dmg` there, which a browser quarantines; then allow nomouse again ([why](install.md#updating)) |
| About nomouse, Quit nomouse (`cmd+q`) | |

The settings window has these pages:

| Page | Settings |
|------|----------|
| General | Accessibility status, start at login, updates, the doctor, About |
| Hotkeys | one recorder per mode: click it and press the chord; a chord macOS already uses (Spotlight, input sources, screenshots) is flagged, Escape keeps the old one |
| Labels | the label letters, the label keys (Homerow bindings), what picking a label does, which windows get labels |
| Appearance | theme (auto, light, dark, original), label size and opacity, the four colours, with a live preview |
| Scroll | the step and the Shift step |
| Search | web pages: `fast` (what is in view) or `exhaustive` (the whole page) |
| Ignored apps | apps where the hotkeys show no labels; **Add App…** picks them from `/Applications` |
| Advanced | press through accessibility, Chromium and Electron apps, **Open Config File**, **Show in Finder**, **Reset to Defaults…** (asks first) |

Each change is saved to `~/.config/nomouse/config.toml` at once (only what differs from the
defaults; your comments stay) and the daemon applies it within a second. A file edited by hand
meanwhile shows up in the open window; an invalid edit is named on the page and the last good
settings stay in effect.

On Linux the tray icon's menu and `nomouse-settings` do the same (see the
[README](../README.md#menu-bar-and-settings)); their update notification names the one-line
install. Start at login there writes an XDG autostart entry; on Sway, Hyprland and i3, which
ignore those, the Start at login row shows the config line to add instead
([install.md](install.md#linux-x11)).

## Commands

Every mode is also a command. When a daemon runs, the command hands the mode to it over its
socket (`$XDG_RUNTIME_DIR/nomouse.sock`), which is how Wayland compositors bind hotkeys; with no
daemon, the mode runs in the command's own process.

| Command | Does |
|---------|------|
| `nomouse` or `nomouse daemon` | start the daemon: registers the hotkeys and waits |
| `nomouse hints [--action A] [--continuous]` | hints mode once |
| `nomouse continuous [--action A]` | continuous mode |
| `nomouse scroll` | scroll mode |
| `nomouse search` | search mode |
| `nomouse reload` | make the daemon re-read the config now |
| `nomouse pause` / `resume` | release the hotkeys (and ignore the mode commands) until resumed, as the menu bar or tray menu's Pause does |
| `nomouse quit` | stop the daemon |
| `nomouse setup` | walk through the permissions nomouse needs, ask the system, wait for them |
| `nomouse launch [--doctor]` | Linux, what the app launcher runs: start the daemon unless it runs and open the settings window (without it: notify the state and hotkeys); `--doctor` opens `nomouse doctor` in a terminal |
| `nomouse-settings [--page <page>]` | Linux, the settings window, on `general`, `hotkeys`, `labels`, `appearance`, `scroll`, `search`, `ignored`, `advanced` or `doctor`; a second start raises the open window |
| `nomouse doctor` | name the flavour (release, or a development build) and its config directory, check the session, accessibility, input, the running daemon (pid and version) and the config, and print the fix for anything missing |
| `nomouse config init` / `path` / `show` / `check [FILE]` | [configuration](configuration.md) helpers |
| `nomouse labels COUNT [--alphabet LETTERS]` | print the labels COUNT elements would get |
| `nomouse --version` | print the version (`(dev)` after it for a development build) |

`--standalone` before or after a mode runs it in the calling process even when a daemon runs.
Set `NOMOUSE_LOG=info` or `debug` for logs on stderr.
