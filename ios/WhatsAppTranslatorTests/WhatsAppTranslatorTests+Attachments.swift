import XCTest
import Intents
import SwiftUI
import UserNotifications
@testable import WhatsAppTranslator

extension WhatsAppTranslatorTests {
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
        for asset in assets { XCTAssertNotNil(UIImage(named: asset), asset) }
    }

    func testOversizedPhotoIsAutomaticallyReducedBelowUploadLimit() throws {
        let image = DemoImageFactory.landscape(size: CGSize(width: 1_200, height: 900))
        var oversizedData = try XCTUnwrap(image.platformJPEGData(compressionQuality: 0.95))
        oversizedData.append(Data(repeating: 0, count: 17 * 1_024 * 1_024))

        let prepared = try XCTUnwrap(
            PhotoUploadPreparer.prepare(
                data: oversizedData,
                mimeType: "image/jpeg",
                image: image,
                maximumBytes: PhotoUploadPreparer.maximumBytes(forPhotoCount: 1)
            )
        )

        XCTAssertLessThanOrEqual(prepared.data.count, 15 * 1_024 * 1_024)
        XCTAssertEqual(prepared.mimeType, "image/jpeg")
        XCTAssertLessThan(prepared.data.count, oversizedData.count)
    }

    func testAlbumPhotoBudgetKeepsCombinedUploadBelowSixtyMegabytes() {
        XCTAssertEqual(PhotoUploadPreparer.maximumBytes(forPhotoCount: 1), 15 * 1_024 * 1_024)
        XCTAssertEqual(PhotoUploadPreparer.maximumBytes(forPhotoCount: 4), 15 * 1_024 * 1_024)
        XCTAssertEqual(PhotoUploadPreparer.maximumBytes(forPhotoCount: 30), 2 * 1_024 * 1_024)
    }

    func testStandaloneEmojiPresentationAcceptsUpToThreeEmojiOnly() {
        func message(_ body: String) -> ChatMessage {
            ChatMessage(
                id: UUID().uuidString,
                contactId: "family@g.us",
                timestamp: 1_700_000_000_000,
                isFromMe: false,
                isForwarded: false,
                senderName: "Virág",
                senderPhone: nil,
                contactName: "Family",
                contactPhone: nil,
                chatType: "group",
                contentType: "Text",
                content: MessageContent(type: "text", body: body, showTranslatedPrimary: nil, replyContext: nil),
                originalText: nil,
                translatedText: nil,
                sourceLanguage: nil,
                isTranslated: false
            )
        }

        XCTAssertEqual(message("😉").standaloneEmojiText, "😉")
        XCTAssertEqual(message("👨‍👩‍👧‍👦👍🏽🥳").standaloneEmojiText, "👨‍👩‍👧‍👦👍🏽🥳")
        XCTAssertNil(message("😀😃🥳❤️").standaloneEmojiText)
        XCTAssertNil(message("Hello 👋").standaloneEmojiText)
        XCTAssertNil(message("123").standaloneEmojiText)
    }

    func testMediaCachePersistsAcrossStoreInstancesAndEvictsOldestFiles() async throws {
        let root = FileManager.default.temporaryDirectory
            .appending(path: "MediaCacheTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: root) }

        let firstStore = MediaCacheStore(directoryURL: root, maximumBytes: 7)
        let firstURL = try await firstStore.store(
            Data([1, 2, 3, 4]),
            messageID: "message/one",
            fileExtension: "jpg"
        )
        try FileManager.default.setAttributes(
            [.modificationDate: Date(timeIntervalSince1970: 1)],
            ofItemAtPath: firstURL.path
        )

        let relaunchedStore = MediaCacheStore(directoryURL: root, maximumBytes: 7)
        let cachedURL = try await relaunchedStore.cachedURL(for: "message/one")
        let restoredURL = try XCTUnwrap(cachedURL)
        XCTAssertEqual(try Data(contentsOf: restoredURL), Data([1, 2, 3, 4]))

        _ = try await relaunchedStore.store(
            Data([5, 6, 7, 8]),
            messageID: "message/two",
            fileExtension: "mp4"
        )

        let evictedURL = try await relaunchedStore.cachedURL(for: "message/one")
        let retainedURL = try await relaunchedStore.cachedURL(for: "message/two")
        XCTAssertNil(evictedURL)
        XCTAssertNotNil(retainedURL)
    }

    func testExpectedRequestCancellationsDoNotBecomeUserFacingErrors() {
        XCTAssertTrue(AppSession.isExpectedCancellation(CancellationError()))
        XCTAssertTrue(AppSession.isExpectedCancellation(URLError(.cancelled)))
        XCTAssertFalse(AppSession.isExpectedCancellation(URLError(.timedOut)))
    }
}
