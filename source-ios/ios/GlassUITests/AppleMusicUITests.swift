import XCTest

/// Add to Apple Music in the demo (no device: the core walks through the steps):
/// picking songs in Local Files, the sheet, a run's progress and result, and the
/// Settings row. Screenshots go to $GLASS_SHOTS like GlassUITests'.
final class AppleMusicUITests: XCTestCase {
    var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
    }

    private func shot(_ name: String) {
        let s = XCUIScreen.main.screenshot()
        let a = XCTAttachment(screenshot: s)
        a.name = name
        a.lifetime = .keepAlways
        add(a)
        if let dir = ProcessInfo.processInfo.environment["GLASS_SHOTS"] {
            try? s.pngRepresentation.write(to: URL(fileURLWithPath: dir).appendingPathComponent("\(name).png"))
        }
    }

    /// The on-screen element among those matching (keyboard shortcuts' hidden buttons share labels).
    private func onScreen(_ q: XCUIElementQuery, _ secs: TimeInterval = 10, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let end = Date().addingTimeInterval(secs)
        repeat {
            if let e = q.allElementsBoundByIndex.first(where: { $0.exists && $0.frame.width > 0 && $0.frame.minX.isFinite && app.frame.intersects($0.frame) }) { return e }
            usleep(250_000)
        } while Date() < end
        XCTFail("not on screen: \(q)", file: file, line: line)
        return q.firstMatch
    }

    /// A text that appears (and may change soon after).
    private func appears(_ predicate: String, _ secs: TimeInterval = 10, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: predicate)).firstMatch.waitForExistence(timeout: secs), "didn't appear: \(predicate)", file: file, line: line)
    }

    private func tap(_ e: XCUIElement) {
        usleep(300_000)
        e.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    }

    private func button(_ label: String) -> XCUIElement {
        onScreen(app.buttons.matching(NSPredicate(format: "label == %@", label)))
    }

    func testPickSongsAndAddThem() {
        app.launchArguments = ["-demo", "-route", "local"]
        app.launch()
        tap(button("add to Apple Music"))
        tap(button("Songs"))
        // Song rows read "title, artist · album, 3:07".
        let rows = app.buttons.matching(NSPredicate(format: "label MATCHES %@", ".+, .+, [0-9]+:[0-9]{2}$"))
        tap(onScreen(rows))
        let second = rows.allElementsBoundByIndex.filter { $0.frame.width > 0 && app.frame.intersects($0.frame) }
        XCTAssertGreaterThan(second.count, 1)
        tap(second[1])
        XCTAssertTrue(onScreen(app.staticTexts.matching(NSPredicate(format: "label == '2 songs'"))).exists)
        XCTAssertEqual(rows.allElementsBoundByIndex.filter { $0.isSelected }.count, 2)
        shot("applemusic-1-picked")

        tap(button("add"))
        XCTAssertTrue(onScreen(app.staticTexts.matching(NSPredicate(format: "label == %@", "Add to Apple Music"))).exists)
        XCTAssertTrue(onScreen(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Adds 2 songs'"))).exists)
        XCTAssertTrue(onScreen(app.staticTexts.matching(NSPredicate(format: "label == 'Not set up yet.'"))).exists)
        shot("applemusic-2-sheet")

        tap(button("Add 2 songs"))
        appears("label BEGINSWITH 'Copying songs'")
        shot("applemusic-3-copying")
        appears("label BEGINSWITH 'Added 2 songs.'", 15)
        shot("applemusic-4-done")

        // Closing the sheet leaves picking (the songs were handed over).
        tap(button("Close"))
        XCTAssertFalse(app.buttons.matching(NSPredicate(format: "label == 'done'")).allElementsBoundByIndex.contains { $0.frame.width > 0 && app.frame.intersects($0.frame) })
    }

    func testSettingsOpensTheSetup() {
        app.launchArguments = ["-demo", "-route", "settings"]
        app.launch()
        let search = onScreen(app.textFields.matching(NSPredicate(format: "placeholderValue == 'Search in Settings'")))
        tap(search)
        search.typeText("apple music")
        tap(button("Set up"))
        XCTAssertTrue(onScreen(app.staticTexts.matching(NSPredicate(format: "label == %@", "Add to Apple Music"))).exists)
        XCTAssertTrue(onScreen(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Get LocalDevVPN'"))).exists)
        shot("applemusic-5-setup")
    }
}
