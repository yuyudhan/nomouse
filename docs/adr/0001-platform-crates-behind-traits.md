# ADR 0001: one core crate, platform crates behind traits, FFI in sys crates

- Status: accepted
- Date: 2026-11-09

## Context

nomouse does the same thing on every OS (collect elements, draw labels, read keys, inject
input), but each OS offers it through a different API: Accessibility, AppKit and Quartz on
macOS; AT-SPI2 over D-Bus, X11 or Wayland protocols on Linux; UI Automation and Win32 on
Windows. The product logic (labels, matching, modes, config) must behave identically everywhere
and be testable without a desktop session. The owner also requires `#![forbid(unsafe_code)]`,
which the macOS and Windows APIs cannot satisfy because they are only reachable through FFI.

## Decision

- **`nomouse-core`** holds all platform-independent logic and the four traits a backend
  implements: `ElementSource`, `Overlay`, `InputInjector`, `HotkeySource` (together
  `Platform`). It has no OS dependencies and forbids unsafe code.
- **One backend crate per OS** (`nomouse-linux`, `nomouse-macos`, `nomouse-windows`) implements
  `Platform`, forbids unsafe code, and keeps its OS calls behind a narrow internal trait (the
  AT-SPI `Tree`, `AxTree`, `System`) so its logic is unit-tested with fakes on every OS.
- **FFI lives only in `nomouse-sys-macos` and `nomouse-sys-windows`**: safe wrappers, a
  `// SAFETY:` comment on every unsafe block, `undocumented_unsafe_blocks` denied.
  `scripts/check-unsafe.sh` fails the build on unsafe anywhere else. Linux needs no sys crate
  (pure-Rust `zbus`, `x11rb`, `wayland-client`).
- **The binary** (`nomouse`) selects exactly one backend with target-specific dependencies and
  owns the CLI, the daemon, the command socket and the event loop.
- **One contract** (`nomouse_core::contract`) states what every `Platform` must do; every
  backend, fake or real, runs it.
- Every platform crate compiles on every OS (OS modules are `cfg`-gated), so clippy checks all
  targets from any machine.

## Consequences

- Adding an OS or display server is a new backend that passes the contract; the core does not
  change.
- Behaviour that differs per OS (Wayland's compositor-bound hotkeys, macOS's Chromium opt-in)
  lives in the backend and is reported through `doctor`, not through flags in the core.
- The unsafe surface is two crates that can be audited on their own.
- The traits are blocking and single-threaded, which keeps backends simple; a backend that
  needs an event loop of its own (AppKit, Wayland) runs it inside its trait calls.
