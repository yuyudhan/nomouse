//! UI Automation: one cached bulk query per window and pattern-based activation.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, CUIAutomation8, ExpandCollapseState_Collapsed,
    ExpandCollapseState_PartiallyExpanded, IUIAutomation, IUIAutomation2,
    IUIAutomationCacheRequest, IUIAutomationCondition, IUIAutomationElement,
    IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
    IUIAutomationLegacyIAccessiblePattern, IUIAutomationSelectionItemPattern,
    IUIAutomationTogglePattern, TreeScope_Descendants, UIA_BoundingRectanglePropertyId,
    UIA_ControlTypePropertyId, UIA_ExpandCollapsePatternId, UIA_InvokePatternId,
    UIA_IsEnabledPropertyId, UIA_IsExpandCollapsePatternAvailablePropertyId,
    UIA_IsInvokePatternAvailablePropertyId, UIA_IsKeyboardFocusablePropertyId,
    UIA_IsLegacyIAccessiblePatternAvailablePropertyId, UIA_IsOffscreenPropertyId,
    UIA_IsScrollPatternAvailablePropertyId, UIA_IsSelectionItemPatternAvailablePropertyId,
    UIA_IsTogglePatternAvailablePropertyId, UIA_LegacyIAccessibleDefaultActionPropertyId,
    UIA_LegacyIAccessiblePatternId, UIA_NamePropertyId, UIA_PATTERN_ID, UIA_PROPERTY_ID,
    UIA_SelectionItemPatternId, UIA_TogglePatternId,
};
use windows::core::{BSTR, Interface};

use crate::com::ComApartment;
use crate::error::Context;
use crate::types::{
    Activation, Edges, Patterns, RawElement, Snapshot, SysError, SysErrorKind, WindowHandle,
    control_type,
};

/// Properties fetched for every element in one round trip.
const CACHED_PROPERTIES: [UIA_PROPERTY_ID; 13] = [
    UIA_BoundingRectanglePropertyId,
    UIA_ControlTypePropertyId,
    UIA_NamePropertyId,
    UIA_IsOffscreenPropertyId,
    UIA_IsEnabledPropertyId,
    UIA_IsKeyboardFocusablePropertyId,
    UIA_IsInvokePatternAvailablePropertyId,
    UIA_IsTogglePatternAvailablePropertyId,
    UIA_IsSelectionItemPatternAvailablePropertyId,
    UIA_IsExpandCollapsePatternAvailablePropertyId,
    UIA_IsLegacyIAccessiblePatternAvailablePropertyId,
    UIA_LegacyIAccessibleDefaultActionPropertyId,
    UIA_IsScrollPatternAvailablePropertyId,
];

/// Patterns cached with each element so activation needs no extra lookup.
const CACHED_PATTERNS: [UIA_PATTERN_ID; 5] = [
    UIA_InvokePatternId,
    UIA_TogglePatternId,
    UIA_SelectionItemPatternId,
    UIA_ExpandCollapsePatternId,
    UIA_LegacyIAccessiblePatternId,
];

/// Pattern-availability properties that make an element a query match (scrollable containers
/// included, for scroll mode).
const PATTERN_AVAILABLE: [UIA_PROPERTY_ID; 5] = [
    UIA_IsInvokePatternAvailablePropertyId,
    UIA_IsTogglePatternAvailablePropertyId,
    UIA_IsSelectionItemPatternAvailablePropertyId,
    UIA_IsExpandCollapsePatternAvailablePropertyId,
    UIA_IsScrollPatternAvailablePropertyId,
];

/// A UI Automation client with a prepared cache request and query condition.
///
/// Elements of the current snapshot are retained so [`Automation::activate`] can act on them.
pub struct Automation {
    client: IUIAutomation,
    cache: IUIAutomationCacheRequest,
    condition: IUIAutomationCondition,
    retained: Vec<IUIAutomationElement>,
    uses_automation8: bool,
    // Declared last: COM objects above are released before the apartment is left.
    _apartment: ComApartment,
}

impl std::fmt::Debug for Automation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Automation")
            .field("retained", &self.retained.len())
            .field("uses_automation8", &self.uses_automation8)
            .finish_non_exhaustive()
    }
}

