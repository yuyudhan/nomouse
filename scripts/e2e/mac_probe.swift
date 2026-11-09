// Accessibility queries and synthetic keys for scripts/e2e-macos.sh.
//
//   mac_probe trusted                 exit 0 when this process is trusted for Accessibility
//   mac_probe front <pid>             make <pid> the frontmost app and raise its first window
//   mac_probe frontmost               print the frontmost app's pid
//   mac_probe value <pid> <id>        print the value of <pid>'s element with AXIdentifier <id>
//   mac_probe windows <pid>           print how many windows <pid> has, as Accessibility lists them
//   mac_probe covering <pid>          print the owners of other apps' windows in front of <pid>'s
//                                     first window and overlapping it (they would take the clicks)
//   mac_probe type <text>             type <text>, each character on the key that types it
//                                     in the current keyboard layout
//   mac_probe key <chord>...          press chords such as escape, return, tab, ctrl+space
//   mac_probe layout                  print the current keyboard layout's input source id
//   mac_probe select-layout <id>      enable and select the keyboard layout input source <id>
//   mac_probe layout-enabled <id>     exit 0 when <id> is in the user's input sources
//   mac_probe disable-layout <id>     remove <id> from the user's input sources
//   mac_probe status-item <pid>       print the description of <pid>'s menu bar (status) item;
//                                     exit 1 when it has none
//   mac_probe menu <pid> <title>      open <pid>'s status item menu and pick the item <title>
//   mac_probe press <pid> <id>        press <pid>'s element with AXIdentifier <id>
//   mac_probe press-title <pid> <title>  press <pid>'s element titled <title> (a toolbar tab)
//   mac_probe choose <pid> <id> <title>  open the pop-up button <id> and pick <title>
//   mac_probe window <pid> <name>     exit 0 when <pid> has a window with AXIdentifier or
//                                     title <name>
//   mac_probe titles <pid>            print each of <pid>'s windows: title, AXIdentifier
//   mac_probe close <pid> <name>      press the close button of <pid>'s window <name>
//   mac_probe panels <pid>            print how many of <pid>'s windows are on screen at the
//                                     overlay's (screen saver) level: its label panels, not
//                                     its menu bar item or settings window
//
// Every query is bounded: Accessibility calls time out after one second.
import AppKit
import ApplicationServices
import Carbon

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("mac_probe: \(message)\n".utf8))
    exit(1)
}

func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else {
        return nil
    }
    return value
}

func children(_ element: AXUIElement) -> [AXUIElement] {
    attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []
}

func application(_ argument: String) -> AXUIElement {
    guard let pid = pid_t(argument) else { fail("not a pid: \(argument)") }
    let app = AXUIElementCreateApplication(pid)
    AXUIElementSetMessagingTimeout(app, 1.0)
    return app
}

/// Breadth-first search for the element with `identifier`, bounded to 5000 nodes.
func find(_ identifier: String, under root: AXUIElement) -> AXUIElement? {
    var queue = [root]
    var seen = 0
    while !queue.isEmpty, seen < 5000 {
        let element = queue.removeFirst()
        seen += 1
        if attribute(element, kAXIdentifierAttribute) as? String == identifier {
            return element
        }
        queue.append(contentsOf: children(element))
    }
    return nil
}

/// Breadth-first search for the element with `role` and `title`, bounded to 5000 nodes.
func find(role: String, title: String, under root: AXUIElement) -> AXUIElement? {
    var queue = [root]
    var seen = 0
    while !queue.isEmpty, seen < 5000 {
        let element = queue.removeFirst()
        seen += 1
        if attribute(element, kAXRoleAttribute) as? String == role,
            attribute(element, kAXTitleAttribute) as? String == title
        {
            return element
        }
        queue.append(contentsOf: children(element))
    }
    return nil
}

/// The first item of `pid`'s status (extras) menu bar.
func statusItem(_ pid: String) -> AXUIElement? {
    guard let bar = attribute(application(pid), "AXExtrasMenuBar") else { return nil }
    // A CFTypeRef cast to a CF type cannot fail.
    return children(bar as! AXUIElement).first
}

func press(_ element: AXUIElement, _ what: String) {
    let status = AXUIElementPerformAction(element, kAXPressAction as CFString)
    guard status == .success else { fail("cannot press \(what): AXError \(status.rawValue)") }
}

