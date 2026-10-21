import CoreGraphics
import Foundation

let owner = CommandLine.arguments.dropFirst().first ?? ""
let options = CGWindowListOption([.optionOnScreenOnly, .excludeDesktopElements])
let info = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] ?? []
var rows: [[String: Any]] = []
for window in info {
    let name = window[kCGWindowOwnerName as String] as? String ?? ""
    if !owner.isEmpty && name != owner { continue }
    let bounds = window[kCGWindowBounds as String] as? [String: Any] ?? [:]
    rows.append([
        "owner": name,
        "id": window[kCGWindowNumber as String] as? Int ?? 0,
        "pid": window[kCGWindowOwnerPID as String] as? Int ?? 0,
        "x": bounds["X"] as? Double ?? 0,
        "y": bounds["Y"] as? Double ?? 0,
        "w": bounds["Width"] as? Double ?? 0,
        "h": bounds["Height"] as? Double ?? 0,
    ])
}
let data = try JSONSerialization.data(withJSONObject: rows, options: [])
FileHandle.standardOutput.write(data)
FileHandle.standardOutput.write(Data([0x0A]))
