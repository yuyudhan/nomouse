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
    how: "In System Settings > Privacy & Security > Accessibility (Device Control and Data \
          Access on macOS 27 and later), turn nomouse on. If it is not in the list, add it with \
          the + button.",
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

/// What asking macOS for Accessibility takes, behind a trait so the order of the steps is
/// tested without touching the real permission.
pub trait TrustSystem {
    /// Whether this process is trusted for Accessibility now.
    fn trusted(&mut self) -> bool;
    /// The bundle identifier macOS checks this process's grant against: the app's own, when it
    /// was opened as an app. `None` when a terminal started it (the terminal's grant covers it)
    /// or it runs outside a bundle.
    fn own_app_id(&mut self) -> Option<String>;
    /// Deletes the Accessibility entry of `bundle_id` from the list (`tccutil reset`).
    ///
    /// # Errors
    /// What went wrong, for the log.
    fn reset(&mut self, bundle_id: &str) -> Result<(), String>;
    /// Shows macOS's prompt, which adds the running copy to the list, switched off.
    fn prompt(&mut self);
    /// Opens the Accessibility pane.
    ///
    /// # Errors
    /// Worded for the user.
    fn open_settings(&mut self) -> Result<(), String>;
}

/// Asks for Accessibility and opens its pane.
///
/// An untrusted app's old entry is deleted first. macOS keys the grant to the code signature
/// that was allowed, and an ad-hoc signed nomouse has a new one with every version: after an
/// update the list still shows nomouse switched on while macOS refuses the new copy ("Failed to
/// match existing code requirement"), and switching it off and on keeps the old signature.
/// Without the old entry, the prompt adds the running copy, so the switch the user turns on is
/// the one macOS checks.
///
/// # Errors
/// When the pane cannot be opened.
pub fn ask_for_accessibility(system: &mut dyn TrustSystem) -> Result<(), String> {
    if !system.trusted() {
        if let Some(id) = system.own_app_id()
            && let Err(error) = system.reset(&id)
        {
            tracing::warn!("cannot clear the old Accessibility entry of {id}: {error}");
        }
        system.prompt();
    }
    system.open_settings()
}

#[cfg(target_os = "macos")]
pub use live::{MacTrust, TrustProbe};

#[cfg(target_os = "macos")]
mod live {
    use std::io::IsTerminal as _;
    use std::process::Command;

    use nomouse_core::setup::{Permission, Probe};
    use nomouse_sys_macos::{app, ax};

    use super::{ACCESSIBILITY_SETTINGS, TrustSystem, ask_for_accessibility};

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
            ask_for_accessibility(&mut MacTrust)
        }
    }

    /// The real permission, `tccutil` and System Settings.
    #[derive(Debug, Default)]
    pub struct MacTrust;

    impl TrustSystem for MacTrust {
        fn trusted(&mut self) -> bool {
            ax::is_process_trusted(false)
        }

        fn own_app_id(&mut self) -> Option<String> {
            if std::io::stderr().is_terminal() {
                return None;
            }
            app::own_bundle_id()
        }

        fn reset(&mut self, bundle_id: &str) -> Result<(), String> {
            let output = Command::new("/usr/bin/tccutil")
                .args(["reset", "Accessibility", bundle_id])
                .output()
                .map_err(|error| format!("cannot run tccutil: {error}"))?;
            if output.status.success() {
                tracing::info!("cleared the Accessibility entry of {bundle_id}");
                Ok(())
            } else {
                Err(format!(
                    "tccutil failed ({}): {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ))
            }
        }

        fn prompt(&mut self) {
            ax::is_process_trusted(true);
        }

        fn open_settings(&mut self) -> Result<(), String> {
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

    /// Records what [`ask_for_accessibility`] did, in order.
    #[derive(Default)]
    struct FakeTrust {
        trusted: bool,
        app_id: Option<&'static str>,
        reset_fails: bool,
        calls: Vec<String>,
    }

    impl TrustSystem for FakeTrust {
        fn trusted(&mut self) -> bool {
            self.trusted
        }

        fn own_app_id(&mut self) -> Option<String> {
            self.app_id.map(str::to_owned)
        }

        fn reset(&mut self, bundle_id: &str) -> Result<(), String> {
            self.calls.push(format!("reset {bundle_id}"));
            if self.reset_fails {
                Err("tccutil failed".to_owned())
            } else {
                Ok(())
            }
        }

        fn prompt(&mut self) {
            self.calls.push("prompt".to_owned());
        }

        fn open_settings(&mut self) -> Result<(), String> {
            self.calls.push("open".to_owned());
            Ok(())
        }
    }

    #[test]
    fn an_untrusted_app_clears_its_old_entry_before_the_prompt_adds_this_copy() {
        let mut system = FakeTrust {
            app_id: Some("com.yuyudhan.nomouse"),
            ..FakeTrust::default()
        };
        assert_eq!(ask_for_accessibility(&mut system), Ok(()));
        assert_eq!(
            system.calls,
            vec!["reset com.yuyudhan.nomouse", "prompt", "open"]
        );
    }

    #[test]
    fn a_failed_reset_still_prompts_and_opens_the_pane() {
        let mut system = FakeTrust {
            app_id: Some("com.yuyudhan.nomouse"),
            reset_fails: true,
            ..FakeTrust::default()
        };
        assert_eq!(ask_for_accessibility(&mut system), Ok(()));
        assert_eq!(
            system.calls,
            vec!["reset com.yuyudhan.nomouse", "prompt", "open"]
        );
    }

    #[test]
    fn without_an_own_app_identity_nothing_is_reset() {
        let mut system = FakeTrust::default();
        assert_eq!(ask_for_accessibility(&mut system), Ok(()));
        assert_eq!(system.calls, vec!["prompt", "open"]);
    }

    #[test]
    fn a_trusted_app_keeps_its_grant_and_only_opens_the_pane() {
        let mut system = FakeTrust {
            trusted: true,
            app_id: Some("com.yuyudhan.nomouse"),
            ..FakeTrust::default()
        };
        assert_eq!(ask_for_accessibility(&mut system), Ok(()));
        assert_eq!(system.calls, vec!["open"]);
    }
}
