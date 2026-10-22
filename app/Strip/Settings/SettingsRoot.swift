import AppKit
import ServiceManagement
import SwiftUI

final class SettingsModel: ObservableObject {
    @Published var document: ConfigDocument
    let url: URL
    var onChange: (ConfigDocument) -> Void = { _ in }
    private var pendingWrite: DispatchWorkItem?

    init(url: URL) {
        self.url = url
        document = ConfigDocument.load(from: url)
    }

    deinit {
        pendingWrite?.cancel()
        if pendingWrite != nil { try? ConfigDocument.write(document, to: url) }
    }

    /// The document updates instantly for the UI; the file write and engine
    /// reload coalesce so dragging a slider does not rewrite the config and
    /// reload the engine on every tick.
    func update(_ body: (inout ConfigDocument) -> Void) {
        body(&document)
        pendingWrite?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.pendingWrite = nil
            try? ConfigDocument.write(self.document, to: self.url)
            self.onChange(self.document)
        }
        pendingWrite = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12, execute: work)
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
                Label(item.title, systemImage: item.icon).tag(item)
            }
            .listStyle(.sidebar)
            .navigationSplitViewColumnWidth(min: 180, ideal: 200)
            .safeAreaInset(edge: .bottom) {
                HStack(spacing: 8) {
                    Circle()
                        .fill(model.managing ? Color.green : Color.secondary)
                        .frame(width: 8, height: 8)
                    Text(model.statusLine)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Spacer()
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
            }
        } detail: {
            sectionView
                .navigationTitle(section.title)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(minWidth: 820, minHeight: 540)
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
    var icon: String {
        switch self {
        case .general: return "gearshape"
        case .layout: return "rectangle.split.3x1"
        case .trackpad: return "hand.draw"
        case .appearance: return "paintbrush"
        case .displays: return "display.2"
        case .rules: return "list.bullet.rectangle"
        case .shortcuts: return "keyboard"
        case .macSetup: return "wrench.and.screwdriver"
        case .about: return "info.circle"
        }
    }
}

/// A labeled slider with its live value, so no control is a bare unlabeled bar.
private struct SliderRow: View {
    var title: String
    @Binding var value: Double
    var range: ClosedRange<Double>
    var format: (Double) -> String

    var body: some View {
        LabeledContent(title) {
            HStack(spacing: 10) {
                Slider(value: $value, in: range)
                    .frame(minWidth: 140)
                Text(format(value))
                    .font(.callout.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .frame(width: 58, alignment: .trailing)
            }
        }
    }
}

private func percent(_ value: Double) -> String { "\(Int((value * 100).rounded()))%" }
private func pixels(_ value: Double) -> String { "\(Int(value.rounded())) px" }
private func times(_ value: Double) -> String { String(format: "%.2f×", value) }

private struct GeneralSection: View {
    @ObservedObject var model: AppModel
    @ObservedObject var settings: SettingsModel
    @ObservedObject var licensing: LicenseController
    @State private var enteredKey = ""

