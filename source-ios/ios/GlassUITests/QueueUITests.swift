import XCTest

/// Now Playing's queue on demo data (as if playing on this iPhone, with Bloom and Says
/// queued): its sections, reordering by dragging, and swiping a song into the queue.
/// Screenshots go to $GLASS_SHOTS like GlassUITests'.
final class QueueUITests: XCTestCase {
    var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-demo", "-route", "nowplaying", "-pane", "queue"]
        app.launch()
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

    private func text(_ predicate: String, _ secs: TimeInterval = 10, file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
        let e = app.staticTexts.matching(NSPredicate(format: predicate)).firstMatch
        XCTAssertTrue(e.waitForExistence(timeout: secs), "didn't appear: \(predicate)", file: file, line: line)
        return e
    }

    /// Waits until `ok` holds (the queue redraws after the core answers).
    private func eventually(_ secs: TimeInterval = 5, _ ok: () -> Bool) -> Bool {
        let end = Date().addingTimeInterval(secs)
        while Date() < end {
            if ok() { return true }
            usleep(200_000)
        }
        return ok()
    }

    func testSectionsDragAndSwipe() {
        let queued = text("label == 'Next in queue'")
        let nextFrom = text("label BEGINSWITH 'Next from'")
        XCTAssertLessThan(queued.frame.minY, nextFrom.frame.minY)
        let kyoto = text("label == 'Kyoto'")
        XCTAssertGreaterThan(kyoto.frame.minY, nextFrom.frame.minY, "Kyoto comes from the playlist")
        _ = text("label BEGINSWITH 'Drag a song'")
        shot("queue-1-sections")

        // Kyoto's handle dragged up onto Bloom (the first queued song): it joins the queue.
        let handle = app.descendants(matching: .any).matching(identifier: "queue-handle-2").firstMatch
        XCTAssertTrue(handle.waitForExistence(timeout: 5))
        let bloom = text("label == 'Bloom'")
        handle.press(forDuration: 0.15, thenDragTo: bloom, withVelocity: .slow, thenHoldForDuration: 0.3)
        XCTAssertTrue(eventually { app.staticTexts.matching(NSPredicate(format: "label == 'Kyoto'")).firstMatch.frame.minY < app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Next from'")).firstMatch.frame.minY }, "Kyoto didn't move into the queue")
        shot("queue-2-dragged")

        // A song from the playlist swiped left is queued.
        // (Not Kerala: it's explicit, which the demo's settings don't play, so it can't be queued.)
        let time = app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH %@", "Time (You and I),")).firstMatch
        XCTAssertTrue(time.waitForExistence(timeout: 5))
        time.swipeLeft()
        _ = text("label CONTAINS 'Added to queue'")
        shot("queue-3-swiped")
    }
}
