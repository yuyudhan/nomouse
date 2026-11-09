//! The Wayland client state and its protocol event handlers.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Read;

use kbvm::lookup::LookupTable;
use kbvm::xkb::diagnostic::Diagnostic;
use kbvm::{GroupIndex, Keycode, ModifierMask};
use nomouse_core::{InputEvent, Modifiers, Rect};
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_region, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1,
};

use crate::x11::keys::{key_press, modifiers};

/// One output (monitor) and what is known of its logical geometry.
#[derive(Debug)]
pub(crate) struct Output {
    /// The registry name of the `wl_output` global.
    pub global: u32,
    /// The bound output.
    pub wl: wl_output::WlOutput,
    /// Its `xdg_output`, when the compositor offers the manager.
    pub xdg: Option<zxdg_output_v1::ZxdgOutputV1>,
    /// The connector name (`HEADLESS-1`, `DP-1`, ...).
    pub name: String,
    /// Position from `wl_output.geometry`.
    pub position: (i32, i32),
    /// Current mode size in pixels.
    pub mode: (i32, i32),
    /// Integer scale factor.
    pub scale: i32,
    /// Logical position from `xdg_output`.
    pub logical_position: Option<(i32, i32)>,
    /// Logical size from `xdg_output`.
    pub logical_size: Option<(i32, i32)>,
}

impl Output {
    /// The output's area in layout coordinates (the space AT-SPI's `XWayland` coordinates and the
    /// compositor's window positions use).
    pub(crate) fn area(&self) -> Rect {
        let (x, y) = self.logical_position.unwrap_or(self.position);
        let (width, height) = self.logical_size.unwrap_or_else(|| {
            let scale = self.scale.max(1);
            (self.mode.0 / scale, self.mode.1 / scale)
        });
        Rect::new(
            x,
            y,
            u32::try_from(width).unwrap_or(0),
            u32::try_from(height).unwrap_or(0),
        )
    }
}

/// One overlay surface on one output.
#[derive(Debug)]
pub(crate) struct Surface {
    /// The output's registry name.
    pub output: u32,
    /// The surface.
    pub wl: wl_surface::WlSurface,
    /// Its layer-shell role.
    pub layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    /// The size from the last `configure`, once acknowledged.
    pub size: Option<(u32, u32)>,
    /// The compositor closed the surface (output gone).
    pub closed: bool,
}

/// The layout of the keyboard the compositor sent, for reading keys and for the virtual
/// keyboard's keymap.
pub(crate) struct Keyboard {
    /// The keymap source (XKB text format v1).
    pub text: String,
    table: LookupTable,
    mods: ModifierMask,
    /// Depressed and latched modifiers (locks excluded), in the core X mask layout: the real
    /// modifiers come first in every keymap xkbcommon writes.
    held: u32,
    group: GroupIndex,
}

/// Everything the event handlers update.
pub(crate) struct State {
    pub compositor: wl_compositor::WlCompositor,
    pub shm: wl_shm::WlShm,
    pub layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    pub xdg_outputs: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    pub pointer_manager: Option<zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1>,
    pub keyboard_manager: Option<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1>,
    pub seat: Option<wl_seat::WlSeat>,
    pub keyboard: Option<wl_keyboard::WlKeyboard>,
    pub layout: Option<Keyboard>,
    pub outputs: Vec<Output>,
    pub surfaces: Vec<Surface>,
    /// An overlay surface has keyboard focus.
    pub focused: bool,
    /// Key presses read while focused, oldest first.
    pub pending: VecDeque<InputEvent>,
}

impl State {
    /// Binds a newly announced `wl_output` (and its `xdg_output`).
    pub(crate) fn add_output(
        &mut self,
        registry: &wl_registry::WlRegistry,
        global: u32,
        version: u32,
        qh: &QueueHandle<Self>,
    ) {
        let wl: wl_output::WlOutput = registry.bind(global, version.min(4), qh, global);
        let xdg = self
            .xdg_outputs
            .as_ref()
            .map(|manager| manager.get_xdg_output(&wl, qh, global));
        self.outputs.push(Output {
            global,
            wl,
            xdg,
            name: String::new(),
            position: (0, 0),
            mode: (0, 0),
            scale: 1,
            logical_position: None,
            logical_size: None,
        });
    }

    fn output_mut(&mut self, global: u32) -> Option<&mut Output> {
        self.outputs.iter_mut().find(|o| o.global == global)
    }

    fn read_keymap(&mut self, fd: std::os::fd::OwnedFd, size: u32) {
        let mut text = String::new();
        let mut file = File::from(fd);
        let limit = u64::from(size);
        if let Err(error) = file.by_ref().take(limit).read_to_string(&mut text) {
            tracing::warn!("reading the keymap: {error}");
            return;
        }
        let text = text.trim_end_matches('\0').to_owned();
        match Keyboard::compile(text) {
            Ok(keyboard) => self.layout = Some(keyboard),
            Err(error) => tracing::warn!("the compositor's keymap does not compile: {error}"),
        }
    }
}

impl Keyboard {
    /// Compiles an XKB v1 keymap (no include paths: compositors send complete keymaps).
    pub(crate) fn compile(text: String) -> Result<Self, String> {
        let mut builder = kbvm::xkb::Context::builder();
        builder.enable_default_includes(false);
        let context = builder.build();
        let keymap = context
            .keymap_from_bytes(Vec::<Diagnostic>::new(), None, text.as_bytes())
            .map_err(|e| e.to_string())?;
        Ok(Self {
            table: keymap.to_builder().build_lookup_table(),
            text,
            mods: ModifierMask(0),
            held: 0,
            group: GroupIndex(0),
        })
    }

