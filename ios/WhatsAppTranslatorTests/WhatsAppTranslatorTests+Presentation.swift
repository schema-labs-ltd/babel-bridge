import XCTest
import Intents
import SwiftUI
import UserNotifications
@testable import WhatsAppTranslator

extension WhatsAppTranslatorTests {
    func testCompleteConversationSearchRequestsAllMessages() {
        XCTAssertEqual(
            APIClient.messagesPath(contactID: "family@g.us", limit: 0),
            "/api/messages/family@g.us?limit=0"
        )
    }

    func testMessageActionAvailabilityMatchesVisibleControls() throws {
        let incoming = try decodeMessage(
            id: "incoming",
            contentType: "Text",
            content: #"{"type":"text","body":"Szia"}"#
        )
        XCTAssertEqual(incoming.availableActions, [.reply, .translate, .aiReply, .star, .react])

        let outgoingData = Data(#"{"id":"outgoing","contactId":"chat@g.us","timestamp":1700000000000,"isFromMe":true,"isForwarded":false,"senderName":null,"senderPhone":"447700900123","contactName":"Family","contactPhone":null,"chatType":"group","contentType":"Text","content":{"type":"text","body":"Hello"},"originalText":null,"translatedText":null,"sourceLanguage":null,"isTranslated":false}"#.utf8)
        let outgoing = try JSONDecoder().decode(ChatMessage.self, from: outgoingData)
        XCTAssertEqual(outgoing.availableActions, [.reply, .star, .react])
    }

    func testSwipeToReplyRequiresADeliberateHorizontalGesture() {
        XCTAssertFalse(MessageSwipeReply.shouldReply(translation: CGSize(width: 42, height: 3)))
        XCTAssertTrue(MessageSwipeReply.shouldReply(translation: CGSize(width: 64, height: 4)))
        XCTAssertFalse(MessageSwipeReply.shouldReply(translation: CGSize(width: 70, height: 80)))
        XCTAssertFalse(MessageSwipeReply.shouldReply(translation: CGSize(width: -80, height: 2)))
        XCTAssertFalse(MessageSwipeReply.shouldBegin(velocity: CGPoint(x: 20, y: 80)))
        XCTAssertFalse(MessageSwipeReply.shouldBegin(velocity: CGPoint(x: -80, y: 2)))
        XCTAssertTrue(MessageSwipeReply.shouldBegin(velocity: CGPoint(x: 80, y: 20)))
    }

    func testSwipeToReplyOffsetIgnoresVerticalAndLeftwardDragsAndClampsItsReveal() {
        XCTAssertEqual(MessageSwipeReply.offset(translation: CGSize(width: -20, height: 0)), 0)
        XCTAssertEqual(MessageSwipeReply.offset(translation: CGSize(width: 30, height: 40)), 0)
        XCTAssertEqual(MessageSwipeReply.offset(translation: CGSize(width: 35, height: 2)), 35)
        XCTAssertEqual(MessageSwipeReply.offset(translation: CGSize(width: 120, height: 2)), 72)
    }

    func testPhotoViewerZoomClampsAndDoubleTapTogglesMagnification() {
        XCTAssertEqual(PhotoViewerZoom.clampedScale(0.5), 1)
        XCTAssertEqual(PhotoViewerZoom.clampedScale(3), 3)
        XCTAssertEqual(PhotoViewerZoom.clampedScale(8), 5)
        XCTAssertEqual(PhotoViewerZoom.toggledScale(from: 1), 2.5)
        XCTAssertEqual(PhotoViewerZoom.toggledScale(from: 2.5), 1)
    }

    @MainActor
    func testPinnedContactsStayAboveUnpinnedChatsInPinOrder() {
        let recent = Contact(
            id: "recent",
            name: "Recent",
            phone: nil,
            type: "private",
            lastMessageTime: 300,
            unreadCount: 0,
            pinnedAt: nil,
            lastMessagePreview: nil
        )
        let secondPinned = Contact(
            id: "second-pinned",
            name: "Second pinned",
            phone: nil,
            type: "private",
            lastMessageTime: 200,
            unreadCount: 0,
            pinnedAt: 200,
            lastMessagePreview: nil
        )
        let firstPinned = Contact(
            id: "first-pinned",
            name: "First pinned",
            phone: nil,
            type: "private",
            lastMessageTime: 100,
            unreadCount: 0,
            pinnedAt: 100,
            lastMessagePreview: nil
        )

        XCTAssertEqual(
            AppSession.orderedContacts([recent, secondPinned, firstPinned]).map(\.id),
            ["first-pinned", "second-pinned", "recent"]
        )
        XCTAssertEqual(
            APIClient.pinPath(contactID: "family/parents@g.us"),
            "/api/contacts/family%2Fparents@g.us/pin"
        )
    }

    @MainActor
    func testCompactMessageLayoutHugsShortTextAndWrapsLongMessages() {
        func measured(_ text: String) -> CGSize {
            let host = UIHostingController(rootView: CompactMessageLayout {
                Text(text).font(.body)
                Text("16:06").font(.caption2).fixedSize()
            }.padding(9))
            return host.sizeThatFits(in: CGSize(width: 300, height: 1_000))
        }
        let short = measured("I do…")
        let long = measured(String(repeating: "A longer message that needs to wrap. ", count: 5))
        XCTAssertLessThan(short.width, 180)
        XCTAssertLessThan(short.height, 60)
        XCTAssertLessThanOrEqual(long.width, 300)
        XCTAssertGreaterThan(long.height, short.height)
    }

    func testChatFiltersDistinguishUnreadAndGroupConversations() {
        let direct = Contact(id: "direct", name: "Direct", phone: nil, type: "private", lastMessageTime: 1, unreadCount: 2, pinnedAt: nil, lastMessagePreview: nil)
        let group = Contact(id: "group", name: "Group", phone: nil, type: "group", lastMessageTime: 2, unreadCount: 0, pinnedAt: nil, lastMessagePreview: nil)
        XCTAssertTrue(ChatFilter.all.includes(direct))
        XCTAssertTrue(ChatFilter.all.includes(group))
        XCTAssertTrue(ChatFilter.unread.includes(direct))
        XCTAssertFalse(ChatFilter.unread.includes(group))
        XCTAssertFalse(ChatFilter.groups.includes(direct))
        XCTAssertTrue(ChatFilter.groups.includes(group))
    }

    func testWhatsAppStatusFeedBecomesDedicatedUpdatesToolbarItem() {
        let updates = Contact(
            id: "status@broadcast",
            name: nil,
            phone: nil,
            type: "broadcast",
            lastMessageTime: 50,
            unreadCount: 2,
            pinnedAt: nil,
            lastMessagePreview: "A status preview that must not be shown"
        )
        XCTAssertTrue(updates.isUpdates)
        XCTAssertEqual(updates.displayName, "Updates")
        XCTAssertTrue(updates.showsAsUpdatesToolbarItem)
        XCTAssertFalse(updates.showsInChatList)
    }

    func testMessageExtractsEveryLinkForMultiplePreviewCards() throws {
        let message = try decodeMessage(
            id: "links",
            contentType: "Text",
            content: #"{"type":"text","body":"Compare https://example.com/one and https://example.org/two"}"#
        )
        XCTAssertEqual(message.extractedURLs.map(\.absoluteString), [
            "https://example.com/one",
            "https://example.org/two",
        ])
    }

    func testMessageTextLinkifierMakesEveryDetectedURLClickable() throws {
        let text = "Watch https://youtube.com/shorts/example?is=abc and visit https://example.org/help."
        let attributedText = MessageTextLinkifier.attributedString(from: text)

        XCTAssertEqual(
            attributedText.runs.compactMap(\.link?.absoluteString),
            [
                "https://youtube.com/shorts/example?is=abc",
                "https://example.org/help",
            ]
        )
    }
}
