//! COM apartment lifetime.

use std::marker::PhantomData;

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

use crate::error::sys_error;
use crate::types::SysError;

/// A single-threaded COM apartment on the current thread, released on drop.
///
/// Not `Send`: COM apartments belong to the thread that entered them.
#[derive(Debug)]
pub struct ComApartment {
    _thread_bound: PhantomData<*const ()>,
}

impl ComApartment {
    /// Enters a single-threaded apartment (`CoInitializeEx(COINIT_APARTMENTTHREADED)`).
    ///
    /// # Errors
    /// Fails with `RPC_E_CHANGED_MODE` when the thread is already multi-threaded.
    pub fn enter() -> Result<Self, SysError> {
        // SAFETY: CoInitializeEx takes no pointers besides the reserved null and may be called
        // any number of times per thread; every success (S_OK or S_FALSE) is balanced by the
        // CoUninitialize in Drop.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        hr.ok()
            .map_err(|error| sys_error("CoInitializeEx", &error))?;
        Ok(Self {
            _thread_bound: PhantomData,
        })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: balances the successful CoInitializeEx in `enter` on the same thread (the type
        // is !Send); objects created in the apartment are owned by structs that drop first.
        unsafe { CoUninitialize() };
    }
}
