import XCTest
import Intents
import SwiftUI
import UserNotifications
@testable import WhatsAppTranslator

@MainActor
final class LiveReactionTests: XCTestCase {
    private func message(_ id: String = "target", contact: String = "family@g.us", emoji: String? = nil, time: Int64 = 100, actor: String = "447700900123") throws -> ChatMessage {
        var json: [String: Any] = ["id": id, "contactId": contact, "timestamp": time, "isFromMe": false, "isForwarded": false, "senderName": "Alex", "senderPhone": actor, "chatType": "group", "contentType": "Text", "content": ["type": "text", "body": "See you at six"], "isTranslated": false]
        if let emoji {
            json["contentType"] = "Reaction"
            json["content"] = ["type": "reaction", "target_message_id": "target", "emoji": emoji]
        }
        return try JSONDecoder().decode(ChatMessage.self, from: JSONSerialization.data(withJSONObject: json))
    }

    private func event(_ message: ChatMessage, type: String = "reaction") throws -> LiveEvent {
        let payload = try JSONSerialization.jsonObject(with: JSONEncoder().encode(message))
        return try JSONDecoder().decode(LiveEvent.self, from: JSONSerialization.data(withJSONObject: ["type": type, "message": payload]))
    }

    func testLiveReactionUpdatesFeedWhenConversationDoesNotContainTarget() throws {
        let session = AppSession(demoMode: false)
        session.phase = .ready
        let target = try message()
        session.feedByID[target.id] = target
        session.handle(try event(message("heart", emoji: "❤️", time: 200)))
        XCTAssertEqual(session.unifiedMessages.first?.reactions, ["❤️": ["447700900123"]])
        XCTAssertEqual(session.feedByID[target.id]?.reactions, ["❤️": ["447700900123"]])
        XCTAssertEqual(session.unifiedMessages.map(\.id), [target.id])
    }

    func testTranslationUpdateCannotEraseLiveReaction() throws {
        let session = AppSession(demoMode: false)
        session.phase = .ready
        let target = try message()
        session.messages[target.contactId] = [target]
        session.feedByID[target.id] = target
        session.handle(try event(message("heart", emoji: "❤️", time: 200)))
        session.handle(try event(target, type: "message_updated"))
        XCTAssertEqual(session.messages[target.contactId]?.first?.reactions, ["❤️": ["447700900123"]])
        XCTAssertEqual(session.unifiedMessages.first?.reactions, ["❤️": ["447700900123"]])
    }

    func testReactionsBeforeTargetRetainLatestActorChoiceAndRemoval() throws {
        let session = AppSession(demoMode: false)
        let heart = try message("heart", emoji: "❤️", time: 200)
        let thumb = try message("thumb", emoji: "👍", time: 300)
        XCTAssertTrue(session.normalizeMessages([thumb, heart]).isEmpty)
        let target = try message()
        var normalized = session.normalizeMessages([target])
        XCTAssertEqual(normalized.first?.reactions, ["👍": ["447700900123"]])
        normalized = session.normalizeMessages(normalized + [heart, try message("other", emoji: "❤️", time: 250, actor: "447700900456")])
        XCTAssertEqual(normalized.first?.reactions, ["👍": ["447700900123"], "❤️": ["447700900456"]])
        normalized = session.normalizeMessages(normalized + [try message("remove", emoji: "", time: 400)])
        normalized = session.normalizeMessages(normalized + [thumb])
        XCTAssertEqual(normalized.first?.reactions, ["❤️": ["447700900456"]])
        XCTAssertNil(session.normalizeMessages([try message(contact: "other@g.us")]).first?.reactions)
    }

