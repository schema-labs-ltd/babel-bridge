import AppKit
import SwiftUI
import XCTest
import UserNotifications
@testable import BabelBridgeMac

final class BabelBridgeMacTests: XCTestCase {

    func testMacNotificationCategorySupportsInlineReply() throws {
        let category = MessagingNotificationContract.category

        XCTAssertEqual(category.identifier, MessagingNotificationContract.categoryIdentifier)
        XCTAssertTrue(category.options.contains(.hiddenPreviewsShowTitle))
        XCTAssertTrue(category.options.contains(.hiddenPreviewsShowSubtitle))
        let reply = try XCTUnwrap(category.actions.first as? UNTextInputNotificationAction)
        XCTAssertEqual(reply.identifier, MessagingNotificationContract.replyActionIdentifier)
        XCTAssertEqual(reply.textInputButtonTitle, "Send")
    }

    func testMacDeclaresSendMessageUserActivity() {
        let activityTypes = Bundle.main.object(forInfoDictionaryKey: "NSUserActivityTypes") as? [String]

        XCTAssertTrue(activityTypes?.contains("INSendMessageIntent") == true)
    }

    func testCrossPlatformImageCanEncodeJPEGData() {
        let image = DemoImageFactory.landscape(size: CGSize(width: 320, height: 210))
        let data = image.platformJPEGData(compressionQuality: 0.8)

        XCTAssertNotNil(data)
        XCTAssertGreaterThan(data?.count ?? 0, 1_000)
    }

    func testMinimumMacWindowCanContainTheLargestMessageBubble() {
        let availableDetailWidth = MacChatLayoutMetrics.minimumWindowWidth
            - MacChatLayoutMetrics.minimumSidebarWidth
            - MacChatLayoutMetrics.timelineHorizontalPadding

        XCTAssertGreaterThanOrEqual(
            availableDetailWidth,
            MacChatLayoutMetrics.maximumBubbleWidth,
            "The minimum Mac window must not force message content outside its bubble."
        )
    }

    func testMacSheetsHaveEnoughRoomForTheirLabelsAndValues() {
        XCTAssertGreaterThanOrEqual(MacChatLayoutMetrics.settingsSheetMinimumWidth, 520)
        XCTAssertGreaterThanOrEqual(MacChatLayoutMetrics.costSheetMinimumWidth, 400)
        XCTAssertGreaterThanOrEqual(MacChatLayoutMetrics.mediaSheetMinimumWidth, 520)
    }

    func testMacMessageTextMakesDetectedURLsClickable() {
        let attributedText = MessageTextLinkifier.attributedString(
            from: "Watch https://youtube.com/shorts/example?is=abc now."
        )

        XCTAssertEqual(
            attributedText.runs.compactMap(\.link?.absoluteString),
            ["https://youtube.com/shorts/example?is=abc"]
        )
    }

    func testPhotoViewerZoomClampsAndDoubleClickTogglesMagnification() {
        XCTAssertEqual(PhotoViewerZoom.clampedScale(0.5), 1)
        XCTAssertEqual(PhotoViewerZoom.clampedScale(3), 3)
        XCTAssertEqual(PhotoViewerZoom.clampedScale(8), 5)
        XCTAssertEqual(PhotoViewerZoom.toggledScale(from: 1), 2.5)
        XCTAssertEqual(PhotoViewerZoom.toggledScale(from: 2.5), 1)
    }

    @MainActor
    func testComposerUsesAMultilineMacTextEditor() {
        let view = ComposerView(
            contactID: "test-contact",
            text: .constant("First line"),
            reply: nil,
            isSending: false,
            cancelReply: {},
            sendImages: { _, _ in true },
            send: {}
        )
        let hostingView = NSHostingView(rootView: view.environment(AppSession(demoMode: true)))
        hostingView.frame = NSRect(x: 0, y: 0, width: 640, height: 140)

        hostingView.layoutSubtreeIfNeeded()

        let textViews = hostingView.descendants(ofType: NSTextView.self)
        XCTAssertTrue(
            textViews.contains(where: { $0.isEditable && $0.isVerticallyResizable }),
            "The Mac composer must use an editable multiline text view so Return inserts a newline."
        )
    }

