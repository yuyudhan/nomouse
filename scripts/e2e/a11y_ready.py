#!/usr/bin/env python3
"""Startup readiness for scripts/e2e-linux.sh: wait until an app exposes an element over AT-SPI.

Usage: a11y_ready.py <pid> <accessible name> <timeout seconds>

Polls the AT-SPI registry (independently of nomouse) until the application with process id
<pid> has a descendant whose accessible name is <accessible name>, then exits 0; exits 1 on
timeout. Toolkits register and fill their trees asynchronously after the window maps
(Chromium can take seconds), so the e2e script waits for this before its single assertion.
"""

import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402

MAX_DEPTH = 40


def has_name(node, name, depth=0):
    if node is None or depth > MAX_DEPTH:
        return False
    try:
        if node.get_name() == name:
            return True
        count = node.get_child_count()
    except Exception:  # noqa: BLE001 - nodes vanish while the tree is being built
        return False
    for i in range(count):
        try:
            child = node.get_child_at_index(i)
        except Exception:  # noqa: BLE001
            continue
        if has_name(child, name, depth + 1):
            return True
    return False


def ready(pid, name):
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        try:
            app = desktop.get_child_at_index(i)
            if app is not None and app.get_process_id() == pid and has_name(app, name):
                return True
        except Exception:  # noqa: BLE001
            continue
    return False


def main():
    pid, name, timeout = int(sys.argv[1]), sys.argv[2], float(sys.argv[3])
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if ready(pid, name):
            return 0
        time.sleep(0.1)
    print(f"a11y_ready: pid {pid} exposed no {name!r} within {timeout:g}s", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