/// Presses an element that opens a menu (a status item, a pop-up button). The press returns
/// only when the menu closes, so the call times out while it is open: that is success too.
func open(_ element: AXUIElement, _ what: String) {
    let status = AXUIElementPerformAction(element, kAXPressAction as CFString)
    guard status == .success || status == .cannotComplete else {
        fail("cannot open \(what): AXError \(status.rawValue)")
    }
}

/// Breadth-first search for the element titled `title` (a toolbar tab, a button), bounded
/// to 5000 nodes.
func find(title: String, under root: AXUIElement) -> AXUIElement? {
    var queue = [root]
    var seen = 0
    while !queue.isEmpty, seen < 5000 {
        let element = queue.removeFirst()
        seen += 1
        if attribute(element, kAXTitleAttribute) as? String == title,
            attribute(element, kAXRoleAttribute) as? String != kAXWindowRole
        {
            return element
        }
        queue.append(contentsOf: children(element))
    }
    return nil
}

/// Waits up to two seconds for the menu item `title` under `root` (a menu opens
/// asynchronously), then presses it.
func pick(_ title: String, under root: AXUIElement) {
    for _ in 0..<40 {
        if let item = find(role: kAXMenuItemRole, title: title, under: root) {
            press(item, "menu item \(title)")
            return
        }
        usleep(50_000)
    }
    fail("no menu item \(title)")
}

/// `pid`'s window with AXIdentifier or title `name`.
func window(_ pid: String, _ name: String) -> AXUIElement? {
    (attribute(application(pid), kAXWindowsAttribute) as? [AXUIElement] ?? []).first {
        attribute($0, kAXIdentifierAttribute) as? String == name
            || attribute($0, kAXTitleAttribute) as? String == name
    }
}

func currentLayout() -> TISInputSource {
    guard let source = TISCopyCurrentKeyboardLayoutInputSource()?.takeRetainedValue() else {
        fail("no current keyboard layout")
    }
    return source
}

func property(_ source: TISInputSource, _ key: CFString) -> UnsafeMutableRawPointer? {
    TISGetInputSourceProperty(source, key)
}

/// The character the key `code` types with `modifiers` (Carbon modifier bits >> 8) in `layout`.
func translate(_ layout: UnsafePointer<UCKeyboardLayout>, _ code: UInt16, _ modifiers: UInt32) -> String? {
    var deadKeys: UInt32 = 0
    var length = 0
    var buffer = [UniChar](repeating: 0, count: 4)
    let status = UCKeyTranslate(
        layout, code, UInt16(kUCKeyActionDown), modifiers, UInt32(LMGetKbdType()),
        OptionBits(kUCKeyTranslateNoDeadKeysBit), &deadKeys, buffer.count, &length, &buffer)
    guard status == noErr, length > 0 else { return nil }
    return String(utf16CodeUnits: buffer, count: length)
}

/// The key code and Shift state that type `character` in the current layout.
func keyFor(_ character: Character) -> (UInt16, Bool) {
    guard let data = property(currentLayout(), kTISPropertyUnicodeKeyLayoutData) else {
        fail("the current input source has no key layout")
    }
    let bytes = Unmanaged<CFData>.fromOpaque(data).takeUnretainedValue()
    guard let base = CFDataGetBytePtr(bytes) else { fail("empty key layout") }
    let layout = UnsafeRawPointer(base).assumingMemoryBound(to: UCKeyboardLayout.self)
    for shift in [false, true] {
        let modifiers: UInt32 = shift ? UInt32(shiftKey >> 8) : 0
        for code in UInt16(0)..<UInt16(128) where translate(layout, code, modifiers) == String(character) {
            return (code, shift)
        }
    }
    fail("no key types \(character) in the current layout")
}

/// The input source with id `id`, among the enabled ones or, with `includeDisabled`, all.
func inputSource(_ id: String, includeDisabled: Bool) -> TISInputSource {
    let filter = [kTISPropertyInputSourceID: id] as CFDictionary
    guard let list = TISCreateInputSourceList(filter, includeDisabled)?.takeRetainedValue() as? [TISInputSource],
        let source = list.first
    else { fail("no input source \(id)") }
    return source
}

let named: [String: UInt16] = [
    "escape": 53, "return": 36, "tab": 48, "space": 49, "backspace": 51,
    "left": 123, "right": 124, "down": 125, "up": 126,
]

