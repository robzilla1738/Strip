import Foundation

struct AppPaths {
    var support: URL
    var config: URL
    var state: URL
    var socket: URL
    var log: URL

    static func standard(appName: String) -> AppPaths {
        let fm = FileManager.default
        let support = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent(appName, isDirectory: true)
        let logs = fm.urls(for: .libraryDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Logs", isDirectory: true)
            .appendingPathComponent(appName, isDirectory: true)
        return AppPaths(
            support: support,
            config: support.appendingPathComponent("config.toml"),
            state: support.appendingPathComponent("state.json"),
            socket: support.appendingPathComponent("engine.sock"),
            log: logs.appendingPathComponent("engine.log")
        )
    }

    func prepare() throws {
        let fm = FileManager.default
        try fm.createDirectory(at: support, withIntermediateDirectories: true)
        try fm.createDirectory(at: log.deletingLastPathComponent(), withIntermediateDirectories: true)
        if !fm.fileExists(atPath: config.path) {
            fm.createFile(atPath: config.path, contents: Data())
        }
        if !fm.fileExists(atPath: log.path) {
            fm.createFile(atPath: log.path, contents: Data())
        }
    }
}
