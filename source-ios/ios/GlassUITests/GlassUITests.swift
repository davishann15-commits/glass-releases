import XCTest

/// UI tests in the simulator, on the demo's sample data (no account, no network)
/// and, for the assistant, a local mock of OpenAI's API (scripts/mock-openai.py on
/// port 8765). Screenshots go to $GLASS_SHOTS (pass TEST_RUNNER_GLASS_SHOTS to
/// xcodebuild) and into the result bundle.
final class GlassUITests: XCTestCase {
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

    private func waitFor(_ e: XCUIElement, _ secs: TimeInterval = 15, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(e.waitForExistence(timeout: secs), "didn't appear: \(e)", file: file, line: line)
    }

    /// Scrolls until `e` exists (lazy stacks build rows only near the screen).
    private func reveal(_ e: XCUIElement, _ swipes: Int = 8) {
        for _ in 0..<swipes where !(e.exists && e.isHittable) { app.swipeUp() }
        waitFor(e)
    }

    /// Taps at the element's center as a finger would. (iOS 26 scales a sheet's content
    /// at its smaller height, and XCUITest's hittability check doesn't follow the scale.)
    private func tapWhenReady(_ e: XCUIElement, _ secs: TimeInterval = 5) {
        waitFor(e, secs)
        usleep(400_000)
        // The keyboard shortcuts' hidden buttons share some labels: take one that's on screen.
        let same = app.buttons.matching(NSPredicate(format: "label == %@", e.label)).allElementsBoundByIndex
        let target = same.first { $0.frame.minX.isFinite && $0.frame.width > 0 } ?? e
        target.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    }

    /// The on-screen button with this label (the keyboard shortcuts' hidden buttons share some labels).
    private func visibleButton(_ label: String, _ swipes: Int = 8) -> XCUIElement {
        for _ in 0...swipes {
            let all = app.buttons.matching(NSPredicate(format: "label == %@", label)).allElementsBoundByIndex
            if let b = all.first(where: { $0.exists && $0.frame.width > 0 && $0.frame.minX.isFinite }) { return b }
            app.swipeUp()
        }
        XCTFail("no visible button \(label)")
        return app.buttons[label]
    }

    func text(_ s: String) -> XCUIElement {
        app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", s)).firstMatch
    }

    /// Ask → the model searches the local library, proposes a playlist and asks to
    /// save it → the user saves it (a real local playlist) → undo removes it.
    func testAssistantPreviewSaveAndUndo() {
        launch(["-assistantMock", "http://127.0.0.1:8765/v1"])
        let open = app.buttons.matching(identifier: "assistant-button").firstMatch
        waitFor(open)
        open.tap()
        let input = app.descendants(matching: .any)["assistant-input"].firstMatch
        waitFor(input)
        input.tap()
        input.typeText("make me a city pop playlist for a night drive")
        app.buttons["send"].firstMatch.tap()
        waitFor(text("The assistant asks to save this as a new playlist."), 30)
        waitFor(text("Night Drive"))
        shot("assistant-preview")
        app.buttons["save"].firstMatch.tap()
        waitFor(text("saved ✓"))
        waitFor(text("Undo is available"))
        shot("assistant-saved")
        app.buttons["undo"].firstMatch.tap()
        waitFor(text("Undone"))
        shot("assistant-undone")
    }

    /// Ask for recommendations → the model reads a playlist and recommends songs for
    /// "Road Trip", leaving out the ones already in it → ▶ and + reach the core (the
    /// demo has no preview clips and sends nothing to Spotify, and says so).
    func testAssistantRecommendations() {
        launch(["-assistantMock", "http://127.0.0.1:8765/v1"])
        let open = app.buttons.matching(identifier: "assistant-button").firstMatch
        waitFor(open)
        open.tap()
        let input = app.descendants(matching: .any)["assistant-input"].firstMatch
        waitFor(input)
        input.tap()
        input.typeText("recommend some songs for my road trip playlist")
        app.buttons["send"].firstMatch.tap()
        waitFor(text("For Road Trip"), 30)
        waitFor(text("none already in it"))
        XCTAssertTrue(text("Paper Planes").exists, "the 11th song, not in Road Trip")
        XCTAssertFalse(text("Driftwood").exists, "the 9th is already in Road Trip, so it's left out")
        shot("assistant-recommendations")
        app.buttons["Play a 30-second preview of Paper Planes"].firstMatch.tap()
        waitFor(text("no preview"))
        app.buttons["Add Paper Planes to Road Trip"].firstMatch.tap()
        waitFor(text("Couldn't add to “Road Trip”"))
        app.buttons["dismiss"].firstMatch.tap()
        XCTAssertTrue(text("For Road Trip").waitForNonExistence(timeout: 5))
    }

