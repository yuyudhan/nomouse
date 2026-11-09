#!/usr/bin/env bash
# Packs one built binary into the release archive the installers download, plus its .sha256.
#
# Usage: scripts/release/package.sh NAME BINARY OUTDIR
#   NAME     asset name: a target triple or universal-apple-darwin
#   BINARY   the built nomouse (or nomouse.exe)
#   OUTDIR   where nomouse-NAME.tar.gz (or .zip for Windows) and its .sha256 go
#
# The archive holds one folder, nomouse-NAME/, with the binary, README.md, CHANGELOG.md,
# LICENSE-MIT and LICENSE-APACHE; Linux archives also hold share/ from packaging/linux/share
# (the app launcher entry and its hicolor icons, which install.sh and the .deb install). Asset
# names carry no version, so releases/latest/download/<asset> always resolves.
set -euo pipefail

if [ "$#" -ne 3 ]; then
    echo "usage: $0 NAME BINARY OUTDIR" >&2
    exit 2
fi
name="$1"
binary="$2"
root="$(cd "$(dirname "$0")/../.." && pwd)"
mkdir -p "$3"
outdir="$(cd "$3" && pwd)"
[ -f "$binary" ] || {
    echo "package: no binary at ${binary}" >&2
    exit 1
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
folder="nomouse-${name}"
mkdir -p "${work}/${folder}"
cp "$binary" "${work}/${folder}/"
cp "${root}/README.md" "${root}/CHANGELOG.md" "${root}/LICENSE-MIT" "${root}/LICENSE-APACHE" \
    "${work}/${folder}/"
if [[ "$name" == *-linux-* ]]; then
    cp -R "${root}/packaging/linux/share" "${work}/${folder}/"
fi
chmod 755 "${work}/${folder}/$(basename "$binary")"

if [[ "$name" == *-windows-* ]]; then
    archive="${folder}.zip"
    rm -f "${outdir}/${archive}"
    if command -v 7z >/dev/null 2>&1; then
        (cd "$work" && 7z a -tzip -bso0 "${outdir}/${archive}" "$folder")
    else
        (cd "$work" && zip -qr "${outdir}/${archive}" "$folder")
    fi
else
    archive="${folder}.tar.gz"
    tar -C "$work" -czf "${outdir}/${archive}" "$folder"
fi

"${root}/scripts/release/checksum.sh" "${outdir}/${archive}"
echo "package: ${outdir}/${archive}" >&2
