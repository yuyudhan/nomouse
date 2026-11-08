# nomouse-windows

The Windows backend of nomouse. It implements the `nomouse-core` platform traits:

| Trait | How |
| --- | --- |
| `ElementSource` | UI Automation (`CUIAutomation8`, falling back to `CUIAutomation`) in a single-threaded COM apartment. One `FindAllBuildCache` per window with a cache request that batches bounding rectangle, control type, name, `IsOffscreen`, `IsEnabled`, `IsKeyboardFocusable` and the Invoke / Toggle / SelectionItem / ExpandCollapse / LegacyIAccessible patterns. |
| `Overlay` | A layered, click-through, topmost tool window (`WS_EX_LAYERED \| WS_EX_TRANSPARENT \| WS_EX_TOPMOST \| WS_EX_TOOLWINDOW \| WS_EX_NOACTIVATE`) updated with `UpdateLayeredWindow` from a premultiplied ARGB buffer. While it is shown a low-level keyboard hook swallows keys (modifiers pass through) and delivers them to the session. |
| `InputInjector` | `SendInput` with absolute virtual-desktop coordinates: click, right/double click, hover, drag (press, eight intermediate moves, release), wheel and horizontal wheel scrolling. `Backend::modifier_click` clicks with ctrl/alt/shift/win held. With `hints.semantic_click = true`, a left click on a labelled element uses its pattern (Invoke, Toggle, Select, Expand/Collapse, then LegacyIAccessible default action) and falls back to the pointer if the pattern fails. |
| `HotkeySource` | `RegisterHotKey` (with `MOD_NOREPEAT`), delivered through the thread's message pump. |

Which elements get labels: buttons, hyperlinks, check boxes, radio buttons, menu items, tab items,
list items, tree items, edits, combo boxes, sliders, split buttons, data and header items, plus any
other element offering an activation pattern or keyboard focus (except whole windows, panes and
documents). Offscreen and zero-sized elements are dropped, and in screen scope so are elements
covered by a window above them.

The process opts into per-monitor DPI awareness (v2), so every coordinate is in physical pixels;
labels are scaled by the DPI of the monitor they sit on. Labels are drawn with Segoe UI Bold (or
another font from `%WINDIR%\Fonts`) through `ab_glyph` and `tiny-skia`, the same approach as the
Linux backend, in the `[overlay]` colours, size and opacity.

Scans are bounded like the Linux backend's: at most 5000 elements across all windows and a
1.5 s budget (`ScanLimits`, adjustable with `Backend::set_scan_limits`). The budget is also the
UIA connection and transaction timeout (via `IUIAutomation2`), so a hung app cannot stall a
scan; once a limit is hit, no further window is queried and `Backend::last_truncation` says
which limit cut the scan short.

While labels are shown, modifiers pass through to the app; when a letter was typed with Alt
or Win held, an unassigned key (`0xE8`) is injected before that modifier's release, so the app
does not see a lone Alt (menu bar) or Win (Start menu).

Scopes: `focused-window` is the foreground window plus any open popup menus. `screen` is every
visible, non-cloaked window, the taskbar and the notification area; `ScopeOptions` (passed to
`platform_with`) can drop the taskbar or just the tray.

All unsafe code lives in `nomouse-sys-windows`; this crate is `#![forbid(unsafe_code)]` and its
logic is written against the `System` trait, tested with a fake on every OS. On Windows,
`tests/winforms.rs` launches a small WinForms window, reads its button through UI Automation and
clicks it through the backend.

## Manual test

Run these on a real Windows 10/11 desktop (`cargo run -p nomouse` from a normal, non-elevated
terminal), with the default hotkey `ctrl+space` unless configured otherwise.

1. **Notepad.** Open Notepad, press the hotkey. Labels appear on the menu bar items (File, Edit,
   View...), the tabs (Windows 11) and the text area. Type the label of *File*: the menu opens.
   Press the hotkey again: the open menu's items get labels; pick *Save as* and the dialog opens.
   Type into the text area's label: the caret lands there. Escape hides the labels without a click.
2. **Explorer.** Open File Explorer. Labels cover the navigation pane tree items, the address bar,
   the ribbon/command bar buttons and every visible file. Pick a folder in the tree: it opens.
   Try right click (`hints.action = "right-click"` or the CLI action): the context menu opens at
   the item. Try double click on a file: it opens. With `scope = "screen"`, the taskbar buttons,
   the start button and the tray icons get labels too.
3. **Edge.** Open Edge on a busy page (a news site). Links, buttons and form fields get labels;
   picking a link navigates. With `semantic_click = true` links are invoked without moving the
   pointer. Scroll mode on the page scrolls `scroll.step` notches per key press (the binary
   applies the step; the backend sends whole notches); drag a tab out
   of the tab strip with the drag action.
4. **VS Code.** Open VS Code (Electron/Chromium). If few labels appear, enable the screen reader
   mode prompt or set `"editor.accessibilitySupport": "on"`; then the activity bar, explorer tree,
   tabs and status bar items get labels and clicking them works.
5. **Scaling.** On a laptop at 150 % with a 100 % external monitor, labels are sized for each
   monitor and land exactly on their elements on both.

## Known limits

- **UIPI.** Windows blocks a normal process from reading or injecting input into windows of a
  higher integrity level: Task Manager, Registry Editor, installers, anything "Run as
  administrator". Those windows yield no labels (screen scope skips them; focused scope reports
  *permission denied*) and `SendInput` into them fails. Run nomouse elevated to reach them.
  `nomouse doctor` shows whether it runs elevated.
- The secure desktop (UAC prompts, the lock screen, Ctrl+Alt+Del) is unreachable for any program.
- The keyboard hook only runs while nomouse pumps messages; a very slow UI Automation query of an
  unresponsive app can delay key handling, and Windows passes keys through if the hook takes
  longer than its timeout.
- Apps that expose no UI Automation tree (some games, custom-drawn toolkits) get no labels.
- Punctuation keys follow the US layout positions while the overlay is shown.
