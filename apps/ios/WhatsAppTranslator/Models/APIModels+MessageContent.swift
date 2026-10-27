import Foundation

struct MessageContent: Codable, Hashable, Sendable {
    let editedAtMS: Int64?
    let type: String?
    let body: String?
    let caption: String?
    let mimeType: String?
    let mediaData: String?
    let hasMedia: Bool?
    let fileSize: Int?
    let durationSeconds: Double?
    let isVoiceNote: Bool?
    let fileName: String?
    let isAnimated: Bool?
    let latitude: Double?
    let longitude: Double?
    let name: String?
    let displayName: String?
    let vcard: String?
    let address: String?
    let question: String?
    let options: [String]?
    let rawType: String?
    let emoji: String?
    let targetMessageId: String?
    let albumId: String?
    let albumIndex: Int?
    /// Pixel dimensions from WhatsApp, so the chat can reserve a photo's frame before it loads.
    let width: Int?
    let height: Int?
    let showTranslatedPrimary: Bool?
    let replyContext: ReplyContext?

    enum CodingKeys: String, CodingKey {
        case editedAtMS = "edited_at_ms"
        case type, body, caption, emoji, showTranslatedPrimary, latitude, longitude, name, address, question, options, vcard, width, height
        case mimeType = "mime_type"
        case mediaData = "media_data"
        case hasMedia = "has_media"
        case fileSize = "file_size"
        case durationSeconds = "duration_seconds"
        case isVoiceNote = "is_voice_note"
        case fileName = "file_name"
        case isAnimated = "is_animated"
        case rawType = "raw_type"
        case displayName = "display_name"
        case targetMessageId = "target_message_id"
        case albumId = "album_id"
        case albumIndex = "album_index"
        case replyContext = "reply_context"
    }

    init(
        type: String?,
        body: String?,
        caption: String? = nil,
        mimeType: String? = nil,
        mediaData: String? = nil,
        hasMedia: Bool? = nil,
        fileSize: Int? = nil,
        durationSeconds: Double? = nil,
        isVoiceNote: Bool? = nil,
        fileName: String? = nil,
        isAnimated: Bool? = nil,
        latitude: Double? = nil,
        longitude: Double? = nil,
        name: String? = nil,
        displayName: String? = nil,
        vcard: String? = nil,
        address: String? = nil,
        question: String? = nil,
        options: [String]? = nil,
        rawType: String? = nil,
        emoji: String? = nil,
        targetMessageId: String? = nil,
        albumId: String? = nil,
        albumIndex: Int? = nil,
        width: Int? = nil,
        height: Int? = nil,
        editedAtMS: Int64? = nil,
        showTranslatedPrimary: Bool?,
        replyContext: ReplyContext?
    ) {
        self.type = type
        self.editedAtMS = editedAtMS
        self.body = body
        self.caption = caption
        self.mimeType = mimeType
        self.mediaData = mediaData
        self.hasMedia = hasMedia
        self.fileSize = fileSize
        self.durationSeconds = durationSeconds
        self.isVoiceNote = isVoiceNote
        self.fileName = fileName
        self.isAnimated = isAnimated
        self.latitude = latitude
        self.longitude = longitude
        self.name = name
        self.displayName = displayName
        self.vcard = vcard
        self.address = address
        self.question = question
        self.options = options
        self.rawType = rawType
        self.emoji = emoji
        self.targetMessageId = targetMessageId
        self.albumId = albumId
        self.albumIndex = albumIndex
        self.width = width
        self.height = height
        self.showTranslatedPrimary = showTranslatedPrimary
        self.replyContext = replyContext
    }
}

struct ReplyContext: Codable, Hashable, Sendable {
    let messageId: String?
    let senderName: String?
    let text: String?
}

struct MessageReplyTarget: Equatable, Sendable {
    let messageID: String
    let senderJID: String?
    let senderName: String
    let text: String
}
