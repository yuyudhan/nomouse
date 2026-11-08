//! Safe wrappers over `AXUIElement`: reading attributes, performing actions, trust checks.

use std::ffi::c_void;
use std::ptr::{self, NonNull};

use objc2_application_services::{
    AXCopyMultipleAttributeOptions, AXError as RawAxError, AXIsProcessTrustedWithOptions,
    AXUIElement, AXValue, AXValueType, kAXTrustedCheckOptionPrompt,
};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGPoint, CGSize,
};

use crate::error::AxError;

/// A decoded attribute value.
#[derive(Debug, Clone, PartialEq)]
pub enum AxValue {
    /// A boolean (`CFBoolean`).
    Bool(bool),
    /// A number (`CFNumber`), widened to `f64`.
    Number(f64),
    /// A string (`CFString`).
    String(String),
    /// A point (`AXValue` of type `kAXValueCGPointType`), in global top-left coordinates.
    Point {
        /// Horizontal coordinate in points.
        x: f64,
        /// Vertical coordinate in points.
        y: f64,
    },
    /// A size (`AXValue` of type `kAXValueCGSizeType`).
    Size {
        /// Width in points.
        width: f64,
        /// Height in points.
        height: f64,
    },
    /// Another element.
    Element(AxElement),
    /// An array of values (one level; nested arrays decode as [`AxValue::Other`]).
    Array(Vec<AxValue>),
    /// A value of a type nomouse does not use.
    Other,
}

/// An accessibility object (`AXUIElementRef`). Equality and hashing follow `CFEqual`/`CFHash`,
/// so two handles to the same UI element compare equal (used for cycle detection).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AxElement(CFRetained<AXUIElement>);

fn check(code: RawAxError) -> Result<(), AxError> {
    AxError::from_code(code.0).map_or(Ok(()), Err)
}

/// Treats "absent attribute" errors as `Ok(None)`.
fn optional<T>(result: Result<T, AxError>) -> Result<Option<T>, AxError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.is_missing_value() => Ok(None),
        Err(error) => Err(error),
    }
}

impl AxElement {
    /// The system-wide element (focused application, focused element).
    pub fn system_wide() -> Self {
        // SAFETY: AXUIElementCreateSystemWide takes no arguments and returns a new +1 reference.
        Self(unsafe { AXUIElement::new_system_wide() })
    }

    /// The top-level element of the application with process id `pid`.
    pub fn application(pid: i32) -> Self {
        // SAFETY: any pid is accepted; for a non-existent process the returned element simply
        // reports `kAXErrorInvalidUIElement` on use.
        Self(unsafe { AXUIElement::new_application(pid) })
    }

    /// Reads one attribute; `Ok(None)` when the element lacks it or it has no value.
    ///
    /// # Errors
    /// Any other [`AxError`], such as [`AxError::CannotComplete`] for an unresponsive app.
    pub fn attribute(&self, name: &str) -> Result<Option<AxValue>, AxError> {
        let name = CFString::from_str(name);
        let mut out: *const CFType = ptr::null();
        // SAFETY: `out` is a valid, writable pointer slot; on success it receives a +1 reference.
        let code = unsafe { self.0.copy_attribute_value(&name, NonNull::from(&mut out)) };
        optional(check(code))?
            // SAFETY: on success `out` holds a +1 reference from AXUIElementCopyAttributeValue.
            .and_then(|()| unsafe { take_retained(out) })
            .map_or(Ok(None), |value| Ok(Some(decode(&value, true))))
    }

