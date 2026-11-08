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

# Clippy with -D warnings for the host only.
clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Clippy with -D warnings for every target triple.
clippy-all:
    for target in {{targets}}; do cargo clippy --workspace --all-targets --all-features --target "$target" -- -D warnings; done

# Everything the pre-commit hook runs.
check: fmt-check guards typos clippy-all

# Unit, integration and property tests.
test:
    cargo test --workspace --all-features

# Licences, advisories, duplicate dependencies, sources.
deny:
    cargo deny check

# The full local gate: check, test, deny.
verify: check test deny

# Use the repo's git hooks.
install-hooks:
    git config core.hooksPath .githooks
