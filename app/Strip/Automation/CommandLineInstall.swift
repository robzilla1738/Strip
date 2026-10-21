import Foundation

enum CommandLineInstall {
    /// Prefer the distinct CLI copy. On a case-insensitive volume the app executable
    /// and a file named `strip` are the same inode, so the bundle stores `strip-cli`.
    static func source(in macOS: URL, fileExists: (String) -> Bool) -> String? {
        let cli = macOS.appendingPathComponent("strip-cli").path
        if fileExists(cli) { return cli }
        let plain = macOS.appendingPathComponent("strip").path
        if fileExists(plain) { return plain }
        return nil
    }

    static func install() {
        let macOS = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS")
        guard let source = source(in: macOS, fileExists: { FileManager.default.fileExists(atPath: $0) }) else { return }
        if linkAsAdmin(source) { return }
        linkFallback(source)
    }

    private static func linkAsAdmin(_ source: String) -> Bool {
        let script = "do shell script \"mkdir -p /usr/local/bin && ln -sf \" & quoted form of \"\(source)\" & \" /usr/local/bin/strip\" with administrator privileges"
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", script]
        let pipe = Pipe()
        process.standardError = pipe
        process.standardOutput = pipe
        do { try process.run() } catch { return false }
        process.waitUntilExit()
        return process.terminationStatus == 0
    }

    private static func linkFallback(_ source: String) {
        let directory = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".local/bin", isDirectory: true)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let dest = directory.appendingPathComponent("strip")
        try? FileManager.default.removeItem(at: dest)
        try? FileManager.default.createSymbolicLink(at: dest, withDestinationURL: URL(fileURLWithPath: source))
    }
}
