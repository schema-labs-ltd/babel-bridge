import AVKit
import UIKit
import XCTest

@MainActor
final class ReadReceiptVisibilityUITests: XCTestCase {
    func testUnifiedMessagesOnlyRecordRowsAfterTheyEnterTheViewport() {
        verifyViewport(conversation: false)
    }
    func testConversationOnlyRecordsRowsAfterTheyEnterTheViewport() {
        verifyViewport(conversation: true)
    }
    private func verifyViewport(conversation: Bool) {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoReadReceipts"] + (conversation ? ["-demoConversation"] : [])
        app.launch()
        let status = app.staticTexts["demo-viewed-receipts"].firstMatch
        XCTAssertTrue(status.waitForExistence(timeout: 10))
        let latest = NSPredicate(format: "label CONTAINS %@", "receipt-39")
        expectation(for: latest, evaluatedWith: status)
        waitForExpectations(timeout: 10)
        XCTAssertFalse(status.label.contains("receipt-0"), "Loading earlier rows must not mark them read")
        let initial = XCTAttachment(screenshot: app.screenshot())
        initial.name = conversation ? "Conversation receipts only for visible messages" : "Combined Messages receipts only for visible messages"
        initial.lifetime = .keepAlways; add(initial)
        for _ in 0..<20 {
            if status.label.contains("receipt-0") { break }
            app.scrollViews.firstMatch.swipeDown()
        }
        let oldest = NSPredicate(format: "label CONTAINS %@", "receipt-0")
        expectation(for: oldest, evaluatedWith: status)
        waitForExpectations(timeout: 10)
        let scrolled = XCTAttachment(screenshot: app.screenshot())
        scrolled.name = "Earlier messages marked only after scrolling into view"; scrolled.lifetime = .keepAlways; add(scrolled)
    }
}

@MainActor
final class ReplyOnlyUITests: XCTestCase {
    func testReplyOnlyDisablesComposerUntilIncomingReplyAndCanBeTurnedOff() {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoUnifiedFeed"]
        app.launch()
        let group = app.buttons["Open chat: Studio team"].firstMatch
        XCTAssertTrue(group.waitForExistence(timeout: 10))
        group.press(forDuration: 1)
        app.buttons["Conversation settings"].tap()
        let toggle = app.switches["conversation-reply-only"].firstMatch
        XCTAssertTrue(toggle.waitForExistence(timeout: 5))
        XCTAssertEqual(toggle.value as? String, "0")
        toggle.switches.firstMatch.tap()
        let settings = XCTAttachment(screenshot: app.screenshot())
        settings.name = "Reply only enabled for Studio team"; settings.lifetime = .keepAlways; add(settings)
        app.buttons["Save"].tap()
        let hint = app.staticTexts["reply-only-hint"].firstMatch
        XCTAssertTrue(hint.waitForExistence(timeout: 5))
        let input = app.textViews["message-composer-input"].firstMatch
        XCTAssertFalse(input.isEnabled)
        app.staticTexts["The new draft is ready."].firstMatch.press(forDuration: 1)
        app.buttons["Reply"].firstMatch.tap()
        XCTAssertTrue(input.isEnabled)
        input.tap(); input.typeText("Thanks from Messages.")
        app.buttons["Send to Studio team"].tap()
        XCTAssertTrue(hint.waitForExistence(timeout: 5))
        group.tap()
        XCTAssertTrue(hint.waitForExistence(timeout: 5))
        XCTAssertFalse(input.isEnabled)
        XCTAssertFalse(app.buttons["Send photos"].isEnabled)
        XCTAssertFalse(app.buttons["Record voice note"].isEnabled)
        let blocked = XCTAttachment(screenshot: app.screenshot())
        blocked.name = "Chat composer waits for an incoming reply"; blocked.lifetime = .keepAlways; add(blocked)
        app.staticTexts["The new draft is ready."].firstMatch.press(forDuration: 1)
        app.buttons["Reply"].firstMatch.tap()
        XCTAssertTrue(input.isEnabled)
        input.tap(); input.typeText("Thanks, I’ll take a look.")
        let ready = XCTAttachment(screenshot: app.screenshot())
        ready.name = "Incoming reply enables sending"; ready.lifetime = .keepAlways; add(ready)
        app.buttons["Send message"].tap()
        XCTAssertTrue(hint.waitForExistence(timeout: 5))
        let header = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Studio team")).firstMatch
        header.tap()
        XCTAssertEqual(toggle.value as? String, "1")
        toggle.switches.firstMatch.tap()
        app.buttons["Save"].tap()
        XCTAssertTrue(input.isEnabled)
        XCTAssertFalse(hint.exists)
    }
}
