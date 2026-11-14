//! `AXEnhancedUserInterface`: when nomouse switches it on, and what to say when a Chromium or
//! Electron app still shows no web content.
//!
//! Chromium builds the accessibility tree of web pages only while the attribute is on, and a
//! relaunched browser starts with it off, so it is checked before every scan rather than once
//! per process.

use std::time::Duration;

use nomouse_core::config::CHROMIUM_BROWSERS;

/// How long Chromium may take to build a page's tree after `AXEnhancedUserInterface` is
/// switched on (a relaunched Chrome 157 showed its page after ~2.8 s). A scan never waits for
/// it: the hint session labels what is there at once and adds the page at its 1 s and 3 s
/// re-scans. Until this much time has passed a missing page is not reported.
pub const PAGE_BUILD: Duration = Duration::from_secs(4);

/// Whether a scan that found `web_areas` web areas in a window (`has_window`) reports the
/// page as missing. `since_switch` is how long ago this process switched the attribute on
/// (`None` when it did not): a page still being built is not missing.
pub fn reports_missing_web(
    since_switch: Option<Duration>,
    has_window: bool,
    web_areas: usize,
) -> bool {
    has_window && web_areas == 0 && since_switch.is_none_or(|since| since >= PAGE_BUILD)
}

/// Whether the attribute must be set before a scan: the app is listed in
/// `macos.enhanced_ui_apps` and does not report it on (`current` is `None` when unreadable).
pub fn needs_enhancing(listed: bool, current: Option<bool>) -> bool {
    listed && current != Some(true)
}

/// Whether the app `bundle_id` draws its content in web areas: a known Chromium browser, or
/// any app the user listed (Electron apps).
pub fn is_web_app(bundle_id: &str, listed: &[String]) -> bool {
    CHROMIUM_BROWSERS.contains(&bundle_id) || listed.iter().any(|id| id == bundle_id)
}

/// Why the web app `bundle_id` shows no web area, and what fixes it.
pub fn missing_web_area(bundle_id: &str, listed: bool, enhanced: Option<bool>) -> String {
    if !listed {
        format!(
            "{bundle_id} shows no web content: add \"{bundle_id}\" to macos.enhanced_ui_apps in \
             ~/.config/nomouse/config.toml"
        )
    } else if enhanced == Some(true) {
        format!(
            "{bundle_id} shows no web content although AXEnhancedUserInterface is on: open a \
             web page in it and activate again, or quit and reopen it"
        )
    } else {
        format!(
            "{bundle_id} shows no web content: AXEnhancedUserInterface is off (Chromium turns it \
             off when it relaunches); activate nomouse in it once to switch it back on"
        )
    }
}

/// How often `nomouse doctor` walks a web app's windows again while none shows a page.
pub const DOCTOR_POLL: Duration = Duration::from_millis(250);

/// What one walk of one running instance of a web app found, for `nomouse doctor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstanceScan {
    /// Its `AXEnhancedUserInterface` (`None` when unreadable).
    pub enhanced: Option<bool>,
    /// Open (not minimized) windows.
    pub windows: usize,
    /// Web areas with content ([`crate::walk::Walk::web_areas`]).
    pub web_areas: usize,
    /// Empty web areas ([`crate::walk::Walk::blank_web_areas`]): blank tabs.
    pub blank_web_areas: usize,
}

impl InstanceScan {
    /// Windows open, none showing a page and none a blank tab: the page tree is missing.
    const fn lacks_page_tree(&self) -> bool {
        self.windows > 0 && self.web_areas == 0 && self.blank_web_areas == 0
    }
}

/// The `nomouse doctor` line for the web app `bundle_id` from one walk of each of its running
/// instances (an automation browser runs next to the user's own). Passes when any instance
/// shows web content, when none has a window, or when the only pages are blank tabs.
///
/// # Errors
/// The problem and its fix when an instance has windows but no page tree at all.
pub fn web_check(bundle_id: &str, listed: bool, scans: &[InstanceScan]) -> Result<String, String> {
    if scans.iter().any(|scan| scan.web_areas > 0) {
        Ok(format!("{bundle_id}: web content readable"))
    } else if let Some(failing) = scans.iter().find(|scan| scan.lacks_page_tree()) {
        Err(missing_web_area(bundle_id, listed, failing.enhanced))
    } else if scans.iter().any(|scan| scan.blank_web_areas > 0) {
        Ok(format!(
            "{bundle_id}: no page open (only blank tabs); open a page to check it"
        ))
    } else {
        Ok(format!("{bundle_id}: no window open to check"))
    }
}

