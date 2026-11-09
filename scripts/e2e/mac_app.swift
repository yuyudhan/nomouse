// The macOS end-to-end fixture: one window whose controls count what happens to them.
//
// Built with `swiftc` by scripts/e2e-macos.sh. Every counter is a static text with an
// accessibility identifier, so the script reads results through Accessibility
// (scripts/e2e/mac_probe.swift `value`), never from this process's output:
//
//   count.One .. count.Ten  left clicks on the buttons One..Ten (a double click adds two)
//   count.Context           left clicks on the Context button
//   menus                   times the Context button's context menu opened
//   drops                   drags released on the Target; the Source then moves onto it
//   first-row               index of the first visible row of the 200-row list
//   field                   the text field's contents
import AppKit

let names = ["One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten"]

/// A read-only label with an accessibility identifier, holding a counter or a state.
final class Counter {
    let field: NSTextField
    private(set) var value = 0

    init(_ identifier: String, in view: NSView, at origin: NSPoint) {
        field = NSTextField(labelWithString: "0")
        field.identifier = NSUserInterfaceItemIdentifier(identifier)
        field.setAccessibilityLabel(identifier)
        field.frame = NSRect(origin: origin, size: NSSize(width: 90, height: 18))
        view.addSubview(field)
    }

    func add(_ amount: Int = 1) {
        value += amount
        field.stringValue = String(value)
    }
}

/// A view that acts as a button for Accessibility, so nomouse labels it.
class PressableView: NSView {
    var title = ""

    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .button }
    override func accessibilityTitle() -> String? { title }
    override func accessibilityPerformPress() -> Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.controlAccentColor.withAlphaComponent(0.3).setFill()
        bounds.fill()
        (title as NSString).draw(at: NSPoint(x: 6, y: 6), withAttributes: nil)
    }
}

/// The drag source: follows the pointer while the left button is held, and lands on the
/// target when released over it.
final class DragSource: PressableView {
    weak var target: NSView?
    var onDrop: (() -> Void)?
    private var grabbedAt = NSPoint.zero

    override func mouseDown(with event: NSEvent) {
        grabbedAt = convert(event.locationInWindow, from: nil)
    }

    override func mouseDragged(with event: NSEvent) {
        guard let parent = superview else { return }
        let point = parent.convert(event.locationInWindow, from: nil)
        setFrameOrigin(NSPoint(x: point.x - grabbedAt.x, y: point.y - grabbedAt.y))
    }

    override func mouseUp(with event: NSEvent) {
        guard let parent = superview, let target else { return }
        let point = parent.convert(event.locationInWindow, from: nil)
        if target.frame.contains(point) {
            setFrameOrigin(target.frame.origin)
            onDrop?()
        }
    }
}

final class Fixture: NSObject, NSApplicationDelegate, NSTableViewDataSource, NSTableViewDelegate,
    NSMenuDelegate, NSTextFieldDelegate
{
    let window = NSWindow(
        contentRect: NSRect(x: 120, y: 120, width: 820, height: 620),
        styleMask: [.titled, .closable, .resizable],
        backing: .buffered,
        defer: false)
    var counters: [String: Counter] = [:]
    var menus: Counter?
    var drops: Counter?
    var firstRow: Counter?
    var fieldMirror: NSTextField?
    let list = NSTableView()

    func applicationDidFinishLaunching(_ notification: Notification) {
        window.title = "nomouse-e2e"
        guard let content = window.contentView else { return }
        for (index, name) in names.enumerated() {
            let column = CGFloat(index % 5)
            let row = CGFloat(index / 5)
            let button = NSButton(
                title: name, target: self, action: #selector(clicked(_:)))
            button.frame = NSRect(x: 20 + column * 110, y: 560 - row * 70, width: 100, height: 30)
            content.addSubview(button)
            counters[name] = Counter("count.\(name)", in: content, at: NSPoint(x: 25 + column * 110, y: 540 - row * 70))
        }
        let context = NSButton(title: "Context", target: self, action: #selector(clicked(_:)))
        context.frame = NSRect(x: 20, y: 400, width: 100, height: 30)
        let menu = NSMenu(title: "Context menu")
        menu.addItem(withTitle: "First item", action: nil, keyEquivalent: "")
        menu.addItem(withTitle: "Second item", action: nil, keyEquivalent: "")
        menu.delegate = self
        context.menu = menu
        content.addSubview(context)
        counters["Context"] = Counter("count.Context", in: content, at: NSPoint(x: 25, y: 380))
        menus = Counter("menus", in: content, at: NSPoint(x: 130, y: 406))

        let field = NSTextField(string: "")
        field.placeholderString = "Type here"
        field.setAccessibilityTitle("Text field")
        field.frame = NSRect(x: 240, y: 400, width: 200, height: 24)
        field.delegate = self
        content.addSubview(field)
        let mirror = NSTextField(labelWithString: "")
        mirror.identifier = NSUserInterfaceItemIdentifier("field")
        mirror.frame = NSRect(x: 450, y: 402, width: 150, height: 18)
        content.addSubview(mirror)
        fieldMirror = mirror

        let target = PressableView(frame: NSRect(x: 380, y: 240, width: 120, height: 60))
        target.title = "Target"
        let source = DragSource(frame: NSRect(x: 240, y: 240, width: 100, height: 40))
        source.title = "Source"
        source.target = target
        content.addSubview(target)
        content.addSubview(source)
        drops = Counter("drops", in: content, at: NSPoint(x: 240, y: 210))
        source.onDrop = { [weak self] in self?.drops?.add() }

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("row"))
        column.title = "Rows"
        column.width = 160
        column.isEditable = false
        list.addTableColumn(column)
        list.dataSource = self
        list.delegate = self
        let scroll = NSScrollView(frame: NSRect(x: 620, y: 40, width: 180, height: 360))
        scroll.documentView = list
        scroll.hasVerticalScroller = true
        content.addSubview(scroll)
        firstRow = Counter("first-row", in: content, at: NSPoint(x: 620, y: 410))
        scroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            self, selector: #selector(scrolled(_:)),
            name: NSView.boundsDidChangeNotification, object: scroll.contentView)

        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    @objc func clicked(_ sender: NSButton) {
        counters[sender.title]?.add()
    }

    @objc func scrolled(_ notification: Notification) {
        let visible = list.rows(in: list.visibleRect)
        guard let counter = firstRow else { return }
        counter.add(visible.location - counter.value)
    }

    func menuWillOpen(_ menu: NSMenu) {
        menus?.add()
    }

    func controlTextDidChange(_ notification: Notification) {
        guard let field = notification.object as? NSTextField else { return }
        fieldMirror?.stringValue = field.stringValue
    }

    func numberOfRows(in tableView: NSTableView) -> Int { 200 }

    func tableView(_ tableView: NSTableView, objectValueFor column: NSTableColumn?, row: Int) -> Any? {
        "Row \(row + 1)"
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

let app = NSApplication.shared
let fixture = Fixture()
app.delegate = fixture
app.setActivationPolicy(.regular)
app.run()
