//! Plain data exchanged with the FFI layer. Compiled on every OS so the backend's pure logic
//! (and its tests) can use these types without Windows.

use std::fmt;

/// UI Automation control type ids (`UIA_*ControlTypeId`), stable values from `UIAutomationClient.h`.
pub mod control_type {
    /// `UIA_ButtonControlTypeId`.
    pub const BUTTON: i32 = 50000;
    /// `UIA_CheckBoxControlTypeId`.
    pub const CHECK_BOX: i32 = 50002;
    /// `UIA_ComboBoxControlTypeId`.
    pub const COMBO_BOX: i32 = 50003;
    /// `UIA_EditControlTypeId`.
    pub const EDIT: i32 = 50004;
    /// `UIA_HyperlinkControlTypeId`.
    pub const HYPERLINK: i32 = 50005;
    /// `UIA_ListItemControlTypeId`.
    pub const LIST_ITEM: i32 = 50007;
    /// `UIA_MenuItemControlTypeId`.
    pub const MENU_ITEM: i32 = 50011;
    /// `UIA_RadioButtonControlTypeId`.
    pub const RADIO_BUTTON: i32 = 50013;
    /// `UIA_ScrollBarControlTypeId`.
    pub const SCROLL_BAR: i32 = 50014;
    /// `UIA_SliderControlTypeId`.
    pub const SLIDER: i32 = 50015;
    /// `UIA_SpinnerControlTypeId`.
    pub const SPINNER: i32 = 50016;
    /// `UIA_TabItemControlTypeId`.
    pub const TAB_ITEM: i32 = 50019;
    /// `UIA_TextControlTypeId`.
    pub const TEXT: i32 = 50020;
    /// `UIA_TreeItemControlTypeId`.
    pub const TREE_ITEM: i32 = 50024;
    /// `UIA_DataItemControlTypeId` (a table or grid cell/row).
    pub const DATA_ITEM: i32 = 50029;
    /// `UIA_DocumentControlTypeId`.
    pub const DOCUMENT: i32 = 50030;
    /// `UIA_SplitButtonControlTypeId`.
    pub const SPLIT_BUTTON: i32 = 50031;
    /// `UIA_WindowControlTypeId`.
    pub const WINDOW: i32 = 50032;
    /// `UIA_PaneControlTypeId`.
    pub const PANE: i32 = 50033;
    /// `UIA_HeaderItemControlTypeId`.
    pub const HEADER_ITEM: i32 = 50035;

    /// The control types the bulk query always returns, whatever their patterns.
    pub const ACTIONABLE: [i32; 14] = [
        BUTTON,
        HYPERLINK,
        CHECK_BOX,
        RADIO_BUTTON,
        MENU_ITEM,
        TAB_ITEM,
        LIST_ITEM,
        TREE_ITEM,
        EDIT,
        COMBO_BOX,
        SLIDER,
        SPLIT_BUTTON,
        DATA_ITEM,
        HEADER_ITEM,
    ];
}

/// An opaque top-level window handle (`HWND` as an integer).
///
/// Holding one is always safe: a stale handle makes later calls fail, never misbehave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowHandle(pub isize);

/// A rectangle by its edges, in physical screen pixels (right and bottom exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Edges {
    /// Left edge.
    pub left: i32,
    /// Top edge.
    pub top: i32,
    /// Right edge (exclusive).
    pub right: i32,
    /// Bottom edge (exclusive).
    pub bottom: i32,
}

impl Edges {
    /// Creates a rectangle from its edges.
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Width, zero when inverted.
    pub fn width(&self) -> u32 {
        u32::try_from(i64::from(self.right) - i64::from(self.left)).unwrap_or(0)
    }

    /// Height, zero when inverted.
    pub fn height(&self) -> u32 {
        u32::try_from(i64::from(self.bottom) - i64::from(self.top)).unwrap_or(0)
    }

    /// Whether `(x, y)` lies inside.
    pub const fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// Which control patterns an element supports (from the cached `Is*PatternAvailable` properties).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Patterns {
    /// `InvokePattern`.
    pub invoke: bool,
    /// `TogglePattern`.
    pub toggle: bool,
    /// `SelectionItemPattern`.
    pub selection_item: bool,
    /// `ExpandCollapsePattern`.
    pub expand_collapse: bool,
    /// `LegacyIAccessiblePattern` with a non-empty default action.
    pub legacy_default_action: bool,
}

impl Patterns {
    /// Whether any activation pattern is present.
    pub const fn any(&self) -> bool {
        self.invoke
            || self.toggle
            || self.selection_item
            || self.expand_collapse
            || self.legacy_default_action
    }
}

/// One element as read from a UI Automation cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawElement {
    /// Index into the [`crate::Automation`] snapshot that produced it; pass it back to activate.
    pub index: usize,
    /// UIA control type id (see [`control_type`]).
    pub control_type: i32,
    /// `Name` property.
    pub name: String,
    /// `BoundingRectangle` in physical pixels.
    pub bounds: Edges,
    /// `IsOffscreen`.
    pub offscreen: bool,
    /// `IsEnabled`.
    pub enabled: bool,
    /// `IsKeyboardFocusable`.
    pub keyboard_focusable: bool,
    /// Supported activation patterns.
    pub patterns: Patterns,
}

