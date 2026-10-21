import Foundation

struct RuleRecord: Equatable {
    var bundleID = ""
    var title = ""
    var role = ""
    var subrole = ""
    var floating = false
    var slot: Int?
}

struct ConfigDocument: Equatable {
    var launchAtLogin = true
    var startManagingOnLaunch = true
    var showInDock = true
    var shortcutSetVersion = 1
    var presetWidths = [0.25, 0.33, 0.5, 0.66, 0.75]
    var defaultColumnWidth = 0.5
    var widthStep = 0.10
    var gapInner = 8.0
    var gapOuter = 8.0
    var dockAware = true
    var centerMode = "never"
    var keepOriginalHeight = false
    var animationDurationMs = 250
    var animationDamping = 0.9
    var swipeFingers = 4
    var swipeNatural = true
    var swipeSensitivity = 1.5
    var swipeDeceleration = 4.0
    var focusFollowsMouse = false
    var mouseFollowsFocus = false
    var dragToReorder = true
    var showIndicator = true
    var focusBorder = false
    var focusBorderWidth = 4.0
    var focusBorderColor = "#4C8DFF"
    var focusBorderRadius = 10.0
    var dimInactive = false
    var dimInactiveOpacity = 0.25
    var ignoredDisplays: [String] = []
    var rules: [RuleRecord] = [RuleRecord(bundleID: "com.apple.systempreferences", floating: true)]
    var bindings: [String: [String]] = ShortcutCodec.defaults

    static func load(from url: URL) -> ConfigDocument {
        guard let text = try? String(contentsOf: url, encoding: .utf8) else { return ConfigDocument() }
        return parse(text)
    }

    static func write(_ document: ConfigDocument, to url: URL) throws {
        let directory = url.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let temporary = directory.appendingPathComponent(".config-\(UUID().uuidString).toml")
        try document.rendered().write(to: temporary, atomically: false, encoding: .utf8)
        if FileManager.default.fileExists(atPath: url.path) {
            _ = try FileManager.default.replaceItemAt(url, withItemAt: temporary)
        } else {
            try FileManager.default.moveItem(at: temporary, to: url)
        }
    }

    func rendered() -> String {
        var lines: [String] = []
        lines.append("[general]")
        lines.append("launch_at_login = \(launchAtLogin)")
        lines.append("start_managing_on_launch = \(startManagingOnLaunch)")
        lines.append("show_in_dock = \(showInDock)")
        lines.append("shortcut_set_version = \(shortcutSetVersion)")
        lines.append("")
        lines.append("[layout]")
        lines.append("preset_column_widths = \(numberArray(presetWidths))")
        lines.append("default_column_width = \(number(defaultColumnWidth))")
        lines.append("width_step = \(number(widthStep))")
        lines.append("gap_inner = \(number(gapInner))")
        lines.append("gap_outer = \(number(gapOuter))")
        lines.append("dock_aware = \(dockAware)")
        lines.append("center_focused_column = \(tomlString(centerMode))")
        lines.append("keep_original_height = \(keepOriginalHeight)")
        lines.append("")
        lines.append("[animation]")
        lines.append("animation_duration_ms = \(animationDurationMs)")
        lines.append("animation_damping = \(number(animationDamping))")
        lines.append("")
        lines.append("[swipe]")
        lines.append("swipe_fingers = \(swipeFingers)")
        lines.append("swipe_natural = \(swipeNatural)")
        lines.append("swipe_sensitivity = \(number(swipeSensitivity))")
        lines.append("swipe_deceleration = \(number(swipeDeceleration))")
        lines.append("")
        lines.append("[focus]")
        lines.append("focus_follows_mouse = \(focusFollowsMouse)")
        lines.append("mouse_follows_focus = \(mouseFollowsFocus)")
        lines.append("drag_to_reorder = \(dragToReorder)")
        lines.append("")
        lines.append("[appearance]")
        lines.append("show_indicator = \(showIndicator)")
        lines.append("focus_border = \(focusBorder)")
        lines.append("focus_border_width = \(number(focusBorderWidth))")
        lines.append("focus_border_color = \(tomlString(focusBorderColor))")
        lines.append("focus_border_radius = \(number(focusBorderRadius))")
        lines.append("dim_inactive = \(dimInactive)")
        lines.append("dim_inactive_opacity = \(number(dimInactiveOpacity))")
        lines.append("")
        lines.append("[displays]")
        lines.append("ignored_displays = \(stringArray(ignoredDisplays))")
        lines.append("")
        for rule in rules {
            lines.append("[[rules]]")
            if !rule.bundleID.isEmpty { lines.append("bundle_id = \(tomlString(rule.bundleID))") }
            if !rule.title.isEmpty { lines.append("title = \(tomlString(rule.title))") }
            if !rule.role.isEmpty { lines.append("role = \(tomlString(rule.role))") }
            if !rule.subrole.isEmpty { lines.append("subrole = \(tomlString(rule.subrole))") }
            lines.append("floating = \(rule.floating)")
            if let slot = rule.slot { lines.append("slot = \(slot)") }
            lines.append("")
        }
        lines.append("[bindings]")
        for action in bindings.keys.sorted() {
            lines.append("\(action) = \(stringArray(bindings[action] ?? []))")
        }
        lines.append("")
        return lines.joined(separator: "\n")
    }

