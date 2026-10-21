import AppKit
import Combine
import SwiftUI

struct SetupCheck: Equatable, Identifiable {
    var id: String
    var title: String
    var passed: Bool
    var instruction: String
    var fixURL: String?
    var warnOnly: Bool
    var quit: [(name: String, bundleID: String)]

    static func == (lhs: SetupCheck, rhs: SetupCheck) -> Bool {
        lhs.id == rhs.id && lhs.passed == rhs.passed && lhs.fixURL == rhs.fixURL && lhs.warnOnly == rhs.warnOnly
            && lhs.quit.map(\.bundleID) == rhs.quit.map(\.bundleID)
    }
}

enum FixLink {
    static let accessibility = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
    static let trackpad = "x-apple.systempreferences:com.apple.Trackpad-Settings.extension"
    static let desktop = "x-apple.systempreferences:com.apple.Desktop-Settings.extension"
    static let displays = "x-apple.systempreferences:com.apple.Displays-Settings.extension"
}

enum MacSetup {
    static let conflictNames = [
        "Rectangle", "Magnet", "Loop", "BetterSnapTool", "OneMenu", "AeroSpace",
        "yabai", "Amethyst", "OmniWM", "Nehir", "Paneru", "Moom", "Divvy",
    ]

    static func accessibility(appTrusted: Bool, engineTrusted: Bool) -> SetupCheck {
        check(
            "accessibility",
            "Accessibility",
            appTrusted && engineTrusted,
            "Allow this app in Privacy & Security → Accessibility.",
            FixLink.accessibility
        )
    }

    static func swipeFour(gesture: Int?) -> SetupCheck {
        check(
            "swipe4",
            "Four-finger swipe",
            gesture == 0,
            "Swipe between full-screen applications: Off.",
            FixLink.trackpad
        )
    }

    static func swipeThree(horiz: Int?, drag: Int?) -> SetupCheck {
        check(
            "swipe3",
            "Three-finger swipe",
            horiz == 0 && drag == 0,
            "Swipe between full-screen applications: four fingers. Swipe between pages: two fingers. Three Finger Drag off.",
            FixLink.trackpad
        )
    }

    static func separateSpaces(spansDisplays: Bool) -> SetupCheck {
        check(
            "spaces",
            "Separate Spaces",
            !spansDisplays,
            "Turn on Displays have separate Spaces, then log out.",
            FixLink.desktop
        )
    }

    static func rearrangeSpaces(mru: Bool) -> SetupCheck {
        check(
            "mru",
            "Rearrange Spaces",
            !mru,
            "Turn off Automatically rearrange Spaces based on most recent use.",
            FixLink.desktop
        )
    }

    static func stageManager(enabled: Bool) -> SetupCheck {
        check(
            "stage",
            "Stage Manager",
            !enabled,
            "Turn Stage Manager off.",
            FixLink.desktop
        )
    }

    /// Nil below macOS 15. The tiling keys are not a conflict on earlier systems.
    static func tiling(edge: Bool, top: Bool, accelerator: Bool, major: Int) -> SetupCheck? {
        guard major >= 15 else { return nil }
        return check(
            "tiling",
            "macOS tiling",
            !edge && !top && !accelerator,
            "Turn off tiling by dragging windows to screen edges and the top.",
            FixLink.desktop
        )
    }

    static func arrangement(frames: [CGRect]) -> SetupCheck {
        var item = check(
            "arrangement",
            "Display arrangement",
            !sideBySide(frames),
            "Arrange displays above or below each other, not side by side.",
            FixLink.displays
        )
        item.warnOnly = true
        return item
    }

    static func conflicts(running: [(name: String, bundleID: String)]) -> SetupCheck {
        let hits = running.filter { isConflict(name: $0.name, bundle: $0.bundleID) }
        var item = check(
            "conflicts",
            "Other window managers",
            hits.isEmpty,
            hits.isEmpty ? "No conflicting window manager is running." : "Quit \(hits.map(\.name).joined(separator: ", ")).",
            nil
        )
        item.quit = hits
        return item
    }

    static func sideBySide(_ frames: [CGRect]) -> Bool {
        for (index, frame) in frames.enumerated() {
            for other in frames.dropFirst(index + 1) {
                let overlap = min(frame.maxY, other.maxY) - max(frame.minY, other.minY)
                if overlap <= 0 { continue }
                if abs(frame.maxX - other.minX) < 1 || abs(other.maxX - frame.minX) < 1 {
                    return true
                }
            }
        }
        return false
    }

    static func isConflict(name: String, bundle: String) -> Bool {
        let last = bundle.split(separator: ".").last.map(String.init)?.lowercased()
        return conflictNames.contains { token in
            name.lowercased() == token.lowercased() || last == token.lowercased()
        }
    }

    static var guide: String {
        """
        Accessibility — Privacy & Security → Accessibility. Fix opens \(FixLink.accessibility).
        Four-finger swipe — set Swipe between full-screen applications to Off. Fix opens \(FixLink.trackpad).
        Three-finger swipe — full-screen swipe on four fingers, page swipe on two fingers, Three Finger Drag off. Fix opens \(FixLink.trackpad).
        Separate Spaces — Displays have separate Spaces on, then log out. Fix opens \(FixLink.desktop).
        Rearrange Spaces — turn off automatic rearrangement. Fix opens \(FixLink.desktop).
        Stage Manager — turn it off. Fix opens \(FixLink.desktop).
        macOS tiling (15 and later) — turn off edge and top tiling. Fix opens \(FixLink.desktop).
        Display arrangement — stack displays vertically. This check only warns. Fix opens \(FixLink.displays).
        Other window managers — quit Rectangle, Magnet, Loop, BetterSnapTool, OneMenu, AeroSpace, yabai, Amethyst, OmniWM, Nehir, Paneru, Moom, or Divvy. The button quits the app.
        """
    }

