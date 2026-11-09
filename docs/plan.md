# Build plan

The core, the Linux X11 and Wayland (wlroots) backends, the macOS and Windows backends, CI and
the release pipeline exist. This plan covers what [features.md](features.md) lists as partial or
missing, in slices of a few hours to a day. Each slice names its features, the machine it is
built and tested on, and the acceptance tests that close it.

Machines: **mac** is the MacBook M4 (`macbookm4`), where every macOS slice is built and tested
with a real Accessibility grant. **linux** is `itlinux3`. Core slices (no OS code) are built on
`itlinux3` and must also pass `just check` and `just test` on `macbookm4` before they close.

Every slice closes with `just verify` green on its machine, the feature rows in features.md
updated, a CHANGELOG entry, and no new `#[allow]`.

## How UI automation is tested for real

- **Linux X11** (exists): `just e2e` starts a private Xvfb, D-Bus session, AT-SPI bus and i3,
  opens `scripts/e2e/gtk_app.py` (GTK 3), `scripts/e2e/qt_app.py` (PyQt5 with the AT-SPI
  bridge) and Chromium, drives nomouse with typed labels and asserts the app's own click
  counters. New behaviour adds assertions there.
- **Linux Wayland** (exists): `scripts/e2e-wayland.sh` runs headless Sway
  (`WLR_BACKENDS=headless`), native and XWayland GTK windows, types with `wtype`, checks pixels
  with `grim`. Slice 13 adds a second headless compositor.
- **macOS** (slice 1 builds it): a fixture app, `scripts/e2e/mac_app.swift`, compiled with
  `swiftc` at test time: one window with known buttons (`One`..`Ten`), a text field, a
  scrollable list of 200 rows, a context-menu target and a drag source/target, each updating an
  accessibility-visible counter label. `scripts/e2e-macos.sh` starts it, runs
  `nomouse hints --standalone` (and the other modes), types labels with a tiny `CGEvent` key
  poster, and asserts the counters through AX. It needs the Accessibility grant for the
  terminal, so it runs on `macbookm4`, never on GitHub's runners (which skip with `SKIP:`).

## Slices

### Milestone A: macOS ready for daily use (P0)

