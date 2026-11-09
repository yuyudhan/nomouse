//! Start at login on Linux: an XDG autostart entry (`~/.config/autostart/nomouse.desktop`)
//! that runs `nomouse daemon` when the desktop session starts. GNOME, KDE, Xfce, Cinnamon and
//! MATE read it; tiling compositors (Sway, Hyprland, i3) start the daemon from their own config
//! instead.

use std::io;
use std::path::{Path, PathBuf};

/// The entry's file name in the autostart directory.
pub const FILE_NAME: &str = "nomouse.desktop";

/// `$XDG_CONFIG_HOME/autostart/nomouse.desktop`, else `~/.config/autostart/nomouse.desktop`.
pub fn path_from(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let get = |name: &str| env(name).filter(|value| !value.is_empty());
    let config = get("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| get("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("autostart").join(FILE_NAME))
}

/// The entry path for this process's environment.
pub fn path() -> Option<PathBuf> {
    path_from(|name| std::env::var(name).ok())
}

/// Whether the entry at `path` starts nomouse: it exists and is neither `Hidden` nor
/// switched off with `X-GNOME-Autostart-enabled=false`.
pub fn is_enabled(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    !text.lines().map(str::trim).any(|line| {
        line.eq_ignore_ascii_case("Hidden=true")
            || line.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
    })
}

/// Quotes `program` for a desktop entry's `Exec` key when it needs it.
fn exec_word(program: &Path) -> String {
    let text = program.to_string_lossy();
    if text
        .chars()
        .any(|c| c.is_whitespace() || "\"'\\><~|&;$*?#()`".contains(c))
    {
        let escaped: String = text
            .chars()
            .flat_map(|c| {
                let escape = matches!(c, '"' | '`' | '$' | '\\');
                escape.then_some('\\').into_iter().chain(std::iter::once(c))
            })
            .collect();
        format!("\"{escaped}\"")
    } else {
        text.into_owned()
    }
}

/// The autostart entry that runs `program daemon`.
pub fn entry(program: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=nomouse\n\
         Comment=Click anything with the keyboard\n\
         Exec={} daemon\n\
         Icon=io.github.yuyudhan.nomouse\n\
         Terminal=false\n\
         NoDisplay=true\n\
         X-GNOME-Autostart-enabled=true\n",
        exec_word(program)
    )
}

/// Writes the entry for `program` at `path` (creating its directory) or removes it.
///
/// # Errors
/// The I/O error writing or removing the file (a missing file is not an error when removing).
pub fn set(path: &Path, enabled: bool, program: &Path) -> io::Result<()> {
    if enabled {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let partial = path.with_extension("desktop.partial");
        std::fs::write(&partial, entry(program))?;
        std::fs::rename(&partial, path)
    } else {
        match std::fs::remove_file(path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_follows_xdg_config_home() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert_eq!(
            path_from(env(&[("HOME", "/home/u")])),
            Some(PathBuf::from("/home/u/.config/autostart/nomouse.desktop"))
        );
        assert_eq!(
            path_from(env(&[("HOME", "/home/u"), ("XDG_CONFIG_HOME", "/cfg")])),
            Some(PathBuf::from("/cfg/autostart/nomouse.desktop"))
        );
        // A relative XDG_CONFIG_HOME is invalid by the spec and ignored.
        assert_eq!(
            path_from(env(&[("HOME", "/home/u"), ("XDG_CONFIG_HOME", "cfg")])),
            Some(PathBuf::from("/home/u/.config/autostart/nomouse.desktop"))
        );
        assert_eq!(path_from(env(&[])), None);
    }

    #[test]
    fn set_writes_and_removes_the_entry() {
        let dir = std::env::temp_dir().join(format!("nomouse-autostart-{}", std::process::id()));
        let path = dir.join("autostart").join(FILE_NAME);
        let program = Path::new("/opt/my apps/nomouse");
        assert!(!is_enabled(&path));
        set(&path, true, program).unwrap();
        assert!(is_enabled(&path));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("Exec=\"/opt/my apps/nomouse\" daemon\n"),
            "{text}"
        );
        set(&path, false, program).unwrap();
        assert!(!path.exists());
        set(&path, false, program).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_hidden_or_disabled_entry_is_off() {
        let dir =
            std::env::temp_dir().join(format!("nomouse-autostart-off-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, "[Desktop Entry]\nHidden=true\n").unwrap();
        assert!(!is_enabled(&path));
        std::fs::write(&path, "[Desktop Entry]\nX-GNOME-Autostart-enabled=false\n").unwrap();
        assert!(!is_enabled(&path));
        assert_eq!(exec_word(Path::new("/usr/bin/nomouse")), "/usr/bin/nomouse");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
