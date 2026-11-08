//! The Wayland session (wlroots compositors): a layer-shell overlay that takes the keyboard
//! exclusively while labels are up, virtual pointer/keyboard input (or `/dev/uinput`), and
//! window positions from the compositor's IPC.
//!
//! Wayland has no global hotkeys for clients: the compositor binds `nomouse hints` (and the
//! other modes) and the command reaches the daemon over its socket.

pub(crate) mod compositor;
mod input;
pub(crate) mod platform;
mod shm;
mod state;

use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use nomouse_core::search::search_box_anchor;
use nomouse_core::{Action, Hint, InputEvent, PlatformError, Rect};
use wayland_client::globals::{GlobalList, registry_queue_init};
use wayland_client::protocol::{wl_compositor, wl_output, wl_seat, wl_shm};
use wayland_client::{Connection, EventQueue, Proxy};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1;

use self::input::{Step, Uinput, Virtual};
use self::state::{State, Surface};
use crate::x11::input::plan;
use crate::x11::render::{Label, Renderer};

pub(crate) use self::input::uinput_writable;

/// The layer-shell namespace of the overlay surfaces.
const NAMESPACE: &str = "nomouse-overlay";
/// How long the compositor gets to deliver an absolute move before the relative nudge after it.
const ENTER_SETTLE_MS: u64 = 15;
/// How long injected input gets to reach clients before the injecting devices go away.
const TEARDOWN_SETTLE: Duration = Duration::from_millis(150);

/// How long `show` waits for the overlay to get keyboard focus.
const FOCUS_WAIT: Duration = Duration::from_millis(1500);

fn lost(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::Backend(format!("Wayland connection: {error}"))
}

/// How input reaches applications.
enum Injector {
    /// wlroots virtual pointer (and keyboard).
    Virtual(Virtual),
    /// `/dev/uinput`, created on the first action.
    Uinput(Option<Uinput>),
}

/// What the compositor offers, for `nomouse doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Globals {
    /// `zwlr_layer_shell_v1` version.
    pub layer_shell: Option<u32>,
    /// `zwlr_virtual_pointer_manager_v1` version.
    pub virtual_pointer: Option<u32>,
    /// `zwp_virtual_keyboard_manager_v1` version.
    pub virtual_keyboard: Option<u32>,
    /// `zxdg_output_manager_v1` version.
    pub xdg_output: Option<u32>,
    /// Output names and layout areas.
    pub outputs: Vec<(String, Rect)>,
}

fn version_of(globals: &GlobalList, interface: &str) -> Option<u32> {
    globals.contents().with_list(|list| {
        list.iter()
            .find(|global| global.interface == interface)
            .map(|global| global.version)
    })
}

/// The Wayland connection and everything that lives on it.
pub(crate) struct Wayland {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    renderer: Renderer,
    injector: Injector,
    frame: Vec<u8>,
    /// When the last action was injected, so teardown can let clients take it first.
    last_action: Option<Instant>,
}

fn connect() -> Result<(Connection, GlobalList, EventQueue<State>, State), PlatformError> {
    let conn = Connection::connect_to_env().map_err(|e| {
        PlatformError::Unsupported(format!("cannot connect to the Wayland compositor: {e}"))
    })?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(lost)?;
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals
        .bind(&qh, 4..=6, ())
        .map_err(|e| PlatformError::Unsupported(format!("wl_compositor: {e}")))?;
    let shm: wl_shm::WlShm = globals
        .bind(&qh, 1..=1, ())
        .map_err(|e| PlatformError::Unsupported(format!("wl_shm: {e}")))?;
    let mut state = State {
        compositor,
        shm,
        layer_shell: globals.bind(&qh, 1..=4, ()).ok(),
        xdg_outputs: globals
            .bind::<zxdg_output_manager_v1::ZxdgOutputManagerV1, _, _>(&qh, 2..=3, ())
            .ok(),
        pointer_manager: globals.bind(&qh, 1..=2, ()).ok(),
        keyboard_manager: globals.bind(&qh, 1..=1, ()).ok(),
        seat: globals.bind::<wl_seat::WlSeat, _, _>(&qh, 1..=7, ()).ok(),
        keyboard: None,
        layout: None,
        outputs: Vec::new(),
        surfaces: Vec::new(),
        focused: false,
        pending: std::collections::VecDeque::new(),
    };
    let outputs: Vec<(u32, u32)> = globals.contents().with_list(|list| {
        list.iter()
            .filter(|g| g.interface == wl_output::WlOutput::interface().name)
            .map(|g| (g.name, g.version))
            .collect()
    });
    for (name, version) in outputs {
        state.add_output(globals.registry(), name, version, &qh);
    }
    // Two round trips: the globals' first events (output geometry, seat capabilities), then
    // the objects they create (xdg_output details, the keyboard's keymap).
    for _ in 0..2 {
        queue.roundtrip(&mut state).map_err(lost)?;
    }
    Ok((conn, globals, queue, state))
}