    @MainActor
    func testVideoMessageCanMountWithoutCrashing() {
        let message = ChatMessage(
            id: "video-message",
            contactId: "contact",
            timestamp: 0,
            isFromMe: false,
            isForwarded: false,
            senderName: "Contact",
            senderPhone: nil,
            contactName: "Contact",
            contactPhone: nil,
            chatType: "private",
            contentType: "video",
            content: MessageContent(
                type: "video",
                body: nil,
                showTranslatedPrimary: nil,
                replyContext: nil
            ),
            originalText: nil,
            translatedText: nil,
            sourceLanguage: nil,
            isTranslated: false
        )
        let view = RichMessageContentView(
            message: message,
            displayText: "Video",
            image: nil,
            mediaURL: URL(fileURLWithPath: "/tmp/babel-bridge-render-test.mp4"),
            isLoading: false,
            failed: false,
            retry: {}
        )
        let hostingView = NSHostingView(rootView: view.environment(AppSession(demoMode: true)))
        hostingView.frame = NSRect(x: 0, y: 0, width: 320, height: 240)

        hostingView.layoutSubtreeIfNeeded()
        hostingView.displayIfNeeded()

        XCTAssertEqual(hostingView.frame.size, NSSize(width: 320, height: 240))
    }

    @MainActor
    func testMacUsesTheSharedPinnedConversationOrdering() {
        let pinned = Contact(
            id: "pinned",
            name: "Pinned",
            phone: nil,
            type: "private",
            lastMessageTime: 1,
            unreadCount: 0,
            pinnedAt: 1,
            lastMessagePreview: nil
        )
        let recent = Contact(
            id: "recent",
            name: "Recent",
            phone: nil,
            type: "private",
            lastMessageTime: 2,
            unreadCount: 0,
            pinnedAt: nil,
            lastMessagePreview: nil
        )

        XCTAssertEqual(AppSession.orderedContacts([recent, pinned]).map(\.id), ["pinned", "recent"])
    }
}

private extension NSView {
    func descendants<T: NSView>(ofType type: T.Type) -> [T] {
        subviews.flatMap { view in
            (view as? T).map { [$0] } ?? view.descendants(ofType: type)
        }
    }
}

extension BabelBridgeMacTests {
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

extension BabelBridgeMacTests {
    @MainActor
    func testTranslatedImageCaptionRendersWithOriginalAlternative() throws {
        let json = #"{"id":"caption-fixture","contactId":"fixture@g.us","timestamp":1700000000000,"isFromMe":false,"isForwarded":false,"chatType":"group","contentType":"Image","content":{"type":"image","caption":"Jó reggelt Budapestről!","mime_type":"image/jpeg"},"originalText":"Jó reggelt Budapestről!","translatedText":"Good morning from Budapest!","sourceLanguage":"Hungarian","isTranslated":true}"#
        let message = try JSONDecoder().decode(ChatMessage.self, from: Data(json.utf8))
        XCTAssertEqual(message.displayText, "Good morning from Budapest!")
        let content = HStack(alignment: .top, spacing: 28) {
            VStack(alignment: .leading, spacing: 12) {
                Text("Translated caption").font(.headline)
                RichMessageContentView(message: message, displayText: message.displayText, image: nil, mediaURL: nil, isLoading: false, failed: false, retry: {})
            }
            VStack(alignment: .leading, spacing: 12) {
                Text("Show original").font(.headline)
                RichMessageContentView(message: message, displayText: message.originalText!, image: nil, mediaURL: nil, isLoading: false, failed: false, retry: {})
            }
        }.padding(24).frame(width: 640).background(.white).foregroundStyle(.black).environment(\.colorScheme, .light)
        let renderer = ImageRenderer(content: content)
        renderer.scale = 2
        let rendered = try XCTUnwrap(renderer.nsImage)
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: try XCTUnwrap(rendered.tiffRepresentation)))
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        let attachment = XCTAttachment(data: png, uniformTypeIdentifier: "public.png")
        attachment.name = "Translated caption and original"; attachment.lifetime = .keepAlways; add(attachment)
        try png.write(to: URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("babelbridge-caption-build33.png"))
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
