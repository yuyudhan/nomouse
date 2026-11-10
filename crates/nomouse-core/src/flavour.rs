//! Which build this is: the released nomouse or a development build. The two run side by side
//! on one machine without touching each other: every name a running nomouse claims (its
//! config directory, lock, command socket, logs, D-Bus names, autostart entry, app bundle) comes
//! from [`Flavour::CURRENT`], and a development build's names all carry `-dev` (or `.dev`, or
//! ` Dev` for the macOS app's name).
//!
//! The flavour is fixed when nomouse is compiled, never guessed at run time: `NOMOUSE_FLAVOUR`
//! (`dev` or `release`) in the build's environment chooses it; without it a debug build
//! (`cargo run`, `cargo test`) is `dev` and a release-profile build (`cargo build --release`,
//! `cargo install`, the release workflow) is `release`. The release workflow sets
//! `NOMOUSE_FLAVOUR=release` explicitly; `just app` sets `dev` for a local app bundle.

/// Which build of nomouse this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    /// The released nomouse: `~/.config/nomouse/`, `nomouse.sock`, `com.yuyudhan.nomouse`.
    Release,
    /// A development build: `~/.config/nomouse-dev/`, `nomouse-dev.sock`,
    /// `com.yuyudhan.nomouse.dev`.
    Dev,
}

/// Whether two byte strings are equal, usable in a constant.
const fn bytes_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

/// The flavour a `NOMOUSE_FLAVOUR` value names: `dev` or `release`; anything else is `None`.
const fn parse(value: &str) -> Option<Flavour> {
    if bytes_equal(value.as_bytes(), b"dev") {
        Some(Flavour::Dev)
    } else if bytes_equal(value.as_bytes(), b"release") {
        Some(Flavour::Release)
    } else {
        None
    }
}

/// The flavour from the build's environment: `NOMOUSE_FLAVOUR` when set and not empty, else
/// the build profile (debug builds are `dev`). `None` for a value that names neither flavour.
const CHOSEN: Option<Flavour> = match option_env!("NOMOUSE_FLAVOUR") {
    Some(value) if !value.is_empty() => parse(value),
    _ if cfg!(debug_assertions) => Some(Flavour::Dev),
    _ => Some(Flavour::Release),
};

const _: () = assert!(
    CHOSEN.is_some(),
    "NOMOUSE_FLAVOUR must be `dev` or `release` (or unset: debug builds are dev)"
);

impl Flavour {
    /// This build's flavour, chosen when it was compiled.
    pub const CURRENT: Self = match CHOSEN {
        Some(flavour) => flavour,
        // Unreachable: the assertion above stops the build first.
        None => Self::Release,
    };

    /// `release` or `dev`, as `NOMOUSE_FLAVOUR` spells it and `nomouse doctor` reports it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Dev => "dev",
        }
    }

    /// The stem of every file and directory name the build claims: `nomouse` or `nomouse-dev`
    /// (`~/.config/<stem>/`, `<stem>.sock`, `<stem>.lock`, `<stem>.log`).
    #[must_use]
    pub const fn stem(self) -> &'static str {
        match self {
            Self::Release => "nomouse",
            Self::Dev => "nomouse-dev",
        }
    }

    /// The name people see for the app: `nomouse` or `nomouse Dev` (the macOS bundle's
    /// `CFBundleName`, so `nomouse Dev.app` and its own Accessibility and Login Items entries).
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Release => "nomouse",
            Self::Dev => "nomouse Dev",
        }
    }

    /// The macOS bundle identifier: `com.yuyudhan.nomouse` or `com.yuyudhan.nomouse.dev`.
    /// macOS keys the Accessibility grant and the login item to it.
    #[must_use]
    pub const fn macos_bundle_id(self) -> &'static str {
        match self {
            Self::Release => "com.yuyudhan.nomouse",
            Self::Dev => "com.yuyudhan.nomouse.dev",
        }
    }

    /// The Linux application id: `io.github.yuyudhan.nomouse` or
    /// `io.github.yuyudhan.nomouse.dev` (the settings window's single-instance D-Bus name).
    #[must_use]
    pub const fn linux_app_id(self) -> &'static str {
        match self {
            Self::Release => "io.github.yuyudhan.nomouse",
            Self::Dev => "io.github.yuyudhan.nomouse.dev",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_environment_names_a_flavour() {
        assert_eq!(parse("dev"), Some(Flavour::Dev));
        assert_eq!(parse("release"), Some(Flavour::Release));
        for other in ["", "Dev", "RELEASE", "debug", "dev ", "releases"] {
            assert_eq!(parse(other), None, "{other:?}");
        }
    }

    #[test]
    fn without_nomouse_flavour_the_profile_decides() {
        if option_env!("NOMOUSE_FLAVOUR").is_none_or(str::is_empty) {
            assert_eq!(Flavour::CURRENT == Flavour::Dev, cfg!(debug_assertions));
        } else {
            assert_eq!(
                Some(Flavour::CURRENT),
                option_env!("NOMOUSE_FLAVOUR").and_then(parse)
            );
        }
    }

    #[test]
    fn release_names_are_unchanged_and_dev_names_are_apart() {
        let release = Flavour::Release;
        assert_eq!(release.stem(), "nomouse");
        assert_eq!(release.display_name(), "nomouse");
        assert_eq!(release.macos_bundle_id(), "com.yuyudhan.nomouse");
        assert_eq!(release.linux_app_id(), "io.github.yuyudhan.nomouse");
        let dev = Flavour::Dev;
        assert_eq!(dev.stem(), "nomouse-dev");
        assert_eq!(dev.display_name(), "nomouse Dev");
        assert_eq!(dev.macos_bundle_id(), "com.yuyudhan.nomouse.dev");
        assert_eq!(dev.linux_app_id(), "io.github.yuyudhan.nomouse.dev");
        assert_eq!((release.name(), dev.name()), ("release", "dev"));
    }
}