impl Automation {
    /// Enters a single-threaded COM apartment and creates the UIA client (`CUIAutomation8` when
    /// available, else `CUIAutomation`).
    ///
    /// Creation is serialised process-wide: when clients are created at the same moment on
    /// several threads (parallel tests, a doctor check beside the backend), `UIAutomationCore`
    /// fails `CreateCacheRequest` with `E_FAIL` (reproduced on windows-latest by
    /// `tests/concurrent_clients.rs`, T96). Clients are created once per backend, so the lock
    /// costs nothing in practice.
    ///
    /// # Errors
    /// Fails when COM or UI Automation cannot be initialised.
    pub fn new() -> Result<Self, SysError> {
        static CREATION: Mutex<()> = Mutex::new(());
        // A poisoned lock only means another creation panicked; the guard protects no data.
        let _serialised = CREATION.lock().unwrap_or_else(PoisonError::into_inner);
        let apartment = ComApartment::enter()?;
        // SAFETY: COM is initialised on this thread (apartment above); the CLSIDs are static and
        // CoCreateInstance returns an owned interface pointer wrapped by the windows crate.
        let (automation, uses_automation8) = match unsafe {
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
        } {
            Ok(automation) => (automation, true),
            // SAFETY: as above, with the pre-Windows-8 class.
            Err(_) => unsafe {
                CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            }
            .map(|automation| (automation, false))
            .context("CoCreateInstance(CUIAutomation)")?,
        };
        let cache = Self::cache_request(&automation)?;
        let condition = Self::condition(&automation)?;
        Ok(Self {
            client: automation,
            cache,
            condition,
            retained: Vec::new(),
            uses_automation8,
            _apartment: apartment,
        })
    }

    /// Whether the client is `CUIAutomation8` (Windows 8 and later).
    pub const fn uses_automation8(&self) -> bool {
        self.uses_automation8
    }

    fn cache_request(automation: &IUIAutomation) -> Result<IUIAutomationCacheRequest, SysError> {
        // SAFETY: `automation` is a live interface; the call returns an owned cache request.
        let cache = unsafe { automation.CreateCacheRequest() }.context("CreateCacheRequest")?;
        for property in CACHED_PROPERTIES {
            // SAFETY: `cache` is live and the id is a documented UIA property.
            unsafe { cache.AddProperty(property) }.context("AddProperty")?;
        }
        for pattern in CACHED_PATTERNS {
            // SAFETY: `cache` is live and the id is a documented UIA pattern.
            unsafe { cache.AddPattern(pattern) }.context("AddPattern")?;
        }
        Ok(cache)
    }

    /// `!IsOffscreen && (ControlType in ACTIONABLE || any pattern available || IsKeyboardFocusable)`.
    fn condition(automation: &IUIAutomation) -> Result<IUIAutomationCondition, SysError> {
        let property = |id: UIA_PROPERTY_ID, value: &VARIANT| {
            // SAFETY: `automation` is live; `value` outlives the call, which copies it.
            unsafe { automation.CreatePropertyCondition(id, value) }
                .context("CreatePropertyCondition")
        };
        let yes = VARIANT::from(true);
        let mut any = property(UIA_IsKeyboardFocusablePropertyId, &yes)?;
        for id in PATTERN_AVAILABLE {
            let next = property(id, &yes)?;
            // SAFETY: both conditions are live interfaces; the result is a new owned condition.
            any = unsafe { automation.CreateOrCondition(&any, &next) }
                .context("CreateOrCondition")?;
        }
        for kind in control_type::ACTIONABLE {
            let next = property(UIA_ControlTypePropertyId, &VARIANT::from(kind))?;
            // SAFETY: as above.
            any = unsafe { automation.CreateOrCondition(&any, &next) }
                .context("CreateOrCondition")?;
        }
        let onscreen = property(UIA_IsOffscreenPropertyId, &VARIANT::from(false))?;
        // SAFETY: as above.
        unsafe { automation.CreateAndCondition(&onscreen, &any) }.context("CreateAndCondition")
    }

    /// Drops the retained elements of earlier snapshots.
    pub fn clear(&mut self) {
        self.retained.clear();
    }

