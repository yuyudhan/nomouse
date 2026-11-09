#!/usr/bin/env python3
"""GTK 3 test window for scripts/e2e-linux.sh.

Ten buttons "Button 1".."Button 10" set the window title to "left: Button N" on a click
("ctrl-left: Button N" when Control was held) and "right: Button N" on a right click; a
button with no label, whose only text is its tooltip "Keep a copy", sets it to
"left: tooltip button". Below them, a large scrolled list of 200 rows sets it to
"scrolled: <offset>" when scrolled, and a narrow list of 40 items beside it to
"scrolled 2: <offset>"; " (top)" or " (bottom)" follows the offset at either end. A running
click count is appended ("#<n>") so repeated clicks are visible.
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


def scrolled_list(window, name, count, title):
    """A scrolled column of `count` buttons "<name> N"; scrolling sets the title to
    "<title>: <offset>", with " (top)" or " (bottom)" at either end."""
    scrolled = Gtk.ScrolledWindow()
    scrolled.set_min_content_height(300)
    rows = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
    for i in range(count):
        rows.pack_start(Gtk.Button(label=f"{name} {i + 1}"), False, False, 0)
    scrolled.add(rows)

    def changed(adjustment):
        value = adjustment.get_value()
        end = ""
        if value <= 0:
            end = " (top)"
        elif value >= adjustment.get_upper() - adjustment.get_page_size() - 0.5:
            end = " (bottom)"
        set_title(window, f"{title}: {int(value)}{end}")

    scrolled.get_vadjustment().connect("value-changed", changed)
    return scrolled


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
    # A drawing as the only child: GTK derives no accessible name from it (an icon would
    # give one, such as "Save"), so the tooltip, exposed as the description, is the only text.
    tooltip_only = Gtk.Button()
    drawing = Gtk.DrawingArea()
    drawing.set_size_request(24, 24)
    tooltip_only.add(drawing)
    tooltip_only.set_tooltip_text("Keep a copy")
    tooltip_only.connect("clicked", lambda _b: on_clicked(window, "tooltip button"))
    row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
    row.pack_start(tooltip_only, False, False, 0)
    box.pack_start(row, False, False, 0)
    lists = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
    lists.pack_start(scrolled_list(window, "Row", 200, "scrolled"), True, True, 0)
    second = scrolled_list(window, "Item", 40, "scrolled 2")
    second.set_size_request(160, -1)
    lists.pack_start(second, False, False, 0)
    box.pack_start(lists, True, True, 0)
    window.add(box)
    window.show_all()
    Gtk.main()


if __name__ == "__main__":
    main()
