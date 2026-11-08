# nomouse

Keyboard-driven clicking for Linux, macOS and Windows, in the spirit of
[Homerow](https://www.homerow.app/): press a global hotkey, hint labels appear on every clickable
element of the focused window (or the whole screen), type a label to click it. Also planned:
right-click, double-click, hover, drag, a scroll mode, and search by element text.

Elements come from each OS's accessibility tree, so nomouse works with every app that exposes
one: native apps, GTK, Qt, Chromium and Electron (with accessibility on), Firefox, and Java apps
with the accessibility bridge. Apps without an accessibility tree (some terminals, games) are not
supported; a clearly separate fallback may come later.

## Status

Pre-alpha. Done: the platform-independent core (`nomouse-core`) and the Linux X11 backend with
the full CLI: `nomouse daemon` (the default; global hotkeys `ctrl+space` hints,
`ctrl+shift+space` continuous, `ctrl+alt+space` scroll mode), `nomouse hints [--continuous]
[--action <kind>]`, `nomouse scroll`, `nomouse doctor`, `nomouse config path|show|init|check` and
`nomouse labels <n>`. Set `NOMOUSE_LOG=info` (or `debug`) for logs. macOS and Windows backends are
in progress; Wayland is detected and refused until M4. See [docs/roadmap.md](docs/roadmap.md).
`scripts/e2e-linux.sh` runs the Linux end-to-end test on a private Xvfb display.

## Platforms and the accessibility requirement

| OS | Elements | Overlay | Input | What you must enable |
|----|----------|---------|-------|----------------------|
| Linux X11 | AT-SPI (D-Bus) | override-redirect windows | XTest | AT-SPI running (GNOME/KDE do this); `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1` for Qt; `--force-renderer-accessibility` for Chromium/Electron |
| Linux Wayland (wlroots) | AT-SPI | layer-shell | virtual-pointer or uinput | as X11, plus `uinput` access when the compositor lacks virtual-pointer |
| macOS | AXUIElement | NSPanel | CGEvent | System Settings > Privacy & Security > Accessibility: allow nomouse |
| Windows | UI Automation | layered windows | SendInput | nothing; run elevated to reach elevated apps |

GNOME Wayland has no layer-shell or virtual pointer; see the roadmap for what is planned there.

## Building

Rust (toolchain pinned in `rust-toolchain.toml`) and [just](https://github.com/casey/just):

    just verify      # format check, guards, typos, clippy for linux/windows/macos targets, tests, cargo-deny
    cargo run -p nomouse -- doctor

See [CONTRIBUTING.md](CONTRIBUTING.md) for the quality rules and
[docs/architecture.md](docs/architecture.md) for the design.

## Licence

Not yet decided; the repository is private until it is.
