//! Start at login through `SMAppService.mainAppService` (macOS 13 and later): the app itself
//! is the login item, listed under System Settings > General > Login Items.

use objc2::runtime::AnyClass;
use objc2_foundation::NSBundle;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// Whether nomouse starts at login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginItem {
    /// It starts at login.
    Enabled,
    /// Registered, but the user has to allow it in System Settings > Login Items.
    NeedsApproval,
    /// It does not start at login.
    Disabled,
    /// It cannot be a login item here; the reason.
    Unavailable(&'static str),
}

/// The app's login item service, when this macOS has `SMAppService` and nomouse runs from
/// its app bundle.
fn service() -> Result<objc2::rc::Retained<SMAppService>, &'static str> {
    if AnyClass::get(c"SMAppService").is_none() {
        return Err("needs macOS 13 or later");
    }
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        return Err("needs nomouse.app");
    }
    // SAFETY: the class exists (checked above) and `mainAppService` takes no arguments.
    Ok(unsafe { SMAppService::mainAppService() })
}

/// Whether nomouse starts at login.
pub fn login_item() -> LoginItem {
    match service() {
        // SAFETY: `status` takes no arguments and only reads the registration.
        Ok(service) => match unsafe { service.status() } {
            SMAppServiceStatus::Enabled => LoginItem::Enabled,
            SMAppServiceStatus::RequiresApproval => LoginItem::NeedsApproval,
            _ => LoginItem::Disabled,
        },
        Err(reason) => LoginItem::Unavailable(reason),
    }
}

/// Makes nomouse start at login (`on`) or not.
///
/// # Errors
/// The system's reason when registering or unregistering fails, or why there is no login item
/// here.
pub fn set_login_item(on: bool) -> Result<(), String> {
    let service = service().map_err(str::to_owned)?;
    // SAFETY: both calls take no arguments and report failure through the returned error.
    let result = unsafe {
        if on {
            service.registerAndReturnError()
        } else {
            service.unregisterAndReturnError()
        }
    };
    result.map_err(|error| error.localizedDescription().to_string())
}

/// Opens System Settings at Login Items, where a login item waiting for approval is allowed.
pub fn open_login_items_settings() {
    if service().is_ok() {
        // SAFETY: a class method without arguments; the class exists (checked by `service`).
        unsafe { SMAppService::openSystemSettingsLoginItems() };
    }
}
