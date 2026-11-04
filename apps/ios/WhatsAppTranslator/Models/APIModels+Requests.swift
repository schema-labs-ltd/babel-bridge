import Foundation

struct SendMessageRequest: Encodable, Sendable {
    var replyOnlyIfNotLatest: Bool? = nil
    let contactId: String
    let text: String
    let replyTo: String?
    let replyToSender: String?
    let replyToText: String?
    let replyToSenderName: String?
}

struct SendMessageResponse: Decodable, Sendable {
    let messageId: String
    let timestamp: Int64
    let isTranslated: Bool
    let translatedText: String?
    let sourceLanguage: String?
    let originalFollowUpSent: Bool
    let originalMessageId: String?
    let originalTimestamp: Int64?
    let originalFollowUpError: String?
}

struct PrepareVoiceRequest: Encodable, Sendable {
    let contactId: String
    let mediaData: String
    let replyTo: String?
    let replyToSender: String?
    let replyToText: String?
    let replyOnlyIfNotLatest: Bool
}

struct SendImageRequest: Encodable, Sendable {
    var mediaKind: String? = nil
    var fileName: String? = nil
    var replyOnlyIfNotLatest: Bool? = nil
    let contactId: String
    let mediaData: String
    let mimeType: String
    let caption: String?
    let replyTo: String?
    let replyToSender: String?
    let replyToText: String?
    let replyToSenderName: String?
}

struct OutgoingAttachment: Sendable {
    let data: Data
    let mimeType: String
    let fileName: String
    let kind: String
    static let maximumBytes = 64 * 1_024 * 1_024
}

struct OutgoingImage: Sendable {
    let data: Data
    let mimeType: String
}

struct SendImageItemRequest: Encodable, Sendable {
    let mediaData: String
    let mimeType: String
}

struct SendImagesRequest: Encodable, Sendable {
    var replyOnlyIfNotLatest: Bool? = nil
    let contactId: String
    let progressId: String?
    let images: [SendImageItemRequest]
    let caption: String?
    let replyTo: String?
    let replyToSender: String?
    let replyToText: String?
    let replyToSenderName: String?
}

struct CreatePhotoAlbumRequest: Encodable, Sendable {
    var replyOnlyIfNotLatest: Bool? = nil
    let jobId: String
    let contactId: String
    let photoCount: Int
    let caption: String?
    let replyTo: String?
    let replyToSender: String?
    let replyToText: String?
    let replyToSenderName: String?
}

struct StagePhotoAlbumItemRequest: Encodable, Sendable {
    let mediaData: String
    let mimeType: String
}

struct PhotoSendProgress: Identifiable, Equatable, Sendable {
    let startedAt = Date()
    enum Stage: String, Sendable { case preparing, transferring, uploading, sending, complete, failed }
    let id: String
    let contactID: String
    var stage: Stage
    var completed: Int
    let total: Int
    var error: String?

    var statusText: String {
        switch stage {
        case .preparing: "Preparing \(completed) of \(total)"
        case .transferring:
            completed == 0 ? "Starting photo transfer…" : "Transferring \(completed) of \(total)"
        case .uploading: "Uploading \(completed) of \(total)"
        case .sending: "Sending \(completed) of \(total)"
        case .complete: "Sent \(total) photo\(total == 1 ? "" : "s")"
        case .failed: error ?? "Couldn’t send photos"
        }
    }

    var fractionCompleted: Double {
        stage == .complete ? 1 : min(1, max(0, Double(completed) / Double(max(total, 1))))
    }
}

struct SendImageResponse: Decodable, Sendable {
    let messageId: String
    let timestamp: Int64
}

struct SendReactionRequest: Encodable, Sendable {
    let contactId: String
    let messageId: String
    let senderJid: String?
    let emoji: String
}

struct TranslateMessageRequest: Encodable, Sendable {
    let text: String
    let messageId: String
    let contactId: String
}

struct TranslateMessageResponse: Decodable, Sendable {
    let success: Bool
    let translatedText: String?
    let sourceLanguage: String?
    let error: String?
}

struct AIReplyRequest: Encodable, Sendable {
    let contactId: String
    let messageId: String
}

struct AIReplyResponse: Decodable, Sendable {
    let success: Bool
    let replyText: String?
    let error: String?
    let costUsd: Double?
}

struct UsageSummary: Decodable, Equatable, Sendable {
    let inputTokens: Int
    let cachedInputTokens: Int
    let outputTokens: Int
    let costUsd: Double
}

/// A text message shown in its chat from the moment Send is tapped, as WhatsApp does: a spinner
/// beside it while it sends, and a resend button in its place if the send fails.
struct PendingSend: Codable, Equatable, Sendable {
    static let idPrefix = "pending-send-"