/// Connects and reports what the compositor offers, for `nomouse doctor`.
///
/// # Errors
/// [`PlatformError::Unsupported`] when the compositor cannot be reached.
pub(crate) fn probe() -> Result<Globals, PlatformError> {
    let (_conn, globals, _queue, state) = connect()?;
    Ok(Globals {
        layer_shell: version_of(
            &globals,
            zwlr_layer_shell_v1::ZwlrLayerShellV1::interface().name,
        ),
        virtual_pointer: version_of(
            &globals,
            zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1::interface().name,
        ),
        virtual_keyboard: version_of(
            &globals,
            zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1::interface().name,
        ),
        xdg_output: version_of(
            &globals,
            zxdg_output_manager_v1::ZxdgOutputManagerV1::interface().name,
        ),
        outputs: state
            .outputs
            .iter()
            .map(|o| (o.name.clone(), o.area()))
            .collect(),
    })
}

/// The bounding box of `areas` (the layout), or an empty rectangle.
pub(crate) fn bounding(areas: impl IntoIterator<Item = Rect>) -> Rect {
    let mut areas = areas.into_iter().filter(|a| !a.is_empty());
    let Some(first) = areas.next() else {
        return Rect::default();
    };
    let (mut left, mut top) = (i64::from(first.x), i64::from(first.y));
    let mut right = left + i64::from(first.width);
    let mut bottom = top + i64::from(first.height);
    for area in areas {
        left = left.min(i64::from(area.x));
        top = top.min(i64::from(area.y));
        right = right.max(i64::from(area.x) + i64::from(area.width));
        bottom = bottom.max(i64::from(area.y) + i64::from(area.height));
    }
    Rect::new(
        i32::try_from(left).unwrap_or(i32::MIN),
        i32::try_from(top).unwrap_or(i32::MIN),
        u32::try_from(right - left).unwrap_or(u32::MAX),
        u32::try_from(bottom - top).unwrap_or(u32::MAX),
    )
}

impl Wayland {
    /// Connects to `$WAYLAND_DISPLAY` and checks for layer-shell.
    ///
    /// # Errors
    /// [`PlatformError::Unsupported`] when the compositor cannot be reached or has no
    /// `zwlr_layer_shell_v1` (GNOME), or when neither virtual input nor `/dev/uinput` works.
    pub(crate) fn connect(renderer: Renderer) -> Result<Self, PlatformError> {
        let (conn, _globals, queue, state) = connect()?;
        if state.layer_shell.is_none() {
            return Err(PlatformError::Unsupported(
                "this compositor has no wlr-layer-shell (GNOME's Mutter does not implement it), \
                 so nomouse cannot draw its overlay; log in to an X11 session instead"
                    .to_owned(),
            ));
        }
        let injector = if let Some(manager) = &state.pointer_manager {
            let qh = queue.handle();
            let pointer = manager.create_virtual_pointer(state.seat.as_ref(), &qh, ());
            let keyboard = match (&state.keyboard_manager, &state.seat, &state.layout) {
                (Some(manager), Some(seat), Some(layout)) => {
                    let keyboard = manager.create_virtual_keyboard(seat, &qh, ());
                    upload_keymap(&keyboard, &layout.text)?;
                    Some(keyboard)
                }
                _ => None,
            };
            Injector::Virtual(Virtual {
                pointer,
                keyboard,
                modifiers: 0,
                epoch: Instant::now(),
            })
        } else {
            uinput_writable().map_err(|e| {
                PlatformError::Unsupported(format!(
                    "the compositor has no wlr virtual pointer and {e}; add the udev rule \
                     from the README so your user can write /dev/uinput"
                ))
            })?;
            Injector::Uinput(None)
        };
        Ok(Self {
            conn,
            queue,
            state,
            renderer,
            injector,
            frame: Vec::new(),
            last_action: None,
        })
    }

