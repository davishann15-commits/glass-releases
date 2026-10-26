import XCTest

/// Adding a folder from Files to Local Files, through the system's Files picker: its
/// songs (subfolders too) show up and play from where they are. Needs the fixture
/// folder in the simulator's On My iPhone first: `scripts/sim-files-fixture.sh <UDID>`.
final class LocalFoldersUITests: XCTestCase {
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

    /// Waits for a picker element (the Files picker is slow to appear in the simulator).
    private func picker(_ label: String) -> XCUIElement {
        let e = app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", label)).firstMatch
        XCTAssertTrue(e.waitForExistence(timeout: 20), "\(label) in the Files picker")
        return e
    }

    private func launch(_ folders: String) {
        app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoFolders", folders, "-route", "local"]
        app.launch()
        XCTAssertTrue(text("Showing songs from").waitForExistence(timeout: 20))
    }

    private var listed: Bool { text("Showing songs from").label.contains("Spotify Local") }

    func testAddAFolderFromFiles() {
        launch("fresh")
        XCTAssertFalse(listed)
        let add = app.buttons.matching(NSPredicate(format: "label == %@", "add a folder")).firstMatch
        add.tap()
        picker("Browse").tap()
        picker("On My iPhone").tap()
        // The folder's icon opens it (its name only selects the place being looked at).
        let folder = app.cells.matching(NSPredicate(format: "label CONTAINS %@", "Spotify Local")).firstMatch
        XCTAssertTrue(folder.waitForExistence(timeout: 20))
        folder.tap()
        XCTAssertTrue(picker("Harbor").exists, "inside the folder")
        shot("folders-1-picker")
        picker("Open").tap()

        // Its songs, the one in a subfolder too.
        XCTAssertTrue(text("Now showing songs from Spotify Local").waitForExistence(timeout: 20))
        XCTAssertTrue(listed, "the folder is listed as a source")
        app.buttons["Songs"].firstMatch.tap()
        XCTAssertTrue(text("Night Drive").waitForExistence(timeout: 20))
        XCTAssertTrue(text("Harbor Lights").exists, "a subfolder's song")
        shot("folders-2-added")

        // Still there after a relaunch (the app opens it again before the core starts).
        app.terminate()
        launch("kept")
        XCTAssertTrue(listed, "kept across launches")
        app.buttons["Songs"].firstMatch.tap()
        XCTAssertTrue(text("Night Drive").waitForExistence(timeout: 20))

        // Removed in Settings: gone, and stays gone after a relaunch.
        app.buttons["settings"].firstMatch.tap()
        let search = app.textFields.firstMatch
        XCTAssertTrue(search.waitForExistence(timeout: 10))
        search.tap()
        search.typeText("Show songs from")
        let remove = app.buttons.matching(NSPredicate(format: "label == %@", "Remove")).firstMatch
        XCTAssertTrue(remove.waitForExistence(timeout: 10))
        XCTAssertTrue(text("Spotify Local").exists)
        shot("folders-3-settings")
        remove.tap()
        XCTAssertTrue(text("Spotify Local").waitForNonExistence(timeout: 10))
        app.terminate()
        launch("kept")
        XCTAssertFalse(listed, "a removed folder doesn't come back")
    }
}
