//! Backend tests against a fake [`System`].

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use nomouse_core::{Element, Rect, Role, assign_hints};
use nomouse_sys_windows::{Button, Patterns, control_type as ct};

use super::*;
use crate::render::{find_font, system_font_dir};

#[derive(Debug, Default)]
struct Fake {
    foreground: Option<WindowHandle>,
    windows: Vec<WindowInfo>,
    trees: HashMap<isize, Result<Vec<RawElement>, SysError>>,
    tray: Option<WindowInfo>,
    activated: Vec<(usize, Activation)>,
    activate_fails: bool,
    sent: Vec<Vec<InputStep>>,
    send_fails: bool,
    bitmaps: Vec<Bitmap>,
    grabbed: bool,
    hotkeys: Vec<(i32, HotkeyModifiers, u32)>,
    taken: Vec<u32>,
    events: VecDeque<SysEvent>,
    snapshots: usize,
}

impl System for Fake {
    fn foreground_window(&mut self) -> Option<WindowHandle> {
        self.foreground
    }
    fn top_level_windows(&mut self) -> Vec<WindowInfo> {
        self.windows.clone()
    }
    fn find_window(&mut self, class: &str) -> Option<WindowHandle> {
        self.windows
            .iter()
            .find(|w| w.class_name == class)
            .map(|w| w.handle)
    }
    fn find_descendant(&mut self, _parent: WindowHandle, class: &str) -> Option<WindowHandle> {
        self.tray
            .as_ref()
            .filter(|tray| tray.class_name == class)
            .map(|tray| tray.handle)
    }
    fn window_info(&mut self, window: WindowHandle) -> Result<WindowInfo, SysError> {
        self.tray
            .iter()
            .chain(&self.windows)
            .find(|w| w.handle == window)
            .cloned()
            .ok_or_else(|| SysError::other("window_info", "gone"))
    }
    fn begin_snapshot(&mut self) {
        self.snapshots += 1;
    }
    fn snapshot(&mut self, window: WindowHandle) -> Result<Vec<RawElement>, SysError> {
        self.trees
            .get(&window.0)
            .cloned()
            .unwrap_or_else(|| Ok(Vec::new()))
    }
    fn activate(&mut self, index: usize, activation: Activation) -> Result<(), SysError> {
        if self.activate_fails {
            return Err(SysError::other("activate", "no"));
        }
        self.activated.push((index, activation));
        Ok(())
    }
    fn virtual_screen(&mut self) -> Edges {
        Edges::new(0, 0, 1000, 800)
    }
    fn dpi_at(&mut self, x: i32, _y: i32) -> u32 {
        if x < 500 { 96 } else { 192 }
    }
    fn send_input(&mut self, steps: &[InputStep]) -> Result<(), SysError> {
        if self.send_fails {
            return Err(SysError {
                kind: SysErrorKind::AccessDenied,
                call: "SendInput",
                code: 0,
                message: "blocked".to_owned(),
            });
        }
        self.sent.push(steps.to_vec());
        Ok(())
    }
    fn overlay_update(&mut self, bitmap: &Bitmap) -> Result<(), SysError> {
        self.bitmaps.push(bitmap.clone());
        Ok(())
    }
    fn overlay_hide(&mut self) {
        self.bitmaps.push(Bitmap::default());
    }
    fn overlay_handle(&self) -> Option<WindowHandle> {
        Some(WindowHandle(99))
    }
    fn grab_keyboard(&mut self, grab: bool) -> Result<(), SysError> {
        self.grabbed = grab;
        Ok(())
    }
    fn register_hotkey(
        &mut self,
        id: i32,
        modifiers: HotkeyModifiers,
        vk: u32,
    ) -> Result<(), SysError> {
        if self.taken.contains(&vk) {
            return Err(SysError {
                kind: SysErrorKind::HotkeyTaken,
                call: "RegisterHotKey",
                code: 0,
                message: "taken".to_owned(),
            });
        }
        self.hotkeys.push((id, modifiers, vk));
        Ok(())
    }
    fn unregister_hotkey(&mut self, id: i32) -> Result<(), SysError> {
        self.hotkeys.retain(|(known, _, _)| *known != id);
        Ok(())
    }
    fn next_event(&mut self, _timeout: Duration) -> Option<SysEvent> {
        self.events.pop_front()
    }
}

fn font() -> Option<PathBuf> {
    find_font(&[
        system_font_dir(),
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
    ])
}

fn backend_with(fake: Fake, config: &Config) -> Option<Backend<Fake>> {
    Some(Backend::with_font(fake, &font()?, config, ScopeOptions::default()).unwrap())
}

fn make(fake: Fake) -> Option<Backend<Fake>> {
    backend_with(fake, &Config::default())
}

fn window(id: isize, class: &str, bounds: Edges) -> WindowInfo {
    WindowInfo {
        handle: WindowHandle(id),
        bounds,
        class_name: class.to_owned(),
        title: String::new(),
        tool_window: false,
    }
}

