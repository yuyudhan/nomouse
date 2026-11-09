#!/usr/bin/env bash
# Renders the hicolor PNG icons from the scalable SVG, after the SVG changes. The PNG files are
# committed, so releases need no SVG renderer.
#
# Usage: packaging/linux/icons.sh
#   Needs python3 with PyGObject and GdkPixbuf's SVG loader (librsvg), as on any GNOME desktop:
#   Debian/Ubuntu python3-gi, gir1.2-gdkpixbuf-2.0, librsvg2-common.
set -euo pipefail

hicolor="$(cd "$(dirname "$0")" && pwd)/share/icons/hicolor"
name="io.github.yuyudhan.nomouse"
# The sizes install.sh installs and removes (icon_sizes there).
sizes="16 22 24 32 48 64 128 256 512"

# shellcheck disable=SC2086 # one argument per size
python3 - "${hicolor}/scalable/apps/${name}.svg" "$hicolor" "$name" $sizes <<'EOF'
import os, sys
import gi
gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf
svg, hicolor, name, *sizes = sys.argv[1:]
for size in map(int, sizes):
    folder = os.path.join(hicolor, f"{size}x{size}", "apps")
    os.makedirs(folder, exist_ok=True)
    out = os.path.join(folder, f"{name}.png")
    GdkPixbuf.Pixbuf.new_from_file_at_size(svg, size, size).savev(out, "png", [], [])
    print(out)
EOF