    /// Applies a `wl_keyboard.modifiers` event.
    pub(crate) const fn set_modifiers(
        &mut self,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) {
        self.mods = ModifierMask(depressed | latched | locked);
        self.held = depressed | latched;
        self.group = GroupIndex(group);
    }

    /// The modifiers held now.
    pub(crate) fn modifiers(&self) -> Modifiers {
        modifiers(u16::try_from(self.held & 0xff).unwrap_or(0))
    }

    /// The keysym of evdev key `key` with the current modifiers and group.
    pub(crate) fn keysym(&self, key: u32) -> Option<u32> {
        self.table
            .lookup(self.group, self.mods, Keycode::from_evdev(key))
            .into_iter()
            .next()
            .map(|props| props.keysym().0)
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == wl_output::WlOutput::interface().name => {
                state.add_output(registry, name, version, qh);
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(index) = state.outputs.iter().position(|o| o.global == name) {
                    let output = state.outputs.remove(index);
                    if let Some(xdg) = output.xdg {
                        xdg.destroy();
                    }
                    if output.wl.version() >= 3 {
                        output.wl.release();
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.output_mut(*global) else {
            return;
        };
        match event {
            wl_output::Event::Geometry { x, y, .. } => output.position = (x, y),
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => output.mode = (width, height),
            wl_output::Event::Scale { factor } => output.scale = factor,
            wl_output::Event::Name { name } => output.name = name,
            _ => {}
        }
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, u32> for State {
    fn event(
        state: &mut Self,
        _: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.output_mut(*global) else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                output.logical_position = Some((x, y));
            }
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                output.logical_size = Some((width, height));
            }
            zxdg_output_v1::Event::Name { name } if output.name.is_empty() => output.name = name,
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        (): &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            let has_keyboard = capabilities.contains(wl_seat::Capability::Keyboard);
            match (has_keyboard, state.keyboard.take()) {
                (true, None) => state.keyboard = Some(seat.get_keyboard(qh, ())),
                (true, Some(keyboard)) => state.keyboard = Some(keyboard),
                (false, Some(keyboard)) => {
                    if keyboard.version() >= 3 {
                        keyboard.release();
                    }
                }
                (false, None) => {}
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                fd,
                size,
            } => state.read_keymap(fd, size),
            wl_keyboard::Event::Enter { surface, .. } => {
                state.focused = state.surfaces.iter().any(|s| s.wl == surface);
                tracing::debug!("keyboard focus entered (overlay: {})", state.focused);
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                if state.surfaces.iter().any(|s| s.wl == surface) {
                    state.focused = false;
                    tracing::debug!("keyboard focus left the overlay");
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(layout) = &mut state.layout {
                    layout.set_modifiers(mods_depressed, mods_latched, mods_locked, group);
                }
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(wl_keyboard::KeyState::Pressed),
                ..
            } => {
                let Some(keysym) = state.layout.as_ref().and_then(|layout| layout.keysym(key))
                else {
                    return;
                };
                let press = key_press(keysym);
                tracing::debug!(
                    "key {key} = keysym {keysym:#x} {press:?} (overlay focused: {})",
                    state.focused
                );
                let held = state
                    .layout
                    .as_ref()
                    .map_or(Modifiers::NONE, Keyboard::modifiers);
                if let Some(press) = press.filter(|_| state.focused) {
                    state.pending.push_back(InputEvent::Key(press.with(held)));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(surface) = state.surfaces.iter_mut().find(|s| &s.layer == layer) else {
            return;
        };
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer.ack_configure(serial);
                surface.size = Some((width, height));
            }
            zwlr_layer_surface_v1::Event::Closed => surface.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(
        _: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Every frame gets a fresh buffer; it is freed once the compositor lets go of it.
        if let wl_buffer::Event::Release = event {
            buffer.destroy();
        }
    }
}

delegate_noop!(State: wl_compositor::WlCompositor);
delegate_noop!(State: wl_region::WlRegion);
delegate_noop!(State: wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(State: zxdg_output_manager_v1::ZxdgOutputManagerV1);
delegate_noop!(State: zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
delegate_noop!(State: zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);
delegate_noop!(State: zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::*;

    const KEYMAP: &str = r#"xkb_keymap {
    xkb_keycodes {
        <a> = 38;
        <esc> = 9;
        <lfsh> = 50;
    };
    xkb_types {
        type "ALPHABETIC" {
            modifiers = Shift+Lock;
            map[Shift] = Level2;
            map[Lock] = Level2;
        };
        type "ONE_LEVEL" {
            modifiers = None;
            map[None] = Level1;
        };
    };
    xkb_compat {
        interpret VoidSymbol {
            repeat = false;
        };
    };
    xkb_symbols {
        key <a> { type[Group1] = "ALPHABETIC", symbols[Group1] = [ a, A ] };
        key <esc> { type[Group1] = "ONE_LEVEL", symbols[Group1] = [ Escape ] };
        key <lfsh> {
            type[Group1] = "ONE_LEVEL",
            actions[Group1] = [ SetMods(modifiers = Shift) ]
        };
    };
};
"#;

    #[test]
    fn keymap_lookup_follows_modifiers() {
        let mut keyboard = Keyboard::compile(KEYMAP.to_owned()).unwrap();
        // evdev 30 is X keycode 38.
        assert_eq!(keyboard.keysym(30), Some(0x61));
        assert_eq!(keyboard.keysym(1), Some(0xff1b));
        keyboard.set_modifiers(1, 0, 0, 0);
        assert_eq!(keyboard.keysym(30), Some(0x41));
        assert_eq!(keyboard.keysym(200), None);
        assert!(keyboard.text.starts_with("xkb_keymap"));
        assert!(Keyboard::compile("not a keymap".to_owned()).is_err());
    }
}
