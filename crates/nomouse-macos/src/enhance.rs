//! `AXEnhancedUserInterface`: when nomouse switches it on, and what to say when a Chromium or
//! Electron app still shows no web content.
//!
//! Chromium builds the accessibility tree of web pages only while the attribute is on, and a
//! relaunched browser starts with it off, so it is checked before every scan rather than once
//! per process.

use nomouse_core::config::CHROMIUM_BROWSERS;

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

/// The `nomouse doctor` line for the running web app `bundle_id`: `windows` open windows
/// holding `web_areas` web areas between them.
///
/// # Errors
/// The problem and its fix when it has windows but none shows web content.
pub fn web_check(
    bundle_id: &str,
    listed: bool,
    enhanced: Option<bool>,
    windows: usize,
    web_areas: usize,
) -> Result<String, String> {
    if windows == 0 {
        Ok(format!("{bundle_id}: no window open to check"))
    } else if web_areas > 0 {
        Ok(format!("{bundle_id}: web content readable"))
    } else {
        Err(missing_web_area(bundle_id, listed, enhanced))
    }
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
    fn doctor_says_what_fixes_a_missing_web_area() {
        let id = "com.google.Chrome";
        assert!(web_check(id, true, Some(false), 0, 0).is_ok());
        assert!(web_check(id, true, Some(true), 2, 1).is_ok());
        let unlisted = web_check(id, false, None, 1, 0).unwrap_err();
        assert!(unlisted.contains("add \"com.google.Chrome\" to macos.enhanced_ui_apps"));
        let off = web_check(id, true, Some(false), 1, 0).unwrap_err();
        assert!(off.contains("AXEnhancedUserInterface is off"));
        let on = web_check(id, true, Some(true), 1, 0).unwrap_err();
        assert!(on.contains("although AXEnhancedUserInterface is on"));
    }
}
