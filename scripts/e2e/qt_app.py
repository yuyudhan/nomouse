#!/usr/bin/env python3
"""Qt 5 test window for scripts/e2e-linux.sh.

Five push buttons "Qt Button 1".."Qt Button 5" set the window title to "qt: Qt Button N" when
clicked; a running click count is appended ("#<n>"). Needs PyQt5 and
QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1 so Qt registers with AT-SPI without a desktop.
"""

import sys

from PyQt5.QtWidgets import QApplication, QGridLayout, QPushButton, QWidget

clicks = 0


def main():
    app = QApplication(sys.argv)
    window = QWidget()
    window.setWindowTitle("nomouse-qt")
    window.resize(700, 300)
    grid = QGridLayout(window)

    def clicked(name):
        global clicks
        clicks += 1
        window.setWindowTitle(f"qt: {name} #{clicks}")

    for i in range(5):
        name = f"Qt Button {i + 1}"
        button = QPushButton(name)
        button.clicked.connect(lambda _checked=False, n=name: clicked(n))
        grid.addWidget(button, 0, i)
    window.show()
    sys.exit(app.exec_())


if __name__ == "__main__":
    main()
