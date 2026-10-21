import CoreGraphics
import Foundation

// Reads "<keycode> <ctrl,alt,shift,cmd>" lines and posts each as a key press.
setvbuf(stdout, nil, _IOLBF, 0)
let source = CGEventSource(stateID: .hidSystemState)
while let line = readLine() {
    let parts = line.split(separator: " ")
    guard parts.count >= 1, let code = UInt16(parts[0]) else { continue }
    var flags = CGEventFlags()
    for name in (parts.count > 1 ? parts[1].split(separator: ",") : []) {
        switch name {
        case "ctrl": flags.insert(.maskControl)
        case "alt": flags.insert(.maskAlternate)
        case "shift": flags.insert(.maskShift)
        case "cmd": flags.insert(.maskCommand)
        default: break
        }
    }
    if let down = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true),
       let up = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false) {
        down.flags = flags
        up.flags = flags
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
    }
    print("posted")
}
