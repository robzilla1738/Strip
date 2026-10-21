import AppKit
import SwiftUI

final class AuxWindows {
    static let shared = AuxWindows()
    private var about: NSWindow?
    private var guide: NSWindow?

    func showAbout() {
        about = present(about, title: Brand.current.appName, view: AboutView())
    }

    func showGuide() {
        guide = present(guide, title: "Setup Guide", view: SetupGuideView())
    }

    private func present<V: View>(_ existing: NSWindow?, title: String, view: V) -> NSWindow {
        if let existing {
            existing.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return existing
        }
        let window = NSWindow(contentViewController: NSHostingController(rootView: view))
        window.title = title
        window.styleMask = [.titled, .closable, .resizable]
        window.setContentSize(NSSize(width: 560, height: 480))
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
            Text("Mac Setup checks and their Fix links are listed on the Mac Setup page.")
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(20)
        }
    }
}