    /// Output areas in layout coordinates.
    pub(crate) fn outputs(&self) -> Vec<Rect> {
        self.state.outputs.iter().map(state::Output::area).collect()
    }

    /// The bounding box of every output.
    pub(crate) fn screen(&self) -> Rect {
        bounding(self.outputs())
    }

    fn dispatch(&mut self) -> Result<(), PlatformError> {
        self.queue.dispatch_pending(&mut self.state).map_err(lost)?;
        self.queue.flush().map_err(lost)
    }

    fn roundtrip(&mut self) -> Result<(), PlatformError> {
        self.queue
            .roundtrip(&mut self.state)
            .map_err(lost)
            .map(drop)
    }

    /// Waits up to `timeout` for a key press while the overlay has the keyboard. Wayland has no
    /// client hotkeys, so nothing else arrives here.
    ///
    /// # Errors
    /// When the connection is lost.
    pub(crate) fn next_event(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<InputEvent>, PlatformError> {
        let deadline = Instant::now() + timeout;
        loop {
            self.dispatch()?;
            if let Some(event) = self.state.pending.pop_front() {
                return Ok(Some(event));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            self.read_events(remaining)?;
        }
    }

    /// Waits up to `timeout` for the compositor's next events and reads them (without
    /// dispatching).
    fn read_events(&mut self, timeout: Duration) -> Result<(), PlatformError> {
        let Some(guard) = self.queue.prepare_read() else {
            return Ok(());
        };
        let timespec = rustix::event::Timespec::try_from(timeout).ok();
        let readable = {
            let fd = guard.connection_fd();
            let mut fds = [rustix::event::PollFd::new(
                &fd,
                rustix::event::PollFlags::IN,
            )];
            match rustix::event::poll(&mut fds, timespec.as_ref()) {
                Ok(count) => count > 0,
                Err(rustix::io::Errno::INTR) => false,
                Err(error) => return Err(lost(error)),
            }
        };
        if readable {
            guard.read().map_err(lost)?;
        }
        Ok(())
    }

    /// Waits until an overlay surface has the keyboard, so keys typed right after the labels
    /// appear reach nomouse, not the window.
    fn await_keyboard(&mut self) -> Result<(), PlatformError> {
        let deadline = Instant::now() + FOCUS_WAIT;
        loop {
            self.dispatch()?;
            if self.state.focused {
                return Ok(());
            }
            if self.state.keyboard.is_none() {
                return Err(PlatformError::Backend(
                    "the seat has no keyboard to type labels with".to_owned(),
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(PlatformError::Backend(
                    "the compositor did not give the overlay keyboard focus (another surface \
                     may hold it exclusively, such as a lock screen)"
                        .to_owned(),
                ));
            }
            self.read_events(remaining)?;
        }
    }

    // ---- overlay ----

    fn ensure_surfaces(&mut self) -> Result<(), PlatformError> {
        let Some(shell) = self.state.layer_shell.clone() else {
            return Err(PlatformError::Unsupported("no wlr-layer-shell".to_owned()));
        };
        self.state.surfaces.retain(|surface| {
            if surface.closed {
                surface.layer.destroy();
                surface.wl.destroy();
            }
            !surface.closed
        });
        let qh = self.queue.handle();
        let mut created = false;
        for output in &self.state.outputs {
            if self
                .state
                .surfaces
                .iter()
                .any(|s| s.output == output.global)
            {
                continue;
            }
            let wl = self.state.compositor.create_surface(&qh, ());
            // An empty input region: pointer events (and the clicks nomouse sends) go through.
            let region = self.state.compositor.create_region(&qh, ());
            wl.set_input_region(Some(&region));
            region.destroy();
            let layer = shell.get_layer_surface(
                &wl,
                Some(&output.wl),
                zwlr_layer_shell_v1::Layer::Overlay,
                NAMESPACE.to_owned(),
                &qh,
                (),
            );
            layer.set_anchor(
                zwlr_layer_surface_v1::Anchor::Top
                    | zwlr_layer_surface_v1::Anchor::Bottom
                    | zwlr_layer_surface_v1::Anchor::Left
                    | zwlr_layer_surface_v1::Anchor::Right,
            );
            layer.set_exclusive_zone(-1);
            layer.set_keyboard_interactivity(
                zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
            );
            wl.commit();
            self.state.surfaces.push(Surface {
                output: output.global,
                wl,
                layer,
                size: None,
                closed: false,
            });
            created = true;
        }
        if created {
            // The compositor answers the first commit with a configure.
            self.roundtrip()?;
        }
        Ok(())
    }

    /// Shows `hints` on the outputs they lie on and takes the keyboard. With a `query`
    /// (search-by-text) a search box `/query` is drawn near the top centre of the output of
    /// the best match (the first hint), or of the first output when nothing matches.
    ///
    /// # Errors
    /// When the surfaces cannot be created or drawn.
    pub(crate) fn show(
        &mut self,
        hints: &[Hint],
        typed: &str,
        query: Option<&str>,
    ) -> Result<(), PlatformError> {
        self.dispatch()?;
        self.ensure_surfaces()?;
        let outputs: Vec<(u32, Rect)> = self
            .state
            .outputs
            .iter()
            .map(|o| (o.global, o.area()))
            .collect();
        if outputs.is_empty() {
            return Err(PlatformError::Backend("no outputs".to_owned()));
        }
        let typed = typed.chars().count();
        let mut per_output: Vec<Vec<Label>> = vec![Vec::new(); outputs.len()];
        let mut first_output = None;
        for hint in hints {
            let bounds = hint.element.bounds;
            let index = outputs
                .iter()
                .position(|(_, area)| area.contains(bounds.center()))
                .or_else(|| {
                    outputs
                        .iter()
                        .position(|(_, area)| area.intersects(&bounds))
                })
                .unwrap_or(0);
            first_output.get_or_insert(index);
            if let Some(label) = self
                .renderer
                .label(&hint.label, typed, bounds, outputs[index].1)
            {
                per_output[index].push(label);
            }
        }
        if let Some(query) = query {
            let index = first_output.unwrap_or(0);
            let area = outputs[index].1;
            let text = format!("/{query}");
            if let Some(label) =
                self.renderer
                    .label(&text, text.chars().count(), search_box_anchor(area), area)
            {
                per_output[index].push(label);
            }
        }
        let qh = self.queue.handle();
        for ((global, area), labels) in outputs.iter().zip(per_output) {
            let Some(surface) = self.state.surfaces.iter().find(|s| s.output == *global) else {
                continue;
            };
            let (width, height) = match surface.size {
                Some((w, h)) if w > 0 && h > 0 => (w, h),
                _ => (area.width, area.height),
            };
            if width == 0 || height == 0 {
                continue;
            }
            shm::compose(&mut self.frame, width, height, *area, &labels);
            let buffer = shm::buffer(&self.state.shm, &self.frame, width, height, &qh)?;
            surface.wl.attach(Some(&buffer), 0, 0);
            surface.wl.damage_buffer(
                0,
                0,
                i32::try_from(width).unwrap_or(i32::MAX),
                i32::try_from(height).unwrap_or(i32::MAX),
            );
            surface.wl.commit();
        }
        self.roundtrip()?;
        self.await_keyboard()
    }

    /// Destroys the overlay surfaces, which hands the keyboard back to the focused window.
    ///
    /// # Errors
    /// When the connection is lost.
    pub(crate) fn hide(&mut self) -> Result<(), PlatformError> {
        if self.state.surfaces.is_empty() {
            return Ok(());
        }
        for surface in self.state.surfaces.drain(..) {
            surface.layer.destroy();
            surface.wl.destroy();
        }
        self.state.focused = false;
        self.state.pending.clear();
        self.roundtrip()
    }

    // ---- input ----

    /// Performs `action`, holding its modifiers.
    ///
    /// # Errors
    /// When the connection or the uinput device fails.
    pub(crate) fn perform(&mut self, action: &Action) -> Result<(), PlatformError> {
        let layout = self.screen();
        if layout.is_empty() {
            return Err(PlatformError::Backend("no outputs".to_owned()));
        }
        let held = input::modifier_keycodes(action.modifiers());
        let steps: Vec<Step> = plan(action, &held)
            .into_iter()
            .filter_map(|fake| input::step(fake, layout))
            .collect();
        for step in steps {
            if let Step::Pause(millis) = step {
                self.queue.flush().map_err(lost)?;
                std::thread::sleep(Duration::from_millis(millis));
                continue;
            }
            match &mut self.injector {
                Injector::Virtual(device) => {
                    // Xwayland does not take its pointer position from an absolute move that
                    // carries the pointer onto its surface from a native one: the click then
                    // lands at its stale position (the last native click). A relative nudge
                    // there and back, after the move is delivered, makes it resync.
                    let motion = matches!(step, Step::Motion(..));
                    device.send(step, layout);
                    if motion {
                        self.queue.flush().map_err(lost)?;
                        std::thread::sleep(Duration::from_millis(ENTER_SETTLE_MS));
                        device.wiggle();
                    }
                }
                Injector::Uinput(device) => {
                    let device = match device {
                        Some(existing) if existing.layout() == layout => existing,
                        slot => slot.insert(Uinput::new(layout)?),
                    };
                    device.send(step)?;
                }
            }
        }
        let result = self.roundtrip();
        self.last_action = Some(Instant::now());
        result
    }

    /// Which input path is in use, for logs.
    pub(crate) const fn input_name(&self) -> &'static str {
        match self.injector {
            Injector::Virtual(_) => "wlr virtual pointer",
            Injector::Uinput(_) => "/dev/uinput",
        }
    }
}

impl Drop for Wayland {
    fn drop(&mut self) {
        // Destroying the virtual pointer right after a click removes the seat's only pointer;
        // with Sway 1.11 and Xwayland 24.1 the X client (GTK) then often never sees the click
        // complete (right-button presses land, left clicks do not). Let the last action settle.
        if let Some(at) = self.last_action {
            std::thread::sleep(TEARDOWN_SETTLE.saturating_sub(at.elapsed()));
        }
        for surface in self.state.surfaces.drain(..) {
            surface.layer.destroy();
            surface.wl.destroy();
        }
        if let Injector::Virtual(device) = &self.injector {
            device.pointer.destroy();
            if let Some(keyboard) = &device.keyboard {
                keyboard.destroy();
            }
        }
        // Best effort: the compositor frees everything when the connection closes anyway.
        let _ = self.conn.flush();
    }
}

/// Gives the virtual keyboard the seat's keymap (required before any key event).
fn upload_keymap(
    keyboard: &wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
    text: &str,
) -> Result<(), PlatformError> {
    use std::os::unix::fs::FileExt;
    let fd = rustix::fs::memfd_create("nomouse-keymap", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(lost)?;
    let file = std::fs::File::from(fd);
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    file.write_all_at(&bytes, 0).map_err(lost)?;
    let size = u32::try_from(bytes.len()).map_err(lost)?;
    // Format 1 is XKB v1.
    keyboard.keymap(1, file.as_fd(), size);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_bounding_box() {
        assert_eq!(bounding([]), Rect::default());
        assert_eq!(
            bounding([
                Rect::new(0, 0, 1600, 1000),
                Rect::new(1600, -200, 1920, 1080)
            ]),
            Rect::new(0, -200, 3520, 1200)
        );
        assert_eq!(
            bounding([Rect::new(10, 10, 0, 0), Rect::new(0, 0, 5, 5)]),
            Rect::new(0, 0, 5, 5)
        );
    }
}
