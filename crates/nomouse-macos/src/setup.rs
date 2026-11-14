//! The guided first run on macOS (NM-APP-10). Accessibility is the one permission nomouse
//! needs: it covers reading other apps' controls, the keyboard event tap and posting clicks.
//! Input Monitoring is not needed because the tap runs under Accessibility.

use nomouse_core::Flavour;
use nomouse_core::setup::Permission;

/// This build's app bundle as the grant messages name it: `nomouse.app`, or `nomouse Dev.app`
/// for a development build (its own bundle id, so its own Accessibility entry).
pub const APP: &str = match Flavour::CURRENT {
    Flavour::Release => "nomouse.app",
    Flavour::Dev => "nomouse Dev.app",
};

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

/// System Settings' Accessibility pane. The legacy pane id still opens it on macOS 27: the
/// Privacy & Security extension declares `legacyBundleIdentifier` `com.apple.preference.security`
/// and handles the `Privacy_Accessibility` anchor (its list is titled Device Control and Data
/// Access there).
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

/// Whose Accessibility grant a trust check in this process measures. macOS attributes a process
/// started from a terminal to that terminal, so its check says nothing about nomouse.app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantHolder {
    /// nomouse.app opened as an app: its own entry in the list.
    App,
    /// A terminal started this process; its name from `TERM_PROGRAM`, when set.
    Terminal(Option<String>),
    /// A nomouse binary outside the app, started without a terminal (launchd, a script).
    Binary,
}

/// The doctor's Accessibility line: who was checked, and what to do when it is not trusted.
///
/// # Errors
/// The problem and its fix when `holder` is not trusted.
pub fn accessibility_check(trusted: bool, holder: &GrantHolder) -> Result<String, String> {
    let requirement = crate::ACCESSIBILITY_REQUIREMENT;
    match (holder, trusted) {
        (GrantHolder::App, true) => Ok(format!("{APP} is trusted")),
        (GrantHolder::App, false) => Err(format!(
            "{APP} is not trusted. {requirement} If nomouse is already switched on there, \
             that switch is for an older copy: choose Open System Settings in nomouse's menu or \
             Welcome window, which replaces it, then switch nomouse on."
        )),
        (GrantHolder::Terminal(name), true) => Ok(format!(
            "{} is trusted, so a nomouse started from it works. {APP} has its own grant; \
             the `{APP_CHECK}` line shows it.",
            terminal(name.as_deref())
        )),
        (GrantHolder::Terminal(name), false) => Err(format!(
            "{} is not trusted, which matters only for a nomouse started from it. {APP} \
             has its own grant: open /Applications/{APP} and its menu shows whether it is \
             allowed. To run nomouse from this terminal, allow the terminal in the same list and \
             open it again.",
            terminal(name.as_deref())
        )),
        (GrantHolder::Binary, true) => Ok("this nomouse binary is trusted".to_owned()),
        (GrantHolder::Binary, false) => Err(format!(
            "this nomouse binary is not trusted. {requirement} Add the binary with + (a rebuilt \
             binary needs adding again), or run {APP} instead."
        )),
    }
}

/// The name of the doctor's line on the app's own Accessibility grant.
pub const APP_CHECK: &str = match Flavour::CURRENT {
    Flavour::Release => "Accessibility (nomouse.app)",
    Flavour::Dev => "Accessibility (nomouse Dev.app)",
};

/// What the running daemon told `nomouse doctor` about its own Accessibility grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonGrant {
    /// No daemon answered with its grant: none runs, or one from before nomouse reported it.
    Unknown,
    /// The daemon `pid` runs under `holder`'s grant, `trusted` or not.
    Reported {
        /// The daemon's process id.
        pid: u32,
        /// Whose grant it runs under.
        holder: GrantHolder,
        /// Whether macOS trusts it.
        trusted: bool,
    },
}

