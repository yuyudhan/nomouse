//! The desktop's light or dark preference for `overlay.theme = "auto"`: the XDG desktop
//! portal's `org.freedesktop.appearance` `color-scheme` setting (1 prefers dark, 2 prefers
//! light, 0 states no preference), read over the session bus before each activation so the
//! labels follow a switch without a restart.

use std::time::Duration;

use async_io::block_on;
use nomouse_core::PlatformError;
use nomouse_core::config::{Appearance, OverlayConfig, Theme};
use zbus::zvariant::Value;

use crate::x11::render::Style;

/// How long the portal may take to answer before the labels use the light colours.
const PORTAL_TIMEOUT: Duration = Duration::from_millis(250);

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// The appearance a `color-scheme` value asks for.
pub(crate) const fn appearance_of(scheme: u32) -> Appearance {
    match scheme {
        1 => Appearance::Dark,
        _ => Appearance::Light,
    }
}

/// The `color-scheme` number in a portal reply: `ReadOne` returns it in one variant, the older
/// `Read` in a variant inside a variant.
fn scheme(value: &Value<'_>) -> Option<u32> {
    match value {
        Value::U32(scheme) => Some(*scheme),
        Value::Value(inner) => scheme(inner),
        _ => None,
    }
}

/// Reads the portal setting over a session-bus connection opened on first use.
#[derive(Default)]
pub(crate) struct Portal {
    conn: Option<zbus::Connection>,
}

impl Portal {
    async fn call(&mut self, method: &str) -> zbus::Result<u32> {
        let conn = if let Some(conn) = &self.conn {
            conn.clone()
        } else {
            let conn = zbus::connection::Builder::session()?
                .method_timeout(PORTAL_TIMEOUT)
                .build()
                .await?;
            self.conn = Some(conn.clone());
            conn
        };
        let reply = conn
            .call_method(
                Some(PORTAL),
                PORTAL_PATH,
                Some(SETTINGS),
                method,
                &(NAMESPACE, KEY),
            )
            .await?;
        let body = reply.body();
        let value: Value<'_> = body.deserialize()?;
        scheme(&value).ok_or_else(|| {
            zbus::Error::Failure(format!("{NAMESPACE} {KEY} is not a number: {value}"))
        })
    }

    /// The system appearance; light when the portal is missing or does not answer.
    pub(crate) fn read(&mut self) -> Appearance {
        let result = block_on(async {
            match self.call("ReadOne").await {
                Ok(scheme) => Ok(scheme),
                // Portals before version 2 only have `Read`.
                Err(_) => self.call("Read").await,
            }
        });
        match result {
            Ok(scheme) => appearance_of(scheme),
            Err(error) => {
                tracing::debug!("appearance portal: {error}; using the light theme");
                Appearance::Light
            }
        }
    }
}

/// The overlay style, re-resolved before each activation when the theme follows the system.
pub(crate) struct Themed {
    overlay: OverlayConfig,
    portal: Portal,
    appearance: Appearance,
}

impl Themed {
    /// The style for `overlay` now, and what keeps it current.
    ///
    /// # Errors
    /// [`PlatformError::Backend`] for an invalid colour.
    pub(crate) fn new(overlay: &OverlayConfig) -> Result<(Self, Style), PlatformError> {
        let mut portal = Portal::default();
        let appearance = if overlay.theme == Theme::Auto {
            portal.read()
        } else {
            Appearance::Light
        };
        let style = Style::from_config(overlay, appearance)?;
        let themed = Self {
            overlay: overlay.clone(),
            portal,
            appearance,
        };
        Ok((themed, style))
    }

    /// The new style when the theme is `auto` and the system appearance changed since the last
    /// look.
    pub(crate) fn changed(&mut self) -> Option<Style> {
        if self.overlay.theme != Theme::Auto {
            return None;
        }
        let appearance = self.portal.read();
        if appearance == self.appearance {
            return None;
        }
        tracing::debug!("system appearance is now {appearance:?}");
        self.appearance = appearance;
        Style::from_config(&self.overlay, appearance).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_scheme_values() {
        assert_eq!(appearance_of(0), Appearance::Light);
        assert_eq!(appearance_of(1), Appearance::Dark);
        assert_eq!(appearance_of(2), Appearance::Light);
        assert_eq!(appearance_of(7), Appearance::Light);
    }

    #[test]
    fn replies_from_read_one_and_the_older_read() {
        assert_eq!(scheme(&Value::U32(1)), Some(1));
        let nested = Value::Value(Box::new(Value::Value(Box::new(Value::U32(2)))));
        assert_eq!(scheme(&nested), Some(2));
        assert_eq!(scheme(&Value::Str("dark".into())), None);
    }
}
