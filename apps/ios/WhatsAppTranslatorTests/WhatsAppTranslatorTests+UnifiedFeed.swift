import XCTest
import Intents
import SwiftUI
import UserNotifications
@testable import WhatsAppTranslator

extension WhatsAppTranslatorTests {
    @MainActor
    func testUnifiedQuickReplyLocksItsTargetAcrossNewMessages() async throws {
        func message(_ id: String, _ contact: String, _ timestamp: Int64) throws -> ChatMessage {
            let data = try JSONSerialization.data(withJSONObject: ["id": id, "contactId": contact, "timestamp": timestamp, "isFromMe": false, "isForwarded": false, "chatType": "group", "contentType": "Text", "isTranslated": false])
            return try JSONDecoder().decode(ChatMessage.self, from: data)
        }
        let original = try message("original", "group@g.us", 100)
        let newer = try message("newer", "group@g.us", 200)
        let elsewhere = try message("elsewhere", "other@g.us", 300)
        var draft = UnifiedReplyDraft()
        draft.updateText("", latestMessage: original)
        XCTAssertNil(draft.selected)
        XCTAssertFalse(draft.isFocused)
        draft.updateText("H", latestMessage: original)
        XCTAssertEqual(draft.selected?.id, original.id)
        XCTAssertTrue(draft.isFocused)
        draft.updateText("Hello", latestMessage: elsewhere)
        XCTAssertEqual(draft.selected?.contactId, original.contactId)
        draft.updateText("", latestMessage: newer)
        draft.updateText("Hello again", latestMessage: newer)
        XCTAssertEqual(draft.selected?.id, original.id)
        XCTAssertTrue(draft.isFocused)

        let session = AppSession(demoMode: true)
        session.messages = [original.contactId: [original, newer], elsewhere.contactId: [elsewhere]]
        await session.loadFeed()
        let target = try XCTUnwrap(draft.selected)
        XCTAssertTrue(session.feedReplyNeedsQuote(target))
        let sent = await session.send(text: draft.text, to: target.contactId, reply: session.replyTarget(for: target), replyOnlyIfNotLatest: true)
        XCTAssertTrue(sent)
        XCTAssertEqual(session.messages[original.contactId]?.last?.content?.replyContext?.messageId, original.id)
        XCTAssertEqual(session.messages[elsewhere.contactId]?.count, 1)
        draft.finishSending(to: target)
        XCTAssertNil(draft.selected)
        XCTAssertEqual(draft.text, "")
        draft.updateText("Next reply", latestMessage: elsewhere)
        XCTAssertEqual(draft.selected?.id, elsewhere.id)
    }

    func testUnifiedQuickReplyManualSelectionAndEmptyFeedStaySafe() throws {
        func message(_ id: String, _ contact: String) throws -> ChatMessage {
            let data = try JSONSerialization.data(withJSONObject: ["id": id, "contactId": contact, "timestamp": 100, "isFromMe": false, "isForwarded": false, "chatType": "group", "contentType": "Text", "isTranslated": false])
            return try JSONDecoder().decode(ChatMessage.self, from: data)
        }
        let first = try message("first", "first@g.us")
        let second = try message("second", "second@g.us")
        var draft = UnifiedReplyDraft()
        draft.updateText("Cannot route", latestMessage: nil)
        XCTAssertNil(draft.selected)
        XCTAssertTrue(draft.drafts.isEmpty)
        draft.updateText("First draft", latestMessage: first)
        draft.select(second)
        XCTAssertTrue(draft.isFocused)
        XCTAssertEqual(draft.text, "")
        draft.updateText("Second draft", latestMessage: first)
        XCTAssertEqual(draft.selected?.id, second.id)
        draft.select(first)
        XCTAssertEqual(draft.text, "First draft")
        draft.cancelSelection()
        XCTAssertFalse(draft.isFocused)
        XCTAssertNil(draft.selected)
        draft.updateText("Fresh reply", latestMessage: second)
        XCTAssertEqual(draft.text, "Fresh reply")
        XCTAssertEqual(draft.selected?.id, second.id)
    }