/// Whether `nomouse doctor` walks the instances again `elapsed` after its first walk: no
/// instance shows a page yet, one with the attribute on has windows without a page tree
/// (Chromium builds it a moment after being asked, and drops it again when idle), and another
/// [`DOCTOR_POLL`] still fits in [`PAGE_BUILD`].
pub fn rechecks(scans: &[InstanceScan], elapsed: Duration) -> bool {
    scans.iter().all(|scan| scan.web_areas == 0)
        && scans
            .iter()
            .any(|scan| scan.lacks_page_tree() && scan.enhanced == Some(true))
        && elapsed + DOCTOR_POLL < PAGE_BUILD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_apps_are_enhanced_until_they_report_it_on() {
        assert!(needs_enhancing(true, None));
        assert!(
            needs_enhancing(true, Some(false)),
            "a relaunched browser turned it off: set it again"
        );
        assert!(!needs_enhancing(true, Some(true)));
        assert!(!needs_enhancing(false, None));
        assert!(!needs_enhancing(false, Some(false)));
    }

    #[test]
    fn web_apps_are_chromium_browsers_or_listed() {
        assert!(is_web_app("com.google.Chrome", &[]));
        assert!(is_web_app("company.thebrowser.Browser", &[]));
        assert!(!is_web_app("com.microsoft.VSCode", &[]));
        assert!(is_web_app(
            "com.microsoft.VSCode",
            &["com.microsoft.VSCode".to_owned()]
        ));
        assert!(!is_web_app("com.apple.Safari", &[]));
    }

    #[test]
    fn a_missing_page_is_reported_only_after_the_page_had_time_to_build() {
        let since = |ms| Some(Duration::from_millis(ms));
        assert!(
            !reports_missing_web(since(0), true, 0),
            "just switched on: the page is still being built"
        );
        assert!(
            !reports_missing_web(since(2800), true, 0),
            "a relaunched Chrome 157 showed its page 2.8 s after the switch"
        );
        assert!(reports_missing_web(since(4000), true, 0));
        assert_eq!(PAGE_BUILD, Duration::from_secs(4));
        assert!(
            reports_missing_web(None, true, 0),
            "not switched on by this process: nothing to wait for"
        );
        assert!(!reports_missing_web(None, true, 1), "the page is there");
        assert!(!reports_missing_web(None, false, 0), "no window");
    }

    fn scan(windows: usize, web_areas: usize, blank_web_areas: usize) -> InstanceScan {
        InstanceScan {
            enhanced: Some(true),
            windows,
            web_areas,
            blank_web_areas,
        }
    }

    #[test]
    fn doctor_says_what_fixes_a_missing_web_area() {
        let id = "com.google.Chrome";
        let off = InstanceScan {
            enhanced: Some(false),
            ..scan(1, 0, 0)
        };
        assert!(web_check(id, true, &[]).is_ok());
        assert!(web_check(id, true, &[scan(0, 0, 0)]).is_ok());
        assert!(web_check(id, true, &[scan(2, 1, 0)]).is_ok());
        let unlisted = web_check(id, false, &[scan(1, 0, 0)]).unwrap_err();
        assert!(unlisted.contains("add \"com.google.Chrome\" to macos.enhanced_ui_apps"));
        let is_off = web_check(id, true, &[off]).unwrap_err();
        assert!(is_off.contains("AXEnhancedUserInterface is off"));
        let on = web_check(id, true, &[scan(1, 0, 0)]).unwrap_err();
        assert!(on.contains("although AXEnhancedUserInterface is on"));
    }

    #[test]
    fn doctor_passes_when_any_instance_shows_a_page() {
        let id = "com.google.Chrome";
        // An automation Chrome with a bare window next to the user's Chrome with a page.
        let readable = web_check(id, true, &[scan(1, 0, 0), scan(3, 2, 0)]).unwrap();
        assert!(readable.contains("web content readable"));
        // Only blank tabs (an empty web area): nothing to check, not a failure.
        let blank = web_check(id, true, &[scan(1, 0, 1)]).unwrap();
        assert!(blank.contains("no page open"), "{blank}");
        // One instance with only a blank tab, another whose page shows nothing: the failure.
        assert!(web_check(id, true, &[scan(1, 0, 1), scan(1, 0, 0)]).is_err());
    }

    #[test]
    fn doctor_looks_again_while_chromium_may_still_build_the_page() {
        let failing = [scan(1, 0, 0)];
        assert!(rechecks(&failing, Duration::ZERO));
        assert!(rechecks(&failing, Duration::from_millis(3700)));
        assert!(!rechecks(&failing, Duration::from_millis(3800)), "bounded");
        assert!(!rechecks(&[scan(1, 1, 0)], Duration::ZERO), "readable");
        assert!(!rechecks(&[scan(0, 0, 0)], Duration::ZERO), "no window");
        let off = InstanceScan {
            enhanced: Some(false),
            ..scan(1, 0, 0)
        };
        assert!(
            !rechecks(&[off], Duration::ZERO),
            "off: nothing will build it"
        );
    }
}
