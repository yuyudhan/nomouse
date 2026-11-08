//! The shared backend contract against the real X11 + AT-SPI backend.
//!
//! Ignored by default because it needs a live session; `scripts/e2e-linux.sh` runs it with
//! `--ignored` on its private Xvfb, i3 and AT-SPI bus.
#![cfg(target_os = "linux")]

use nomouse_core::Config;
use nomouse_core::contract::{Probe, assert_platform};

#[test]
#[ignore = "needs a live X11 session with AT-SPI; run by scripts/e2e-linux.sh"]
fn the_x11_backend_passes_the_contract() {
    let mut platform = nomouse_linux::platform(&Config::default()).expect("backend starts");
    assert_platform(platform.as_mut(), Probe::default());
}