/// A semantic activation performed through a control pattern instead of synthetic input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Activation {
    /// `IUIAutomationInvokePattern::Invoke`.
    Invoke,
    /// `IUIAutomationTogglePattern::Toggle`.
    Toggle,
    /// `IUIAutomationSelectionItemPattern::Select`.
    Select,
    /// Expand when collapsed, collapse otherwise (`IUIAutomationExpandCollapsePattern`).
    ExpandCollapse,
    /// `IUIAutomationLegacyIAccessiblePattern::DoDefaultAction`.
    DefaultAction,
}

/// A top-level window, as listed in z-order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// Its handle.
    pub handle: WindowHandle,
    /// Outer bounds in physical pixels.
    pub bounds: Edges,
    /// Window class name.
    pub class_name: String,
    /// Title text.
    pub title: String,
    /// Whether it is a tool window (`WS_EX_TOOLWINDOW`).
    pub tool_window: bool,
}

/// A mouse button for [`InputStep::Button`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    /// Left button.
    Left,
    /// Right button.
    Right,
    /// Middle button.
    Middle,
}

/// One synthetic input event for [`crate::send_input`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputStep {
    /// Absolute move in normalized virtual-desktop coordinates (0..=65535 on each axis).
    Move {
        /// Normalized horizontal coordinate.
        x: i32,
        /// Normalized vertical coordinate.
        y: i32,
    },
    /// Press or release a button at the current position.
    Button {
        /// Which button.
        button: Button,
        /// `true` presses, `false` releases.
        down: bool,
    },
    /// Vertical wheel by raw delta (`WHEEL_DELTA` = 120 per notch, positive = away from the user).
    Wheel(i32),
    /// Horizontal wheel by raw delta (positive = right).
    HorizontalWheel(i32),
    /// Press or release a virtual key (used for modifier clicks).
    Key {
        /// Virtual-key code.
        vk: u16,
        /// `true` presses, `false` releases.
        down: bool,
    },
}

/// An event from [`crate::EventPump::next_event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SysEvent {
    /// `WM_HOTKEY` for a hotkey id registered with [`crate::EventPump::register_hotkey`].
    Hotkey(i32),
    /// A key went down while the keyboard hook was grabbing.
    Key {
        /// Virtual-key code.
        vk: u32,
        /// Whether Shift was held.
        shift: bool,
    },
}

/// Hotkey modifier flags for [`crate::EventPump::register_hotkey`] (the `MOD_*` values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HotkeyModifiers {
    /// `MOD_CONTROL`.
    pub ctrl: bool,
    /// `MOD_ALT`.
    pub alt: bool,
    /// `MOD_SHIFT`.
    pub shift: bool,
    /// `MOD_WIN`.
    pub win: bool,
}

/// A premultiplied ARGB image placed at screen coordinates, for [`crate::OverlayWindow::update`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bitmap {
    /// Left edge on screen.
    pub x: i32,
    /// Top edge on screen.
    pub y: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height` pixels, row-major from the top, `0xAARRGGBB` with premultiplied colour.
    pub pixels: Vec<u32>,
}

/// What kind of failure a [`SysError`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SysErrorKind {
    /// The OS refused access (UIPI, elevated target, `E_ACCESSDENIED`).
    AccessDenied,
    /// `RegisterHotKey` found the chord already registered.
    HotkeyTaken,
    /// The object (window, element) no longer exists.
    Gone,
    /// Anything else.
    Other,
}

/// A failed OS call: which call, its HRESULT or Win32 error, and the kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysError {
    /// Classification.
    pub kind: SysErrorKind,
    /// The OS call that failed.
    pub call: &'static str,
    /// HRESULT (or `HRESULT_FROM_WIN32` of the last error).
    pub code: i32,
    /// The system message for `code`.
    pub message: String,
}

impl SysError {
    /// Creates an error of kind [`SysErrorKind::Other`] without an OS code.
    pub fn other(call: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: SysErrorKind::Other,
            call,
            code: 0,
            message: message.into(),
        }
    }
}

impl fmt::Display for SysError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} failed (0x{:08X}): {}",
            self.call, self.code, self.message
        )
    }
}

impl std::error::Error for SysError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_size_and_contains() {
        let e = Edges::new(10, 20, 30, 25);
        assert_eq!((e.width(), e.height()), (20, 5));
        assert!(e.contains(10, 20));
        assert!(!e.contains(30, 20));
        assert_eq!(Edges::new(5, 5, 1, 1).width(), 0);
    }

    #[test]
    fn patterns_any() {
        assert!(!Patterns::default().any());
        assert!(
            Patterns {
                toggle: true,
                ..Patterns::default()
            }
            .any()
        );
    }
}
