# Changelog

All notable changes to nomouse are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). `just release X.Y.Z` turns the Unreleased section into
the release's section; add entries under Unreleased as changes land.

## [Unreleased]

### Added

- Hint mode: a hotkey labels every clickable element of the focused window from the
  accessibility tree; typing a label clicks it (left, right, double, middle, move, semantic
  click), with a continuous mode that keeps hinting after each click.
- Scroll mode and configurable keys, alphabet, colours and per-app settings in `config.toml`
  (`nomouse config init|path|show`).
- Linux backend: AT-SPI elements, X11 overlay, XTEST input and XGrabKey hotkeys.
- macOS backend: AX tree walk, NSPanel overlay, CGEvent input and event-tap hotkeys.
- Windows backend: UI Automation elements, layered overlay, SendInput and hotkeys.
- `nomouse doctor` reports whether the platform's accessibility and input access work.
- Release pipeline: archives for Linux (x86_64, aarch64), macOS (universal) and Windows
  (x86_64) with SHA-256 checksums, `.deb` packages, a Homebrew formula, and no-sudo installers
  (`install.sh`, `install.ps1`).
