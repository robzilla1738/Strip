import AppKit
import XCTest

final class SettingsTests: XCTestCase {
    func testConfigRoundTripValidatesInTheEngine() throws {
        let edited = ConfigDocument().withEveryControlChanged()
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("strip-config-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("config.toml")
        try ConfigDocument.write(edited, to: url)
        let parsed = ConfigDocument.parse(try String(contentsOf: url, encoding: .utf8))
        XCTAssertEqual(parsed, edited)
        let engine = try engineBinary()
        let process = Process()
        process.executableURL = engine
        process.arguments = ["--validate-config", url.path]
        let pipe = Pipe()
        process.standardError = pipe
        process.standardOutput = pipe
        try process.run()
        process.waitUntilExit()
        let output = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        XCTAssertEqual(process.terminationStatus, 0, output)
        try? FileManager.default.removeItem(at: directory)
    }

    func testRecorderMapsKeycode4WithControlOptionToH() {
        let wire = ShortcutCodec.wire(keycode: 4, ctrl: true, alt: true, shift: false, cmd: false)
        XCTAssertEqual(wire, "ctrl+alt-h")
        XCTAssertFalse(ShortcutCodec.translated(4).isEmpty)
    }

    func testShortcutHelpFormattingAndSymbols() {
        XCTAssertEqual(ShortcutCodec.display("ctrl+alt-left"), "⌃⌥←")
        XCTAssertEqual(ShortcutCodec.display("ctrl+alt-rightbracket"), "⌃⌥]")
        XCTAssertEqual(ShortcutCodec.formattedKeys(for: "focus_index"), "⌃⌥1 … ⌃⌥9")
        XCTAssertEqual(ShortcutCodec.title(for: "focus_left"), "Focus Left")
        for group in ShortcutCodec.groups {
            for action in group.1 {
                XCTAssertFalse(ShortcutCodec.title(for: action).isEmpty)
                XCTAssertFalse(ShortcutCodec.formattedKeys(for: action).isEmpty)
            }
        }
    }

    func testTileFocusFollowsPinnedColumns() {
        XCTAssertTrue(tileIsFocused(focus: "pinned_left", focusedColumn: 0, pin: .left, column: -1))
        XCTAssertFalse(tileIsFocused(focus: "pinned_left", focusedColumn: 0, pin: nil, column: 0))
        XCTAssertTrue(tileIsFocused(focus: "column", focusedColumn: 2, pin: nil, column: 2))
        XCTAssertFalse(tileIsFocused(focus: "none", focusedColumn: 0, pin: nil, column: 0))
        XCTAssertTrue(tileIsFocused(focus: nil, focusedColumn: 1, pin: nil, column: 1))
    }

    func testBorderColorParsesHex() {
        let color = color(hex: "#4C8DFF")
        var red: CGFloat = 0
        var green: CGFloat = 0
        var blue: CGFloat = 0
        var alpha: CGFloat = 0
        color.usingColorSpace(.sRGB)?.getRed(&red, green: &green, blue: &blue, alpha: &alpha)
        XCTAssertEqual(red, 76.0 / 255.0, accuracy: 0.01)
        XCTAssertEqual(green, 141.0 / 255.0, accuracy: 0.01)
        XCTAssertEqual(blue, 1, accuracy: 0.01)
    }

    func testConflictDetectionFlagsDuplicates() {
        var bindings = ShortcutCodec.defaults
        bindings["focus_right"] = ["ctrl+alt-h"]
        let conflicts = ShortcutCodec.conflicts(bindings)
        XCTAssertTrue(conflicts.contains("focus_left"))
        XCTAssertTrue(conflicts.contains("focus_right"))
        XCTAssertFalse(ShortcutCodec.conflicts(ShortcutCodec.defaults).contains("focus_left"))
    }

    func testIndicatorSnapshotShowsFiveTilesWithFocusAndPins() {
        let tiles = [
            IndicatorTile(title: "Pinned", bundleID: "", pid: 0, count: 1, focused: false, pin: .left, windowID: 1, columnIndex: 0),
            IndicatorTile(title: "Focus", bundleID: "", pid: 0, count: 2, focused: true, pin: nil, windowID: 2, columnIndex: 1),
            IndicatorTile(title: "Third", bundleID: "", pid: 0, count: 1, focused: false, pin: nil, windowID: 3, columnIndex: 2),
            IndicatorTile(title: "Fourth", bundleID: "", pid: 0, count: 3, focused: false, pin: nil, windowID: 4, columnIndex: 3),
            IndicatorTile(title: "End", bundleID: "", pid: 0, count: 1, focused: false, pin: .right, windowID: 5, columnIndex: 4),
        ]
        XCTAssertEqual(tiles.count, 5)
        XCTAssertEqual(IndicatorArt.frames(count: tiles.count).count, 5)
        let rep = IndicatorArt.render(tiles)
        var pillPixels = 0
        var accentPixels = 0
        var pinPixels = 0
        for y in 0..<rep.pixelsHigh {
            for x in 0..<rep.pixelsWide {
                guard let color = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                let spread = max(color.redComponent, color.greenComponent, color.blueComponent) - min(color.redComponent, color.greenComponent, color.blueComponent)
                if color.redComponent > 0.75, spread < 0.35, color.alphaComponent > 0.15, color.alphaComponent < 0.95 {
                    pillPixels += 1
                }
                if color.blueComponent > 0.7, color.blueComponent > color.redComponent + 0.25, color.alphaComponent > 0.8 {
                    accentPixels += 1
                }
                if color.redComponent > 0.85, color.greenComponent > 0.5, color.blueComponent < 0.5, color.alphaComponent > 0.8 {
                    pinPixels += 1
                }
            }
        }
        XCTAssertGreaterThan(pillPixels, 800, "focused tile should sit on a bright glass pill")
        XCTAssertLessThan(pillPixels, 2600, "only the focused tile is highlighted")
        XCTAssertGreaterThan(accentPixels, 5, "focused tile should carry an accent dot")
        XCTAssertLessThan(accentPixels, 300, "the accent is a dot and badge, not a filled tile")
        XCTAssertGreaterThan(pinPixels, 40, "pinned tiles should draw a pin glyph")
    }

    private func engineBinary() throws -> URL {
        let env = ProcessInfo.processInfo.environment
        var candidates: [URL] = []
        if let app = env["STRIP_APP_PATH"] {
            candidates.append(URL(fileURLWithPath: app).appendingPathComponent("Contents/MacOS/strip-engine"))
        }
        if let root = env["STRIP_REPO_ROOT"] {
            candidates.append(URL(fileURLWithPath: root).appendingPathComponent("engine/target/debug/strip-engine"))
        }
        if let match = candidates.first(where: { FileManager.default.fileExists(atPath: $0.path) }) {
            return match
        }
        throw NSError(domain: "settings", code: 1, userInfo: [NSLocalizedDescriptionKey: "strip-engine not found"])
    }
}
