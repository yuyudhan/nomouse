//! The click-through, topmost, layered overlay window drawn with `UpdateLayeredWindow`.

use std::marker::PhantomData;

use windows::Win32::Foundation::{
    COLORREF, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HDC,
    ReleaseDC, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HTTRANSPARENT, HWND_TOPMOST, RegisterClassExW,
    SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow,
    ULW_ALPHA, UpdateLayeredWindow, WM_MOUSEACTIVATE, WM_NCHITTEST, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::error::{Context, last_error};
use crate::types::{Bitmap, SysError, WindowHandle};

const CLASS_NAME: PCWSTR = w!("nomouse-overlay");
/// `MA_NOACTIVATE`.
const MA_NOACTIVATE: isize = 3;

unsafe extern "system" fn overlay_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // Clicks fall through to the windows below; the overlay never takes focus.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE),
        // SAFETY: forwards the unmodified message of our own window to the default procedure.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// A hidden-until-updated overlay window owned by the current thread.
#[derive(Debug)]
pub struct OverlayWindow {
    window: HWND,
    visible: bool,
    _thread_bound: PhantomData<*const ()>,
}

impl OverlayWindow {
    /// Registers the window class (once per process) and creates the hidden overlay with
    /// `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`.
    ///
    /// # Errors
    /// Fails when the class or window cannot be created.
    pub fn new() -> Result<Self, SysError> {
        // SAFETY: a null module name returns this executable's handle.
        let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.context("GetModuleHandleW")?;
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or(0),
            lpfnWndProc: Some(overlay_proc),
            hInstance: module.into(),
            lpszClassName: CLASS_NAME,
            ..WNDCLASSEXW::default()
        };
        // SAFETY: `class` is fully initialised and its strings are static.
        if unsafe { RegisterClassExW(&raw const class) } == 0 {
            let error = last_error("RegisterClassExW");
            if error.code != ERROR_CLASS_ALREADY_EXISTS.to_hresult().0 {
                return Err(error);
            }
        }
        // SAFETY: the class is registered above; all strings are static and no create
        // parameter is passed.
        let window = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!("nomouse"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(module.into()),
                None,
            )
        }
        .context("CreateWindowExW")?;
        Ok(Self {
            window,
            visible: false,
            _thread_bound: PhantomData,
        })
    }

    /// The overlay's own handle, so window listings can skip it.
    pub fn handle(&self) -> WindowHandle {
        WindowHandle(self.window.0 as isize)
    }

    /// Replaces the overlay's contents with `bitmap`, moves it there and shows it without
    /// activating it. An empty bitmap hides the overlay.
    ///
    /// # Errors
    /// Fails when the pixel buffer size does not match or a GDI call fails.
    pub fn update(&mut self, bitmap: &Bitmap) -> Result<(), SysError> {
        let pixel_count = usize::try_from(u64::from(bitmap.width) * u64::from(bitmap.height))
            .unwrap_or(usize::MAX);
        if bitmap.pixels.len() != pixel_count {
            return Err(SysError::other(
                "OverlayWindow::update",
                format!(
                    "{} pixels for a {}x{} bitmap",
                    bitmap.pixels.len(),
                    bitmap.width,
                    bitmap.height
                ),
            ));
        }
        if pixel_count == 0 {
            self.hide();
            return Ok(());
        }
        let width = i32::try_from(bitmap.width).map_err(|_| too_large())?;
        let height = i32::try_from(bitmap.height).map_err(|_| too_large())?;
        // SAFETY: a null window returns the screen DC, released at the end of `draw`.
        let screen = unsafe { GetDC(None) };
        if screen.is_invalid() {
            return Err(last_error("GetDC"));
        }
        let result = self.draw(screen, bitmap, width, height);
        // SAFETY: releases the DC obtained above exactly once.
        unsafe { ReleaseDC(None, screen) };
        result?;
        if !self.visible {
            // SAFETY: shows our own window without activating it; the return value is the
            // previous visibility, not an error.
            let _ = unsafe { ShowWindow(self.window, SW_SHOWNOACTIVATE) };
            self.visible = true;
        }
        // SAFETY: re-asserts topmost order for our own window without moving or activating it.
        unsafe {
            SetWindowPos(
                self.window,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        }
        .context("SetWindowPos")
    }

    fn draw(&self, screen: HDC, bitmap: &Bitmap, width: i32, height: i32) -> Result<(), SysError> {
        // SAFETY: creates a memory DC compatible with the live screen DC; deleted below.
        let memory = unsafe { CreateCompatibleDC(Some(screen)) };
        if memory.is_invalid() {
            return Err(last_error("CreateCompatibleDC"));
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).unwrap_or(0),
                biWidth: width,
                // Negative height: top-down rows, matching `Bitmap::pixels`.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
        // SAFETY: `info` describes a 32-bit top-down DIB; `bits` receives a pointer to its
        // width*height*4 bytes, owned by the returned bitmap.
        let dib = unsafe {
            CreateDIBSection(
                Some(memory),
                &raw const info,
                DIB_RGB_COLORS,
                &raw mut bits,
                None,
                0,
            )
        };
        let result = dib.context("CreateDIBSection").and_then(|dib| {
            if !bits.is_null() {
                // SAFETY: the DIB holds exactly `pixels.len()` u32 values (checked by the caller)
                // in 0xAARRGGBB little-endian order, the same layout as `pixels`; the regions do
                // not overlap.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        bitmap.pixels.as_ptr(),
                        bits.cast::<u32>(),
                        bitmap.pixels.len(),
                    );
                }
            }
            // SAFETY: selects our live DIB into our live memory DC; the previous object is
            // restored before deletion.
            let previous = unsafe { SelectObject(memory, dib.into()) };
            let blend = BLENDFUNCTION {
                BlendOp: u8::try_from(AC_SRC_OVER).unwrap_or(0),
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: u8::try_from(AC_SRC_ALPHA).unwrap_or(1),
            };
            let position = POINT {
                x: bitmap.x,
                y: bitmap.y,
            };
            let size = SIZE {
                cx: width,
                cy: height,
            };
            let origin = POINT::default();
            // SAFETY: all pointers refer to live locals; the DCs and window are ours.
            let updated = unsafe {
                UpdateLayeredWindow(
                    self.window,
                    Some(screen),
                    Some(&raw const position),
                    Some(&raw const size),
                    Some(memory),
                    Some(&raw const origin),
                    COLORREF(0),
                    Some(&raw const blend),
                    ULW_ALPHA,
                )
            }
            .context("UpdateLayeredWindow");
            // SAFETY: restores the DC's original bitmap, then frees our DIB exactly once.
            unsafe {
                SelectObject(memory, previous);
                let _ = DeleteObject(dib.into());
            }
            updated
        });
        // SAFETY: deletes the memory DC created above exactly once.
        let _ = unsafe { DeleteDC(memory) };
        result
    }

    /// Hides the overlay.
    pub fn hide(&mut self) {
        if self.visible {
            // SAFETY: hides our own window; the return value is the previous visibility.
            let _ = unsafe { ShowWindow(self.window, SW_HIDE) };
            self.visible = false;
        }
    }
}

fn too_large() -> SysError {
    SysError::other("OverlayWindow::update", "bitmap dimensions exceed i32")
}

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        // SAFETY: destroys our own window exactly once, on the thread that created it (!Send).
        let _ = unsafe { DestroyWindow(self.window) };
    }
}
