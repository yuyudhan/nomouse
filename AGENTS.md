# Rules for agents

Read [CONTRIBUTING.md](CONTRIBUTING.md), [docs/architecture.md](docs/architecture.md) and
[docs/testing.md](docs/testing.md) before changing code. This file is the short version and the
rules that apply to automated contributors in particular.

## Where things go

- Platform-independent logic (labels, matching, sessions, modes, config) goes in
  `crates/nomouse-core`. If it can be tested without a desktop, it belongs there.
- OS code goes in its backend crate (`nomouse-linux`, `nomouse-macos`, `nomouse-windows`)
  behind that crate's internal system trait, so a fake can test the logic above it.
- `unsafe` and FFI go only in `nomouse-sys-macos` and `nomouse-sys-windows`, with a `// SAFETY:`
  comment on every block and a safe public API. Every other crate root keeps
  `#![forbid(unsafe_code)]`; `scripts/check-unsafe.sh` enforces it.
- The CLI, daemon, socket and event loop live in `crates/nomouse`.
- Features have stable ids in [docs/features.md](docs/features.md). Reference the id in the
  commit body and flip its status there when a feature is done on an OS.

## Gates

| Command | When |
|---------|------|
| `just fmt` | before every commit |
| `just check` | before every commit: format check, guards, typos, shellcheck, clippy `-D warnings` for Linux, Windows and macOS targets |
| `just test` | before every commit: unit, property and contract tests |
| `just verify` | before handing work back: check, test, cargo-deny |
| `just e2e` / `scripts/e2e-wayland.sh` | any change to the Linux backend, on a Linux machine (private Xvfb or headless Sway; never your session) |

Never weaken a gate to pass it: no `#[allow]` to silence clippy, no skipped or loosened tests, no
lowered coverage floors. Fix the code.

## Code rules

- No `unwrap`, `expect`, `panic!`, `todo!` or `unimplemented!` outside tests (denied by lints).
- Every public item has a doc comment (`missing_docs` is denied).
- Files stay under 3000 lines; split modules first.
- Every behaviour change comes with a test; invariants get a proptest in
  `crates/*/tests/*_prop.rs`. A new `Platform` runs `nomouse_core::contract::assert_platform`.
- New config keys: a default, validation, a test, and entries in `docs/architecture.md` and the
  README.
- Changes users notice go under `## [Unreleased]` in `CHANGELOG.md`.

## Commits and branches

- Work on `nightly`. Commit subjects follow the commit-msg hook: `<gitmoji> [scope: ]summary`,
  one line, at most 100 characters, no trailing period, no tool or assistant credits.
- Small commits, one concern each.
- Never push, tag, release or change repository settings unless the owner asked for that
  exact action. `just release` publishes.
- Never commit secrets, signing material or machine-specific paths.

## Testing UI automation for real

- Linux: `just e2e` (GTK, Qt, Chromium on a private Xvfb with i3 and AT-SPI) and
  `scripts/e2e-wayland.sh` (headless Sway). Both run in CI.
- macOS: grant Accessibility to the terminal, then
  `cargo test -p nomouse-macos --test macos_accessibility`; the manual checklist is in
  `crates/nomouse-macos/README.md`. GitHub's macOS runners have no grant, so this test skips
  there; run it on a real Mac before calling macOS work done.
- Report exactly what ran and where. A skipped test is not a passed test.
