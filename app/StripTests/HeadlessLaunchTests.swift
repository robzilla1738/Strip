import XCTest

final class HeadlessLaunchTests: XCTestCase {
    func testHeadlessStartsEngineQueriesStateAndRestarts() throws {
        let app = try locateApp()
        let info = NSDictionary(contentsOf: app.appendingPathComponent("Contents/Info.plist"))
        let bundleID = info?["CFBundleIdentifier"] as? String ?? ""
        XCTAssertFalse(bundleID.isEmpty, "built app has no bundle id")
        let write = Process()
        write.executableURL = URL(fileURLWithPath: "/usr/bin/defaults")
        write.arguments = ["write", bundleID, "hasCompletedOnboarding", "-bool", "false"]
        try write.run()
        write.waitUntilExit()
        XCTAssertEqual(write.terminationStatus, 0)
        let executable = app.appendingPathComponent("Contents/MacOS/\(app.deletingPathExtension().lastPathComponent)")
        let process = Process()
        process.executableURL = executable
        process.arguments = ["--headless-test"]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        try process.run()
        DispatchQueue.global().asyncAfter(deadline: .now() + 25) {
            if process.isRunning {
                process.terminate()
            }
        }
        process.waitUntilExit()
        let output = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        XCTAssertEqual(process.terminationStatus, 0, output)
        XCTAssertTrue(output.contains("hello.ax_trusted="), output)
        XCTAssertTrue(output.contains("query state ok"), output)
        XCTAssertTrue(output.contains("restarted"), output)
        XCTAssertTrue(output.contains("onboarding:shown"), output)
    }

    private func locateApp() throws -> URL {
        if let raw = ProcessInfo.processInfo.environment["STRIP_APP_PATH"] {
            let url = URL(fileURLWithPath: raw)
            if FileManager.default.fileExists(atPath: url.path) {
                return url
            }
        }
        let products = Bundle(for: HeadlessLaunchTests.self).bundleURL.deletingLastPathComponent()
        let apps = try FileManager.default.contentsOfDirectory(at: products, includingPropertiesForKeys: nil)
        if let app = apps.first(where: { url in
            url.pathExtension == "app" &&
                FileManager.default.fileExists(atPath: url.appendingPathComponent("Contents/MacOS/strip-engine").path)
        }) {
            return app
        }
        throw NSError(domain: "headless", code: 1, userInfo: [NSLocalizedDescriptionKey: "app not found in \(products.path)"])
    }
}
