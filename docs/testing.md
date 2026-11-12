# Testing

nomouse is tested in layers. Each layer has a fixed home, a fixed command, and runs in CI on
every push to `nightly`. Nothing is skipped silently: a test that needs something missing on
the machine either fails or says `SKIP:` with the reason.

| Layer | Where | Runs on | Command |
| --- | --- | --- | --- |
| Unit | `#[cfg(test)] mod tests` next to the code | Linux, macOS, Windows | `just test` |
| Property | `crates/*/tests/*_prop.rs` (proptest) | Linux, macOS, Windows | `just test` |
| Backend contract, fakes | `nomouse_core::contract` run over each fake | Linux, macOS, Windows | `just test` |
| Backend contract, real | the same suite over the real backend | Linux (e2e), Windows, macOS | see below |
| OS integration | `crates/nomouse-windows/tests/winforms.rs`, `crates/nomouse-macos/tests/macos_accessibility.rs`, `crates/nomouse-macos/tests/settings_window.rs` | their OS | `just test` |
| End to end, Linux X11 | `scripts/e2e-linux.sh` | Linux | `just e2e` |
| End to end, Linux desktop app (tray, settings window, launcher) | `scripts/e2e-desktop.sh` | Linux | `just e2e-desktop` |
| End to end, macOS | `scripts/e2e-macos.sh` | a Mac with the Accessibility grant | `just e2e-macos` |
| Coverage floor | `scripts/coverage.sh` | Linux | `just coverage` |

`just verify` (the pre-push hook) runs format, guards, typos, shellcheck, clippy for every
target, all tests and `cargo deny`. `just e2e` and `just coverage` are separate because they
need an X server and instrumentation; CI runs both as their own jobs.

## Unit and property tests

Pure logic lives in `nomouse-core` (labels, filtering, sessions, config, hotkeys, actions) and
in the binary's `run.rs` (the activation loop) and `lock.rs` (single instance). Backends keep
their OS calls behind a narrow trait so the logic above it is unit-tested on every OS:

- Windows: `nomouse_windows::System`, faked in `crates/nomouse-windows/src/backend_tests.rs`.
- macOS: `AxTree` in `walk.rs`, faked in its tests.
- Linux: the AT-SPI `Tree` in `a11y/walk.rs`, faked in its tests.

Property tests (proptest) cover invariants over arbitrary input, for example
`crates/nomouse-core/tests/labels_prop.rs`: labels are unique, prefix-free and as short as the
alphabet allows.

## The backend contract

`crates/nomouse-core/src/contract.rs` (feature `testing`) is the one statement of what every
`Platform` must do, written as checks rather than prose:

- `name()` is short kebab-case.
- `elements()` returns ids unique within a snapshot, or only the documented errors
  (`AccessibilityUnavailable`, `PermissionDenied`, `NoFocusedWindow`).
- `hide()` while hidden is a no-op; `show()` replaces what was shown; `show(&[])` works;
  `show_search()` draws with an empty query, a query, and no matches; `hide()` after either
  succeeds, twice.
- `unregister()` of an unknown id is a no-op; two registered chords get distinct ids; a chord
  registered twice (by another program or by this backend) is `HotkeyTaken`; a chord is free
  again after `unregister()`.
- `next_event()` on an idle session returns `Ok` within its timeout (plus 2 s slack).
- `perform(hover)` succeeds or is `PermissionDenied`. The suite never clicks.

`check_platform` returns the violations; `assert_platform` panics with all of them. It also
holds `FakePlatform`, the reference in-memory backend for code above the backends.

Who runs it:

| Implementation | Test | Where |
| --- | --- | --- |
| `FakePlatform` | `contract::tests::the_fake_passes_the_contract` | every OS |
| Windows backend over its fake `System` | `backend_tests::the_fake_system_passes_the_contract` | every OS |
| Windows backend, real desktop | `winforms::the_real_backend_passes_the_contract` | Windows CI |
| macOS backend, real session | `macos_accessibility` (after the trust check) | macOS with an Accessibility grant |
| Linux X11 backend, real session | `crates/nomouse-linux/tests/contract.rs` (`#[ignore]`) | `just e2e`, Linux CI |

The suite also proves itself: `contract::tests::every_check_can_fail` and
`the_remaining_checks_can_fail` run it over two backends that break the rules between them, and
expect every check to report.

