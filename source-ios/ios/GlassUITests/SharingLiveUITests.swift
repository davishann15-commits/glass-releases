import XCTest

/// Local files sharing for real, over the network, with Glass on a computer: the
/// desktop demo started with `GLASS_DEMO_SHARE` (both sign in as the demo account,
/// which only demos have). Skipped unless `GLASS_LIVE_SHARE` is set to the computer's
/// name; `scripts/live-share.sh` sets everything up and runs it.
final class SharingLiveUITests: XCTestCase {
    var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
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

    private func button(_ s: String) -> XCUIElement {
        app.buttons.containing(NSPredicate(format: "label CONTAINS %@", s)).firstMatch
    }

    func testGetsAndSendsSongsWithGlassOnAComputer() throws {
        guard let computer = ProcessInfo.processInfo.environment["GLASS_LIVE_SHARE"], !computer.isEmpty else {
            throw XCTSkip("needs Glass sharing on a computer on this network: scripts/live-share.sh")
        }
        app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoShare", "-demoFolders", "fresh", "-route", "local"]
        app.launch()

        // The computer shows up by itself.
        let device = button(computer)
        XCTAssertTrue(device.waitForExistence(timeout: 40), "\(computer) is found on the network")
        shot("live-1-found")
        device.tap()

        // Its songs, listed by it: pick two and get them.
        let listed = button("Mac Song One").waitForExistence(timeout: 30)
        if !listed { shot("live-2-not-listed") }
        XCTAssertTrue(listed, "its songs are listed")
        shot("live-2-its-songs")
        button("Mac Song One").tap()
        button("Mac Song Two").tap()
        app.buttons["Get 2 songs"].firstMatch.tap()
        XCTAssertTrue(text("Got 2 songs from").waitForExistence(timeout: 60), "both songs arrive")
        XCTAssertTrue(text("on this iPhone").waitForExistence(timeout: 10), "and are marked as here")
        shot("live-3-got")

        // They play from Local Files now.
        app.navigationBars.buttons.firstMatch.tap()
        app.buttons["Songs"].firstMatch.tap()
        XCTAssertTrue(text("Mac Song One").waitForExistence(timeout: 20))

        // The other way: one of this iPhone's songs goes to the computer.
        let mine = text("Phone Song")
        XCTAssertTrue(mine.waitForExistence(timeout: 10), "the iPhone's own song")
        mine.press(forDuration: 1.0)
        let sendTo = app.buttons["Send to"].firstMatch
        XCTAssertTrue(sendTo.waitForExistence(timeout: 5))
        sendTo.tap()
        let target = app.buttons[computer].firstMatch
        XCTAssertTrue(target.waitForExistence(timeout: 5))
        target.tap()
        XCTAssertTrue(text("Sent 1 song to").waitForExistence(timeout: 60), "it arrives on the computer")
        shot("live-4-sent")
    }
}
