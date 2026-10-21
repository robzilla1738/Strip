import AppKit
import Carbon

enum ShortcutCodec {
    static let keys: [(String, UInt16)] = [
        ("a", 0x00), ("s", 0x01), ("d", 0x02), ("f", 0x03), ("h", 0x04),
        ("g", 0x05), ("z", 0x06), ("x", 0x07), ("c", 0x08), ("v", 0x09),
        ("b", 0x0b), ("q", 0x0c), ("w", 0x0d), ("e", 0x0e), ("r", 0x0f),
        ("y", 0x10), ("t", 0x11), ("1", 0x12), ("2", 0x13), ("3", 0x14),
        ("4", 0x15), ("6", 0x16), ("5", 0x17), ("equal", 0x18), ("9", 0x19),
        ("7", 0x1a), ("minus", 0x1b), ("8", 0x1c), ("0", 0x1d),
        ("rightbracket", 0x1e), ("o", 0x1f), ("u", 0x20), ("leftbracket", 0x21),
        ("i", 0x22), ("p", 0x23), ("l", 0x25), ("j", 0x26), ("k", 0x28),
        ("n", 0x2d), ("m", 0x2e), ("left", 0x7b), ("right", 0x7c),
        ("down", 0x7d), ("up", 0x7e),
    ]

    static let groups: [(String, [String])] = [
        ("Focus", ["focus_left", "focus_right", "focus_up", "focus_down", "focus_first", "focus_last", "focus_index"]),
        ("Move", ["move_to_index", "move_left", "move_right", "move_up", "move_down", "move_first", "move_last"]),
        ("Size", ["cycle_width", "full_width", "center", "wider", "narrower", "taller", "shorter", "equalize_heights"]),
        ("Stack", ["stack_left", "stack_right", "unstack_right", "unstack_left", "toggle_tabbed"]),
        ("Window", ["toggle_float", "send_next_display", "pin_left", "pin_right", "stop"]),
    ]

    static var defaults: [String: [String]] {
        [
            "focus_left": ["ctrl+alt-h", "ctrl+alt-left"],
            "focus_right": ["ctrl+alt-l", "ctrl+alt-right"],
            "focus_up": ["ctrl+alt-k", "ctrl+alt-up"],
            "focus_down": ["ctrl+alt-j", "ctrl+alt-down"],
            "focus_first": ["ctrl+alt+cmd-left"],
            "focus_last": ["ctrl+alt+cmd-right"],
            "focus_index": ["ctrl+alt-1", "ctrl+alt-2", "ctrl+alt-3", "ctrl+alt-4", "ctrl+alt-5", "ctrl+alt-6", "ctrl+alt-7", "ctrl+alt-8", "ctrl+alt-9"],
            "move_to_index": ["ctrl+alt+shift-1", "ctrl+alt+shift-2", "ctrl+alt+shift-3", "ctrl+alt+shift-4", "ctrl+alt+shift-5", "ctrl+alt+shift-6", "ctrl+alt+shift-7", "ctrl+alt+shift-8", "ctrl+alt+shift-9"],
            "move_left": ["ctrl+alt+shift-h", "ctrl+alt+shift-left"],
            "move_right": ["ctrl+alt+shift-l", "ctrl+alt+shift-right"],
            "move_up": ["ctrl+alt+shift-k", "ctrl+alt+shift-up"],
            "move_down": ["ctrl+alt+shift-j", "ctrl+alt+shift-down"],
            "move_first": ["ctrl+alt+shift+cmd-left"],
            "move_last": ["ctrl+alt+shift+cmd-right"],
            "cycle_width": ["ctrl+alt-r"],
            "full_width": ["ctrl+alt-f"],
            "center": ["ctrl+alt-c"],
            "wider": ["ctrl+alt-equal"],
            "narrower": ["ctrl+alt-minus"],
            "taller": ["ctrl+alt+shift-equal"],
            "shorter": ["ctrl+alt+shift-minus"],
            "equalize_heights": ["ctrl+alt-0"],
            "stack_left": ["ctrl+alt-rightbracket"],
            "stack_right": ["ctrl+alt-leftbracket"],
            "unstack_right": ["ctrl+alt+shift-rightbracket"],
            "unstack_left": ["ctrl+alt+shift-leftbracket"],
            "toggle_tabbed": ["ctrl+alt+shift-t"],
            "toggle_float": ["ctrl+alt-t"],
            "send_next_display": ["ctrl+alt-n"],
            "pin_left": ["ctrl+alt-p"],
            "pin_right": ["ctrl+alt+shift-p"],
            "stop": ["ctrl+alt-q"],
        ]
    }

    static func name(for keycode: UInt16) -> String? {
        keys.first { $0.1 == keycode }?.0
    }

    static func wire(keycode: UInt16, ctrl: Bool, alt: Bool, shift: Bool, cmd: Bool) -> String? {
        guard let name = name(for: keycode) else { return nil }
        var mods: [String] = []
        if ctrl { mods.append("ctrl") }
        if alt { mods.append("alt") }
        if shift { mods.append("shift") }
        if cmd { mods.append("cmd") }
        return "\(mods.joined(separator: "+"))-\(name)"
    }

    static func conflicts(_ bindings: [String: [String]]) -> Set<String> {
        var owner: [String: String] = [:]
        var flagged: Set<String> = []
        for action in bindings.keys.sorted() {
            for key in bindings[action] ?? [] {
                if let previous = owner[key] {
                    flagged.insert(previous)
                    flagged.insert(action)
                } else {
                    owner[key] = action
                }
            }
        }
        return flagged
    }

    static func display(_ wire: String) -> String {
        guard let key = wire.split(separator: "-").last else { return wire }
        guard let code = keys.first(where: { $0.0 == key })?.1 else { return wire }
        let mods = wire.split(separator: "-").dropLast().joined(separator: "-")
        return label(
            keycode: code,
            ctrl: mods.contains("ctrl"),
            alt: mods.contains("alt"),
            shift: mods.contains("shift"),
            cmd: mods.contains("cmd")
        )
    }

    static func label(keycode: UInt16, ctrl: Bool, alt: Bool, shift: Bool, cmd: Bool) -> String {
        var prefix = ""
        if ctrl { prefix += "⌃" }
        if alt { prefix += "⌥" }
        if shift { prefix += "⇧" }
        if cmd { prefix += "⌘" }
        return prefix + translated(keycode)
    }

    static func translated(_ keycode: UInt16) -> String {
        if let layout = layoutLabel(keycode) { return layout }
        return name(for: keycode)?.uppercased() ?? "?"
    }

    private static func layoutLabel(_ keycode: UInt16) -> String? {
        guard let source = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue() else { return nil }
        guard let raw = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData) else { return nil }
        let data = Unmanaged<CFData>.fromOpaque(raw).takeUnretainedValue() as Data
        var dead: UInt32 = 0
        var chars = [UniChar](repeating: 0, count: 8)
        var length = 0
        let status = data.withUnsafeBytes { buffer -> OSStatus in
            guard let base = buffer.baseAddress?.assumingMemoryBound(to: UCKeyboardLayout.self) else { return -1 }
            return UCKeyTranslate(
                base,
                keycode,
                UInt16(kUCKeyActionDisplay),
                0,
                UInt32(LMGetKbdType()),
                OptionBits(kUCKeyTranslateNoDeadKeysBit),
                &dead,
                8,
                &length,
                &chars
            )
        }
        guard status == noErr, length > 0 else { return nil }
        let text = String(utf16CodeUnits: chars, count: length).trimmingCharacters(in: .whitespaces)
        return text.isEmpty ? nil : text.uppercased()
    }
}