GitHub's macOS runners have no Accessibility grant and the backend refuses to start without
one, so on CI the macOS integration test prints `skipped: this process is not trusted` and
passes; on a Mac with the grant (`System Settings > Privacy & Security > Accessibility` for the
terminal) `cargo test -p nomouse-macos --test macos_accessibility` runs the contract and the
Finder test.

## End to end on Linux X11

`scripts/e2e-linux.sh` (`just e2e`) builds the release binary, then starts a private session
that never touches yours: Xvfb on its own display (default `:97`, `--display :NN` to change),
a private D-Bus session bus, `at-spi-bus-launcher` and i3 with a private IPC socket, with
`HOME`, `XDG_RUNTIME_DIR` and `XDG_CONFIG_HOME` in a temporary directory. It runs:

1. the real-backend contract test;
2. a GTK 3 window (`scripts/e2e/gtk_app.py`): 10 single clicks, continuous mode, right click,
   semantic (AT-SPI action) click with the pointer parked away, scroll mode;
3. the hotkey daemon: single instance, five hotkey-to-labels latency samples, a click;
4. a Chromium/Chrome `data:` page with `--force-renderer-accessibility`, and a second page with
   a link 6000 px below the viewport (`hints.browser`): `fast` search does not reach it, hint
   mode under `exhaustive` labels only what is in view, and `exhaustive` search scrolls to the
   link and clicks it five times out of five;
5. a Qt 5 window (`scripts/e2e/qt_app.py`, PyQt5) with `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1`;
6. no `nomouse-overlay` window is left behind.

Chromium and PyQt5 are optional locally (`SKIP:` when absent; `--no-chromium`, `--no-qt`) and
required in CI (`--require-chromium --require-qt`). `NOMOUSE_E2E_QT_PYTHON` points at an
interpreter with PyQt5. It must be a Qt built with the AT-SPI bridge, such as Debian/Ubuntu's
`python3-pyqt5`. The PyQt5 pip wheel has no bridge: its windows never appear on the
accessibility bus, so the Qt check fails with it. Without root, unpack the distribution
packages (`apt-get download python3-pyqt5 python3-pyqt5.sip libqt5core5a libqt5gui5
libqt5widgets5 libqt5dbus5 ...`, then `dpkg -x`) and point a wrapper at them with
`LD_LIBRARY_PATH`, `PYTHONPATH` and `QT_PLUGIN_PATH`. `--hold` keeps the session up for
debugging. `NOMOUSE_E2E_KEEP=1` keeps the run's logs (each `nomouse` debug log and app stderr)
and prints where they are. A failed Chromium or Qt check prints the tail of the `nomouse` log and
of the app's stderr.

Determinism rules for the script:

- Bounded polling (`wait_for <seconds>`) is used only for startup readiness: the X server, the
  bus, i3, app windows, `nomouse` reaching its first scan, and each app's accessibility tree.
  `scripts/e2e/a11y_ready.py <pid> <name> <seconds>` polls AT-SPI directly (not through
  nomouse) until the app exposes the element the check will use. Chromium fills its tree
  seconds after its window maps.
- Every assertion is attempted once, with a fixed timeout. There are no retries.
- Every child is reaped with a deadline (`reap`). A hung `nomouse` fails its step instead of
  stalling the job, and CI caps the job at 30 minutes.
- It is shellcheck-clean at `--severity=style` like every script (`just shellcheck`).
- Under `CI=true`, Chromium runs with `--no-sandbox`. GitHub's Ubuntu runners forbid the
  unprivileged user namespaces its sandbox needs, so without the flag it aborts at start.

Debian/Ubuntu packages: `xvfb i3-wm xdotool x11-utils dbus-daemon at-spi2-core python3-gi
gir1.2-gtk-3.0 gir1.2-atspi-2.0`, plus `python3-pyqt5` and Chromium/Chrome for the optional
targets.

## End to end on Wayland (headless Sway)