    /// Bounds how long UIA waits for an unresponsive provider: `connection` for connecting to
    /// it, `transaction` for each call. Needs `IUIAutomation2` (Windows 8+); returns `false`
    /// when the client lacks it and the system defaults apply.
    ///
    /// # Errors
    /// When the client rejects the values.
    pub fn set_timeouts(
        &self,
        connection: Duration,
        transaction: Duration,
    ) -> Result<bool, SysError> {
        let Ok(client) = self.client.cast::<IUIAutomation2>() else {
            return Ok(false);
        };
        let millis = |d: Duration| u32::try_from(d.as_millis()).unwrap_or(u32::MAX).max(1);
        // SAFETY: `client` is a live IUIAutomation2; the calls take plain integers.
        unsafe { client.SetConnectionTimeout(millis(connection)) }
            .context("SetConnectionTimeout")?;
        // SAFETY: as above.
        unsafe { client.SetTransactionTimeout(millis(transaction)) }
            .context("SetTransactionTimeout")?;
        Ok(true)
    }

    /// Appends up to `limit` matching descendants of `window` to the snapshot with one
    /// `FindAllBuildCache` call and returns them with their cached properties; `truncated`
    /// tells whether the query found more than `limit`.
    ///
    /// # Errors
    /// [`SysErrorKind::AccessDenied`] for windows of a higher integrity level (UIPI),
    /// [`SysErrorKind::Gone`] when the window closed.
    pub fn snapshot(&mut self, window: WindowHandle, limit: usize) -> Result<Snapshot, SysError> {
        let hwnd = HWND(window.0 as *mut core::ffi::c_void);
        // SAFETY: `client` is live; a stale HWND makes the call fail, not misbehave.
        let root = unsafe { self.client.ElementFromHandle(hwnd) }.context("ElementFromHandle")?;
        // SAFETY: `root`, the condition and the cache request are live interfaces.
        let found =
            unsafe { root.FindAllBuildCache(TreeScope_Descendants, &self.condition, &self.cache) }
                .context("FindAllBuildCache")?;
        // SAFETY: `found` is a live element array.
        let length = unsafe { found.Length() }.context("IUIAutomationElementArray::Length")?;
        let length = usize::try_from(length).unwrap_or(0);
        let taken = length.min(limit);
        let mut elements = Vec::with_capacity(taken);
        for position in (0..taken).filter_map(|p| i32::try_from(p).ok()) {
            // SAFETY: `position` is within 0..Length.
            let element = unsafe { found.GetElement(position) }.context("GetElement")?;
            // Elements that vanish between the query and the read are skipped.
            if let Ok(raw) = read_cached(&element, self.retained.len()) {
                self.retained.push(element);
                elements.push(raw);
            }
        }
        Ok(Snapshot {
            elements,
            truncated: length > taken,
        })
    }

    /// Performs `activation` on the element with snapshot index `index`.
    ///
    /// # Errors
    /// [`SysErrorKind::Gone`] for an unknown index or a vanished element; the pattern's error
    /// when the element does not support `activation`.
    pub fn activate(&self, index: usize, activation: Activation) -> Result<(), SysError> {
        let element = self.retained.get(index).ok_or_else(|| SysError {
            kind: SysErrorKind::Gone,
            call: "Automation::activate",
            code: 0,
            message: format!("no element {index} in the current snapshot"),
        })?;
        match activation {
            Activation::Invoke => {
                let pattern: IUIAutomationInvokePattern =
                    cached_pattern(element, UIA_InvokePatternId)?;
                // SAFETY: `pattern` is a live pattern interface of a live element.
                unsafe { pattern.Invoke() }.context("Invoke")
            }
            Activation::Toggle => {
                let pattern: IUIAutomationTogglePattern =
                    cached_pattern(element, UIA_TogglePatternId)?;
                // SAFETY: as above.
                unsafe { pattern.Toggle() }.context("Toggle")
            }
            Activation::Select => {
                let pattern: IUIAutomationSelectionItemPattern =
                    cached_pattern(element, UIA_SelectionItemPatternId)?;
                // SAFETY: as above.
                unsafe { pattern.Select() }.context("Select")
            }
            Activation::ExpandCollapse => {
                let pattern: IUIAutomationExpandCollapsePattern =
                    cached_pattern(element, UIA_ExpandCollapsePatternId)?;
                // SAFETY: as above.
                let state = unsafe { pattern.CurrentExpandCollapseState() }
                    .context("CurrentExpandCollapseState")?;
                if state == ExpandCollapseState_Collapsed
                    || state == ExpandCollapseState_PartiallyExpanded
                {
                    // SAFETY: as above.
                    unsafe { pattern.Expand() }.context("Expand")
                } else {
                    // SAFETY: as above.
                    unsafe { pattern.Collapse() }.context("Collapse")
                }
            }
            Activation::DefaultAction => {
                let pattern: IUIAutomationLegacyIAccessiblePattern =
                    cached_pattern(element, UIA_LegacyIAccessiblePatternId)?;
                // SAFETY: as above.
                unsafe { pattern.DoDefaultAction() }.context("DoDefaultAction")
            }
        }
    }
}

