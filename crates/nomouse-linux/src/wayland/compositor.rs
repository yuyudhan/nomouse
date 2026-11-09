//! Window positions from the compositor's own IPC.
//!
//! AT-SPI cannot place native Wayland windows: a Wayland client does not know where its window
//! is, so GTK and Qt report "screen" extents relative to the window's top-left corner. `XWayland`
//! clients report real coordinates. The compositor knows every window's position, so this module
//! asks it: Sway through `get_tree` on `$SWAYSOCK`, Hyprland through `j/clients` on its request
//! socket. Other compositors (GNOME, KDE) have no such interface for clients; there, native
//! Wayland windows cannot be labelled at the right place, which `nomouse doctor` reports.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nomouse_core::{AppIdentity, Point, Rect};
use serde_json::Value;

/// How long one IPC exchange may take before the scan goes on without window positions.
const IPC_TIMEOUT: Duration = Duration::from_millis(300);

/// The i3/Sway IPC magic.
const SWAY_MAGIC: &[u8; 6] = b"i3-ipc";
/// Sway IPC message types.
const SWAY_GET_TREE: u32 = 4;
const SWAY_GET_VERSION: u32 = 7;

/// One application window as the compositor sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowInfo {
    /// The owning process.
    pub pid: u32,
    /// The window's content area in layout (logical) coordinates.
    pub content: Rect,
    /// A native Wayland client (`false` for `XWayland`, whose AT-SPI coordinates are global).
    pub native: bool,
    /// The window has keyboard focus.
    pub focused: bool,
    /// The window is on a visible workspace.
    pub visible: bool,
    /// The app id (native windows) or `WM_CLASS` names (`XWayland` windows); no process name.
    pub app: AppIdentity,
}

/// The windows on screen and the focused one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Windows {
    /// Every application window found.
    pub all: Vec<WindowInfo>,
}

impl Windows {
    /// The focused window.
    pub(crate) fn focused(&self) -> Option<&WindowInfo> {
        self.all.iter().find(|w| w.focused)
    }

    /// The process owning the focused window.
    pub(crate) fn focused_pid(&self) -> Option<u32> {
        self.focused().map(|w| w.pid)
    }

    /// The offset to add to each native Wayland process's window-relative AT-SPI extents: the
    /// origin of its focused window, else of its first visible one. `XWayland` processes get none.
    pub(crate) fn native_offsets(&self) -> HashMap<u32, Point> {
        let mut offsets = HashMap::new();
        let candidates = self
            .all
            .iter()
            .filter(|w| w.native && w.focused)
            .chain(self.all.iter().filter(|w| w.native && w.visible));
        for window in candidates {
            offsets
                .entry(window.pid)
                .or_insert(Point::new(window.content.x, window.content.y));
        }
        offsets
    }
}

/// The compositor of this Wayland session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Compositor {
    /// Sway, with its IPC socket.
    Sway(PathBuf),
    /// Hyprland, with its request socket.
    Hyprland(PathBuf),
    /// GNOME Shell (Mutter).
    Gnome,
    /// KDE Plasma (`KWin`).
    Kde,
    /// Anything else, by its `XDG_CURRENT_DESKTOP`.
    Other(String),
}

impl Compositor {
    /// Detects the compositor from the environment.
    pub(crate) fn detect(env: impl Fn(&str) -> Option<String>) -> Self {
        let get = |name: &str| env(name).filter(|v| !v.is_empty());
        if let Some(socket) = get("SWAYSOCK") {
            return Self::Sway(PathBuf::from(socket));
        }
        if let Some(signature) = get("HYPRLAND_INSTANCE_SIGNATURE") {
            let runtime = get("XDG_RUNTIME_DIR").map(PathBuf::from);
            return Self::Hyprland(hyprland_socket(runtime.as_deref(), &signature));
        }
        let desktop = get("XDG_CURRENT_DESKTOP").unwrap_or_default();
        let upper = desktop.to_ascii_uppercase();
        if upper.split(':').any(|part| part == "GNOME") {
            Self::Gnome
        } else if upper.split(':').any(|part| part == "KDE") {
            Self::Kde
        } else {
            Self::Other(desktop)
        }
    }

    /// Detects the compositor from this process's environment.
    pub(crate) fn current() -> Self {
        Self::detect(|name| std::env::var(name).ok())
    }

