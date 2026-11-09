//! The guided first run on Linux (NM-APP-10): the session's accessibility switch.
//!
//! The switch is optional for the daemon (its start turns it on anyway, see
//! [`crate::platform`]), so `nomouse setup` explains and turns it on, and the daemon never
//! waits for it.

use nomouse_core::setup::{Permission, Presenter, Probe};

use crate::a11y::bus;

/// `org.a11y.Status.IsEnabled`, the switch Qt and Chromium check before exposing their trees.
pub const ACCESSIBILITY_SWITCH: Permission = Permission {
    id: "a11y-switch",
    name: "Accessibility switch (org.a11y.Status.IsEnabled)",
    why: "Qt and Chromium apps expose their buttons and links on the AT-SPI bus only while the \
          session's accessibility switch is on.",
    how: "nomouse turns it on now; restart Qt and Chromium apps that were already running.",
    required: false,
};

/// The permissions the guided first run walks through on Linux.
pub fn first_run_permissions() -> Vec<Permission> {
    vec![ACCESSIBILITY_SWITCH]
}

/// Reads and sets the switch over the session bus.
#[derive(Debug, Default)]
struct SwitchProbe;

impl Probe for SwitchProbe {
    fn granted(&mut self, _permission: &Permission) -> bool {
        bus::accessibility_enabled().unwrap_or(false)
    }

    fn request(&mut self, _permission: &Permission) -> Result<(), String> {
        bus::enable_accessibility().map_err(|error| error.to_string())
    }
}

/// Checks and sets the switch for the guided first run. Without a session bus the request
/// fails with the bus error, which the flow shows.
pub fn first_run_probe() -> Box<dyn Probe> {
    Box::new(SwitchProbe)
}

/// No dialogs on Linux: the first run talks through the terminal.
pub fn first_run_dialogs() -> Option<Box<dyn Presenter>> {
    None
}
