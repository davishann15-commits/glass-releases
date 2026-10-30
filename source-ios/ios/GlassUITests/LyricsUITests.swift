import XCTest

/// Synced lyrics on the demo's sample song (no network): "-lyrics word" is LRCLIB's
/// line timing upgraded to a sample lrc.red word sync with a duet and backing vocals;
/// "-demoAt"/"-demoPaused" hold the song at a known spot.
final class LyricsUITests: XCTestCase {
    var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ args: [String]) {
        app = XCUIApplication()
        app.launchArguments = ["-demo", "-route", "nowplaying", "-pane", "lyrics"] + args
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

    private func any(_ label: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", label)).firstMatch
    }

    private func position() -> String {
        app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "Position")).firstMatch.value as? String ?? ""
    }

    /// Word timing replaces line timing, credited and linked; a tap on a line plays from it.
    func testWordTimedLyricsAndSeeking() {
        launch(["-lyrics", "word", "-demoAt", "68000", "-demoPaused"])
        XCTAssertTrue(any("lyrics / syllable sync").waitForExistence(timeout: 15), "the word sync replaced LRCLIB's line sync")
        XCTAssertTrue(app.links["lrc.red"].firstMatch.exists, "lrc.red is credited with a link")
        // Two voices at once: both lines are marked as being sung.
        let duet = any("We let the evening carry us")
        let next = any("drifting softly past the moon")
        XCTAssertTrue(duet.waitForExistence(timeout: 5) && next.exists)
        XCTAssertTrue(duet.isSelected && next.isSelected, "overlapping lines are lit together")
        XCTAssertFalse(any("Every corner hums a tune").isSelected)
        shot("lyrics-word-overlap")
        // Tap a line: the song plays from it.
        any("Every corner hums a tune").tap()
        let moved = expectation(for: NSPredicate { _, _ in self.position() == "0:57" }, evaluatedWith: nil)
        wait(for: [moved], timeout: 5)
        XCTAssertTrue(any("Every corner hums a tune").isSelected, "and it's the line being sung")
        // Backing vocals are read with their line.
        XCTAssertTrue(any("Count the windows, one by one. Backing vocals: one by one").exists)
    }

    /// Japanese timed a character at a time (NetEase): the source is named, the line is lit.
    func testCharacterTimedJapanese() {
        launch(["-lyrics", "cjk", "-demoAt", "54500", "-demoPaused"])
        XCTAssertTrue(any("lyrics / word sync").waitForExistence(timeout: 15), "NetEase's word sync replaced LRCLIB's line sync")
        XCTAssertTrue(any("netease").exists, "the source is named at the top")
        XCTAssertTrue(any("街の灯りがゆっくり消えていく").isSelected)
        XCTAssertFalse(any("どの角にも歌が聞こえる").isSelected)
        shot("lyrics-cjk")
    }

    /// Line timing alone (no word timing anywhere): lines are lit whole, LRCLIB named, no link.
    func testLineOnlyLyrics() {
        launch(["-lyrics", "line", "-demoAt", "60000", "-demoPaused"])
        XCTAssertTrue(any("lyrics / line sync").waitForExistence(timeout: 15))
        XCTAssertTrue(any("lrclib").exists)
        XCTAssertFalse(app.links["lrc.red"].exists)
        XCTAssertTrue(any("Every corner hums a tune").isSelected)
        shot("lyrics-line")
    }

    /// No lyrics anywhere: said plainly, with a search and a retry.
    func testMissingLyrics() {
        launch(["-lyrics", "none"])
        XCTAssertTrue(any("no lyrics found").waitForExistence(timeout: 20))
        XCTAssertTrue(app.buttons["search LRCLIB"].exists && app.buttons["look again"].exists)
        shot("lyrics-none")
    }

    /// Settings: the word timing services and the keys they need.
    func testWordTimingSettings() {
        app = XCUIApplication()
        app.launchArguments = ["-demo", "-route", "settings"]
        app.launch()
        let search = app.textFields["Search in Settings"].firstMatch
        XCTAssertTrue(search.waitForExistence(timeout: 15))
        search.tap()
        search.typeText("word timing\n")
        XCTAssertTrue(any("Word timing: lrc.red").waitForExistence(timeout: 5))
        XCTAssertTrue(any("Word timing: NetEase").exists)
        let spicy = app.switches["Spicy Lyrics"].firstMatch
        XCTAssertTrue(spicy.exists && !spicy.isEnabled, "Spicy Lyrics waits for a key")
        shot("settings-word-timing")
    }
}
