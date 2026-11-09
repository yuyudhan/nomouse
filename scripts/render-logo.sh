#!/usr/bin/env bash
# Renders the logo SVGs in assets/logo/ into the PNG files the packages use, on macOS 14 or later:
#
#   assets/logo/png/nomouse-<size>.png        app icon, 16 to 1024 px (the macOS .icns is built
#                                             from these by scripts/release/app.sh)
#   assets/logo/png/nomouse-menubar-18.png    macOS menu bar template, 1x and 2x (compiled into
#   assets/logo/png/nomouse-menubar-36.png    the binary)
#
# and copies the Linux app launcher icons into packaging/linux/share/icons/hicolor (the
# tarball, install.sh and the .deb install them): the app icon as scalable SVG and 16 to
# 512 px PNG files, and the monochrome symbolic SVG for panels and trays.
#
# Run it after changing an SVG and commit the PNG files with it.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
logo="${root}/assets/logo"
out="${logo}/png"
hicolor="${root}/packaging/linux/share/icons/hicolor"
app_id="io.github.yuyudhan.nomouse"
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

# The sizes install.sh installs and removes (icon_sizes there).
for size in 16 22 24 32 48 64 128 256 512; do
    mkdir -p "${hicolor}/${size}x${size}/apps"
    cp "${out}/nomouse-${size}.png" "${hicolor}/${size}x${size}/apps/${app_id}.png"
done
mkdir -p "${hicolor}/scalable/apps" "${hicolor}/symbolic/apps"
cp "${logo}/nomouse.svg" "${hicolor}/scalable/apps/${app_id}.svg"
cp "${logo}/nomouse-symbolic.svg" "${hicolor}/symbolic/apps/${app_id}-symbolic.svg"
echo "rendered into ${out} and ${hicolor}"
