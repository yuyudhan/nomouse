#!/usr/bin/env bash
# Builds the release nomouse binary for one target triple and prints its path. On Linux it also
# builds nomouse-settings (the GTK 4 settings window, which needs libgtk-4-dev and
# libadwaita-1-dev) into the same folder.
#
# Usage: scripts/release/build.sh TARGET
#
# Native targets build with cargo. aarch64-unknown-linux-gnu from another machine links with
# aarch64-linux-gnu-gcc when installed (CI installs it), else with cargo-zigbuild (needs zig).
set -euo pipefail

target="${1:-}"
[ -n "$target" ] || {
    echo "usage: $0 TARGET" >&2
    exit 2
}
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

rustup target add "$target" >/dev/null 2>&1 || true
host="$(rustc -vV | sed -n 's/^host: //p')"
args=(--release --locked --package nomouse --target "$target")
[[ "$target" == *-linux-* ]] && args+=(--package nomouse-settings)

if [ "$target" = "$host" ] || [[ "$target" == *-apple-darwin ]] || [[ "$target" == *-windows-* ]]; then
    cargo build "${args[@]}" >&2
elif [ "$target" = "aarch64-unknown-linux-gnu" ] && command -v aarch64-linux-gnu-gcc >/dev/null 2>&1; then
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc cargo build "${args[@]}" >&2
elif command -v cargo-zigbuild >/dev/null 2>&1; then
    cargo zigbuild "${args[@]}" >&2
else
    echo "build: no linker for ${target}: install aarch64-linux-gnu-gcc or cargo-zigbuild + zig" >&2
    exit 1
fi

target_dir="${CARGO_TARGET_DIR:-${root}/target}"
binary="${target_dir}/${target}/release/nomouse"
[[ "$target" == *-windows-* ]] && binary="${binary}.exe"
[ -f "$binary" ] || {
    echo "build: ${binary} was not produced" >&2
    exit 1
}
if [[ "$target" == *-linux-* ]] && [ ! -f "$(dirname "$binary")/nomouse-settings" ]; then
    echo "build: $(dirname "$binary")/nomouse-settings was not produced" >&2
    exit 1
fi
echo "$binary"
