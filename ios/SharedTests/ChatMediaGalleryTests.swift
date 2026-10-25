import XCTest
#if os(macOS)
@testable import BabelBridgeMac
#else
@testable import WhatsAppTranslator
#endif

@MainActor
final class ChatMediaGalleryTests: XCTestCase {
    private func media(_ id: String, kind: String = "image", chat: String = "gallery@g.us", timestamp: Int64 = 100, revision: Int64 = 0) throws -> ChatMessage {
        let value: [String: Any] = [
            "id": id, "contactId": chat, "timestamp": timestamp,
            "isFromMe": false, "isForwarded": false, "chatType": "group",
            "contentType": kind, "isTranslated": false,
            "content": ["type": kind, "has_media": true, "edited_at_ms": revision]
        ]
        return try JSONDecoder().decode(ChatMessage.self, from: JSONSerialization.data(withJSONObject: value))
    }

    func testGalleryIncludesOnlyChatPhotosAndVideosInStableNewestFirstOrder() throws {
        let items = try [media("photo"), media("video", kind: "video"), media("audio", kind: "audio"), media("sticker", kind: "sticker"), media("document", kind: "document"), media("text", kind: "text"), media("other", chat: "other@g.us"), media("recent", timestamp: 200)]
        XCTAssertEqual(ChatMediaGalleryModel.ordered(items, contactID: "gallery@g.us").map(\.id), ["recent", "video", "photo"])
    }

    func testPaginationUsesServerCursorWithoutSkippingHistoryAfterLiveArrival() async throws {
        let model = ChatMediaGalleryModel(contactID: "gallery@g.us")
        let latest = try media("latest", timestamp: 200)
        let boundary = try media("boundary", kind: "video", timestamp: 100)
        await model.load { cursor in
            XCTAssertNil(cursor)
            return MessagesResponse(messages: [boundary, latest], hasMore: true)
        }
        model.merge([try media("live-old", timestamp: 20), try media("live-new", timestamp: 300)])
        await model.load { cursor in
            XCTAssertEqual(cursor?.id, "boundary")
            return MessagesResponse(messages: [try self.media("older", timestamp: 50)], hasMore: false)
        }
        XCTAssertEqual(model.messages.map(\.id), ["live-new", "latest", "boundary", "older", "live-old"])
        XCTAssertFalse(model.hasMore)
    }

    func testRevokedPhotoCannotReappearFromAnOlderPage() throws {
        let model = ChatMediaGalleryModel(contactID: "gallery@g.us")
        let original = try media("photo", revision: 500)
        model.merge([original])
        let revoked = try media("photo", kind: "revoked")
        model.merge([revoked])
        model.merge([original])
        XCTAssertTrue(model.messages.isEmpty)
        XCTAssertTrue(ChatMediaGalleryModel.ordered([original, revoked, original], contactID: model.contactID).isEmpty)
    }

    func testFailedPageCanRetryWithoutLosingLoadedMedia() async throws {
        let model = ChatMediaGalleryModel(contactID: "gallery@g.us")
        await model.load { _ in MessagesResponse(messages: [try self.media("photo")], hasMore: true) }
        await model.load { _ in throw URLError(.notConnectedToInternet) }
        XCTAssertEqual(model.messages.count, 1)
        XCTAssertNotNil(model.error)
        await model.load { cursor in
            XCTAssertEqual(cursor?.id, "photo")
            return MessagesResponse(messages: [try self.media("older", timestamp: 50)], hasMore: false)
        }
        XCTAssertNil(model.error)
        XCTAssertEqual(model.messages.count, 2)
    }

    func testImageFrameIsReservedFromKnownDimensionsBeforeTheImageLoads() throws {
        let json = #"{"id":"sized","contactId":"gallery@g.us","timestamp":100,"isFromMe":false,"isForwarded":false,"chatType":"group","contentType":"Image","isTranslated":false,"content":{"type":"image","has_media":true,"width":1179,"height":2556}}"#
        let message = try JSONDecoder().decode(ChatMessage.self, from: Data(json.utf8))
        XCTAssertEqual(message.content?.width, 1179)
        XCTAssertEqual(message.content?.height, 2556)
        XCTAssertEqual(AppSession(demoMode: true).reservedImageSize(for: message), CGSize(width: 1179, height: 2556))

        XCTAssertEqual(ChatImageFrame.size(pixelSize: CGSize(width: 1179, height: 2556), maxWidth: 260, maxHeight: 360), CGSize(width: 260, height: 360))
        XCTAssertEqual(ChatImageFrame.size(pixelSize: CGSize(width: 1600, height: 1200), maxWidth: 260, maxHeight: 360), CGSize(width: 260, height: 195))
        XCTAssertEqual(ChatImageFrame.size(pixelSize: nil, maxWidth: 260, maxHeight: 360), CGSize(width: 260, height: 260))
        XCTAssertEqual(ChatImageFrame.size(pixelSize: CGSize(width: 512, height: 512), maxWidth: 180, maxHeight: 180), CGSize(width: 180, height: 180))
    }

    func testRememberedImageSizesSurviveRelaunchAndAreClearedWithTheCache() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "image-sizes-\(UUID().uuidString)/MediaCache", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory.deletingLastPathComponent()) }
        let store = MediaCacheStore(directoryURL: directory, maximumRememberedSizes: 2)
        await store.rememberImageSize(CGSize(width: 800, height: 600), messageID: "oldest")
        await store.rememberImageSize(CGSize(width: 600, height: 800), messageID: "middle")
        await store.rememberImageSize(CGSize(width: 1000, height: 1000), messageID: "newest")
        await store.flushRememberedSizes()

        let relaunched = await MediaCacheStore(directoryURL: directory).rememberedImageSizes()
        XCTAssertEqual(relaunched, ["middle": CGSize(width: 600, height: 800), "newest": CGSize(width: 1000, height: 1000)])

        await store.clear()
        let cleared = await MediaCacheStore(directoryURL: directory).rememberedImageSizes()
        XCTAssertTrue(cleared.isEmpty)
    }
}
