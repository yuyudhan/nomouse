#!/usr/bin/env bash
# Renders the logo SVGs in assets/logo/ into the PNG files the packages use, on macOS 14 or later:
#
#   assets/logo/png/nomouse-<size>.png        app icon, 16 to 1024 px (the macOS .icns is built
#                                             from these by scripts/release/app.sh; Linux
#                                             installs them as hicolor icons)
#   assets/logo/png/nomouse-menubar-18.png    macOS menu bar template, 1x and 2x
#   assets/logo/png/nomouse-menubar-36.png
#
# Run it after changing an SVG and commit the PNG files with it.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
logo="${root}/assets/logo"
out="${logo}/png"
mkdir -p "$out"

render() {
    swift "${logo}/render.swift" "$1" "$2" "$3"
}

for size in 16 22 24 32 48 64 128 256 512 1024; do
    render "${logo}/nomouse.svg" "${out}/nomouse-${size}.png" "$size"
done
for size in 18 36; do
    render "${logo}/nomouse-menubar.svg" "${out}/nomouse-menubar-${size}.png" "$size"
done
echo "rendered into ${out}"
