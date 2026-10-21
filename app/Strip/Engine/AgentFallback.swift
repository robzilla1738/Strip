import Foundation
import ServiceManagement

enum AgentFallback {
    /// Register the bundled LaunchAgent when the app is trusted and the child
    /// engine is not. The plist name is `\(bundleID).engine.plist`.
    static func register(bundleID: String) {
        guard !bundleID.isEmpty, !bundleID.contains("{") else { return }
        do {
            try SMAppService.agent(plistName: "\(bundleID).engine.plist").register()
        } catch {
            fputs("launch agent: \(error)\n", stderr)
        }
    }
}
