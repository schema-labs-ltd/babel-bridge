import Foundation

extension APIClient {
    func status() async throws -> BackendStatus {
        try await authorizedRequest("/api/status")
    }

    func whatsAppQRCode() async throws -> WhatsAppQRResponse {
        try await authorizedRequest("/api/qr")
    }

    func contacts() async throws -> [Contact] {
        try await authorizedRequest("/api/contacts")
    }

    static func pinPath(contactID: String) -> String {
        "/api/contacts/\(contactID.urlPathEncoded)/pin"
    }

    func togglePin(contactID: String) async throws -> Bool {
        let response: PinResponse = try await authorizedRequest(
            Self.pinPath(contactID: contactID),
            method: "POST"
        )
        guard response.success else { throw APIError.server("Couldn’t update the pinned conversation.") }
        return response.pinned
    }

    func avatar(contactID: String) async throws -> URL? {
        let response: AvatarResponse = try await authorizedRequest("/api/avatar/\(contactID.urlPathEncoded)")
        return response.url
    }

    static func messagesPath(
        contactID: String,
        limit: Int = 50,
        before: Int64? = nil,
        beforeID: String? = nil
    ) -> String {
        var components = URLComponents()
        components.queryItems = [URLQueryItem(name: "limit", value: String(limit))]
        if let before { components.queryItems?.append(URLQueryItem(name: "before", value: String(before))) }
        if let beforeID { components.queryItems?.append(URLQueryItem(name: "before_id", value: beforeID)) }
        let query = components.percentEncodedQuery.map { "?\($0)" } ?? ""
        return "/api/messages/\(contactID.urlPathEncoded)\(query)"
    }

    func messages(
        contactID: String,
        limit: Int = 50,
        before: Int64? = nil,
        beforeID: String? = nil
    ) async throws -> MessagesResponse {
        try await authorizedRequest(
            Self.messagesPath(contactID: contactID, limit: limit, before: before, beforeID: beforeID)
        )
    }

    func topics() async throws -> TopicCatalog {
        try await authorizedRequest("/api/topics")
    }

    func messageTopics(ids: [String]) async throws -> MessageTopicsResponse {
        try await authorizedRequest("/api/topics/messages", method: "POST",
                                    body: JSONSerialization.data(withJSONObject: ["messageIds": ids]))
    }

    func topicImportPreview() async throws -> TopicImportSummary {
        try await authorizedRequest("/api/topics/import")
    }

    func importRecentTopics() async throws -> TopicImportSummary {
        try await authorizedRequest("/api/topics/import", method: "POST")
    }

    func setTopicsEnabled(_ enabled: Bool, contactID: String) async throws -> TopicCatalog {
        try await authorizedRequest("/api/contacts/\(contactID.urlPathEncoded)/topics", method: "PUT",
                                    body: JSONSerialization.data(withJSONObject: ["enabled": enabled]))
    }

    func topicMessages(id: String, before: Int64? = nil, beforeID: String? = nil) async throws -> MessagesResponse {
        var components = URLComponents()
        components.queryItems = [URLQueryItem(name: "limit", value: "50")]
        if let before { components.queryItems?.append(URLQueryItem(name: "before", value: String(before))) }
        if let beforeID { components.queryItems?.append(URLQueryItem(name: "before_id", value: beforeID)) }
        return try await authorizedRequest("/api/topics/\(id.urlPathEncoded)/messages?" + (components.percentEncodedQuery ?? ""))
    }

    func feed(before: Int64? = nil, beforeID: String? = nil) async throws -> MessagesResponse {
        var components = URLComponents()
        components.queryItems = [URLQueryItem(name: "limit", value: "50")]
        if let before { components.queryItems?.append(URLQueryItem(name: "before", value: String(before))) }
        if let beforeID { components.queryItems?.append(URLQueryItem(name: "before_id", value: beforeID)) }
        return try await authorizedRequest("/api/feed?" + (components.percentEncodedQuery ?? ""))
    }

    func send(contactID: String, text: String, reply: MessageReplyTarget? = nil, replyOnlyIfNotLatest: Bool = false) async throws -> SendMessageResponse {
        let payload = SendMessageRequest(
            replyOnlyIfNotLatest: replyOnlyIfNotLatest ? true : nil,
            contactId: contactID,
            text: text,
            replyTo: reply?.messageID,
            replyToSender: reply?.senderJID,
            replyToText: reply?.text,
            replyToSenderName: reply?.senderName
        )
        return try await authorizedRequest(
            "/api/send",
            method: "POST",
            body: JSONEncoder.backend.encode(payload)
        )
    }

