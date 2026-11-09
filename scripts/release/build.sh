#!/usr/bin/env bash
# Builds the release nomouse binary for one target triple and prints its path. On Linux it also
# builds nomouse-settings (the GTK 4 settings window, which needs libgtk-4-dev and
# libadwaita-1-dev) into the same folder.
#
# Usage: scripts/release/build.sh TARGET
#
# Linux targets build on a machine of that architecture: nomouse-settings links the system's
# GTK 4 and libadwaita, which a cross linker cannot find (the release workflow uses an arm64
# runner for aarch64). macOS and Windows targets build with cargo from any host that has their
# toolchain.
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

if [[ "$target" == *-linux-* ]] && [ "$target" != "$host" ]; then
    echo "build: ${target} links GTK 4 for the settings window: build it on a ${target%%-*} Linux machine (this is ${host})" >&2
    exit 1
fi
cargo build "${args[@]}" >&2

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
