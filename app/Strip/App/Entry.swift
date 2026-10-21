import AppKit
import Carbon
import Sparkle
import SwiftUI

@main
enum EntryPoint {
    static func main() {
        if CommandLine.arguments.contains("--headless-test") {
            HeadlessRun.run()
        } else {
            StripApp.main()
        }
    }
}

struct StripApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var model: AppModel
    @StateObject private var updates: UpdateController

    init() {
        let model = AppModel()
        let updates = UpdateController(model: model)
        model.canCheckForUpdates = updates.canCheck
        model.onCheckForUpdates = { updates.check() }
        _model = StateObject(wrappedValue: model)
        _updates = StateObject(wrappedValue: updates)
        AppRuntime.model = model
    }

    var body: some Scene {
        MenuBarExtra {
            StatusMenu(model: model)
        } label: {
            Image(systemName: model.axTrusted == false ? "exclamationmark.triangle" : "rectangle.split.3x1")
        }
        .menuBarExtraStyle(.menu)

        Settings {
            MainView(model: model)
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var duplicate = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSAppleEventManager.shared().setEventHandler(
            self,
            andSelector: #selector(handleGetURL(_:withReplyEvent:)),
            forEventClass: AEEventClass(kInternetEventClass),
            andEventID: AEEventID(kAEGetURL)
        )
        let bundle = Bundle.main.bundleIdentifier ?? ""
        let others = NSRunningApplication.runningApplications(withBundleIdentifier: bundle)
            .filter { $0.processIdentifier != ProcessInfo.processInfo.processIdentifier }
        if !others.isEmpty && ProcessInfo.processInfo.environment["STRIP_ADOPT_ENGINE"] != "1" {
            duplicate = true
            return
        }
        DistributedNotificationCenter.default.addObserver(
            self,
            selector: #selector(handleForwardedURL(_:)),
            name: Notification.Name(bundle + ".open-url"),
            object: nil
        )
        AppRuntime.model?.launch()
    }

    func applicationWillTerminate(_ notification: Notification) {
        AppRuntime.model?.shutdownForRelaunch()
    }

    @objc private func handleGetURL(_ event: NSAppleEventDescriptor, withReplyEvent: NSAppleEventDescriptor) {
        guard let string = event.paramDescriptor(forKeyword: keyDirectObject)?.stringValue,
              let url = URL(string: string) else { return }
        if duplicate {
            let bundle = Bundle.main.bundleIdentifier ?? ""
            DistributedNotificationCenter.default().postNotificationName(
                Notification.Name(bundle + ".open-url"),
                object: url.absoluteString,
                userInfo: nil,
                deliverImmediately: true
            )
            NSApp.terminate(nil)
            return
        }
        AppRuntime.model?.open(url: url)
    }

    @objc private func handleForwardedURL(_ note: Notification) {
        guard let string = note.object as? String, let url = URL(string: string) else { return }
        AppRuntime.model?.open(url: url)
    }
}

final class UpdateController: NSObject, ObservableObject, SPUUpdaterDelegate {
    private var controller: SPUStandardUpdaterController?
    private weak var model: AppModel?
    let canCheck: Bool

    init(model: AppModel) {
        self.model = model
        let info = Bundle.main.infoDictionary ?? [:]
        let feed = info["SUFeedURL"] as? String ?? ""
        let key = info["SUPublicEDKey"] as? String ?? ""
        let feedReady = feed.hasPrefix("https://") && !feed.contains("{")
        let keyReady = !key.isEmpty && !key.contains("{")
        canCheck = feedReady && keyReady
        super.init()
        if !feedReady {
            fputs("MISSING: {{APPCAST_URL}}\n", stderr)
        }
        if !keyReady {
            fputs("MISSING: {{SPARKLE_PUBLIC_ED_KEY}}\n", stderr)
        }
        guard canCheck else { return }
        controller = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: self,
            userDriverDelegate: nil
        )
    }

    func check() {
        controller?.checkForUpdates(nil)
    }

    func updaterWillRelaunchApplication(_ updater: SPUUpdater) {
        model?.shutdownForRelaunch()
    }
}
