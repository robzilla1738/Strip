import XCTest

final class URLCommandTests: XCTestCase {
    func testFocusIndexReadsHostAndOneBasedN() {
        let url = URL(string: "strip://focus_index?n=2")!
        let command = URLCommand.parse(url, scheme: "strip")
        XCTAssertEqual(command, URLCommand(action: "focus_index", args: .object(["n": .int(2)])))
    }

    func testReloadConfigHasNoArguments() {
        let url = URL(string: "strip://reload_config")!
        let command = URLCommand.parse(url, scheme: "strip")
        XCTAssertEqual(command?.action, "reload_config")
        XCTAssertEqual(command?.args, .object([:]))
    }

    func testOtherSchemeIsIgnored() {
        let url = URL(string: "other://focus_index?n=2")!
        XCTAssertNil(URLCommand.parse(url, scheme: "strip"))
    }

    func testCLISourcePrefersDistinctBinary() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        FileManager.default.createFile(atPath: dir.appendingPathComponent("strip-cli").path, contents: Data("cli".utf8))
        FileManager.default.createFile(atPath: dir.appendingPathComponent("strip").path, contents: Data("app".utf8))
        let path = CommandLineInstall.source(in: dir, fileExists: { FileManager.default.fileExists(atPath: $0) })
        XCTAssertEqual(path.map { URL(fileURLWithPath: $0).lastPathComponent }, "strip-cli")
    }
}