    func testInFlightFeedAndConversationSnapshotsKeepLiveReactions() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ReactionSnapshotProtocol.self]
        let urlSession = URLSession(configuration: configuration)
        defer { urlSession.invalidateAndCancel(); ReactionSnapshotProtocol.state.handler = nil }
        let api = APIClient(session: urlSession)
        await api.configure(try ServerConfiguration.make(address: "https://reactions.example.test", password: "test"))
        let session = AppSession(api: api, demoMode: false)
        session.phase = .ready
        let target = try message()
        session.messages[target.contactId] = [target]
        session.feedByID[target.id] = target
        let payload = try JSONSerialization.jsonObject(with: JSONEncoder().encode(target))
        let snapshot = try JSONSerialization.data(withJSONObject: ["messages": [payload], "hasMore": true])
        for operation in 0..<3 {
            let started = expectation(description: "Snapshot request started")
            let pending = ReactionRequestState()
            ReactionSnapshotProtocol.state.handler = { request in
                if request.request.url?.path == "/api/topics/messages" {
                    request.respond(Data(#"{"topics":[]}"#.utf8))
                } else if request.request.url?.path.hasSuffix("/read") == true {
                    request.respond(Data(#"{"success":true}"#.utf8))
                } else {
                    pending.request = request
                    started.fulfill()
                }
            }
            let request = Task {
                if operation == 0 { await session.loadFeed() }
                else { await session.loadMessages(for: target.contactId, older: operation == 2) }
            }
            let waitResult = await XCTWaiter.fulfillment(of: [started], timeout: 3)
            XCTAssertEqual(waitResult, .completed)
            let emoji = operation == 0 ? "❤️" : operation == 1 ? "👍" : ""
            session.handle(try event(message("live-\(operation)", emoji: emoji, time: Int64(200 + operation))))
            try XCTUnwrap(pending.request).respond(snapshot)
            await request.value
            let expected: [String: [String]] = emoji.isEmpty ? [:] : [emoji: ["447700900123"]]
            XCTAssertEqual(session.messages[target.contactId]?.first?.reactions, expected)
            XCTAssertEqual(session.unifiedMessages.first?.reactions, expected)
        }
    }
}

private final class ReactionSnapshotProtocol: URLProtocol, @unchecked Sendable {
    static let state = ReactionRequestState()
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "reactions.example.test" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() { Self.state.handler?(self) }
    override func stopLoading() {}
    func respond(_ data: Data) {
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }
}

private final class ReactionRequestState: @unchecked Sendable {
    private let lock = NSLock()
    private var storedHandler: (@Sendable (ReactionSnapshotProtocol) -> Void)?
    private var storedRequest: ReactionSnapshotProtocol?
    var handler: (@Sendable (ReactionSnapshotProtocol) -> Void)? {
        get { lock.withLock { storedHandler } }
        set { lock.withLock { storedHandler = newValue } }
    }
    var request: ReactionSnapshotProtocol? {
        get { lock.withLock { storedRequest } }
        set { lock.withLock { storedRequest = newValue } }
    }
}

@MainActor
final class LiveConnectionIndicatorTests: XCTestCase {
    func testQuickReconnectNeverShowsConnecting() async throws {
        let session = AppSession(demoMode: false)
        session.phase = .ready
        session.connectingIndicatorDelay = .milliseconds(200)
        session.handle(.signal("live_reconnecting"))
        XCTAssertTrue(session.liveUpdatesReconnecting)
        XCTAssertFalse(session.showsConnectingIndicator)
        session.clearConnectingIndicator()
        try await Task.sleep(for: .milliseconds(400))
        XCTAssertFalse(session.showsConnectingIndicator)
    }

    func testSlowReconnectShowsConnectingUntilRestored() async throws {
        let session = AppSession(demoMode: false)
        session.phase = .ready
        session.connectingIndicatorDelay = .milliseconds(50)
        session.handle(.signal("live_reconnecting"))
        session.handle(.signal("live_reconnecting"))
        for _ in 0..<40 where !session.showsConnectingIndicator {
            try await Task.sleep(for: .milliseconds(50))
        }
        XCTAssertTrue(session.showsConnectingIndicator)
        session.clearConnectingIndicator()
        XCTAssertFalse(session.showsConnectingIndicator)
        XCTAssertFalse(session.liveUpdatesReconnecting)
    }

    func testFirstLiveRetryIsImmediateThenBacksOff() {
        XCTAssertLessThan(APIClient.reconnectDelay(attempt: 1), 1)
        XCTAssertEqual((2...7).map { APIClient.reconnectDelay(attempt: $0) }, [2, 4, 8, 16, 30, 30])
    }
}

@MainActor
final class MarkChatReadTests: XCTestCase {
    private func message(_ id: String, time: Int64) throws -> ChatMessage {
        let json: [String: Any] = ["id": id, "contactId": "ana@s.whatsapp.net", "timestamp": time, "isFromMe": false, "isForwarded": false, "senderName": "Ana", "senderPhone": "447700900123", "chatType": "individual", "contentType": "Text", "content": ["type": "text", "body": "Hi"], "isTranslated": false]
        return try JSONDecoder().decode(ChatMessage.self, from: JSONSerialization.data(withJSONObject: json))
    }

    private func page(_ messages: [ChatMessage], hasMore: Bool) throws -> Data {
        let payload = try messages.map { try JSONSerialization.jsonObject(with: JSONEncoder().encode($0)) }
        return try JSONSerialization.data(withJSONObject: ["messages": payload, "hasMore": hasMore])
    }

    /// Two pages cover the unread count, the first ending in a shared second. A live message from that
    /// same second, known only to the cache, is not part of the count and gets no receipt.
    func testMarkChatReadReceiptsTheFetchedUnreadMessagesOnly() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MarkReadProtocol.self]
        let urlSession = URLSession(configuration: configuration)
        defer { urlSession.invalidateAndCancel(); MarkReadProtocol.state.handler = nil }
        let api = APIClient(session: urlSession)
        await api.configure(try ServerConfiguration.make(address: "https://markread.example.test", password: "test"))
        let session = AppSession(api: api, demoMode: false)
        let first = try page([message("m2", time: 200_000), message("m3", time: 200_000)], hasMore: true)
        let second = try page([message("m1", time: 100_000)], hasMore: false)
        MarkReadProtocol.state.handler = { request in
            let query = request.request.url?.query ?? ""
            if request.request.url?.path == "/api/topics/messages" {
                request.respond(Data(#"{"topics":[]}"#.utf8))
            } else {
                request.respond(query.contains("before=") ? second : first)
            }
        }
        session.messages["ana@s.whatsapp.net"] = [try message("live", time: 200_000)]
        session.viewedReceiptMessageIDs.insert("m0")
        let contact = Contact(id: "ana@s.whatsapp.net", name: "Ana", phone: nil, type: "individual", lastMessageTime: 200_000,
                              unreadCount: 3, pinnedAt: nil, lastMessagePreview: "Hi")
        await session.markChatRead(contact)
        XCTAssertEqual(session.pendingReadReceipts["ana@s.whatsapp.net"], ["m1", "m2", "m3"])
        XCTAssertEqual(Set(session.messages["ana@s.whatsapp.net"]?.map(\.id) ?? []), ["live", "m1", "m2", "m3"])
    }
}

private final class MarkReadProtocol: URLProtocol, @unchecked Sendable {
    static let state = MarkReadState()
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "markread.example.test" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() { Self.state.handler?(self) }
    override func stopLoading() {}
    func respond(_ data: Data) {
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }
}

private final class MarkReadState: @unchecked Sendable {
    private let lock = NSLock()
    private var storedHandler: (@Sendable (MarkReadProtocol) -> Void)?
    var handler: (@Sendable (MarkReadProtocol) -> Void)? {
        get { lock.withLock { storedHandler } }
        set { lock.withLock { storedHandler = newValue } }
    }
}
