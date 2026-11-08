//! Which display server the session runs.

use std::fmt;

use nomouse_core::PlatformError;

/// The display server of the current session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayServer {
    /// An X11 session on this display (`$DISPLAY`).
    X11(String),
    /// A Wayland session on this socket (`$WAYLAND_DISPLAY`).
    Wayland(String),
    /// Neither variable is set (a console or an SSH session).
    None,
}

impl DisplayServer {
    /// Detects the session from the environment: Wayland when `WAYLAND_DISPLAY` is set or
    /// `XDG_SESSION_TYPE` is `wayland` (Xwayland's `DISPLAY` does not make it X11), else X11
    /// when `DISPLAY` is set.
    pub fn detect(env: impl Fn(&str) -> Option<String>) -> Self {
        let get = |name: &str| env(name).filter(|value| !value.is_empty());
        if let Some(socket) = get("WAYLAND_DISPLAY") {
            return Self::Wayland(socket);
        }
        if get("XDG_SESSION_TYPE").is_some_and(|kind| kind.eq_ignore_ascii_case("wayland")) {
            return Self::Wayland("wayland-0".to_owned());
        }
        get("DISPLAY").map_or(Self::None, Self::X11)
    }

    /// Detects the session from this process's environment.
    pub fn current() -> Self {
        Self::detect(|name| std::env::var(name).ok())
    }

    /// `Ok(display)` for X11, otherwise the typed error the backend reports.
    ///
    /// # Errors
    /// [`PlatformError::Unsupported`] for Wayland (until milestone M4) and for no session.
    pub fn require_x11(&self) -> Result<&str, PlatformError> {
        match self {
            Self::X11(display) => Ok(display),
            Self::Wayland(socket) => Err(PlatformError::Unsupported(format!(
                "Wayland session ({socket}): Wayland support is not implemented yet (roadmap \
                 milestone M4); log in to an X11 session to use nomouse"
            ))),
            Self::None => Err(PlatformError::Unsupported(
                "no graphical session: neither DISPLAY nor WAYLAND_DISPLAY is set".to_owned(),
            )),
        }
    }
}

impl fmt::Display for DisplayServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::X11(display) => write!(f, "X11 ({display})"),
            Self::Wayland(socket) => write!(f, "Wayland ({socket})"),
            Self::None => f.write_str("none"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| {
            owned
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn detection() {
        assert_eq!(
            DisplayServer::detect(env(&[("DISPLAY", ":0")])),
            DisplayServer::X11(":0".into())
        );
        assert_eq!(
            DisplayServer::detect(env(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-1")])),
            DisplayServer::Wayland("wayland-1".into())
        );
        assert!(matches!(
            DisplayServer::detect(env(&[("DISPLAY", ":1"), ("XDG_SESSION_TYPE", "wayland")])),
            DisplayServer::Wayland(_)
        ));
        assert_eq!(
            DisplayServer::detect(env(&[("DISPLAY", "")])),
            DisplayServer::None
        );
    }

    #[test]
    fn wayland_is_a_typed_unsupported_error() {
        let error = DisplayServer::Wayland("wayland-0".into())
            .require_x11()
            .unwrap_err();
        assert!(matches!(&error, PlatformError::Unsupported(m) if m.contains("M4")));
        assert_eq!(DisplayServer::X11(":5".into()).require_x11(), Ok(":5"));
        assert!(DisplayServer::None.require_x11().is_err());
    }
}