    func sendAttachment(_ attachment: OutgoingAttachment, contactID: String, caption: String?, reply: MessageReplyTarget?, replyOnlyIfNotLatest: Bool) async throws -> SendImageResponse {
        let payload = SendImageRequest(
            mediaKind: attachment.kind, fileName: attachment.fileName,
            replyOnlyIfNotLatest: replyOnlyIfNotLatest ? true : nil,
            contactId: contactID, mediaData: attachment.data.base64EncodedString(),
            mimeType: attachment.mimeType, caption: caption,
            replyTo: reply?.messageID, replyToSender: reply?.senderJID,
            replyToText: reply?.text, replyToSenderName: reply?.senderName
        )
        return try await authorizedRequest("/api/send-media", method: "POST", body: JSONEncoder.backend.encode(payload), timeoutInterval: 240)
    }

    func sendImage(
        contactID: String,
        data: Data,
        mimeType: String,
        caption: String? = nil,
        reply: MessageReplyTarget? = nil,
        replyOnlyIfNotLatest: Bool = false
    ) async throws -> SendImageResponse {
        let payload = SendImageRequest(
            replyOnlyIfNotLatest: replyOnlyIfNotLatest ? true : nil,
            contactId: contactID,
            mediaData: data.base64EncodedString(),
            mimeType: mimeType,
            caption: caption,
            replyTo: reply?.messageID,
            replyToSender: reply?.senderJID,
            replyToText: reply?.text,
            replyToSenderName: reply?.senderName
        )
        return try await authorizedRequest(
            "/api/send-image",
            method: "POST",
            body: JSONEncoder.backend.encode(payload)
        )
    }

    func sendImages(
        contactID: String,
        images: [OutgoingImage],
        progressID: String? = nil,
        caption: String? = nil,
        reply: MessageReplyTarget? = nil,
        replyOnlyIfNotLatest: Bool = false,
        transferProgress: (@Sendable (Int, Int) async -> Void)? = nil
    ) async throws -> SendImageResponse {
        if images.count == 1, let image = images.first {
            return try await sendImage(
                contactID: contactID,
                data: image.data,
                mimeType: image.mimeType,
                caption: caption,
                reply: reply, replyOnlyIfNotLatest: replyOnlyIfNotLatest
            )
        }
        let jobID = progressID ?? UUID().uuidString
        let createPayload = CreatePhotoAlbumRequest(
            replyOnlyIfNotLatest: replyOnlyIfNotLatest ? true : nil,
            jobId: jobID,
            contactId: contactID,
            photoCount: images.count,
            caption: caption,
            replyTo: reply?.messageID,
            replyToSender: reply?.senderJID,
            replyToText: reply?.text,
            replyToSenderName: reply?.senderName
        )
        let _: SuccessResponse = try await authorizedRequest(
            "/api/photo-albums", method: "POST", body: JSONEncoder.backend.encode(createPayload)
        )
        for (index, image) in images.enumerated() {
            let item = StagePhotoAlbumItemRequest(
                mediaData: image.data.base64EncodedString(), mimeType: image.mimeType
            )
            let body = try JSONEncoder.backend.encode(item)
            var lastError: Error?
            for attempt in 0..<2 {
                do {
                    let _: SuccessResponse = try await durableAuthorizedUpload(
                        "/api/photo-albums/\(jobID)/images/\(index)", method: "PUT", body: body
                    )
                    lastError = nil
                    break
                } catch {
                    lastError = error
                    if attempt == 0, (error as? URLError)?.code != .timedOut { continue }
                }
            }
            if let lastError { throw lastError }
            await transferProgress?(index + 1, images.count)
        }
        return try await durableAuthorizedUpload(
            "/api/photo-albums/\(jobID)/send", method: "POST", body: Data("{}".utf8),
            timeoutInterval: 15 * 60
        )
    }

    func react(to message: ChatMessage, emoji: String, senderJID: String?) async throws -> ChatMessage? {
        let payload = SendReactionRequest(
            contactId: message.contactId,
            messageId: message.id,
            senderJid: senderJID,
            emoji: emoji
        )
        let response: ReactionResponse = try await authorizedRequest(
            "/api/react",
            method: "POST",
            body: JSONEncoder.backend.encode(payload)
        )
        guard response.success else { throw APIError.server(response.error ?? "Reaction was not sent.") }
        return response.reaction
    }

    func translate(_ message: ChatMessage) async throws -> TranslateMessageResponse {
        guard let text = message.contentText else { throw APIError.server("No text to translate.") }
        let payload = TranslateMessageRequest(text: text, messageId: message.id, contactId: message.contactId)
        return try await authorizedRequest(
            "/api/translate",
            method: "POST",
            body: JSONEncoder.backend.encode(payload)
        )
    }
}