`scripts/e2e-wayland.sh` starts a private D-Bus session, AT-SPI and Sway with
`WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman` in a private
`XDG_RUNTIME_DIR` (never the user's session), opens `scripts/e2e/gtk_app.py` twice, natively
(`GDK_BACKEND=wayland`) and on XWayland side by side with title bars on (so the native window's
content is offset inside its container), and types labels with `wtype`. It asserts, for both
windows: 10/10 left clicks, a right click and scroll mode landing; that labels change the
screen (`grim`) and leave it identical after Escape; that the daemon takes `nomouse hints` over
its socket and from a Sway `bindsym Ctrl+space`; that a second daemon is refused and the socket
is removed on exit; and, with Chrome installed (native Wayland), the same `hints.browser`
checks as on X11. `--keep` keeps the temp dir with logs and screenshots. The same
determinism rules apply (bounded readiness polling, one attempt per assertion, every child
reaped with a deadline).

Debian/Ubuntu packages: `sway xwayland wtype grim dbus-daemon at-spi2-core python3-gi
gir1.2-gtk-3.0`. The `/dev/uinput` path cannot run there (the headless backend reads no
libinput devices); it is covered by unit tests of the event translation and needs a manual
check on a compositor without virtual pointer (KDE).

## End to end: the Linux desktop app

`scripts/e2e-desktop.sh` (`just e2e-desktop`) tests what a desktop user touches: the launcher
entry, the tray icon and menu, and the settings window, on two private sessions.

On X11 (Xvfb, i3, AT-SPI, `scripts/e2e/sni_watcher.py` as the tray host's watcher and
`scripts/e2e/fake_portal.py` for the light or dark style) it runs the installed-style desktop
entry with `gio launch` and checks that the daemon starts and the settings window opens, and
that a second launch keeps one window. It reads the tray menu over D-Bus as a host does
(`scripts/e2e/tray_menu.py`) and checks its rows, the icon names, and Pause from the menu
(the window's switch follows; the switch resumes). It drives the window over AT-SPI
(`scripts/e2e/settings_driver.py`) with real pointer clicks and key presses from `xdotool`:
records `super+shift+h` as the hints hotkey and checks `config.toml`, the daemon's reload and
that the new chord starts hints; picks the Dark theme and checks the file and the reload; asks
Reset to defaults, checks that nothing changes before the confirmation and everything after;
closes the window and reopens it from the tray's Settings…; and quits the daemon from the menu.

On headless Sway with waybar as a real tray host it clicks the icon in the bar (Sway's
`seat cursor` commands) and checks that the menu opens, opens the window natively on Wayland
from Settings… and checks that its Hotkeys page shows the `bindsym` lines.

`--shots DIR` also saves each settings page and the tray menu in light and dark
(`settings-<page>-<light|dark>.png`, `tray-menu-<light|dark>.png`); the README's copies live in
`docs/media`. GTK 4.6 reports widget positions relative to the window, so pointer clicks are
used only in the main window (which i3 places at 0,0); dialog buttons are pressed through
their accessible action.

Debian/Ubuntu packages: `xvfb i3-wm xdotool x11-utils imagemagick dbus-daemon at-spi2-core
libglib2.0-bin python3-gi gir1.2-atspi-2.0 libgtk-4-dev libadwaita-1-dev sway waybar grim`.

The settings window also has an in-process UI test, `crates/nomouse-settings/tests/ui.rs`: it
opens every page, records hotkeys, sets each kind of control through the real widgets, edits
the file on disk while the window is open, and answers the reset dialog, checking
`config.toml` after each step. It needs an X server and skips without one;
`NOMOUSE_UI_TESTS=1 GDK_BACKEND=x11 xvfb-run -a dbus-run-session -- cargo test -p
nomouse-settings --test ui` runs it (CI does, in the `e2e-desktop` job). `GDK_BACKEND=x11`
matters: GTK 4 would otherwise open the window on the Wayland desktop you are logged in to.

## End to end on macOS

`scripts/e2e-macos.sh` (`just e2e-macos`) runs on the logged-in, unlocked desktop of a Mac:
macOS has no private display, so it moves the real pointer and types for real. It compiles two
Swift programs with `swiftc` (Xcode or the Command Line Tools):

- `scripts/e2e/mac_app.swift`, the fixture: one window `nomouse-e2e` with buttons `One`..`Ten`,
  a `Context` button with a context menu, an untitled button whose only text is its tooltip
  `Keep a copy`, a text field, a 200-row and a 40-row list and a drag `Source` and `Target`.
  Each control updates a static text with an accessibility identifier (`count.<button>`,
  `count.Tooltip`, `cmd-clicks`, `menus`, `drops`, `first-row`, `last-row`, `first-row.b`,
  `field`).
- `scripts/e2e/mac_probe.swift`: reads those counters through Accessibility (`value`), brings
  the fixture to the front (`front`), counts a process's windows as Accessibility lists them
  (`windows`) and its overlay panels (`panels`), and posts key events (`type`, `key
  ctrl+space`) on the key that types each character in the current layout, with real modifier
  key presses around chords. It also reads, selects and removes keyboard layouts (`layout`,
  `select-layout`, `layout-enabled`, `disable-layout`), and drives an app's menu bar item and
  windows: `status-item`, `menu <pid> <item>`, `press`, `press-title`, `choose`, `window`,
  `titles` and `close`.

With a private `XDG_CONFIG_HOME` and `XDG_RUNTIME_DIR` (the user's config and daemon are never
touched) it asserts: 10/10 left clicks on `One`..`Ten`; a right click opens the context menu
without a left click; a double click adds two; a drag moves the Source onto the Target; scroll
mode labels both lists, starts on the larger one (`j` scrolls it, `G` shows its last row, `gg`
its first, Shift-`j` dashes further than `jjj`) and Tab moves to the smaller one; search
`seven` + Enter clicks `Seven`, and `keep a copy` + Enter clicks the untitled tooltip button;
a stray Return before any Tab leaves the labels up, Tab moves the highlight to `Three` and
Return clicks it; with `hints.keys = "homerow"` a label
then Return clicks, Return twice double clicks, Shift-Return opens the context menu and
Cmd-Return clicks with Command held; continuous mode ends after clicking the text field and
the text typed next lands in it; `overlay.theme = "auto"` picks the colours of the system
appearance (`defaults read -g AppleInterfaceStyle`), as logged; the daemon's `ctrl+space`
hotkey then a label clicks; `nomouse hints` hands its mode to the daemon, and after Escape the
daemon has no window left; the hotkey, and `nomouse hints`, pressed again while the labels are
up leave no window and click nothing; with the fixture's executable `mac_app` in
`ignore.apps`, `doctor` lists it and the hotkey shows nothing, and after the file is rewritten
with a new alphabet, `nomouse reload` makes the next labels use it; a daemon with the hints
hotkey `ctrl+alt+o` keeps working when the layout switches to Dvorak under it (`ctrl+alt+o`
pressed on the key that types `o`, then the label typed on Dvorak, clicks `Three`). The user's
layout is restored, and Dvorak removed again if the run added it, even when a check fails.
The menu bar section starts a daemon on its own config and checks that the menu bar has the
`nomouse` item (`mac_probe status-item`); that **Settings…** in its menu opens the window
(`menu`, `window nomouse-settings`); that a hotkey recorded on the Hotkeys tab (`press-title`,
`press hotkeys.hints`, then the chord `ctrl+alt+j`) and the theme picked on the Appearance tab
(`choose overlay.theme Dark`) are in `config.toml` at once and each makes the daemon log
`config reloaded`; that the recorded hotkey then labels the fixture; and that **Quit nomouse**
ends the daemon. The daemon checks count only its overlay panels (`mac_probe panels`, windows
at the screen-saver level), since the menu bar item and the settings window are windows too.
`--bin <path>` tests another binary, a development build as well as a release one (a bundle's
`nomouse.app/Contents/MacOS/nomouse` or `nomouse Dev.app/Contents/MacOS/nomouse` runs in place,
so that bundle's own grant applies); `--keep` keeps the logs.

The settings window also has an in-process test, `crates/nomouse-macos/tests/settings_window.rs`
(`cargo test -p nomouse-macos --features testing --test settings_window`, part of `just test`):
over a scratch config it opens the window on Hotkeys, clicks the Hints recorder and sends it
Control-Option-J through AppKit's own key event path, picks Dark in the Theme pop-up, and
checks that `config.toml` has both at once, that a second `Settings` (as the daemon holds)
reloads each, that the earlier hotkey stays in the file, and that the window closes. It needs
a logged-in session but no Accessibility grant; elsewhere it prints `SKIP:`.

The result windows (the doctor's report, the update check's answer) have an in-process test,
`crates/nomouse-macos/tests/result_windows.rs` (`cargo test -p nomouse-macos --features testing
--test result_windows`, part of `just test`): it opens both, then works the daemon's loop as the
daemon does: a Show Hints pick is handed out by the menu bar and a configuration edit reloads
while both windows are open, and OK closes each. A modal window never returns from the first
call, so the test kills it after a minute and fails.

Start at login has a real test, `crates/nomouse-macos/tests/start_at_login.rs` (`cargo test -p
nomouse-macos --features testing --test start_at_login`, part of `just test`): it wraps its own
executable in a signed `nomouse.app` with a separate bundle identifier (`scripts/release/app.sh`),
runs the menu's Start at login pick in it, and checks `SMAppService` status and that System
Events lists the app as a login item, then picks it again and checks both are gone. It does
this twice: started by its path in the bundle and through a symlink to it, as `install.sh`
links `nomouse` (the daemon starts itself again from the real path, `bundle::reexec_from_bundle`).
The installed nomouse's login item is never touched. It needs the user's login session, so it
skips on GitHub Actions.

The documentation's pictures of the menu and every settings page, light and dark
(`docs/media/macos/`), come from `scripts/screenshots-macos.sh`: the app draws its own
windows into PNG files (`nomouse screenshots <dir>`), so it needs no Screen Recording
permission and works on a locked screen.

The label colours are checked through the log, not by pixels: `screencapture` of other apps'
windows needs the Screen Recording permission, which the test does not assume.

The layout tables in `crates/nomouse-macos/testdata/layouts/` (US, Dvorak, Colemak, AZERTY,
German) were captured with `swift crates/nomouse-macos/testdata/layouts/capture.swift <input
source id>`; the hotkey tests in `keys.rs` run over them on every OS.

The grant belongs to the process that runs the script: the terminal, or `sshd` for a remote
run. Without it the script fails within seconds and says where to grant it; under `CI=true`
(GitHub's runners have no grant) it prints `SKIP:` and passes. The determinism rules above
apply: bounded polling only for the fixture window and the first scan, one attempt per
assertion, every `nomouse` reaped with a deadline. The desktop is shared, so before each
`nomouse` run the script checks that the fixture is still running and that no other app's
window is in front of it (`mac_probe covering`, for example a floating launcher panel; the
pointer's own Window Server window is not counted, and a covering window gets three seconds
to go); either ends the run with a `FAIL:` naming the cause instead of failing every later
check.

## Coverage

`scripts/coverage.sh` (`just coverage`, `--html` for a report in `target/llvm-cov/html`) runs
the workspace tests under `cargo-llvm-cov` and prints line coverage for two groups:

- `core`: `crates/nomouse-core/src`;
- `cli-logic`: the binary's non-OS logic, `crates/nomouse/src/run.rs` and `lock.rs`.

It fails when a group is below its floor in `scripts/coverage-floors.txt`. Each floor is the
value measured when it was set minus 2 points. Raise a floor when coverage grows; never lower
one to make a change pass: add tests instead. Backend crates are not floored because most of
their code runs only against a live OS session; their logic is covered through the fakes.

Install: `cargo install cargo-llvm-cov` (or `cargo binstall cargo-llvm-cov`) and
`rustup component add llvm-tools-preview`.

## CI

`.github/workflows/ci.yml` on every push and pull request to `nightly`:

- `check` on ubuntu, macOS and Windows: format, guards, typos, shellcheck (Linux), clippy,
  then `Test (core, binary, Linux backend)` and `Test (Windows + macOS backends, integration on
  their OS)`. The second step is where the WinForms/UI Automation test and the real-backend
  contract run on the Windows desktop, and the Finder test on macOS.
- `e2e-linux`: installs the packages above and runs
  `scripts/e2e-linux.sh --display :99 --require-chromium --require-qt`.
- `e2e-wayland`: `scripts/e2e-wayland.sh`.
- `e2e-desktop`: `scripts/e2e-desktop.sh --display :98` on Ubuntu 22.04 (the oldest GTK 4 and
  libadwaita the settings window supports).
- `coverage`: `scripts/coverage.sh`, failing below the floors.

## Writing tests

- New logic goes where a fake can reach it; an OS call goes behind the backend's system trait.
- A new `Platform` (or a new backend fake) must run `nomouse_core::contract::assert_platform`.
- Use the contract and `FakePlatform` instead of writing another in-memory platform.
- If a test exposes a backend bug, fix the bug; never weaken or skip the test.
