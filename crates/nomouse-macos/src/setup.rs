//! The guided first run on macOS (NM-APP-10). Accessibility is the one permission nomouse
//! needs: it covers reading other apps' controls, the keyboard event tap and posting clicks.
//! Input Monitoring is not needed because the tap runs under Accessibility.

use nomouse_core::setup::Permission;

/// The Accessibility permission.
pub const ACCESSIBILITY: Permission = Permission {
    id: "accessibility",
    name: "Accessibility",
    why: "nomouse reads the buttons, links and fields of the app in front, catches its hotkeys \
          and clicks for you. macOS allows that only to apps you trust with Accessibility.",
    how: "In System Settings > Privacy & Security > Accessibility, turn nomouse on. If it is not \
          in the list, add it with the + button.",
    required: true,
};

/// System Settings' Accessibility pane.
pub const ACCESSIBILITY_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

/// The permissions nomouse needs on macOS, in the order the first run asks for them.
pub fn permissions() -> Vec<Permission> {
    vec![ACCESSIBILITY]
}

/// Dialog text and buttons for one step of the flow, or `None` when the step needs no dialog
/// (it is logged instead).
pub fn dialog(step: &nomouse_core::setup::Step) -> Option<Dialog> {
    use nomouse_core::setup::Step;
    match step {
        Step::Explain(permission) => Some(Dialog {
            title: format!("nomouse needs {}", permission.name),
            message: format!(
                "{}\n\n{}\n\nnomouse opens that pane now and starts by itself once you allow it.",
                permission.why, permission.how
            ),
            buttons: vec!["Open System Settings", "Quit"],
        }),
        Step::TimedOut(permission) => Some(Dialog {
            title: format!("nomouse still has no {} access", permission.name),
            message: format!(
                "{}\n\nnomouse quits now. Open it again after allowing it.",
                permission.how
            ),
            buttons: vec!["Quit"],
        }),
        Step::RequestFailed(permission, reason) => Some(Dialog {
            title: format!("nomouse could not ask for {}", permission.name),
            message: format!("{reason}\n\n{}", permission.how),
            buttons: vec!["Quit"],
        }),
        Step::AlreadyGranted(_)
        | Step::Requested(_)
        | Step::Waiting(_)
        | Step::Granted(_)
        | Step::Done { .. } => None,
    }
}

/// A modal dialog: title, text and buttons (the first is the default).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
    /// Bold first line.
    pub title: String,
    /// Explanation below it.
    pub message: String,
    /// Button titles, default first.
    pub buttons: Vec<&'static str>,
}

#[cfg(target_os = "macos")]
pub use live::TrustProbe;

#[cfg(target_os = "macos")]
mod live {
    use std::process::Command;

    use nomouse_core::setup::{Permission, Probe};
    use nomouse_sys_macos::ax;

    use super::ACCESSIBILITY_SETTINGS;

    /// Accessibility trust of this process.
    #[derive(Debug, Default)]
    pub struct TrustProbe;

    impl Probe for TrustProbe {
        fn granted(&mut self, _permission: &Permission) -> bool {
            ax::is_process_trusted(false)
        }

        /// Asks with macOS's own prompt, which also adds nomouse to the Accessibility list so
        /// the user only flips its switch, then opens that pane.
        fn request(&mut self, _permission: &Permission) -> Result<(), String> {
            ax::is_process_trusted(true);
            let status = Command::new("/usr/bin/open")
                .arg(ACCESSIBILITY_SETTINGS)
                .status()
                .map_err(|error| format!("cannot open System Settings: {error}"))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("cannot open System Settings ({status})"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use nomouse_core::setup::Step;

    use super::*;

    #[test]
    fn explaining_offers_settings_first_and_quit() {
        let shown = dialog(&Step::Explain(ACCESSIBILITY)).unwrap();
        assert_eq!(shown.buttons, vec!["Open System Settings", "Quit"]);
        assert!(shown.title.contains("Accessibility"));
        assert!(shown.message.contains(ACCESSIBILITY.why));
        assert!(shown.message.contains(ACCESSIBILITY.how));
    }

    #[test]
    fn stopping_explains_what_to_do_and_only_quits() {
        for step in [
            Step::TimedOut(ACCESSIBILITY),
            Step::RequestFailed(ACCESSIBILITY, "no System Settings".to_owned()),
        ] {
            let shown = dialog(&step).unwrap();
            assert_eq!(shown.buttons, vec!["Quit"]);
            assert!(shown.message.contains(ACCESSIBILITY.how));
        }
    }

    #[test]
    fn progress_steps_show_no_dialog() {
        for step in [
            Step::AlreadyGranted(ACCESSIBILITY),
            Step::Requested(ACCESSIBILITY),
            Step::Waiting(ACCESSIBILITY),
            Step::Granted(ACCESSIBILITY),
            Step::Done { restart: true },
        ] {
            assert_eq!(dialog(&step), None);
        }
    }
}