    /// Reads several attributes in one IPC round trip (`AXUIElementCopyMultipleAttributeValues`).
    /// Each slot is `None` when that attribute is absent or failed.
    ///
    /// # Errors
    /// An [`AxError`] when the whole request failed (dead element, app not answering).
    pub fn attributes(&self, names: &[&str]) -> Result<Vec<Option<AxValue>>, AxError> {
        let names: Vec<CFRetained<CFString>> =
            names.iter().map(|name| CFString::from_str(name)).collect();
        let array = CFArray::from_retained_objects(&names);
        let mut out: *const CFArray = ptr::null();
        // SAFETY: the attribute array holds only CFStrings and `out` is a valid slot that
        // receives a +1 CFArray on success. Option 0 reports per-attribute errors in place.
        let code = unsafe {
            self.0.copy_multiple_attribute_values(
                array.as_opaque(),
                AXCopyMultipleAttributeOptions(0),
                NonNull::from(&mut out),
            )
        };
        check(code)?;
        // SAFETY: on success `out` holds a +1 CFArray from CopyMultipleAttributeValues.
        let Some(values) = (unsafe { take_retained(out) }) else {
            return Ok(vec![None; names.len()]);
        };
        let mut decoded: Vec<Option<AxValue>> = array_items(&values)
            .iter()
            .map(|value| match decode(value, true) {
                AxValue::Other if is_error_value(value) => None,
                other => Some(other),
            })
            .collect();
        decoded.resize(names.len(), None);
        Ok(decoded)
    }

