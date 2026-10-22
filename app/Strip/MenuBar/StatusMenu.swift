import SwiftUI

struct StatusMenu: View {
    @ObservedObject var model: AppModel

    var body: some View {
        Button(model.appName.isEmpty ? model.statusLine : "\(model.appName) — \(model.statusLine)") {}
            .disabled(true)
        if model.axTrusted == false {
            Button("Grant Accessibility Access…") { model.openMain() }
        }
        Divider()
        Toggle("Manage Windows", isOn: managing)
        Button("Restart Engine") { model.restartEngine() }
        Divider()
        Menu("Shortcuts") {
            Button("Keyboard Shortcuts Reference…") { model.openShortcuts() }
            Button("Hold ⌃⌥ to Show Strip Indicator") {}
                .disabled(true)
            Divider()
            ForEach(ShortcutCodec.groups, id: \.0) { group in
                Menu(group.0) {
                    ForEach(group.1, id: \.self) { action in
                        Button(menuLabel(for: action)) {
                            trigger(action)
                        }
                    }
                }
            }
            Divider()
            Button("Customize Shortcuts…") { model.openMain() }
        }
        Button("Keyboard Shortcuts…") { model.openShortcuts() }
            .keyboardShortcut("/", modifiers: .command)
        Button("Settings…") { model.openMain() }
            .keyboardShortcut(",", modifiers: .command)
            .disabled(model.appName.isEmpty)
        Button("Check for Updates…") { model.onCheckForUpdates() }
            .disabled(!model.canCheckForUpdates)
        Button("Install Command Line Tool…") { model.installCommandLineTool() }
        Divider()
        Menu("Help") {
            Button("Keyboard Shortcuts") { model.openShortcuts() }
            Button("Setup Guide") { model.openGuide() }
            Divider()
            Button("Reveal Engine Log") { model.revealLog() }
            Button("Reveal Config File") { model.revealConfig() }
        }
        Button("About \(model.appName)") { model.openAbout() }
            .disabled(model.appName.isEmpty)
        Divider()
        Button("Quit \(model.appName)") { model.quit() }
            .keyboardShortcut("q", modifiers: .command)
    }

    private var bindings: [String: [String]] {
        model.settings?.document.bindings ?? ShortcutCodec.defaults
    }

    private func menuLabel(for action: String) -> String {
        let title = ShortcutCodec.title(for: action)
        let keys = ShortcutCodec.formattedKeys(for: action, in: bindings)
        return "\(title)   (\(keys))"
    }

    private func trigger(_ action: String) {
        if action == "focus_index" || action == "move_to_index" {
            model.send(action, .object(["n": .int(1)]))
        } else {
            model.send(action, .object([:]))
        }
    }

    private var managing: Binding<Bool> {
        Binding(
            get: { model.managing },
            set: { if $0 { model.startManaging() } else { model.stopManaging() } }
        )
    }
}
