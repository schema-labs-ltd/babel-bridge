import AVKit
import UIKit
import XCTest

extension XCUIApplication {
    /// The first element with this identifier or label, whatever its type.
    /// The iOS 26.5 simulator can report a different type for a SwiftUI
    /// element than its legacy attributes (a row as Button, text as Other, a
    /// menu action as PopUpButton), and iPad context menus differ from
    /// iPhone's, so a query for one type misses it. Images are left out: their
    /// labels name the symbol (every bubble has a "Reply" swipe arrow).
    func named(_ name: String) -> XCUIElement {
        descendants(matching: .any)
            .matching(NSPredicate(format: "(identifier == %@ OR label == %@) AND elementType != %@",
                                  name, name, NSNumber(value: XCUIElement.ElementType.image.rawValue)))
            .firstMatch
    }
}

extension XCUIElement {
    /// Long-presses a message bubble beside its text, where the message's
    /// menu opens. A long press on the selectable text itself can start text
    /// selection instead (the iPad simulator shows Copy and Share).
    func pressBubble() {
        coordinate(withNormalizedOffset: CGVector(dx: 1, dy: 0.5)).withOffset(CGVector(dx: 20, dy: 0)).press(forDuration: 1)
    }
}

@MainActor
final class ConversationTranslationUITests: XCTestCase {
    func testChatGalleryBrowsesPhotosAndVideosAndLoadsOlderMedia() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoChatGallery"]
        app.launch()
        let open = app.buttons["open-chat-gallery"]
        XCTAssertTrue(open.waitForExistence(timeout: 10))
        open.tap()
        let photo = app.buttons["gallery-item-chat-gallery-000"]
        XCTAssertTrue(photo.waitForExistence(timeout: 10))
        let grid = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        grid.name = "Chat gallery square photos and videos"
        grid.lifetime = .keepAlways
        add(grid)
        photo.tap()
        XCTAssertTrue(app.images["Full-screen photo"].waitForExistence(timeout: 5))
        // An iPhone shows the first page ("1 of 60+"); an iPad's taller grid
        // has already loaded the older page ("1 of 75"). Item 074 below checks
        // that older media loads either way.
        let position = app.staticTexts["gallery-position"].label
        XCTAssertTrue(["1 of 60+", "1 of 75"].contains(position), position)
        let fullPhoto = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        fullPhoto.name = "Full screen gallery photo with navigation"
        fullPhoto.lifetime = .keepAlways
        add(fullPhoto)
        app.images["Full-screen photo"].swipeLeft()
        let video = app.otherElements["gallery-video-player"].firstMatch
        XCTAssertTrue(video.waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["gallery-position"].label.hasPrefix("2 of"))
        let playback = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        playback.name = "Full screen video inside chat gallery"
        playback.lifetime = .keepAlways
        add(playback)
        video.swipeLeft()
        XCTAssertTrue(app.images["Full-screen photo"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["gallery-position"].label.hasPrefix("3 of"))
        app.images["Full-screen photo"].swipeRight()
        XCTAssertTrue(video.waitForExistence(timeout: 5))
        app.buttons["Previous media"].tap()
        XCTAssertTrue(app.images["Full-screen photo"].waitForExistence(timeout: 5))
        app.buttons["Close photo"].tap()
        XCTAssertTrue(photo.waitForExistence(timeout: 5))
        let oldest = app.buttons["gallery-item-chat-gallery-074"]
        for _ in 0..<22 {
            if oldest.exists && oldest.isHittable { break }
            app.scrollViews.firstMatch.swipeUp()
        }
        XCTAssertTrue(oldest.waitForExistence(timeout: 10))
        oldest.tap()
        XCTAssertTrue(app.staticTexts["gallery-position"].label.hasPrefix("75 of 75"))
        app.buttons["Close photo"].tap()
        app.buttons["close-chat-gallery"].tap()
        XCTAssertTrue(open.waitForExistence(timeout: 5))
    }

