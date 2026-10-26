import XCTest
import Intents
import SwiftUI
import UserNotifications
@testable import WhatsAppTranslator

final class WhatsAppTranslatorTests: XCTestCase {

    func decodeMessage(id: String, contentType: String, content: String) throws -> ChatMessage {
        let data = Data(
            """
            {"id":"\(id)","contactId":"chat@g.us","timestamp":1700000000000,"isFromMe":false,"isForwarded":false,"senderName":"Virag","senderPhone":"3630","contactName":"Family","contactPhone":null,"chatType":"group","contentType":"\(contentType)","content":\(content),"originalText":null,"translatedText":null,"sourceLanguage":null,"isTranslated":false}
            """.utf8
        )
        return try JSONDecoder().decode(ChatMessage.self, from: data)
    }
}

extension WhatsAppTranslatorTests {
    func testTranslatedVoiceContractPreservesOriginalAndTranslatedPlayback() throws {
        let json = #"{"id":"prepared-note","contactId":"contact","transcript":"Good morning","translation":"Jó reggelt","targetLanguage":"Hungarian","voice":"shimmer","audioData":"dHJhbnNsYXRlZA==","originalData":"b3JpZ2luYWw=","mimeType":"audio/mpeg","durationSeconds":2,"originalFollowUp":true}"#
        let note = try JSONDecoder.backend.decode(TranslatedVoiceNote.self, from: Data(json.utf8))
        XCTAssertEqual(note.voice, "shimmer")
        XCTAssertEqual(Data(base64Encoded: note.originalData), Data("original".utf8))
        XCTAssertEqual(Data(base64Encoded: note.audioData), Data("translated".utf8))
        XCTAssertTrue(note.originalFollowUp)
    }

    func testVoiceReadinessEventDecodesWithoutTreatingItAsANewMessage() throws {
        let json = #"{"type":"voice_ready","message_id":"voice-message"}"#
        let event = try JSONDecoder.backend.decode(LiveEvent.self, from: Data(json.utf8))
        XCTAssertEqual(event.messageId, "voice-message")
        XCTAssertNil(event.message)
    }

    func testMicrophonePurposeIsDeclared() throws {
        let purpose = try XCTUnwrap(Bundle.main.object(forInfoDictionaryKey: "NSMicrophoneUsageDescription") as? String)
        XCTAssertTrue(purpose.contains("voice"))
    }
}

final class ReadReceiptTests: XCTestCase {
    private func message(_ id: String, own: Bool = false, type: String = "text") throws -> ChatMessage {
        try JSONDecoder().decode(ChatMessage.self, from: JSONSerialization.data(withJSONObject: ["id":id,"contactId":"family@g.us","timestamp":1700000000000,"isFromMe":own,"isForwarded":false,"chatType":"group","contentType":type,"content":["type":type,"body":"Test message"],"isTranslated":false]))
    }

    @MainActor
    func testLoadingTranslationAndOutgoingMessagesDoNotCreateReadReceipts() async throws {
        let session = AppSession(demoMode: true)
        let incoming = try message("incoming")
        let own = try message("own", own: true)
        let reaction = try message("reaction", type: "reaction")
        let deleted = try message("deleted", type: "revoked")
        session.messages = [incoming.contactId: [incoming, own, reaction, deleted]]
        await session.loadMessages(for: incoming.contactId)
        await session.loadFeed()
        XCTAssertTrue(session.viewedReceiptMessageIDs.isEmpty)
        await session.recordMessageViews([incoming, own, reaction, deleted])
        XCTAssertEqual(session.viewedReceiptMessageIDs, ["incoming"])
        await session.recordMessageViews([incoming])
        XCTAssertEqual(session.viewedReceiptMessageIDs.count, 1)
        await session.loadFeed()
        XCTAssertEqual(session.viewedReceiptMessageIDs, ["incoming"])
    }

