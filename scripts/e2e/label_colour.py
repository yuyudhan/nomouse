#!/usr/bin/env python3
"""Usage: label_colour.py <without labels.png> <with labels.png>. Prints whether the labels of
the light or dark theme are on screen: "dark", "light" or "other", with the colour.

Most pixels the labels change are the label background, but the app's window also repaints
itself as inactive (GTK's backdrop state) while the overlay holds the keyboard. Adwaita's
colours are warm greys (red > blue), the light and dark themes' label backgrounds neutral
ones, so the verdict takes the most common changed neutral grey."""

import collections
import sys

import gi

gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf  # noqa: E402

before = GdkPixbuf.Pixbuf.new_from_file(sys.argv[1])
after = GdkPixbuf.Pixbuf.new_from_file(sys.argv[2])
step, stride = after.get_n_channels(), after.get_rowstride()
old, new = before.get_pixels(), after.get_pixels()
changed = collections.Counter()
for y in range(min(before.get_height(), after.get_height())):
    row = slice(y * stride, y * stride + after.get_width() * step)
    if old[row] == new[row]:
        continue
    for x in range(row.start, row.stop, step):
        if old[x : x + 3] != new[x : x + 3]:
            changed[tuple(new[x : x + 3])] += 1
greys = collections.Counter({rgb: n for rgb, n in changed.items() if max(rgb) - min(rgb) <= 1})
if not greys:
    print(f"other: no neutral grey among {sum(changed.values())} changed pixels")
    sys.exit(1)
((rgb, count),) = greys.most_common(1)
if max(rgb) < 0x60:
    kind = "dark"
elif min(rgb) > 0xE0:
    kind = "light"
else:
    kind = "other"
total = sum(changed.values())
print(f"{kind} #{rgb[0]:02x}{rgb[1]:02x}{rgb[2]:02x} ({count} of {total} changed pixels)")
