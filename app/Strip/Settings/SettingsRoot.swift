import AppKit
import ServiceManagement
import SwiftUI

final class SettingsModel: ObservableObject {
    @Published var document: ConfigDocument
    let url: URL
    var onChange: (ConfigDocument) -> Void = { _ in }

    init(url: URL) {
        self.url = url
        document = ConfigDocument.load(from: url)
    }

    func update(_ body: (inout ConfigDocument) -> Void) {
        body(&document)
        try? ConfigDocument.write(document, to: url)
        onChange(document)
    }

    func useDefaultShortcuts() {
        update { document in
            document.bindings = ShortcutCodec.defaults
            document.shortcutSetVersion = 1
        }
    }
}

struct SettingsRoot: View {
    @ObservedObject var model: AppModel
    @ObservedObject var settings: SettingsModel
    @State private var section: SettingsSection = .general

    var body: some View {
        NavigationSplitView {
            List(SettingsSection.allCases, selection: $section) { item in
                Text(item.title).tag(item)
            }
            .navigationSplitViewColumnWidth(min: 160, ideal: 180)
        } detail: {
            ScrollView {
                sectionView
                    .padding(24)
                    .frame(maxWidth: 640, alignment: .leading)
            }
            .navigationTitle(section.title)
        }
        .frame(minWidth: 760, minHeight: 520)
        .background(.ultraThinMaterial)
    }

    @ViewBuilder private var sectionView: some View {
        switch section {
        case .general: GeneralSection(model: model, settings: settings, licensing: model.licensing)
        case .layout: LayoutSection(settings: settings)
        case .trackpad: TrackpadSection(settings: settings)
        case .appearance: AppearanceSection(settings: settings)
        case .displays: DisplaysSection(settings: settings)
        case .rules: RulesSection(settings: settings)
        case .shortcuts: ShortcutsSection(settings: settings)
        case .macSetup: MacSetupPage(model: model)
        case .about: AboutView()
        }
    }
}

enum SettingsSection: String, CaseIterable, Identifiable {
    case general, layout, trackpad, appearance, displays, rules, shortcuts, macSetup, about
    var id: String { rawValue }
    var title: String {
        switch self {
        case .general: return "General"
        case .layout: return "Layout"
        case .trackpad: return "Trackpad"
        case .appearance: return "Appearance"
        case .displays: return "Displays"
        case .rules: return "Rules"
        case .shortcuts: return "Shortcuts"
        case .macSetup: return "Mac Setup"
        case .about: return "About"
        }
    }
}

private struct GeneralSection: View {
    @ObservedObject var model: AppModel
    @ObservedObject var settings: SettingsModel
    @ObservedObject var licensing: LicenseController
    @State private var enteredKey = ""

    var body: some View {
        Form {
            Toggle("Launch at login", isOn: binding(\.launchAtLogin))
            Toggle("Start managing on launch", isOn: binding(\.startManagingOnLaunch))
            Toggle("Show in Dock", isOn: binding(\.showInDock))
            LabeledContent("License", value: licensing.phase.statusText)
            if case .licensed(let masked, let product) = licensing.phase {
                LabeledContent("Key", value: masked)
                LabeledContent("Product", value: product)
                Button("Deactivate") { licensing.deactivate(now: Date()) }
            }
            TextField("Enter key", text: $enteredKey)
            Button("Activate") { licensing.activate(enteredKey, now: Date()) }
            Button("Buy License") { licensing.openCheckout() }
            LabeledContent("Status", value: model.statusLine)
        }
        .formStyle(.grouped)
    }

    private func binding(_ key: WritableKeyPath<ConfigDocument, Bool>) -> Binding<Bool> {
        Binding(
            get: { settings.document[keyPath: key] },
            set: { value in settings.update { $0[keyPath: key] = value } }
        )
    }
}

private struct LayoutSection: View {
    @ObservedObject var settings: SettingsModel

    var body: some View {
        Form {
            TextField("Preset widths", text: widths)
            Slider(value: double(\.defaultColumnWidth), in: 0.1...1)
            LabeledContent("Default width", value: settings.document.defaultColumnWidth.formatted())
            Slider(value: double(\.widthStep), in: 0.01...1)
            Slider(value: double(\.gapInner), in: 0...64)
            LabeledContent("Inner gap", value: settings.document.gapInner.formatted())
            Slider(value: double(\.gapOuter), in: 0...64)
            Toggle("Dock aware", isOn: bool(\.dockAware))
            Picker("Center focused column", selection: string(\.centerMode)) {
                Text("Never").tag("never")
                Text("On overflow").tag("on_overflow")
                Text("Always").tag("always")
            }
            Toggle("Keep original height", isOn: bool(\.keepOriginalHeight))
        }
        .formStyle(.grouped)
    }

