# Roadmap

- **M0, done**: workspace, quality gates, CI on three OSes, `nomouse-core` with tests.
- **M1, core + Linux X11 end to end**: AT-SPI element source, X11 overlay, XTest input, XGrabKey
  hotkeys, the event loop in the binary; click, right-click, double-click, hover, drag, scroll mode,
  search by text. Tested against GTK, Qt, Firefox and Chromium apps under Xvfb.
- **M2, macOS**: `nomouse-sys-macos` wrappers (AX, CGEvent, NSPanel), the macOS backend, the
  Accessibility permission flow.
- **M3, Windows**: `nomouse-sys-windows` wrappers (UIA, SendInput, layered windows), the Windows
  backend, per-monitor DPI.
- **M4, Wayland (wlroots)**: layer-shell overlay, virtual-pointer/uinput input, portal or compositor
  hotkeys. GNOME note: no layer-shell or virtual pointer; needs a Shell extension or the
  RemoteDesktop portal, evaluated here.
- **M5, packaging**: `cargo install`, a Homebrew tap, an apt repository or PPA, winget and an MSI.

Open: the licence (undecided).
