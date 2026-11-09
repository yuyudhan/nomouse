#!/usr/bin/env python3
"""Reads `swaymsg -t get_tree` JSON on stdin for scripts/e2e-wayland.sh.

    sway_tree.py field <xdg_shell|xwayland> <key>   the key (pid, name, ...) of that window
    sway_tree.py pid <pid> <key>                     the key (name, ...) of that process's window
    sway_tree.py layout                              one line per application window
"""

import json
import sys


def windows(node):
    if node.get("pid"):
        yield node
    for child in node.get("nodes", []) + node.get("floating_nodes", []):
        yield from windows(child)


def main():
    tree = json.load(sys.stdin)
    if sys.argv[1] == "field":
        shell, key = sys.argv[2], sys.argv[3]
        for window in windows(tree):
            if window.get("shell") == shell:
                sys.stdout.write(f"{window[key]}\n")
                return
    elif sys.argv[1] == "pid":
        for window in windows(tree):
            if window["pid"] == int(sys.argv[2]):
                sys.stdout.write(f"{window[sys.argv[3]]}\n")
                return
    elif sys.argv[1] == "layout":
        for window in windows(tree):
            rect, inner = window["rect"], window["window_rect"]
            sys.stdout.write(
                f"  {window['shell']:9} pid {window['pid']}: container "
                f"{rect['x']},{rect['y']} {rect['width']}x{rect['height']}, "
                f"content offset {inner['x']},{inner['y']}\n"
            )


if __name__ == "__main__":
    main()
