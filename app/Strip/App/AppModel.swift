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
    private var trustTimer: Timer?
    private var restartedForTrust = false
    private var connecting = false
    private let eventLock = NSLock()
    private var pendingEvents: [WireLine] = []
    private var drainScheduled = false

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
            self?.overlays.applyAppearance(document)
        }
        overlays.applyAppearance(settings.document)
        if !adoptedEngine {
            settings.document.applyRuntimeSideEffects()
        }
        if !AXIsProcessTrusted() {
            ControlAccess.prompt()
            watchForTrust()
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
            DispatchQueue.main.async {
                self?.refresh()
                self?.reconnectIfNeeded()
            }
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

    /// The event tap is created at engine start. After the user grants access,
    /// start the engine again so the tap is created with the new permission.
    private func watchForTrust() {
        trustTimer?.invalidate()
        trustTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] timer in
            guard let self, !self.didShutdown else {
                timer.invalidate()
                return
            }
            guard AXIsProcessTrusted() else { return }
            timer.invalidate()
            self.trustTimer = nil
            guard self.axTrusted != true, !self.restartedForTrust else { return }
            self.restartedForTrust = true
            self.restartEngine()
        }
    }

    func refresh() {
        running = adoptedEngine || supervisor.currentPID() != nil
        if supervisor.gaveUp {
            statusLine = "Engine stopped — Reveal Log / Restart"
        } else if axTrusted == false {
            statusLine = "Needs \(ControlAccess.paneName)"
            accessibilityLine = "Not granted"
        } else if running && managing {
            statusLine = "Running"
            accessibilityLine = axTrusted == true ? "Granted" : "Checking"
        } else {
            statusLine = "Stopped"
        }
    }

    private func connect(to path: String) {
        clientLock.lock()
        if connecting {
            clientLock.unlock()
            return
        }
        connecting = true
        clientLock.unlock()
        defer {
            clientLock.lock()
            connecting = false
            clientLock.unlock()
        }
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
            client.close()
            DispatchQueue.main.async { self.refresh() }
            return
        }
        clientLock.lock()
        let previousClient = self.client
        self.client = client
        clientLock.unlock()
        previousClient?.close()
        let events = IpcClient()
        events.onEvent = { [weak self] line in
            self?.enqueue(line)
        }
        events.onClose = { [weak self, weak events] _ in
            DispatchQueue.main.async {
                guard let self, let events else { return }
                self.eventStreamEnded(events, path: path)
            }
        }
        if (try? events.open(socketPath: path, client: "app")) != nil {
            _ = try? events.subscribe(["state", "focus", "indicator", "tour", "log"])
            clientLock.lock()
            let previousEvents = self.events
            self.events = events
            clientLock.unlock()
            previousEvents?.close()
        } else {
            events.close()
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

    /// State lines can arrive at display refresh rate. Only the newest of a run of
    /// consecutive states is worth applying; other events keep their order.
    private func enqueue(_ line: WireLine) {
        eventLock.lock()
        if line.event == "state", let last = pendingEvents.last, last.event == "state" {
            pendingEvents[pendingEvents.count - 1] = line
        } else {
            pendingEvents.append(line)
        }
        let schedule = !drainScheduled
        drainScheduled = true
        eventLock.unlock()
        if schedule {
            DispatchQueue.main.async { [weak self] in self?.drainEvents() }
        }
    }

    private func drainEvents() {
        eventLock.lock()
        let batch = pendingEvents
        pendingEvents.removeAll(keepingCapacity: true)
        drainScheduled = false
        eventLock.unlock()
        for line in batch {
            overlays.apply(event: line)
            if let action = line.tourAction {
                tour.receive(action)
            }
        }
    }

    private func eventStreamEnded(_ stream: IpcClient, path: String) {
        clientLock.lock()
        let current = events === stream
        if current { events = nil }
        clientLock.unlock()
        guard current else { return }
        overlays.reset()
        refresh()
        guard !didShutdown else { return }
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + 0.5) { [weak self] in
            guard let self, !self.didShutdown else { return }
            self.connect(to: path)
        }
    }

    /// The supervisor respawns a crashed engine on its own; follow it to the new socket.
    private func reconnectIfNeeded() {
        guard !didShutdown, supervisor.currentPID() != nil, let path = paths?.socket.path else { return }
        clientLock.lock()
        let attached = events != nil || connecting
        clientLock.unlock()
        guard !attached else { return }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.connect(to: path)
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
