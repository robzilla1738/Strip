import Darwin
import Foundation

/// Keeps the engine process alive. The first restart waits 0.5 s. Five exits
/// inside 60 s stop the retries.
final class Supervisor {
    var extraEnvironment: [String: String] = [:]
    var onChange: (() -> Void)?

    private let lock = NSLock()
    private var process: Process?
    private var suppress = false
    private var exits: [Date] = []
    private var backoffIndex = 0
    private let steps: [TimeInterval] = [0.5, 1, 2, 4, 8, 16, 30]
    private var engineURL: URL?
    private var arguments: [String] = []
    private var logURL: URL?
    private(set) var gaveUp = false

    func configure(engine: URL, arguments: [String], log: URL) {
        lock.lock()
        engineURL = engine
        self.arguments = arguments
        logURL = log
        lock.unlock()
    }

    func currentPID() -> Int32? {
        lock.lock()
        let process = self.process
        lock.unlock()
        guard let process, process.isRunning else { return nil }
        return process.processIdentifier
    }

    func allowRestart() {
        lock.lock()
        suppress = false
        gaveUp = false
        exits.removeAll()
        backoffIndex = 0
        lock.unlock()
        onChange?()
    }

    func suppressRestart() {
        lock.lock()
        suppress = true
        lock.unlock()
    }

    func start() {
        allowRestart()
        spawn()
    }

    func spawn() {
        lock.lock()
        if suppress || gaveUp {
            lock.unlock()
            return
        }
        if let process, process.isRunning {
            lock.unlock()
            return
        }
        guard let engineURL, let logURL else {
            lock.unlock()
            return
        }
        let arguments = self.arguments
        var environment = ProcessInfo.processInfo.environment
        for (key, value) in extraEnvironment {
            environment[key] = value
        }
        lock.unlock()

        let process = Process()
        process.executableURL = engineURL
        process.arguments = arguments
        process.environment = environment
        prepareLog(logURL)
        if let handle = FileHandle(forWritingAtPath: logURL.path) {
            handle.seekToEndOfFile()
            process.standardOutput = handle
            process.standardError = handle
        }
        process.terminationHandler = { [weak self] proc in
            self?.handleExit(proc)
        }
        do {
            try process.run()
        } catch {
            handleExit(process)
            return
        }
        lock.lock()
        self.process = process
        lock.unlock()
        onChange?()
    }

    func waitUntilExit(timeout: TimeInterval) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if currentPID() == nil {
                return
            }
            Thread.sleep(forTimeInterval: 0.05)
        }
    }

    func forceTerminate() {
        lock.lock()
        let process = self.process
        lock.unlock()
        guard let process, process.isRunning else { return }
        kill(process.processIdentifier, SIGTERM)
        waitUntilExit(timeout: 0.4)
        if process.isRunning {
            kill(process.processIdentifier, SIGKILL)
        }
    }

    private func handleExit(_ proc: Process) {
        lock.lock()
        if suppress {
            lock.unlock()
            onChange?()
            return
        }
        let now = Date()
        exits.append(now)
        exits.removeAll { now.timeIntervalSince($0) > 60 }
        if exits.count >= 5 {
            gaveUp = true
            lock.unlock()
            onChange?()
            return
        }
        let delay = steps[min(backoffIndex, steps.count - 1)]
        backoffIndex += 1
        lock.unlock()
        onChange?()
        DispatchQueue.global().asyncAfter(deadline: .now() + delay) { [weak self] in
            self?.spawn()
        }
        _ = proc
    }

    private func prepareLog(_ url: URL) {
        let fm = FileManager.default
        let directory = url.deletingLastPathComponent()
        try? fm.createDirectory(at: directory, withIntermediateDirectories: true)
        let path = url.path
        let size = (try? fm.attributesOfItem(atPath: path)[.size] as? NSNumber)?.intValue ?? 0
        if size >= 5 * 1024 * 1024 {
            let older = path + ".1"
            let oldest = path + ".2"
            try? fm.removeItem(atPath: oldest)
            try? fm.moveItem(atPath: older, toPath: oldest)
            try? fm.moveItem(atPath: path, toPath: older)
        }
        if !fm.fileExists(atPath: path) {
            fm.createFile(atPath: path, contents: nil)
        }
    }
}
