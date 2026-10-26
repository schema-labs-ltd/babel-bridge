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
