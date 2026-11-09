# nomouse-macos

The macOS backend of nomouse: it reads clickable elements through the Accessibility API
(`AXUIElement`), draws hint labels in click-through `NSPanel`s, catches hotkeys and typed keys
with a Quartz event tap, and clicks, drags, hovers and scrolls with synthetic `CGEvent`s.
All FFI lives in `nomouse-sys-macos`; this crate is `#![forbid(unsafe_code)]`.

## Permissions

| Permission | Needed for | How to grant |
| --- | --- | --- |
| **Accessibility** | Reading other apps' UI, performing `AXPress`, the keyboard event tap (an active tap filters keys, which macOS allows only for trusted processes), posting clicks | System Settings > Privacy & Security > Accessibility: add and enable `nomouse.app` (or the `nomouse` binary, or the terminal it runs from). `nomouse setup`, and the daemon's first start, explain it (dialogs when there is no terminal), show macOS's prompt, open that pane, wait for the switch and restart the daemon. |
| Input Monitoring | Not needed: the active tap runs under Accessibility. `nomouse doctor` still reports its state. | System Settings > Privacy & Security > Input Monitoring |

Granting Accessibility to a terminal (Terminal, iTerm2) also covers anything started from it,
which is convenient while testing. A rebuilt binary at a new path needs granting again.

`nomouse doctor` lists the macOS version, Accessibility trust, whether synthetic input is
allowed, the Input Monitoring state and the Chromium/Electron opt-in list.

## Behaviour

- **Scope `focused-window`**: the frontmost app's focused window, its open menus, its menu bar
  (top-level items only) and every app's menu bar extras (status items).
  **Scope `screen`**: every non-minimized window of every regular app, plus the same menus.
- **Tree walk**: depth-first, cycle-safe (elements compared with `CFEqual`), limited to depth
  48, 4000 nodes and 600 ms. Tables and outlines use `AXVisibleRows`; windows and web areas
  prefer `AXChildrenInNavigationOrder`. Subtrees outside the visible part of their window or
  scroll area are skipped. Each Accessibility call waits at most 250 ms for a busy app.
- **Hinted roles**: buttons, links, check boxes, radio buttons (tabs inside tab groups), pop-up
  buttons, combo boxes, menu items and menu bar items, text fields and areas, sliders and
  steppers, rows, standalone cells, disclosure triangles, plus any group, image or web element
  that lists `AXPress`.
- **Chromium and Electron**: nomouse sets `AXManualAccessibility` on the frontmost app once per
  process (Electron builds its tree only when asked). Chrome, Edge, Brave and similar need
  `AXEnhancedUserInterface`, which can slow window animations, so it is opt-in per app:

  ```toml
  [macos]
  enhanced_ui_apps = ["com.google.Chrome", "com.microsoft.VSCode"]
  ```

  The tree appears a moment after the attribute is set, so the very first activation in such
  an app may find few elements; activate again.
- **Hotkeys** on a character key match the key that types that character on the current
  keyboard layout (re-matched when the layout changes); characters no key types alone (digits
  on AZERTY, letters on Cyrillic) keep their ANSI position. Modifiers are exact; `meta` is
  Command and `alt` is Option. The default `ctrl+space` is also macOS's "select previous input
  source" shortcut: nomouse's tap sees it first and swallows it while nomouse runs. F21 to F24
  do not exist on Mac keyboards and are rejected.
- **Semantic click** (`hints.semantic_click = true`): a left click on a labelled element runs
  `AXPress`, a right click `AXShowMenu`; when the element refuses, nomouse clicks the pointer.
- Coordinates are points in the global top-left space Accessibility and Quartz share; labels
  are drawn at each screen's backing scale.

## Running the integration test

`tests/macos_accessibility.rs` opens Finder, collects elements, shows and hides the overlay and
hovers an element. It skips (and passes) when the test binary is not trusted for
Accessibility, which is the case on GitHub's runners. To run it for real, grant your terminal
Accessibility, then:

```sh
cargo test -p nomouse-macos --test macos_accessibility
```

