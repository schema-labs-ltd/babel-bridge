import XCTest
import Intents
import SwiftUI
import UserNotifications
@testable import WhatsAppTranslator

final class PhotoGalleryTests: XCTestCase {
    func testConsecutivePhotosWithoutAlbumMetadataBecomeOneGallery() throws {
        let photos = try (0..<30).map { try photo("p\($0)", timestamp: 1_783_940_000_000 + Int64($0) * 1_000) }
        let items = ConversationTimelineBuilder.items(from: photos)
        XCTAssertEqual(items.count, 1)
        XCTAssertEqual(items.first?.messages.map(\.id), photos.map(\.id))
        XCTAssertEqual(items.first?.id, photos.first?.id)
        XCTAssertEqual(ConversationTimelineBuilder.items(from: []).count, 0)
    }

    func testInterveningMessagesSendersChatsAndDayChangesBreakGalleries() throws {
        let first = try photo("first", album: "shared")
        let last = try photo("last", album: "shared")
        let boundaries = [
            try photo("text", kind: "text"),
            try photo("video", kind: "video"),
            try photo("someone-else", sender: "447700900456"),
            try photo("another-chat", contact: "another@g.us"),
            try photo("outgoing", fromMe: true),
            try photo("tomorrow", timestamp: 1_783_940_000_000 + 86_400_000),
        ]
        for boundary in boundaries {
            let items = ConversationTimelineBuilder.items(from: [first, boundary, last])
            XCTAssertEqual(items.count, 3, boundary.id)
            XCTAssertEqual(items.flatMap(\.messages).map(\.id), [first.id, boundary.id, last.id])
        }
    }

    func testAdjacentSeparateAlbumsAndStandaloneImagesShareOneGallery() throws {
        let photos = [try photo("a", album: "first"), try photo("b", album: "second"), try photo("c")]
        XCTAssertEqual(ConversationTimelineBuilder.items(from: photos).count, 1)
        XCTAssertEqual(ConversationTimelineBuilder.items(from: photos).first?.messages.map(\.id), ["a", "b", "c"])
    }

    func testJoinedAlbumsRetainTheirOwnWhatsAppPhotoOrder() throws {
        let photos = [try photo("a1", album: "a", albumIndex: 1), try photo("a0", album: "a", albumIndex: 0), try photo("single"), try photo("b1", album: "b", albumIndex: 1), try photo("b0", album: "b", albumIndex: 0)]
        let items = ConversationTimelineBuilder.items(from: photos)
        XCTAssertEqual(items.count, 1)
        XCTAssertEqual(items.first?.messages.map(\.id), ["a0", "a1", "single", "b0", "b1"])
    }

    func testLiveAppendAndHistoryPagesPreserveEveryPhotoAndStableFirstAnchor() throws {
        let photos = try (0..<8).map { try photo("p\($0)") }
        let first = ConversationTimelineBuilder.items(from: [photos[0]])
        let extended = ConversationTimelineBuilder.items(from: photos)
        XCTAssertEqual(first.first?.id, extended.first?.id)
        let recent = Array(photos.suffix(3))
        XCTAssertEqual(ConversationTimelineBuilder.items(from: recent).count, 1)
        XCTAssertEqual(ConversationTimelineBuilder.items(from: Array(photos.prefix(5)) + recent).first?.messages.count, 8)
        XCTAssertEqual(extended.first?.messages[7].content?.caption, "Caption p7")
        XCTAssertEqual(extended.first?.messages[7].reactions, ["❤️": ["actor"]])
        XCTAssertEqual(extended.first?.messages[7].replyTarget.messageID, "p7")
    }

    func testUnknownGroupSenderDoesNotMergeNamesButDirectAndOutgoingPhotosGroup() throws {
        let unknown = [try photo("a", sender: nil), try photo("b", sender: nil)]
        XCTAssertEqual(ConversationTimelineBuilder.items(from: unknown).count, 2)
        XCTAssertEqual(ConversationTimelineBuilder.items(from: [try photo("a", sender: nil, contact: "one@s.whatsapp.net"), try photo("b", sender: nil, contact: "one@s.whatsapp.net")]).count, 1)
        XCTAssertEqual(ConversationTimelineBuilder.items(from: [try photo("a", sender: nil, fromMe: true), try photo("b", sender: nil, fromMe: true)]).count, 1)
    }

    func testCompactPreviewAndGalleryNavigationKeepEveryPageReachable() {
        XCTAssertEqual(PhotoGalleryLayout.previewLimit, 4)
        XCTAssertEqual(PhotoGalleryLayout.hiddenCount(total: 30), 26)
        XCTAssertEqual(PhotoGalleryLayout.hiddenCount(total: 2), 0)
        var page = 0
        for expected in 1..<30 {
            page = PhotoGalleryLayout.page(after: 1, current: page, count: 30)
            XCTAssertEqual(page, expected)
        }
        XCTAssertEqual(PhotoGalleryLayout.page(after: 1, current: page, count: 30), 29)
        XCTAssertEqual(PhotoGalleryLayout.page(after: -1, current: 0, count: 30), 0)
    }

    private func photo(_ id: String, sender: String? = "447700900123", contact: String = "gallery@g.us", fromMe: Bool = false, kind: String = "image", timestamp: Int64 = 1_783_940_000_000, album: String? = nil, albumIndex: Int? = nil) throws -> ChatMessage {
        var content: [String: Any] = ["type": kind, "caption": "Caption \(id)", "has_media": true]
        if let album { content["album_id"] = album }
        if let albumIndex { content["album_index"] = albumIndex }
        var payload: [String: Any] = [
            "id": id, "contactId": contact, "timestamp": timestamp, "isFromMe": fromMe,
            "isForwarded": false, "senderName": "Alex", "contactName": "Weekend photos",
            "chatType": contact.contains("@g.us") ? "group" : "private", "contentType": kind,
            "content": content, "isTranslated": false, "reactions": ["❤️": ["actor"]],
        ]
        if let sender { payload["senderPhone"] = sender }
        return try JSONDecoder().decode(ChatMessage.self, from: JSONSerialization.data(withJSONObject: payload))
    }
}
