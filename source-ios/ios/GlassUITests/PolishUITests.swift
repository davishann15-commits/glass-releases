import XCTest

/// Now Playing's playlist pane (scrolled to the song that's playing; songs swipe
/// right to join the queue) and Local Files' other devices (browse one, pick songs,
/// get them), on the demo's sample data. Screenshots go to $GLASS_SHOTS like GlassUITests'.
final class PolishUITests: XCTestCase {
    var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ args: [String]) {
        app = XCUIApplication()
        app.launchArguments = ["-demo"] + args
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

    private func text(_ s: String) -> XCUIElement {
        app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", s)).firstMatch
    }

    func testPlaylistPaneFollowsTheSongAndSwipesRightToQueue() {
        // The 14th of the playlist's 15 songs is playing.
        launch(["-route", "nowplaying", "-pane", "playlist", "-demoTrack", "13"])
        let playing = app.staticTexts["Morning Light"].firstMatch
        XCTAssertTrue(playing.waitForExistence(timeout: 20))
        sleep(2)
        shot("polish-1-playlist-pane")
        XCTAssertTrue(playing.isHittable, "the pane opens on the song that's playing")
        XCTAssertFalse(app.staticTexts["Bloom"].firstMatch.isHittable, "not at the top")

        // Swipe a song right, as in Spotify: it joins the queue.
        let row = app.staticTexts["Coastline"].firstMatch
        XCTAssertTrue(row.isHittable)
        let from = row.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5))
        from.press(forDuration: 0.05, thenDragTo: from.withOffset(CGVector(dx: 220, dy: 0)), withVelocity: .slow, thenHoldForDuration: 0.1)
        XCTAssertTrue(text("Added to queue").waitForExistence(timeout: 5), "swiping right queues it")
        shot("polish-2-swiped-right")
    }

    func testBrowseAnotherDeviceAndGetSongs() {
        launch(["-route", "local"])
        let device = app.buttons.containing(NSPredicate(format: "label CONTAINS 'Arch Desktop'")).firstMatch
        XCTAssertTrue(device.waitForExistence(timeout: 20), "the other device shows in Local Files")
        shot("polish-3-local-devices")
        device.tap()
        let song = app.buttons.containing(NSPredicate(format: "label CONTAINS 'Stay With Me'")).firstMatch
        XCTAssertTrue(song.waitForExistence(timeout: 10), "its songs are listed")
        XCTAssertTrue(text("on this iPhone").exists, "songs already here are marked")
        song.tap()
        app.buttons.containing(NSPredicate(format: "label CONTAINS 'Sparkle'")).firstMatch.tap()
        XCTAssertTrue(text("2 songs").waitForExistence(timeout: 3))
        shot("polish-4-device-picked")
        app.buttons["Get 2 songs"].firstMatch.tap()
        XCTAssertTrue(text("Getting 2 songs from Arch Desktop").waitForExistence(timeout: 5))
    }

    /// The bottom bar takes the theme: a colored preset, then the bar photographed.
    func testBottomBarFollowsTheTheme() {
        launch(["-route", "settings"])
        let search = app.textFields["Search in Settings"].firstMatch
        XCTAssertTrue(search.waitForExistence(timeout: 20))
        search.tap()
        search.typeText("theme")
        app.keyboards.buttons["search"].firstMatch.tap()
        let customize = app.buttons.matching(NSPredicate(format: "label == 'Customize'")).allElementsBoundByIndex.first { $0.frame.width > 0 }
        XCTAssertNotNil(customize)
        customize?.tap()
        let rose = app.buttons["Preset rose"].firstMatch
        XCTAssertTrue(rose.waitForExistence(timeout: 5))
        rose.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        sleep(1)
        app.buttons["Done"].firstMatch.tap()
        app.buttons["library"].firstMatch.tap()
        sleep(2)
        shot("polish-5-bar-rose")
        // Back to the default theme for the other tests.
        app.buttons["settings"].firstMatch.tap()
        let again = app.buttons.matching(NSPredicate(format: "label == 'Customize'")).allElementsBoundByIndex.first { $0.frame.width > 0 }
        again?.tap()
        let glass = app.buttons["Preset glass"].firstMatch
        if glass.waitForExistence(timeout: 5) { glass.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap() }
        sleep(1)
    }
}
