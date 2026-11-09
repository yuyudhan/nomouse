//! Small panels and system services around the windows: alerts, the about panel, the app
//! picker, opening links and files, and app names and icons.

use std::path::{Path, PathBuf};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSAboutPanelOptionApplicationIcon, NSAboutPanelOptionApplicationName,
    NSAboutPanelOptionApplicationVersion, NSAboutPanelOptionCredits, NSAboutPanelOptionVersion,
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSImage, NSModalResponseOK,
    NSOpenPanel, NSPasteboard, NSPasteboardTypeString, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSBundle, NSDictionary, NSSize, NSString, NSURL,
};
use objc2_uniform_type_identifiers::UTType;

use super::{activate, image};

/// Shows a modal alert in front of other apps and returns the index of the button clicked
/// (the first is the default and answers Return). `warning` draws the caution badge.
pub fn alert(
    mtm: MainThreadMarker,
    title: &str,
    message: &str,
    buttons: &[&str],
    warning: bool,
) -> usize {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    if warning {
        alert.setAlertStyle(NSAlertStyle::Warning);
    }
    for button in buttons {
        alert.addButtonWithTitle(&NSString::from_str(button));
    }
    activate(mtm);
    let response = alert.runModal();
    usize::try_from(response - NSAlertFirstButtonReturn).unwrap_or(0)
}

/// Puts `text` on the clipboard.
pub fn copy(text: &str) {
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    // SAFETY: NSPasteboardTypeString is an immutable NSString constant.
    board.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
}

/// Shows macOS's standard about panel with `name`, `version`, `icon` (PNG bytes) and a line
/// of `credits`.
pub fn about(mtm: MainThreadMarker, name: &str, version: &str, icon: &[u8], credits: &str) {
    let name = NSString::from_str(name);
    let version_text = NSString::from_str(version);
    let credits = NSAttributedString::from_nsstring(&NSString::from_str(credits));
    let mut keys = Vec::new();
    let mut values: Vec<Retained<AnyObject>> = Vec::new();
    // SAFETY: the option keys are immutable NSString constants exported by AppKit.
    unsafe {
        keys.push(NSAboutPanelOptionApplicationName);
        values.push(Retained::into_super(Retained::into_super(name)));
        keys.push(NSAboutPanelOptionApplicationVersion);
        values.push(Retained::into_super(Retained::into_super(version_text)));
        keys.push(NSAboutPanelOptionVersion);
        values.push(Retained::into_super(Retained::into_super(
            NSString::from_str(""),
        )));
        keys.push(NSAboutPanelOptionCredits);
        values.push(Retained::into_super(Retained::into_super(credits)));
        if let Some(icon) = image(icon, 128.0, false) {
            keys.push(NSAboutPanelOptionApplicationIcon);
            values.push(Retained::into_super(Retained::into_super(icon)));
        }
    }
    let values: Vec<&AnyObject> = values.iter().map(|value| &**value).collect();
    let options = NSDictionary::from_slices(&keys, &values);
    activate(mtm);
    // SAFETY: every value has the type its key documents (strings, an attributed string and
    // an image).
    unsafe {
        NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanelWithOptions(&options);
    }
}

/// Opens `url` in the default browser.
pub fn open_url(url: &str) -> bool {
    NSURL::URLWithString(&NSString::from_str(url))
        .is_some_and(|url| NSWorkspace::sharedWorkspace().openURL(&url))
}

/// An application the user picked or the configuration names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    /// Its bundle id (`com.apple.Terminal`), or its executable name when it has none.
    pub id: String,
    /// The name Finder shows.
    pub name: String,
    /// Where it is installed, when it was found.
    pub path: Option<PathBuf>,
}

/// Asks for one or more applications with an open panel starting in `/Applications`; empty
/// when cancelled.
pub fn choose_apps(mtm: MainThreadMarker, prompt: &str) -> Vec<AppInfo> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(true);
    panel.setMessage(Some(&NSString::from_str(prompt)));
    panel.setPrompt(Some(&NSString::from_str("Add")));
    panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&NSString::from_str(
        "/Applications",
    ))));
    if let Some(bundle) =
        UTType::typeWithIdentifier(&NSString::from_str("com.apple.application-bundle"))
    {
        panel.setAllowedContentTypes(&NSArray::from_retained_slice(&[bundle]));
    }
    activate(mtm);
    if panel.runModal() != NSModalResponseOK {
        return Vec::new();
    }
    panel
        .URLs()
        .iter()
        .filter_map(|url| url.path())
        .map(|path| app_at(Path::new(&path.to_string())))
        .collect()
}

/// The app bundle at `path`: its bundle id (or executable name) and display name.
pub fn app_at(path: &Path) -> AppInfo {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy()));
    let id = bundle
        .as_ref()
        .and_then(|bundle| bundle.bundleIdentifier())
        .map_or_else(|| stem.clone(), |id| id.to_string());
    AppInfo {
        id,
        name: display_name(path).unwrap_or(stem),
        path: Some(path.to_owned()),
    }
}

/// The installed app with bundle id `id`, or `id` alone when no such app is installed (an
/// executable name, or an app on another Mac).
pub fn app_for(id: &str) -> AppInfo {
    let url = NSWorkspace::sharedWorkspace()
        .URLForApplicationWithBundleIdentifier(&NSString::from_str(id));
    match url.and_then(|url| url.path()) {
        Some(path) => {
            let path = PathBuf::from(path.to_string());
            AppInfo {
                id: id.to_owned(),
                name: display_name(&path).unwrap_or_else(|| id.to_owned()),
                path: Some(path),
            }
        }
        None => AppInfo {
            id: id.to_owned(),
            name: id.to_owned(),
            path: None,
        },
    }
}

fn display_name(path: &Path) -> Option<String> {
    let manager = objc2_foundation::NSFileManager::defaultManager();
    let name = manager.displayNameAtPath(&NSString::from_str(&path.to_string_lossy()));
    let name = name.to_string();
    Some(name.trim_end_matches(".app").to_owned()).filter(|name| !name.is_empty())
}

/// The Finder icon of the file or app at `path`, `points` wide.
pub(crate) fn icon(path: &Path, points: f64) -> Retained<NSImage> {
    let icon =
        NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()));
    icon.setSize(NSSize::new(points, points));
    icon
}
