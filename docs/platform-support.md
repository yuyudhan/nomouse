# Platform support

What runs where, and how each platform is tested. "e2e" is an end-to-end test that starts real
apps, shows real labels and asserts the clicks landed; [testing.md](testing.md) describes each.

## At a glance

| Platform | Status | Release asset | How it is tested |
|----------|--------|---------------|------------------|
| macOS 11+, Apple silicon | supported | `nomouse-universal-apple-darwin.app.zip` (also `.dmg`, `.tar.gz`) | unit and fake-system tests in CI; desktop e2e (`just e2e-macos`) on an Apple silicon Mac against a Swift fixture app; every release runs the shipped tarball and app on macOS 15 |
| macOS 11+, Intel | supported | the same universal build | every release runs the shipped tarball and app on macOS 15 Intel (`--version`, `doctor`); no Intel desktop test |
| macOS 11 to 14 | built for, not tested | the same universal build | the binaries are built for macOS 11.0 (`MACOSX_DEPLOYMENT_TARGET`, checked in the release); no runner runs them |
| Linux X11, x86_64 | supported, glibc 2.35+ | `nomouse-x86_64-unknown-linux-gnu.tar.gz`, `nomouse_amd64.deb` | e2e in CI on Xvfb + i3 with GTK, Chromium and Qt apps |
| Linux X11, aarch64 | supported, glibc 2.35+ | `nomouse-aarch64-unknown-linux-gnu.tar.gz`, `nomouse_arm64.deb` | built on an arm64 runner in CI; same code as x86_64 |
| Linux Wayland: Sway | supported | the Linux builds | e2e in CI on headless Sway with native Wayland and XWayland GTK windows, hotkey from a Sway `bindsym` |
| Linux Wayland: Hyprland | supported, not e2e-tested | the Linux builds | the same wlroots protocols as Sway; window positions from Hyprland's IPC (`j/clients`) |
| Linux Wayland: KDE Plasma | partial | the Linux builds | labels on the layer shell; clicks through `/dev/uinput`; only XWayland apps are placed correctly; no automated test |
| Linux Wayland: GNOME | not supported | | the installer and `nomouse doctor` detect it and say to log in with "GNOME on Xorg" ("Ubuntu on Xorg" on Ubuntu) |
| Linux with glibc older than 2.35, musl (Alpine, Void musl), NixOS and Guix | from source | none | the installer stops before downloading and points at [building from source](install.md#building-from-source); NixOS with nix-ld passes its check |
| Windows 10/11 x86_64 | early | `nomouse-x86_64-pc-windows-msvc.zip` (C runtime linked in, no Visual C++ Redistributable needed) | unit tests and a UI Automation integration test on the CI runner's desktop; `install.ps1` end to end on windows-latest |
| Windows 11 on ARM | early, under x64 emulation | the same x86_64 build | `install.ps1` accepts it and says it runs under emulation; no ARM hardware test |
| Windows 10 on ARM, 32-bit Windows | not supported | | `install.ps1` refuses them (no x64 emulation) |

## macOS

| Part | How |
|------|-----|
| Elements | the `AXUIElement` accessibility tree |
| Labels | a click-through panel above every window |
| Clicks | `CGEventPost`; `AXPress` / `AXShowMenu` with `semantic_click` |
| Hotkeys | a `CGEventTap` |
| Permission | Accessibility only (the pane is called Device Control and Data Access on macOS 27 and later) |

The `.app` runs without a Dock icon. Releases are ad-hoc signed and not notarized yet; the
installer clears the quarantine flag, so Gatekeeper does not stop it. Because the signature
changes with every version, macOS asks for Accessibility again after each update until
releases are Developer ID signed; the installer and the Welcome window remove the stale entry
([install.md](install.md#allowing-nomouse-again-after-an-update-macos)).

Chromium-based browsers and Electron apps expose their tree once asked: the common Chromium
browsers are asked by default, Electron apps once they are listed in
[`macos.enhanced_ui_apps`](configuration.md#macos-macos-only).

## Linux X11

| Part | How |
|------|-----|
| Elements | AT-SPI2 over D-Bus |
| Labels | an override-redirect, click-through window per monitor |
| Clicks | XTest; AT-SPI `do_action` with `semantic_click` |
| Hotkeys | `XGrabKey` |
| Permission | the AT-SPI switch (`nomouse setup` turns it on) |

Works on any X11 window manager or desktop: GNOME on Xorg, KDE on X11, Xfce, Cinnamon, MATE,
i3 and others. Qt apps need `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1`; Chromium and Electron apps
need `--force-renderer-accessibility`.

## Linux Wayland

| Part | Sway, Hyprland (wlroots) | KDE Plasma | GNOME |
|------|--------------------------|------------|-------|
| Labels | `zwlr_layer_shell_v1` overlay layer | layer shell | none (Mutter has no layer shell) |
| Clicks | `zwlr_virtual_pointer_v1` + `zwp_virtual_keyboard_v1` | `/dev/uinput` (needs the udev rule) | none |
| Hotkeys | compositor binds `nomouse hints` | custom shortcut runs `nomouse hints` | none |
| Native window positions | Sway `get_tree`, Hyprland `j/clients` | not available: XWayland apps only | |

Apps running on XWayland report global coordinates and work fully on every compositor above.
Setup for each is in [install.md](install.md#linux-wayland); the internals are in
[wayland.md](wayland.md).

## The Linux app: tray icon and settings window

| Desktop | Tray icon | Settings window |
|---------|-----------|-----------------|
| GNOME (Xorg or Wayland) | with the AppIndicator extension (on by default on Ubuntu) | yes; on GNOME Wayland it says to log in with "Ubuntu on Xorg" |
| KDE Plasma, Xfce, Cinnamon, MATE | yes (StatusNotifierItem) | yes |
| Sway, Hyprland | with a bar that has a tray (waybar's `tray` module) | yes; the Hotkeys page shows the compositor lines, and Start at login shows the `exec` / `exec-once` line to add (they ignore autostart entries) |
| i3 | not in i3bar, which shows only the older XEmbed icons; `snixembed` bridges them | yes; Start at login shows the `exec --no-startup-id` line to add |

The settings window needs GTK 4.6 and libadwaita 1.1 or later (Ubuntu 22.04, Debian 12,
Fedora 36 and later). The daemon runs without them. Tested in CI on Xvfb + i3 with a stand-in
tray host and on headless Sway with waybar (`scripts/e2e-desktop.sh`).

## Apps

nomouse labels what an app's accessibility tree exposes, so it works with native macOS apps,
GTK, Qt, Firefox, and Chromium and Electron apps (with accessibility on). Terminals, games and
apps that draw everything themselves expose no elements and get no labels.

On macOS, Linux (X11 and Wayland) and Windows a hint label sits in the middle of its element,
centred on both axes (also when the label is larger than the element), and is kept inside the
screen.
