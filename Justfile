set shell := ["bash", "-euo", "pipefail", "-c"]
set windows-shell := ["bash", "-euo", "pipefail", "-c"]

# Targets clippy runs for; the cross targets need only `rustup target add` (no linker).
targets := "x86_64-unknown-linux-gnu x86_64-pc-windows-gnu aarch64-apple-darwin"

default:
    @just --list

# Format Rust and TOML in place.
fmt:
    cargo fmt --all
    taplo fmt

# Check formatting without changing files.
fmt-check:
    cargo fmt --all -- --check
    taplo fmt --check

# Line-count (3000) and unsafe guards.
guards:
    scripts/check-line-count.sh
    scripts/check-unsafe.sh

# Spelling.
typos:
    typos

# shellcheck over every tracked shell script and git hook; install.sh as POSIX sh, which the
# one-line install pipes it into.
shellcheck:
    git ls-files -z -- '*.sh' '.githooks/*' | xargs -0 shellcheck --severity=style
    shellcheck --shell=sh --severity=style scripts/install.sh

# Clippy with -D warnings for the host only.
clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Clippy with -D warnings for every target triple. Cross-checking Linux from another OS skips
# nomouse-settings: its GTK and libadwaita build scripts need the target's system libraries,
# so it is linted on Linux hosts and in CI, where they are installed.
clippy-all:
    #!/usr/bin/env bash
    set -euo pipefail
    for target in {{targets}}; do
        skip=()
        if [[ "$target" == *-linux-* && "$(uname -s)" != Linux ]]; then
            echo "clippy-all: $target without nomouse-settings (needs GTK 4 for that target)"
            skip=(--exclude nomouse-settings)
        fi
        cargo clippy --workspace ${skip[@]+"${skip[@]}"} --all-targets --all-features --target "$target" -- -D warnings
    done

# Lint: clippy with -D warnings for every target triple (same as clippy-all).
lint: clippy-all

# Fast checks on the host: what the pre-commit hook runs on the staged tree.
pre-commit: fmt-check guards typos shellcheck clippy

# Format check, guards, typos, shellcheck and clippy for every target.
check: fmt-check guards typos shellcheck clippy-all

# Unit, integration and property tests, install.sh against a fake release, the documented
# one-line install against a local server, and just dev-config.
test:
    cargo test --workspace --all-features
    scripts/test-installer.sh
    scripts/test-install-command.sh
    scripts/test-dev-config.sh

# Licences, advisories, duplicate dependencies, sources.
deny:
    cargo deny check

# Linux X11 end-to-end tests on a private Xvfb/i3/AT-SPI session (never your display).
e2e *args:
    scripts/e2e-linux.sh {{args}}

# The Linux desktop app end to end: launcher entry, tray menu and settings window, on private
# Xvfb and headless Sway sessions. `--shots docs/media` refreshes the README's screenshots.
e2e-desktop *args:
    scripts/e2e-desktop.sh {{args}}

# macOS end-to-end tests against a Swift fixture app on the logged-in desktop (needs the
# Accessibility grant; moves the real pointer). `--bin <path>` tests another nomouse binary.
e2e-macos *args:
    scripts/e2e-macos.sh {{args}}

# Line coverage of nomouse-core and the binary's non-OS logic; fails below the floors.
coverage *args:
    scripts/coverage.sh {{args}}

# The full local gate (what the pre-push hook runs on each pushed commit): check, test, deny.
verify: check test deny

# Use the repo's git hooks (pre-commit, commit-msg, pre-push). Safe to run again.
install-hooks:
    git config core.hooksPath .githooks
    @echo "hooks: core.hooksPath = $(git config core.hooksPath)"

# Build every Linux release asset from the committed HEAD (a snapshot under target/hook-tree/,
# so uncommitted edits are left out) into target/dist/ without tagging or publishing, then test
# install.sh against them.
release-dry-run:
    scripts/hooks/in-snapshot.sh release HEAD -- scripts/release/dry-run.sh "$PWD/target/dist"
    scripts/test-install.sh target/dist

# install.sh end to end against a release directory, in an env -i sandbox with a temp HOME.
test-install dist="target/dist":
    scripts/test-install.sh {{dist}}

# Build "nomouse Dev.app" (ad-hoc signed, bundle id com.yuyudhan.nomouse.dev) and its zip from
# an optimised development build into target/app/. It runs beside an installed nomouse.app
# with its own ~/.config/nomouse-dev/, Accessibility grant and login item; the release's
# nomouse.app comes only from the release workflow (scripts/release/build.sh, then app.sh).
app:
    NOMOUSE_FLAVOUR=dev cargo build --release -p nomouse
    scripts/release/app.sh target/release/nomouse target/app

# Build this checkout as nomouse.app and install it with scripts/install.sh (macOS), which
# signs it with the stable "nomouse local signing" identity, so Accessibility grants survive
# rebuilds (docs/install.md#updating). Never copy target/release/nomouse over the install.
install-local:
    cargo build --release --locked -p nomouse
    scripts/release/app.sh target/release/nomouse target/install-local
    scripts/install.sh --from target/install-local

# Run a development build (debug: ~/.config/nomouse-dev/, nomouse-dev.sock, its own D-Bus names
# and hotkeys), e.g. `just dev doctor`; no arguments runs the daemon.
dev *args:
    cargo run -p nomouse -- {{args}}

# Install config/dev.example.toml (every option, commented) as the development build's
# ~/.config/nomouse-dev/config.toml. Keeps an existing file unless `--force`, which backs it
# up as config.toml.bak-<time> first. Never touches ~/.config/nomouse/.
dev-config *args:
    scripts/dev-config.sh {{args}}

# Check that a development build bundles as "nomouse Dev.app" with the dev bundle id (macOS).
test-app-dev *args:
    scripts/test-app-dev.sh {{args}}

# Build nomouse.app twice and check its Info.plist, signature, stable requirement and zip.
test-app *args:
    scripts/test-app.sh {{args}}

# Cut release VERSION: bump, changelog, verify, commit, tag, push. Only on the owner's word.
release version:
    scripts/release/release.sh {{version}}
