// capture <input source id> : prints "code<TAB>base<TAB>shift" (hex code points, - for none)
import Carbon

let id = CommandLine.arguments[1]
let filter = [kTISPropertyInputSourceID as String: id] as CFDictionary
guard let list = TISCreateInputSourceList(filter, true)?.takeRetainedValue() as? [TISInputSource],
    let source = list.first,
    let raw = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData)
else {
    FileHandle.standardError.write(Data("no layout \(id)\n".utf8))
    exit(1)
}
let data = Unmanaged<CFData>.fromOpaque(raw).takeUnretainedValue()
let layout = UnsafeRawPointer(CFDataGetBytePtr(data)!).assumingMemoryBound(to: UCKeyboardLayout.self)

func translate(_ code: UInt16, _ modifiers: UInt32) -> String {
    var dead: UInt32 = 0
    var length = 0
    var buffer = [UniChar](repeating: 0, count: 4)
    let status = UCKeyTranslate(
        layout, code, UInt16(kUCKeyActionDown), modifiers, UInt32(LMGetKbdType()),
        OptionBits(kUCKeyTranslateNoDeadKeysMask), &dead, buffer.count, &length, &buffer)
    guard status == noErr, length == 1, let scalar = Unicode.Scalar(buffer[0]),
        !CharacterSet.controlCharacters.contains(scalar)
    else { return "-" }
    return String(format: "%04x", buffer[0])
}

print("# \(id): key code, character alone, character with Shift (UTF-16 hex, - for none)")
for code in UInt16(0)..<UInt16(128) {
    let base = translate(code, 0)
    let shift = translate(code, UInt32(shiftKey >> 8))
    if base != "-" || shift != "-" {
        print("\(code)\t\(base)\t\(shift)")
    }
}
