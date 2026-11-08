//! The X11 session: overlay windows, XTEST input, XGrabKey hotkeys and the event loop, all on
//! one connection (the keyboard grab's events must arrive where the hotkeys are read).

pub(crate) mod input;
pub(crate) mod keys;
pub(crate) mod render;

use std::collections::BTreeMap;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use nomouse_core::search::search_box_anchor;
use nomouse_core::{
    Action, Hint, Hotkey, HotkeyId, InputEvent, Modifiers, PlatformError, Point, Rect,
};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::errors::{ConnectionError, ReplyError};
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    self, AtomEnum, ClipOrdering, ColormapAlloc, ConfigureWindowAux, ConnectionExt as _,
    CreateGCAux, CreateWindowAux, EventMask, GrabMode, GrabStatus, ImageFormat, StackMode,
    WindowClass,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::protocol::{Event, randr, xtest};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use input::{Fake, plan};
use keys::{KeyMap, sym};
use render::{Label, Renderer};

fn lost(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::Backend(format!("X connection: {error}"))
}

/// One registered hotkey: its chord and the key grabs made for it.
#[derive(Debug, Clone)]
struct Grab {
    hotkey: Hotkey,
    mask: u16,
    keycodes: Vec<u8>,
}

/// An overlay window covering one monitor.
#[derive(Debug)]
struct OverlayWindow {
    id: xproto::Window,
    gc: xproto::Gcontext,
    area: Rect,
    mapped: bool,
    labels: Vec<Label>,
}

/// The X11 connection and everything that lives on it.
pub(crate) struct X11 {
    conn: RustConnection,
    root: xproto::Window,
    screen: Rect,
    depth: u8,
    visual: xproto::Visualid,
    colormap: xproto::Colormap,
    keymap: KeyMap,
    numlock: u16,
    hotkeys: BTreeMap<HotkeyId, Grab>,
    next_id: u32,
    windows: Vec<OverlayWindow>,
    renderer: Renderer,
    grabbed: bool,
    net_active_window: xproto::Atom,
    net_wm_pid: xproto::Atom,
}

/// What the X server offers, for `nomouse doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerInfo {
    /// `XTEST` version, if present.
    pub xtest: Option<(u8, u16)>,
    /// Whether `SHAPE` is present (required for input-transparent overlays).
    pub shape: bool,
    /// Monitors reported by `RandR`.
    pub monitors: Vec<Rect>,
    /// Whether a 32-bit ARGB visual exists.
    pub argb: bool,
}

fn intern(conn: &RustConnection, name: &str) -> Result<xproto::Atom, PlatformError> {
    Ok(conn
        .intern_atom(false, name.as_bytes())
        .map_err(lost)?
        .reply()
        .map_err(lost)?
        .atom)
}

fn keymap(conn: &RustConnection) -> Result<KeyMap, PlatformError> {
    let setup = conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let reply = conn
        .get_keyboard_mapping(min, max - min + 1)
        .map_err(lost)?
        .reply()
        .map_err(lost)?;
    Ok(KeyMap::new(min, reply.keysyms_per_keycode, reply.keysyms))
}

fn numlock_mask(conn: &RustConnection, map: &KeyMap) -> Result<u16, PlatformError> {
    let reply = conn
        .get_modifier_mapping()
        .map_err(lost)?
        .reply()
        .map_err(lost)?;
    let numlock = map.keycodes(sym::NUM_LOCK);
    let per = usize::from(reply.keycodes_per_modifier());
    Ok(reply
        .keycodes
        .chunks(per.max(1))
        .position(|codes| codes.iter().any(|code| numlock.contains(code)))
        .and_then(|index| u32::try_from(index).ok())
        .map_or(0, |index| 1 << index))
}

fn rect_of(x: i16, y: i16, width: u16, height: u16) -> Rect {
    Rect::new(
        i32::from(x),
        i32::from(y),
        u32::from(width),
        u32::from(height),
    )
}

fn clamp_i16(value: i32) -> i16 {
    i16::try_from(value).unwrap_or(if value < 0 { i16::MIN } else { i16::MAX })
}

