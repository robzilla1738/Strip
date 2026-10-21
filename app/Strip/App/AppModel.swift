import AppKit
import ApplicationServices

final class AppModel: ObservableObject {
    @Published var appName = ""
    @Published var statusLine = "Stopped"
    @Published var accessibilityLine = "Checking"
    @Published var canCheckForUpdates = false
    @Published var axTrusted: Bool?
    @Published var running = false
    @Published var managing = true

    var onCheckForUpdates: () -> Void = {}
    let supervisor = Supervisor()
    private(set) var paths: AppPaths?
    private var client: IpcClient?
    private var didShutdown = false
    private let clientLock = NSLock()

    func launch() {
        let report = AppPrivateAPI.check()
        if !report.missing.isEmpty {
            fputs("private symbols missing: \(report.missing.joined(separator: ", "))\n", stderr)
        }
        let brand = Brand.current
        appName = brand.appName
        let paths = AppPaths.standard(appName: brand.appName)
        self.paths = paths
        try? paths.prepare()
        NSApp.setActivationPolicy(.regular)
        let engine = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/strip-engine")
        supervisor.configure(
            engine: engine,
            arguments: [
                "--socket", paths.socket.path,
                "--config", paths.config.path,
                "--state", paths.state.path,
                "--log", paths.log.path,
            ],
            log: paths.log
        )
        supervisor.onChange = { [weak self] in
            DispatchQueue.main.async { self?.refresh() }
        }
        supervisor.start()
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.connect(to: paths.socket.path)
        }
    }

    func startManaging() {
        if supervisor.currentPID() == nil {
            didShutdown = false
            supervisor.start()
            guard let path = paths?.socket.path else { return }
            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                self?.connect(to: path)
            }
            return
        }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            _ = try? self?.currentClient()?.command("start")
            DispatchQueue.main.async {
                self?.managing = true
                self?.refresh()
            }
        }
    }

    func stopManaging() {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            _ = try? self?.currentClient()?.command("stop")
            DispatchQueue.main.async {
                self?.managing = false
                self?.refresh()
            }
        }
    }

    func restartEngine() {
        didShutdown = false
        supervisor.suppressRestart()
        currentClient()?.shutdown()
        supervisor.forceTerminate()
        supervisor.allowRestart()
        supervisor.start()
        guard let path = paths?.socket.path else { return }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.connect(to: path)
        }
    }

    func openMain() {
        NSApp.activate(ignoringOtherApps: true)
        NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
    }

    func openGuide() {
        AuxWindows.shared.showGuide()
    }

    func openAbout() {
        AuxWindows.shared.showAbout()
    }

    func revealLog() {
        guard let paths else { return }
        NSWorkspace.shared.activateFileViewerSelecting([paths.log])
    }

    func revealConfig() {
        guard let paths else { return }
        NSWorkspace.shared.activateFileViewerSelecting([paths.config])
    }

    func quit() {
        shutdownForRelaunch()
        NSApp.terminate(nil)
    }

    func shutdownForRelaunch() {
        if didShutdown { return }
        didShutdown = true
        supervisor.suppressRestart()
        currentClient()?.shutdown()
        supervisor.waitUntilExit(timeout: 2)
        supervisor.forceTerminate()
    }

    func refresh() {
        running = supervisor.currentPID() != nil
        if supervisor.gaveUp {
            statusLine = "Engine stopped — Reveal Log / Restart"
        } else if axTrusted == false {
            statusLine = "Needs Accessibility"
            accessibilityLine = "Not granted"
        } else if running && managing {
            statusLine = "Running"
            accessibilityLine = axTrusted == true ? "Granted" : "Checking"
        } else {
            statusLine = "Stopped"
        }
    }

    private func connect(to path: String) {
        let deadline = Date().addingTimeInterval(5)
        var hello: WireLine?
        let client = IpcClient()
        while Date() < deadline {
            if FileManager.default.fileExists(atPath: path), let line = try? client.open(socketPath: path, client: "app") {
                hello = line
                break
            }
            Thread.sleep(forTimeInterval: 0.1)
        }
        guard let hello, let trusted = hello.axTrusted else {
            DispatchQueue.main.async { self.refresh() }
            return
        }
        clientLock.lock()
        self.client = client
        clientLock.unlock()
        if AXIsProcessTrusted() && !trusted {
            AgentFallback.register(bundleID: Brand.current.bundleID)
        }
        DispatchQueue.main.async {
            self.axTrusted = trusted
            self.managing = true
            self.refresh()
        }
    }

    private func currentClient() -> IpcClient? {
        clientLock.lock()
        defer { clientLock.unlock() }
        return client
    }
}

enum AppRuntime {
    static var model: AppModel?
}