    /// A short name for messages.
    pub(crate) fn name(&self) -> String {
        match self {
            Self::Sway(_) => "Sway".to_owned(),
            Self::Hyprland(_) => "Hyprland".to_owned(),
            Self::Gnome => "GNOME".to_owned(),
            Self::Kde => "KDE Plasma".to_owned(),
            Self::Other(desktop) if desktop.is_empty() => "an unknown compositor".to_owned(),
            Self::Other(desktop) => desktop.clone(),
        }
    }

    /// Whether native Wayland window positions can be read.
    pub(crate) const fn locates_windows(&self) -> bool {
        matches!(self, Self::Sway(_) | Self::Hyprland(_))
    }

    /// The compositor's version, when its IPC tells.
    ///
    /// # Errors
    /// When the IPC socket cannot be reached or answers nonsense.
    pub(crate) fn version(&self) -> Result<String, String> {
        match self {
            Self::Sway(socket) => {
                let reply = sway_request(socket, SWAY_GET_VERSION)?;
                Ok(reply["human_readable"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_owned())
            }
            Self::Hyprland(socket) => {
                let reply = hyprland_request(socket, "j/version")?;
                Ok(reply["tag"].as_str().unwrap_or("unknown").to_owned())
            }
            _ => Err(format!("{} has no client IPC", self.name())),
        }
    }

    /// The windows on screen.
    ///
    /// # Errors
    /// When the compositor has no IPC or the socket fails.
    pub(crate) fn windows(&self) -> Result<Windows, String> {
        match self {
            Self::Sway(socket) => Ok(sway_windows(&sway_request(socket, SWAY_GET_TREE)?)),
            Self::Hyprland(socket) => {
                let clients = hyprland_request(socket, "j/clients")?;
                let active = hyprland_request(socket, "j/activewindow")?;
                Ok(hyprland_windows(&clients, &active))
            }
            _ => Err(format!(
                "{} exposes no window positions to clients",
                self.name()
            )),
        }
    }
}

/// Hyprland's request socket: `$XDG_RUNTIME_DIR/hypr/<sig>/.socket.sock` (0.40 and later),
/// else `/tmp/hypr/<sig>/.socket.sock` (older releases).
fn hyprland_socket(runtime: Option<&Path>, signature: &str) -> PathBuf {
    let current = runtime.map(|dir| dir.join("hypr").join(signature).join(".socket.sock"));
    match current {
        Some(path) if path.exists() => path,
        _ => PathBuf::from("/tmp/hypr")
            .join(signature)
            .join(".socket.sock"),
    }
}

fn connect(socket: &Path) -> Result<UnixStream, String> {
    let stream = UnixStream::connect(socket)
        .map_err(|e| format!("cannot reach {}: {e}", socket.display()))?;
    stream
        .set_read_timeout(Some(IPC_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(IPC_TIMEOUT)))
        .map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Encodes one i3/Sway IPC message.
pub(crate) fn sway_message(kind: u32, payload: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(14 + payload.len());
    message.extend_from_slice(SWAY_MAGIC);
    message.extend_from_slice(
        &u32::try_from(payload.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    message.extend_from_slice(&kind.to_le_bytes());
    message.extend_from_slice(payload);
    message
}

fn sway_request(socket: &Path, kind: u32) -> Result<Value, String> {
    let io = |e: std::io::Error| format!("sway IPC: {e}");
    let mut stream = connect(socket)?;
    stream.write_all(&sway_message(kind, &[])).map_err(io)?;
    let mut header = [0_u8; 14];
    stream.read_exact(&mut header).map_err(io)?;
    if &header[..6] != SWAY_MAGIC {
        return Err("sway IPC: bad reply magic".to_owned());
    }
    let mut length = [0_u8; 4];
    length.copy_from_slice(&header[6..10]);
    let length = usize::try_from(u32::from_le_bytes(length)).map_err(|e| e.to_string())?;
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).map_err(io)?;
    serde_json::from_slice(&body).map_err(|e| format!("sway IPC: {e}"))
}

fn hyprland_request(socket: &Path, request: &str) -> Result<Value, String> {
    let io = |e: std::io::Error| format!("Hyprland IPC: {e}");
    let mut stream = connect(socket)?;
    stream.write_all(request.as_bytes()).map_err(io)?;
    let mut body = Vec::new();
    stream.read_to_end(&mut body).map_err(io)?;
    serde_json::from_slice(&body).map_err(|e| format!("Hyprland IPC: {e}"))
}

fn int(value: &Value) -> i32 {
    value
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .unwrap_or(0)
}

fn size(value: &Value) -> u32 {
    value
        .as_i64()
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0)
}

fn pid_of(value: &Value) -> Option<u32> {
    value
        .as_i64()
        .and_then(|v| u32::try_from(v).ok())
        .filter(|pid| *pid > 0)
}

fn text(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// A Sway view's identity: `app_id` for native windows, `window_properties` `instance` and
/// `class` for `XWayland` ones.
fn sway_app(node: &Value) -> AppIdentity {
    let properties = &node["window_properties"];
    AppIdentity {
        app_id: text(&node["app_id"]),
        wm_class: [&properties["instance"], &properties["class"]]
            .into_iter()
            .filter_map(text)
            .collect(),
        process: None,
    }
}

/// A Hyprland client's identity: its `class` is the app id of a native window and the
/// `WM_CLASS` class of an `XWayland` one.
fn hyprland_app(client: &Value, xwayland: bool) -> AppIdentity {
    let class = text(&client["class"]);
    if xwayland {
        AppIdentity {
            wm_class: class.into_iter().collect(),
            ..AppIdentity::default()
        }
    } else {
        AppIdentity {
            app_id: class,
            ..AppIdentity::default()
        }
    }
}

/// The application windows of a Sway `get_tree` reply. The content origin is the container's
/// `rect` plus the view's `window_rect` offset (borders and title bar).
pub(crate) fn sway_windows(tree: &Value) -> Windows {
    let mut all = Vec::new();
    let mut stack = vec![tree];
    while let Some(node) = stack.pop() {
        if let Some(pid) = pid_of(&node["pid"]) {
            let rect = &node["rect"];
            let inner = &node["window_rect"];
            all.push(WindowInfo {
                pid,
                content: Rect::new(
                    int(&rect["x"]).saturating_add(int(&inner["x"])),
                    int(&rect["y"]).saturating_add(int(&inner["y"])),
                    size(&inner["width"]),
                    size(&inner["height"]),
                ),
                native: node["shell"].as_str() != Some("xwayland"),
                focused: node["focused"].as_bool().unwrap_or(false),
                visible: node["visible"].as_bool().unwrap_or(false),
                app: sway_app(node),
            });
        }
        // Pushed in reverse so tiled windows come before floating ones, in tree order.
        for key in ["floating_nodes", "nodes"] {
            if let Some(children) = node[key].as_array() {
                stack.extend(children.iter().rev());
            }
        }
    }
    Windows { all }
}

/// The application windows of Hyprland's `j/clients`, `active` being `j/activewindow`.
pub(crate) fn hyprland_windows(clients: &Value, active: &Value) -> Windows {
    let active_address = active["address"].as_str();
    let all = clients
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|client| {
                    let pid = pid_of(&client["pid"])?;
                    let xwayland = client["xwayland"].as_bool().unwrap_or(false);
                    Some(WindowInfo {
                        pid,
                        content: Rect::new(
                            int(&client["at"][0]),
                            int(&client["at"][1]),
                            size(&client["size"][0]),
                            size(&client["size"][1]),
                        ),
                        native: !xwayland,
                        focused: active_address.is_some()
                            && client["address"].as_str() == active_address,
                        visible: client["mapped"].as_bool().unwrap_or(true)
                            && !client["hidden"].as_bool().unwrap_or(false),
                        app: hyprland_app(client, xwayland),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Windows { all }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    #[test]
    fn detection() {
        assert_eq!(
            Compositor::detect(env(&[("SWAYSOCK", "/run/sway.sock")])),
            Compositor::Sway(PathBuf::from("/run/sway.sock"))
        );
        assert_eq!(
            Compositor::detect(env(&[("HYPRLAND_INSTANCE_SIGNATURE", "abc")])),
            Compositor::Hyprland(PathBuf::from("/tmp/hypr/abc/.socket.sock"))
        );
        assert_eq!(
            Compositor::detect(env(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")])),
            Compositor::Gnome
        );
        assert_eq!(
            Compositor::detect(env(&[("XDG_CURRENT_DESKTOP", "KDE")])),
            Compositor::Kde
        );
        assert_eq!(
            Compositor::detect(env(&[("XDG_CURRENT_DESKTOP", "river")])),
            Compositor::Other("river".to_owned())
        );
        assert!(Compositor::Sway(PathBuf::new()).locates_windows());
        assert!(!Compositor::Gnome.locates_windows());
    }

    #[test]
    fn sway_message_framing() {
        let message = sway_message(SWAY_GET_TREE, b"{}");
        assert_eq!(&message[..6], b"i3-ipc");
        assert_eq!(&message[6..10], &2_u32.to_le_bytes());
        assert_eq!(&message[10..14], &4_u32.to_le_bytes());
        assert_eq!(&message[14..], b"{}");
    }

    #[test]
    fn sway_tree_is_flattened_with_content_origins() {
        let tree = serde_json::json!({
            "type": "root", "nodes": [{
                "type": "output", "nodes": [{
                    "type": "workspace",
                    "nodes": [
                        {"pid": 10, "shell": "xdg_shell", "focused": false, "visible": true,
                         "app_id": "org.gnome.Terminal",
                         "rect": {"x": 0, "y": 0, "width": 800, "height": 1000},
                         "window_rect": {"x": 2, "y": 24, "width": 796, "height": 974}},
                        {"pid": 11, "shell": "xwayland", "focused": true, "visible": true,
                         "app_id": null,
                         "window_properties": {"class": "XTerm", "instance": "xterm"},
                         "rect": {"x": 800, "y": 0, "width": 800, "height": 1000},
                         "window_rect": {"x": 0, "y": 0, "width": 800, "height": 1000}}
                    ],
                    "floating_nodes": [
                        {"pid": 12, "shell": "xdg_shell", "focused": false, "visible": false,
                         "rect": {"x": 100, "y": 100, "width": 300, "height": 200},
                         "window_rect": {"x": 0, "y": 0, "width": 300, "height": 200}}
                    ]
                }]
            }]
        });
        let windows = sway_windows(&tree);
        assert_eq!(windows.all.len(), 3);
        assert_eq!(windows.all[0].pid, 10);
        assert_eq!(windows.all[0].content, Rect::new(2, 24, 796, 974));
        assert!(windows.all[0].native);
        assert!(!windows.all[1].native);
        assert_eq!(windows.focused_pid(), Some(11));
        assert_eq!(
            windows.all[0].app.app_id.as_deref(),
            Some("org.gnome.Terminal")
        );
        assert_eq!(
            windows.focused().map(|w| &w.app),
            Some(&AppIdentity {
                app_id: None,
                wm_class: vec!["xterm".to_owned(), "XTerm".to_owned()],
                process: None,
            })
        );
        let offsets = windows.native_offsets();
        assert_eq!(offsets.get(&10), Some(&Point::new(2, 24)));
        assert_eq!(
            offsets.get(&11),
            None,
            "XWayland coordinates are already global"
        );
        assert_eq!(offsets.get(&12), None, "hidden windows give no offset");
    }

    #[test]
    fn focused_native_window_wins_the_offset() {
        let window = |focused, x| WindowInfo {
            pid: 5,
            content: Rect::new(x, 0, 10, 10),
            native: true,
            focused,
            visible: true,
            app: AppIdentity::default(),
        };
        let windows = Windows {
            all: vec![window(false, 100), window(true, 300)],
        };
        assert_eq!(windows.native_offsets().get(&5), Some(&Point::new(300, 0)));
    }

    #[test]
    fn hyprland_clients() {
        let clients = serde_json::json!([
            {"address": "0x1", "pid": 20, "at": [10, 40], "size": [640, 480],
             "xwayland": false, "mapped": true, "hidden": false, "class": "firefox"},
            {"address": "0x2", "pid": 21, "at": [700, 40], "size": [300, 300],
             "xwayland": true, "mapped": true, "hidden": false, "class": "XTerm"},
            {"address": "0x3", "pid": -1, "at": [0, 0], "size": [0, 0]}
        ]);
        let active = serde_json::json!({"address": "0x2"});
        let windows = hyprland_windows(&clients, &active);
        assert_eq!(windows.all.len(), 2);
        assert_eq!(windows.focused_pid(), Some(21));
        assert_eq!(windows.all[0].app.app_id.as_deref(), Some("firefox"));
        assert_eq!(windows.all[1].app.wm_class, ["XTerm"]);
        assert_eq!(windows.all[1].app.app_id, None);
        assert_eq!(windows.all[0].content, Rect::new(10, 40, 640, 480));
        assert_eq!(windows.native_offsets().get(&20), Some(&Point::new(10, 40)));
    }
}
