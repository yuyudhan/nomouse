# ADR 0005: the macOS menu bar app and settings window are AppKit through objc2

- Status: accepted
- Date: 2026-11-09

## Context

nomouse.app should behave like Homerow: an agent app (no Dock icon) with a template icon in the
menu bar, a menu for the common actions, and a settings window that writes every change to
`~/.config/nomouse/config.toml` at once while the daemon reloads it. The repository is Rust
first: `unsafe` and FFI live only in `nomouse-sys-macos`, every other crate forbids `unsafe`, and
the release is one universal binary built by `cargo` plus a bundle script, with no Xcode project.

The options:

- **SwiftUI** (a `Settings` scene or a hosted view). The most native look for free, but it adds
  a second language, a Swift build step and a Swift/Rust bridge (a C ABI or a helper process)
  to the release, and its state would have to mirror the Rust settings model across that
  bridge.
- **A Rust GUI toolkit** (egui, iced, Slint). One language, but none draws real AppKit
  controls: no system pop-up buttons, colour wells, toolbar tabs, focus rings, VoiceOver
  labels or accent colours, so the window looks foreign next to System Settings. None of them
  provides `NSStatusItem` either.
- **AppKit through `objc2`**. The controls are AppKit's own (`NSTabViewController` toolbar
  tabs, `NSGridView` rows, `NSPopUpButton`, `NSColorWell`, `NSStepper`), so the window follows
  light and dark mode, the accent colour and accessibility like any Mac app. `objc2` is
  already the crate the overlay and the event tap use, and its generated bindings cover
  `NSStatusItem`, `SMAppService` and `NSOpenPanel`.

## Decision

The menu bar item, its menu, the settings window, the welcome (first run) window and the small
panels (alerts, About, the app picker) are AppKit through `objc2`:

- `nomouse-sys-macos::ui` holds the safe wrappers: the status item and menu, a generic tabbed
  form built from plain page and row descriptions (choices, numbers with steppers, toggles,
  colour wells, shortcut recorders, app lists, a live label preview), the panels, and login
  items through `SMAppService`. All `unsafe` stays in that crate.
- `nomouse-macos::menubar` (no `unsafe`) builds the pages from
  `nomouse_core::settings::FIELDS`, so the macOS and Linux windows list the same settings in
  the same order, and saves every change through `nomouse_core::settings::Settings`.
- The daemon owns the menu bar on its main thread, beside the overlay and the event tap: one
  process, so pause, the hotkeys in effect and the Accessibility state are shown without IPC.

## Consequences

- No Swift or Xcode in the build; the release stays `cargo build` plus
  `scripts/release/app.sh`.
- Layout is written in code (Auto Layout constraints and grid views) rather than designed in
  Interface Builder or SwiftUI previews; the screenshots in `docs/media/macos/` are drawn by
  the app itself (`nomouse screenshots`) to review it.
- The window is tested in process through test-only drivers (`testing` feature) that click,
  pick and type through AppKit's own actions, and end to end through the Accessibility API in
  `scripts/e2e-macos.sh`.
- The Linux window (ADR 0004) uses GTK 4 for the same reason: each OS gets its native
  controls over one shared settings model.
