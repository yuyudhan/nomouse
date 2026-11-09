// Draws nomouse's app icon into an .iconset directory for `iconutil -c icns`.
//
// Usage: swift packaging/macos/icon.swift <out.iconset>
//
// The icon is a hint label as the overlay draws it by default (`#ffd75f` box, `#1c1c1c` text
// and border) on a dark rounded square, so the app looks like what it puts on screen.
import AppKit

let arguments = CommandLine.arguments
guard arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: icon.swift <out.iconset>\n".utf8))
    exit(2)
}
let directory = URL(fileURLWithPath: arguments[1])

func colour(_ hex: UInt32) -> NSColor {
    NSColor(
        srgbRed: CGFloat((hex >> 16) & 0xff) / 255,
        green: CGFloat((hex >> 8) & 0xff) / 255,
        blue: CGFloat(hex & 0xff) / 255,
        alpha: 1)
}

/// Renders the icon at `pixels` square into a PNG.
func render(_ pixels: Int) -> Data? {
    guard
        let bitmap = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8,
            samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
            bytesPerRow: 0, bitsPerPixel: 0),
        let context = NSGraphicsContext(bitmapImageRep: bitmap)
    else { return nil }
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = context
    let size = CGFloat(pixels)
    // macOS icon grid: the shape fills about 80% of the canvas, corner radius about 22% of it.
    let inset = size * 0.1
    let tile = NSRect(x: inset, y: inset, width: size - 2 * inset, height: size - 2 * inset)
    colour(0x1c1c1c).setFill()
    NSBezierPath(roundedRect: tile, xRadius: tile.width * 0.225, yRadius: tile.width * 0.225).fill()

    let label = NSRect(
        x: size * 0.22, y: size * 0.33, width: size * 0.56, height: size * 0.34)
    let box = NSBezierPath(roundedRect: label, xRadius: size * 0.05, yRadius: size * 0.05)
    colour(0xffd75f).setFill()
    box.fill()
    colour(0x1c1c1c).setStroke()
    box.lineWidth = max(1, size * 0.012)
    box.stroke()

    let font = NSFont.monospacedSystemFont(ofSize: size * 0.24, weight: .bold)
    let text = NSAttributedString(
        string: "NM", attributes: [.font: font, .foregroundColor: colour(0x1c1c1c)])
    let measured = text.size()
    text.draw(at: NSPoint(x: label.midX - measured.width / 2, y: label.midY - measured.height / 2))
    NSGraphicsContext.restoreGraphicsState()
    return bitmap.representation(using: .png, properties: [:])
}

do {
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    for points in [16, 32, 128, 256, 512] {
        for scale in [1, 2] {
            let name = scale == 1 ? "icon_\(points)x\(points).png" : "icon_\(points)x\(points)@2x.png"
            guard let png = render(points * scale) else {
                FileHandle.standardError.write(Data("icon.swift: cannot draw \(name)\n".utf8))
                exit(1)
            }
            try png.write(to: directory.appendingPathComponent(name))
        }
    }
} catch {
    FileHandle.standardError.write(Data("icon.swift: \(error)\n".utf8))
    exit(1)
}