    var body: some View {
        Form {
            Section {
                Toggle("Launch at login", isOn: binding(\.launchAtLogin))
                Toggle("Start managing on launch", isOn: binding(\.startManagingOnLaunch))
                Toggle("Show in Dock", isOn: binding(\.showInDock))
            } header: {
                Text("Startup")
            } footer: {
                Text("The strip arranges your windows as soon as it starts managing.")
            }
            Section {
                LabeledContent("Status", value: model.statusLine)
                HStack {
                    Button("Start") { model.startManaging() }
                        .disabled(model.managing)
                    Button("Stop") { model.stopManaging() }
                        .disabled(!model.managing)
                    Button("Restart Engine") { model.restartEngine() }
                }
            } header: {
                Text("Engine")
            }
            Section {
                LabeledContent("Status", value: licensing.phase.statusText)
                if case .notConfigured = licensing.phase {
                    Text("This build has no payment products configured, so licensing is off and everything runs.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                } else if case .licensed(let masked, let product) = licensing.phase {
                    LabeledContent("Key", value: masked)
                    LabeledContent("Product", value: product)
                    Button("Deactivate", role: .destructive) { licensing.deactivate(now: Date()) }
                } else {
                    HStack {
                        TextField("License key", text: $enteredKey)
                            .textFieldStyle(.roundedBorder)
                        Button("Activate") { licensing.activate(enteredKey, now: Date()) }
                            .disabled(enteredKey.isEmpty)
                    }
                    Button("Buy License…") { licensing.openCheckout() }
                }
            } header: {
                Text("License")
            }
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
            Section {
                SliderRow(title: "Default width", value: double(\.defaultColumnWidth), range: 0.1...1, format: percent)
                SliderRow(title: "Width step", value: double(\.widthStep), range: 0.01...1, format: percent)
                LabeledContent("Preset widths") {
                    TextField("0.25, 0.33, 0.5, 0.66, 0.75", text: widths)
                        .textFieldStyle(.roundedBorder)
                        .frame(minWidth: 220)
                }
            } header: {
                Text("Columns")
            } footer: {
                Text("Cycle a window through the preset widths with the cycle-width shortcut. Widths are fractions of the display, separated by commas.")
            }
            Section {
                SliderRow(title: "Between windows", value: double(\.gapInner), range: 0...64, format: pixels)
                SliderRow(title: "At screen edges", value: double(\.gapOuter), range: 0...64, format: pixels)
                Toggle("Keep windows clear of the Dock", isOn: bool(\.dockAware))
            } header: {
                Text("Gaps")
            }
            Section {
                Picker("Center focused column", selection: string(\.centerMode)) {
                    Text("Never").tag("never")
                    Text("On overflow").tag("on_overflow")
                    Text("Always").tag("always")
                }
                .pickerStyle(.segmented)
                Toggle("Keep original window heights", isOn: bool(\.keepOriginalHeight))
                Toggle("Focus follows mouse", isOn: bool(\.focusFollowsMouse))
                Toggle("Mouse follows focus", isOn: bool(\.mouseFollowsFocus))
                Toggle("Drag title bar to reorder", isOn: bool(\.dragToReorder))
            } header: {
                Text("Behavior")
            }
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
            Section {
                Picker("Swipe with", selection: fingers) {
                    Text("Off").tag(0)
                    Text("3 fingers").tag(3)
                    Text("4 fingers").tag(4)
                    Text("5 fingers").tag(5)
                }
                .pickerStyle(.segmented)
                Toggle("Natural direction", isOn: bool(\.swipeNatural))
                SliderRow(title: "Sensitivity", value: double(\.swipeSensitivity), range: 0.2...3, format: times)
                SliderRow(title: "Fling friction", value: glide, range: 0.5...12, format: { String(format: "%.1f", $0) })
            } header: {
                Text("Scroll the strip")
            } footer: {
                Text("Swipe sideways to scroll the strip under your fingers. Natural direction moves the windows with your fingers. Higher friction stops a fling sooner.")
            }
            if let conflict = conflictText {
                Section {
                    Label(conflict, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                    Button("Open Trackpad Settings…") {
                        if let url = URL(string: "x-apple.systempreferences:com.apple.Trackpad-Settings.extension") {
                            NSWorkspace.shared.open(url)
                        }
                    }
                } header: {
                    Text("Avoid conflicts")
                } footer: {
                    Text("Otherwise macOS and the strip both react to the same swipe, which feels jerky.")
                }
            }
        }
        .formStyle(.grouped)
    }

    private var conflictText: String? {
        switch settings.document.swipeFingers {
        case 4: return "Turn off “Swipe between full-screen apps” with four fingers in Trackpad settings."
        case 3: return "Turn off three-finger swipe and three-finger drag in Trackpad settings."
        default: return nil
        }
    }

    /// Deceleration shown directly; kept as its own binding for the label.
    private var glide: Binding<Double> { double(\.swipeDeceleration) }

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
            Section {
                Toggle("Show strip indicator", isOn: bool(\.showIndicator))
            } header: {
                Text("Strip indicator")
            } footer: {
                Text("The floating pill at the bottom of the screen. Click a column to jump to it, or drag to reorder.")
            }
            Section {
                Toggle("Focus border", isOn: bool(\.focusBorder))
                Group {
                    SliderRow(title: "Width", value: double(\.focusBorderWidth), range: 1...12, format: pixels)
                    SliderRow(title: "Corner radius", value: double(\.focusBorderRadius), range: 0...24, format: pixels)
                    LabeledContent("Color") {
                        HStack(spacing: 10) {
                            ColorPicker("", selection: borderColor, supportsOpacity: false)
                                .labelsHidden()
                            TextField("#4C8DFF", text: string(\.focusBorderColor))
                                .textFieldStyle(.roundedBorder)
                                .font(.callout.monospaced())
                                .frame(width: 90)
                        }
                    }
                }
                .disabled(!settings.document.focusBorder)
                .opacity(settings.document.focusBorder ? 1 : 0.45)
            } header: {
                Text("Focused window")
            }
            Section {
                Toggle("Dim inactive windows", isOn: bool(\.dimInactive))
                SliderRow(title: "Dim amount", value: double(\.dimInactiveOpacity), range: 0...0.8, format: percent)
                    .disabled(!settings.document.dimInactive)
                    .opacity(settings.document.dimInactive ? 1 : 0.45)
            } header: {
                Text("Inactive windows")
            }
            Section {
                HStack(spacing: 8) {
                    Button("Snappy") { setAnimation(duration: 160, damping: 1.0) }
                    Button("Balanced") { setAnimation(duration: 250, damping: 0.9) }
                    Button("Smooth") { setAnimation(duration: 360, damping: 0.85) }
                    Button("Off") { setAnimation(duration: 0, damping: 1.0) }
                }
                SliderRow(title: "Duration", value: duration, range: 0...1000, format: { "\(Int($0.rounded())) ms" })
                SliderRow(title: "Spring damping", value: double(\.animationDamping), range: 0.1...1, format: { String(format: "%.2f", $0) })
            } header: {
                Text("Animation")
            } footer: {
                Text("Lower damping adds a slight overshoot as the strip settles. 1.00 never overshoots.")
            }
        }
        .formStyle(.grouped)
    }

