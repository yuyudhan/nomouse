//! Pictures of the app's own windows and open menu, for the documentation's screenshots.
//!
//! The views draw themselves into a bitmap in this process, so no Screen Recording
//! permission is needed (`screencapture` needs it even for a process's own windows).

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, Message as _, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSApplication, NSBitmapImageFileType, NSMenu, NSView, NSWindow,
};
use objc2_foundation::{NSDictionary, NSObject, NSRunLoop, NSRunLoopCommonModes, NSTimer};

/// A menu being pictured, and its picture once taken.
type Pictured = (Retained<NSMenu>, Option<Vec<u8>>);

thread_local! {
    static MENU: RefCell<Option<Pictured>> = const { RefCell::new(None) };
}

/// Draws the app's windows and menus light (`Some(false)`), dark (`Some(true)`) or as the
/// system does (`None`).
pub fn set_appearance(mtm: MainThreadMarker, dark: Option<bool>) {
    let app = NSApplication::sharedApplication(mtm);
    let appearance = dark.and_then(|dark| {
        // SAFETY: both names are immutable NSString constants exported by AppKit.
        let name = unsafe {
            if dark {
                NSAppearanceNameDarkAqua
            } else {
                NSAppearanceNameAqua
            }
        };
        NSAppearance::appearanceNamed(name)
    });
    app.setAppearance(appearance.as_deref());
}

/// `view` as drawn now, as PNG bytes at the screen's scale.
pub(crate) fn render(view: &NSView) -> Option<Vec<u8>> {
    let bounds = view.bounds();
    let rep = view.bitmapImageRepForCachingDisplayInRect(bounds)?;
    view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
    // SAFETY: an empty properties dictionary is valid for PNG.
    let png = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(png.to_vec())
}

/// `window` with its title bar as drawn now (its content view's superview, the frame view),
/// as PNG bytes.
pub(crate) fn render_window(window: &NSWindow) -> Option<Vec<u8>> {
    let content = window.contentView()?;
    // SAFETY: `superview` only reads the view hierarchy; the window's views live on the main
    // thread this runs on.
    let frame = unsafe { content.superview() }.unwrap_or(content);
    render(&frame)
}

/// Pictures the app's visible menu windows and closes the menu: run from a timer while the
/// menu tracks the mouse.
fn picture_open_menu(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    let picture = app
        .windows()
        .iter()
        .filter(|window| window.isVisible())
        .filter(|window| {
            let class: &AnyObject = window;
            class.class().name().to_string_lossy().contains("Menu")
        })
        .find_map(|window| render_window(&window));
    MENU.with(|cell| {
        if let Some((menu, png)) = cell.borrow_mut().as_mut() {
            *png = picture;
            menu.cancelTracking();
        }
    });
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and `Snapper` does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct Snapper;

    impl Snapper {
        #[unsafe(method(snap:))]
        fn snap(&self, _timer: Option<&AnyObject>) {
            picture_open_menu(self.mtm());
        }
    }
);

impl Snapper {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's `init` on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

/// Opens `menu` with `open` (which returns when the menu closes) and pictures it once it is
/// drawn; `None` when no menu window appeared.
pub(crate) fn picture_menu(
    mtm: MainThreadMarker,
    menu: &NSMenu,
    open: impl FnOnce(),
) -> Option<Vec<u8>> {
    MENU.with(|cell| *cell.borrow_mut() = Some((menu.retain(), None)));
    // A status item's menu follows the menu bar's look, not the app's: give it the app's
    // appearance for the picture, then hand it back.
    let before = menu.appearance();
    menu.setAppearance(
        NSApplication::sharedApplication(mtm)
            .appearance()
            .as_deref(),
    );
    let snapper = Snapper::new(mtm);
    // SAFETY: `snap:` is implemented by `snapper`, which the timer retains until it fires.
    let timer = unsafe {
        NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
            0.5,
            &snapper,
            sel!(snap:),
            None,
            false,
        )
    };
    // SAFETY: NSRunLoopCommonModes is an immutable NSString constant; common modes include
    // the event tracking mode a menu runs in.
    unsafe { NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
    open();
    timer.invalidate();
    menu.setAppearance(before.as_deref());
    MENU.with(|cell| cell.borrow_mut().take())
        .and_then(|(_, png)| png)
}
