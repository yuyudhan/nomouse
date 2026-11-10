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
#
# macOS targets build for macOS 11 (MACOSX_DEPLOYMENT_TARGET, the bundle's LSMinimumSystemVersion)
# and the binary's LC_BUILD_VERSION minos is checked against it. Windows MSVC targets link the C
# runtime statically (+crt-static), so nomouse.exe needs no Visual C++ Redistributable.
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

# Keep in step with LSMinimumSystemVersion in packaging/macos/Info.plist.in.
macos_minimum="11.0"
if [[ "$target" == *-apple-darwin ]]; then
    export MACOSX_DEPLOYMENT_TARGET="$macos_minimum"
    echo "build: MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET}" >&2
fi
if [[ "$target" == *-pc-windows-msvc ]]; then
    # RUSTFLAGS with --target applies to the target only, not to build scripts.
    export RUSTFLAGS="${RUSTFLAGS:+${RUSTFLAGS} }-C target-feature=+crt-static"
    echo "build: RUSTFLAGS=${RUSTFLAGS}" >&2
fi

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
if [[ "$target" == *-apple-darwin ]]; then
    minos="$(otool -l "$binary" | awk '$1 == "cmd" { build = ($2 == "LC_BUILD_VERSION") } build && $1 == "minos" { print $2; exit }')"
    if [ "$minos" != "$macos_minimum" ]; then
        echo "build: ${binary} has minos '${minos}', expected ${macos_minimum}" >&2
        exit 1
    fi
    echo "build: ${binary} runs on macOS ${minos} and later" >&2
fi
if [[ "$target" == *-pc-windows-msvc ]]; then
    # A dynamically linked C runtime imports VCRUNTIME140.dll and the UCRT's api-ms-win-crt-*
    # sets; their names are plain ASCII in the import table.
    if LC_ALL=C grep -qaiE 'vcruntime[0-9]+\.dll|api-ms-win-crt-' "$binary"; then
        echo "build: ${binary} still imports the Visual C++ runtime (VCRUNTIME/UCRT DLLs)" >&2
        exit 1
    fi
    echo "build: ${binary} links the C runtime statically (no VCRUNTIME140.dll import)" >&2
fi
echo "$binary"
