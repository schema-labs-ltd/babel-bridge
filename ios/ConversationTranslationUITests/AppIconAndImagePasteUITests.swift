import AVKit
import UIKit
import XCTest

@MainActor
final class AppIconUITests: XCTestCase {
    func testIconPickerChangesHomeScreenPersistsAndRestoresOriginal() {
        let app = XCUIApplication()
        app.launchArguments = ["-demo"]
        app.launch()
        openPicker(in: app)
        choose("neonOrbit", in: app)
        let picker = XCTAttachment(screenshot: app.screenshot())
        picker.name = "App icon gallery with Neon Orbit selected"
        picker.lifetime = .keepAlways
        add(picker)
        XCUIDevice.shared.press(.home)
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        XCTAssertTrue(springboard.icons["Babel Bridge"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(springboard.icons["Babel Bridge"].firstMatch.isHittable)
        let home = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        home.name = "Alternate icon on the Home Screen"
        home.lifetime = .keepAlways
        add(home)
        app.terminate()
        app.launch()
        openPicker(in: app)
        XCTAssertEqual(app.buttons["app-icon-neonOrbit"].value as? String, "Selected")
        choose("sunrise", in: app)
        choose("original", in: app)
    }

    private func openPicker(in app: XCUIApplication) {
        let settings = app.buttons["Settings"].firstMatch
        XCTAssertTrue(settings.waitForExistence(timeout: 10), app.debugDescription)
        settings.tap()
        let row = app.buttons["app-icon-settings"].firstMatch
        for _ in 0..<5 {
            if row.exists && row.isHittable { break }
            app.swipeUp()
        }
        XCTAssertTrue(row.waitForExistence(timeout: 5), app.debugDescription)
        row.tap()
        XCTAssertTrue(app.buttons["app-icon-original"].waitForExistence(timeout: 5))
    }

    private func choose(_ icon: String, in app: XCUIApplication) {
        let choice = app.buttons["app-icon-\(icon)"]
        choice.tap()
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let confirmation = springboard.alerts.buttons["OK"].firstMatch
        if confirmation.waitForExistence(timeout: 3) { confirmation.tap() }
        let selected = NSPredicate(format: "value == %@", "Selected")
        expectation(for: selected, evaluatedWith: choice)
        waitForExpectations(timeout: 10)
        XCTAssertFalse(app.alerts["Couldn’t change icon"].exists)
    }
}

@MainActor
final class ImagePasteUITests: XCTestCase {
    func testPastedImagesCanBeReviewedCancelledAndSentWithoutLosingDraft() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoChatGallery", "-demoClipboardImage"]
        app.launch()
        let editor = app.textViews["message-composer-input"].firstMatch
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        editor.tap()
        editor.typeText("Keep my draft")
        paste(into: editor, in: app)
        XCTAssertTrue(app.navigationBars["Send 2 photos"].waitForExistence(timeout: 5), app.debugDescription)
        let preview = XCTAttachment(screenshot: app.screenshot())
        preview.name = "Two pasted images in review before sending"
        preview.lifetime = .keepAlways
        add(preview)
        app.buttons["Cancel"].firstMatch.tap()
        XCTAssertEqual(editor.value as? String, "Keep my draft")
        paste(into: editor, in: app)
        XCTAssertTrue(app.navigationBars["Send 2 photos"].waitForExistence(timeout: 5))
        let caption = app.textFields["Add a caption"].firstMatch
        XCTAssertTrue(caption.waitForExistence(timeout: 5), app.debugDescription)
        caption.tap()
        caption.typeText("Pasted photo caption")
        app.buttons["Send"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "Pasted photo caption")).firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertEqual(editor.value as? String, "Keep my draft")
        let sent = XCTAttachment(screenshot: app.screenshot())
        sent.name = "Pasted photos sent while original text draft remains"
        sent.lifetime = .keepAlways
        add(sent)
    }

    func testOrdinaryMultilineTextPasteStillWorks() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoChatGallery", "-demoClipboardText"]
        app.launch()
        let editor = app.textViews["message-composer-input"].firstMatch
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        paste(into: editor, in: app)
        XCTAssertEqual(editor.value as? String, "Copied text\nsecond line")
        XCTAssertFalse(app.navigationBars["Send photo"].exists)
        let pastedText = XCTAttachment(screenshot: app.screenshot())
        pastedText.name = "Ordinary multiline text paste remains supported"
        pastedText.lifetime = .keepAlways
        add(pastedText)
        app.buttons["Send message"].tap()
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@ AND label CONTAINS %@", "Copied text", "second line")).firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertEqual(editor.value as? String, "")
    }

    func testUnifiedMessagesAlsoAcceptPastedImages() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoUnifiedFeed", "-demoClipboardImage"]
        app.launch()
        let editor = app.textViews["message-composer-input"].firstMatch
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        paste(into: editor, in: app)
        XCTAssertTrue(app.navigationBars["Send 2 photos"].waitForExistence(timeout: 5), app.debugDescription)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Pasted images capture the unified message destination"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        app.buttons["Cancel"].firstMatch.tap()
        XCTAssertTrue(editor.waitForExistence(timeout: 5))
    }

    private func paste(into editor: XCUIElement, in app: XCUIApplication) {
        // Empty editors expose the menu at the caret; with a draft, press after the text.
        let empty = (editor.value as? String ?? "").isEmpty
        let caret = editor.coordinate(withNormalizedOffset: CGVector(dx: empty ? 0.03 : 0.5, dy: 0.5))
        caret.tap()
        caret.press(forDuration: 1.2)
        let menuItem = app.menuItems["Paste"].firstMatch
        if menuItem.waitForExistence(timeout: 2) { menuItem.tap() }
        else {
            let button = app.buttons["Paste"].firstMatch
            XCTAssertTrue(button.waitForExistence(timeout: 5), app.debugDescription)
            button.tap()
        }
    }

}
