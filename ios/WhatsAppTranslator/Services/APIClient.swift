import Foundation

actor APIClient {
    let session: URLSession
    private var configuration: ServerConfiguration?
    var token: String?
    var webSocket: URLSessionWebSocketTask?
    var receiveTask: Task<Void, Never>?
    var liveGeneration = UUID()
    var lastLiveMessage = Date()

    init(session: URLSession = .shared) {
        self.session = session
    }

    func configure(_ configuration: ServerConfiguration) {
        disconnectLiveEvents()
        self.configuration = configuration
        token = nil
    }

    func authenticate() async throws -> BackendStatus {
        guard let configuration else { throw APIError.notConfigured }
        let health: HealthResponse = try await request("/api/health", authenticated: false)
        guard health.ok else { throw APIError.invalidServer }

        try await prepareAuthenticatedRequests(configuration: configuration)
        return try await status()
    }

    func prepareAuthenticatedRequests() async throws {
        guard let configuration else { throw APIError.notConfigured }
        try await prepareAuthenticatedRequests(configuration: configuration)
    }

    private func prepareAuthenticatedRequests(configuration: ServerConfiguration) async throws {
        let check: AuthCheckResponse = try await request("/api/auth/check", authenticated: false)
        if check.required {
            let body = try JSONEncoder.backend.encode(["password": configuration.password])
            let response: AuthResponse = try await request(
                "/api/auth",
                method: "POST",
                body: body,
                authenticated: false
            )
            guard response.success else { throw APIError.server(response.error ?? "Incorrect password") }
            token = response.token
        }
    }

    func voiceSample(preference: String) async throws -> VoiceSample {
        try await authorizedRequest("/api/voice/sample/\(preference.urlPathEncoded)", method: "POST", timeoutInterval: 90)
    }

    func voicePreferences(scope: String) async throws -> VoicePreferences {
        try await authorizedRequest("/api/voice/settings/\(scope.urlPathEncoded)")
    }

    static func messageTonePath(contactID: String?) -> String {
        guard let contactID else { return "/api/settings/message-tone" }
        var components = URLComponents()
        components.queryItems = [URLQueryItem(name: "contactId", value: contactID)]
        let query = (components.percentEncodedQuery ?? "").replacingOccurrences(of: "+", with: "%2B")
        return "/api/settings/message-tone?\(query)"
    }

    func messageToneSettings(contactID: String?) async throws -> MessageToneSettings {
        try await authorizedRequest(Self.messageTonePath(contactID: contactID))
    }

    func saveMessageTone(_ tone: MessageTone?, contactID: String?) async throws -> MessageToneSettings {
        try await authorizedRequest(
            Self.messageTonePath(contactID: contactID), method: "PUT",
            body: JSONEncoder.backend.encode(MessageToneUpdate(tone: tone))
        )
    }

    func saveVoicePreferences(_ preferences: VoicePreferences, scope: String) async throws {
        let _: VoicePreferences = try await authorizedRequest("/api/voice/settings/\(scope.urlPathEncoded)", method: "PUT", body: JSONEncoder.backend.encode(preferences))
    }

    func translateVoice(messageID: String) async throws -> TranslatedVoiceNote {
        try await authorizedRequest("/api/voice/translate/\(messageID.urlPathEncoded)", method: "POST", timeoutInterval: 300)
    }

    func prepareVoice(data: Data, contactID: String, reply: MessageReplyTarget?, replyOnlyIfNotLatest: Bool = false) async throws -> TranslatedVoiceNote {
        let payload = PrepareVoiceRequest(
            contactId: contactID, mediaData: data.base64EncodedString(),
            replyTo: reply?.messageID, replyToSender: reply?.senderJID,
            replyToText: reply?.text, replyOnlyIfNotLatest: replyOnlyIfNotLatest
        )
        return try await authorizedRequest("/api/voice/prepare", method: "POST", body: JSONEncoder.backend.encode(payload), timeoutInterval: 300)
    }

    func sendVoice(preparationID: String) async throws -> VoiceSendResult {
        try await authorizedRequest("/api/voice/send", method: "POST", body: JSONEncoder.backend.encode(["preparationId": preparationID]), timeoutInterval: 120)
    }

    func authorizedRequest<T: Decodable & Sendable>(
        _ path: String,
        method: String = "GET",
        body: Data? = nil,
        timeoutInterval: TimeInterval? = nil
    ) async throws -> T {
        do {
            return try await request(path, method: method, body: body, authenticated: true, timeoutInterval: timeoutInterval)
        } catch APIError.unauthorized {
            _ = try await authenticate()
            return try await request(path, method: method, body: body, authenticated: true, timeoutInterval: timeoutInterval)
        }
    }

    func durableAuthorizedUpload<T: Decodable & Sendable>(
        _ path: String,
        method: String,
        body: Data,
        timeoutInterval: TimeInterval = 300
    ) async throws -> T {
        #if os(iOS) && !APP_EXTENSION
        return try await backgroundAuthorizedUpload(
            path, method: method, body: body, timeoutInterval: timeoutInterval
        )
        #else
        return try await authorizedRequest(
            path, method: method, body: body, timeoutInterval: timeoutInterval
        )
        #endif
    }

    #if os(iOS) && !APP_EXTENSION
    private func backgroundAuthorizedUpload<T: Decodable & Sendable>(
        _ path: String,
        method: String,
        body: Data,
        timeoutInterval: TimeInterval
    ) async throws -> T {
        do {
            return try await performBackgroundUpload(
                path, method: method, body: body, timeoutInterval: timeoutInterval
            )
        } catch APIError.unauthorized {
            _ = try await authenticate()
            return try await performBackgroundUpload(
                path, method: method, body: body, timeoutInterval: timeoutInterval
            )
        }
    }

    private func performBackgroundUpload<T: Decodable & Sendable>(
        _ path: String,
        method: String,
        body: Data,
        timeoutInterval: TimeInterval
    ) async throws -> T {
        guard let configuration,
              let url = configuration.endpointURL(for: path) else {
            throw APIError.notConfigured
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.timeoutInterval = timeoutInterval
        let sendIdentity = try await SendRecoveryStore.shared.identity(server: configuration.baseURL, path: path, method: method, body: body)
        if let sendIdentity { request.setValue(sendIdentity.key, forHTTPHeaderField: "Idempotency-Key") }
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if let token { request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
        let result = try await BackgroundPhotoUploadSession.shared.upload(request: request, body: body)
        guard let response = result.response as? HTTPURLResponse else { throw APIError.invalidServer }
        try await SendRecoveryStore.shared.settle(sendIdentity, response: response)
        if response.statusCode == 401 { throw APIError.unauthorized }
        guard (200..<300).contains(response.statusCode) else {
            let message = (try? JSONDecoder.backend.decode(ErrorResponse.self, from: result.data).error)
                ?? String(data: result.data, encoding: .utf8) ?? "Request failed"
            throw APIError.server(message)
        }
        return try JSONDecoder.backend.decode(T.self, from: result.data)
    }
    #endif

    private func request<T: Decodable & Sendable>(
        _ path: String,
        method: String = "GET",
        body: Data? = nil,
        authenticated: Bool,
        timeoutInterval: TimeInterval? = nil
    ) async throws -> T {
        guard let configuration else { throw APIError.notConfigured }
        guard let url = configuration.endpointURL(for: path) else {
            throw APIError.invalidServer
        }
        var request = URLRequest(url: url)
        if let timeoutInterval { request.timeoutInterval = timeoutInterval }
        let sendIdentity = try await SendRecoveryStore.shared.identity(server: configuration.baseURL, path: path, method: method, body: body)
        if let sendIdentity { request.setValue(sendIdentity.key, forHTTPHeaderField: "Idempotency-Key") }
        request.httpMethod = method
        request.httpBody = body
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if authenticated, let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }

        let data: Data
        let response: URLResponse
        do { (data, response) = try await session.data(for: request) }
        catch {
            if sendIdentity != nil { throw APIError.server("Delivery is uncertain. Retry the same action to check its result without sending twice.") }
            throw error
        }
        guard let http = response as? HTTPURLResponse else { throw APIError.invalidServer }
        try await SendRecoveryStore.shared.settle(sendIdentity, response: http)
        if http.statusCode == 401 { throw APIError.unauthorized }
        if http.statusCode == 404 && path.hasPrefix("/api/voice/") {
            throw APIError.server("This server needs the voice-note update. Update the backend, then retry.")
        }
        guard (200..<300).contains(http.statusCode) else {
            let message = (try? JSONDecoder.backend.decode(ErrorResponse.self, from: data).error)
                ?? String(data: data, encoding: .utf8)
                ?? "Request failed"
            throw APIError.server(message)
        }
        do {
            return try JSONDecoder.backend.decode(T.self, from: data)
        } catch {
            throw APIError.decoding(error.localizedDescription)
        }
    }
}

enum APIError: LocalizedError, Equatable {
    case notConfigured
    case invalidServer
    case unauthorized
    case server(String)
    case decoding(String)

    var errorDescription: String? {
        switch self {
        case .notConfigured: "Set up your translator server first."
        case .invalidServer: "That address is not a Babel Bridge server."
        case .unauthorized: "The password is no longer valid."
        case .server(let message), .decoding(let message): message
        }
    }
}

extension JSONDecoder {
    static var backend: JSONDecoder { JSONDecoder() }
}

extension JSONEncoder {
    static var backend: JSONEncoder { JSONEncoder() }
}

private extension String {
    var urlPathEncoded: String {
        addingPercentEncoding(withAllowedCharacters: .urlPathAllowed.subtracting(CharacterSet(charactersIn: "/"))) ?? self
    }
}