/// The doctor's line on the app's own Accessibility grant. A doctor started from a terminal
/// cannot check it itself (macOS answers for the terminal), so it takes the running daemon's
/// word: the daemon of the app checks its own grant.
///
/// # Errors
/// The problem and its fix when the app (or the daemon started outside it) is not trusted.
pub fn app_accessibility_check(daemon: &DaemonGrant) -> Result<String, String> {
    match daemon {
        DaemonGrant::Unknown => Ok(format!(
            "not checked: no {APP} daemon answers (the `daemon` line says why), and only {APP} \
             itself can check its grant. Start it and run the doctor again."
        )),
        DaemonGrant::Reported {
            pid,
            holder: GrantHolder::App,
            trusted,
        } => accessibility_check(*trusted, &GrantHolder::App)
            .map(|trusted| format!("{trusted} (its daemon, pid {pid}, checked)")),
        DaemonGrant::Reported {
            pid,
            holder,
            trusted,
        } => {
            let from = match holder {
                GrantHolder::Terminal(_) => "a terminal",
                GrantHolder::App | GrantHolder::Binary => "a nomouse binary outside the app",
            };
            let note = format!(
                "the running daemon (pid {pid}) was started from {from}, not as {APP}, so it \
                 runs under that program's grant"
            );
            if *trusted {
                Ok(format!(
                    "not checked: {note}, which is trusted. Quit it and start {APP} to check \
                     the app's own grant."
                ))
            } else {
                Err(format!(
                    "{note}, which is not trusted: it cannot read other apps. Quit it and start \
                     {APP}."
                ))
            }
        }
    }
}

fn terminal(name: Option<&str>) -> String {
    name.map_or_else(
        || "this terminal".to_owned(),
        |name| format!("this terminal ({name})"),
    )
}

