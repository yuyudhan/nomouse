//! Window enumeration, monitors, DPI awareness and process elevation.

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_AWARENESS_PER_MONITOR_AWARE,
    DPI_AWARENESS_SYSTEM_AWARE, DPI_AWARENESS_UNAWARE, GetAwarenessFromDpiAwarenessContext,
    GetDpiForMonitor, GetThreadDpiAwarenessContext, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GW_HWNDNEXT, GWL_EXSTYLE, GetClassNameW, GetForegroundWindow,
    GetSystemMetrics, GetTopWindow, GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextW,
    IsIconic, IsWindowVisible, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, WS_EX_TOOLWINDOW,
};
use windows::core::{HSTRING, PCWSTR};

use crate::error::{Context, last_error};
use crate::types::{Edges, SysError, WindowHandle, WindowInfo};

/// The DPI of a 100 % scale monitor.
pub const BASE_DPI: u32 = 96;

const fn hwnd(handle: WindowHandle) -> HWND {
    HWND(handle.0 as *mut core::ffi::c_void)
}

fn handle(hwnd: HWND) -> WindowHandle {
    WindowHandle(hwnd.0 as isize)
}

/// Opts the process into per-monitor DPI awareness (v2) so UIA rectangles, window rectangles
/// and `SendInput` coordinates all use physical pixels.
///
/// # Errors
/// Fails (`E_ACCESSDENIED`) when the awareness was already set, by a manifest or an earlier
/// call; [`dpi_awareness`] then tells the effective mode.
pub fn enable_per_monitor_dpi() -> Result<(), SysError> {
    // SAFETY: takes a predefined pseudo-handle constant and no pointers.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .context("SetProcessDpiAwarenessContext")
}

/// The calling thread's DPI awareness: `"per-monitor"`, `"system"`, `"unaware"` or `"unknown"`.
pub fn dpi_awareness() -> &'static str {
    // SAFETY: both calls take and return plain values; the context is a pseudo-handle.
    let awareness = unsafe { GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) };
    match awareness {
        DPI_AWARENESS_PER_MONITOR_AWARE => "per-monitor",
        DPI_AWARENESS_SYSTEM_AWARE => "system",
        DPI_AWARENESS_UNAWARE => "unaware",
        _ => "unknown",
    }
}

/// The effective DPI of the monitor nearest `(x, y)`; [`BASE_DPI`] when unknown.
pub fn dpi_at(x: i32, y: i32) -> u32 {
    // SAFETY: MonitorFromPoint takes a POINT by value and never returns null with NEAREST.
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
    // SAFETY: both out-pointers point at live locals for the duration of the call.
    match unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y) } {
        Ok(()) if dpi_x > 0 => dpi_x,
        _ => BASE_DPI,
    }
}

/// The bounds of the virtual desktop (all monitors).
pub fn virtual_screen() -> Edges {
    // SAFETY: GetSystemMetrics takes an index and returns a value.
    let metric = |index| unsafe { GetSystemMetrics(index) };
    let (left, top) = (metric(SM_XVIRTUALSCREEN), metric(SM_YVIRTUALSCREEN));
    Edges::new(
        left,
        top,
        left.saturating_add(metric(SM_CXVIRTUALSCREEN)),
        top.saturating_add(metric(SM_CYVIRTUALSCREEN)),
    )
}

/// The foreground window, if any.
pub fn foreground_window() -> Option<WindowHandle> {
    // SAFETY: no arguments; returns a possibly-null handle.
    let window = unsafe { GetForegroundWindow() };
    (!window.is_invalid()).then(|| handle(window))
}

/// Information about `window`.
///
/// # Errors
/// Fails when the window no longer exists.
pub fn window_info(window: WindowHandle) -> Result<WindowInfo, SysError> {
    let raw = hwnd(window);
    let mut rect = RECT::default();
    // SAFETY: `rect` is a live out-parameter; stale handles fail.
    unsafe { GetWindowRect(raw, &raw mut rect) }.context("GetWindowRect")?;
    let mut class = [0u16; 256];
    // SAFETY: the buffer is live and its length is passed through the slice.
    let class_len = unsafe { GetClassNameW(raw, &mut class) };
    let mut title = [0u16; 512];
    // SAFETY: as above.
    let title_len = unsafe { GetWindowTextW(raw, &mut title) };
    // SAFETY: reads a window long of a (possibly stale) handle; returns 0 on failure.
    let ex_style = unsafe { GetWindowLongPtrW(raw, GWL_EXSTYLE) };
    Ok(WindowInfo {
        handle: window,
        bounds: Edges::new(rect.left, rect.top, rect.right, rect.bottom),
        class_name: utf16(&class, class_len),
        title: utf16(&title, title_len),
        tool_window: ex_style & isize::try_from(WS_EX_TOOLWINDOW.0).unwrap_or(0) != 0,
    })
}