    private var widths: Binding<String> {
        Binding(
            get: { settings.document.presetWidths.map { number($0) }.joined(separator: ", ") },
            set: { text in
                let values = text.split(separator: ",").compactMap { Double($0.trimmingCharacters(in: .whitespaces)) }
                guard !values.isEmpty else { return }
                settings.update { $0.presetWidths = values }
            }
        )
    }

    private func double(_ key: WritableKeyPath<ConfigDocument, Double>) -> Binding<Double> {
        Binding(get: { settings.document[keyPath: key] }, set: { value in settings.update { $0[keyPath: key] = value } })
    }

    private func bool(_ key: WritableKeyPath<ConfigDocument, Bool>) -> Binding<Bool> {
        Binding(get: { settings.document[keyPath: key] }, set: { value in settings.update { $0[keyPath: key] = value } })
    }

    private func string(_ key: WritableKeyPath<ConfigDocument, String>) -> Binding<String> {
        Binding(get: { settings.document[keyPath: key] }, set: { value in settings.update { $0[keyPath: key] = value } })
    }
}

private struct TrackpadSection: View {
    @ObservedObject var settings: SettingsModel

    var body: some View {
        Form {
            Picker("Fingers", selection: fingers) {
                Text("Off").tag(0)
                Text("3").tag(3)
                Text("4").tag(4)
                Text("5").tag(5)
            }
            Toggle("Natural direction", isOn: bool(\.swipeNatural))
            Slider(value: double(\.swipeSensitivity), in: 0.2...3)
            Slider(value: double(\.swipeDeceleration), in: 0.5...12)
            if settings.document.swipeFingers == 4 {
                Text("Four-finger horizontal swipe should be off in Trackpad settings.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            if settings.document.swipeFingers == 3 {
                Text("Three-finger swipe and three-finger drag should be off.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }

    private var fingers: Binding<Int> {
        Binding(get: { settings.document.swipeFingers }, set: { value in settings.update { $0.swipeFingers = value } })
    }

    private func bool(_ key: WritableKeyPath<ConfigDocument, Bool>) -> Binding<Bool> {
        Binding(get: { settings.document[keyPath: key] }, set: { value in settings.update { $0[keyPath: key] = value } })
    }

    private func double(_ key: WritableKeyPath<ConfigDocument, Double>) -> Binding<Double> {
        Binding(get: { settings.document[keyPath: key] }, set: { value in settings.update { $0[keyPath: key] = value } })
    }
}

private struct AppearanceSection: View {
    @ObservedObject var settings: SettingsModel

    var body: some View {
        Form {
            Toggle("Show indicator", isOn: bool(\.showIndicator))
            Toggle("Focus border", isOn: bool(\.focusBorder))
            Slider(value: double(\.focusBorderWidth), in: 1...12)
            TextField("Border color", text: string(\.focusBorderColor))
            Slider(value: double(\.focusBorderRadius), in: 0...24)
            Toggle("Dim inactive", isOn: bool(\.dimInactive))
            Slider(value: double(\.dimInactiveOpacity), in: 0...0.8)
            Slider(value: duration, in: 0...1000)
            LabeledContent("Animation", value: "\(settings.document.animationDurationMs) ms")
            Slider(value: double(\.animationDamping), in: 0.1...1)
        }
        .formStyle(.grouped)
    }

    private var duration: Binding<Double> {
        Binding(
            get: { Double(settings.document.animationDurationMs) },
            set: { value in settings.update { $0.animationDurationMs = Int(value) } }
        )
    }

    private func bool(_ key: WritableKeyPath<ConfigDocument, Bool>) -> Binding<Bool> {
        Binding(get: { settings.document[keyPath: key] }, set: { value in settings.update { $0[keyPath: key] = value } })
    }

    private func double(_ key: WritableKeyPath<ConfigDocument, Double>) -> Binding<Double> {
        Binding(get: { settings.document[keyPath: key] }, set: { value in settings.update { $0[keyPath: key] = value } })
    }

    private func string(_ key: WritableKeyPath<ConfigDocument, String>) -> Binding<String> {
        Binding(get: { settings.document[keyPath: key] }, set: { value in settings.update { $0[keyPath: key] = value } })
    }
}

private struct DisplaysSection: View {
    @ObservedObject var settings: SettingsModel
    @State private var rows = DisplayCatalog.current()

    var body: some View {
        Form {
            if rows.isEmpty {
                Text("No displays reported.")
            }
            ForEach(rows) { row in
                Toggle(row.name, isOn: managed(row.id))
            }
        }
        .formStyle(.grouped)
        .onAppear { rows = DisplayCatalog.current() }
    }

    private func managed(_ uuid: String) -> Binding<Bool> {
        Binding(
            get: { !settings.document.ignoredDisplays.contains(uuid) },
            set: { value in
                settings.update { document in
                    document.ignoredDisplays.removeAll { $0 == uuid }
                    if !value { document.ignoredDisplays.append(uuid) }
                }
            }
        )
    }
}

private struct RulesSection: View {
    @ObservedObject var settings: SettingsModel
    @State private var picking = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(settings.document.rules.indices, id: \.self) { index in
                RuleEditor(settings: settings, index: index)
            }
            Button("Add rule…") { picking = true }
        }
        .sheet(isPresented: $picking) {
            AppPicker { app in
                settings.update { document in
                    document.rules.append(RuleRecord(bundleID: app.bundleID, floating: true))
                }
                picking = false
            }
        }
    }
}

private struct RuleEditor: View {
    @ObservedObject var settings: SettingsModel
    var index: Int

    var body: some View {
        let rule = settings.document.rules[index]
        return VStack(alignment: .leading, spacing: 8) {
            Text(rule.bundleID.isEmpty ? "Any app" : rule.bundleID).font(.headline)
            TextField("Title regex", text: text(\.title))
            TextField("Role", text: text(\.role))
            TextField("Subrole", text: text(\.subrole))
            Toggle("Float", isOn: flag)
            Stepper(value: slot, in: 0...20) {
                Text(rule.slot.map { "Slot \($0)" } ?? "No slot")
            }
            Button("Remove") {
                settings.update { document in
                    guard document.rules.indices.contains(index) else { return }
                    document.rules.remove(at: index)
                }
            }
        }
        .padding(12)
        .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 10))
    }

    private func text(_ key: WritableKeyPath<RuleRecord, String>) -> Binding<String> {
        Binding(
            get: { settings.document.rules[index][keyPath: key] },
            set: { value in settings.update { $0.rules[index][keyPath: key] = value } }
        )
    }

    private var flag: Binding<Bool> {
        Binding(
            get: { settings.document.rules[index].floating },
            set: { value in settings.update { $0.rules[index].floating = value } }
        )
    }

    private var slot: Binding<Int> {
        Binding(
            get: { settings.document.rules[index].slot ?? 0 },
            set: { value in settings.update { $0.rules[index].slot = value == 0 ? nil : value } }
        )
    }
}

private struct ShortcutsSection: View {
    @ObservedObject var settings: SettingsModel

