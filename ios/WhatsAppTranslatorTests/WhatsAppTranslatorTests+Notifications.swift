import XCTest
import Intents
import SwiftUI
import UserNotifications
@testable import WhatsAppTranslator

extension WhatsAppTranslatorTests {
    func testNotificationSenderUsesExactPhoneNumberIdentity() {
        let identity = NotificationPersonIdentity.sender(
            senderID: "447700900123",
            senderName: "Sample Sender"
        )

        XCTAssertEqual(identity.handleValue, "+447700900123")
        XCTAssertEqual(identity.handleType, .phoneNumber)
        XCTAssertFalse(identity.isContactSuggestion)
        XCTAssertEqual(identity.suggestionType, .none)

        let formattedIdentity = NotificationPersonIdentity.sender(
            senderID: "+44 (7700) 900-123",
            senderName: "Sample Sender"
        )
        XCTAssertEqual(formattedIdentity.handleValue, "+447700900123")

        let appIdentity = NotificationPersonIdentity.sender(
            senderID: "whatsapp-user-id",
            senderName: "Sample Sender"
        )
        XCTAssertEqual(appIdentity.handleType, .unknown)
        XCTAssertTrue(appIdentity.isContactSuggestion)
        XCTAssertEqual(appIdentity.suggestionType, .instantMessageAddress)
    }

    func testGroupNotificationKeepsSenderGroupAndTranslationOnSeparateLines() throws {
        let content = UNMutableNotificationContent()
        content.title = "Judit"
        content.body = "Szia"
        content.userInfo = [
            "contactId": "family@g.us", "senderName": "Judit",
            "conversationName": "Family", "recipientCount": 3,
            "messageBody": "[Family] Hello"
        ]
        let prepared = NotificationMessagePresentation.preparedContent(content)
        let communication = NotificationMessagePresentation.messagingContent(content, avatarData: nil, donate: false)
        XCTAssertEqual(communication.title, "Judit")
        XCTAssertEqual(communication.subtitle, "Family")
        XCTAssertEqual(communication.body, "Hello")
        XCTAssertEqual(prepared.title, "Judit")
        XCTAssertEqual(prepared.subtitle, "Family")
        XCTAssertEqual(prepared.body, "Hello")
        XCTAssertEqual(NotificationMessagePresentation.preparedContent(prepared).body, "Hello")

        content.userInfo["messageBody"] = "Hello"
        XCTAssertEqual(NotificationMessagePresentation.preparedContent(content).body, "Hello")

        content.userInfo["messageBodyIncludesGroup"] = false
        content.userInfo["messageBody"] = "[Family] Hello"
        XCTAssertEqual(NotificationMessagePresentation.preparedContent(content).body, "[Family] Hello")
        content.userInfo["messageBody"] = "Hello"

        content.userInfo.removeValue(forKey: "recipientCount")
        let fallback = NotificationMessagePresentation.messagingContent(content, avatarData: nil, donate: false)
        XCTAssertEqual(fallback.title, "Judit")
        XCTAssertEqual(fallback.subtitle, "Family")
        XCTAssertEqual(fallback.body, "Hello")

        content.userInfo["contactId"] = "friend@s.whatsapp.net"
        XCTAssertEqual(NotificationMessagePresentation.preparedContent(content).body, "Hello")
        XCTAssertEqual(NotificationMessagePresentation.messagingContent(content, avatarData: nil, donate: false).subtitle, "")
    }

    func testAccessibilityLayoutUsesStackedRowsAndCompactMessageChrome() {
        XCTAssertFalse(NativeLayoutPolicy.usesStackedChatRow(for: .large))
        XCTAssertFalse(NativeLayoutPolicy.usesCompactMessageChrome(for: .large))
        XCTAssertTrue(NativeLayoutPolicy.usesStackedChatRow(for: .accessibility1))
        XCTAssertTrue(NativeLayoutPolicy.usesCompactMessageChrome(for: .accessibility1))
        XCTAssertTrue(NativeLayoutPolicy.usesCompactMessageChrome(for: .accessibility5))
    }

    func testMessagingNotificationCategorySupportsInlineReplyWatchAndCarPlay() throws {
        let category = MessagingNotificationContract.category

        XCTAssertEqual(category.identifier, MessagingNotificationContract.categoryIdentifier)
        XCTAssertTrue(category.options.contains(.allowInCarPlay))
        XCTAssertTrue(category.options.contains(.allowAnnouncement))
        XCTAssertTrue(category.options.contains(.hiddenPreviewsShowTitle))
        XCTAssertEqual(category.intentIdentifiers, [INSearchForMessagesIntentIdentifier])
        let reply = try XCTUnwrap(category.actions.first as? UNTextInputNotificationAction)
        XCTAssertEqual(reply.identifier, MessagingNotificationContract.replyActionIdentifier)
        XCTAssertEqual(reply.textInputButtonTitle, "Send")
    }

    @MainActor
    func testFirstNotificationAuthorizationRequestsCarPlayAndRequiresConsent() async {
        for granted in [true, false] {
            var requested: UNAuthorizationOptions?
            let authorized = await MessagingNotificationAuthorization.authorize(status: .notDetermined) {
                requested = $0
                return granted
            }
            XCTAssertEqual(authorized, granted)
            XCTAssertTrue(requested?.contains(.carPlay) == true)
            XCTAssertTrue(requested?.isSuperset(of: [.alert, .sound, .badge]) == true)
            XCTAssertFalse(requested?.contains(.provisional) == true)
        }
    }

