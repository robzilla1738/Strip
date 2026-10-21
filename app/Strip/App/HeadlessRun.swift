import Darwin
import Foundation

enum HeadlessRun {
    static func run() -> Never {
        let report = AppPrivateAPI.check()
        if !report.missing.isEmpty {
            fail("private symbols missing: \(report.missing)")
        }
        let pid = getpid()
        let dir = "/tmp/st\(pid)"
        let socket = "\(dir)/e.sock"
        let config = "\(dir)/c.toml"
        let state = "\(dir)/state.json"
        let log = "\(dir)/engine.log"
        let fm = FileManager.default
        try? fm.removeItem(atPath: dir)
        do {
            try fm.createDirectory(atPath: dir, withIntermediateDirectories: true)
            fm.createFile(atPath: config, contents: Data())
        } catch {
            fail("workdir: \(error)")
        }
        workdir = dir

        let engine = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/strip-engine")
        guard fm.isExecutableFile(atPath: engine.path) else {
            fail("missing engine at \(engine.path)")
        }
        let supervisor = Supervisor()
        engineSupervisor = supervisor
        var environment = ProcessInfo.processInfo.environment
        if environment["STRIP_ONLY_OWNER"] == nil {
            environment["STRIP_ONLY_OWNER"] = "__strip_headless_none__"
        }
        supervisor.extraEnvironment = environment.filter { $0.key == "STRIP_ONLY_OWNER" || $0.key == "STRIP_APP_NAME" }
        if supervisor.extraEnvironment["STRIP_APP_NAME"] == nil {
            supervisor.extraEnvironment["STRIP_APP_NAME"] = Brand.current.appName
        }
        supervisor.configure(
            engine: engine,
            arguments: ["--socket", socket, "--config", config, "--state", state, "--log", log],
            log: URL(fileURLWithPath: log)
        )
        supervisor.start()

        let client = IpcClient()
        var hello: WireLine?
        let connectDeadline = Date().addingTimeInterval(8)
        while Date() < connectDeadline {
            if fm.fileExists(atPath: socket), let line = try? client.open(socketPath: socket, client: "app") {
                hello = line
                break
            }
            Thread.sleep(forTimeInterval: 0.1)
        }
        guard let hello, let trusted = hello.axTrusted else {
            fail("no hello\n\(tail(log))")
        }
        say("hello.ax_trusted=\(trusted)")
        guard let stateLine = try? client.query("state"), stateLine.ok == true else {
            fail("query state failed\n\(tail(log))")
        }
        say("query state ok")

        guard let old = supervisor.currentPID() else {
            fail("engine pid missing")
        }
        let killed = Date()
        kill(old, SIGKILL)
        var restarted = false
        while Date().timeIntervalSince(killed) < 1 {
            if let pid = supervisor.currentPID(), pid != old {
                restarted = true
                say(String(format: "restarted in %.3fs", Date().timeIntervalSince(killed)))
                break
            }
            Thread.sleep(forTimeInterval: 0.02)
        }
        guard restarted else {
            fail("engine was not restarted within 1s\n\(tail(log))")
        }
        let onboarding = !UserDefaults.standard.bool(forKey: "hasCompletedOnboarding")
        say("onboarding:\(onboarding ? "shown" : "hidden")")
        finish(0)
    }

    private static var engineSupervisor: Supervisor?
    private static var workdir: String?

    private static func say(_ line: String) {
        fputs(line + "\n", stdout)
        fflush(stdout)
    }

    private static func fail(_ message: String) -> Never {
        fputs(message + "\n", stderr)
        fflush(stderr)
        finish(1)
    }

    private static func finish(_ code: Int32) -> Never {
        engineSupervisor?.suppressRestart()
        if let pid = engineSupervisor?.currentPID() {
            kill(pid, SIGTERM)
        }
        engineSupervisor?.waitUntilExit(timeout: 2)
        engineSupervisor?.forceTerminate()
        if let workdir {
            try? FileManager.default.removeItem(atPath: workdir)
        }
        exit(code)
    }

    private static func tail(_ path: String) -> String {
        guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { return "" }
        return String(text.suffix(2000))
    }
}
