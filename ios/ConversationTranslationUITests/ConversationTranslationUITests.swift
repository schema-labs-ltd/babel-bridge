import AVKit
import UIKit
import XCTest

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
        XCTAssertEqual(app.staticTexts["gallery-position"].label, "1 of 60+")
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
        let video = app.otherElements["Video"].firstMatch
        XCTAssertTrue(video.waitForExistence(timeout: 10))
        video.tap()
        let fullScreen = app.buttons.matching(NSPredicate(format: "label CONTAINS[c] 'full screen'")).firstMatch
        XCTAssertTrue(fullScreen.waitForExistence(timeout: 5), app.debugDescription)
        fullScreen.tap()
        XCTAssertGreaterThan(video.frame.width, app.frame.width * 0.95, app.debugDescription)
        defer { XCUIDevice.shared.orientation = .portrait }
        if UIDevice.current.userInterfaceIdiom == .phone {
            XCUIDevice.shared.orientation = .landscapeLeft
            XCTAssertGreaterThan(app.frame.width, app.frame.height)
        }
        // Reveal controls if their normal auto-hide has already elapsed.
        video.tap()
        let done = app.buttons.matching(NSPredicate(format: "label IN %@", ["Done", "Close", "Exit Full Screen"])).firstMatch
        XCTAssertTrue(done.waitForExistence(timeout: 5), app.debugDescription)
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "Video full screen"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        done.tap()
        XCTAssertTrue(app.staticTexts["A sample video to try full screen and picture in picture."].firstMatch.waitForExistence(timeout: 5))
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
        let video = app.otherElements["Video"].firstMatch
        XCTAssertTrue(video.waitForExistence(timeout: 10))
        video.tap()
        XCTAssertTrue(app.buttons["Pause"].firstMatch.waitForExistence(timeout: 5), app.debugDescription)
        if fullScreen {
            app.buttons.matching(NSPredicate(format: "label CONTAINS[c] 'full screen'")).firstMatch.tap()
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
        target.press(forDuration: 1)
        let heart = app.buttons["React with ❤️"].firstMatch
        XCTAssertTrue(heart.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Reply"].exists)
        XCTAssertTrue(app.buttons["More reactions"].isHittable)
        let menu = XCTAttachment(screenshot: app.screenshot())
        menu.name = "Quick reactions and reply on message long press"
        menu.lifetime = .keepAlways
        add(menu)
        heart.tap()
        let dismissed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.buttons["Reply"].firstMatch)
        XCTAssertEqual(XCTWaiter.wait(for: [dismissed], timeout: 5), .completed)
        XCTAssertTrue(app.staticTexts["❤️ 1"].firstMatch.waitForExistence(timeout: 5))
        let visibleGroup = try XCTUnwrap(app.buttons.matching(identifier: "Open chat: Studio team").allElementsBoundByIndex.first(where: { $0.isHittable }))
        visibleGroup.tap()
        XCTAssertTrue(app.staticTexts["❤️ 1"].firstMatch.waitForExistence(timeout: 5))
        target.press(forDuration: 1)
        app.buttons["React with 👍"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["👍 1"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["❤️ 1"].exists)
        target.press(forDuration: 1)
        app.buttons["More reactions"].tap()
        let extra = app.buttons["React with 🎉"].firstMatch
        XCTAssertTrue(extra.waitForExistence(timeout: 5))
        extra.tap()
        XCTAssertTrue(app.staticTexts["🎉 1"].firstMatch.waitForExistence(timeout: 5))
        app.buttons["Messages"].tap()
        XCTAssertTrue(app.staticTexts["🎉 1"].firstMatch.waitForExistence(timeout: 5))
        target.press(forDuration: 1)
        app.buttons["Remove reaction"].tap()
        XCTAssertFalse(app.staticTexts["🎉 1"].waitForExistence(timeout: 1))
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