    static func live(appTrusted: Bool, engineTrusted: Bool, fingers: Int) -> [SetupCheck] {
        var items = [accessibility(appTrusted: appTrusted, engineTrusted: engineTrusted)]
        if fingers == 4 {
            items.append(swipeFour(gesture: preferenceInt("TrackpadFourFingerHorizSwipeGesture", domains: trackpadDomains)))
        }
        if fingers == 3 {
            items.append(swipeThree(
                horiz: preferenceInt("TrackpadThreeFingerHorizSwipeGesture", domains: trackpadDomains),
                drag: preferenceInt("TrackpadThreeFingerDrag", domains: trackpadDomains)
            ))
        }
        items.append(separateSpaces(spansDisplays: preferenceBool("spans-displays", domain: "com.apple.spaces")))
        items.append(rearrangeSpaces(mru: preferenceBool("mru-spaces", domain: "com.apple.dock")))
        items.append(stageManager(enabled: preferenceBool("GloballyEnabled", domain: "com.apple.WindowManager")))
        if let tiling = tiling(
            edge: preferenceBool("EnableTilingByEdgeDrag", domain: "com.apple.WindowManager"),
            top: preferenceBool("EnableTopTilingByEdgeDrag", domain: "com.apple.WindowManager"),
            accelerator: preferenceBool("EnableTilingOptionAccelerator", domain: "com.apple.WindowManager"),
            major: ProcessInfo.processInfo.operatingSystemVersion.majorVersion
        ) {
            items.append(tiling)
        }
        items.append(arrangement(frames: NSScreen.screens.map(\.frame)))
        let running = NSWorkspace.shared.runningApplications.map {
            (name: $0.localizedName ?? "", bundleID: $0.bundleIdentifier ?? "")
        }
        items.append(conflicts(running: running))
        return items
    }

    private static let trackpadDomains = [
        "com.apple.AppleMultitouchTrackpad",
        "com.apple.driver.AppleBluetoothMultitouch.trackpad",
    ]

    private static func check(_ id: String, _ title: String, _ passed: Bool, _ instruction: String, _ url: String?) -> SetupCheck {
        SetupCheck(id: id, title: title, passed: passed, instruction: instruction, fixURL: url, warnOnly: false, quit: [])
    }
}

func preferenceInt(_ key: String, domains: [String]) -> Int? {
    let values = domains.compactMap { preferenceInt(key, domain: $0) }
    return values.max()
}

func preferenceInt(_ key: String, domain: String) -> Int? {
    if let value = CFPreferencesCopyAppValue(key as CFString, domain as CFString) {
        if let number = value as? NSNumber { return number.intValue }
        if let text = value as? String, let number = Int(text) { return number }
    }
    guard let text = defaultsText(key, domain: domain) else { return nil }
    return Int(text)
}

func preferenceBool(_ key: String, domain: String) -> Bool {
    if let value = CFPreferencesCopyAppValue(key as CFString, domain as CFString) {
        if let number = value as? NSNumber { return number.intValue != 0 }
        if let text = value as? String { return text == "1" || text == "true" }
    }
    guard let text = defaultsText(key, domain: domain) else { return false }
    return text == "1" || text == "true"
}

private func defaultsText(_ key: String, domain: String) -> String? {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/defaults")
    process.arguments = ["read", domain, key]
    let pipe = Pipe()
    process.standardOutput = pipe
    process.standardError = Pipe()
    do { try process.run() } catch { return nil }
    process.waitUntilExit()
    guard process.terminationStatus == 0 else { return nil }
    let text = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)?
        .trimmingCharacters(in: .whitespacesAndNewlines)
    if text?.isEmpty == true { return nil }
    return text
}

struct MacSetupList: View {
    @ObservedObject var model: AppModel
    @State private var checks: [SetupCheck] = []
    private let poll = Timer.publish(every: 5, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(checks) { item in
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: item.passed ? "checkmark.circle.fill" : (item.warnOnly ? "exclamationmark.triangle" : "xmark.circle"))
                        .foregroundStyle(item.passed ? .green : (item.warnOnly ? .yellow : .red))
                    VStack(alignment: .leading, spacing: 4) {
                        Text(item.title).font(.headline)
                        Text(item.instruction).font(.callout).foregroundStyle(.secondary)
                        if !item.passed, let url = item.fixURL {
                            Button(item.warnOnly ? "Show arrangement" : "Fix") {
                                NSWorkspace.shared.open(URL(string: url)!)
                            }
                        }
                        if !item.passed && !item.quit.isEmpty {
                            Button("Quit \(item.quit.map(\.name).joined(separator: ", "))") {
                                quit(item.quit)
                            }
                        }
                    }
                }
            }
        }
        .onAppear { refresh() }
        .onReceive(poll) { _ in refresh() }
    }

    private func refresh() {
        let fingers = model.settings?.document.swipeFingers ?? 4
        checks = MacSetup.live(appTrusted: AXIsProcessTrusted(), engineTrusted: model.axTrusted == true, fingers: fingers)
    }

    private func quit(_ apps: [(name: String, bundleID: String)]) {
        for app in NSWorkspace.shared.runningApplications {
            guard let bundle = app.bundleIdentifier, apps.contains(where: { $0.bundleID == bundle }) else { continue }
            app.terminate()
        }
    }
}
