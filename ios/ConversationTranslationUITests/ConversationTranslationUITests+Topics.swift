import AVKit
import UIKit
import XCTest

extension ConversationTranslationUITests {
    func testOutgoingMessageShowsSavedTopicInItsMenu() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoTopics", "-demoOutgoingTopic"]
        app.launch()
        let target = app.staticTexts["The replay is brilliant too."].firstMatch
        XCTAssertTrue(target.waitForExistence(timeout: 10))
        target.press(forDuration: 1)
        let label = app.buttons["Football"].firstMatch
        XCTAssertTrue(label.waitForExistence(timeout: 5))
        XCTAssertFalse(label.isEnabled)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Outgoing message with its saved Football category"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

    func testLongPressShowsSavedTopicInCombinedMessagesAndChat() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoTopics"]
        app.launch()
        let target = app.staticTexts["The replay is brilliant too."].firstMatch
        XCTAssertTrue(target.waitForExistence(timeout: 10))
        target.press(forDuration: 1)
        let label = app.buttons["Football"].firstMatch
        XCTAssertTrue(label.waitForExistence(timeout: 5))
        XCTAssertFalse(label.isEnabled, "Saved topic is information, not a message action")
        let combined = XCTAttachment(screenshot: app.screenshot())
        combined.name = "Saved topic in combined message long press menu"; combined.lifetime = .keepAlways; add(combined)
        app.buttons["React with ❤️"].firstMatch.tap()
        app.buttons["Open chat: Family"].firstMatch.tap()
        XCTAssertTrue(target.waitForExistence(timeout: 5))
        target.press(forDuration: 1)
        XCTAssertTrue(label.waitForExistence(timeout: 5))
        let chat = XCTAttachment(screenshot: app.screenshot())
        chat.name = "Saved topic in conversation long press menu"; chat.lifetime = .keepAlways; add(chat)
    }

    func testChatListKeepsCompactRowsAndVisibleFiltersWhenScrolling() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoChatList"]
        app.launch()
        app.buttons["Chats"].firstMatch.tap()
        let groups = app.buttons["Groups"].firstMatch
        XCTAssertTrue(groups.waitForExistence(timeout: 5))
        let first = app.staticTexts["Community group 1"].firstMatch
        let second = app.staticTexts["Community group 2"].firstMatch
        XCTAssertTrue(first.waitForExistence(timeout: 5))
        let initial = XCTAttachment(screenshot: app.screenshot())
        initial.name = "Chat list initial spacing"
        initial.lifetime = .keepAlways
        add(initial)
        XCTAssertTrue(groups.isHittable, "Chat filters must remain visible below the navigation bar")
        XCTAssertGreaterThanOrEqual(groups.frame.minY, app.navigationBars.firstMatch.frame.maxY - 1)
        XCTAssertLessThan(first.frame.minY - groups.frame.maxY, 60, "The first chat must sit directly below the filters")
        XCTAssertLessThan(second.frame.minY - first.frame.minY, 100, "Standard chat rows must not stack list padding on top of row padding")
        groups.tap()
        let filterY = groups.frame.minY
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4)).press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.8)))
        XCTAssertEqual(groups.frame.minY, filterY, accuracy: 2, "Pulling the list must not expand a blank navigation title")
        app.swipeUp()
        let filtersSettled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "hittable == true"), object: groups)
        XCTAssertEqual(XCTWaiter.wait(for: [filtersSettled], timeout: 5), .completed)
        XCTAssertTrue(groups.isHittable)
        XCTAssertEqual(groups.frame.minY, filterY, accuracy: 2)
        let scrolled = XCTAttachment(screenshot: app.screenshot())
        scrolled.name = "Chat list after scrolling"
        scrolled.lifetime = .keepAlways
        add(scrolled)
    }

    func testTopicFilterStaysFixedWhenPullingShortTimeline() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoTopics"]
        app.launch()
        let filter = app.buttons["topic-filter"].firstMatch
        XCTAssertTrue(filter.waitForExistence(timeout: 10))
        filter.tap()
        let combined = app.buttons["Football"].firstMatch
        (combined.exists ? combined : app.buttons["Football · Family"].firstMatch).tap()
        XCTAssertTrue(app.staticTexts["The replay is brilliant too."].firstMatch.waitForExistence(timeout: 5))
        app.swipeUp()
        let initialY = filter.frame.minY
        let before = XCTAttachment(screenshot: app.screenshot()); before.name = "Topic header before pulling timeline"; before.lifetime = .keepAlways; add(before)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45)).press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.8)))
        let after = XCTAttachment(screenshot: app.screenshot()); after.name = "Topic header after pulling timeline"; after.lifetime = .keepAlways; add(after)
        XCTAssertEqual(filter.frame.minY, initialY, accuracy: 2, "Pulling messages must not expand the header or move the topic filter")
    }

    func testTopicsCombineChatsAndShowQuotesOutsideLoadedHistory() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoTopics"]
        app.launch()
        let filter = app.buttons["topic-filter"].firstMatch
        XCTAssertTrue(filter.waitForExistence(timeout: 10))
        let before = XCTAttachment(screenshot: app.screenshot()); before.name = "All group discussions in the unified feed"; before.lifetime = .keepAlways; add(before)
        filter.tap()
        let familyTopic = app.buttons["Weekend plans"].firstMatch
        XCTAssertTrue(familyTopic.waitForExistence(timeout: 5))
        XCTAssertEqual(app.buttons.matching(identifier: "Weekend plans").count, 1)
        familyTopic.tap()
        XCTAssertTrue(app.staticTexts["I’ll bring the picnic blanket and sandwiches."].firstMatch.waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["What a goal in last night’s match!"].exists)
        XCTAssertTrue(app.staticTexts["Sunday brunch at eleven?"].exists)
        XCTAssertTrue(app.staticTexts["Could someone bring lunch?"].exists)
        let filtered = XCTAttachment(screenshot: app.screenshot()); filtered.name = "Weekend plans combines Family and Friends and preserves the original quote"; filtered.lifetime = .keepAlways; add(filtered)
        app.buttons["Open chat: Family"].firstMatch.tap()
        openConversationTopicFilter(in: app)
        app.buttons["Football"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["The replay is brilliant too."].firstMatch.waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["I’ll bring the picnic blanket and sandwiches."].exists)
        let conversation = XCTAttachment(screenshot: app.screenshot()); conversation.name = "Conversation focused on the Football topic"; conversation.lifetime = .keepAlways; add(conversation)
        openConversationTopicFilter(in: app)
        tapManageTopics(in: app)
        let topicToggle = app.switches["topics-enabled-family@g.us"].firstMatch
        XCTAssertTrue(topicToggle.waitForExistence(timeout: 5))
        XCTAssertEqual(topicToggle.value as? String, "1")
        topicToggle.switches.firstMatch.tap()
        XCTAssertEqual(topicToggle.value as? String, "0")
        app.buttons["Done"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["I’ll bring the picnic blanket and sandwiches."].firstMatch.waitForExistence(timeout: 5))
        let restored = XCTAttachment(screenshot: app.screenshot()); restored.name = "Disabling topics restores all messages"; restored.lifetime = .keepAlways; add(restored)
        openConversationTopicFilter(in: app)
        tapManageTopics(in: app)
        let initialImport = app.buttons["import-recent-topics"].firstMatch
        if !initialImport.isHittable { app.swipeUp() }
        XCTAssertTrue(initialImport.waitForExistence(timeout: 5))
        initialImport.tap()
        XCTAssertTrue(app.buttons["Start import"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "5 messages across 2 chats")).firstMatch.exists)
        let preview = XCTAttachment(screenshot: app.screenshot()); preview.name = "Initial import previews seven days across all chats"; preview.lifetime = .keepAlways; add(preview)
        app.buttons["Start import"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "Queued 5 messages across 2 chats")).firstMatch.waitForExistence(timeout: 5))
        app.buttons["Done"].firstMatch.tap()
        openConversationTopicFilter(in: app)
        XCTAssertTrue(app.buttons["Football"].firstMatch.waitForExistence(timeout: 5))
    }

    /// Opens the topic filter inside a conversation. There it is a submenu of the overflow menu beside
    /// the starred filter: the app's own "Conversation" menu, or the system overflow menu on iOS 27.
    private func openConversationTopicFilter(in app: XCUIApplication) {
        let ownMenu = app.buttons["Conversation"].firstMatch
        let overflow = ownMenu.exists ? ownMenu : app.buttons["More"].firstMatch
        XCTAssertTrue(overflow.waitForExistence(timeout: 5))
        overflow.tap()
        let byIdentifier = app.buttons["topic-filter"].firstMatch
        let filter = byIdentifier.exists ? byIdentifier : app.buttons["Filter by topic"].firstMatch
        XCTAssertTrue(filter.waitForExistence(timeout: 5))
        filter.tap()
    }

    /// Chooses Manage topics from an open topic filter menu.
    private func tapManageTopics(in app: XCUIApplication) {
        let byIdentifier = app.buttons["manage-topics"].firstMatch
        let manage = byIdentifier.exists ? byIdentifier : app.buttons["Manage topics"].firstMatch
        XCTAssertTrue(manage.waitForExistence(timeout: 5))
        manage.tap()
    }
}
