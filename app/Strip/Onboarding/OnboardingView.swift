import AppKit
import SwiftUI

struct OnboardingView: View {
    @ObservedObject var model: AppModel
    @ObservedObject var tour: TourProgress
    @State private var page = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(pageTitle).font(.title2.weight(.semibold))
            pageBody
            HStack {
                if page > 0 {
                    Button("Back") { page -= 1 }
                }
                Spacer()
                Button(page == 4 ? "Done" : "Continue") { advance() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(page == 3 && !tour.finished)
            }
        }
        .padding(24)
        .frame(width: 520, height: 420, alignment: .topLeading)
    }

    private var pageTitle: String {
        switch page {
        case 0: return model.appName.isEmpty ? "Welcome" : "Welcome to \(model.appName)"
        case 1: return ControlAccess.paneName
        case 2: return "Mac Setup"
        case 3: return "Tour"
        default: return "Ready"
        }
    }

    @ViewBuilder private var pageBody: some View {
        switch page {
        case 0:
            Text("Columns sit on a horizontal strip. Shortcuts and a trackpad swipe move between them.")
        case 1:
            Text(model.axTrusted == true
                ? "\(ControlAccess.paneName) is granted."
                : "Allow this app in Privacy & Security → \(ControlAccess.paneName). This page updates when macOS grants it.")
            if model.axTrusted != true {
                Button("Open \(ControlAccess.paneName)") {
                    ControlAccess.openSettings()
                }
            }
        case 2:
            MacSetupList(model: model)
        case 3:
            VStack(alignment: .leading, spacing: 8) {
                ForEach(Array(TourProgress.titles.enumerated()), id: \.offset) { offset, title in
                    Label(title, systemImage: offset < tour.index ? "checkmark.circle.fill" : "circle")
                        .foregroundStyle(offset == tour.index ? .primary : .secondary)
                }
                Text("Use the shortcut or gesture. The step advances when the engine reports it.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        default:
            Text("You can reopen these checks from Mac Setup and Help → Setup Guide.")
        }
    }

    private func advance() {
        if page == 4 {
            Onboarding.complete()
            OnboardingWindow.shared.close()
            return
        }
        page += 1
    }
}

final class OnboardingWindow {
    static let shared = OnboardingWindow()
    private var window: NSWindow?

    func show(model: AppModel) {
        if let window {
            window.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return
        }
        let host = NSHostingController(rootView: OnboardingView(model: model, tour: model.tour))
        let window = NSWindow(contentViewController: host)
        window.title = model.appName
        window.styleMask = [.titled, .closable, .miniaturizable]
        window.isReleasedWhenClosed = false
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        self.window = window
    }

    func close() {
        window?.close()
        window = nil
    }
}
