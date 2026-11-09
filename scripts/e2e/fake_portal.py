#!/usr/bin/env python3
"""A stand-in XDG desktop portal for the e2e tests: answers
org.freedesktop.portal.Settings ReadOne/Read for org.freedesktop.appearance color-scheme.

Usage: fake_portal.py <state file>. The file holds the color-scheme number (0 no preference,
1 dark, 2 light), re-read on every call; a second word "old" makes the portal answer only the
pre-version-2 Read method (a variant inside a variant), as old portals do.
"""

import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

NODE = Gio.DBusNodeInfo.new_for_xml(
    """
<node>
  <interface name="org.freedesktop.portal.Settings">
    <method name="ReadOne">
      <arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/>
    </method>
    <method name="Read">
      <arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/>
    </method>
    <property name="version" type="u" access="read"/>
  </interface>
</node>
"""
)
STATE = sys.argv[1]


def state():
    try:
        with open(STATE, encoding="utf-8") as handle:
            words = handle.read().split()
        return int(words[0]), "old" in words[1:]
    except (OSError, ValueError, IndexError):
        return 0, False


def call(_conn, _sender, _path, _iface, method, params, invocation):
    namespace, key = params.unpack()
    scheme, old = state()
    if method == "ReadOne" and old:
        invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "no ReadOne")
        return
    if (namespace, key) != ("org.freedesktop.appearance", "color-scheme"):
        invocation.return_dbus_error(
            "org.freedesktop.portal.Error.NotFound", f"{namespace} {key} not found"
        )
        return
    value = GLib.Variant("u", scheme)
    if method == "Read":
        value = GLib.Variant("v", value)
    invocation.return_value(GLib.Variant("(v)", (value,)))


def get_property(*_args):
    return GLib.Variant("u", 1 if state()[1] else 2)


def acquired(conn, _name):
    conn.register_object(
        "/org/freedesktop/portal/desktop", NODE.interfaces[0], call, get_property, None
    )


Gio.bus_own_name(
    Gio.BusType.SESSION,
    "org.freedesktop.portal.Desktop",
    Gio.BusNameOwnerFlags.NONE,
    acquired,
    None,
    None,
)
GLib.MainLoop().run()
