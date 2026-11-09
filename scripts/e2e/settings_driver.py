#!/usr/bin/env python3
"""Drives the nomouse settings window over AT-SPI (the accessibility bus), as a screen reader
user would, for scripts/e2e-desktop.sh. Never touches anything but the settings app.

Usage:
  settings_driver.py wait [seconds]           wait for the window to be on the bus
  settings_driver.py dump                     print the window's tree: depth, role, name, states
  settings_driver.py has <role> <name>        exit 0 when a widget matches
  settings_driver.py act <role> <name>        run the widget's first action (click, activate,
                                              toggle), exit 1 when it is missing
  settings_driver.py click <role> <name>      click the widget's centre with the pointer
                                              (xdotool), as a user would. GTK 4.6 reports
                                              positions relative to the window: use it in
                                              a main window the window manager put at 0,0
                                              (dialog buttons: act)
  settings_driver.py text <role> <name>       print the widget's text (labels, entries)
  settings_driver.py texts                    print every label's and text box's text
  settings_driver.py value <role> <name>      print the widget's current value (spin, scale)
  settings_driver.py set-value <role> <name> <number>   set a spin button or scale
  settings_driver.py checked <role> <name>    print 1 when checked/pressed, else 0
<name> matches exactly, or as a prefix when it ends with '*'. <role> is an AT-SPI role name
such as "push button", "combo box", "check box", "label", "text", "spin button", "list item",
"list", or '*' for any. Instead of one <role> <name> pair, a chain of pairs finds each inside
the previous one: `act list Sections "list item" Scroll` (the sidebar's Scroll row),
`act "list item" Hints "push button" "*"` (the Hints row's button). GTK 4.6 reports no
"showing" state, so hidden pages are searched too: pick the page first.
"""

import subprocess
import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402

# The window registers under its application id (GTK uses the program name, set to it).
APP_NAMES = ("io.github.yuyudhan.nomouse", "nomouse", "nomouse-settings")


def app():
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        child = desktop.get_child_at_index(i)
        if child is not None and (child.get_name() or "") in APP_NAMES:
            return child
    return None


def walk(node, depth=0):
    if node is None or depth > 60:
        return
    yield depth, node
    try:
        count = node.get_child_count()
    except Exception:  # noqa: BLE001 - a widget gone mid-walk
        return
    for i in range(count):
        try:
            child = node.get_child_at_index(i)
        except Exception:  # noqa: BLE001
            continue
        yield from walk(child, depth + 1)


def matches(node, role, name):
    try:
        node_role = node.get_role_name()
        node_name = node.get_name() or ""
    except Exception:  # noqa: BLE001
        return False
    if role != "*" and node_role != role:
        return False
    if name.endswith("*"):
        return node_name.startswith(name[:-1])
    return node_name == name


def find(chain):
    """The first widget matching the last (role, name) pair inside the ones before it."""
    node = app()
    for role, name in zip(chain[::2], chain[1::2]):
        node = next((n for _, n in walk(node) if n is not node and matches(n, role, name)), None)
        if node is None:
            return None
    return node


def need(chain):
    node = find(chain)
    if node is None:
        print(f"no widget {' > '.join(chain)}", file=sys.stderr)
        sys.exit(1)
    return node


def main():
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    command, args = sys.argv[1], sys.argv[2:]
    if command == "wait":
        deadline = time.time() + float(args[0] if args else 15)
        while time.time() < deadline:
            root = app()
            if root is not None and root.get_child_count() > 0:
                return
            time.sleep(0.2)
        print("the settings window never appeared on the accessibility bus", file=sys.stderr)
        sys.exit(1)
    if command == "dump":
        for depth, node in walk(app()):
            states = [s.value_nick for s in node.get_state_set().get_states()]
            keep = [s for s in states if s in ("focused", "checked", "pressed", "selected", "sensitive")]
            print(f"{'  ' * depth}{node.get_role_name()} {node.get_name()!r} {','.join(keep)}")
        return
    if command == "texts":
        for _, node in walk(app()):
            if node.get_role_name() == "label":
                print(node.get_name() or "")
            elif "Text" in node.get_interfaces():
                print(node.get_text(0, node.get_character_count()))
        return
    if command == "set-value":
        args, number = args[:-1], float(args[-1])
    if len(args) < 2 or len(args) % 2:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    label = " > ".join(args)
    if command == "has":
        sys.exit(0 if find(args) is not None else 1)
    node = need(args)
    if command == "act":
        if "Action" not in node.get_interfaces() or node.get_n_actions() == 0:
            print(f"{label} has no action", file=sys.stderr)
            sys.exit(1)
        node.do_action(0)
    elif command == "click":
        # GTK 4.6 offers no grab_focus and no actions on combo rows; a pointer click at the
        # widget's centre works on everything that is on screen.
        extents = node.get_extents(Atspi.CoordType.SCREEN)
        if extents.width <= 0 or extents.height <= 0:
            print(f"{label} is not on screen", file=sys.stderr)
            sys.exit(1)
        x = extents.x + extents.width // 2
        y = extents.y + extents.height // 2
        subprocess.run(["xdotool", "mousemove", str(x), str(y), "click", "1"], check=True)
    elif command == "text":
        if "Text" in node.get_interfaces():
            print(node.get_text(0, node.get_character_count()))
        else:
            print(node.get_name() or "")
    elif command == "value":
        print(node.get_current_value())
    elif command == "set-value":
        if not node.set_current_value(number):
            sys.exit(1)
    elif command == "checked":
        states = node.get_state_set()
        on = states.contains(Atspi.StateType.CHECKED) or states.contains(Atspi.StateType.PRESSED)
        print(1 if on else 0)
    else:
        print(__doc__, file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()