    /// The action names the element supports, such as `AXPress` and `AXShowMenu`.
    ///
    /// # Errors
    /// An [`AxError`] other than "no actions".
    pub fn action_names(&self) -> Result<Vec<String>, AxError> {
        let mut out: *const CFArray = ptr::null();
        // SAFETY: `out` is a valid slot that receives a +1 CFArray on success.
        let code = unsafe { self.0.copy_action_names(NonNull::from(&mut out)) };
        let Some(()) = optional(check(code))? else {
            return Ok(Vec::new());
        };
        // SAFETY: on success `out` holds a +1 CFArray from AXUIElementCopyActionNames.
        Ok(unsafe { take_retained(out) }
            .map(|names| {
                array_items(&names)
                    .iter()
                    .filter_map(|name| name.downcast_ref::<CFString>().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Performs an action such as `AXPress`.
    ///
    /// # Errors
    /// [`AxError::ActionUnsupported`] when the element lacks it, or another [`AxError`].
    pub fn perform_action(&self, action: &str) -> Result<(), AxError> {
        let action = CFString::from_str(action);
        // SAFETY: both arguments are valid CF objects.
        check(unsafe { self.0.perform_action(&action) })
    }

    /// Sets a boolean attribute, such as `AXManualAccessibility` or `AXEnhancedUserInterface`.
    ///
    /// # Errors
    /// An [`AxError`] when the attribute is not settable.
    pub fn set_bool_attribute(&self, name: &str, value: bool) -> Result<(), AxError> {
        let name = CFString::from_str(name);
        // SAFETY: the value is a valid CFBoolean, which every boolean attribute accepts.
        check(unsafe { self.0.set_attribute_value(&name, CFBoolean::new(value)) })
    }

    /// Sets how long calls on this element (or, on the system-wide element, on every element)
    /// wait for the application before failing with [`AxError::CannotComplete`].
    ///
    /// # Errors
    /// [`AxError::IllegalArgument`] for a negative timeout.
    pub fn set_messaging_timeout(&self, seconds: f32) -> Result<(), AxError> {
        // SAFETY: the element is valid; the call only stores the timeout.
        check(unsafe { self.0.set_messaging_timeout(seconds) })
    }

    /// The process id of the application owning the element.
    ///
    /// # Errors
    /// [`AxError::InvalidElement`] when the element is gone.
    pub fn pid(&self) -> Result<i32, AxError> {
        let mut pid: i32 = 0;
        // SAFETY: `pid` is a valid, writable i32 slot.
        check(unsafe { self.0.pid(NonNull::from(&mut pid)) })?;
        Ok(pid)
    }

    /// The deepest element at global top-left point (`x`, `y`) in points. Call on the
    /// system-wide element to hit-test every application.
    ///
    /// # Errors
    /// An [`AxError`]; `Ok(None)` when nothing accessible is there.
    pub fn element_at(&self, x: f32, y: f32) -> Result<Option<Self>, AxError> {
        let mut out: *const AXUIElement = ptr::null();
        // SAFETY: `out` is a valid slot that receives a +1 AXUIElement on success.
        let code = unsafe {
            self.0
                .copy_element_at_position(x, y, NonNull::from(&mut out))
        };
        Ok(optional(check(code))?
            // SAFETY: on success `out` holds a +1 element from CopyElementAtPosition.
            .and_then(|()| unsafe { take_retained(out) })
            .map(Self))
    }
}

/// Whether this process is trusted for Accessibility. With `prompt`, macOS shows its
/// "would like to control this computer" dialog when it is not (once per launch).
pub fn is_process_trusted(prompt: bool) -> bool {
    // SAFETY: kAXTrustedCheckOptionPrompt is an immutable CFString constant.
    let key: &CFString = unsafe { kAXTrustedCheckOptionPrompt };
    let options =
        CFDictionary::<CFString, CFBoolean>::from_slices(&[key], &[CFBoolean::new(prompt)]);
    // SAFETY: the dictionary maps the documented option key to a CFBoolean.
    unsafe { AXIsProcessTrustedWithOptions(Some(options.as_opaque())) }
}

/// Takes ownership of a +1 reference written by a `Copy` function.
///
/// # Safety
/// `raw` must be null or a valid pointer to a `T` carrying a +1 retain count that the caller
/// owns (as returned by a CF/AX "Copy" or "Create" function); ownership moves to the result.
unsafe fn take_retained<T: objc2_core_foundation::Type>(raw: *const T) -> Option<CFRetained<T>> {
    // SAFETY: guaranteed by this function's contract: a non-null `raw` is an owned +1
    // reference, so adopting it without an extra retain balances the count.
    NonNull::new(raw.cast_mut()).map(|raw| unsafe { CFRetained::from_raw(raw) })
}

fn array_items(array: &CFArray) -> Vec<CFRetained<CFType>> {
    // SAFETY: every element of a CFArray is a CF object, so viewing it as `CFArray<CFType>` is
    // sound; the retained copies outlive the borrow.
    unsafe { array.cast_unchecked::<CFType>() }.to_vec()
}

fn is_error_value(value: &CFType) -> bool {
    value
        .downcast_ref::<AXValue>()
        // SAFETY: AXValueGetType only reads the type tag of a valid AXValue.
        .is_some_and(|value| unsafe { value.r#type() } == AXValueType::AXError)
}

fn decode(value: &CFType, allow_array: bool) -> AxValue {
    if let Some(string) = value.downcast_ref::<CFString>() {
        return AxValue::String(string.to_string());
    }
    if let Some(boolean) = value.downcast_ref::<CFBoolean>() {
        return AxValue::Bool(boolean.as_bool());
    }
    if let Some(number) = value.downcast_ref::<CFNumber>() {
        return number.as_f64().map_or(AxValue::Other, AxValue::Number);
    }
    if let Some(element) = value.downcast_ref::<AXUIElement>() {
        // SAFETY: `element` is a live AXUIElement, so taking an extra +1 reference is sound.
        let owned = unsafe { CFRetained::retain(NonNull::from(element)) };
        return AxValue::Element(AxElement(owned));
    }
    if let Some(ax_value) = value.downcast_ref::<AXValue>() {
        return decode_ax_value(ax_value);
    }
    if allow_array && let Some(array) = value.downcast_ref::<CFArray>() {
        return AxValue::Array(
            array_items(array)
                .iter()
                .map(|item| decode(item, false))
                .collect(),
        );
    }
    AxValue::Other
}

fn decode_ax_value(value: &AXValue) -> AxValue {
    // SAFETY: AXValueGetType only reads the type tag of a valid AXValue.
    let kind = unsafe { value.r#type() };
    if kind == AXValueType::CGPoint {
        let mut point = CGPoint::new(0.0, 0.0);
        let slot: NonNull<c_void> = NonNull::from(&mut point).cast();
        // SAFETY: the tag says the payload is a CGPoint and `slot` points to one.
        if unsafe { value.value(kind, slot) } {
            return AxValue::Point {
                x: point.x,
                y: point.y,
            };
        }
    } else if kind == AXValueType::CGSize {
        let mut size = CGSize::new(0.0, 0.0);
        let slot: NonNull<c_void> = NonNull::from(&mut size).cast();
        // SAFETY: the tag says the payload is a CGSize and `slot` points to one.
        if unsafe { value.value(kind, slot) } {
            return AxValue::Size {
                width: size.width,
                height: size.height,
            };
        }
    }
    AxValue::Other
}
