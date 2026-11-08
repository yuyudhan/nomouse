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

# shellcheck over every tracked shell script and git hook.
shellcheck:
    git ls-files -z -- '*.sh' '.githooks/*' | xargs -0 shellcheck --severity=style

# Clippy with -D warnings for the host only.
clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Clippy with -D warnings for every target triple.
clippy-all:
    for target in {{targets}}; do cargo clippy --workspace --all-targets --all-features --target "$target" -- -D warnings; done

# Fast checks on the host: what the pre-commit hook runs on the staged tree.
pre-commit: fmt-check guards typos shellcheck clippy

# Format check, guards, typos, shellcheck and clippy for every target.
check: fmt-check guards typos shellcheck clippy-all

# Unit, integration and property tests.
test:
    cargo test --workspace --all-features

# Licences, advisories, duplicate dependencies, sources.
deny:
    cargo deny check

# The full local gate (what the pre-push hook runs on each pushed commit): check, test, deny.
verify: check test deny

# Use the repo's git hooks (pre-commit, commit-msg, pre-push). Safe to run again.
install-hooks:
    git config core.hooksPath .githooks
    @echo "hooks: core.hooksPath = $(git config core.hooksPath)"

# Build every Linux release asset into target/dist/ without tagging or publishing.
release-dry-run:
    scripts/release/dry-run.sh

# Cut release VERSION: bump, changelog, verify, commit, tag, push. Only on the owner's word.
release version:
    scripts/release/release.sh {{version}}