    var body: some View {
        let conflicts = ShortcutCodec.conflicts(settings.document.bindings)
        return VStack(alignment: .leading, spacing: 16) {
            Button("Use default set") { settings.useDefaultShortcuts() }
            ForEach(ShortcutCodec.groups, id: \.0) { group in
                Text(group.0).font(.headline)
                ForEach(group.1, id: \.self) { action in
                    ShortcutRow(settings: settings, action: action, conflict: conflicts.contains(action))
                }
            }
        }
    }
}

private struct ShortcutRow: View {
    @ObservedObject var settings: SettingsModel
    var action: String
    var conflict: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(action.replacingOccurrences(of: "_", with: " "))
                .frame(width: 160, alignment: .leading)
            VStack(alignment: .leading) {
                ForEach(bindings.indices, id: \.self) { index in
                    HStack {
                        RecorderField(text: ShortcutCodec.display(bindings[index]), conflict: conflict) { wire in
                            settings.update { document in
                                var list = document.bindings[action] ?? []
                                if list.indices.contains(index) { list[index] = wire }
                                document.bindings[action] = list
                            }
                        }
                        Button("Remove") {
                            settings.update { document in
                                var list = document.bindings[action] ?? []
                                if list.indices.contains(index) { list.remove(at: index) }
                                document.bindings[action] = list
                            }
                        }
                    }
                }
                Button("Add binding") {
                    settings.update { document in
                        document.bindings[action, default: []].append("ctrl+alt-a")
                    }
                }
            }
            if conflict {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
            }
        }
    }

    private var bindings: [String] {
        settings.document.bindings[action] ?? []
    }
}

struct MacSetupPage: View {
    @ObservedObject var model: AppModel

