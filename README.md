# homerow-linux

Keyboard-driven clicking for Linux, like [Homerow](https://www.homerow.app/) on macOS: press a shortcut, see hint labels over clickable elements, type a label to click it.

## Status

Planning. Nothing is implemented yet.

## Planned v1 stack (pending confirmation)

- Rust, using the [`atspi`](https://crates.io/crates/atspi) crate to read the AT-SPI accessibility tree
- X11 and wlroots-based Wayland compositors first
- Hints only for elements exposed in the accessibility tree

## License

To be decided.
