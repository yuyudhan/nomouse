#!/usr/bin/env python3
"""GTK 3 test window for scripts/e2e-linux.sh.

Ten buttons "Button 1".."Button 10" set the window title to "left: Button N" on a click
("ctrl-left: Button N" when Control was held) and "right: Button N" on a right click; a
scrolled list of rows sets it to "scrolled: <offset>" when scrolled. A running click count is
appended ("#<n>") so repeated clicks are visible.
"""

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, Gtk  # noqa: E402

clicks = 0


def set_title(window, text):
    global clicks
    clicks += 1
    window.set_title(f"{text} #{clicks}")


def on_clicked(window, name):
    ok, state = Gtk.get_current_event_state()
    held = ok and bool(state & Gdk.ModifierType.CONTROL_MASK)
    set_title(window, f"{'ctrl-left' if held else 'left'}: {name}")


def main():
    window = Gtk.Window(title="nomouse-e2e")
    window.set_default_size(900, 700)
    window.connect("destroy", Gtk.main_quit)
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
    grid = Gtk.Grid(column_spacing=8, row_spacing=8)
    for i in range(10):
        name = f"Button {i + 1}"
        button = Gtk.Button(label=name)
        button.connect("clicked", lambda _b, n=name: on_clicked(window, n))

        def on_press(_b, event, n=name):
            if event.button == 3:
                set_title(window, f"right: {n}")
            return False

        button.connect("button-press-event", on_press)
        grid.attach(button, i % 5, i // 5, 1, 1)
    box.pack_start(grid, False, False, 0)
    scrolled = Gtk.ScrolledWindow()
    scrolled.set_min_content_height(300)
    rows = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
    for i in range(60):
        rows.pack_start(Gtk.Button(label=f"Row {i + 1}"), False, False, 0)
    scrolled.add(rows)
    adjustment = scrolled.get_vadjustment()
    adjustment.connect(
        "value-changed", lambda a: set_title(window, f"scrolled: {int(a.get_value())}")
    )
    box.pack_start(scrolled, True, True, 0)
    window.add(box)
    window.show_all()
    Gtk.main()


if __name__ == "__main__":
    main()