/// Modifier flags and the left-hand key that sets each one.
let modifierKeys: [(CGEventFlags, UInt16)] = [
    (.maskControl, 59), (.maskAlternate, 58), (.maskShift, 56), (.maskCommand, 55),
]

func send(_ code: UInt16, down: Bool, _ flags: CGEventFlags) {
    let source = CGEventSource(stateID: .hidSystemState)
    guard let event = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: down) else {
        fail("cannot create a key event")
    }
    event.flags = flags
    event.post(tap: .cghidEventTap)
    usleep(15_000)
}

/// Presses key `code` with `flags` held. The modifier keys themselves go down first and up
/// last, as on a real keyboard, so the system's modifier state is clean again afterwards and
/// the next key types its plain character.
func post(_ code: UInt16, _ flags: CGEventFlags) {
    let held = modifierKeys.filter { flags.contains($0.0) }
    var current: CGEventFlags = []
    for (flag, key) in held {
        current.insert(flag)
        send(key, down: true, current)
    }
    send(code, down: true, flags)
    send(code, down: false, flags)
    for (flag, key) in held.reversed() {
        current.remove(flag)
        send(key, down: false, current)
    }
    usleep(25_000)
}

func chord(_ spec: String) -> (UInt16, CGEventFlags) {
    var flags: CGEventFlags = []
    var key = ""
    for part in spec.lowercased().split(separator: "+").map(String.init) {
        switch part {
        case "ctrl": flags.insert(.maskControl)
        case "alt": flags.insert(.maskAlternate)
        case "shift": flags.insert(.maskShift)
        case "meta", "cmd": flags.insert(.maskCommand)
        default: key = part
        }
    }
    if let code = named[key] { return (code, flags) }
    guard key.count == 1, let character = key.first else { fail("unknown key \(spec)") }
    let (code, shift) = keyFor(character)
    if shift { flags.insert(.maskShift) }
    return (code, flags)
}

func waitFrontmost(_ pid: pid_t) -> Bool {
    for _ in 0..<100 {
        if NSWorkspace.shared.frontmostApplication?.processIdentifier == pid { return true }
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.02))
    }
    return false
}

