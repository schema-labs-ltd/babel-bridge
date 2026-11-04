import XCTest
import Intents
import SwiftUI
import UserNotifications
@testable import WhatsAppTranslator

extension WhatsAppTranslatorTests {
    func testServerConfigurationNormalizesAddress() throws {
        let configuration = try ServerConfiguration.make(
            address: " translator.example.com/ ",
            password: "secret"
        )
        XCTAssertEqual(configuration.baseURL.absoluteString, "https://translator.example.com")
        XCTAssertEqual(configuration.password, "secret")
    }

    func testContactAndMessageDecodeBackendPayloads() throws {
        let contactData = Data(#"{"id":"chat@g.us","name":"Family","phone":null,"type":"group","lastMessageTime":1700000000000,"unreadCount":3,"pinnedAt":null,"lastMessagePreview":"Hello"}"#.utf8)
        let contact = try JSONDecoder().decode(Contact.self, from: contactData)
        XCTAssertTrue(contact.isGroup)
        XCTAssertEqual(contact.displayName, "Family")

        let messageData = Data(#"{"id":"m1","contactId":"chat@g.us","timestamp":1700000000000,"isFromMe":false,"isForwarded":false,"senderName":"Virag","senderPhone":null,"contactName":"Family","contactPhone":null,"chatType":"group","contentType":"Text","content":{"type":"text","body":"Szia"},"originalText":"Szia","translatedText":"Hello","sourceLanguage":"Hungarian","isTranslated":true}"#.utf8)
        let message = try JSONDecoder().decode(ChatMessage.self, from: messageData)
        XCTAssertEqual(message.displayText, "Hello")
        XCTAssertEqual(message.alternateText, "Szia")
    }

    func testOutgoingMessageDecodesDeliveryAndReadStates() throws {
        let delivered = try JSONDecoder().decode(
            ChatMessage.self,
            from: Data(#"{"id":"m1","contactId":"chat@g.us","timestamp":1700000000000,"isFromMe":true,"isForwarded":false,"senderName":null,"senderPhone":null,"contactName":"Family","contactPhone":null,"chatType":"group","contentType":"Text","content":{"type":"text","body":"Hello"},"originalText":null,"translatedText":null,"sourceLanguage":null,"isTranslated":false,"deliveryStatus":"delivered"}"#.utf8)
        )
        let read = try JSONDecoder().decode(
            ChatMessage.self,
            from: Data(#"{"id":"m2","contactId":"chat@g.us","timestamp":1700000000000,"isFromMe":true,"isForwarded":false,"senderName":null,"senderPhone":null,"contactName":"Family","contactPhone":null,"chatType":"group","contentType":"Text","content":{"type":"text","body":"Hello"},"originalText":null,"translatedText":null,"sourceLanguage":null,"isTranslated":false,"deliveryStatus":"read"}"#.utf8)
        )

        XCTAssertEqual(delivered.deliveryState, .delivered)
        XCTAssertEqual(read.deliveryState, .read)
        XCTAssertEqual(read.deliveryState.accessibilityLabel, "Read")
    }

    func testLiveReceiptAndReadEventsDecodeBackendFieldNames() throws {
        let receipt = try JSONDecoder().decode(
            LiveEvent.self,
            from: Data(#"{"type":"receipt","message_ids":["m1","m2"],"status":"read"}"#.utf8)
        )
        let markRead = try JSONDecoder().decode(
            LiveEvent.self,
            from: Data(#"{"type":"mark_as_read","chat_id":"family@g.us"}"#.utf8)
        )

        XCTAssertEqual(receipt.messageIds, ["m1", "m2"])
        XCTAssertEqual(receipt.status, "read")
        XCTAssertEqual(markRead.chatId, "family@g.us")
    }

    func testDraftStorePersistsAndRemovesPerChatDrafts() throws {
        let suite = "WhatsAppTranslatorTests-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = DraftStore(defaults: defaults)

        store.save("Still typing", for: "chat-1")
        XCTAssertEqual(store.text(for: "chat-1"), "Still typing")
        store.save("   ", for: "chat-1")
        XCTAssertEqual(store.text(for: "chat-1"), "")
    }

    func testPushDeviceRegistrationEncodesBackendContract() throws {
        let registration = PushDeviceRegistration(
            installationId: "installation-1",
            token: "0123456789abcdef",
            environment: "sandbox"
        )

        let data = try JSONEncoder().encode(registration)
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: String])

        XCTAssertEqual(payload["installationId"], "installation-1")
        XCTAssertEqual(payload["token"], "0123456789abcdef")
        XCTAssertEqual(payload["environment"], "sandbox")
    }

    func testChatCachePersistsMessagesForTheConfiguredServer() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "WhatsAppTranslatorTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }

        let configuration = try ServerConfiguration.make(
            address: "https://translator.example.com",
            password: "secret"
        )
        let contact = try JSONDecoder().decode(
            Contact.self,
            from: Data(#"{"id":"chat@g.us","name":"Family","phone":null,"type":"group","lastMessageTime":1700000000000,"unreadCount":3,"pinnedAt":null,"lastMessagePreview":"Hello"}"#.utf8)
        )
        let message = try JSONDecoder().decode(
            ChatMessage.self,
            from: Data(#"{"id":"m1","contactId":"chat@g.us","timestamp":1700000000000,"isFromMe":false,"isForwarded":false,"senderName":"Virag","senderPhone":null,"contactName":"Family","contactPhone":null,"chatType":"group","contentType":"Text","content":{"type":"text","body":"Szia"},"originalText":"Szia","translatedText":"Hello","sourceLanguage":"Hungarian","isTranslated":true}"#.utf8)
        )
        let store = ChatCacheStore(directoryURL: directory)

        await store.save(
            ChatCacheSnapshot(
                serverBaseURL: configuration.baseURL.absoluteString,
                contacts: [contact],
                messages: [contact.id: [message]],
                updatedAt: Date(timeIntervalSince1970: 1_700_000_000)
            )
        )

        let restored = await store.load(for: configuration)
        XCTAssertEqual(restored?.contacts, [contact])
        XCTAssertEqual(restored?.messages[contact.id], [message])
    }

    @MainActor
    func testCachedFeedShowsAtLaunchUntilTheServerAnswers() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "WhatsAppTranslatorTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let configuration = try ServerConfiguration.make(address: "https://translator.example.com", password: "secret")
        let message = try JSONDecoder().decode(
            ChatMessage.self,
            from: Data(#"{"id":"m1","contactId":"chat@g.us","timestamp":1700000000000,"isFromMe":false,"isForwarded":false,"chatType":"group","contentType":"Text","content":{"type":"text","body":"Szia"},"isTranslated":false}"#.utf8)
        )
        let store = ChatCacheStore(directoryURL: directory)
        await store.save(ChatCacheSnapshot(
            serverBaseURL: configuration.baseURL.absoluteString,
            contacts: [],
            messages: [message.contactId: [message]],
            updatedAt: Date(),
            feed: [message]
        ))
        // A background sync rewrites the file and must keep the feed.
        let loaded = await store.load(for: configuration)
        let synced = try XCTUnwrap(loaded).merging(contacts: [], messages: [], for: "other@s.whatsapp.net")
        XCTAssertEqual(synced.feed, [message])

        let session = AppSession(cache: store, mediaCache: MediaCacheStore(directoryURL: directory.appending(path: "Media")), demoMode: false)
        session.configuration = configuration
        let restored = await session.restoreCachedState()
        XCTAssertTrue(restored)
        XCTAssertTrue(session.feedHasLoaded)
        XCTAssertEqual(session.unifiedMessages.map(\.id), ["m1"])
        XCTAssertEqual(session.feedCursor?.id, "m1")

        // A feed already loaded in this session is newer than the file and stays.
        session.feedByID = [:]
        _ = await session.restoreCachedState()
        XCTAssertTrue(session.unifiedMessages.isEmpty)
    }

    func testChatCacheFromAnEarlierVersionLoadsWithoutAFeed() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "WhatsAppTranslatorTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let configuration = try ServerConfiguration.make(address: "https://translator.example.com", password: "secret")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let json = #"{"contacts":[],"messages":{},"serverBaseURL":"\#(configuration.baseURL.absoluteString)","updatedAt":1700000000000}"#
        try Data(json.utf8).write(to: directory.appending(path: "chat-cache.json"))

        let restored = await ChatCacheStore(directoryURL: directory).load(for: configuration)
        XCTAssertNotNil(restored)
        XCTAssertNil(restored?.feed)
    }

    func testMediaCacheLookupNoticesFilesRemovedOutsideTheStore() async throws {
        let root = FileManager.default.temporaryDirectory
            .appending(path: "MediaCacheTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = MediaCacheStore(directoryURL: root)
        let url = try await store.store(Data([1, 2, 3]), messageID: "photo", fileExtension: "jpg")
        let found = try await store.cachedURL(for: "photo")
        XCTAssertEqual(found, url)

        try FileManager.default.removeItem(at: url)
        let missing = try await store.cachedURL(for: "photo")
        XCTAssertNil(missing)

        let replaced = try await store.store(Data([4]), messageID: "photo", fileExtension: "png")
        let relaunched = try await MediaCacheStore(directoryURL: root).cachedURL(for: "photo")
        XCTAssertEqual(relaunched?.lastPathComponent, replaced.lastPathComponent)
        try await store.remove(messageID: "photo")
        let removed = try await store.cachedURL(for: "photo")
        XCTAssertNil(removed)
    }

    func testChatCacheDoesNotRestoreAnotherServer() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "WhatsAppTranslatorTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let first = try ServerConfiguration.make(address: "https://one.example.com", password: "secret")
        let second = try ServerConfiguration.make(address: "https://two.example.com", password: "secret")
        let store = ChatCacheStore(directoryURL: directory)

        await store.save(
            ChatCacheSnapshot(
                serverBaseURL: first.baseURL.absoluteString,
                contacts: [],
                messages: [:],
                updatedAt: Date()
            )
        )

        let restored = await store.load(for: second)
        XCTAssertNil(restored)
    }

    func testOlderChatCacheWriteCannotReplaceNewerBackgroundData() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "WhatsAppTranslatorTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let configuration = try ServerConfiguration.make(
            address: "https://translator.example.com",
            password: "secret"
        )
        let store = ChatCacheStore(directoryURL: directory)
        let newerDate = Date(timeIntervalSince1970: 1_700_000_100)

        await store.save(
            ChatCacheSnapshot(
                serverBaseURL: configuration.baseURL.absoluteString,
                contacts: [],
                messages: [:],
                updatedAt: newerDate
            )
        )
        await store.save(
            ChatCacheSnapshot(
                serverBaseURL: configuration.baseURL.absoluteString,
                contacts: [],
                messages: [:],
                updatedAt: Date(timeIntervalSince1970: 1_700_000_000)
            )
        )

        let restored = await store.load(for: configuration)
        XCTAssertEqual(restored?.updatedAt, newerDate)
    }

    func testMessageContentDecodesRichMediaAndReactionContracts() throws {
        let image = try JSONDecoder().decode(
            MessageContent.self,
            from: Data(#"{"type":"image","caption":"A view","mime_type":"image/jpeg","has_media":true,"file_size":2048}"#.utf8)
        )
        XCTAssertEqual(image.caption, "A view")
        XCTAssertEqual(image.mimeType, "image/jpeg")
        XCTAssertEqual(image.hasMedia, true)
        XCTAssertEqual(image.fileSize, 2_048)

        let reaction = try JSONDecoder().decode(
            MessageContent.self,
            from: Data(#"{"type":"reaction","emoji":"❤️","target_message_id":"m1"}"#.utf8)
        )
        XCTAssertEqual(reaction.emoji, "❤️")
        XCTAssertEqual(reaction.targetMessageId, "m1")
    }

    func testPhotoAlbumMetadataDecodesAndGroupsInWhatsAppOrder() throws {
        let later = try decodeMessage(
            id: "photo-2",
            contentType: "Image",
            content: #"{"type":"image","mime_type":"image/jpeg","album_id":"album-1","album_index":2}"#
        )
        let first = try decodeMessage(
            id: "photo-0",
            contentType: "Image",
            content: #"{"type":"image","mime_type":"image/jpeg","album_id":"album-1","album_index":0}"#
        )
        let middle = try decodeMessage(
            id: "photo-1",
            contentType: "Image",
            content: #"{"type":"image","mime_type":"image/jpeg","album_id":"album-1","album_index":1}"#
        )

        let items = ConversationTimelineBuilder.items(from: [later, first, middle])

        XCTAssertEqual(items.count, 1)
        guard case let .photoAlbum(album) = items[0] else {
            return XCTFail("Expected one grouped photo album")
        }
        XCTAssertEqual(album.id, "photo-2")
        XCTAssertEqual(album.messages.map(\.id), ["photo-0", "photo-1", "photo-2"])
        XCTAssertEqual(album.messages.map(\.content?.albumIndex), [0, 1, 2])
    }

    func testEveryWebVisibleRichMessageTypeHasANativePresentationContract() throws {
        let video = try decodeMessage(
            id: "video",
            contentType: "Video",
            content: #"{"type":"video","caption":"At the park","mime_type":"video/mp4","has_media":true,"duration_seconds":12.5}"#
        )
        XCTAssertEqual(video.mediaKind, .video)
        XCTAssertEqual(video.displayText, "At the park")

        let voiceNote = try decodeMessage(
            id: "voice",
            contentType: "Audio",
            content: #"{"type":"audio","mime_type":"audio/ogg","has_media":true,"is_voice_note":true,"duration_seconds":9}"#
        )
        XCTAssertEqual(voiceNote.mediaKind, .audio)
        XCTAssertEqual(voiceNote.displayText, "Voice note")

        let document = try decodeMessage(
            id: "document",
            contentType: "Document",
            content: #"{"type":"document","mime_type":"application/pdf","has_media":true,"file_name":"travel-plan.pdf","file_size":4096}"#
        )
        XCTAssertEqual(document.mediaKind, .document)
        XCTAssertEqual(document.displayText, "travel-plan.pdf")

        let sticker = try decodeMessage(
            id: "sticker",
            contentType: "Sticker",
            content: #"{"type":"sticker","mime_type":"image/webp","has_media":true,"is_animated":false}"#
        )
        XCTAssertEqual(sticker.mediaKind, .sticker)
        XCTAssertEqual(sticker.displayText, "Sticker")

        let location = try decodeMessage(
            id: "location",
            contentType: "Location",
            content: #"{"type":"location","name":"Budapest Parliament","address":"Kossuth Lajos tér","latitude":47.5071,"longitude":19.0457}"#
        )
        XCTAssertEqual(location.displayText, "Budapest Parliament")
        XCTAssertEqual(location.locationURL?.host(), "maps.apple.com")

        let contact = try decodeMessage(
            id: "contact",
            contentType: "Contact",
            content: #"{"type":"contact","display_name":"Sample Sender","vcard":"BEGIN:VCARD\\nTEL:+447700900123\\nEND:VCARD"}"#
        )
        XCTAssertEqual(contact.displayText, "Sample Sender")

        let poll = try decodeMessage(
            id: "poll",
            contentType: "Poll",
            content: #"{"type":"poll","question":"Dinner?","options":["Pizza","Curry"]}"#
        )
        XCTAssertEqual(poll.displayText, "Dinner?")
        XCTAssertEqual(poll.content?.options, ["Pizza", "Curry"])

        let revoked = try decodeMessage(
            id: "revoked",
            contentType: "Revoked",
            content: #"{"type":"revoked"}"#
        )
        XCTAssertEqual(revoked.displayText, "This message was deleted")
    }

    func testTypingIndicatorsFollowPresenceAndExpire() {
        let now = Date(timeIntervalSince1970: 1_000)
        var typing = TypingIndicators()
        let none: (String) -> String? = { _ in nil }
        typing.apply(chatID: "ana", userID: "ana", state: "typing", now: now)
        XCTAssertEqual(typing.label(in: "ana", isGroup: false, now: now, name: none), "typing…")
        typing.apply(chatID: "ana", userID: "ana", state: "recording", now: now)
        XCTAssertEqual(typing.label(in: "ana", isGroup: false, now: now, name: none), "recording audio…")
        typing.apply(chatID: "ana", userID: "ana", state: "paused", now: now)
        XCTAssertNil(typing.label(in: "ana", isGroup: false, now: now, name: none))

        typing.apply(chatID: "family", userID: "alex", state: "typing", now: now)
        XCTAssertEqual(typing.label(in: "family", isGroup: true, now: now) { _ in "Alex" }, "Alex is typing…")
        typing.apply(chatID: "family", userID: "sam", state: "typing", now: now)
        XCTAssertEqual(typing.label(in: "family", isGroup: true, now: now, name: none), "2 people are typing…")
        typing.stop(chatID: "family", userID: "sam", now: now)
        XCTAssertEqual(typing.active(in: "family", now: now).map(\.userID), ["alex"])

        // A typing state not heard again lapses, as it does in WhatsApp.
        let later = now.addingTimeInterval(TypingIndicators.lifetime + 1)
        XCTAssertNil(typing.label(in: "family", isGroup: true, now: later, name: none))
        typing.prune(now: later)
        XCTAssertEqual(typing.chats, [:])
    }
}