/// The pid of the process holding secure event input, from `ioreg -l -w 0 -d 1` output
/// (`"kCGSSessionSecureInputPID"=<pid>` in `IOConsoleUsers`; absent while nobody holds it).
pub fn secure_input_pid(ioreg: &str) -> Option<i32> {
    let key = "\"kCGSSessionSecureInputPID\"=";
    let start = ioreg.find(key)? + key.len();
    let digits: String = ioreg[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok().filter(|pid| *pid > 0)
}

/// The doctor's secure input line; `holder` is the pid and app name holding it, when known.
///
/// # Errors
/// When secure input is on: who holds it and how to release it.
pub fn secure_input_check(
    on: bool,
    holder: Option<(i32, Option<String>)>,
) -> Result<String, String> {
    if !on {
        return Ok("off".to_owned());
    }
    let by = match holder {
        Some((pid, Some(name))) => format!(", held by {name} (pid {pid})"),
        Some((pid, None)) => format!(", held by pid {pid}"),
        None => String::new(),
    };
    Err(format!(
        "on{by}: while it is on, macOS sends no keys to nomouse, so its hotkeys do nothing. Leave \
         the password field, or turn off Secure Keyboard Entry in Terminal's or iTerm2's menu."
    ))
}

#[cfg(target_os = "macos")]
pub use live::{MacTrust, TrustProbe, grant_holder, own_grant};

#[cfg(target_os = "macos")]
mod live {
    use std::io::IsTerminal as _;
    use std::process::Command;

    use nomouse_core::setup::{Permission, Probe};
    use nomouse_sys_macos::{app, ax};

    use super::{ACCESSIBILITY_SETTINGS, GrantHolder, TrustSystem, ask_for_accessibility};

    /// Whose grant this process's trust checks measure. macOS attributes a process to the
    /// terminal app that started it, even with its output piped, so a terminal on stderr or a
    /// `TERM_PROGRAM` set by one means the terminal's grant.
    pub fn grant_holder() -> GrantHolder {
        let term_program = std::env::var("TERM_PROGRAM")
            .ok()
            .filter(|name| !name.is_empty());
        if term_program.is_some() || std::io::stderr().is_terminal() {
            GrantHolder::Terminal(term_program)
        } else if app::own_bundle_id().is_some() {
            GrantHolder::App
        } else {
            GrantHolder::Binary
        }
    }

    /// Whose Accessibility grant this process runs under, and whether macOS trusts it.
    pub fn own_grant() -> (GrantHolder, bool) {
        (grant_holder(), ax::is_process_trusted(false))
    }

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
            if grant_holder() == GrantHolder::App {
                app::own_bundle_id()
            } else {
                None
            }
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

    #[test]
    fn the_accessibility_line_names_whose_grant_it_checked() {
        let terminal = GrantHolder::Terminal(Some("Apple_Terminal".to_owned()));
        let untrusted = accessibility_check(false, &terminal).unwrap_err();
        assert!(untrusted.starts_with("this terminal (Apple_Terminal) is not trusted"));
        assert!(untrusted.contains(&format!("{APP} has its own grant")));
        let trusted = accessibility_check(true, &GrantHolder::Terminal(None)).unwrap();
        assert!(trusted.starts_with("this terminal is trusted"));
        assert_eq!(
            accessibility_check(true, &GrantHolder::App),
            Ok(format!("{APP} is trusted"))
        );
        assert_eq!(APP, format!("{}.app", Flavour::CURRENT.display_name()));
        let app = accessibility_check(false, &GrantHolder::App).unwrap_err();
        assert!(app.contains("older copy"));
        assert!(app.contains("Device Control and Data Access"));
        let binary = accessibility_check(false, &GrantHolder::Binary).unwrap_err();
        assert!(binary.starts_with("this nomouse binary is not trusted"));
    }

    #[test]
    fn the_app_line_takes_the_running_daemons_word_for_the_apps_own_grant() {
        let reported = |holder: GrantHolder, trusted: bool| DaemonGrant::Reported {
            pid: 4242,
            holder,
            trusted,
        };
        assert_eq!(
            app_accessibility_check(&reported(GrantHolder::App, true)),
            Ok(format!("{APP} is trusted (its daemon, pid 4242, checked)"))
        );
        let untrusted = app_accessibility_check(&reported(GrantHolder::App, false)).unwrap_err();
        assert!(
            untrusted.starts_with(&format!("{APP} is not trusted")),
            "{untrusted}"
        );
        let unknown = app_accessibility_check(&DaemonGrant::Unknown).unwrap();
        assert!(unknown.starts_with("not checked: no"), "{unknown}");
        let terminal = reported(GrantHolder::Terminal(Some("tmux".to_owned())), true);
        let from_terminal = app_accessibility_check(&terminal).unwrap();
        assert!(
            from_terminal.contains("started from a terminal"),
            "{from_terminal}"
        );
        let binary = app_accessibility_check(&reported(GrantHolder::Binary, false)).unwrap_err();
        assert!(binary.contains("is not trusted"), "{binary}");
        assert!(APP_CHECK.contains(APP));
        let terminal_line = accessibility_check(true, &GrantHolder::Terminal(None)).unwrap();
        assert!(terminal_line.contains(APP_CHECK), "{terminal_line}");
    }

    #[test]
    fn the_secure_input_holder_is_read_from_the_console_session() {
        let held = r#"| "IOConsoleUsers" = ({"kCGSSessionSecureInputPID"=4242,"kCGSSessionUserIDKey"=503})"#;
        assert_eq!(secure_input_pid(held), Some(4242));
        let free =
            r#"| "IOConsoleUsers" = ({"kCGSSessionOnConsoleKey"=Yes,"kCGSSessionUserIDKey"=503})"#;
        assert_eq!(secure_input_pid(free), None);
        assert_eq!(secure_input_pid(r#""kCGSSessionSecureInputPID"=0"#), None);
    }

    #[test]
    fn secure_input_on_names_its_holder_and_the_way_out() {
        assert_eq!(secure_input_check(false, None), Ok("off".to_owned()));
        let held = secure_input_check(true, Some((4242, Some("Terminal".to_owned())))).unwrap_err();
        assert!(held.starts_with("on, held by Terminal (pid 4242):"));
        assert!(held.contains("Secure Keyboard Entry"));
        let unknown = secure_input_check(true, None).unwrap_err();
        assert!(unknown.starts_with("on: "));
    }
}
