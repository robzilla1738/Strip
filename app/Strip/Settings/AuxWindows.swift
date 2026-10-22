import AppKit
import SwiftUI

final class AuxWindows {
    static let shared = AuxWindows()
    private var about: NSWindow?
    private var guide: NSWindow?
    private var shortcuts: NSWindow?

    func showAbout() {
        about = present(about, title: Brand.current.appName, size: NSSize(width: 560, height: 480), view: AboutView())
    }

    func showGuide() {
        guide = present(guide, title: "Setup Guide", size: NSSize(width: 560, height: 480), view: SetupGuideView())
    }

    func showShortcuts(model: AppModel) {
        shortcuts = present(
            shortcuts,
            title: "\(Brand.current.appName) — Keyboard Shortcuts",
            size: NSSize(width: 660, height: 620),
            view: ShortcutsReferenceView(model: model)
        )
    }

    private func present<V: View>(_ existing: NSWindow?, title: String, size: NSSize, view: V) -> NSWindow {
        if let existing {
            existing.contentViewController = NSHostingController(rootView: view)
            existing.setContentSize(size)
            existing.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return existing
        }
        let window = NSWindow(contentViewController: NSHostingController(rootView: view))
        window.title = title
        window.styleMask = [.titled, .closable, .resizable]
        window.setContentSize(size)
        window.center()
        window.isReleasedWhenClosed = false
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        return window
    }
}

struct AboutView: View {
    var body: some View {
        ScrollView {
            Text(acknowledgements)
                .font(.body)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(20)
        }
    }

    private var acknowledgements: String {
        if let url = Bundle.main.url(forResource: "ACKNOWLEDGEMENTS", withExtension: "md"),
           let text = try? String(contentsOf: url, encoding: .utf8) {
            return text
        }
        return "Engine derived from Paneru (https://github.com/karinushka/paneru), MIT License."
    }
}

struct SetupGuideView: View {
    var body: some View {
        ScrollView {
            Text(MacSetup.guide)
                .font(.body)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(20)
        }
    }
}

struct ShortcutsReferenceView: View {
    @ObservedObject var model: AppModel
    @State private var filter = ""

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 3) {
                    Text("Keyboard Shortcuts & Gestures")
                        .font(.headline)
                    Text("Hold ⌃⌥ to reveal the strip indicator. Swipe horizontally with \(swipeFingers) fingers to scroll the strip.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                TextField("Filter shortcuts…", text: $filter)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 190)
            }
            .padding(16)

            Divider()

            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    ForEach(filteredGroups, id: \.0) { group in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(group.0)
                                .font(.subheadline.weight(.semibold))
                                .foregroundStyle(.secondary)
                                .padding(.horizontal, 4)

                            VStack(spacing: 1) {
                                ForEach(Array(group.1.enumerated()), id: \.element) { index, action in
                                    ShortcutReferenceRow(
                                        title: ShortcutCodec.title(for: action),
                                        keys: ShortcutCodec.formattedKeys(for: action, in: bindings)
                                    )
                                    if index + 1 < group.1.count {
                                        Divider().padding(.leading, 12)
                                    }
                                }
                            }
                            .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 10))
                        }
                    }
                }
                .padding(16)
            }

            Divider()

            HStack {
                Text("All shortcuts can be customized in Settings → Shortcuts.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Customize in Settings…") {
                    model.openMain()
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
    }

    private var bindings: [String: [String]] {
        model.settings?.document.bindings ?? ShortcutCodec.defaults
    }

    private var swipeFingers: Int {
        let count = model.settings?.document.swipeFingers ?? 4
        return count > 0 ? count : 4
    }

    private var filteredGroups: [(String, [String])] {
        let query = filter.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return ShortcutCodec.groups }
        return ShortcutCodec.groups.compactMap { group, actions in
            let matches = actions.filter { action in
                ShortcutCodec.title(for: action).localizedCaseInsensitiveContains(query)
                    || action.localizedCaseInsensitiveContains(query)
                    || ShortcutCodec.formattedKeys(for: action, in: bindings).localizedCaseInsensitiveContains(query)
            }
            return matches.isEmpty ? nil : (group, matches)
        }
    }
}

private struct ShortcutReferenceRow: View {
    let title: String
    let keys: String

    var body: some View {
        HStack {
            Text(title)
                .font(.body)
            Spacer()
            Text(keys)
                .font(.system(.callout, design: .monospaced).weight(.semibold))
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(.quaternary.opacity(0.75), in: RoundedRectangle(cornerRadius: 6))
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }
}
