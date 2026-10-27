import XCTest
#if os(macOS)
@testable import BabelBridgeMac
#else
@testable import WhatsAppTranslator
#endif

@MainActor
final class WhatsAppConnectionTests: XCTestCase {
    private func event(_ json: String) throws -> LiveEvent {
        try JSONDecoder.backend.decode(LiveEvent.self, from: Data(json.utf8))
    }

    func testSessionRemovalOpensLinkingAndReconnectPreservesChats() async throws {
        let session = AppSession(demoMode: true)
        await session.start()
        let contacts = session.contacts
        let messages = session.messages
        session.configuration = try ServerConfiguration.make(address: "https://example.com", password: "test-only")
        session.handle(try event(#"{"type":"status","connected":false,"connection_state":"linking_required","qr":null}"#))
        XCTAssertTrue(session.requiresWhatsAppLink)
        XCTAssertNil(session.whatsAppQRCode)
        session.handle(try event(#"{"type":"qr","data":"test-link-code"}"#))
        XCTAssertEqual(session.whatsAppQRCode, "test-link-code")
        session.handle(try event(#"{"type":"connected","name":"Preview","phone":"447700900123"}"#))
        XCTAssertFalse(session.requiresWhatsAppLink)
        XCTAssertTrue(session.backendStatus.connected)
        XCTAssertNil(session.whatsAppQRCode)
        XCTAssertEqual(session.backendStatus.name, "Preview")
        XCTAssertEqual(session.contacts, contacts)
        XCTAssertEqual(session.messages, messages)
        XCTAssertEqual(session.configuration?.password, "test-only")
    }

    func testTemporaryDisconnectKeepsInboxAndDoesNotRequestNewLink() async throws {
        let session = AppSession(demoMode: true)
        await session.start()
        session.handle(try event(#"{"type":"disconnected"}"#))
        XCTAssertFalse(session.backendStatus.connected)
        XCTAssertFalse(session.requiresWhatsAppLink)
        XCTAssertEqual(session.phase, .ready)
        XCTAssertFalse(session.contacts.isEmpty)
    }

    func testFreshStatusRecoversMissedLinkEventAndRemovesExpiredCode() throws {
        let session = AppSession(demoMode: true)
        session.phase = .ready
        let status = try JSONDecoder.backend.decode(BackendStatus.self, from: Data(#"{"connected":false,"connection_state":"linking_required","qr":"fresh-code"}"#.utf8))
        session.applyBackendStatus(status)
        XCTAssertTrue(session.requiresWhatsAppLink)
        XCTAssertEqual(session.whatsAppQRCode, "fresh-code")
        session.handle(try event(#"{"type":"status","connected":false,"connection_state":"linking_required","qr":null}"#))
        XCTAssertTrue(session.requiresWhatsAppLink)
        XCTAssertNil(session.whatsAppQRCode)
        session.handle(try event(#"{"type":"status","connected":true,"connection_state":"connected","qr":null}"#))
        XCTAssertFalse(session.requiresWhatsAppLink)
    }

    func testLegacyStatusAndQRRemainCompatible() throws {
        let session = AppSession(demoMode: true)
        session.phase = .ready
        let status = try JSONDecoder.backend.decode(BackendStatus.self, from: Data(#"{"connected":false,"phone":null,"name":null}"#.utf8))
        session.applyBackendStatus(status)
        XCTAssertFalse(session.requiresWhatsAppLink)
        session.handle(try event(#"{"type":"qr","data":"legacy-code"}"#))
        XCTAssertTrue(session.requiresWhatsAppLink)
        session.handle(try event(#"{"type":"status","connected":true}"#))
        XCTAssertFalse(session.requiresWhatsAppLink)
        XCTAssertNil(session.whatsAppQRCode)
    }

    private func flakySession(failures: Int, code: URLError.Code = .networkConnectionLost) async throws -> (AppSession, URLSession) {
        FlakyRefreshProtocol.state.reset(failures: failures, code: code)
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [FlakyRefreshProtocol.self]
        let urlSession = URLSession(configuration: configuration)
        let api = APIClient(session: urlSession)
        await api.configure(try ServerConfiguration.make(address: "https://flaky-refresh.example.test", password: "test"))
        let session = AppSession(api: api, demoMode: false)
        session.phase = .ready
        session.transientRefreshRetryDelays = [.zero, .zero]
        return (session, urlSession)
    }

    func testRefreshAfterUnlockRetriesDroppedConnectionQuietly() async throws {
        let (session, urlSession) = try await flakySession(failures: 2)
        defer { urlSession.invalidateAndCancel() }
        await session.refresh()
        XCTAssertNil(session.errorMessage)
        XCTAssertEqual(session.contacts.map(\.id), ["family@g.us"])
    }

    func testAutomaticRefreshDoesNotAlertWhileConnectionStaysDown() async throws {
        let (session, urlSession) = try await flakySession(failures: .max)
        defer { urlSession.invalidateAndCancel() }
        await session.refresh()
        XCTAssertNil(session.errorMessage)
        XCTAssertFalse(session.isRefreshing)
    }

    func testPullToRefreshReportsConnectionThatStaysDown() async throws {
        let (session, urlSession) = try await flakySession(failures: .max)
        defer { urlSession.invalidateAndCancel() }
        await session.refresh(reportTransientFailures: true)
        XCTAssertEqual(session.errorTitle, "Couldn’t refresh chats")
        XCTAssertNotNil(session.errorMessage)
    }

    func testTransientNetworkErrorClassification() {
        XCTAssertTrue(AppSession.isTransientNetworkError(URLError(.networkConnectionLost)))
        XCTAssertTrue(AppSession.isTransientNetworkError(URLError(.notConnectedToInternet)))
        let wrapped = NSError(domain: "Wrapper", code: 1, userInfo: [NSUnderlyingErrorKey: URLError(.timedOut)])
        XCTAssertTrue(AppSession.isTransientNetworkError(wrapped))
        XCTAssertFalse(AppSession.isTransientNetworkError(URLError(.badServerResponse)))
        XCTAssertFalse(AppSession.isTransientNetworkError(APIError.server("Nope")))
    }
}

private final class FlakyRefreshProtocol: URLProtocol, @unchecked Sendable {
    static let state = FlakyRefreshState()
    override class func canInit(with request: URLRequest) -> Bool { request.url?.host == "flaky-refresh.example.test" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        guard let data = Self.state.respond(to: request) else {
            client?.urlProtocol(self, didFailWithError: URLError(Self.state.code))
            return
        }
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

private final class FlakyRefreshState: @unchecked Sendable {
    private let lock = NSLock()
    private var remainingFailures = 0
    private var failureCode = URLError.Code.networkConnectionLost
    var code: URLError.Code { lock.withLock { failureCode } }
    func reset(failures: Int, code: URLError.Code) { lock.withLock { remainingFailures = failures; failureCode = code } }
    /// Fails status requests while failures remain, standing in for a connection dropped by the lock screen.
    func respond(to request: URLRequest) -> Data? {
        lock.withLock {
            switch request.url?.path {
            case "/api/status":
                guard remainingFailures == 0 else { remainingFailures -= 1; return nil }
                return Data(#"{"connected":true}"#.utf8)
            case "/api/contacts":
                return Data(#"[{"id":"family@g.us","name":"Family","type":"group","lastMessageTime":2,"unreadCount":0}]"#.utf8)
            default:
                return Data("{}".utf8)
            }
        }
    }
}
