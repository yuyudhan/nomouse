# Roadmap

- **M0, done**: workspace, quality gates, CI on three OSes, `nomouse-core` with tests.
- **M1, core + Linux X11 end to end**: AT-SPI element source, X11 overlay, XTest input, XGrabKey
  hotkeys, the event loop in the binary; click, right-click, double-click, hover, drag, scroll mode,
  search by text. Tested against GTK, Qt, Firefox and Chromium apps under Xvfb.
- **M2, macOS**: `nomouse-sys-macos` wrappers (AX, CGEvent, NSPanel), the macOS backend, the
  Accessibility permission flow.
- **M3, Windows**: `nomouse-sys-windows` wrappers (UIA, SendInput, layered windows), the Windows
  backend, per-monitor DPI.
- **M4, Wayland (wlroots), done**: layer-shell overlay, virtual pointer/keyboard or uinput input,
  compositor-bound hotkeys over the daemon socket, native window placement from Sway/Hyprland
  IPC; e2e on headless Sway. Evaluated and not done: GNOME needs a Shell extension (no
  layer-shell, no virtual pointer, no client hotkeys); the GlobalShortcuts portal is not needed
  since every supported compositor binds commands, and KDE's KWin offers no window positions to
  clients (native apps there are placed only through XWayland).
- **M5, packaging**: `cargo install`, a Homebrew tap, an apt repository or PPA, winget and an MSI.

Open: the licence (undecided).

The remaining work, in slices with machines and acceptance tests, is in [plan.md](plan.md); the
feature inventory and gap analysis against Homerow is in [features.md](features.md).
