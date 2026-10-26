import AppKit
import SwiftUI
import XCTest
import UserNotifications
@testable import BabelBridgeMac

extension BabelBridgeMacTests {
    @MainActor
    func testUnifiedAttachmentsCaptureBeforePickingAndPreserveTextDraft() async throws {
        func message(_ id: String, _ contact: String, _ time: Int64) throws -> ChatMessage {
            let json: [String: Any] = ["id": id, "contactId": contact, "timestamp": time, "isFromMe": false, "isForwarded": false, "chatType": "group", "contentType": "Text", "isTranslated": false]
            return try JSONDecoder().decode(ChatMessage.self, from: JSONSerialization.data(withJSONObject: json))
        }
        let first = try message("selected", "one@g.us", 100)
        let newer = try message("newer", "one@g.us", 200)
        let elsewhere = try message("elsewhere", "two@g.us", 300)
        var draft = UnifiedReplyDraft()
        XCTAssertNil(draft.beginAttachment(latestMessage: nil))
        XCTAssertFalse(draft.isFocused)
        let target = try XCTUnwrap(draft.beginAttachment(latestMessage: first))
        XCTAssertTrue(draft.isFocused)
        XCTAssertEqual(draft.beginAttachment(latestMessage: elsewhere)?.id, first.id)
        draft.updateText("Keep this draft", latestMessage: newer)
        let session = AppSession(demoMode: true)
        session.messages = [first.contactId: [first, newer], elsewhere.contactId: [elsewhere]]
        await session.loadFeed()
        let attachment = OutgoingAttachment(data: Data("Test file".utf8), mimeType: "text/plain", fileName: "Note.txt", kind: "document")
        let sent = await session.sendAttachment(attachment, caption: "Attached", to: target.contactId, reply: session.replyTarget(for: target), replyOnlyIfNotLatest: true)
        XCTAssertTrue(sent)
        XCTAssertEqual(session.messages[first.contactId]?.last?.content?.replyContext?.messageId, first.id)
        XCTAssertEqual(session.messages[elsewhere.contactId]?.count, 1)
        XCTAssertEqual(session.unifiedMessages.last?.contactId, first.contactId)
        draft.finishMediaSending(to: target)
        XCTAssertNil(draft.selected)
        XCTAssertEqual(draft.drafts[first.contactId], "Keep this draft")
        draft.select(elsewhere)
        XCTAssertEqual(draft.beginAttachment(latestMessage: newer)?.id, elsewhere.id)
        let normal = await session.sendAttachment(attachment, caption: nil, to: elsewhere.contactId, reply: session.replyTarget(for: elsewhere), replyOnlyIfNotLatest: true)
        XCTAssertTrue(normal)
        XCTAssertNil(session.messages[elsewhere.contactId]?.last?.content?.replyContext)
        draft.cancelSelection()
        XCTAssertEqual(draft.beginAttachment(latestMessage: newer)?.id, newer.id)
    }

    func testUnifiedAttachmentRequestsRetainTargetAndConditionalQuote() throws {
        let media = SendImageRequest(mediaKind: "video", fileName: "Video.mp4", replyOnlyIfNotLatest: true, contactId: "one@g.us", mediaData: "ZmlsZQ==", mimeType: "video/mp4", caption: nil, replyTo: "selected", replyToSender: "sam@s.whatsapp.net", replyToText: "Original", replyToSenderName: "Sam")
        let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(media)) as? [String: Any])
        XCTAssertEqual(encoded["contactId"] as? String, "one@g.us")
        XCTAssertEqual(encoded["replyTo"] as? String, "selected")
        XCTAssertEqual(encoded["replyOnlyIfNotLatest"] as? Bool, true)
        XCTAssertEqual(encoded["mediaKind"] as? String, "video")
        let album = CreatePhotoAlbumRequest(replyOnlyIfNotLatest: true, jobId: "album-job", contactId: "one@g.us", photoCount: 2, caption: nil, replyTo: "selected", replyToSender: nil, replyToText: nil, replyToSenderName: nil)
        let staged = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(album)) as? [String: Any])
        XCTAssertEqual(staged["replyTo"] as? String, "selected")
        XCTAssertEqual(staged["replyOnlyIfNotLatest"] as? Bool, true)
        XCTAssertTrue(SendRecoveryStore.isSend(path: "/api/send-media", method: "POST"))
        let voice = PrepareVoiceRequest(contactId: "one@g.us", mediaData: "ZmlsZQ==", replyTo: "selected", replyToSender: nil, replyToText: "Original", replyOnlyIfNotLatest: true)
        let recording = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(voice)) as? [String: Any])
        XCTAssertEqual(recording["replyTo"] as? String, "selected")
        XCTAssertEqual(recording["replyOnlyIfNotLatest"] as? Bool, true)
        XCTAssertNil(recording["replyToSender"])
    }

    @MainActor
    func testWallpaperUpgradePreservesExistingPreferencesAndPersistsSelection() throws {
        let suite = "WallpaperMigration-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let legacy = Data(#"{"starredMessageIDs":{"chat":["message"]},"conversations":{"chat":{"nickname":"Friend","timezoneIdentifier":"Europe/London"}},"theme":"ocean","colorMode":"dark"}"#.utf8)
        defaults.set(legacy, forKey: "whatsapp-translator-ios-preferences-v1")
        let store = AppPreferencesStore(defaults: defaults)
        XCTAssertEqual(store.wallpaper, .classic)
        XCTAssertEqual(store.theme, .ocean)
        XCTAssertEqual(store.colorMode, .dark)
        XCTAssertTrue(store.isStarred(messageID: "message", contactID: "chat"))
        XCTAssertEqual(store.nickname(for: "chat"), "Friend")
        store.wallpaper = .celestial
        let restored = AppPreferencesStore(defaults: defaults)
        XCTAssertEqual(restored.wallpaper, .celestial)
        XCTAssertEqual(restored.theme, .ocean)
        XCTAssertTrue(restored.isStarred(messageID: "message", contactID: "chat"))
    }

    func testAllTenGeneratedWallpaperAssetsAreBundled() throws {
        let assets = AppWallpaper.allCases.compactMap(\.assetName)
        XCTAssertEqual(assets.count, 10)
        XCTAssertEqual(Set(assets).count, 10)
        for asset in assets { XCTAssertNotNil(NSImage(named: asset), asset) }
    }

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
}
