import Foundation

struct AuthCheckResponse: Decodable, Sendable { let required: Bool }

struct HealthResponse: Decodable, Sendable { let ok: Bool }
struct SuccessResponse: Decodable, Sendable { let success: Bool }
struct ErrorResponse: Decodable, Sendable { let error: String }

struct PinResponse: Decodable, Sendable {
    let success: Bool
    let pinned: Bool
}

struct ViewedMessagesRequest: Encodable, Sendable { let messageIds: [String] }
struct ViewedMessagesResponse: Decodable, Sendable { let success: Bool; let unreadCount: Int }

struct AuthResponse: Decodable, Sendable {
    let success: Bool
    let token: String?
    let error: String?
}

struct BackendStatus: Decodable, Sendable {
    let connected: Bool
    let phone: String?
    let name: String?
    var connectionState: String? = nil
    var qr: String? = nil

    private enum CodingKeys: String, CodingKey {
        case connected, phone, name, qr
        case connectionState = "connection_state"
    }
}

struct WhatsAppQRResponse: Decodable, Sendable { let qr: String? }

struct AvatarResponse: Decodable, Sendable { let url: URL? }

struct Contact: Codable, Identifiable, Hashable, Sendable {
    let id: String
    let name: String?
    let phone: String?
    let type: String?
    let lastMessageTime: Int64
    var unreadCount: Int
    let pinnedAt: Int64?
    let lastMessagePreview: String?

    var isUpdates: Bool { id.caseInsensitiveCompare("status@broadcast") == .orderedSame || type == "status" }
    var showsAsUpdatesToolbarItem: Bool { isUpdates }
    var showsInChatList: Bool { !isUpdates }
    var displayName: String { isUpdates ? "Updates" : (name?.nilIfBlank ?? phone?.nilIfBlank ?? "Unknown chat") }
    var isGroup: Bool { type == "group" || id.contains("@g.us") }
    var initials: String {
        let parts = displayName.split(separator: " ").prefix(2)
        return parts.compactMap(\.first).map(String.init).joined().uppercased()
    }
}

struct MessagesResponse: Decodable, Sendable {
    let messages: [ChatMessage]
    let hasMore: Bool
}

enum MessageMediaKind: String, Equatable, Sendable {
    case image
    case video
    case audio
    case document
    case sticker
}

enum MessageActionKind: Equatable, Sendable {
    case reply
    case translate
    case aiReply
    case star
    case react
}

struct ChatMessage: Codable, Identifiable, Hashable, Sendable {
    let id: String
    let contactId: String
    let timestamp: Int64
    let isFromMe: Bool
    let isForwarded: Bool
    let senderName: String?
    let senderPhone: String?
    let contactName: String?
    let contactPhone: String?
    let chatType: String
    let contentType: String
    let content: MessageContent?
    let originalText: String?
    let translatedText: String?
    let sourceLanguage: String?
    let isTranslated: Bool
    var deliveryStatus: String? = nil
    var reactions: [String: [String]]? = nil
    var reactionStates: [String: MessageReactionState]? = nil

    var ownReactionEmoji: String? {
        reactions?.first(where: { $0.value.contains("me") })?.key
    }

    var editRevision: Int64 { content?.editedAtMS ?? 0 }
    var isEdited: Bool { editRevision > 0 }

    var deliveryState: MessageDeliveryState {
        guard isFromMe else { return .none }
        return switch deliveryStatus?.lowercased() {
        case "sending": .sending
        case "uncertain": .uncertain
        case "read", "played": .read
        case "delivered": .delivered
        default: .sent
        }
    }

    var displayText: String {
        let richFallback: String? = switch normalizedContentType {
        case "audio": content?.isVoiceNote == true ? "Voice note" : "Audio"
        case "document": content?.fileName?.nilIfBlank ?? "Document"
        case "sticker": "Sticker"
        case "location": content?.name?.nilIfBlank ?? content?.address?.nilIfBlank ?? "Location"
        case "contact": content?.displayName?.nilIfBlank ?? content?.name?.nilIfBlank ?? "Contact"
        case "poll": content?.question?.nilIfBlank ?? "Poll"
        case "revoked": "This message was deleted"
        case "video": "Video"
        case "image": "Image"
        default: nil
        }
        if isFromMe {
            return content?.body?.nilIfBlank ?? content?.caption?.nilIfBlank ?? originalText?.nilIfBlank ?? translatedText?.nilIfBlank ?? richFallback ?? contentType
        }
        return translatedText?.nilIfBlank ?? content?.body?.nilIfBlank ?? content?.caption?.nilIfBlank ?? originalText?.nilIfBlank ?? richFallback ?? contentType
    }

