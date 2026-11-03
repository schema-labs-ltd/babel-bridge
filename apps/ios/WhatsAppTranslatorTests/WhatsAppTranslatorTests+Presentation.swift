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

    func testChatListOrganisationSurvivesRelaunchAndOlderPreferences() throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "chat-organisation-\(UUID().uuidString)"))
        // Preferences saved before mute and archive existed must still load.
        defaults.set(Data(#"{"starredMessageIDs":{},"conversations":{},"theme":"ocean","colorMode":"dark"}"#.utf8), forKey: "whatsapp-translator-ios-preferences-v1")
        let store = AppPreferencesStore(defaults: defaults)
        XCTAssertEqual(store.theme, .ocean)
        store.setMuted(true, for: "family@g.us")
        store.setArchived(true, for: "old@s.whatsapp.net")
        store.setMarkedUnread(true, for: "ana@s.whatsapp.net")

        let reloaded = AppPreferencesStore(defaults: defaults)
        XCTAssertTrue(reloaded.isMuted("family@g.us"))
        XCTAssertTrue(reloaded.isArchived("old@s.whatsapp.net"))
        XCTAssertTrue(reloaded.isMarkedUnread("ana@s.whatsapp.net"))
        XCTAssertEqual(defaults.stringArray(forKey: AppPreferencesStore.mutedChatsDefaultsKey), ["family@g.us"])
    }

    func testGroupSubtitleListsWritersThenYou() throws {
        let message = try decodeMessage(id: "m", contentType: "text", content: #"{"type":"text","body":"Hi"}"#)
        XCTAssertEqual(ConversationHeaderSummary.groupMembers(in: [message]), "Virag, You")
        XCTAssertEqual(ConversationHeaderSummary.groupMembers(in: []), "tap here for group info")
        // Two people who share a first name are both listed.
        func alex(_ phone: String) throws -> ChatMessage {
            try JSONDecoder().decode(ChatMessage.self, from: Data(#"{"id":"\#(phone)","contactId":"chat@g.us","timestamp":1700000000000,"isFromMe":false,"isForwarded":false,"senderName":"Alex \#(phone)","senderPhone":"\#(phone)","contactName":"Family","contactPhone":null,"chatType":"group","contentType":"text","content":{"type":"text","body":"Hi"},"originalText":null,"translatedText":null,"sourceLanguage":null,"isTranslated":false}"#.utf8))
        }
        XCTAssertEqual(ConversationHeaderSummary.groupMembers(in: [try alex("1"), try alex("2")]), "Alex, Alex, You")
    }

    func testDayHeadersNameTheDayLikeWhatsApp() throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = try XCTUnwrap(TimeZone(identifier: "Europe/London"))
        let now = try XCTUnwrap(calendar.date(from: DateComponents(year: 2026, month: 11, day: 3, hour: 20)))
        func daysAgo(_ days: Int) -> Date { calendar.date(byAdding: .day, value: -days, to: now)! }
        XCTAssertEqual(TimelineGrouping.dayLabel(for: daysAgo(0), now: now, calendar: calendar), "Today")
        XCTAssertEqual(TimelineGrouping.dayLabel(for: daysAgo(1), now: now, calendar: calendar), "Yesterday")
        XCTAssertEqual(TimelineGrouping.dayLabel(for: daysAgo(3), now: now, calendar: calendar), daysAgo(3).formatted(.dateTime.weekday(.wide)))
        XCTAssertNotEqual(TimelineGrouping.dayLabel(for: daysAgo(400), now: now, calendar: calendar), daysAgo(400).formatted(.dateTime.weekday(.wide)))
    }

    func testConsecutiveMessagesFromOneSenderFormARun() throws {
        let first = try decodeMessage(id: "a", contentType: "text", content: #"{"type":"text","body":"One"}"#)
        let second = try decodeMessage(id: "b", contentType: "text", content: #"{"type":"text","body":"Two"}"#)
        let items: [ConversationTimelineItem] = [.message(first), .message(second)]
        XCTAssertTrue(TimelineGrouping.startsRun(items, at: 0))
        XCTAssertFalse(TimelineGrouping.startsRun(items, at: 1))
        XCTAssertEqual(TimelineGrouping.days(items).map(\.indices), [0..<2])

        // Two group messages with only a shared name may be two people, so each starts a run.
        func anonymous(_ id: String) throws -> ChatMessage {
            try JSONDecoder().decode(ChatMessage.self, from: Data(#"{"id":"\#(id)","contactId":"chat@g.us","timestamp":1700000000000,"isFromMe":false,"isForwarded":false,"senderName":"Alex","senderPhone":null,"contactName":"Family","contactPhone":null,"chatType":"group","contentType":"text","content":{"type":"text","body":"Hi"},"originalText":null,"translatedText":null,"sourceLanguage":null,"isTranslated":false}"#.utf8))
        }
        let alexes: [ConversationTimelineItem] = [.message(try anonymous("x")), .message(try anonymous("y"))]
        XCTAssertTrue(TimelineGrouping.startsRun(alexes, at: 1))
        // Pinned djb2 results: a per-launch hash such as hashValue would fail these.
        XCTAssertEqual(SenderNameColor.index(for: "3630"), 5)
        XCTAssertEqual(SenderNameColor.index(for: "Virag"), 4)
    }

    func testLiveArrivalsReportOnlyWhatCameSinceTheLastLook() {
        var arrivals = LiveArrivals()
        arrivals.record("a")
        let seen = arrivals.count
        arrivals.record("b")
        arrivals.record("b")
        arrivals.record("c")
        XCTAssertEqual(arrivals.ids(since: seen), ["b", "c"])
        XCTAssertEqual(arrivals.ids(since: arrivals.count), [])
        for index in 0..<60 { arrivals.record("m\(index)") }
        XCTAssertEqual(arrivals.ids(since: 0).count, 50)
    }

    func testUnreadBarSitsAboveTheFirstUnreadIncomingMessage() throws {
        let messages = try ["a", "b", "c"].map { try decodeMessage(id: $0, contentType: "text", content: #"{"type":"text","body":"Hi"}"#) }
        XCTAssertEqual(UnreadMarker.firstUnreadID(in: messages, unreadCount: 2), "b")
        XCTAssertEqual(UnreadMarker.firstUnreadID(in: messages, unreadCount: 9), "a")
        XCTAssertNil(UnreadMarker.firstUnreadID(in: messages, unreadCount: 0))
        // A message already viewed here, or a deleted one, can't be one of the unread.
        XCTAssertEqual(UnreadMarker.firstUnreadID(in: messages, unreadCount: 2, viewed: ["c"]), "a")
        let deleted = try decodeMessage(id: "d", contentType: "revoked", content: #"{"type":"revoked"}"#)
        XCTAssertEqual(UnreadMarker.firstUnreadID(in: messages + [deleted], unreadCount: 1), "c")
    }

    func testChatListPreviewTidiesServerLabelsLikeWhatsApp() {
        XCTAssertEqual(ChatListPreview.tidy("[ Image ] Beach day"), "📷 Beach day")
        XCTAssertEqual(ChatListPreview.tidy("You: [ Image ]"), "You: 📷 Photo")
        XCTAssertEqual(ChatListPreview.tidy("[ Voice Note ]"), "🎤 Voice message")
        XCTAssertEqual(ChatListPreview.tidy("[ Document: menu.pdf ]"), "📄 menu.pdf")
        XCTAssertEqual(ChatListPreview.tidy("[ Poll: Dinner? ]"), "📊 Dinner?")
        XCTAssertEqual(ChatListPreview.tidy("[ Message deleted ]"), "🚫 This message was deleted")
        XCTAssertEqual(ChatListPreview.tidy("Line one\nline two"), "Line one line two")
        XCTAssertEqual(ChatListPreview.tidy("Plain [ text ]"), "Plain [ text ]")
    }

    func testChatListPreviewNamesGroupSendersAndShowsTicksForOwnMessages() throws {
        let group = Contact(id: "chat@g.us", name: "Family", phone: nil, type: "group", lastMessageTime: 1_699_999_999_000,
                            unreadCount: 0, pinnedAt: nil, lastMessagePreview: "Szia")
        let incoming = try decodeMessage(id: "m1", contentType: "text", content: #"{"type":"text","body":"Szia"}"#)
        XCTAssertEqual(ChatListPreview.line(for: group, latest: incoming), .init(text: "Virag: Szia"))

        let photo = try decodeMessage(id: "m2", contentType: "image", content: #"{"type":"image"}"#)
        XCTAssertEqual(ChatListPreview.line(for: group, latest: photo).text, "Virag: 📷 Photo")

        var own = try JSONDecoder().decode(ChatMessage.self, from: Data(#"{"id":"m3","contactId":"chat@g.us","timestamp":1700000000000,"isFromMe":true,"isForwarded":false,"senderName":null,"senderPhone":null,"contactName":"Family","contactPhone":null,"chatType":"group","contentType":"text","content":{"type":"text","body":"On my way"},"originalText":null,"translatedText":null,"sourceLanguage":null,"isTranslated":false}"#.utf8))
        own.deliveryStatus = "read"
        XCTAssertEqual(ChatListPreview.line(for: group, latest: own), .init(text: "On my way", delivery: .read))

        // An older loaded message never replaces a newer server preview.
        let newer = Contact(id: "chat@g.us", name: "Family", phone: nil, type: "group", lastMessageTime: 1_700_000_100_000,
                            unreadCount: 1, pinnedAt: nil, lastMessagePreview: "[ Sticker ]")
        XCTAssertEqual(ChatListPreview.line(for: newer, latest: incoming), .init(text: "Sticker"))

        // In the same second, the loaded message is used only when it is the one the server summarised.
        let sameSecond = Contact(id: "chat@g.us", name: "Family", phone: nil, type: "group", lastMessageTime: 1_700_000_000_000,
                                 unreadCount: 1, pinnedAt: nil, lastMessagePreview: "Virag: Szia")
        XCTAssertEqual(ChatListPreview.line(for: sameSecond, latest: incoming), .init(text: "Virag: Szia"))
        let otherSameSecond = Contact(id: "chat@g.us", name: "Family", phone: nil, type: "group", lastMessageTime: 1_700_000_000_000,
                                      unreadCount: 1, pinnedAt: nil, lastMessagePreview: "Bence: Hello")
        XCTAssertEqual(ChatListPreview.line(for: otherSameSecond, latest: incoming), .init(text: "Bence: Hello"))
        // A server preview cut with "…", or a live update's display text, still matches its message.
        let long = try decodeMessage(id: "m4", contentType: "text", content: #"{"type":"text","body":"See you at the station tonight"}"#)
        let cut = Contact(id: "chat@g.us", name: "Family", phone: nil, type: "group", lastMessageTime: 1_700_000_000_000,
                          unreadCount: 1, pinnedAt: nil, lastMessagePreview: "Virag: See you at the…")
        XCTAssertEqual(ChatListPreview.line(for: cut, latest: long).text, "Virag: See you at the station tonight")
        let live = Contact(id: "chat@g.us", name: "Family", phone: nil, type: "group", lastMessageTime: 1_700_000_000_000,
                           unreadCount: 1, pinnedAt: nil, lastMessagePreview: photo.displayText)
        XCTAssertEqual(ChatListPreview.line(for: live, latest: photo).text, "Virag: 📷 Photo")
        // Matching is on the whole message, not its ending.
        let ok = try decodeMessage(id: "m5", contentType: "text", content: #"{"type":"text","body":"OK"}"#)
        let notOK = Contact(id: "chat@g.us", name: "Family", phone: nil, type: "group", lastMessageTime: 1_700_000_000_000,
                            unreadCount: 1, pinnedAt: nil, lastMessagePreview: "Bence: No, not OK")
        XCTAssertEqual(ChatListPreview.line(for: notOK, latest: ok).text, "Bence: No, not OK")
    }
}
