//! The application owning the focused window, and `ignore.apps`: apps where the hotkeys show
//! no labels (terminals and editors with their own keyboard navigation, games).

/// Who owns the focused window, as far as the platform can tell. Every field may be missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppIdentity {
    /// The macOS bundle id (`com.apple.Terminal`) or the Wayland app id (`org.gnome.Terminal`,
    /// `firefox`).
    pub app_id: Option<String>,
    /// The X11 `WM_CLASS` names: instance then class (`gnome-terminal-server`,
    /// `Gnome-terminal`).
    pub wm_class: Vec<String>,
    /// The process name (`/proc/<pid>/comm` on Linux: the executable name, at most 15 bytes).
    pub process: Option<String>,
}

impl AppIdentity {
    /// Every name the app goes by, app id first.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.app_id
            .iter()
            .chain(&self.wm_class)
            .chain(&self.process)
            .map(String::as_str)
            .filter(|name| !name.is_empty())
    }

    /// Whether `pattern` names this app: equal to one of its [`names`](Self::names), ignoring
    /// ASCII case. A process name may be a truncated `comm` (15 bytes), so a longer pattern
    /// matches when the process name is its first 15 bytes.
    pub fn matches(&self, pattern: &str) -> bool {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return false;
        }
        let ids = self.app_id.iter().chain(&self.wm_class);
        if ids.into_iter().any(|id| id.eq_ignore_ascii_case(pattern)) {
            return true;
        }
        self.process.as_deref().is_some_and(|process| {
            process.eq_ignore_ascii_case(pattern)
                || (process.len() == COMM_LEN
                    && pattern
                        .get(..COMM_LEN)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(process)))
        })
    }

    /// The first of `patterns` naming this app.
    pub fn ignored_by<'a>(&self, patterns: &'a [String]) -> Option<&'a str> {
        patterns
            .iter()
            .map(String::as_str)
            .find(|pattern| self.matches(pattern))
    }
}

/// The longest process name Linux keeps in `/proc/<pid>/comm`.
const COMM_LEN: usize = 15;

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal() -> AppIdentity {
        AppIdentity {
            app_id: Some("org.gnome.Terminal".to_owned()),
            wm_class: vec![
                "gnome-terminal-server".to_owned(),
                "Gnome-terminal".to_owned(),
            ],
            process: Some("gnome-terminal-".to_owned()),
        }
    }

    #[test]
    fn app_id_and_bundle_id_match_ignoring_case() {
        let app = terminal();
        assert!(app.matches("org.gnome.Terminal"));
        assert!(app.matches("ORG.GNOME.TERMINAL"));
        assert!(!app.matches("org.gnome"));
        let mac = AppIdentity {
            app_id: Some("com.apple.Terminal".to_owned()),
            ..AppIdentity::default()
        };
        assert!(mac.matches(" com.apple.Terminal "));
        assert!(!mac.matches("Terminal"));
    }

    #[test]
    fn wm_class_instance_or_class_matches() {
        let app = terminal();
        assert!(app.matches("gnome-terminal-server"));
        assert!(app.matches("gnome-terminal"));
        assert!(!app.matches("terminal"));
    }

    #[test]
    fn process_name_matches_whole_or_as_a_truncated_comm() {
        let app = terminal();
        // `comm` holds the first 15 bytes of "gnome-terminal-server".
        assert!(app.matches("gnome-terminal-server"));
        let short = AppIdentity {
            process: Some("kitty".to_owned()),
            ..AppIdentity::default()
        };
        assert!(short.matches("kitty"));
        assert!(!short.matches("kitty-extra"));
        assert!(!short.matches("kit"));
        let truncated = AppIdentity {
            process: Some("gnome-terminal-".to_owned()),
            ..AppIdentity::default()
        };
        assert!(truncated.matches("gnome-terminal-server"));
        assert!(!truncated.matches("gnome-terminal"));
    }

    #[test]
    fn empty_patterns_and_unknown_apps_match_nothing() {
        assert!(!terminal().matches(""));
        assert!(!terminal().matches("  "));
        assert!(!AppIdentity::default().matches("firefox"));
        assert_eq!(AppIdentity::default().names().count(), 0);
    }

    #[test]
    fn ignored_by_names_the_first_matching_pattern() {
        let patterns = vec![
            "firefox".to_owned(),
            "Gnome-terminal".to_owned(),
            "org.gnome.Terminal".to_owned(),
        ];
        assert_eq!(terminal().ignored_by(&patterns), Some("Gnome-terminal"));
        assert_eq!(terminal().ignored_by(&patterns[..1]), None);
        let app = terminal();
        let names: Vec<&str> = app.names().collect();
        assert_eq!(names[0], "org.gnome.Terminal");
        assert_eq!(names.len(), 4);
    }
}