fn utf16(buffer: &[u16], len: i32) -> String {
    let len = usize::try_from(len).unwrap_or(0).min(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

/// Whether the window is visible, not minimised and not cloaked (other virtual desktop,
/// suspended UWP app).
pub fn is_shown(window: WindowHandle) -> bool {
    let raw = hwnd(window);
    // SAFETY: both take a handle by value; stale handles return false.
    let visible = unsafe { IsWindowVisible(raw).as_bool() && !IsIconic(raw).as_bool() };
    if !visible {
        return false;
    }
    let mut cloaked = 0u32;
    // SAFETY: the out-pointer is a live u32 and the size passed matches it.
    let result = unsafe {
        DwmGetWindowAttribute(
            raw,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            u32::try_from(size_of::<u32>()).unwrap_or(4),
        )
    };
    result.is_err() || cloaked == 0
}

/// Every shown top-level window, topmost first (z-order).
pub fn top_level_windows() -> Vec<WindowInfo> {
    let mut windows = Vec::new();
    // SAFETY: null parent means the desktop; returns the topmost child or an error.
    let mut next = unsafe { GetTopWindow(None) }.ok();
    while let Some(current) = next.filter(|window| !window.is_invalid()) {
        let window = handle(current);
        if is_shown(window)
            && let Ok(info) = window_info(window)
        {
            windows.push(info);
        }
        // SAFETY: walks the z-order from a handle we just read; errors end the walk.
        next = unsafe { GetWindow(current, GW_HWNDNEXT) }.ok();
    }
    windows
}

/// The first top-level window of class `class`.
pub fn find_window(class: &str) -> Option<WindowHandle> {
    let class = HSTRING::from(class);
    // SAFETY: `class` is a live NUL-terminated wide string for the duration of the call.
    let window = unsafe { FindWindowW(&class, PCWSTR::null()) }.ok()?;
    (!window.is_invalid()).then(|| handle(window))
}

/// The first descendant window of `parent` with class `class`, searched depth first.
pub fn find_descendant(parent: WindowHandle, class: &str) -> Option<WindowHandle> {
    let wide = HSTRING::from(class);
    // SAFETY: as in `find_window`; a stale parent makes the call fail.
    let direct = unsafe { FindWindowExW(Some(hwnd(parent)), None, &wide, PCWSTR::null()) };
    if let Ok(window) = direct
        && !window.is_invalid()
    {
        return Some(handle(window));
    }
    let mut child = None;
    loop {
        // SAFETY: enumerates children of `parent` after `child`; null class matches any.
        let next =
            unsafe { FindWindowExW(Some(hwnd(parent)), child, PCWSTR::null(), PCWSTR::null()) };
        let Ok(next) = next else { return None };
        if next.is_invalid() {
            return None;
        }
        if let Some(found) = find_descendant(handle(next), class) {
            return Some(found);
        }
        child = Some(next);
    }
}

/// Whether this process runs elevated (UIPI lets it reach elevated windows).
///
/// # Errors
/// Fails when the process token cannot be queried.
pub fn is_elevated() -> Result<bool, SysError> {
    let mut token = HANDLE::default();
    // SAFETY: the pseudo-handle of the current process needs no closing; `token` is a live
    // out-parameter closed below.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) }
        .context("OpenProcessToken")?;
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    let size = u32::try_from(size_of::<TOKEN_ELEVATION>()).unwrap_or(4);
    // SAFETY: the buffer is a live TOKEN_ELEVATION of exactly `size` bytes.
    let result = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&raw mut elevation).cast()),
            size,
            &raw mut returned,
        )
    }
    .context("GetTokenInformation");
    // SAFETY: `token` was opened above and is closed exactly once.
    if unsafe { CloseHandle(token) }.is_err() {
        return Err(last_error("CloseHandle"));
    }
    result.map(|()| elevation.TokenIsElevated != 0)
}
