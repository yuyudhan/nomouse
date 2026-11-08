#!/usr/bin/env bash
# Packs the already-built Linux binary for TARGET into a .deb with cargo-deb (metadata in
# crates/nomouse/Cargo.toml [package.metadata.deb]) and writes its .sha256.
#
# Usage: scripts/release/deb.sh TARGET OUTDIR
#   Run scripts/release/build.sh TARGET first. The .deb is named nomouse_<arch>.deb so
#   releases/latest/download/ resolves it; `dpkg -i` installs it system-wide (that one needs
#   root - the no-sudo path is install.sh).
set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "usage: $0 TARGET OUTDIR" >&2
    exit 2
fi
target="$1"
root="$(cd "$(dirname "$0")/../.." && pwd)"
mkdir -p "$2"
outdir="$(cd "$2" && pwd)"
case "$target" in
    x86_64-unknown-linux-gnu) arch=amd64 ;;
    aarch64-unknown-linux-gnu) arch=arm64 ;;
    *)
        echo "deb: no Debian architecture for ${target}" >&2
        exit 1
        ;;
esac
command -v cargo-deb >/dev/null 2>&1 || {
    echo "deb: cargo-deb is missing: cargo install cargo-deb --locked" >&2
    exit 1
}

cd "$root"
# --no-strip: the release profile already strips; cross strip tools may be absent.
cargo deb --package nomouse --target "$target" --no-build --no-strip \
    --output "${outdir}/nomouse_${arch}.deb" >&2
"${root}/scripts/release/checksum.sh" "${outdir}/nomouse_${arch}.deb"
echo "deb: ${outdir}/nomouse_${arch}.deb" >&2