    var alternateText: String? {
        let candidate = isFromMe ? translatedText : originalText
        guard let candidate = candidate?.nilIfBlank, candidate != displayText else { return nil }
        return candidate
    }

    var date: Date { Date(timeIntervalSince1970: TimeInterval(timestamp) / 1_000) }
    var normalizedContentType: String {
        content?.type?.lowercased().nilIfBlank ?? contentType.lowercased()
    }
    var isReaction: Bool {
        contentType.caseInsensitiveCompare("reaction") == .orderedSame
            || content?.type?.caseInsensitiveCompare("reaction") == .orderedSame
    }
    var mediaKind: MessageMediaKind? { MessageMediaKind(rawValue: normalizedContentType) }
    var isImage: Bool { mediaKind == .image }
    var albumID: String? { content?.albumId?.nilIfBlank }
    var albumIndex: Int? { content?.albumIndex }
    var standaloneEmojiText: String? {
        guard normalizedContentType == "text", content?.replyContext == nil else { return nil }
        let value = displayText.trimmingCharacters(in: .whitespacesAndNewlines)
        let characters = Array(value)
        guard (1...3).contains(characters.count), characters.allSatisfy(\.isEmojiCharacter) else {
            return nil
        }
        return value
    }
    var canSendReadReceipt: Bool {
        !isFromMe && !contactId.hasSuffix("@broadcast") && !contactId.hasSuffix("@newsletter")
            && !["reaction", "revoked", "protocol", "unknown"].contains(normalizedContentType)
    }
    var canTranslate: Bool { !isFromMe && !isTranslated && contentText != nil }
    var canGenerateAIReply: Bool { !isFromMe && contentText != nil }
    var contentText: String? { content?.body?.nilIfBlank ?? content?.caption?.nilIfBlank }
    var availableActions: [MessageActionKind] {
        var actions: [MessageActionKind] = [.reply]
        if canTranslate { actions.append(.translate) }
        if canGenerateAIReply { actions.append(.aiReply) }
        actions.append(contentsOf: [.star, .react])
        return actions
    }
    var locationURL: URL? {
        guard normalizedContentType == "location",
              let latitude = content?.latitude,
              let longitude = content?.longitude else { return nil }
        var components = URLComponents(string: "https://maps.apple.com/")
        let coordinate = "\(latitude),\(longitude)"
        components?.queryItems = [
            URLQueryItem(name: "ll", value: coordinate),
            URLQueryItem(name: "q", value: content?.name?.nilIfBlank ?? content?.address?.nilIfBlank ?? "Location"),
        ]
        return components?.url
    }
    var senderJID: String? {
        guard let senderPhone = senderPhone?.nilIfBlank else { return nil }
        return senderPhone.contains("@") ? senderPhone : "\(senderPhone)@s.whatsapp.net"
    }
    var replyTarget: MessageReplyTarget {
        MessageReplyTarget(
            messageID: id,
            senderJID: senderJID,
            senderName: isFromMe ? "You" : (senderName?.nilIfBlank ?? contactName?.nilIfBlank ?? senderPhone?.nilIfBlank ?? "Contact"),
            text: contentText ?? displayText
        )
    }
    var extractedURLs: [URL] {
        let candidates = displayText.split(whereSeparator: { $0.isWhitespace })
        return candidates.compactMap { value in
            let cleaned = value.trimmingCharacters(in: CharacterSet(charactersIn: "()[]{}<>,.!?;:\"'"))
            guard cleaned.hasPrefix("https://") || cleaned.hasPrefix("http://") else { return nil }
            return URL(string: cleaned)
        }
    }
}

struct MessageReactionState: Codable, Hashable, Sendable {
    let id: String
    let timestamp: Int64
    let emoji: String
}

struct ReactionResponse: Decodable, Sendable {
    let success: Bool
    let reaction: ChatMessage?
    let error: String?
}

struct PhotoAlbumTimeline: Identifiable {
    let id: String
    let messages: [ChatMessage]

    var primaryMessage: ChatMessage { messages[0] }
    var date: Date { messages.map(\.date).min() ?? primaryMessage.date }
}

enum PhotoGalleryLayout {
    static let previewLimit = 4
    static func hiddenCount(total: Int) -> Int { max(0, total - previewLimit) }
    static func page(after offset: Int, current: Int, count: Int) -> Int {
        min(max(0, count - 1), max(0, current + offset))
    }
}

enum ConversationTimelineItem: Identifiable {
    case message(ChatMessage)
    case photoAlbum(PhotoAlbumTimeline)

