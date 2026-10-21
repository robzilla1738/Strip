import AppKit
import CoreGraphics

let args = CommandLine.arguments
guard args.count >= 3, let code = UInt16(args[1]) else {
    fputs("usage: post-key <keycode> <ctrl,alt,shift,cmd> [--pid N]\n", stderr)
    exit(1)
}
var flags = CGEventFlags()
for name in args[2].split(separator: ",") where !name.isEmpty {
    switch name {
    case "ctrl": flags.insert(.maskControl)
    case "alt": flags.insert(.maskAlternate)
    case "shift": flags.insert(.maskShift)
    case "cmd": flags.insert(.maskCommand)
    default:
        fputs("unknown flag \(name)\n", stderr)
        exit(1)
    }
}
if let index = args.firstIndex(of: "--pid"), args.indices.contains(index + 1), let pid = Int32(args[index + 1]) {
    NSRunningApplication(processIdentifier: pid)?.activate()
    Thread.sleep(forTimeInterval: 0.25)
}
let source = CGEventSource(stateID: .hidSystemState)
guard let down = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true),
      let up = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false) else {
    exit(1)
}
down.flags = flags
up.flags = flags
down.post(tap: .cghidEventTap)
up.post(tap: .cghidEventTap)