fn element(index: usize, control_type: i32, bounds: Edges) -> RawElement {
    RawElement {
        index,
        control_type,
        name: format!("e{index}"),
        bounds,
        offscreen: false,
        enabled: true,
        keyboard_focusable: false,
        patterns: Patterns::default(),
    }
}

fn app() -> Fake {
    let mut invoke = element(0, ct::BUTTON, Edges::new(10, 10, 50, 30));
    invoke.patterns.invoke = true;
    let mut fake = Fake {
        foreground: Some(WindowHandle(1)),
        windows: vec![
            window(99, "nomouse-overlay", Edges::new(0, 0, 1000, 800)),
            window(1, "App", Edges::new(0, 0, 600, 400)),
            window(2, "Other", Edges::new(0, 0, 1000, 800)),
        ],
        ..Fake::default()
    };
    fake.trees.insert(
        1,
        Ok(vec![
            invoke,
            element(1, ct::EDIT, Edges::new(10, 50, 300, 70)),
            element(2, ct::PANE, Edges::new(0, 0, 600, 400)),
            element(3, ct::BUTTON, Edges::new(10, 10, 10, 30)),
        ]),
    );
    fake.trees.insert(
        2,
        Ok(vec![
            element(4, ct::HYPERLINK, Edges::new(100, 100, 120, 110)),
            element(5, ct::HYPERLINK, Edges::new(700, 500, 720, 510)),
        ]),
    );
    fake
}

#[test]
fn focused_window_elements_are_filtered_and_converted() {
    let Some(mut backend) = make(app()) else {
        return;
    };
    let elements = backend.elements(Scope::FocusedWindow).unwrap();
    let ids: Vec<u64> = elements.iter().map(|e| e.id.0).collect();
    assert_eq!(ids, vec![0, 1]);
    assert_eq!(elements[0].role, Role::Button);
    assert_eq!(elements[1].role, Role::TextField);
    assert_eq!(elements[0].bounds, Rect::new(10, 10, 40, 20));
}

#[test]
fn no_focus_and_errors() {
    let mut fake = app();
    fake.foreground = None;
    let Some(mut backend) = make(fake) else {
        return;
    };
    assert_eq!(
        backend.elements(Scope::FocusedWindow),
        Err(PlatformError::NoFocusedWindow)
    );

    let mut fake = app();
    fake.trees.insert(
        1,
        Err(SysError {
            kind: SysErrorKind::AccessDenied,
            call: "ElementFromHandle",
            code: 5,
            message: "denied".to_owned(),
        }),
    );
    let Some(mut backend) = make(fake) else {
        return;
    };
    assert!(matches!(
        backend.elements(Scope::FocusedWindow),
        Err(PlatformError::PermissionDenied(_))
    ));
    // The same failure only skips that window in screen scope.
    let elements = backend.elements(Scope::Screen).unwrap();
    assert_eq!(elements.iter().map(|e| e.id.0).collect::<Vec<_>>(), vec![5]);
}

#[test]
fn screen_scope_skips_occluded_elements() {
    let Some(mut backend) = make(app()) else {
        return;
    };
    let elements = backend.elements(Scope::Screen).unwrap();
    let ids: Vec<u64> = elements.iter().map(|e| e.id.0).collect();
    // Link 4 sits under window 1; link 5 is visible.
    assert_eq!(ids, vec![0, 1, 5]);
}

#[test]
fn tray_is_excluded_on_request() {
    let mut fake = Fake {
        windows: vec![window(7, TASKBAR_CLASS, Edges::new(0, 760, 1000, 800))],
        tray: Some(window(8, TRAY_CLASS, Edges::new(800, 760, 1000, 800))),
        ..Fake::default()
    };
    fake.trees.insert(
        7,
        Ok(vec![
            element(0, ct::BUTTON, Edges::new(10, 765, 40, 795)),
            element(1, ct::BUTTON, Edges::new(900, 765, 930, 795)),
        ]),
    );
    let Some(mut backend) = make(fake) else {
        return;
    };
    assert_eq!(backend.elements(Scope::Screen).unwrap().len(), 2);
    backend.set_scope_options(ScopeOptions {
        include_taskbar: true,
        include_tray: false,
    });
    assert_eq!(backend.elements(Scope::Screen).unwrap().len(), 1);
    backend.set_scope_options(ScopeOptions {
        include_taskbar: false,
        include_tray: false,
    });
    assert_eq!(
        backend.elements(Scope::Screen).unwrap(),
        [] as [nomouse_core::Element; 0]
    );
}

#[test]
fn semantic_click_uses_patterns_when_enabled() {
    let mut config = Config::default();
    config.hints.semantic_click = true;
    let Some(mut backend) = backend_with(app(), &config) else {
        return;
    };
    let elements = backend.elements(Scope::FocusedWindow).unwrap();
    backend
        .perform(&Action::click(elements[0].target()))
        .unwrap();
    assert_eq!(backend.system.activated, vec![(0, Activation::Invoke)]);
    assert_eq!(
        backend.system.sent,
        [] as [std::vec::Vec<nomouse_sys_windows::InputStep>; 0]
    );
    // Text fields get a pointer click; so do right clicks.
    backend
        .perform(&Action::click(elements[1].target()))
        .unwrap();
    backend
        .perform(&Action::right_click(elements[0].target()))
        .unwrap();
    assert_eq!(backend.system.sent.len(), 2);
    // A failing pattern falls back to the pointer.
    backend.system.activate_fails = true;
    backend
        .perform(&Action::click(elements[0].target()))
        .unwrap();
    assert_eq!(backend.system.sent.len(), 3);
}