    static func parse(_ text: String) -> ConfigDocument {
        var document = ConfigDocument()
        var section = ""
        var sawRules = false
        var current: RuleRecord?
        func flush() {
            if let current {
                document.rules.append(current)
            }
            current = nil
        }
        if !text.contains("[[rules]]") {
            // Keep the default rule until a rules table appears.
        } else {
            document.rules = []
        }
        for raw in text.split(separator: "\n", omittingEmptySubsequences: false) {
            let line = stripComment(String(raw)).trimmingCharacters(in: .whitespaces)
            if line.isEmpty { continue }
            if line == "[[rules]]" {
                if !sawRules {
                    sawRules = true
                    document.rules = []
                }
                flush()
                section = "rules"
                current = RuleRecord()
                continue
            }
            if line.hasPrefix("[") && line.hasSuffix("]") {
                flush()
                section = String(line.dropFirst().dropLast())
                continue
            }
            guard let separator = line.firstIndex(of: "=") else { continue }
            let key = line[..<separator].trimmingCharacters(in: .whitespaces)
            let value = line[line.index(after: separator)...].trimmingCharacters(in: .whitespaces)
            if section == "rules" {
                assignRule(&current, key: String(key), value: String(value))
            } else {
                assign(&document, section: section, key: String(key), value: String(value))
            }
        }
        flush()
        return document
    }

    /// Every control writes a value that is not the factory default.
    func withEveryControlChanged() -> ConfigDocument {
        var edited = self
        edited.launchAtLogin = false
        edited.startManagingOnLaunch = false
        edited.showInDock = false
        edited.shortcutSetVersion = 1
        edited.presetWidths = [0.2, 0.4, 0.8]
        edited.defaultColumnWidth = 0.4
        edited.widthStep = 0.05
        edited.gapInner = 16
        edited.gapOuter = 12
        edited.dockAware = false
        edited.centerMode = "always"
        edited.keepOriginalHeight = true
        edited.animationDurationMs = 120
        edited.animationDamping = 1
        edited.swipeFingers = 3
        edited.swipeNatural = false
        edited.swipeSensitivity = 1.5
        edited.swipeDeceleration = 2
        edited.focusFollowsMouse = true
        edited.mouseFollowsFocus = true
        edited.dragToReorder = false
        edited.showIndicator = false
        edited.focusBorder = true
        edited.focusBorderWidth = 6
        edited.focusBorderColor = "#112233"
        edited.focusBorderRadius = 4
        edited.dimInactive = true
        edited.dimInactiveOpacity = 0.4
        edited.ignoredDisplays = ["DISPLAY-UUID"]
        edited.rules = [RuleRecord(bundleID: "com.example.tool", title: "Find.*", role: "AXWindow", subrole: "AXDialog", floating: true, slot: 2)]
        edited.bindings["focus_left"] = ["ctrl+alt-x"]
        return edited
    }
}

