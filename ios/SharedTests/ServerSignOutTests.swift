import XCTest
#if os(macOS)
@testable import BabelBridgeMac
#else
@testable import WhatsAppTranslator
#endif

@MainActor
final class ServerSignOutTests: XCTestCase {
    func testRemovalPathDeletesOnlyThisInstallation() {
        XCTAssertEqual(
            APIClient.pushDeviceDeletionPath(installationID: "install/1"),
            "/api/push/devices/install%2F1"
        )
        XCTAssertNil(APIClient.pushDeviceDeletionPath(installationID: " \n "))
        XCTAssertFalse(APIClient.pushDeviceDeletionPath(installationID: "install/1")?.contains("logout") == true)
    }

    func testForgettingAnOnlineServerRemovesThePushRegistration() async throws {
        let harness = try Harness()
        defer { harness.cleanup() }
        let session = try await harness.signedInSession()
        let installationID = PushNotificationCoordinator.shared.currentInstallationID
        let before = ServerSignOutURLProtocol.state.requests.count

        await session.forgetServer()

        let signOut = Array(ServerSignOutURLProtocol.state.requests.dropFirst(before))
        XCTAssertEqual(signOut.map(\.method), ["DELETE"])
        XCTAssertEqual(signOut.map(\.path), ["/api/push/devices/\(installationID)"])
        XCTAssertEqual(signOut.map(\.authorization), ["Bearer session-token"])
        XCTAssertNoLogout(signOut)
        await harness.assertForgotten(session)
        let callsAfterForget = ServerSignOutURLProtocol.state.requests.count
        await session.registerPushDevice(token: "fixture-device-token", installationID: installationID, environment: "sandbox")
        XCTAssertEqual(ServerSignOutURLProtocol.state.requests.count, callsAfterForget)
    }

    func testForgettingStillClearsTheDeviceWhenTheServerIsUnreachable() async throws {
        let harness = try Harness()
        defer { harness.cleanup() }
        let session = try await harness.signedInSession()
        ServerSignOutURLProtocol.state.mode = .offline
        let before = ServerSignOutURLProtocol.state.requests.count

        await session.forgetServer()

        let signOut = Array(ServerSignOutURLProtocol.state.requests.dropFirst(before))
        XCTAssertEqual(signOut.map(\.method), ["DELETE"])
        XCTAssertEqual(signOut.map(\.authorization), ["Bearer session-token"])
        XCTAssertNoLogout(signOut)
        await harness.assertForgotten(session)
    }

    func testRejectedPushRemovalStillClearsTheDevice() async throws {
        let harness = try Harness()
        defer { harness.cleanup() }
        let session = try await harness.signedInSession()
        ServerSignOutURLProtocol.state.mode = .deleteRejected
        let before = ServerSignOutURLProtocol.state.requests.count

        await session.forgetServer()

        let signOut = Array(ServerSignOutURLProtocol.state.requests.dropFirst(before))
        XCTAssertEqual(signOut.map(\.method), ["DELETE"])
        XCTAssertEqual(signOut.map(\.path), [
            "/api/push/devices/\(PushNotificationCoordinator.shared.currentInstallationID)",
        ])
        XCTAssertNoLogout(signOut)
        await harness.assertForgotten(session)
    }

    func testUnauthorizedPushRemovalDoesNotSignInAgain() async throws {
        let harness = try Harness()
        defer { harness.cleanup() }
        let session = try await harness.signedInSession()
        ServerSignOutURLProtocol.state.mode = .unauthorized
        let before = ServerSignOutURLProtocol.state.requests.count

        await session.forgetServer()

        let signOut = Array(ServerSignOutURLProtocol.state.requests.dropFirst(before))
        XCTAssertEqual(signOut.map(\.path), [
            "/api/push/devices/\(PushNotificationCoordinator.shared.currentInstallationID)",
        ])
        XCTAssertFalse(signOut.contains { $0.path == "/api/auth" || $0.path == "/api/auth/check" })
        XCTAssertNoLogout(signOut)
        await harness.assertForgotten(session)
    }

    func testDisconnectDoesNotForgetTheServerOrSignOut() async throws {
        let harness = try Harness()
        defer { harness.cleanup() }
        let session = try await harness.signedInSession()
        let before = ServerSignOutURLProtocol.state.requests.count

        session.handle(LiveEvent.signal("disconnected"))

        XCTAssertEqual(session.phase, .ready)
        XCTAssertEqual(session.configuration, harness.configuration)
        XCTAssertEqual(harness.credentials.load(), harness.configuration)
        XCTAssertEqual(harness.credentials.clearCount, 0)
        XCTAssertFalse(session.contacts.isEmpty)
        let later = Array(ServerSignOutURLProtocol.state.requests.dropFirst(before))
        XCTAssertFalse(later.contains { $0.method == "DELETE" })
        XCTAssertNoLogout(later)
    }
}

private func XCTAssertNoLogout(
    _ requests: [ServerSignOutURLProtocol.Exchange],
    file: StaticString = #filePath,
    line: UInt = #line
) {
    XCTAssertFalse(requests.contains { $0.path == "/api/logout" }, "forgetting a server must not call /api/logout", file: file, line: line)
}

@MainActor
private struct Harness {
    let configuration: ServerConfiguration
    let credentials: MemoryCredentialStore
    let api: APIClient
    let urlSession: URLSession

