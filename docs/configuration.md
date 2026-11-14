# Configuration

nomouse works without a config file. To change something, create one:

```sh
nomouse config init     # writes every default to the config file
nomouse config path     # prints where it is
nomouse config show     # prints the settings in effect (the file, or the defaults)
nomouse config check    # validates the file and names the first bad key
```

The file is `~/.config/nomouse/config.toml` on macOS and Linux (`$XDG_CONFIG_HOME/nomouse/config.toml`
when `XDG_CONFIG_HOME` is set) and `%APPDATA%\nomouse\config.toml` on Windows.

A development build (`cargo run`, any debug build, `just dev` or `just app`) has its own
config, `~/.config/nomouse-dev/config.toml`, so it never changes the installed nomouse's
settings. `just dev-config` installs `config/dev.example.toml` there: every key set to a
development build's default, with comments giving each one's values and effect. It keeps an
existing file unless you pass `--force`, which backs it up as `config.toml.bak-<time>` first.

Every key is optional: leave out what you do not change. The running daemon re-reads the file
within two seconds of a save, or at once with `nomouse reload`. A file with a mistake is
reported in the daemon's log and the previous settings stay in effect, so a typo never leaves
you without hotkeys. Unknown keys are errors, which catches misspellings.

The settings window (the menu bar's **Settings...** on macOS; on Linux the app launcher's
nomouse entry, the tray menu's **Settings…** or `nomouse-settings`) edits this same file and saves
each change at once. It writes only the keys you change, removes a key you set back to its
default, and keeps your comments, blank lines and key order, so editing by hand and in the
window mix freely. Whichever wrote it last, the daemon picks the change up within a second.

## A typical file

```toml
[hints]
alphabet = "fjdksagh"         # home-row letters, index fingers first
keys = "homerow"              # a label highlights its element; Return clicks

[hotkeys]
hints = "cmd+shift+space"
search = "cmd+shift+/"

[overlay]
theme = "auto"                # light or dark labels, following the system
font_size = 15

[ignore]
apps = ["com.mitchellh.ghostty", "kitty"]
```

## Every key

The tables list every key nomouse reads, with its default. A test
(`crates/nomouse-core/tests/config_reference.rs`) compares them with the configuration code on
every build, so this page cannot fall behind it.

### `[hints]`: the labels

| Key | Default | Values |
|-----|---------|--------|
| `hints.alphabet` | `"asdfghjk"` | the label letters, most preferred first: at least 2 distinct lowercase ASCII letters, without `i`, `l` and `o` (they look like `1` and `0`) |
| `hints.scope` | `"focused-window"` | `"focused-window"` labels the window you are in; `"screen"` labels every visible window on every monitor |
| `hints.action` | `"click"` | what picking a label does: `"click"`, `"right-click"`, `"double-click"`, `"hover"`, `"drag"` (pick the start, then the end) or `"scroll"` (pick an area, then scroll it) |
| `hints.semantic_click` | `false` | `true` presses the element through the accessibility API (AT-SPI `do_action`, `AXPress`, UI Automation `Invoke`) when it offers that, so the pointer does not move; `false` moves the pointer and clicks |
| `hints.keys` | `"nomouse"` | `"nomouse"`: typing a complete label acts at once. `"homerow"`: a complete label only highlights; Return clicks, Return twice double-clicks, Shift-Return right-clicks, Cmd-Return (Super-Return) cmd-clicks |
| `hints.browser` | `"fast"` | how much of a web page (a browser tab, an Electron window) a scan reads. `"fast"`: only what is in view. `"exhaustive"`: the whole document, so search also finds links and buttons scrolled out of view, and picking one scrolls it into view first. Hint mode labels only what is in view either way. Linux and macOS |
| `hints.early_rescan_ms` | `1000` | labels show at once for everything already there; while nothing is typed, elements that appeared since get labels this many milliseconds after the hotkey (a page still loading). Labels already shown never change. `0` turns this stage off; at most `10000` |
| `hints.late_rescan_ms` | `3000` | the last stage, for slow content (a browser building its page's accessibility tree right after nomouse switched it on): later than `hints.early_rescan_ms`, `0` turns it off, at most `10000`. Typing a label or closing the labels stops the stages early |

### `[hotkeys]`: the global shortcuts

The defaults below are the release's. A development build defaults to `ctrl+alt+shift+space`
(hints), `ctrl+alt+shift+period` (continuous), `ctrl+alt+shift+comma` (scroll) and
`ctrl+alt+shift+slash` (search), so it never fights the installed nomouse over a chord.

| Key | Default | Mode it starts |
|-----|---------|----------------|
| `hotkeys.hints` | `"ctrl+space"` | hints: label the window, type a label to click |
| `hotkeys.continuous` | `"ctrl+shift+space"` | continuous: label again after every click, until Escape or a click into a text field |
| `hotkeys.scroll` | `"ctrl+alt+space"` | scroll: pick a scrollable area, then scroll it with keys |
| `hotkeys.search` | `"ctrl+alt+slash"` | search: type part of an element's text (`ctrl+alt+/` is the same hotkey) |

A hotkey is one or more modifiers and a key joined by `+`, case-insensitive:

- modifiers: `ctrl` (or `control`), `alt` (or `option`), `shift`, `super` (or `cmd`,
  `command`, `meta`, `win`). At least one is required.
- keys: a letter `a`-`z`, a digit, `f1`-`f24`, `space`, `tab`, `enter` (or `return`), `escape`
  (or `esc`), `backspace`, and `/` `;` `,` `.` (or `slash`, `semicolon`, `comma`, `period`).

Two modes cannot share a hotkey. On Wayland the compositor owns global shortcuts, so these keys
are not grabbed there: bind `nomouse hints` (and the other modes) in the compositor instead, as
[install.md](install.md#linux-wayland) shows.

### `[scroll]`: scroll mode

| Key | Default | Values |
|-----|---------|--------|
| `scroll.step` | `3` | wheel steps per `h` `j` `k` `l` (or arrow) press, 1 to 50 |
| `scroll.dash_step` | `15` | wheel steps per Shift with `H` `J` `K` `L`, 1 to 100 |

`d` and `u` always scroll five times `scroll.step`.

### `[overlay]`: how labels look

| Key | Default | Values |
|-----|---------|--------|
| `overlay.font_size` | `13` | label text size in points, 6 to 72 |
| `overlay.opacity` | `95` | label opacity in percent, 10 to 100 |
| `overlay.theme` | `"original"` | `"original"` (yellow labels, dark text), `"light"`, `"dark"`, or `"auto"` (light or dark, following the system appearance: the macOS appearance, or the XDG portal `color-scheme` on Linux) |
| `overlay.background` | unset | `"#rrggbb"`: label background, overriding the theme's |
| `overlay.foreground` | unset | `"#rrggbb"`: label text |
| `overlay.typed_foreground` | unset | `"#rrggbb"`: the label letters you have already typed |
| `overlay.border` | unset | `"#rrggbb"`: label border |

An unset colour comes from the theme, so `theme = "auto"` with only `border` set still switches
the rest between light and dark.

### `[ignore]`: apps without labels

| Key | Default | Values |
|-----|---------|--------|
| `ignore.apps` | `[]` | apps where the hotkeys show no labels. Each entry is a macOS bundle id (`com.apple.Terminal`) or executable name, a Wayland app id, an X11 `WM_CLASS` instance or class, or a process name; matching ignores case. No blank entries |

`nomouse doctor` lists the apps it ignores.

### `[macos]`: macOS only

| Key | Default | Values |
|-----|---------|--------|
| `macos.enhanced_ui_apps` | `["com.google.Chrome", "com.google.Chrome.beta", "com.google.Chrome.dev", "com.google.Chrome.canary", "org.chromium.Chromium", "company.thebrowser.Browser", "com.brave.Browser", "com.microsoft.edgemac", "com.vivaldi.Vivaldi", "com.operasoftware.Opera"]` | bundle ids whose accessibility tree nomouse switches on with `AXEnhancedUserInterface`, checked before every activation (a relaunched browser turns it off). The default covers Chrome (stable, Beta, Dev, Canary), Chromium, Arc, Brave, Edge, Vivaldi and Opera. A list in the file replaces the default, so to add an Electron app copy the default and append its bundle id (`osascript -e 'id of app "Slack"'` prints it); `[]` turns it off. The attribute can slow window animations, which is why it is per app. No blank entries. Ignored on other systems |

```toml
[macos]
# The default browsers plus two Electron apps.
enhanced_ui_apps = [
  "com.google.Chrome", "com.google.Chrome.beta", "com.google.Chrome.dev", "com.google.Chrome.canary",
  "org.chromium.Chromium", "company.thebrowser.Browser", "com.brave.Browser", "com.microsoft.edgemac",
  "com.vivaldi.Vivaldi", "com.operasoftware.Opera",
  "com.microsoft.VSCode", "com.tinyspeck.slackmacgap",
]
```

`nomouse doctor` checks every running instance of each Chromium browser and listed app for web
content: it passes when any instance shows a page, reports blank tabs as "no page open", and
otherwise says what to change. It also validates the file, including the `hints.*_rescan_ms`
stages (in range, in order).

## Logs

Set `NOMOUSE_LOG=info` (or `debug`) in the environment the daemon starts from to log to stderr,
including why a reloaded file was rejected.