    @MainActor
    func testFirstAuthorizationFallsBackToStandardAlertsIfCarPlayRequestThrows() async {
        struct AuthorizationError: Error {}
        for granted in [true, false] {
            var requests: [UNAuthorizationOptions] = []
            let authorized = await MessagingNotificationAuthorization.authorize(status: .notDetermined) {
                requests.append($0)
                if $0.contains(.carPlay) { throw AuthorizationError() }
                return granted
            }
            XCTAssertEqual(authorized, granted)
            XCTAssertEqual(requests.count, 2)
            XCTAssertTrue(requests.first?.contains(.carPlay) == true)
            XCTAssertFalse(requests.last?.contains(.carPlay) == true)
            XCTAssertTrue(requests.last?.isSuperset(of: [.alert, .sound, .badge]) == true)
        }
    }

    @MainActor
    func testExistingNotificationAuthorizationAlsoRequestsCarPlayOnUpgrade() async {
        var requested: UNAuthorizationOptions?
        let authorized = await MessagingNotificationAuthorization.authorize(status: .authorized) {
            requested = $0
            return true
        }
        XCTAssertTrue(authorized)
        XCTAssertTrue(requested?.contains(.carPlay) == true)
    }

    @MainActor
    func testCarPlayAuthorizationErrorPreservesExistingPhoneAndWatchRegistration() async {
        struct AuthorizationError: Error {}
        for status: UNAuthorizationStatus in [.authorized, .provisional] {
            let authorized = await MessagingNotificationAuthorization.authorize(status: status) { _ in
                throw AuthorizationError()
            }
            XCTAssertTrue(authorized)
        }
        let firstRequest = await MessagingNotificationAuthorization.authorize(status: .notDetermined) { _ in
            throw AuthorizationError()
        }
        XCTAssertFalse(firstRequest)
    }

    @MainActor
    func testProvisionalNotificationUpgradePreservesQuietAuthorization() async {
        var requested: UNAuthorizationOptions?
        let authorized = await MessagingNotificationAuthorization.authorize(status: .provisional) {
            requested = $0
            return true
        }
        XCTAssertTrue(authorized)
        XCTAssertTrue(requested?.contains(.carPlay) == true)
        XCTAssertTrue(requested?.contains(.provisional) == true)
    }

    @MainActor
    func testDeniedNotificationsDoNotRequestAgainOrRegisterForPush() async {
        let authorized = await MessagingNotificationAuthorization.authorize(status: .denied) { _ in
            XCTFail("Do not re-request after the user denies notifications")
            return true
        }
        XCTAssertFalse(authorized)
    }

    func testSiriVocabularyProvidesEnglishExamplesForEverySupportedMessagingIntent() throws {
        let vocabularyURL = try XCTUnwrap(
            Bundle.main.url(
                forResource: "AppIntentVocabulary",
                withExtension: "plist",
                subdirectory: nil,
                localization: "en"
            )
        )
        let data = try Data(contentsOf: vocabularyURL)
        let propertyList = try PropertyListSerialization.propertyList(from: data, format: nil)
        let root = try XCTUnwrap(propertyList as? [String: Any])
        let phraseEntries = try XCTUnwrap(root["IntentPhrases"] as? [[String: Any]])
        let examplesByIntent = Dictionary(
            uniqueKeysWithValues: phraseEntries.compactMap { entry -> (String, [String])? in
                guard let intentName = entry["IntentName"] as? String,
                      let examples = entry["IntentExamples"] as? [String] else {
                    return nil
                }
                return (intentName, examples)
            }
        )

        for intentName in [
            "INSendMessageIntent",
            "INSearchForMessagesIntent",
            "INSetMessageAttributeIntent",
        ] {
            let examples = try XCTUnwrap(examplesByIntent[intentName], "Missing examples for \(intentName)")
            XCTAssertFalse(examples.isEmpty, "Expected at least one example for \(intentName)")
            XCTAssertTrue(examples.allSatisfy { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
        }
    }

    func testMessagingNotificationRoutingReadsBackendMetadata() {
        let routing = MessagingNotificationRouting(userInfo: [
            "contactId": "family@g.us",
            "messageId": "message-1",
            "senderName": "Virág",
            "conversationName": "Sample Group",
            "chatType": "group",
        ])

        XCTAssertEqual(routing?.contactID, "family@g.us")
        XCTAssertEqual(routing?.messageID, "message-1")
        XCTAssertEqual(routing?.senderName, "Virág")
        XCTAssertEqual(routing?.conversationName, "Sample Group")
        XCTAssertTrue(routing?.isGroup == true)
    }

    func testReadingConversationRemovesOnlyItsDeliveredNotifications() {
        let deliveries = [
            DeliveredMessagingNotification(identifier: "family-message-1", contactID: "family@g.us"),
            DeliveredMessagingNotification(identifier: "family-message-2", contactID: "family@g.us"),
            DeliveredMessagingNotification(identifier: "virag-message-1", contactID: "virag@s.whatsapp.net"),
        ]

        XCTAssertEqual(
            MessagingNotificationReadState.identifiersToRemove(
                for: "family@g.us",
                from: deliveries
            ),
            ["family-message-1", "family-message-2"]
        )
    }
}
