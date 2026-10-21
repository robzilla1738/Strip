import AppKit

let arguments = CommandLine.arguments
func flag(_ name: String) -> Bool { arguments.contains(name) }
func value(_ name: String, fallback: String) -> String {
    guard let index = arguments.firstIndex(of: name), arguments.indices.contains(index + 1) else {
        return fallback
    }
    return arguments[index + 1]
}

let count = Int(value("--count", fallback: "1")) ?? 1
let titlePrefix = value("--title", fallback: "Window")
let fixed = flag("--fixed-size")
let dialog = flag("--dialog")
let background = flag("--background")
let keyLog = value("--key-log", fallback: "")

let app = NSApplication.shared
app.setActivationPolicy(background ? .accessory : .regular)

/// Borderless so the window server will place it at a 5 pt corner. Titled windows
/// on this OS keep their title bar inside the visible frame. The standard-window
/// subrole is what the engine's auto-float rule looks for.
final class StripTestWindow: NSWindow {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}

final class Delegate: NSObject, NSApplicationDelegate {
    var windows: [NSWindow] = []

    func applicationDidFinishLaunching(_ notification: Notification) {
        for index in 0..<count {
            let origin = NSPoint(x: 80 + CGFloat(index) * 30, y: 80 + CGFloat(index) * 24)
            let size = NSSize(width: 480, height: 320)
            let window: NSWindow
            if dialog {
                window = NSPanel(
                    contentRect: NSRect(origin: origin, size: size),
                    styleMask: [.titled, .closable, .utilityWindow],
                    backing: .buffered,
                    defer: false
                )
            } else {
                let created = StripTestWindow(
                    contentRect: NSRect(origin: origin, size: size),
                    styleMask: [.borderless, .resizable],
                    backing: .buffered,
                    defer: false
                )
                created.setAccessibilitySubrole(.standardWindow)
                window = created
            }
            window.title = "\(titlePrefix) \(index + 1)"
            if fixed {
                window.contentMinSize = size
                window.contentMaxSize = size
                window.styleMask.remove(.resizable)
            }
            let field = NSTextField(string: window.title)
            field.frame = NSRect(x: 20, y: 20, width: 220, height: 24)
            window.contentView?.addSubview(field)
            window.makeKeyAndOrderFront(nil)
            window.makeFirstResponder(field)
            windows.append(window)
        }
        if !keyLog.isEmpty {
            NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
                let line = "\(event.keyCode)\n"
                if let data = line.data(using: .utf8) {
                    if FileManager.default.fileExists(atPath: keyLog),
                       let handle = FileHandle(forWritingAtPath: keyLog) {
                        handle.seekToEndOfFile()
                        handle.write(data)
                        try? handle.close()
                    } else {
                        FileManager.default.createFile(atPath: keyLog, contents: data)
                    }
                }
                return event
            }
        }
        if !background {
            app.activate(ignoringOtherApps: true)
        }
    }
}

let delegate = Delegate()
app.delegate = delegate
app.run()
