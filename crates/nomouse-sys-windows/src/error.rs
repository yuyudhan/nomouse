//! Conversion of `windows` crate errors into [`SysError`].

use windows::core::Error;

use crate::types::{SysError, SysErrorKind};

/// `E_ACCESSDENIED`.
const E_ACCESSDENIED: u32 = 0x8007_0005;
/// `UIA_E_ELEMENTNOTAVAILABLE`.
const UIA_E_ELEMENTNOTAVAILABLE: u32 = 0x8004_0201;
/// `HRESULT_FROM_WIN32(ERROR_INVALID_WINDOW_HANDLE)`.
const INVALID_WINDOW_HANDLE: u32 = 0x8007_0578;
/// `HRESULT_FROM_WIN32(ERROR_HOTKEY_ALREADY_REGISTERED)`.
const HOTKEY_ALREADY_REGISTERED: u32 = 0x8007_0581;

/// Classifies an OS error for the call `call`.
pub(crate) fn sys_error(call: &'static str, error: &Error) -> SysError {
    let code = error.code().0;
    let kind = match u32::from_ne_bytes(code.to_ne_bytes()) {
        E_ACCESSDENIED => SysErrorKind::AccessDenied,
        UIA_E_ELEMENTNOTAVAILABLE | INVALID_WINDOW_HANDLE => SysErrorKind::Gone,
        HOTKEY_ALREADY_REGISTERED => SysErrorKind::HotkeyTaken,
        _ => SysErrorKind::Other,
    };
    SysError {
        kind,
        call,
        code,
        message: error.message(),
    }
}

/// The calling thread's last Win32 error as a [`SysError`].
pub(crate) fn last_error(call: &'static str) -> SysError {
    sys_error(call, &Error::from_thread())
}

/// Adds the call name to a `windows` result.
pub(crate) trait Context<T> {
    /// Maps the error with [`sys_error`].
    fn context(self, call: &'static str) -> Result<T, SysError>;
}

impl<T> Context<T> for windows::core::Result<T> {
    fn context(self, call: &'static str) -> Result<T, SysError> {
        self.map_err(|error| sys_error(call, &error))
    }
}
