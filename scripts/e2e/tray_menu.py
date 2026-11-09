#!/usr/bin/env python3
"""Reads and clicks the nomouse tray menu over D-Bus, as a tray host does.

Usage:
  tray_menu.py <bus name> labels            print each menu row: id, enabled, toggle, label
  tray_menu.py <bus name> click <label>     click the row whose label starts with <label>
  tray_menu.py <bus name> item <property>   print a StatusNotifierItem property
Exits 1 when the row is missing or disabled.
"""

import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

name, command = sys.argv[1], sys.argv[2]
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)


def call(path, interface, method, args, reply):
    return bus.call_sync(
        name, path, interface, method, args, GLib.VariantType(reply), Gio.DBusCallFlags.NONE, 3000, None
    ).unpack()


def rows():
    _revision, (_root, _props, children) = call(
        "/MenuBar", "com.canonical.dbusmenu", "GetLayout", GLib.Variant("(iias)", (0, -1, [])), "(u(ia{sv}av))"
    )
    for child in children:
        ident, props, _ = child
        yield ident, props


if command == "labels":
    for ident, props in rows():
        if props.get("type") == "separator":
            print(f"{ident}\t-\t-\t----")
            continue
        toggle = props.get("toggle-state", "-") if "toggle-type" in props else "-"
        print(f"{ident}\t{int(props.get('enabled', True))}\t{toggle}\t{props.get('label', '')}")
elif command == "click":
    wanted = sys.argv[3]
    for ident, props in rows():
        if props.get("label", "").startswith(wanted):
            if not props.get("enabled", True):
                print(f"{wanted!r} is disabled", file=sys.stderr)
                sys.exit(1)
            call(
                "/MenuBar",
                "com.canonical.dbusmenu",
                "Event",
                GLib.Variant("(isvu)", (ident, "clicked", GLib.Variant("i", 0), 0)),
                "()",
            )
            sys.exit(0)
    print(f"no row starts with {wanted!r}", file=sys.stderr)
    sys.exit(1)
elif command == "item":
    (value,) = call(
        "/StatusNotifierItem",
        "org.freedesktop.DBus.Properties",
        "Get",
        GLib.Variant("(ss)", ("org.kde.StatusNotifierItem", sys.argv[3])),
        "(v)",
    )
    print(value)
else:
    print(__doc__, file=sys.stderr)
    sys.exit(2)
