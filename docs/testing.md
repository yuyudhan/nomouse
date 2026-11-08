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
| OS integration | `crates/nomouse-windows/tests/winforms.rs`, `crates/nomouse-macos/tests/macos_accessibility.rs` | their OS | `just test` |
| End to end, Linux X11 | `scripts/e2e-linux.sh` | Linux | `just e2e` |
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
  is free again after `unregister()`; a taken chord is `HotkeyTaken`.
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

The suite also proves itself: `contract::tests::every_check_can_fail` runs it over a backend
that breaks every rule and expects each check to report.

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
4. a Chromium/Chrome `data:` page with `--force-renderer-accessibility`;
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
is removed on exit. `--keep` keeps the temp dir with logs and screenshots. The same
determinism rules apply (bounded readiness polling, one attempt per assertion, every child
reaped with a deadline).

Debian/Ubuntu packages: `sway xwayland wtype grim dbus-daemon at-spi2-core python3-gi
gir1.2-gtk-3.0`. The `/dev/uinput` path cannot run there (the headless backend reads no
libinput devices); it is covered by unit tests of the event translation and needs a manual
check on a compositor without virtual pointer (KDE).

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
- `coverage`: `scripts/coverage.sh`, failing below the floors.

## Writing tests

- New logic goes where a fake can reach it; an OS call goes behind the backend's system trait.
- A new `Platform` (or a new backend fake) must run `nomouse_core::contract::assert_platform`.
- Use the contract and `FakePlatform` instead of writing another in-memory platform.
- If a test exposes a backend bug, fix the bug; never weaken or skip the test.
