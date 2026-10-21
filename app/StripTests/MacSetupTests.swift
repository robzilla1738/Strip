import XCTest

final class MacSetupTests: XCTestCase {
    func testEachCheckParserPassesAndFailsWithItsFixURL() {
        let accessibility = MacSetup.accessibility(appTrusted: true, engineTrusted: true)
        XCTAssertTrue(accessibility.passed)
        XCTAssertEqual(accessibility.fixURL, FixLink.accessibility)
        let blocked = MacSetup.accessibility(appTrusted: true, engineTrusted: false)
        XCTAssertFalse(blocked.passed)
        XCTAssertEqual(blocked.fixURL, FixLink.accessibility)

        let four = MacSetup.swipeFour(gesture: 0)
        XCTAssertTrue(four.passed)
        XCTAssertEqual(four.fixURL, FixLink.trackpad)
        XCTAssertFalse(MacSetup.swipeFour(gesture: 2).passed)
        XCTAssertFalse(MacSetup.swipeFour(gesture: nil).passed)

        let three = MacSetup.swipeThree(horiz: 0, drag: 0)
        XCTAssertTrue(three.passed)
        XCTAssertEqual(three.fixURL, FixLink.trackpad)
        XCTAssertFalse(MacSetup.swipeThree(horiz: 0, drag: 1).passed)

        let spaces = MacSetup.separateSpaces(spansDisplays: false)
        XCTAssertTrue(spaces.passed)
        XCTAssertEqual(spaces.fixURL, FixLink.desktop)
        XCTAssertFalse(MacSetup.separateSpaces(spansDisplays: true).passed)

        let mru = MacSetup.rearrangeSpaces(mru: false)
        XCTAssertTrue(mru.passed)
        XCTAssertEqual(mru.fixURL, FixLink.desktop)
        XCTAssertFalse(MacSetup.rearrangeSpaces(mru: true).passed)

        let stage = MacSetup.stageManager(enabled: false)
        XCTAssertTrue(stage.passed)
        XCTAssertEqual(stage.fixURL, FixLink.desktop)
        XCTAssertFalse(MacSetup.stageManager(enabled: true).passed)

        XCTAssertNil(MacSetup.tiling(edge: true, top: true, accelerator: true, major: 14))
        let tiling = MacSetup.tiling(edge: false, top: false, accelerator: false, major: 15)
        XCTAssertEqual(tiling?.passed, true)
        XCTAssertEqual(tiling?.fixURL, FixLink.desktop)
        XCTAssertEqual(MacSetup.tiling(edge: true, top: false, accelerator: false, major: 15)?.passed, false)

        let stacked = MacSetup.arrangement(frames: [CGRect(x: 0, y: 0, width: 100, height: 80), CGRect(x: 0, y: 80, width: 100, height: 80)])
        XCTAssertTrue(stacked.passed)
        XCTAssertTrue(stacked.warnOnly)
        XCTAssertEqual(stacked.fixURL, FixLink.displays)
        let beside = MacSetup.arrangement(frames: [CGRect(x: 0, y: 0, width: 100, height: 80), CGRect(x: 100, y: 0, width: 100, height: 80)])
        XCTAssertFalse(beside.passed)
        XCTAssertTrue(beside.warnOnly)

        let clear = MacSetup.conflicts(running: [(name: "Finder", bundleID: "com.apple.finder")])
        XCTAssertTrue(clear.passed)
        XCTAssertNil(clear.fixURL)
        let hit = MacSetup.conflicts(running: [(name: "Rectangle", bundleID: "com.knollsoft.Rectangle")])
        XCTAssertFalse(hit.passed)
        XCTAssertNil(hit.fixURL)
        XCTAssertEqual(hit.quit.map(\.bundleID), ["com.knollsoft.Rectangle"])
    }

    func testControlAccessNameFollowsTheOS() {
        XCTAssertEqual(ControlAccess.paneName(major: 13), "Accessibility")
        XCTAssertEqual(ControlAccess.paneName(major: 26), "Accessibility")
        XCTAssertEqual(ControlAccess.paneName(major: 27), "Device Control and Data Access")
        XCTAssertTrue(FixLink.accessibility.contains("PrivacySecurity.extension"))
        XCTAssertTrue(FixLink.accessibility.contains("Privacy_Accessibility"))
        XCTAssertFalse(FixLink.accessibility.contains("preference.security"))
    }

    func testTourAdvancesOnSyntheticEvents() {
        let tour = TourProgress()
        tour.receive("focus_right")
        XCTAssertEqual(tour.index, 1)
        tour.receive("cycle_width")
        XCTAssertEqual(tour.index, 1)
        tour.receive("move_left")
        tour.receive("cycle_width")
        tour.receive("stack_left")
        tour.receive("swipe")
        XCTAssertTrue(tour.finished)
        tour.receive("focus_left")
        XCTAssertTrue(tour.finished)
    }
}