    func testReadRequestsIdentifyOnlyViewedMessagesAndPartialReadEventsPreserveUnreadCount() throws {
        let body = try JSONSerialization.jsonObject(with: JSONEncoder().encode(ViewedMessagesRequest(messageIds: ["one", "two"]))) as? [String: Any]
        XCTAssertEqual(body?["messageIds"] as? [String], ["one", "two"])
        XCTAssertNil(body?["timestamp"])
        XCTAssertNil(body?["senderJid"])
        let event = try JSONDecoder().decode(LiveEvent.self, from: Data(#"{"type":"messages_viewed","chat_id":"family@g.us","message_ids":["one"],"unread_count":3}"#.utf8))
        XCTAssertEqual(event.messageIds, ["one"])
        XCTAssertEqual(event.unreadCount, 3)
    }

    @MainActor
    func testPartialReadEventDoesNotClearUnseenMessagesOrAnotherChat() throws {
        let session = AppSession(demoMode: true)
        session.phase = .ready
        session.contacts = [Contact(id: "family@g.us", name: "Family", phone: nil, type: "group", lastMessageTime: 0, unreadCount: 4, pinnedAt: nil, lastMessagePreview: nil), Contact(id: "other@g.us", name: "Other", phone: nil, type: "group", lastMessageTime: 0, unreadCount: 7, pinnedAt: nil, lastMessagePreview: nil)]
        let event = try JSONDecoder().decode(LiveEvent.self, from: Data(#"{"type":"messages_viewed","chat_id":"family@g.us","message_ids":["one"],"unread_count":3}"#.utf8))
        session.handle(event)
        XCTAssertEqual(session.contacts[0].unreadCount, 3)
        XCTAssertEqual(session.contacts[1].unreadCount, 7)
        XCTAssertEqual(session.viewedReceiptMessageIDs, ["one"])
    }

    func testOfflineViewedIDsSurviveCacheRestartAndCannotCrossServers() async throws {
        let root = FileManager.default.temporaryDirectory.appending(path: "ReceiptTests-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let configuration = try ServerConfiguration.make(address: "https://receipts.example.test", password: "")
        let snapshot = ChatCacheSnapshot(serverBaseURL: configuration.baseURL.absoluteString, contacts: [], messages: [:], updatedAt: Date(), pendingReadReceipts: ["family@g.us":["seen-offline"]])
        let merged = snapshot.merging(contacts: [], messages: [], for: "family@g.us")
        XCTAssertEqual(merged.pendingReadReceipts, snapshot.pendingReadReceipts)
        await ChatCacheStore(directoryURL: root).save(merged)
        let restored = await ChatCacheStore(directoryURL: root).load(for: configuration)
        XCTAssertEqual(restored?.pendingReadReceipts, ["family@g.us":["seen-offline"]])
        let other = try ServerConfiguration.make(address: "https://other.example.test", password: "")
        let wrong = await ChatCacheStore(directoryURL: root).load(for: other)
        XCTAssertNil(wrong)
    }
}

final class UnifiedFeedSectionTests: XCTestCase {
    private let utc: Calendar = {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        return calendar
    }()
    // 2026-10-25 12:00 UTC.
    private let noon: Int64 = 1_792_929_600_000

    func testConsecutiveMessagesFromOneChatOnOneDayShareOneHeading() throws {
        let messages = [try message("a", contact: "virag@s.whatsapp.net", at: noon),
                        try message("b", contact: "virag@s.whatsapp.net", at: noon + 60_000),
                        try message("c", contact: "virag@s.whatsapp.net", at: noon + 120_000, fromMe: true)]
        let sections = UnifiedFeedSectionBuilder.sections(from: ConversationTimelineBuilder.items(from: messages), calendar: utc)
        XCTAssertEqual(sections.count, 1)
        XCTAssertEqual(sections[0].id, "a")
        XCTAssertEqual(sections[0].primaryMessage.contactId, "virag@s.whatsapp.net")
        XCTAssertEqual(sections[0].items.map(\.id), ["a", "b", "c"])
    }

    func testAnotherChatOrAnotherDayStartsANewHeading() throws {
        let messages = [try message("a", contact: "virag@s.whatsapp.net", at: noon),
                        try message("b", contact: "family@g.us", at: noon + 60_000),
                        try message("c", contact: "virag@s.whatsapp.net", at: noon + 120_000),
                        try message("d", contact: "virag@s.whatsapp.net", at: noon + 86_400_000)]
        let sections = UnifiedFeedSectionBuilder.sections(from: ConversationTimelineBuilder.items(from: messages), calendar: utc)
        XCTAssertEqual(sections.map(\.id), ["a", "b", "c", "d"])
        XCTAssertEqual(sections.map(\.primaryMessage.contactId), ["virag@s.whatsapp.net", "family@g.us", "virag@s.whatsapp.net", "virag@s.whatsapp.net"])
        XCTAssertEqual(sections.map(\.items.count), [1, 1, 1, 1])
    }

    func testLiveMessagesExtendTheLastRunWithoutChangingItsHeading() throws {
        let earlier = [try message("a", contact: "family@g.us", at: noon), try message("b", contact: "family@g.us", at: noon + 1_000)]
        let extended = earlier + [try message("c", contact: "family@g.us", at: noon + 2_000)]
        let before = UnifiedFeedSectionBuilder.sections(from: ConversationTimelineBuilder.items(from: earlier), calendar: utc)
        let after = UnifiedFeedSectionBuilder.sections(from: ConversationTimelineBuilder.items(from: extended), calendar: utc)
        XCTAssertEqual(before.map(\.id), after.map(\.id))
        XCTAssertEqual(after[0].items.count, 3)
        XCTAssertTrue(UnifiedFeedSectionBuilder.sections(from: [], calendar: utc).isEmpty)
    }

    private func message(_ id: String, contact: String, at timestamp: Int64, fromMe: Bool = false) throws -> ChatMessage {
        try JSONDecoder().decode(ChatMessage.self, from: JSONSerialization.data(withJSONObject: [
            "id": id, "contactId": contact, "timestamp": timestamp, "isFromMe": fromMe, "isForwarded": false,
            "contactName": "Chat", "chatType": contact.contains("@g.us") ? "group" : "private",
            "contentType": "text", "content": ["type": "text", "body": "Message \(id)"], "isTranslated": false,
        ]))
    }
}

final class ChatGPTSignInTests: XCTestCase {
    func testLoopbackRequestTargetKeepsTheCallbackQueryForTheServer() {
        XCTAssertEqual(
            ChatGPTSignInCoordinator.requestTarget("GET /auth/callback?code=a&state=b HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"),
            "/auth/callback?code=a&state=b"
        )
        XCTAssertNil(ChatGPTSignInCoordinator.requestTarget("POST /auth/callback HTTP/1.1\r\n\r\n"))
        XCTAssertNil(ChatGPTSignInCoordinator.requestTarget(""))
    }

    func testStatusDecodesWithoutOptionalFields() throws {
        let status = try JSONDecoder().decode(
            ChatGPTStatus.self,
            from: Data(#"{"available":true,"signedIn":true,"models":[{"slug":"gpt-6.1-sol","displayName":"GPT-6.1 Sol"}]}"#.utf8)
        )
        XCTAssertTrue(status.signedIn)
        XCTAssertEqual(status.models?.first?.slug, "gpt-6.1-sol")
        XCTAssertNil(status.email)
    }
}