private func assign(_ document: inout ConfigDocument, section: String, key: String, value: String) {
    switch (section, key) {
    case ("general", "launch_at_login"): document.launchAtLogin = bool(value)
    case ("general", "start_managing_on_launch"): document.startManagingOnLaunch = bool(value)
    case ("general", "show_in_dock"): document.showInDock = bool(value)
    case ("general", "shortcut_set_version"): document.shortcutSetVersion = integer(value) ?? document.shortcutSetVersion
    case ("layout", "preset_column_widths"): document.presetWidths = numbers(value)
    case ("layout", "default_column_width"): document.defaultColumnWidth = double(value) ?? document.defaultColumnWidth
    case ("layout", "width_step"): document.widthStep = double(value) ?? document.widthStep
    case ("layout", "gap_inner"): document.gapInner = double(value) ?? document.gapInner
    case ("layout", "gap_outer"): document.gapOuter = double(value) ?? document.gapOuter
    case ("layout", "dock_aware"): document.dockAware = bool(value)
    case ("layout", "center_focused_column"): document.centerMode = string(value)
    case ("layout", "keep_original_height"): document.keepOriginalHeight = bool(value)
    case ("animation", "animation_duration_ms"): document.animationDurationMs = integer(value) ?? document.animationDurationMs
    case ("animation", "animation_damping"): document.animationDamping = double(value) ?? document.animationDamping
    case ("swipe", "swipe_fingers"): document.swipeFingers = integer(value) ?? document.swipeFingers
    case ("swipe", "swipe_natural"): document.swipeNatural = bool(value)
    case ("swipe", "swipe_sensitivity"): document.swipeSensitivity = double(value) ?? document.swipeSensitivity
    case ("swipe", "swipe_deceleration"): document.swipeDeceleration = double(value) ?? document.swipeDeceleration
    case ("focus", "focus_follows_mouse"): document.focusFollowsMouse = bool(value)
    case ("focus", "mouse_follows_focus"): document.mouseFollowsFocus = bool(value)
    case ("focus", "drag_to_reorder"): document.dragToReorder = bool(value)
    case ("appearance", "show_indicator"): document.showIndicator = bool(value)
    case ("appearance", "focus_border"): document.focusBorder = bool(value)
    case ("appearance", "focus_border_width"): document.focusBorderWidth = double(value) ?? document.focusBorderWidth
    case ("appearance", "focus_border_color"): document.focusBorderColor = string(value)
    case ("appearance", "focus_border_radius"): document.focusBorderRadius = double(value) ?? document.focusBorderRadius
    case ("appearance", "dim_inactive"): document.dimInactive = bool(value)
    case ("appearance", "dim_inactive_opacity"): document.dimInactiveOpacity = double(value) ?? document.dimInactiveOpacity
    case ("displays", "ignored_displays"): document.ignoredDisplays = strings(value)
    case ("bindings", _): document.bindings[key] = strings(value)
    default: break
    }
}

private func assignRule(_ rule: inout RuleRecord?, key: String, value: String) {
    guard rule != nil else { return }
    switch key {
    case "bundle_id": rule?.bundleID = string(value)
    case "title": rule?.title = string(value)
    case "role": rule?.role = string(value)
    case "subrole": rule?.subrole = string(value)
    case "floating": rule?.floating = bool(value)
    case "slot": rule?.slot = integer(value)
    default: break
    }
}

private func stripComment(_ line: String) -> String {
    var quoted = false
    var escaped = false
    var kept = ""
    for character in line {
        if escaped {
            kept.append(character)
            escaped = false
            continue
        }
        if character == "\\" && quoted {
            escaped = true
            kept.append(character)
            continue
        }
        if character == "\"" {
            quoted.toggle()
            kept.append(character)
            continue
        }
        if character == "#" && !quoted { break }
        kept.append(character)
    }
    return kept
}

private func bool(_ value: String) -> Bool { value == "true" }

private func integer(_ value: String) -> Int? { Int(value) }

private func double(_ value: String) -> Double? { Double(value) }

private func string(_ value: String) -> String {
    var text = value
    if text.hasPrefix("\""), text.hasSuffix("\""), text.count >= 2 {
        text = String(text.dropFirst().dropLast())
    }
    return text.replacingOccurrences(of: "\\\"", with: "\"").replacingOccurrences(of: "\\\\", with: "\\")
}

private func numbers(_ value: String) -> [Double] {
    arrayBody(value).compactMap(Double.init)
}

private func strings(_ value: String) -> [String] {
    arrayBody(value).map(string)
}

private func arrayBody(_ value: String) -> [String] {
    guard value.hasPrefix("["), value.hasSuffix("]") else { return [] }
    let body = value.dropFirst().dropLast()
    if body.trimmingCharacters(in: .whitespaces).isEmpty { return [] }
    return splitArray(String(body))
}

private func splitArray(_ body: String) -> [String] {
    var items: [String] = []
    var current = ""
    var quoted = false
    var escaped = false
    for character in body {
        if escaped {
            current.append(character)
            escaped = false
            continue
        }
        if character == "\\" && quoted {
            escaped = true
            current.append(character)
            continue
        }
        if character == "\"" {
            quoted.toggle()
            current.append(character)
            continue
        }
        if character == "," && !quoted {
            items.append(current.trimmingCharacters(in: .whitespaces))
            current = ""
            continue
        }
        current.append(character)
    }
    let tail = current.trimmingCharacters(in: .whitespaces)
    if !tail.isEmpty { items.append(tail) }
    return items
}

private func number(_ value: Double) -> String {
    if value == value.rounded(), abs(value) < 1_000_000 {
        return String(Int(value))
    }
    var text = String(format: "%.4f", value)
    while text.contains("."), text.hasSuffix("0") || text.hasSuffix(".") {
        text.removeLast()
    }
    return text
}

private func numberArray(_ values: [Double]) -> String {
    "[\(values.map(number).joined(separator: ", "))]"
}

private func stringArray(_ values: [String]) -> String {
    "[\(values.map(tomlString).joined(separator: ", "))]"
}

private func tomlString(_ value: String) -> String {
    let escaped = value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
    return "\"\(escaped)\""
}