`just e2e-macos` (`scripts/e2e-macos.sh`) drives the real binary against a Swift fixture app
and checks clicks, right click, double click, drag, scroll mode, search, focus-then-act, the
Homerow keys, continuous mode, the `auto` theme, `ignore.apps` with `nomouse reload` and the
daemon through Accessibility; see [docs/testing.md](../../docs/testing.md#end-to-end-on-macos).

## Manual test checklist

### Setup

1. Build: `cargo build --release` (the binary is `target/release/nomouse`).
2. Grant Accessibility to the terminal you run it from (System Settings > Privacy & Security >
   Accessibility), then fully quit and reopen that terminal.
3. Create the config file: `target/release/nomouse config init` writes the defaults to
   `~/.config/nomouse/config.toml` (`$XDG_CONFIG_HOME/nomouse/` when set; `nomouse config path`
   prints it).
   For the Chromium/Electron steps, add:

   ```toml
   [macos]
   enhanced_ui_apps = ["com.google.Chrome", "com.microsoft.VSCode"]
   ```

   and check the file with `target/release/nomouse config check`.
4. `target/release/nomouse doctor`: every line should be `ok`.
5. Start the daemon: `target/release/nomouse` (or `nomouse daemon`); leave it running.
   Default hotkeys: hints `ctrl+space`, continuous hints `ctrl+shift+space`, scroll
   `ctrl+alt+space`, search `ctrl+alt+/`. `nomouse hints` shows hints once without the daemon.

### Steps

For each app below press the hints hotkey, type a label, and confirm the result.

1. **Permissions**: on a fresh grant list, the first start shows the macOS Accessibility prompt
   and `nomouse doctor` reports Accessibility as failing; after granting and restarting, it
   passes.
2. **TextEdit**: open a new document. Hints cover the toolbar/format bar controls, the text area
   and the menu bar items (File, Edit, ...). Picking the text area puts the caret there; picking
   `Format` opens the menu, and a second activation labels the open menu's items.
3. **Finder**: a window in list view. Hints sit on the sidebar items, toolbar buttons and the
   visible rows only (scroll a long folder: rows that scrolled away get no hints, new ones do).
   Disclosure triangles expand folders. Right-click mode opens the context menu of a row.
   Scroll mode scrolls the list in the direction typed.
4. **Safari**: a page with many links (for example a news site). Links, buttons, the address
   field and tabs are labelled; picking a link navigates. Labels stay inside the window.
5. **Chrome**: without `macos.enhanced_ui_apps`, web content may have no hints; add
   `com.google.Chrome`, restart nomouse, activate twice, and links in the page get hints.
6. **VS Code** (Electron): the activity bar, editor tabs and explorer rows get hints thanks to
   `AXManualAccessibility`; if not, add `com.microsoft.VSCode` to `enhanced_ui_apps`.
7. **Menu bar extras**: the clock, Control Center and third-party status items are labelled;
   picking one opens it.
8. **Multiple screens** (if available): with `scope = "screen"`, labels appear on every screen
   at the right places, including a screen arranged above or left of the main one.
9. **Keyboard**: while hints are shown, typed keys do not reach the app; Escape hides the
   hints; Backspace removes the last typed character. After hiding, typing reaches the app
   normally.
10. **Double click and drag**: double-click mode selects a word in TextEdit; drag mode moves a
    Finder icon to another folder.
11. **Secure input**: with the daemon running, focus a password field (for example the
    login field in Safari, or Terminal with Secure Keyboard Entry turned on). macOS disables
    nomouse's event tap while secure input is on, so the hotkey does nothing there. Leave the
    field (or turn Secure Keyboard Entry off), press the hotkey again: hints appear without
    restarting nomouse (the tap re-enables itself).
12. **Scroll amount**: in a long Finder list or Safari page, press the scroll hotkey, pick the
    target, press `j` once: the content moves by `scroll.step` lines (default 3). Set
    `[scroll] step = 10` in the config, restart the daemon, and `j` now moves clearly further;
    `k` scrolls back the same amount, `h`/`l` scroll sideways.
13. **Robustness**: activate while an app is busy (for example during a large Finder copy):
    hints appear within about a second with whatever could be read, and nomouse stays
    responsive.