    func testLostWhatsAppSessionOpensLinkingAndReturnsToInbox() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoRelinking"]
        app.launch()
        XCTAssertTrue(app.staticTexts["Link WhatsApp"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.images["whatsapp-link-qr"].waitForExistence(timeout: 8))
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "Fresh WhatsApp linking flow after session removal"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        XCTAssertTrue(app.staticTexts["Link WhatsApp"].waitForNonExistence(timeout: 15))
        XCTAssertFalse(app.buttons["Connect translator"].exists)
    }
    func testVideoOpensFullScreenAndReturnsToTheSameMessage() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoVideo"]
        app.launch()
        // Tapping the bubble opens the shared player full screen and plays it.
        let video = app.named("Play video full screen")
        XCTAssertTrue(video.waitForExistence(timeout: 10))
        video.tap()
        let caption = app.staticTexts["A sample video to try full screen and picture in picture."].firstMatch
        XCTAssertTrue(caption.waitForNonExistence(timeout: 5) || !caption.isHittable, "The player must cover the conversation")
        defer { XCUIDevice.shared.orientation = .portrait }
        if UIDevice.current.userInterfaceIdiom == .phone {
            XCUIDevice.shared.orientation = .landscapeLeft
            XCTAssertGreaterThan(app.frame.width, app.frame.height)
        }
        // Reveal controls if their normal auto-hide has already elapsed.
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        let done = app.buttons.matching(NSPredicate(format: "label IN %@", ["Done", "Close", "Exit Full Screen"])).firstMatch
        XCTAssertTrue(done.waitForExistence(timeout: 5), app.debugDescription)
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "Video full screen"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        done.tap()
        XCTAssertTrue(caption.waitForExistence(timeout: 5) && caption.isHittable, "Closing the player must return to the same message")
    }

    func testPlayingVideoStartsPictureInPictureWhenLeavingApp() throws {
        try checkAutomaticPictureInPicture(fullScreen: false)
    }

    func testFullScreenVideoStartsPictureInPictureWhenLeavingApp() throws {
        try checkAutomaticPictureInPicture(fullScreen: true)
    }

    private func checkAutomaticPictureInPicture(fullScreen: Bool) throws {
        try XCTSkipUnless(AVPictureInPictureController.isPictureInPictureSupported(),
                          "AVKit reports that picture in picture is unavailable on this simulator/device")
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoVideo"]
        app.launch()
        let video = app.named("Play video full screen")
        XCTAssertTrue(video.waitForExistence(timeout: 10))
        // The bubble opens the player full screen and starts playback; closing
        // it keeps the video playing in the bubble.
        video.tap()
        XCTAssertTrue(app.buttons["Pause"].firstMatch.waitForExistence(timeout: 5), app.debugDescription)
        if !fullScreen {
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
            let done = app.buttons.matching(NSPredicate(format: "label IN %@", ["Done", "Close", "Exit Full Screen"])).firstMatch
            XCTAssertTrue(done.waitForExistence(timeout: 5), app.debugDescription)
            done.tap()
            XCTAssertTrue(video.waitForExistence(timeout: 5))
        }
        let appWidth = app.frame.width
        XCUIDevice.shared.press(.home)
        XCUIApplication(bundleIdentifier: "com.apple.Preferences").activate()
        // AVKit exposes the floating window in the originating application's tree.
        let pip = app.otherElements["PIPUIView"]
        XCTAssertTrue(pip.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertLessThan(pip.frame.width, appWidth * 0.75)
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "Video continues in picture in picture outside Babel Bridge"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        app.activate()
    }

    func testLiveEditChangesGetToGrrInUnifiedAndNormalViewsWithoutRelaunch() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoLiveEdits"]
        app.launch()
        XCTAssertTrue(app.staticTexts["Get"].firstMatch.waitForExistence(timeout: 5))
        let before = XCTAttachment(screenshot: app.screenshot())
        before.name = "Unified feed before incoming edit"
        before.lifetime = .keepAlways
        add(before)
        XCTAssertTrue(app.staticTexts["Grr"].firstMatch.waitForExistence(timeout: 15))
        XCTAssertFalse(app.staticTexts["Get"].exists)
        XCTAssertTrue(app.staticTexts["Edited"].firstMatch.exists)
        XCTAssertTrue(app.staticTexts["Get what?"].exists)
        let after = XCTAttachment(screenshot: app.screenshot())
        after.name = "Unified feed shows Grr and Edited without relaunch"
        after.lifetime = .keepAlways
        add(after)
        app.buttons["Open chat: Edit preview"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Grr"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Edited"].firstMatch.exists)
        XCTAssertFalse(app.staticTexts["Get"].exists)
        let chat = XCTAttachment(screenshot: app.screenshot())
        chat.name = "Normal conversation shows the same corrected message"
        chat.lifetime = .keepAlways
        add(chat)
    }

    func testLongPressReactionsUpdateUnifiedAndNormalViewsWithoutRelaunch() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoUnifiedFeed"]
        app.launch()
        let target = app.staticTexts["The new draft is ready."].firstMatch
        XCTAssertTrue(target.waitForExistence(timeout: 10))
        target.pressBubble()
        let heart = app.named("React with ❤️")
        XCTAssertTrue(heart.waitForExistence(timeout: 5))
        XCTAssertTrue(app.named("Reply").exists)
        XCTAssertTrue(app.named("More reactions").isHittable)
        let menu = XCTAttachment(screenshot: app.screenshot())
        menu.name = "Quick reactions and reply on message long press"
        menu.lifetime = .keepAlways
        add(menu)
        heart.tap()
        let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.named("Reply"))
        XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 5), .completed)
        XCTAssertTrue(app.descendants(matching: .any)["Reactions: ❤️, 1 in total"].firstMatch.waitForExistence(timeout: 5))
        let visibleGroup = try XCTUnwrap(app.buttons.matching(identifier: "Open chat: Studio team").allElementsBoundByIndex.first(where: { $0.isHittable }))
        visibleGroup.tap()
        XCTAssertTrue(app.descendants(matching: .any)["Reactions: ❤️, 1 in total"].firstMatch.waitForExistence(timeout: 5))
        target.pressBubble()
        app.named("React with 👍").tap()
        XCTAssertTrue(app.descendants(matching: .any)["Reactions: 👍, 1 in total"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertFalse(app.descendants(matching: .any)["Reactions: ❤️, 1 in total"].exists)
        target.pressBubble()
        app.named("More reactions").tap()
        let extra = app.named("React with 🎉")
        XCTAssertTrue(extra.waitForExistence(timeout: 5))
        extra.tap()
        XCTAssertTrue(app.descendants(matching: .any)["Reactions: 🎉, 1 in total"].firstMatch.waitForExistence(timeout: 5))
        app.buttons["Messages"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["Reactions: 🎉, 1 in total"].firstMatch.waitForExistence(timeout: 5))
        target.pressBubble()
        app.named("Remove reaction").tap()
        XCTAssertFalse(app.descendants(matching: .any)["Reactions: 🎉, 1 in total"].waitForExistence(timeout: 1))
        let result = XCTAttachment(screenshot: app.screenshot())
        result.name = "Unified feed updated after reaction removal without relaunch"
        result.lifetime = .keepAlways
        add(result)
    }

    func testGroupTranslationSwitchSavesAcrossUnifiedMessagesAndChatsAndKeepsPeopleOff() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-demoUnifiedFeed"]
        app.launch()
        let group = app.buttons["Open chat: Studio team"].firstMatch
        XCTAssertTrue(group.waitForExistence(timeout: 10))
        group.press(forDuration: 1)
        app.buttons["Conversation settings"].tap()
        let translation = app.switches["conversation-translation-enabled"].firstMatch
        XCTAssertTrue(translation.waitForExistence(timeout: 5))
        XCTAssertEqual(translation.value as? String, "0")
        translation.switches.firstMatch.tap()
        XCTAssertEqual(translation.value as? String, "1")
        XCTAssertTrue(app.staticTexts["Language"].waitForExistence(timeout: 3))
        app.buttons["Save"].tap()

        // Reopen the same group's setting from the unified feed.
        XCTAssertTrue(group.waitForExistence(timeout: 5))
        group.press(forDuration: 1)
        app.buttons["Conversation settings"].tap()
        XCTAssertTrue(translation.waitForExistence(timeout: 5))
        XCTAssertEqual(translation.value as? String, "1")
        app.buttons["Cancel"].tap()

        // Open the normal chat from the same feed and check the shared setting.
        group.tap()
        let header = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "Studio team")).firstMatch
        XCTAssertTrue(header.waitForExistence(timeout: 5))
        header.tap()
        XCTAssertTrue(translation.waitForExistence(timeout: 5))
        XCTAssertEqual(translation.value as? String, "1")
        translation.switches.firstMatch.tap()
        XCTAssertEqual(translation.value as? String, "0")
        app.buttons["Save"].tap()
        app.buttons["Messages"].tap()

        let person = app.buttons["Open chat: Jordan"].firstMatch
        XCTAssertTrue(person.waitForExistence(timeout: 5))
        person.press(forDuration: 1)
        app.buttons["Conversation settings"].tap()
        XCTAssertTrue(translation.waitForExistence(timeout: 5))
        XCTAssertEqual(translation.value as? String, "0")
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Person translation remains off after changing a group"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }
}
