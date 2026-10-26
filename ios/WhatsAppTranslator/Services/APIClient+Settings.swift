import Foundation

extension APIClient {
    func generateAIReply(to message: ChatMessage) async throws -> AIReplyResponse {
        let payload = AIReplyRequest(contactId: message.contactId, messageId: message.id)
        return try await authorizedRequest(
            "/api/ai-reply",
            method: "POST",
            body: JSONEncoder.backend.encode(payload)
        )
    }

    func conversationUsage(contactID: String) async throws -> UsageSummary {
        try await authorizedRequest("/api/usage/\(contactID.urlPathEncoded)")
    }

    func media(messageID: String) async throws -> MediaResponse {
        try await authorizedRequest("/api/media/\(messageID.urlPathEncoded)")
    }

    func gallery(contactID: String, before: Int64? = nil, beforeID: String? = nil) async throws -> MessagesResponse {
        let path = Self.messagesPath(contactID: contactID, limit: 60, before: before, beforeID: beforeID)
            .replacingOccurrences(of: "/api/messages/", with: "/api/gallery/")
        return try await authorizedRequest(path)
    }

    func mediaThumbnail(messageID: String) async throws -> MediaResponse {
        try await authorizedRequest("/api/media/\(messageID.urlPathEncoded)/thumbnail")
    }

    func linkPreview(for url: URL) async throws -> LinkPreview {
        var components = URLComponents()
        components.queryItems = [URLQueryItem(name: "url", value: url.absoluteString)]
        let query = components.percentEncodedQuery.map { "?\($0)" } ?? ""
        return try await authorizedRequest("/api/link-preview\(query)")
    }

    func markViewedMessages(contactID: String, messageIDs: [String]) async throws -> ViewedMessagesResponse {
        let body = try JSONEncoder.backend.encode(ViewedMessagesRequest(messageIds: messageIDs))
        return try await authorizedRequest(
            "/api/contacts/\(contactID.urlPathEncoded)/read",
            method: "POST",
            body: body
        )
    }

    func conversationSettings(contactID: String) async throws -> ConversationSettings {
        try await authorizedRequest("/api/contacts/\(contactID.urlPathEncoded)/settings")
    }

    func updateConversationSettings(contactID: String, settings: ConversationSettings) async throws {
        let _: SuccessResponse = try await authorizedRequest(
            "/api/contacts/\(contactID.urlPathEncoded)/settings",
            method: "PUT",
            body: JSONEncoder.backend.encode(settings)
        )
    }

    func openAISettings() async throws -> OpenAISettings {
        try await authorizedRequest("/api/settings/openai")
    }

    func updateOpenAISettings(_ settings: OpenAISettings) async throws {
        let _: SuccessResponse = try await authorizedRequest(
            "/api/settings/openai",
            method: "PUT",
            body: JSONEncoder.backend.encode(settings)
        )
    }

    func chatGPTStatus() async throws -> ChatGPTStatus {
        try await authorizedRequest("/api/chatgpt")
    }

    func startChatGPTSignIn(redirectURI: String) async throws -> ChatGPTSignInStart {
        try await authorizedRequest(
            "/api/chatgpt/sign-in",
            method: "POST",
            body: JSONEncoder.backend.encode(["redirectUri": redirectURI])
        )
    }

    func completeChatGPTSignIn(callbackURL: String) async throws -> ChatGPTStatus {
        try await authorizedRequest(
            "/api/chatgpt/callback",
            method: "POST",
            body: JSONEncoder.backend.encode(["callbackUrl": callbackURL])
        )
    }

    func setChatGPTModel(_ model: String) async throws -> ChatGPTStatus {
        try await authorizedRequest(
            "/api/chatgpt/model",
            method: "PUT",
            body: JSONEncoder.backend.encode(["model": model])
        )
    }

    func signOutOfChatGPT() async throws -> ChatGPTStatus {
        try await authorizedRequest("/api/chatgpt/sign-out", method: "POST", body: Data("{}".utf8))
    }

    func registerPushDevice(_ registration: PushDeviceRegistration) async throws {
        let _: SuccessResponse = try await authorizedRequest(
            "/api/push/devices",
            method: "PUT",
            body: JSONEncoder.backend.encode(registration)
        )
    }
}