    /// Settings › Assistant, off and on (the demo keeps nothing).
    func testAssistantSettings() {
        launch(["-route", "settings"])
        let search = app.textFields["Search in Settings"].firstMatch
        waitFor(search)
        search.tap()
        search.typeText("assistant")
        waitFor(text("Turn on the assistant"))
        shot("settings-assistant-off")
    }

    /// Settings › Activity: what Glass stores (measured) and the request log.
    func testActivity() {
        launch(["-route", "settings"])
        let search = app.textFields["Search in Settings"].firstMatch
        waitFor(search)
        search.tap()
        search.typeText("activity")
        let open = app.buttons["Open"].firstMatch
        waitFor(open)
        open.tap()
        waitFor(text("What Glass stores"))
        waitFor(text("settings.json"))
        shot("activity")
        for _ in 0..<6 where !text("requests (").exists { app.swipeUp() }
        waitFor(text("requests ("))
        shot("activity-log")
    }

    /// The command bar: a command by its words, then a library item.
    func testCommandBar() {
        launch([])
        let open = app.buttons.matching(identifier: "command-button").firstMatch
        waitFor(open)
        open.tap()
        let input = app.textFields["command-input"].firstMatch
        waitFor(input)
        input.typeText("hearts")
        waitFor(text("liked songs"))
        shot("command-bar")
        input.typeText("\n")
        waitFor(text("Liked Songs"))
        shot("command-liked-songs")
    }

    /// A playlist's featured artists; an artist's discography chips.
    func testFeaturedAndDiscography() {
        launch(["-route", "playlist"])
        waitFor(app.staticTexts["Late Night Drive"].firstMatch)
        for _ in 0..<8 where !text("Featured artists").exists { app.swipeUp() }
        waitFor(text("Featured artists"))
        shot("playlist-featured-artists")
        app.terminate()
        launch(["-route", "artist"])
        let chip = app.buttons["Singles and EPs"].firstMatch
        for _ in 0..<6 where !chip.exists { app.swipeUp() }
        waitFor(chip)
        chip.tap()
        shot("artist-discography-singles")
    }

    /// ChatGPT sign-in's iOS half against the mock: the web sign-in sheet opens, the
    /// mock "consents" and sends it to the core's loopback, the sheet closes, and the
    /// code is exchanged (the mock refuses it, so the refusal is what shows).
    func testAssistantSignInPlumbing() {
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        launch(["-assistantMock", "http://127.0.0.1:8765/v1", "-route", "settings"])
        let search = app.textFields["Search in Settings"].firstMatch
        waitFor(search)
        search.tap()
        search.typeText("assistant")
        search.typeText("\n")
        let plan = app.buttons["my ChatGPT plan"].firstMatch
        reveal(plan)
        plan.tap()
        let go = app.buttons["Continue with ChatGPT"].firstMatch
        reveal(go)
        go.tap()
        // iOS asks before a sign-in sheet that shares Safari's cookies.
        let allow = springboard.buttons["Continue"]
        if allow.waitForExistence(timeout: 8) { allow.tap() }
        // The mock refuses the code with invalid_grant, which Glass words as:
        let note = text("The sign-in code expired")
        let deadline = Date().addingTimeInterval(30)
        while !note.exists && Date() < deadline { app.swipeUp(); sleep(1) }
        waitFor(note)
        shot("assistant-signin-plumbing")
    }

