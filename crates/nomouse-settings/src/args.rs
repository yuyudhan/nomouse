//! The window's pages and the command line (`--page`, `--about`, `--no-daemon`).

use nomouse_core::settings::Section;

/// One page of the settings window: General, then the shared settings sections in the order
/// the macOS window shows them, then Doctor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    /// On/off, daemon status, start at login, updates (Linux only).
    General,
    /// A section of [`nomouse_core::settings::FIELDS`].
    Settings(Section),
    /// `nomouse doctor` (Linux only).
    Doctor,
}

impl Page {
    /// Every page, in sidebar order.
    pub const ALL: [Self; 9] = [
        Self::General,
        Self::Settings(Section::ALL[0]),
        Self::Settings(Section::ALL[1]),
        Self::Settings(Section::ALL[2]),
        Self::Settings(Section::ALL[3]),
        Self::Settings(Section::ALL[4]),
        Self::Settings(Section::ALL[5]),
        Self::Settings(Section::ALL[6]),
        Self::Doctor,
    ];

    /// The `--page` name, also the stack child name.
    pub const fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Settings(Section::Hotkeys) => "hotkeys",
            Self::Settings(Section::Labels) => "labels",
            Self::Settings(Section::Appearance) => "appearance",
            Self::Settings(Section::Scroll) => "scroll",
            Self::Settings(Section::Search) => "search",
            Self::Settings(Section::IgnoredApps) => "ignored",
            Self::Settings(Section::Advanced) => "advanced",
            Self::Doctor => "doctor",
        }
    }

    /// The sidebar title.
    pub const fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Settings(section) => section.title(),
            Self::Doctor => "Doctor",
        }
    }

    /// One line saying what the page is for.
    pub const fn summary(self) -> &'static str {
        match self {
            Self::General => "Whether nomouse runs, and when it starts.",
            Self::Settings(section) => section.summary(),
            Self::Doctor => "What works in this session, and what to fix.",
        }
    }

    /// The symbolic icon shown next to the title.
    pub const fn icon(self) -> &'static str {
        match self {
            Self::General => "emblem-system-symbolic",
            Self::Settings(Section::Hotkeys) => "input-keyboard-symbolic",
            Self::Settings(Section::Labels) => "font-x-generic-symbolic",
            Self::Settings(Section::Appearance) => "applications-graphics-symbolic",
            Self::Settings(Section::Scroll) => "view-continuous-symbolic",
            Self::Settings(Section::Search) => "system-search-symbolic",
            Self::Settings(Section::IgnoredApps) => "action-unavailable-symbolic",
            Self::Settings(Section::Advanced) => "preferences-other-symbolic",
            Self::Doctor => "emblem-ok-symbolic",
        }
    }

    /// The page named `id` (see [`Page::id`]).
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|page| page.id() == id)
    }
}

/// The command line usage.
pub const USAGE: &str = "usage: nomouse-settings [--page <general|hotkeys|labels|appearance|scroll|search|ignored|advanced|doctor>] [--about] [--no-daemon] | --version | --help";

/// What the command line asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    /// The page to show.
    pub page: Option<Page>,
    /// Open the About dialog.
    pub about: bool,
    /// Do not start the daemon (tests, screenshots).
    pub no_daemon: bool,
    /// Print the version and exit, without a display.
    pub version: bool,
    /// Print the usage and exit, without a display.
    pub help: bool,
}

/// What `--version` prints.
pub fn version_line() -> String {
    format!("nomouse-settings {}", env!("CARGO_PKG_VERSION"))
}

/// Parses the arguments after the program name.
///
/// # Errors
/// An unknown option, a missing or unknown page name, with [`USAGE`].
pub fn parse<I, S>(args: I) -> Result<Args, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut parsed = Args::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        let page = match arg {
            "--about" => {
                parsed.about = true;
                continue;
            }
            "--no-daemon" => {
                parsed.no_daemon = true;
                continue;
            }
            "--version" | "-V" => {
                parsed.version = true;
                continue;
            }
            "--help" | "-h" => {
                parsed.help = true;
                continue;
            }
            "--page" => args
                .next()
                .map(|value| value.as_ref().to_owned())
                .ok_or_else(|| format!("--page needs a page name\n{USAGE}"))?,
            other => match other.strip_prefix("--page=") {
                Some(value) => value.to_owned(),
                None => return Err(format!("unknown argument {other:?}\n{USAGE}")),
            },
        };
        parsed.page =
            Some(Page::from_id(&page).ok_or_else(|| format!("unknown page {page:?}\n{USAGE}"))?);
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_option() {
        assert_eq!(parse::<_, &str>([]).unwrap(), Args::default());
        assert_eq!(
            parse(["--page", "scroll", "--about", "--no-daemon"]).unwrap(),
            Args {
                page: Some(Page::Settings(Section::Scroll)),
                about: true,
                no_daemon: true,
                ..Args::default()
            }
        );
        assert!(parse(["--version"]).unwrap().version);
        assert!(parse(["-h"]).unwrap().help);
        assert_eq!(
            version_line(),
            format!("nomouse-settings {}", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(parse(["--page=doctor"]).unwrap().page, Some(Page::Doctor));
        assert_eq!(
            parse(["--page", "ignored"]).unwrap().page,
            Some(Page::Settings(Section::IgnoredApps))
        );
    }

    #[test]
    fn refuses_unknown_input() {
        assert!(parse(["--page"]).unwrap_err().contains("needs a page"));
        assert!(
            parse(["--page", "shortcuts"])
                .unwrap_err()
                .contains("unknown page")
        );
        assert!(
            parse(["--verbose"])
                .unwrap_err()
                .contains("unknown argument")
        );
    }

    #[test]
    fn pages_cover_every_section_once_with_unique_ids() {
        let ids: Vec<&str> = Page::ALL.iter().map(|page| page.id()).collect();
        assert_eq!(
            ids,
            [
                "general",
                "hotkeys",
                "labels",
                "appearance",
                "scroll",
                "search",
                "ignored",
                "advanced",
                "doctor"
            ]
        );
        for section in Section::ALL {
            assert!(Page::ALL.contains(&Page::Settings(section)));
        }
        for page in Page::ALL {
            assert_eq!(Page::from_id(page.id()), Some(page));
            assert_ne!(page.summary(), "");
        }
        assert_eq!(Page::Settings(Section::IgnoredApps).title(), "Ignored apps");
    }
}