    @MainActor
    func testUnifiedFeedOrdersAcrossChatsAndScopesReplyContextToDestination() async throws {
        func message(_ id: String, _ contact: String, _ timestamp: Int64) throws -> ChatMessage {
            let data = try JSONSerialization.data(withJSONObject: ["id": id, "contactId": contact, "timestamp": timestamp, "isFromMe": false, "isForwarded": false, "chatType": "group", "contentType": "Text", "isTranslated": false])
            return try JSONDecoder().decode(ChatMessage.self, from: data)
        }
        let session = AppSession(demoMode: true)
        let first = try message("a", "group@g.us", 100)
        let latest = try message("b", "group@g.us", 100)
        let elsewhere = try message("c", "other@g.us", 200)
        session.messages = [first.contactId: [first, latest], elsewhere.contactId: [elsewhere]]
        await session.loadFeed()
        XCTAssertEqual(session.mainTab, .messages)
        XCTAssertEqual(session.unifiedMessages.map(\.id), ["a", "b", "c"])
        XCTAssertTrue(session.feedReplyNeedsQuote(first))
        XCTAssertFalse(session.feedReplyNeedsQuote(latest))
        XCTAssertFalse(session.feedReplyNeedsQuote(elsewhere))
        // Loading another chat must not remove older messages from the feed.
        session.messages[first.contactId] = [latest]
        XCTAssertEqual(session.unifiedMessages.map(\.id), ["a", "b", "c"])
        let normalSent = await session.send(text: "Normal", to: elsewhere.contactId, reply: session.replyTarget(for: elsewhere), replyOnlyIfNotLatest: true)
        XCTAssertTrue(normalSent)
        XCTAssertNil(session.messages[elsewhere.contactId]?.last?.content?.replyContext)
        let quotedSent = await session.send(text: "Quoted", to: first.contactId, reply: session.replyTarget(for: first), replyOnlyIfNotLatest: true)
        XCTAssertTrue(quotedSent)
        XCTAssertEqual(session.messages[first.contactId]?.last?.content?.replyContext?.messageId, first.id)
    }

    func testUnifiedFeedSendEncodesConditionalQuoteWithoutChangingOrdinaryReplies() throws {
        var request = SendMessageRequest(contactId: "group@g.us", text: "Reply", replyTo: "selected", replyToSender: nil, replyToText: "Original", replyToSenderName: nil)
        let ordinary = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
        XCTAssertNil(ordinary["replyOnlyIfNotLatest"])
        request.replyOnlyIfNotLatest = true
        let feed = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
        XCTAssertEqual(feed["replyOnlyIfNotLatest"] as? Bool, true)
        XCTAssertEqual(feed["contactId"] as? String, "group@g.us")
        XCTAssertEqual(feed["replyTo"] as? String, "selected")
    }

    func testSendMessageRequestEncodesReplyContext() throws {
        let request = SendMessageRequest(
            contactId: "chat@g.us",
            text: "On my way",
            replyTo: "message-1",
            replyToSender: "447700900123@s.whatsapp.net",
            replyToText: "Where are you?",
            replyToSenderName: "Virág"
        )

        let data = try JSONEncoder().encode(request)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(payload["replyTo"] as? String, "message-1")
        XCTAssertEqual(payload["replyToSender"] as? String, "447700900123@s.whatsapp.net")
        XCTAssertEqual(payload["replyToText"] as? String, "Where are you?")
        XCTAssertEqual(payload["replyToSenderName"] as? String, "Virág")
    }

    func testSendImagesRequestEncodesOrderedAlbumAndReplyContext() throws {
        let request = SendImagesRequest(
            contactId: "chat@g.us",
            progressId: "job-7",
            images: [
                SendImageItemRequest(mediaData: "first", mimeType: "image/jpeg"),
                SendImageItemRequest(mediaData: "second", mimeType: "image/png"),
            ],
            caption: "Holiday",
            replyTo: "message-1",
            replyToSender: "447700900123@s.whatsapp.net",
            replyToText: "Send photos",
            replyToSenderName: "Virág"
        )

        let data = try JSONEncoder().encode(request)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let images = try XCTUnwrap(payload["images"] as? [[String: Any]])
        XCTAssertEqual(images.count, 2)
        XCTAssertEqual(images[0]["mediaData"] as? String, "first")
        XCTAssertEqual(images[1]["mimeType"] as? String, "image/png")
        XCTAssertEqual(payload["caption"] as? String, "Holiday")
        XCTAssertEqual(payload["replyTo"] as? String, "message-1")
        XCTAssertEqual(payload["progressId"] as? String, "job-7")
    }

    func testPhotoSendProgressUsesNamedStagesAndCounts() {
        let progress = PhotoSendProgress(
            id: "job-7", contactID: "chat@g.us", stage: .preparing,
            completed: 3, total: 10, error: nil
        )
        XCTAssertEqual(progress.statusText, "Preparing 3 of 10")
        XCTAssertEqual(progress.fractionCompleted, 0.3, accuracy: 0.001)
        var transferring = progress
        transferring.stage = .transferring
        XCTAssertEqual(transferring.statusText, "Transferring 3 of 10")
    }

