import Darwin
import Foundation

enum AppPrivateAPI {
    struct Report: Equatable {
        var missing: [String]
    }

    /// Startup check. The app's required list is empty; window-server symbols
    /// are loaded by the engine. This function is the app's only `dlsym` site.
    static func check(required: [String] = []) -> Report {
        Report(missing: required.filter { load($0) == nil })
    }

    static func load(_ symbol: String) -> UnsafeMutableRawPointer? {
        guard let handle = UnsafeMutableRawPointer(bitPattern: -2) else {
            return nil
        }
        return symbol.withCString { dlsym(handle, $0) }
    }
}