    init() throws {
        configuration = try ServerConfiguration.make(address: "https://sign-out.test", password: "fixture")
        credentials = MemoryCredentialStore()
        let sessionConfiguration = URLSessionConfiguration.ephemeral
        sessionConfiguration.protocolClasses = [ServerSignOutURLProtocol.self]
        urlSession = URLSession(configuration: sessionConfiguration)
        api = APIClient(session: urlSession)
        ServerSignOutURLProtocol.state.reset()
    }

    func signedInSession() async throws -> AppSession {
        await api.configure(configuration)
        _ = try await api.authenticate()
        try credentials.save(configuration)
        let session = AppSession(api: api, credentials: credentials, demoMode: false)
        session.configuration = configuration
        session.phase = .ready
        session.contacts = [
            Contact(
                id: "chat-1",
                name: nil,
                phone: nil,
                type: nil,
                lastMessageTime: 1,
                unreadCount: 1,
                pinnedAt: nil,
                lastMessagePreview: nil
            ),
        ]
        session.selectedContactID = "chat-1"
        return session
    }

    func assertForgotten(_ session: AppSession, file: StaticString = #filePath, line: UInt = #line) async {
        XCTAssertEqual(session.phase, .needsConfiguration, file: file, line: line)
        XCTAssertNil(session.configuration, file: file, line: line)
        XCTAssertNil(session.selectedContactID, file: file, line: line)
        XCTAssertTrue(session.contacts.isEmpty, file: file, line: line)
        XCTAssertNil(credentials.load(), file: file, line: line)
        XCTAssertEqual(credentials.clearCount, 1, file: file, line: line)
        let before = ServerSignOutURLProtocol.state.requests.count
        do {
            _ = try await api.status()
            XCTFail("A forgotten server must not keep an authenticated client", file: file, line: line)
        } catch let error as APIError {
            XCTAssertEqual(error, .notConfigured, file: file, line: line)
        } catch {
            XCTFail("Unexpected error: \(error)", file: file, line: line)
        }
        XCTAssertEqual(ServerSignOutURLProtocol.state.requests.count, before, file: file, line: line)
    }

    func cleanup() {
        urlSession.invalidateAndCancel()
        ServerSignOutURLProtocol.state.reset()
    }
}

private final class MemoryCredentialStore: ServerCredentialStoring, @unchecked Sendable {
    private let lock = NSLock()
    private var stored: ServerConfiguration?
    private var clears = 0

    func load() -> ServerConfiguration? { lock.withLock { stored } }
    func save(_ configuration: ServerConfiguration) throws { lock.withLock { stored = configuration } }
    func clear() { lock.withLock { stored = nil; clears += 1 } }
    var clearCount: Int { lock.withLock { clears } }
}

private final class ServerSignOutURLProtocol: URLProtocol, @unchecked Sendable {
    enum Mode: Equatable {
        case success
        case offline
        case unauthorized
        case deleteRejected
    }

    struct Exchange: Equatable {
        var method: String
        var path: String
        var authorization: String?
    }

    final class State: @unchecked Sendable {
        private let lock = NSLock()
        private var recorded: [Exchange] = []
        private var failure = Mode.success

        var mode: Mode {
            get { lock.withLock { failure } }
            set { lock.withLock { failure = newValue } }
        }
        var requests: [Exchange] { lock.withLock { recorded } }

        func reset() { lock.withLock { recorded = []; failure = .success } }

        func record(_ request: URLRequest) -> Mode {
            lock.withLock {
                recorded.append(Exchange(
                    method: request.httpMethod ?? "GET",
                    path: request.url?.path ?? "",
                    authorization: request.value(forHTTPHeaderField: "Authorization")
                ))
                return failure
            }
        }
    }

    static let state = State()

    override class func canInit(with request: URLRequest) -> Bool {
        request.url?.host == "sign-out.test"
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func stopLoading() {}

    override func startLoading() {
        let path = request.url?.path ?? ""
        let method = request.httpMethod ?? "GET"
        let mode = Self.state.record(request)
        let signOut = method == "DELETE" && path.hasPrefix("/api/push/devices/")
        if signOut {
            switch mode {
            case .offline:
                client?.urlProtocol(self, didFailWithError: URLError(.notConnectedToInternet))
                return
            case .unauthorized:
                finish(status: 401, body: Data(#"{"error":"unauthorized"}"#.utf8))
                return
            case .deleteRejected where method == "DELETE":
                finish(status: 500, body: Data(#"{"error":"unavailable"}"#.utf8))
                return
            case .success, .deleteRejected:
                finish(status: 200, body: Data(#"{"success":true}"#.utf8))
                return
            }
        }

        let body: Data
        switch path {
        case "/api/health":
            body = Data(#"{"ok":true}"#.utf8)
        case "/api/auth/check":
            body = Data(#"{"required":true}"#.utf8)
        case "/api/auth":
            body = Data(#"{"success":true,"token":"session-token"}"#.utf8)
        case "/api/status":
            body = Data(#"{"connected":true}"#.utf8)
        default:
            finish(status: 404, body: Data(#"{"error":"unexpected"}"#.utf8))
            return
        }
        finish(status: 200, body: body)
    }

    private func finish(status: Int, body: Data) {
        let response = HTTPURLResponse(
            url: request.url!,
            statusCode: status,
            httpVersion: nil,
            headerFields: ["Content-Type": "application/json"]
        )!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: body)
        client?.urlProtocolDidFinishLoading(self)
    }
}