    // The first message remains a scroll anchor when a live photo extends a run.
    var id: String {
        switch self {
        case let .message(message): message.id
        case let .photoAlbum(album): album.id
        }
    }
    var messages: [ChatMessage] {
        switch self {
        case let .message(message): [message]
        case let .photoAlbum(album): album.messages
        }
    }
    var primaryMessage: ChatMessage { messages[0] }
    var date: Date {
        switch self {
        case let .message(message): message.date
        case let .photoAlbum(album): album.date
        }
    }
}

enum ConversationTimelineBuilder {
    static func items(from messages: [ChatMessage]) -> [ConversationTimelineItem] {
        var result: [ConversationTimelineItem] = []
        var run: [ChatMessage] = []
        func flush() {
            guard let first = run.first else { return }
            if run.count == 1 {
                result.append(.message(first))
            } else {
                // Preserve order within each explicit album even when one run
                // joins several albums and ordinary image messages.
                var ordered: [ChatMessage] = []
                var start = 0
                while start < run.count {
                    var end = start + 1
                    if let albumID = run[start].albumID {
                        while end < run.count && run[end].albumID == albumID { end += 1 }
                    }
                    ordered += run[start..<end].sorted {
                        if $0.albumIndex != $1.albumIndex { return ($0.albumIndex ?? Int.max) < ($1.albumIndex ?? Int.max) }
                        return $0.timestamp == $1.timestamp ? $0.id < $1.id : $0.timestamp < $1.timestamp
                    }
                    start = end
                }
                result.append(.photoAlbum(PhotoAlbumTimeline(id: first.id, messages: ordered)))
            }
            run.removeAll(keepingCapacity: true)
        }
        for message in messages {
            if let previous = run.last, !canGroup(previous, message) { flush() }
            if message.isImage {
                run.append(message)
            } else {
                result.append(.message(message))
            }
        }
        flush()
        return result
    }

    private static func canGroup(_ left: ChatMessage, _ right: ChatMessage) -> Bool {
        guard left.isImage, right.isImage, left.contactId == right.contactId,
              left.isFromMe == right.isFromMe,
              Calendar.current.isDate(left.date, inSameDayAs: right.date) else { return false }
        if left.isFromMe { return true }
        if !left.contactId.contains("@g.us"), left.chatType.lowercased() != "group" { return true }
        if left.senderJID != nil || right.senderJID != nil {
            return left.senderJID != nil && left.senderJID == right.senderJID
        }
        // A display name alone cannot distinguish two group participants.
        return left.albumID != nil && left.albumID == right.albumID
    }
}

/// A run of unified-feed items from one chat on one day, shown under that chat's heading.
struct UnifiedFeedSection: Identifiable {
    let items: [ConversationTimelineItem]

    /// The first item anchors the section, so a run keeps its identity as live messages extend it.
    var id: String { items[0].id }
    var primaryMessage: ChatMessage { items[0].primaryMessage }
    var date: Date { items[0].date }
}

enum UnifiedFeedSectionBuilder {
    /// Groups consecutive items from the same chat and day. Each run has one heading, which the feed
    /// pins while the run scrolls, so the chat being read stays visible without scrolling back up.
    static func sections(from items: [ConversationTimelineItem], calendar: Calendar = .current) -> [UnifiedFeedSection] {
        var sections: [UnifiedFeedSection] = []
        var run: [ConversationTimelineItem] = []
        for item in items {
            if let previous = run.last, !continues(previous, item, calendar: calendar) {
                sections.append(UnifiedFeedSection(items: run))
                run.removeAll(keepingCapacity: true)
            }
            run.append(item)
        }
        if !run.isEmpty { sections.append(UnifiedFeedSection(items: run)) }
        return sections
    }

    private static func continues(_ previous: ConversationTimelineItem, _ next: ConversationTimelineItem, calendar: Calendar) -> Bool {
        previous.primaryMessage.contactId == next.primaryMessage.contactId
            && calendar.isDate(previous.date, inSameDayAs: next.date)
    }
}

private extension Character {
    var isEmojiCharacter: Bool {
        let scalars = unicodeScalars
        if scalars.count == 1, let scalar = scalars.first, scalar.isASCII,
           (scalar.properties.numericType != nil || scalar.value == 35 || scalar.value == 42) {
            return false
        }
        return scalars.contains { $0.properties.isEmoji }
    }
}

enum MessageDeliveryState: Equatable, Sendable {
    case none
    case sending
    case uncertain
    case sent
    case delivered
    case read

    var accessibilityLabel: String {
        switch self {
        case .none: ""
        case .sending: "Sending"
        case .uncertain: "Delivery uncertain — check before sending again"
        case .sent: "Sent"
        case .delivered: "Delivered"
        case .read: "Read"
        }
    }
}

private extension String {
    var nilIfBlank: String? { trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : self }
}