#[test]
fn pointer_click_by_default() {
    let Some(mut backend) = make(app()) else {
        return;
    };
    let elements = backend.elements(Scope::FocusedWindow).unwrap();
    backend
        .perform(&Action::click(elements[0].target()))
        .unwrap();
    assert_eq!(
        backend.system.activated,
        [] as [(usize, nomouse_sys_windows::Activation); 0]
    );
    assert_eq!(backend.system.sent.len(), 1);
    assert_eq!(
        backend.system.sent[0][1],
        InputStep::Button {
            button: Button::Left,
            down: true
        }
    );
}

#[test]
fn scroll_reads_step_and_blocked_input_is_permission_denied() {
    let mut config = Config::default();
    config.scroll.step = 2;
    let Some(mut backend) = backend_with(app(), &config) else {
        return;
    };
    backend
        .perform(&Action::scroll(Point::new(5, 5), 0, 1))
        .unwrap();
    assert_eq!(backend.system.sent[0][1], InputStep::Wheel(-240));
    backend.system.send_fails = true;
    assert!(matches!(
        backend.perform(&Action::hover(Point::new(1, 1))),
        Err(PlatformError::PermissionDenied(_))
    ));
}

#[test]
fn drag_is_three_batches_and_modifier_click_holds_keys() {
    let Some(mut backend) = make(app()) else {
        return;
    };
    backend
        .perform(&Action::drag(Point::new(1, 1), Point::new(50, 50)))
        .unwrap();
    assert_eq!(backend.system.sent.len(), 3);
    let modifiers = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    backend
        .modifier_click(Point::new(5, 5), MouseButton::Left, modifiers)
        .unwrap();
    let last = backend.system.sent.last().unwrap();
    assert_eq!(
        last[0],
        InputStep::Key {
            vk: 0x11,
            down: true
        }
    );
}

#[test]
fn overlay_grabs_keyboard_and_renders() {
    let Some(mut backend) = make(app()) else {
        return;
    };
    let elements: Vec<Element> = backend.elements(Scope::FocusedWindow).unwrap();
    let hints = assign_hints(&nomouse_core::Alphabet::default(), elements);
    backend.show(&hints, "").unwrap();
    assert!(backend.system.grabbed);
    let bitmap = backend.system.bitmaps.last().unwrap().clone();
    assert!(bitmap.width > 0 && bitmap.height > 0);
    assert_eq!(
        bitmap.pixels.len(),
        usize::try_from(bitmap.width * bitmap.height).unwrap()
    );
    backend.show(&hints[..1], "a").unwrap();
    backend.hide().unwrap();
    assert!(!backend.system.grabbed);
    assert_eq!(backend.system.bitmaps.last(), Some(&Bitmap::default()));
    backend.hide().unwrap();
}

#[test]
fn hotkeys_register_and_deliver() {
    let mut fake = app();
    fake.taken.push(0x20);
    let Some(mut backend) = make(fake) else {
        return;
    };
    let chord: Hotkey = "ctrl+alt+space".parse().unwrap();
    assert_eq!(
        backend.register(chord),
        Err(PlatformError::HotkeyTaken(chord))
    );
    let id = backend.register("ctrl+f1".parse().unwrap()).unwrap();
    assert_eq!(backend.system.hotkeys.len(), 1);
    assert_eq!(backend.system.hotkeys[0].2, 0x70);

    backend.system.events.extend([
        SysEvent::Hotkey(77),
        SysEvent::Key {
            vk: 0x41,
            shift: false,
        },
        SysEvent::Hotkey(i32::try_from(id.0).unwrap()),
    ]);
    // Unknown hotkeys and keys outside a session are dropped.
    assert_eq!(
        backend.next_event(Duration::from_millis(10)).unwrap(),
        Some(InputEvent::Hotkey(id))
    );
    backend.show(&[], "").unwrap();
    backend.system.events.push_back(SysEvent::Key {
        vk: 0x53,
        shift: true,
    });
    assert_eq!(
        backend.next_event(Duration::ZERO).unwrap(),
        Some(InputEvent::Key(nomouse_core::KeyPress::Char('s')))
    );
    assert_eq!(backend.next_event(Duration::ZERO).unwrap(), None);

    backend.unregister(id).unwrap();
    backend.unregister(id).unwrap();
    assert_eq!(
        backend.system.hotkeys,
        [] as [(i32, nomouse_sys_windows::HotkeyModifiers, u32); 0]
    );
    assert_eq!(backend.name(), "windows-uia");
}