    /// Evidence: the main screens, one launch each (demo data).
    func testTour() {
        let stops: [(String, [String], String)] = [
            ("tour-01-library", [], "library"),
            ("tour-02-home", ["-route", "home"], "DJ"),
            ("tour-03-search", ["-route", "search"], "Browse all"),
            ("tour-04-playlist", ["-route", "playlist"], "Late Night Drive"),
            ("tour-05-album", ["-route", "album"], "Quiet Hours"),
            ("tour-06-artist", ["-route", "artist"], "Popular"),
            ("tour-07-now-playing-lyrics", ["-route", "nowplaying", "-pane", "lyrics", "-lyrics", "word"], "Golden Hour"),
            ("tour-08-now-playing-queue", ["-route", "nowplaying", "-pane", "queue"], "Golden Hour"),
            ("tour-09-local-files", ["-route", "local"], "local files"),
            ("tour-10-settings", ["-route", "settings"], "Account"),
        ]
        for (name, args, expect) in stops {
            launch(args)
            waitFor(text(expect), 20)
            sleep(2)
            shot(name)
            app.terminate()
        }
    }

    /// The theme manager: a preset, a lyrics choice and album colors everywhere apply.
    func testThemeManager() {
        launch(["-route", "settings"])
        let search = app.textFields["Search in Settings"].firstMatch
        waitFor(search)
        search.tap()
        search.typeText("theme")
        search.typeText("\n")
        visibleButton("Customize").tap()
        let midnight = app.buttons["Preset midnight"].firstMatch
        tapWhenReady(midnight)
        let accentHex = app.textFields["accent, hex"].firstMatch
        waitFor(accentHex)
        let deadline = Date().addingTimeInterval(5)
        while (accentHex.value as? String) != "#7AA2F7" && Date() < deadline { usleep(200_000) }
        XCTAssertEqual(accentHex.value as? String, "#7AA2F7", "the preset's accent applied")
        shot("theme-1-midnight")
        // Full height, so the lyrics rows are on screen.
        let done = app.buttons["Done"].firstMatch
        done.coordinate(withNormalizedOffset: CGVector(dx: -2, dy: 0.5)).press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.02)))
        sleep(1)
        tapWhenReady(app.buttons["lyrics"].firstMatch)
        let left = app.buttons["left"].firstMatch
        tapWhenReady(left)
        let selected = Date().addingTimeInterval(5)
        while !left.isSelected && Date() < selected { usleep(200_000) }
        XCTAssertTrue(left.isSelected, "lyrics align = left was saved")
        shot("theme-2-lyrics-left")
        tapWhenReady(app.buttons["album"].firstMatch)
        let everywhere = app.buttons["everywhere"].firstMatch
        tapWhenReady(everywhere)
        let on = Date().addingTimeInterval(5)
        while !everywhere.isSelected && Date() < on { usleep(200_000) }
        XCTAssertTrue(everywhere.isSelected, "album colors apply everywhere")
        sleep(2)
        shot("theme-3-album-everywhere")
        // Back to the defaults for the next tests.
        tapWhenReady(app.buttons["Now Playing"].firstMatch)
        tapWhenReady(app.buttons["lyrics"].firstMatch)
        tapWhenReady(app.buttons["center"].firstMatch)
        tapWhenReady(app.buttons["colors"].firstMatch)
        tapWhenReady(app.buttons["Preset glass"].firstMatch)
        sleep(1)
    }

    /// The player bar: swipe up opens Now Playing, pulling it down closes it; swipes
    /// left and right change the song.
    func testPlayerGestures() {
        launch([])
        let bar = app.descendants(matching: .any)["player-bar"].firstMatch
        waitFor(bar)
        let first = bar.label
        bar.swipeLeft()
        let changed = Date().addingTimeInterval(5)
        while bar.label == first && Date() < changed { usleep(200_000) }
        XCTAssertNotEqual(bar.label, first, "swiping the bar left skipped to the next song")
        bar.swipeRight()
        let back = Date().addingTimeInterval(5)
        while bar.label != first && Date() < back { usleep(200_000) }
        XCTAssertEqual(bar.label, first, "swiping right went back")
        bar.swipeUp()
        waitFor(text("now-playing"))
        shot("gesture-now-playing-open")
        let top = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.1))
        top.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.75)), withVelocity: .default, thenHoldForDuration: 0)
        let closed = Date().addingTimeInterval(5)
        while text("now-playing").exists && Date() < closed { usleep(200_000) }
        XCTAssertFalse(text("now-playing").exists, "pulling Now Playing down closed it")
    }

    /// A song swiped left is queued, not played; vertical scrolling still works.
    func testRowSwipeToQueue() {
        launch(["-route", "playlist"])
        waitFor(app.staticTexts["Late Night Drive"].firstMatch)
        let bar = app.descendants(matching: .any)["player-bar"].firstMatch
        let playing = bar.label
        let row = app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH %@", "Bloom")).firstMatch
        waitFor(row)
        row.swipeLeft()
        waitFor(text("Added to queue"), 5)
        shot("gesture-row-queued")
        sleep(1)
        XCTAssertEqual(bar.label, playing, "the swiped song didn't start playing")
        // Scrolling still works: a gentle drag up brings a later song into view.
        let later = app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH %@", "Pale Blue")).firstMatch
        for _ in 0..<4 where !(later.exists && later.isHittable) {
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.7)).press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45)), withVelocity: .slow, thenHoldForDuration: 0.2)
        }
        XCTAssertTrue(later.isHittable, "the list still scrolls")
    }

    /// Swiping back: from the left edge, and from the middle of the page.
    /// Swiping right goes back from anywhere on a page, rows included (they swipe left to queue).
    func testSwipeBack() {
        let cases: [(String, [String], String, Double, Double)] = [
            ("edge", ["-route", "playlist"], "Late Night Drive", 0.01, 0.45),
            ("playlist-header", ["-route", "playlist"], "Late Night Drive", 0.35, 0.22),
            ("playlist-rows", ["-route", "playlist"], "Late Night Drive", 0.35, 0.6),
            ("album-rows", ["-route", "album"], "Quiet Hours", 0.35, 0.6),
            ("liked-rows", ["-route", "liked"], "Liked Songs", 0.35, 0.6),
        ]
        for (name, args, title, x, y) in cases {
            launch(args)
            waitFor(app.staticTexts[title].firstMatch)
            let from = app.coordinate(withNormalizedOffset: CGVector(dx: x, dy: y))
            let speed = ProcessInfo.processInfo.environment["SWIPE_SPEED"].flatMap(Double.init).map { XCUIGestureVelocity(CGFloat($0)) } ?? .slow
            from.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.95, dy: y)), withVelocity: speed, thenHoldForDuration: 0)
            sleep(1)
            // The greeting (good morning, afternoon or evening) heads the library and home pages.
            let back = text("good ").exists
            print("SWIPE \(name): back=\(back)")
            if !back { shot("swipe-back-\(name)-failed") }
            XCTAssertTrue(back, "swipe back from \(name)")
            app.terminate()
        }
    }


    /// Lyrics translation shown under each line and in place of it. The simulator has
    /// no on-device translation (every pair reports unsupported), so `-mockTranslate`
    /// stands in for Apple's engine; the rest is the real path (core → app → lyrics).
    func testLyricsTranslation() {
        launch(["-mockTranslate", "-lyrics", "word", "-route", "settings"])
        let search = app.textFields["Search in Settings"].firstMatch
        waitFor(search)
        search.tap()
        search.typeText("translat")
        search.typeText("\n")
        tapWhenReady(app.buttons["original + translation"].firstMatch)
        // "Translate into": "system" at first (or what an earlier run left).
        let into = app.textFields.matching(NSPredicate(format: "value IN %@", ["system", "es"])).firstMatch
        waitFor(into)
        into.tap()
        into.press(forDuration: 1.0)
        if app.menuItems["Select All"].waitForExistence(timeout: 2) { app.menuItems["Select All"].tap() }
        into.typeText("es\n")
        let bar = app.descendants(matching: .any)["player-bar"].firstMatch
        bar.tap()
        waitFor(text("[es] "), 20)
        shot("translation-below")
        // 文A again: the translation in place of the lyrics.
        let mode = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Translation:")).firstMatch
        mode.tap()
        sleep(1)
        shot("translation-only")
        mode.tap()  // back to the original
        // Back to the defaults.
        app.buttons["Close Now Playing"].firstMatch.tap()
        let back = app.textFields.matching(NSPredicate(format: "value == %@", "es")).firstMatch
        if back.waitForExistence(timeout: 5) {
            back.tap()
            back.press(forDuration: 1.0)
            if app.menuItems["Select All"].waitForExistence(timeout: 2) { app.menuItems["Select All"].tap() }
            back.typeText("system\n")
        }
        tapWhenReady(app.buttons["original"].firstMatch)
    }
}