    private func setAnimation(duration: Int, damping: Double) {
        settings.update {
            $0.animationDurationMs = duration
            $0.animationDamping = damping
        }
    }

    private var borderColor: Binding<Color> {
        Binding(
            get: { Color(nsColor: color(hex: settings.document.focusBorderColor)) },
            set: { newColor in
                guard let srgb = NSColor(newColor).usingColorSpace(.sRGB) else { return }
                let hex = String(format: "#%02X%02X%02X", Int((srgb.redComponent * 255).rounded()), Int((srgb.greenComponent * 255).rounded()), Int((srgb.blueComponent * 255).rounded()))
                settings.update { $0.focusBorderColor = hex }
            }
        )
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
            Section {
                if rows.isEmpty {
                    Text("No displays reported.").foregroundStyle(.secondary)
                }
                ForEach(rows) { row in
                    Toggle(isOn: managed(row.id)) {
                        Label(row.name, systemImage: "display")
                    }
                }
            } header: {
                Text("Managed displays")
            } footer: {
                Text("Each managed display keeps its own strip. Turn a display off to leave its windows alone.")
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
        Form {
            if settings.document.rules.isEmpty {
                Section {
                    Text("No rules yet. Add one to float an app above the strip or send it to a fixed slot.")
                        .foregroundStyle(.secondary)
                }
            }
            ForEach(settings.document.rules.indices, id: \.self) { index in
                RuleEditor(settings: settings, index: index)
            }
            Section {
                Button {
                    picking = true
                } label: {
                    Label("Add Rule…", systemImage: "plus")
                }
            }
        }
        .formStyle(.grouped)
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
        return Section {
            LabeledContent("Title contains") {
                TextField("Regular expression", text: text(\.title)).textFieldStyle(.roundedBorder)
            }
            LabeledContent("Role") {
                TextField("AXWindow", text: text(\.role)).textFieldStyle(.roundedBorder)
            }
            LabeledContent("Subrole") {
                TextField("AXStandardWindow", text: text(\.subrole)).textFieldStyle(.roundedBorder)
            }
            Toggle("Float above the strip", isOn: flag)
            Stepper(value: slot, in: 0...20) {
                LabeledContent("Preferred slot", value: rule.slot.map { "\($0)" } ?? "None")
            }
        } header: {
            HStack {
                if let icon = appIcon(rule.bundleID) {
                    Image(nsImage: icon).resizable().frame(width: 18, height: 18)
                }
                Text(rule.bundleID.isEmpty ? "Any app" : appName(rule.bundleID))
                Spacer()
                Button(role: .destructive) {
                    settings.update { document in
                        guard document.rules.indices.contains(index) else { return }
                        document.rules.remove(at: index)
                    }
                } label: {
                    Image(systemName: "trash")
                }
                .buttonStyle(.borderless)
                .help("Remove rule")
            }
        } footer: {
            if !rule.bundleID.isEmpty {
                Text(rule.bundleID).font(.caption.monospaced())
            }
        }
    }

    private func appName(_ bundleID: String) -> String {
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) else { return bundleID }
        return FileManager.default.displayName(atPath: url.path)
    }