| # | Slice | Features | Machine | Acceptance |
|---|-------|----------|---------|------------|
| 1 | macOS fixture app and `scripts/e2e-macos.sh`, `just e2e-macos` | test infra | mac | 10/10 left clicks on `One`..`Ten`; right click opens the context menu; double click increments by two; drag moves the source to the target; scroll mode moves the list; search `seven` + Enter clicks `Seven`; no overlay panel left after Escape (AX query of nomouse's windows); the script fails, not hangs, when the grant is missing |
| 2 | `nomouse.app` bundle: `Info.plist` (LSUIElement, bundle id, version from Cargo), icon, ad-hoc signed in CI, built by `scripts/release/app.sh` | NM-DIST-02 (unsigned part) | mac | `codesign --verify --deep --strict` passes on the ad-hoc build; `open nomouse.app` starts the daemon with no Dock icon; the Accessibility grant to the `.app` survives a rebuild with the same signature; slice 1's e2e passes against the `.app`'s binary |
| 3 | Guided first run: `nomouse setup` (and on first daemon start) explains each permission, opens the Accessibility pane, polls trust, restarts; Linux enables AT-SPI and prints the uinput rule | NM-APP-10 | mac, then linux | unit tests of the step machine with a fake trust probe; on mac with the grant revoked (`tccutil reset Accessibility <bundle id>`) the flow reaches "granted" after the toggle and the e2e passes after it; on linux the switch is on afterwards (`busctl --user get-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled`) |
| 4 | Signing and notarization in `release.yml`: Developer ID, hardened runtime, `notarytool`, `stapler`, `.dmg` | NM-DIST-02 | mac (CI) | blocked on the owner's Developer ID certificate and App Store Connect key as repository secrets; a manual workflow run produces a `.dmg` that `spctl --assess --type open --context context:primary-signature` accepts and that opens on a fresh user account without a Gatekeeper prompt |
| 5 | Homebrew cask in `yuyudhan/homebrew-tap` generated from the release, formula kept for the CLI | NM-DIST-03 | mac | `brew install --cask ./nomouse.rb` from a local file installs the `.app`; `brew uninstall --cask` removes it; `brew audit --cask --strict` is clean |
| 6 | Layout-aware hotkeys and keys on macOS (`TISCopyCurrentKeyboardLayoutInputSource` + `UCKeyTranslate` in `nomouse-sys-macos`) | NM-KEY-03 | mac | unit tests over captured layout tables (US, Dvorak, Colemak, AZERTY, German); e2e with the Dvorak input source selected: labels type as shown and `ctrl+space` triggers by character |

### Milestone B: Homerow parity in the core (P1)

| # | Slice | Features | Machine | Acceptance |
|---|-------|----------|---------|------------|
| 7 | Activation toggles; focus-then-act (Tab/↓/Ctrl-N, Shift-Tab/↑/Ctrl-P, Return); optional Homerow bindings (`hints.keys = "homerow"`: Return click, Return twice double click, Shift-Return right click, Cmd-Return cmd click) | NM-HINT-10, NM-HINT-11, NM-ACT-09 | linux (core), mac | `HintSession` unit tests for each binding; proptest: any sequence of focus moves keeps the highlight on a visible hint; e2e (X11, macOS fixture): Tab twice + Return clicks the third label's button; hotkey twice leaves no overlay |
| 8 | Search: spaces ignored, `*` and `;` match all, matching over description, value and role | NM-SRCH-04, NM-SRCH-05 | linux (core), mac | proptests in `config_prop.rs`/a new `search_prop.rs` (query with spaces scores as without; `*` returns every searchable element); backend tests that description and value reach `Element`; e2e: a button whose only text is its tooltip is found |
| 9 | Scroll mode: `gg`/`G`, Shift-`hjkl` dash scroll with `scroll.dash_step`, Tab and digits to cycle/jump areas, largest area preselected, highlight border | NM-SCRL-03, NM-SCRL-04, NM-SCRL-05, NM-SCRL-07 | linux, then mac | `run.rs` unit tests for every key; config tests for `dash_step` range; e2e on the 200-row list (GTK fixture and macOS fixture): `G` shows the last row, `gg` the first, Tab moves to the second list |
| 10 | Ignored apps (`ignore.apps` by bundle id, app id, `WM_CLASS` or process name) and config reload (`nomouse reload` over the socket, plus a file watch) | NM-APP-05, NM-APP-07 | linux, mac | unit tests of the matcher per id kind; e2e: with the fixture app ignored, the hotkey shows no labels and `doctor` lists it; editing the alphabet then `nomouse reload` changes the next labels without restarting the daemon |
| 11 | Themes: `overlay.theme = light | dark | original | auto`, explicit colours override; `auto` follows `NSApp.effectiveAppearance` and the `org.freedesktop.appearance color-scheme` portal setting | NM-LOOK-02 | linux, mac | config tests for each theme and the override order; screenshot e2e: dark appearance gives dark labels (`grim`/`screencapture` pixel sample at a label) |
| 12 | Stop continuous mode at text inputs; browser fast vs exhaustive collection (`hints.browser = fast | exhaustive`) | NM-ACT-10, NM-HINT-14 | mac, linux | e2e: continuous mode ends after clicking the fixture's text field and typed text lands in it; on a long Chromium page `exhaustive` labels off-viewport links reachable by scrolling, `fast` does not |

### Milestone C: desktop integration and Linux distribution (P1)

| # | Slice | Features | Machine | Acceptance |
|---|-------|----------|---------|------------|
| 13 | Second headless Wayland compositor in e2e (Hyprland headless, or KWin `--virtual`), same assertions as Sway; Qt 6 fixture | test infra | linux | `scripts/e2e-wayland.sh --compositor hyprland` green; CI job added |
| 14 | Menu bar item on macOS (`NSStatusItem`: enabled toggle, open config, doctor, quit) | NM-APP-08 (mac) | mac | AX test finds the status item and its menu; toggling disables the hotkey (e2e asserts no overlay); quit removes the socket |
| 15 | Tray on Linux (StatusNotifierItem over `zbus`), XDG autostart entry, systemd user unit | NM-APP-08 (linux), NM-APP-09 (linux) | linux | D-Bus test against a private `org.kde.StatusNotifierWatcher`; `systemd-analyze --user verify` on the unit; autostart file validates with `desktop-file-validate` |
| 16 | Launch at login on macOS (`SMAppService.mainApp`, needs slice 2) | NM-APP-09 (mac) | mac | `nomouse login enable` registers, `launchctl print gui/$UID` lists it; after logout and login the daemon runs (manual, recorded in the slice report) |
| 17 | `.rpm`, AUR `nomouse-bin` PKGBUILD, AppImage, each with the uinput udev rule and autostart file | NM-DIST-05, NM-DIST-06, NM-DIST-07, NM-DIST-08 | linux | `rpm -qlp` lists binary, rule and autostart; `namcap` on the PKGBUILD; the AppImage runs `nomouse doctor` on a clean Fedora and Arch container; `just release-dry-run` builds all of them |

### Milestone D: beyond Homerow (P2)

| # | Slice | Features | Machine | Acceptance |
|---|-------|----------|---------|------------|
| 18 | Grid mode for apps without a tree: labelled 3×3 grid, narrows recursively, click at the cell | NM-FALL-01 | linux, mac | core unit tests of the subdivision; e2e clicks a known pixel in an `xterm` (no AT-SPI tree) and in the macOS fixture's canvas view |
| 19 | Normal mode: `hjkl` pointer movement with acceleration, keys for click/drag | NM-FALL-02 | linux, mac | unit tests of the motion model; e2e drags the fixture's slider by keys |
| 20 | Tutor (`?` shows role, name, properties) and sound feedback | NM-HINT-12, NM-LOOK-04 | mac, linux | session unit tests; e2e asserts the tutor overlay text via screenshot sample; sound off by default |
| 21 | KDE and GNOME input through libei (RemoteDesktop portal) and the GlobalShortcuts portal | NM-FALL-04, NM-KEY-06 | linux | headless KWin e2e clicks without `/dev/uinput`; portal consent is granted once by a pre-seeded permission store in the test session |
| 22 | GNOME Wayland Shell extension (overlay, input and hotkeys inside Shell, talking to the daemon over D-Bus) | NM-FALL-03 | linux | `gnome-shell --nested --headless` e2e with the GTK fixture: 10/10 clicks |
| 23 | Mission Control labels, modifier-only shortcuts, Hyperkey verification, search and hints in one activation, per-app overrides, settings window, update check | NM-HINT-13, NM-KEY-04, NM-KEY-05, NM-SRCH-06, NM-APP-06, NM-APP-11, NM-APP-12 | mac, linux | one slice each when picked up; acceptance written then, in the same style |

## Order

Slice 1 first: every later macOS slice proves itself with that harness. Then 2 and 3 (they
make the first macOS release usable), 6, then milestone B in order. Slice 4 waits for signing
credentials; 5 follows 4. Milestone C can run on `itlinux3` in parallel with macOS work on
`macbookm4`.