fn clamp_u16(value: u32) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

fn monitors_of(conn: &RustConnection, root: xproto::Window, screen: Rect) -> Vec<Rect> {
    let monitors = conn
        .randr_get_monitors(root, true)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| {
            reply
                .monitors
                .iter()
                .map(|m| rect_of(m.x, m.y, m.width, m.height))
                .filter(|r| !r.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if monitors.is_empty() {
        vec![screen]
    } else {
        monitors
    }
}

fn has_extension(conn: &RustConnection, name: &'static str) -> bool {
    conn.extension_information(name).ok().flatten().is_some()
}

/// Opens `$DISPLAY` and reports its extensions and monitors, for `nomouse doctor`.
///
/// # Errors
/// [`PlatformError::Unsupported`] when the display cannot be opened.
pub(crate) fn probe() -> Result<ServerInfo, PlatformError> {
    let (conn, screen_num) = RustConnection::connect(None)
        .map_err(|e| PlatformError::Unsupported(format!("cannot open the X display: {e}")))?;
    let screen = conn
        .setup()
        .roots
        .get(screen_num)
        .ok_or_else(|| lost("no screen"))?;
    let bounds = rect_of(0, 0, screen.width_in_pixels, screen.height_in_pixels);
    let xtest = if has_extension(&conn, xtest::X11_EXTENSION_NAME) {
        conn.xtest_get_version(2, 2)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| (reply.major_version, reply.minor_version))
    } else {
        None
    };
    Ok(ServerInfo {
        xtest,
        shape: has_extension(&conn, shape::X11_EXTENSION_NAME),
        monitors: if has_extension(&conn, randr::X11_EXTENSION_NAME) {
            monitors_of(&conn, screen.root, bounds)
        } else {
            Vec::new()
        },
        argb: screen.allowed_depths.iter().any(|depth| depth.depth == 32),
    })
}

impl X11 {
    /// Connects to `$DISPLAY` and checks for XTEST and SHAPE.
    ///
    /// # Errors
    /// [`PlatformError::Unsupported`] without a display or a required extension.
    pub(crate) fn connect(renderer: Renderer) -> Result<Self, PlatformError> {
        let (conn, screen_num) = RustConnection::connect(None)
            .map_err(|e| PlatformError::Unsupported(format!("cannot open the X display: {e}")))?;
        for name in [xtest::X11_EXTENSION_NAME, shape::X11_EXTENSION_NAME] {
            if conn.extension_information(name).map_err(lost)?.is_none() {
                return Err(PlatformError::Unsupported(format!(
                    "the X server lacks the {name} extension"
                )));
            }
        }
        let screen = conn
            .setup()
            .roots
            .get(screen_num)
            .ok_or_else(|| lost("no screen"))?
            .clone();
        let root = screen.root;
        let argb = screen
            .allowed_depths
            .iter()
            .find(|depth| depth.depth == 32)
            .and_then(|depth| {
                depth
                    .visuals
                    .iter()
                    .find(|v| v.class == xproto::VisualClass::TRUE_COLOR)
            })
            .map(|visual| visual.visual_id);
        let (depth, visual, colormap) = match argb {
            Some(visual) => {
                let colormap = conn.generate_id().map_err(lost)?;
                conn.create_colormap(ColormapAlloc::NONE, colormap, root, visual)
                    .map_err(lost)?
                    .check()
                    .map_err(lost)?;
                (32, visual, colormap)
            }
            None => (
                screen.root_depth,
                screen.root_visual,
                screen.default_colormap,
            ),
        };
        let keymap = keymap(&conn)?;
        let numlock = numlock_mask(&conn, &keymap)?;
        let net_active_window = intern(&conn, "_NET_ACTIVE_WINDOW")?;
        let net_wm_pid = intern(&conn, "_NET_WM_PID")?;
        Ok(Self {
            screen: rect_of(0, 0, screen.width_in_pixels, screen.height_in_pixels),
            conn,
            root,
            depth,
            visual,
            colormap,
            keymap,
            numlock,
            hotkeys: BTreeMap::new(),
            next_id: 1,
            windows: Vec::new(),
            renderer,
            grabbed: false,
            net_active_window,
            net_wm_pid,
        })
    }

    /// The root window's bounds.
    pub(crate) const fn screen(&self) -> Rect {
        self.screen
    }

    /// The monitors (`RandR`), or the whole screen without `RandR`.
    pub(crate) fn monitors(&self) -> Vec<Rect> {
        monitors_of(&self.conn, self.root, self.screen)
    }

    fn property32(
        &self,
        window: xproto::Window,
        atom: xproto::Atom,
        kind: AtomEnum,
    ) -> Option<u32> {
        let reply = self
            .conn
            .get_property(false, window, atom, kind, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        reply.value32()?.next()
    }

    /// The process owning the active window (`_NET_ACTIVE_WINDOW`, `_NET_WM_PID`).
    pub(crate) fn active_pid(&self) -> Option<u32> {
        let window = self
            .property32(self.root, self.net_active_window, AtomEnum::WINDOW)
            .filter(|w| *w != 0)?;
        self.property32(window, self.net_wm_pid, AtomEnum::CARDINAL)
    }

    fn sync(&self) -> Result<(), PlatformError> {
        self.conn.sync().map_err(lost)
    }

    // ---- hotkeys ----

    fn grab_chord(&self, grab: &Grab) -> Result<(), PlatformError> {
        let mut done: Vec<(u8, u16)> = Vec::new();
        for keycode in &grab.keycodes {
            for variant in keys::grab_variants(grab.mask, self.numlock) {
                let result = self
                    .conn
                    .grab_key(
                        false,
                        self.root,
                        xproto::ModMask::from(variant),
                        *keycode,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                    )
                    .map_err(lost)?
                    .check();
                match result {
                    Ok(()) => done.push((*keycode, variant)),
                    Err(ReplyError::X11Error(_)) => {
                        for (code, mask) in done {
                            // Best-effort rollback of the grabs made so far.
                            let _ =
                                self.conn
                                    .ungrab_key(code, self.root, xproto::ModMask::from(mask));
                        }
                        self.sync()?;
                        return Err(PlatformError::HotkeyTaken(grab.hotkey));
                    }
                    Err(error) => return Err(lost(error)),
                }
            }
        }
        Ok(())
    }

    fn ungrab_chord(&self, grab: &Grab) -> Result<(), PlatformError> {
        for keycode in &grab.keycodes {
            for variant in keys::grab_variants(grab.mask, self.numlock) {
                self.conn
                    .ungrab_key(*keycode, self.root, xproto::ModMask::from(variant))
                    .map_err(lost)?;
            }
        }
        self.sync()
    }

    /// Grabs `hotkey` on the root window, in every Caps/Num Lock variant.
    ///
    /// # Errors
    /// [`PlatformError::HotkeyTaken`] when another client holds the chord.
    pub(crate) fn register(&mut self, hotkey: Hotkey) -> Result<HotkeyId, PlatformError> {
        let keycodes = self.keymap.keycodes(keys::hotkey_keysym(hotkey.key));
        let Some(first) = keycodes.first() else {
            return Err(PlatformError::Unsupported(format!(
                "no key on this keyboard layout produces {}",
                hotkey.key
            )));
        };
        let grab = Grab {
            hotkey,
            mask: keys::modifier_mask(hotkey.modifiers),
            keycodes: vec![*first],
        };
        self.grab_chord(&grab)?;
        let id = HotkeyId(self.next_id);
        self.next_id += 1;
        self.hotkeys.insert(id, grab);
        Ok(id)
    }

    /// Releases a hotkey's grabs.
    ///
    /// # Errors
    /// When the connection is lost.
    pub(crate) fn unregister(&mut self, id: HotkeyId) -> Result<(), PlatformError> {
        match self.hotkeys.remove(&id) {
            Some(grab) => self.ungrab_chord(&grab),
            None => Ok(()),
        }
    }

    /// Re-reads the keyboard layout and moves the hotkey grabs to the new keycodes.
    fn remap(&mut self) -> Result<(), PlatformError> {
        let grabs: Vec<Grab> = self.hotkeys.values().cloned().collect();
        for grab in &grabs {
            self.ungrab_chord(grab)?;
        }
        self.keymap = keymap(&self.conn)?;
        self.numlock = numlock_mask(&self.conn, &self.keymap)?;
        for grab in self.hotkeys.values_mut() {
            grab.keycodes = self
                .keymap
                .keycodes(keys::hotkey_keysym(grab.hotkey.key))
                .into_iter()
                .take(1)
                .collect();
        }
        for grab in self.hotkeys.values() {
            if let Err(error) = self.grab_chord(grab) {
                tracing::warn!("re-grabbing {} after a layout change: {error}", grab.hotkey);
            }
        }
        Ok(())
    }

    fn translate(&self, event: &Event) -> Option<InputEvent> {
        let Event::KeyPress(press) = event else {
            return None;
        };
        let state = u16::from(press.state);
        if self.grabbed {
            let keysym = self.keymap.keysym(press.detail, state);
            return keys::key_press(keysym).map(InputEvent::Key);
        }
        self.hotkeys
            .iter()
            .find(|(_, grab)| {
                grab.keycodes.contains(&press.detail)
                    && keys::chord_matches(state, grab.mask, self.numlock)
            })
            .map(|(id, _)| InputEvent::Hotkey(*id))
    }

    fn handle(&mut self, event: &Event) -> Result<Option<InputEvent>, PlatformError> {
        match event {
            Event::Error(error) => {
                tracing::warn!("X error: {error:?}");
                Ok(None)
            }
            Event::MappingNotify(_) => {
                self.remap()?;
                Ok(None)
            }
            Event::Expose(expose) => {
                if let Some(index) = self.windows.iter().position(|w| w.id == expose.window) {
                    self.draw(index)?;
                }
                Ok(None)
            }
            other => Ok(self.translate(other)),
        }
    }

    /// Waits up to `timeout` for a hotkey or, while the keyboard is grabbed, a key press.
    ///
    /// # Errors
    /// When the connection is lost.
    pub(crate) fn next_event(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<InputEvent>, PlatformError> {
        let deadline = Instant::now() + timeout;
        loop {
            self.conn.flush().map_err(lost)?;
            while let Some(event) = self.conn.poll_for_event().map_err(lost)? {
                if let Some(input) = self.handle(&event)? {
                    return Ok(Some(input));
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let timespec = rustix::event::Timespec::try_from(remaining).ok();
            let fd = self.conn.stream().as_fd();
            let mut fds = [rustix::event::PollFd::new(
                &fd,
                rustix::event::PollFlags::IN,
            )];
            match rustix::event::poll(&mut fds, timespec.as_ref()) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(lost(error)),
            }
        }
    }

    // ---- overlay ----

    fn window_for(&mut self, area: Rect) -> Result<usize, PlatformError> {
        if let Some(index) = self.windows.iter().position(|w| w.area == area) {
            return Ok(index);
        }
        let id = self.conn.generate_id().map_err(lost)?;
        let aux = CreateWindowAux::new()
            .override_redirect(1)
            .background_pixmap(x11rb::NONE)
            .border_pixel(0)
            .colormap(self.colormap)
            .event_mask(EventMask::EXPOSURE);
        self.conn
            .create_window(
                self.depth,
                id,
                self.root,
                clamp_i16(area.x),
                clamp_i16(area.y),
                clamp_u16(area.width),
                clamp_u16(area.height),
                0,
                WindowClass::INPUT_OUTPUT,
                self.visual,
                &aux,
            )
            .map_err(lost)?;
        self.conn
            .change_property8(
                xproto::PropMode::REPLACE,
                id,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                b"nomouse-overlay",
            )
            .map_err(lost)?;
        // An empty input region makes the window click-through.
        self.conn
            .shape_rectangles(
                shape::SO::SET,
                shape::SK::INPUT,
                ClipOrdering::UNSORTED,
                id,
                0,
                0,
                &[],
            )
            .map_err(lost)?;
        let gc = self.conn.generate_id().map_err(lost)?;
        self.conn
            .create_gc(gc, id, &CreateGCAux::new().graphics_exposures(0))
            .map_err(lost)?;
        self.windows.push(OverlayWindow {
            id,
            gc,
            area,
            mapped: false,
            labels: Vec::new(),
        });
        Ok(self.windows.len() - 1)
    }

    fn draw(&self, index: usize) -> Result<(), PlatformError> {
        let Some(window) = self.windows.get(index) else {
            return Ok(());
        };
        for label in &window.labels {
            self.conn
                .put_image(
                    ImageFormat::Z_PIXMAP,
                    window.id,
                    window.gc,
                    clamp_u16(label.bounds.width),
                    clamp_u16(label.bounds.height),
                    clamp_i16(label.bounds.x - window.area.x),
                    clamp_i16(label.bounds.y - window.area.y),
                    0,
                    self.depth,
                    &label.pixels,
                )
                .map_err(lost)?;
        }
        Ok(())
    }

    fn grab_keyboard(&mut self) -> Result<(), PlatformError> {
        if self.grabbed {
            return Ok(());
        }
        // A window manager or a held key can hold the keyboard briefly; retry for ~0.5 s.
        for _ in 0..50 {
            let status = self
                .conn
                .grab_keyboard(
                    false,
                    self.root,
                    x11rb::CURRENT_TIME,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .map_err(lost)?
                .reply()
                .map_err(lost)?
                .status;
            if status == GrabStatus::SUCCESS {
                self.grabbed = true;
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(PlatformError::Backend(
            "another program holds the keyboard grab".to_owned(),
        ))
    }

    /// Shows `hints` on the monitors they lie on and grabs the keyboard. With a `query`
    /// (search-by-text) a search box `/query` is drawn near the top centre of the monitor of
    /// the best match (the first hint), or of the first monitor when nothing matches.
    ///
    /// # Errors
    /// When windows cannot be created or the keyboard cannot be grabbed.
    pub(crate) fn show(
        &mut self,
        hints: &[Hint],
        typed: &str,
        query: Option<&str>,
    ) -> Result<(), PlatformError> {
        let monitors = self.monitors();
        let typed = typed.chars().count();
        let mut per_monitor: Vec<Vec<Label>> = vec![Vec::new(); monitors.len()];
        let mut first_monitor = None;
        for hint in hints {
            let bounds = hint.element.bounds;
            let index = monitors
                .iter()
                .position(|m| m.contains(bounds.center()))
                .or_else(|| monitors.iter().position(|m| m.intersects(&bounds)))
                .unwrap_or(0);
            first_monitor.get_or_insert(index);
            if let Some(label) = self
                .renderer
                .label(&hint.label, typed, bounds, monitors[index])
            {
                per_monitor[index].push(label);
            }
        }
        if let Some(query) = query {
            let index = first_monitor.unwrap_or(0);
            let area = monitors[index];
            let text = format!("/{query}");
            let anchor = search_box_anchor(area);
            if let Some(label) = self
                .renderer
                .label(&text, text.chars().count(), anchor, area)
            {
                per_monitor[index].push(label);
            }
        }
        let mut used = Vec::new();
        for (area, labels) in monitors.iter().zip(per_monitor) {
            if labels.is_empty() {
                continue;
            }
            let index = self.window_for(*area)?;
            used.push(index);
            let rects: Vec<xproto::Rectangle> = labels
                .iter()
                .map(|l| xproto::Rectangle {
                    x: clamp_i16(l.bounds.x - area.x),
                    y: clamp_i16(l.bounds.y - area.y),
                    width: clamp_u16(l.bounds.width),
                    height: clamp_u16(l.bounds.height),
                })
                .collect();
            let id = self.windows[index].id;
            self.conn
                .shape_rectangles(
                    shape::SO::SET,
                    shape::SK::BOUNDING,
                    ClipOrdering::UNSORTED,
                    id,
                    0,
                    0,
                    &rects,
                )
                .map_err(lost)?;
            self.windows[index].labels = labels;
            if !self.windows[index].mapped {
                self.conn.map_window(id).map_err(lost)?;
                self.windows[index].mapped = true;
            }
            self.conn
                .configure_window(id, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))
                .map_err(lost)?;
            self.draw(index)?;
        }
        for (index, window) in self.windows.iter_mut().enumerate() {
            if window.mapped && !used.contains(&index) {
                self.conn.unmap_window(window.id).map_err(lost)?;
                window.mapped = false;
                window.labels.clear();
            }
        }
        self.grab_keyboard()?;
        self.sync()
    }

    /// Unmaps every overlay window and releases the keyboard.
    ///
    /// # Errors
    /// When the connection is lost.
    pub(crate) fn hide(&mut self) -> Result<(), PlatformError> {
        for window in &mut self.windows {
            if window.mapped {
                self.conn.unmap_window(window.id).map_err(lost)?;
                window.mapped = false;
            }
            window.labels.clear();
        }
        if self.grabbed {
            self.conn
                .ungrab_keyboard(x11rb::CURRENT_TIME)
                .map_err(lost)?;
            self.grabbed = false;
        }
        self.sync()
    }

    // ---- input ----

    fn modifier_keycodes(&self, modifiers: Modifiers) -> Vec<u8> {
        modifier_keysyms(modifiers)
            .into_iter()
            .filter_map(|keysym| self.keymap.keycodes(keysym).first().copied())
            .collect()
    }

    /// Performs `action` with XTEST, holding its [`Action::modifiers`] (ctrl+click, ...).
    ///
    /// # Errors
    /// When the connection is lost.
    pub(crate) fn perform(&mut self, action: &Action) -> Result<(), PlatformError> {
        let held = self.modifier_keycodes(action.modifiers());
        for fake in plan(action, &held) {
            let (kind, detail, at) = match fake {
                Fake::Motion(at) => (xproto::MOTION_NOTIFY_EVENT, 0, at),
                Fake::Press(button) => (xproto::BUTTON_PRESS_EVENT, button, Point::default()),
                Fake::Release(button) => (xproto::BUTTON_RELEASE_EVENT, button, Point::default()),
                Fake::KeyDown(code) => (xproto::KEY_PRESS_EVENT, code, Point::default()),
                Fake::KeyUp(code) => (xproto::KEY_RELEASE_EVENT, code, Point::default()),
                Fake::Pause(millis) => {
                    self.conn.flush().map_err(lost)?;
                    std::thread::sleep(Duration::from_millis(millis));
                    continue;
                }
            };
            self.conn
                .xtest_fake_input(
                    kind,
                    detail,
                    x11rb::CURRENT_TIME,
                    self.root,
                    clamp_i16(at.x),
                    clamp_i16(at.y),
                    0,
                )
                .map_err(lost)?;
        }
        self.sync()
    }
}

impl Drop for X11 {
    fn drop(&mut self) {
        // Best effort: the server frees everything when the connection closes anyway.
        for window in &self.windows {
            let _ = self.conn.destroy_window(window.id);
        }
        if self.grabbed {
            let _ = self.conn.ungrab_keyboard(x11rb::CURRENT_TIME);
        }
        let _: Result<(), ConnectionError> = self.conn.flush();
    }
}

/// The keysyms pressed for `modifiers`, in press order (released in reverse).
fn modifier_keysyms(modifiers: Modifiers) -> Vec<u32> {
    [
        (modifiers.ctrl, sym::CONTROL_L),
        (modifiers.shift, sym::SHIFT_L),
        (modifiers.alt, sym::ALT_L),
        (modifiers.meta, sym::SUPER_L),
    ]
    .into_iter()
    .filter_map(|(on, keysym)| on.then_some(keysym))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_map_to_left_keysyms_in_press_order() {
        assert_eq!(modifier_keysyms(Modifiers::NONE), Vec::<u32>::new());
        let all = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            meta: true,
        };
        assert_eq!(
            modifier_keysyms(all),
            vec![sym::CONTROL_L, sym::SHIFT_L, sym::ALT_L, sym::SUPER_L]
        );
        let meta = Modifiers {
            meta: true,
            ..Modifiers::NONE
        };
        assert_eq!(modifier_keysyms(meta), vec![sym::SUPER_L]);
    }
}