    let id: String
    let contactID: String
    let text: String
    let reply: MessageReplyTarget?
    let replyOnlyIfNotLatest: Bool
    let timestamp: Int64
    /// The quote the bubble shows; a feed reply to the latest message goes without one.
    let quote: MessageReplyTarget?
    let contactName: String?
    let chatType: String
    /// The chat's messages already loaded when Send was tapped, which can't be this send's copy
    /// even when they say the same, as a second "ok" does.
    var earlierIDs: Set<String> = []
    var failure: String?
    /// Set once the server accepted it, so a copy restored after a relaunch is never resent
    /// under a new idempotency key.
    var confirmed = false

    var failed: Bool { failure != nil }

    var message: ChatMessage {
        ChatMessage(
            id: id, contactId: contactID, timestamp: timestamp, isFromMe: true, isForwarded: false,
            senderName: nil, senderPhone: nil, contactName: contactName, contactPhone: nil,
            chatType: chatType, contentType: "Text",
            content: MessageContent(type: "text", body: text, showTranslatedPrimary: nil, replyContext: quote.map { ReplyContext(messageId: $0.messageID, senderName: $0.senderName, text: $0.text) }),
            originalText: nil, translatedText: nil, sourceLanguage: nil, isTranslated: false,
            deliveryStatus: "sending"
        )
    }

    /// The sends the server has no copy of yet. The server stores what was typed as soon as it
    /// starts sending, so a loaded message with the same words, still sending or uncertain on
    /// the server, takes its place; each loaded message stands in for one send at most. Once
    /// the server confirms a send, its reply names the message, so words are never matched to a
    /// sent one.
    static func unconfirmed(_ pending: [PendingSend], loaded: [ChatMessage]) -> [PendingSend] {
        let matches = copies(of: pending, in: loaded)
        return pending.sorted { ($0.timestamp, $0.id) < ($1.timestamp, $1.id) }.filter { matches[$0.id] == nil }
    }

    /// Pairs sends with their copies oldest first, each copy going to one send only, so a
    /// copy of an earlier send is never taken for a later one with the same words.
    static func copies(of pending: [PendingSend], in loaded: [ChatMessage]) -> [String: ChatMessage] {
        var claimed = Set<String>()
        var matches: [String: ChatMessage] = [:]
        for send in pending.sorted(by: { ($0.timestamp, $0.id) < ($1.timestamp, $1.id) }) {
            guard let match = send.copy(in: loaded, excluding: claimed) else { continue }
            claimed.insert(match.id)
            matches[send.id] = match
        }
        return matches
    }

    /// The server's unconfirmed copy of this send among the loaded messages, if it has arrived:
    /// the row it stores while sending, which stays when the send is uncertain. The row for a
    /// translated send's original-text follow-up is never it.
    func copy(in loaded: [ChatMessage], excluding claimed: Set<String> = []) -> ChatMessage? {
        loaded.first { message in
            message.isFromMe && message.contactId == contactID
                && ["sending", "uncertain"].contains(message.deliveryStatus?.lowercased() ?? "")
                && !message.id.hasPrefix("pending_original_")
                && !claimed.contains(message.id) && !earlierIDs.contains(message.id)
                // Allow for the phone's clock running ahead of the server's.
                && message.timestamp >= timestamp - 120_000
                && (message.content?.body ?? message.originalText)?.trimmingCharacters(in: .whitespacesAndNewlines) == text
        }
    }

    /// The message as the server confirmed it, shown until the chat reloads with its own copy.
    func confirmedMessage(_ response: SendMessageResponse) -> ChatMessage {
        ChatMessage(
            id: response.messageId, contactId: contactID, timestamp: response.timestamp, isFromMe: true, isForwarded: false,
            senderName: nil, senderPhone: nil, contactName: contactName, contactPhone: nil,
            chatType: chatType, contentType: "Text",
            content: MessageContent(type: "text", body: text, showTranslatedPrimary: nil, replyContext: quote.map { ReplyContext(messageId: $0.messageID, senderName: $0.senderName, text: $0.text) }),
            originalText: response.isTranslated ? text : nil, translatedText: response.translatedText,
            sourceLanguage: response.sourceLanguage, isTranslated: response.isTranslated,
            deliveryStatus: "sent"
        )
    }

    /// Places each pending message by its time among the loaded ones, keeping their order.
    static func merged(_ pending: [ChatMessage], into loaded: [ChatMessage]) -> [ChatMessage] {
        var result = loaded
        for message in pending.sorted(by: { ($0.timestamp, $0.id) < ($1.timestamp, $1.id) }) {
            let index = result.lastIndex { $0.timestamp <= message.timestamp }.map { $0 + 1 } ?? 0
            result.insert(message, at: index)
        }
        return result
    }
}