    var body: some View {
        MacSetupList(model: model)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct RecorderField: NSViewRepresentable {
    var text: String
    var conflict: Bool
    var onCommit: (String) -> Void

    func makeNSView(context: Context) -> RecorderCapture {
        let view = RecorderCapture()
        view.onCommit = onCommit
        return view
    }

    func updateNSView(_ view: RecorderCapture, context: Context) {
        view.label = text
        view.conflict = conflict
        view.onCommit = onCommit
        view.needsDisplay = true
    }
}

final class RecorderCapture: NSView {
    var label = ""
    var conflict = false
    var onCommit: (String) -> Void = { _ in }
    private var monitor: Any?

    override var acceptsFirstResponder: Bool { true }
    override var intrinsicContentSize: NSSize { NSSize(width: 140, height: 24) }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
    }

    override func becomeFirstResponder() -> Bool {
        let accepted = super.becomeFirstResponder()
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self, self.window?.firstResponder === self else { return event }
            let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            guard let wire = ShortcutCodec.wire(
                keycode: event.keyCode,
                ctrl: flags.contains(.control),
                alt: flags.contains(.option),
                shift: flags.contains(.shift),
                cmd: flags.contains(.command)
            ) else { return nil }
            self.onCommit(wire)
            return nil
        }
        needsDisplay = true
        return accepted
    }

    override func resignFirstResponder() -> Bool {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
        needsDisplay = true
        return super.resignFirstResponder()
    }

    override func draw(_ dirtyRect: NSRect) {
        let focused = window?.firstResponder === self
        let fill = conflict ? NSColor.systemOrange.withAlphaComponent(0.25) : NSColor.controlBackgroundColor
        fill.setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 1, dy: 1), xRadius: 6, yRadius: 6).fill()
        (focused ? NSColor.controlAccentColor : NSColor.separatorColor).setStroke()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 1, dy: 1), xRadius: 6, yRadius: 6).stroke()
        let text = label.isEmpty ? "Record" : label
        text.draw(at: NSPoint(x: 8, y: 4), withAttributes: [.font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.labelColor])
    }
}

struct KnownApp: Identifiable, Hashable {
    var id: String { bundleID }
    var name: String
    var bundleID: String
}

enum AppCatalog {
    static func load() -> [KnownApp] {
        var seen = Set<String>()
        var apps: [KnownApp] = []
        func add(name: String, bundle: String) {
            guard !bundle.isEmpty, seen.insert(bundle).inserted else { return }
            apps.append(KnownApp(name: name.isEmpty ? bundle : name, bundleID: bundle))
        }
        for app in NSWorkspace.shared.runningApplications {
            add(name: app.localizedName ?? "", bundle: app.bundleIdentifier ?? "")
        }
        let roots = [
            URL(fileURLWithPath: "/Applications", isDirectory: true),
            FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Applications"),
        ]
        for root in roots {
            let urls = (try? FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
            for url in urls where url.pathExtension == "app" {
                guard let bundle = Bundle(url: url), let identifier = bundle.bundleIdentifier else { continue }
                let name = (bundle.object(forInfoDictionaryKey: "CFBundleName") as? String) ?? url.deletingPathExtension().lastPathComponent
                add(name: name, bundle: identifier)
            }
        }
        return apps.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }
}

private struct AppPicker: View {
    var choose: (KnownApp) -> Void
    @State private var apps = AppCatalog.load()

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Add rule").font(.headline)
            List(apps) { app in
                Button(app.name) { choose(app) }
                    .buttonStyle(.plain)
            }
            .frame(width: 360, height: 320)
        }
        .padding(16)
    }
}

enum DisplayCatalog {
    struct Row: Identifiable {
        var id: String
        var name: String
    }

    static func current() -> [Row] {
        NSScreen.screens.compactMap { screen in
            guard let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else { return nil }
            let display = CGDirectDisplayID(number.uint32Value)
            return Row(id: uuidString(display), name: screen.localizedName)
        }
    }
}

@_silgen_name("CGDisplayCreateUUIDFromDisplayID")
private func copyDisplayUUID(_ display: UInt32) -> Unmanaged<CFUUID>?

private func uuidString(_ display: CGDirectDisplayID) -> String {
    guard let uuid = copyDisplayUUID(display)?.takeRetainedValue() else {
        return String(display)
    }
    return CFUUIDCreateString(nil, uuid) as String
}

private func number(_ value: Double) -> String {
    if value == value.rounded() { return String(Int(value)) }
    return String(format: "%.2f", value)
}

extension ConfigDocument {
    func applyRuntimeSideEffects() {
        NSApp.setActivationPolicy(showInDock ? .regular : .accessory)
        if launchAtLogin {
            try? SMAppService.mainApp.register()
        } else {
            try? SMAppService.mainApp.unregister()
        }
    }
}