fn cached_pattern<T: Interface>(
    element: &IUIAutomationElement,
    pattern: UIA_PATTERN_ID,
) -> Result<T, SysError> {
    // SAFETY: `element` is live and was built with a cache request containing `pattern`.
    unsafe { element.GetCachedPatternAs::<T>(pattern) }.context("GetCachedPatternAs")
}

fn cached_flag(element: &IUIAutomationElement, property: UIA_PROPERTY_ID) -> bool {
    // SAFETY: `element` is live and `property` was in its cache request.
    unsafe { element.GetCachedPropertyValue(property) }
        .ok()
        .and_then(|value| bool::try_from(&value).ok())
        .unwrap_or(false)
}

fn read_cached(element: &IUIAutomationElement, index: usize) -> Result<RawElement, SysError> {
    // SAFETY: (all reads below) `element` is live and every property is in its cache request;
    // the returned BSTR/RECT/BOOL values are owned copies.
    let rect = unsafe { element.CachedBoundingRectangle() }.context("CachedBoundingRectangle")?;
    // SAFETY: see above.
    let kind = unsafe { element.CachedControlType() }.context("CachedControlType")?;
    // SAFETY: see above.
    let name = unsafe { element.CachedName() }.unwrap_or_else(|_| BSTR::new());
    // SAFETY: see above.
    let offscreen = unsafe { element.CachedIsOffscreen() }.is_ok_and(windows::core::BOOL::as_bool);
    // SAFETY: see above.
    let enabled = unsafe { element.CachedIsEnabled() }.is_ok_and(windows::core::BOOL::as_bool);
    // SAFETY: see above.
    let focusable =
        unsafe { element.CachedIsKeyboardFocusable() }.is_ok_and(windows::core::BOOL::as_bool);
    let default_action = cached_pattern::<IUIAutomationLegacyIAccessiblePattern>(
        element,
        UIA_LegacyIAccessiblePatternId,
    )
    .ok()
    // SAFETY: the pattern was cached with the element, so its cached properties are readable.
    .and_then(|pattern| unsafe { pattern.CachedDefaultAction() }.ok())
    .is_some_and(|action| !action.is_empty());
    Ok(RawElement {
        index,
        control_type: kind.0,
        name: name.to_string(),
        bounds: Edges::new(rect.left, rect.top, rect.right, rect.bottom),
        offscreen,
        enabled,
        keyboard_focusable: focusable,
        patterns: Patterns {
            invoke: cached_flag(element, UIA_IsInvokePatternAvailablePropertyId),
            toggle: cached_flag(element, UIA_IsTogglePatternAvailablePropertyId),
            selection_item: cached_flag(element, UIA_IsSelectionItemPatternAvailablePropertyId),
            expand_collapse: cached_flag(element, UIA_IsExpandCollapsePatternAvailablePropertyId),
            legacy_default_action: default_action
                && cached_flag(element, UIA_IsLegacyIAccessiblePatternAvailablePropertyId),
            scroll: cached_flag(element, UIA_IsScrollPatternAvailablePropertyId),
        },
    })
}
