import Foundation

struct MediaResponse: Decodable, Sendable {
    let mediaData: String
    let mimeType: String

    enum CodingKeys: String, CodingKey {
        case mediaData = "media_data"
        case mimeType = "mime_type"
    }
}

struct LinkPreview: Decodable, Equatable, Sendable {
    let url: URL
    let title: String?
    let description: String?
    let imageURL: URL?
    let siteName: String?
    let error: String?

    enum CodingKeys: String, CodingKey {
        case url, title, description, siteName, error
        case imageURL = "imageUrl"
    }
}

struct ConversationSettings: Codable, Equatable, Sendable {
    var replyOnly: Bool
    var languageOverride: String?
    var translationStyle: String?
    var sendOriginalFollowUp: Bool
    var translationEnabled: Bool

    init(languageOverride: String? = nil, translationStyle: String? = nil, sendOriginalFollowUp: Bool = false, translationEnabled: Bool = false, replyOnly: Bool = false) {
        self.replyOnly = replyOnly
        self.languageOverride = languageOverride
        self.translationStyle = translationStyle
        self.sendOriginalFollowUp = sendOriginalFollowUp
        self.translationEnabled = translationEnabled
    }

    private enum CodingKeys: String, CodingKey {
        case languageOverride, translationStyle, sendOriginalFollowUp, translationEnabled, replyOnly
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        replyOnly = try values.decodeIfPresent(Bool.self, forKey: .replyOnly) ?? false
        languageOverride = try values.decodeIfPresent(String.self, forKey: .languageOverride)
        translationStyle = try values.decodeIfPresent(String.self, forKey: .translationStyle)
        sendOriginalFollowUp = try values.decodeIfPresent(Bool.self, forKey: .sendOriginalFollowUp) ?? false
        translationEnabled = try values.decodeIfPresent(Bool.self, forKey: .translationEnabled) ?? false
    }
}

struct ChatTopic: Codable, Equatable, Identifiable, Sendable {
    let id: String
    let contactId: String
    let contactName: String
    let title: String
    let messageCount: Int
    let lastMessageTime: Int64
    var categoryId: String? = nil
}

struct TopicSetting: Codable, Equatable, Sendable {
    let contactId: String
    var enabled: Bool
    var pendingCount: Int
    var failedCount: Int
}

struct MessageTopic: Decodable, Sendable {
    let messageId: String
    let contactId: String
    let revision: Int64
    let title: String?
    let state: String

    var menuLabel: String {
        if let title = title?.nilIfBlank { return "\(title)" }
        return switch state {
        case "pending": "Organising…"
        case "failed": "Needs retry"
        case "off": "Off for this chat"
        default: "Not categorised"
        }
    }
}

struct MessageTopicsResponse: Decodable, Sendable { let topics: [MessageTopic] }

struct TopicCatalog: Codable, Sendable {
    var topics: [ChatTopic]
    var settings: [TopicSetting]
    var available: Bool
    static let empty = TopicCatalog(topics: [], settings: [], available: false)

    func pendingCount(contactID: String? = nil) -> Int {
        settings.filter { $0.enabled && (contactID == nil || $0.contactId == contactID) }.reduce(0) { $0 + $1.pendingCount }
    }

    var unifiedTopics: [ChatTopic] {
        Dictionary(grouping: topics, by: { $0.categoryId ?? $0.id }).map { id, members in
            let first = members.sorted { $0.id < $1.id }[0]
            return ChatTopic(id: id, contactId: "", contactName: "", title: first.title,
                             messageCount: members.reduce(0) { $0 + $1.messageCount },
                             lastMessageTime: members.map(\.lastMessageTime).max() ?? 0,
                             categoryId: first.categoryId)
        }.sorted { $0.lastMessageTime == $1.lastMessageTime ? $0.id < $1.id : $0.lastMessageTime > $1.lastMessageTime }
    }

    var allTopicIDs: Set<String> { Set(topics.map(\.id) + unifiedTopics.map(\.id)) }

    func contactIDs(for topicID: String) -> Set<String> {
        Set(topics.filter { $0.id == topicID || $0.categoryId == topicID }.map(\.contactId))
    }
}

struct TopicImportSummary: Codable, Sendable {
    let days: Int
    let chatCount: Int
    let messageCount: Int
}

struct TopicPage {
    var messages: [ChatMessage] = []
    var hasMore = false
    var error: String?
}

struct OpenAISettings: Codable, Equatable, Sendable {
    var model: String?
    var reasoningEffort: String?
    var available: Bool?
}

struct ChatGPTModel: Codable, Equatable, Sendable, Identifiable {
    var slug: String
    var displayName: String
    var id: String { slug }
}

/// Sign in with ChatGPT state on the server. Text AI uses the ChatGPT plan while signed in.
struct ChatGPTStatus: Codable, Equatable, Sendable {
    var available: Bool?
    var signedIn: Bool
    var email: String?
    var name: String?
    var model: String?
    var models: [ChatGPTModel]?
    var apiKeyConfigured: Bool?
    var revocationConfirmed: Bool?
}

struct ChatGPTSignInStart: Decodable, Sendable {
    let authorizeUrl: String
}

struct PushDeviceRegistration: Encodable, Sendable {
    let installationId: String
    let token: String
    let environment: String
}

struct LiveEvent: Decodable, Sendable {
    var unreadCount: Int? = nil
    var settings: ConversationSettings? = nil
    var connectionState: String? = nil
    var qr: String? = nil
    var phone: String? = nil
    var name: String? = nil
    var data: String? = nil
    let type: String
    let connected: Bool?
    let chatId: String?
    let message: ChatMessage?
    let messageIds: [String]?
    let messageId: String?
    let status: String?
    let progressId: String?
    let stage: String?
    let completed: Int?
    let total: Int?

    static func signal(_ type: String) -> LiveEvent {
        LiveEvent(type: type, connected: nil, chatId: nil, message: nil, messageIds: nil, messageId: nil, status: nil, progressId: nil, stage: nil, completed: nil, total: nil)
    }

    static func reaction(_ message: ChatMessage) -> LiveEvent {
        LiveEvent(type: "reaction", connected: nil, chatId: nil, message: message, messageIds: nil, messageId: nil, status: nil, progressId: nil, stage: nil, completed: nil, total: nil)
    }

    private enum CodingKeys: String, CodingKey {
        case unreadCount = "unread_count"
        case phone, name, data, qr
        case connectionState = "connection_state"
        case type
        case settings
        case connected
        case message
        case status
        case stage, completed, total
        case progressId = "progress_id"
        case chatId = "chat_id"
        case messageIds = "message_ids"
        case messageId = "message_id"
    }
}

struct VoicePreferences: Codable, Sendable {
    var voice: String = "auto"
}

struct TranslatedVoiceNote: Decodable, Sendable, Identifiable {
    let isTranslated: Bool?
    var usesTranslation: Bool { isTranslated ?? true }
    let id: String
    let contactId: String
    let transcript: String
    let translation: String
    let targetLanguage: String
    let voice: String
    let audioData: String
    let originalData: String
    let mimeType: String
    let durationSeconds: Int
    let originalFollowUp: Bool
}

struct VoiceSendResult: Decodable, Sendable {
    let success: Bool
    let messageId: String
    let warning: String?
}

struct VoiceSample: Decodable, Sendable { let audioData: String; let mimeType: String }

private extension String {
    var nilIfBlank: String? { trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : self }
}
