//! Global hotkeys (`RegisterHotKey`), the session keyboard grab (a `WH_KEYBOARD_LL` hook) and
//! the thread's message pump that delivers both.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    RegisterHotKey, UnregisterHotKey, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
    VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, MSG, MWMO_INPUTAVAILABLE,
    MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, QS_ALLINPUT, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_HOTKEY, WM_KEYDOWN, WM_SYSKEYDOWN,
};
use windows::core::PCWSTR;

use crate::error::Context;
use crate::types::{HotkeyModifiers, SysError, SysEvent};

thread_local! {
    /// Events produced on this thread by the hook and the pump, oldest first.
    static QUEUE: RefCell<VecDeque<SysEvent>> = const { RefCell::new(VecDeque::new()) };
    /// Whether the hook on this thread swallows keys.
    static GRABBING: Cell<bool> = const { Cell::new(false) };
}

/// Modifier keys pass through the grab so the system's modifier state stays consistent.
const MODIFIER_KEYS: [u16; 11] = [
    VK_SHIFT.0,
    VK_LSHIFT.0,
    VK_RSHIFT.0,
    VK_CONTROL.0,
    VK_LCONTROL.0,
    VK_RCONTROL.0,
    VK_MENU.0,
    VK_LMENU.0,
    VK_RMENU.0,
    VK_LWIN.0,
    VK_RWIN.0,
];

/// The low-level keyboard hook: while grabbing, queues key-downs and swallows non-modifier keys.
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == i32::try_from(HC_ACTION).unwrap_or(0) && GRABBING.with(Cell::get) {
        // SAFETY: for WH_KEYBOARD_LL with HC_ACTION, lparam points at a KBDLLHOOKSTRUCT that is
        // valid for the duration of this call.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let modifier = u16::try_from(info.vkCode).is_ok_and(|vk| MODIFIER_KEYS.contains(&vk));
        if !modifier {
            let message = u32::try_from(wparam.0).unwrap_or(0);
            if message == WM_KEYDOWN || message == WM_SYSKEYDOWN {
                // SAFETY: GetAsyncKeyState takes a key code and returns its state.
                let shift = unsafe { GetAsyncKeyState(i32::from(VK_SHIFT.0)) } < 0;
                QUEUE.with_borrow_mut(|queue| {
                    queue.push_back(SysEvent::Key {
                        vk: info.vkCode,
                        shift,
                    });
                });
            }
            // A non-zero result swallows the key.
            return LRESULT(1);
        }
    }
    // SAFETY: forwards the unmodified arguments to the next hook, as the hook contract requires.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// The current thread's hotkeys, keyboard grab and message pump. Not `Send`: hotkeys and hooks
/// deliver to the thread that installed them.
#[derive(Debug)]
pub struct EventPump {
    hotkeys: Vec<i32>,
    hook: Option<HHOOK>,
    _thread_bound: PhantomData<*const ()>,
}

impl EventPump {
    /// Creates a pump for the current thread.
    pub fn new() -> Self {
        Self {
            hotkeys: Vec::new(),
            hook: None,
            _thread_bound: PhantomData,
        }
    }

    /// Registers a global hotkey with id `id` (auto-repeat suppressed).
    ///
    /// # Errors
    /// [`crate::SysErrorKind::HotkeyTaken`] when another program owns the chord.
    pub fn register_hotkey(
        &mut self,
        id: i32,
        modifiers: HotkeyModifiers,
        vk: u32,
    ) -> Result<(), SysError> {
        let mut flags = MOD_NOREPEAT;
        for (on, flag) in [
            (modifiers.ctrl, MOD_CONTROL),
            (modifiers.alt, MOD_ALT),
            (modifiers.shift, MOD_SHIFT),
            (modifiers.win, MOD_WIN),
        ] {
            if on {
                flags |= flag;
            }
        }
        // SAFETY: a null window posts WM_HOTKEY to this thread's queue; no pointers involved.
        unsafe { RegisterHotKey(None, id, HOT_KEY_MODIFIERS(flags.0), vk) }
            .context("RegisterHotKey")?;
        self.hotkeys.push(id);
        Ok(())
    }

    /// Unregisters hotkey `id`; unknown ids are a no-op.
    ///
    /// # Errors
    /// Fails when the OS rejects the call.
    pub fn unregister_hotkey(&mut self, id: i32) -> Result<(), SysError> {
        let Some(position) = self.hotkeys.iter().position(|&known| known == id) else {
            return Ok(());
        };
        self.hotkeys.swap_remove(position);
        // SAFETY: unregisters an id this thread registered with a null window.
        unsafe { UnregisterHotKey(None, id) }.context("UnregisterHotKey")
    }

    /// Starts (`true`) or stops (`false`) swallowing the keyboard; while grabbing, key-downs
    /// arrive as [`SysEvent::Key`].
    ///
    /// # Errors
    /// Fails when the hook cannot be installed.
    pub fn grab_keyboard(&mut self, grab: bool) -> Result<(), SysError> {
        if grab && self.hook.is_none() {
            // SAFETY: a null module name returns this executable's handle.
            let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.context("GetModuleHandleW")?;
            // SAFETY: `keyboard_hook` matches HOOKPROC and lives for the whole program; a
            // low-level hook runs on this thread while it pumps messages.
            let hook = unsafe {
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(module.into()), 0)
            }
            .context("SetWindowsHookExW")?;
            self.hook = Some(hook);
        }
        GRABBING.with(|flag| flag.set(grab));
        if !grab {
            QUEUE.with_borrow_mut(|queue| {
                queue.retain(|event| !matches!(event, SysEvent::Key { .. }));
            });
            if let Some(hook) = self.hook.take() {
                // SAFETY: `hook` was installed by this thread and is removed exactly once.
                unsafe { UnhookWindowsHookEx(hook) }.context("UnhookWindowsHookEx")?;
            }
        }
        Ok(())
    }

    /// Pumps this thread's messages (which also runs the keyboard hook and the overlay's
    /// window procedure) until an event arrives or `timeout` passes.
    pub fn next_event(&mut self, timeout: Duration) -> Option<SysEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            pump_messages();
            if let Some(event) = QUEUE.with_borrow_mut(VecDeque::pop_front) {
                return Some(event);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let millis = u32::try_from(remaining.as_millis())
                .unwrap_or(u32::MAX)
                .max(1);
            // SAFETY: no handles are passed; the call only waits for queue input or the timeout.
            unsafe {
                MsgWaitForMultipleObjectsEx(None, millis, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            }
        }
    }
}

impl Default for EventPump {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for EventPump {
    fn drop(&mut self) {
        // Teardown errors cannot be reported from Drop; the OS frees both at thread exit anyway.
        let _ = self.grab_keyboard(false);
        for id in std::mem::take(&mut self.hotkeys) {
            // SAFETY: unregisters ids this thread registered.
            let _ = unsafe { UnregisterHotKey(None, id) };
        }
    }
}

fn pump_messages() {
    let mut message = MSG::default();
    // SAFETY: `message` is a live out-parameter; a null window reads every message of the thread.
    while unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        if message.message == WM_HOTKEY && message.hwnd.is_invalid() {
            let id = i32::try_from(message.wParam.0).unwrap_or(-1);
            QUEUE.with_borrow_mut(|queue| queue.push_back(SysEvent::Hotkey(id)));
            continue;
        }
        // SAFETY: `message` was just filled by PeekMessageW.
        unsafe {
            let _ = TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
    }
}
