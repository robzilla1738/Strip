import XCTest

final class FixtureDecodeTests: XCTestCase {
    func testEveryRustFixtureDecodes() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let dir = root.appendingPathComponent("engine/crates/strip-ipc/fixtures")
        let files = try FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "json" }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
        XCTAssertGreaterThanOrEqual(files.count, 16)
        for file in files {
            let data = try Data(contentsOf: file)
            let line = try JSONDecoder().decode(WireLine.self, from: data)
            XCTAssertFalse(line.type.isEmpty, file.lastPathComponent)
            switch line.type {
            case "hello":
                XCTAssertEqual(line.protocolVersion, 1, file.lastPathComponent)
                if line.engineVersion != nil {
                    XCTAssertNotNil(line.axTrusted, file.lastPathComponent)
                    XCTAssertNotNil(line.privateAPI?.skylight, file.lastPathComponent)
                    XCTAssertNotNil(line.privateAPI?.axGetWindow, file.lastPathComponent)
                } else {
                    XCTAssertNotNil(line.client, file.lastPathComponent)
                }
            case "command":
                XCTAssertFalse(line.action?.isEmpty ?? true, file.lastPathComponent)
            case "query":
                XCTAssertNotNil(line.what, file.lastPathComponent)
            case "subscribe":
                XCTAssertFalse(line.events?.isEmpty ?? true, file.lastPathComponent)
            case "set_managing":
                XCTAssertNotNil(line.enabled, file.lastPathComponent)
            case "result":
                XCTAssertNotNil(line.ok, file.lastPathComponent)
            case "event":
                XCTAssertNotNil(line.event, file.lastPathComponent)
                XCTAssertNotNil(line.data, file.lastPathComponent)
            case "reload_config", "shutdown":
                XCTAssertNotNil(line.id, file.lastPathComponent)
            default:
                XCTFail("unknown type \(line.type) in \(file.lastPathComponent)")
            }
        }
    }
}
