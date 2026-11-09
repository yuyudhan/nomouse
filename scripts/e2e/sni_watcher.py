#!/usr/bin/env python3
"""A stand-in StatusNotifierWatcher (the tray host registry KDE, waybar and GNOME's AppIndicator
extension provide) for the e2e tests, on the private session bus.

Usage: sni_watcher.py <items file> [--no-host]. Each registered item's bus name is appended to
the items file. --no-host reports IsStatusNotifierHostRegistered false (a watcher without a
tray to show items).
"""

import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

NAME = "org.kde.StatusNotifierWatcher"
NODE = Gio.DBusNodeInfo.new_for_xml(
    """
<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
    <method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
  </interface>
</node>
"""
)

items_file = sys.argv[1]
has_host = "--no-host" not in sys.argv[2:]
items = []


def on_call(connection, sender, path, interface, method, params, invocation):
    if method == "RegisterStatusNotifierItem":
        service = params.unpack()[0]
        # An item may register with its object path instead of a bus name.
        name = sender if service.startswith("/") else service
        items.append(name)
        with open(items_file, "a", encoding="utf-8") as out:
            out.write(name + "\n")
        connection.emit_signal(
            None, path, interface, "StatusNotifierItemRegistered", GLib.Variant("(s)", (name,))
        )
    invocation.return_value(None)


def on_get(connection, sender, path, interface, prop):
    if prop == "RegisteredStatusNotifierItems":
        return GLib.Variant("as", items)
    if prop == "IsStatusNotifierHostRegistered":
        return GLib.Variant("b", has_host)
    return GLib.Variant("i", 0)


def on_bus(connection, _name):
    connection.register_object("/StatusNotifierWatcher", NODE.interfaces[0], on_call, on_get, None)


Gio.bus_own_name(Gio.BusType.SESSION, NAME, Gio.BusNameOwnerFlags.NONE, on_bus, None, None)
GLib.MainLoop().run()