    func testStagedAlbumRequestsKeepTransferCheckpointIdentity() throws {
        let create = CreatePhotoAlbumRequest(
            jobId: "job-12", contactId: "chat@g.us", photoCount: 12,
            caption: "Walk", replyTo: nil, replyToSender: nil,
            replyToText: nil, replyToSenderName: nil
        )
        let createPayload = try XCTUnwrap(
            JSONSerialization.jsonObject(with: JSONEncoder().encode(create)) as? [String: Any]
        )
        XCTAssertEqual(createPayload["jobId"] as? String, "job-12")
        XCTAssertEqual(createPayload["photoCount"] as? Int, 12)

        let item = StagePhotoAlbumItemRequest(mediaData: "photo-two", mimeType: "image/jpeg")
        let itemPayload = try XCTUnwrap(
            JSONSerialization.jsonObject(with: JSONEncoder().encode(item)) as? [String: Any]
        )
        XCTAssertEqual(itemPayload["mediaData"] as? String, "photo-two")
    }

    func testAppPreferencesPersistStarsConversationPresentationAndTheme() throws {
        let suite = "WhatsAppTranslatorPreferencesTests-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }

        var store = AppPreferencesStore(defaults: defaults)
        store.toggleStar(messageID: "m1", contactID: "chat-1")
        store.setConversationPreferences(
            ConversationPresentationPreferences(nickname: "Mum", timezoneIdentifier: "Europe/Budapest"),
            for: "chat-1"
        )
        store.theme = .dracula
        store.colorMode = .dark

        store = AppPreferencesStore(defaults: defaults)
        XCTAssertTrue(store.isStarred(messageID: "m1", contactID: "chat-1"))
        XCTAssertEqual(store.conversationPreferences(for: "chat-1").nickname, "Mum")
        XCTAssertEqual(store.conversationPreferences(for: "chat-1").timezoneIdentifier, "Europe/Budapest")
        XCTAssertEqual(store.theme, .dracula)
        XCTAssertEqual(store.colorMode, .dark)

        store.toggleStar(messageID: "m1", contactID: "chat-1")
        XCTAssertFalse(store.isStarred(messageID: "m1", contactID: "chat-1"))
    }

    func testLinkPreviewDecodesBackendContract() throws {
        let data = Data(#"{"url":"https://example.com/story","title":"A story","description":"Preview text","imageUrl":"https://example.com/image.jpg","siteName":"Example"}"#.utf8)
        let preview = try JSONDecoder().decode(LinkPreview.self, from: data)
        XCTAssertEqual(preview.title, "A story")
        XCTAssertEqual(preview.imageURL?.absoluteString, "https://example.com/image.jpg")
        XCTAssertEqual(preview.siteName, "Example")
    }

    @MainActor
    func testReactionMessagesAreAppliedToTheirTargetAndHidden() throws {
        let target = try JSONDecoder().decode(
            ChatMessage.self,
            from: Data(#"{"id":"m1","contactId":"chat@g.us","timestamp":1700000000000,"isFromMe":false,"isForwarded":false,"senderName":"Virag","senderPhone":"3630","contactName":"Family","contactPhone":null,"chatType":"group","contentType":"Text","content":{"type":"text","body":"Hello"},"originalText":null,"translatedText":null,"sourceLanguage":null,"isTranslated":false}"#.utf8)
        )
        let reaction = try JSONDecoder().decode(
            ChatMessage.self,
            from: Data(#"{"id":"r1","contactId":"chat@g.us","timestamp":1700000000100,"isFromMe":false,"isForwarded":false,"senderName":"Virag","senderPhone":"3630","contactName":"Family","contactPhone":null,"chatType":"group","contentType":"Reaction","content":{"type":"reaction","emoji":"❤️","target_message_id":"m1"},"originalText":null,"translatedText":null,"sourceLanguage":null,"isTranslated":false}"#.utf8)
        )

        let normalized = AppSession(demoMode: false).normalizeMessages([target, reaction])
        XCTAssertEqual(normalized.map(\.id), ["m1"])
        XCTAssertEqual(normalized.first?.reactions?["❤️"], ["3630"])
    }

    @MainActor
    func testDeclaredReactionTypeCannotRenderAsAStandaloneMessage() throws {
        let target = try decodeMessage(
            id: "m1",
            contentType: "Text",
            content: #"{"type":"text","body":"Agreed"}"#
        )
        let reaction = try decodeMessage(
            id: "r1",
            contentType: "Reaction",
            content: #"{"type":"text","body":"Reaction","emoji":"❤️","target_message_id":"m1"}"#
        )

        XCTAssertTrue(reaction.isReaction)
        let normalized = AppSession(demoMode: false).normalizeMessages([target, reaction])
        XCTAssertEqual(normalized.map(\.id), ["m1"])
        XCTAssertEqual(normalized.first?.reactions?["❤️"], ["3630"])
    }
}