let arguments = Array(CommandLine.arguments.dropFirst())
guard let command = arguments.first else { fail("usage: mac_probe <command> [arguments]") }
switch (command, arguments.count) {
case ("trusted", 1):
    exit(AXIsProcessTrusted() ? 0 : 1)
case ("front", 2):
    let app = application(arguments[1])
    AXUIElementSetAttributeValue(app, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
    if let window = (attribute(app, kAXWindowsAttribute) as? [AXUIElement])?.first {
        AXUIElementPerformAction(window, kAXRaiseAction as CFString)
        AXUIElementSetAttributeValue(window, kAXMainAttribute as CFString, kCFBooleanTrue)
    }
    exit(waitFrontmost(pid_t(arguments[1]) ?? 0) ? 0 : 1)
case ("frontmost", 1):
    print(NSWorkspace.shared.frontmostApplication?.processIdentifier ?? 0)
case ("value", 3):
    guard let element = find(arguments[2], under: application(arguments[1])) else {
        fail("no element \(arguments[2])")
    }
    print(attribute(element, kAXValueAttribute).map { "\($0)" } ?? "")
case ("windows", 2):
    var value: CFTypeRef?
    let status = AXUIElementCopyAttributeValue(
        application(arguments[1]), kAXWindowsAttribute as CFString, &value)
    guard status == .success, let windows = value as? [AXUIElement] else {
        fail("cannot list the windows of \(arguments[1]): AXError \(status.rawValue)")
    }
    print(windows.count)
case ("panels", 2):
    guard let pid = Int(arguments[1]) else { fail("not a pid: \(arguments[1])") }
    let level = Int(CGWindowLevelForKey(.screenSaverWindow))
    let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
    print(windows.filter {
        $0[kCGWindowOwnerPID as String] as? Int == pid && $0[kCGWindowLayer as String] as? Int ?? 0 >= level
    }.count)
case ("covering", 2):
    guard let pid = Int(arguments[1]) else { fail("not a pid: \(arguments[1])") }
    // Front to back, so everything listed before the app's window is in front of it.
    let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
        as? [[String: Any]] ?? []
    func bounds(_ window: [String: Any]) -> CGRect {
        guard let dictionary = window[kCGWindowBounds as String] as? NSDictionary,
            let rect = CGRect(dictionaryRepresentation: dictionary)
        else { return .null }
        return rect
    }
    guard let index = windows.firstIndex(where: { $0[kCGWindowOwnerPID as String] as? Int == pid }) else {
        fail("\(pid) has no window on screen")
    }
    let target = bounds(windows[index])
    // The pointer itself is a Window Server window at the cursor level while it is drawn in
    // software (after typing, for example); it takes no clicks.
    let cursorLevel = Int(CGWindowLevelForKey(.cursorWindow))
    for window in windows[..<index] where bounds(window).intersects(target) {
        let layer = window[kCGWindowLayer as String] as? Int ?? 0
        if layer == cursorLevel { continue }
        let owner = window[kCGWindowOwnerName as String] as? String ?? "?"
        let ownerPid = window[kCGWindowOwnerPID as String] as? Int ?? 0
        let frame = bounds(window)
        print("\(owner) (pid \(ownerPid), layer \(layer), \(Int(frame.minX)),\(Int(frame.minY)) \(Int(frame.width))x\(Int(frame.height)))")
    }
case ("type", 2):
    for character in arguments[1] {
        let (code, shift) = keyFor(character)
        post(code, shift ? .maskShift : [])
    }
case ("key", _) where arguments.count > 1:
    for spec in arguments.dropFirst() {
        let (code, flags) = chord(spec)
        post(code, flags)
    }
case ("layout", 1):
    guard let id = property(currentLayout(), kTISPropertyInputSourceID) else { fail("no id") }
    print(Unmanaged<CFString>.fromOpaque(id).takeUnretainedValue())
case ("select-layout", 2):
    let source = inputSource(arguments[1], includeDisabled: true)
    TISEnableInputSource(source)
    guard TISSelectInputSource(source) == noErr else { fail("cannot select \(arguments[1])") }
case ("layout-enabled", 2):
    let filter = [kTISPropertyInputSourceID: arguments[1]] as CFDictionary
    let list = TISCreateInputSourceList(filter, false)?.takeRetainedValue() as? [TISInputSource]
    exit(list?.isEmpty == false ? 0 : 1)
case ("disable-layout", 2):
    guard TISDisableInputSource(inputSource(arguments[1], includeDisabled: true)) == noErr else {
        fail("cannot disable \(arguments[1])")
    }
case ("status-item", 2):
    guard let item = statusItem(arguments[1]) else { fail("\(arguments[1]) has no status item") }
    print(attribute(item, kAXDescriptionAttribute) as? String ?? attribute(item, kAXTitleAttribute) as? String ?? "")
case ("menu", 3):
    guard let item = statusItem(arguments[1]) else { fail("\(arguments[1]) has no status item") }
    open(item, "the status item's menu")
    pick(arguments[2], under: item)
case ("press", 3):
    guard let element = find(arguments[2], under: application(arguments[1])) else {
        fail("no element \(arguments[2])")
    }
    press(element, arguments[2])
case ("press-title", 3):
    guard let element = find(title: arguments[2], under: application(arguments[1])) else {
        fail("no element titled \(arguments[2])")
    }
    press(element, arguments[2])
case ("choose", 4):
    guard let popup = find(arguments[2], under: application(arguments[1])) else {
        fail("no element \(arguments[2])")
    }
    open(popup, arguments[2])
    pick(arguments[3], under: popup)
case ("window", 3):
    exit(window(arguments[1], arguments[2]) == nil ? 1 : 0)
case ("titles", 2):
    for found in attribute(application(arguments[1]), kAXWindowsAttribute) as? [AXUIElement] ?? [] {
        let title = attribute(found, kAXTitleAttribute) as? String ?? ""
        let id = attribute(found, kAXIdentifierAttribute) as? String ?? ""
        let size = attribute(found, kAXSizeAttribute).map { "\($0)" } ?? ""
        print("\(title)\t\(id)\t\(size)\t\(children(found).count) children")
    }
case ("close", 3):
    guard let found = window(arguments[1], arguments[2]),
        let button = attribute(found, kAXCloseButtonAttribute)
    else { fail("no window \(arguments[2]) with a close button") }
    // A CFTypeRef cast to a CF type cannot fail; `as?` would only warn about that.
    press(button as! AXUIElement, "the close button")
default:
    fail("unknown command: \(arguments.joined(separator: " "))")
}
