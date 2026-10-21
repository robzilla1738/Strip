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
    let overlays = OverlayController()
    let tour = TourProgress()
    let licensing = LicenseController.live()
    private var licenseWantsManaging: Bool?
    @Published var settings: SettingsModel?
    private(set) var paths: AppPaths?
    private var client: IpcClient?
    private var events: IpcClient?
    private var didShutdown = false
    private var adoptedEngine = false
    private var pendingCommand: URLCommand?
    private let clientLock = NSLock()

    func launch() {
        let env = ProcessInfo.processInfo.environment
        adoptedEngine = env["STRIP_ADOPT_ENGINE"] == "1"
        let report = AppPrivateAPI.check()
        if !report.missing.isEmpty {
            fputs("private symbols missing: \(report.missing.joined(separator: ", "))\n", stderr)
        }
        let brand = Brand.current
        appName = brand.appName
        var paths = AppPaths.standard(appName: brand.appName)
        if let override = env["STRIP_SOCKET"], !override.isEmpty {
            paths.socket = URL(fileURLWithPath: override)
        }
        self.paths = paths
        try? paths.prepare()
        let settings = SettingsModel(url: paths.config)
        settings.onChange = { [weak self] document in
            document.applyRuntimeSideEffects()
            self?.overlays.setShowIndicator(document.showIndicator)
        }
        overlays.setShowIndicator(settings.document.showIndicator)
        if !adoptedEngine {
            settings.document.applyRuntimeSideEffects()
        }
        overlays.onFocus = { [weak self] id in
            self?.send("focus_window", .object(["window_id": .int(Int(id))]))
        }
        overlays.onMove = { [weak self] from, to in
            self?.send("move_column", .object(["from": .int(from), "to": .int(to)]))
        }
        overlays.onTab = { [weak self] id in
            self?.send("focus_window", .object(["window_id": .int(Int(id))]))
        }
        self.settings = settings
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
        if !adoptedEngine {
            supervisor.start()
        }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.connect(to: paths.socket.path)
        }
        if !adoptedEngine && Onboarding.shouldShow {
            OnboardingWindow.shared.show(model: self)
        }
        if !adoptedEngine {
            licensing.onManaging = { [weak self] enabled in
                self?.applyLicenseManaging(enabled)
            }
            licensing.refresh(now: Date())
            licensing.startSchedule()
        }
    }

    private func applyLicenseManaging(_ enabled: Bool) {
        licenseWantsManaging = enabled
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self else { return }
            _ = try? self.currentClient()?.setManaging(enabled)
            DispatchQueue.main.async {
                self.managing = enabled
                self.refresh()
            }
        }
    }

    func open(url: URL) {
        guard let command = URLCommand.parse(url, scheme: Brand.current.urlScheme) else { return }
        pendingCommand = command
        flushPending()
    }

    func installCommandLineTool() {
        DispatchQueue.global(qos: .userInitiated).async {
            CommandLineInstall.install()
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
        if adoptedEngine { return }
        supervisor.suppressRestart()
        currentClient()?.shutdown()
        supervisor.waitUntilExit(timeout: 2)
        supervisor.forceTerminate()
    }

    func refresh() {
        running = adoptedEngine || supervisor.currentPID() != nil
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
        let events = IpcClient()
        if (try? events.open(socketPath: path, client: "app")) != nil {
            events.onEvent = { [weak self] line in
                DispatchQueue.main.async {
                    guard let self else { return }
                    self.overlays.apply(event: line)
                    if let action = line.tourAction {
                        self.tour.receive(action)
                    }
                }
            }
            _ = try? events.subscribe(["state", "focus", "indicator", "tour", "log"])
            clientLock.lock()
            self.events = events
            clientLock.unlock()
        }
        if AXIsProcessTrusted() && !trusted {
            AgentFallback.register(bundleID: Brand.current.bundleID)
        }
        DispatchQueue.main.async {
            self.axTrusted = trusted
            self.managing = true
            self.refresh()
            self.flushPending()
            if let enabled = self.licenseWantsManaging {
                self.applyLicenseManaging(enabled)
            }
        }
    }

    private func flushPending() {
        guard currentClient() != nil, let command = pendingCommand else { return }
        pendingCommand = nil
        send(command.action, command.args)
    }

    func send(_ action: String, _ args: JSONValue) {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            _ = try? self?.currentClient()?.command(action, args: args)
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