    private func appIcon(_ bundleID: String) -> NSImage? {
        guard !bundleID.isEmpty, let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) else { return nil }
        return NSWorkspace.shared.icon(forFile: url.path)
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
        return Form {
            if !conflicts.isEmpty {
                Section {
                    Label("Some shortcuts are assigned twice. Conflicting rows are marked.", systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                }
            }
            ForEach(ShortcutCodec.groups, id: \.0) { group in
                Section(group.0) {
                    ForEach(group.1, id: \.self) { action in
                        ShortcutRow(settings: settings, action: action, conflict: conflicts.contains(action))
                    }
                }
            }
            Section {
                Button("Restore Default Shortcuts") { settings.useDefaultShortcuts() }
            } footer: {
                Text("Click a shortcut, then press the new keys. The defaults hold Control and Option with each key.")
            }
        }
        .formStyle(.grouped)
    }
}

private struct ShortcutRow: View {
    @ObservedObject var settings: SettingsModel
    var action: String
    var conflict: Bool

    var body: some View {
        LabeledContent {
            VStack(alignment: .trailing, spacing: 6) {
                ForEach(bindings.indices, id: \.self) { index in
                    HStack(spacing: 6) {
                        RecorderField(text: ShortcutCodec.display(bindings[index]), conflict: conflict) { wire in
                            settings.update { document in
                                var list = document.bindings[action] ?? []
                                if list.indices.contains(index) { list[index] = wire }
                                document.bindings[action] = list
                            }
                        }
                        Button(role: .destructive) {
                            settings.update { document in
                                var list = document.bindings[action] ?? []
                                if list.indices.contains(index) { list.remove(at: index) }
                                document.bindings[action] = list
                            }
                        } label: {
                            Image(systemName: "minus.circle.fill")
                        }
                        .buttonStyle(.borderless)
                        .help("Remove binding")
                    }
                }
                Button {
                    settings.update { document in
                        document.bindings[action, default: []].append("ctrl+alt-a")
                    }
                } label: {
                    Label("Add", systemImage: "plus")
                }
                .controlSize(.small)
            }
        } label: {
            HStack(spacing: 6) {
                Text(ShortcutCodec.title(for: action))
                if conflict {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .help("This shortcut is also assigned elsewhere")
                }
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
        ScrollView {
            MacSetupList(model: model)
                .frame(maxWidth: 640, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(24)
        }
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
        let rect = bounds.insetBy(dx: 1, dy: 1)
        let path = NSBezierPath(roundedRect: rect, xRadius: 7, yRadius: 7)
        let fill = conflict
            ? NSColor.systemOrange.withAlphaComponent(0.25)
            : (focused ? NSColor.controlAccentColor.withAlphaComponent(0.12) : NSColor.controlBackgroundColor)
        fill.setFill()
        path.fill()
        (focused ? NSColor.controlAccentColor : NSColor.separatorColor).setStroke()
        path.lineWidth = focused ? 1.5 : 1
        path.stroke()
        let text = label.isEmpty ? (focused ? "Press keys…" : "Record") : label
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 12, weight: .medium),
            .foregroundColor: focused && label.isEmpty ? NSColor.secondaryLabelColor : NSColor.labelColor,
        ]
        let size = (text as NSString).size(withAttributes: attributes)
        text.draw(at: NSPoint(x: (bounds.width - size.width) / 2, y: (bounds.height - size.height) / 2), withAttributes: attributes)
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
    @State private var search = ""
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Add a rule for…").font(.headline)
            TextField("Search apps", text: $search)
                .textFieldStyle(.roundedBorder)
            List(filtered) { app in
                Button {
                    choose(app)
                } label: {
                    HStack(spacing: 10) {
                        if let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: app.bundleID) {
                            Image(nsImage: NSWorkspace.shared.icon(forFile: url.path))
                                .resizable()
                                .frame(width: 22, height: 22)
                        }
                        VStack(alignment: .leading, spacing: 1) {
                            Text(app.name)
                            Text(app.bundleID).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
                .buttonStyle(.plain)
            }
            .frame(width: 380, height: 340)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                    .keyboardShortcut(.cancelAction)
            }
        }
        .padding(16)
    }

    private var filtered: [KnownApp] {
        guard !search.isEmpty else { return apps }
        return apps.filter { $0.name.localizedCaseInsensitiveContains(search) || $0.bundleID.localizedCaseInsensitiveContains(search) }
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
            return Row(id: displayUUID(display), name: screen.localizedName)
        }
    }
}

@_silgen_name("CGDisplayCreateUUIDFromDisplayID")
private func copyDisplayUUID(_ display: UInt32) -> Unmanaged<CFUUID>?

func displayUUID(_ display: CGDirectDisplayID) -> String {
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
