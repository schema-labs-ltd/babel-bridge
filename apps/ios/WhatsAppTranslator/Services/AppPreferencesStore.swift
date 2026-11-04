import Foundation
import Observation
import SwiftUI

enum AppTheme: String, CaseIterable, Codable, Identifiable, Sendable {
    case whatsapp, ocean, sunset, github, dracula, nord, linear, vercel

    var id: String { rawValue }
    var title: String { rawValue.prefix(1).uppercased() + rawValue.dropFirst() }
}

enum AppColorMode: String, CaseIterable, Codable, Identifiable, Sendable {
    case system, light, dark

    var id: String { rawValue }
    var title: String {
        switch self {
        case .system: "Follow system"
        case .light: "Always light"
        case .dark: "Always dark"
        }
    }

    var colorScheme: ColorScheme? {
        switch self {
        case .system: nil
        case .light: .light
        case .dark: .dark
        }
    }
}

enum AppWallpaper: String, CaseIterable, Codable, Identifiable, Sendable {
    case classic, botanical, celestial, ocean, geometric, mountains, travel, cafe, music, origami, garden

    var id: String { rawValue }
    var title: String { self == .cafe ? "Café" : rawValue.prefix(1).uppercased() + rawValue.dropFirst() }
    var assetName: String? { self == .classic ? nil : "Wallpaper-\(rawValue)" }
}

struct ConversationPresentationPreferences: Codable, Equatable, Sendable {
    var nickname: String?
    var timezoneIdentifier: String?

    static let empty = ConversationPresentationPreferences(nickname: nil, timezoneIdentifier: nil)
}

@Observable
final class AppPreferencesStore {
    private struct StoredState: Codable {
        var starredMessageIDs: [String: Set<String>] = [:]
        var conversations: [String: ConversationPresentationPreferences] = [:]
        var theme: AppTheme = .whatsapp
        var colorMode: AppColorMode = .system
        var wallpaper: AppWallpaper? = nil
        // Optional so preferences saved before these existed still decode.
        var mutedChatIDs: Set<String>? = nil
        var archivedChatIDs: Set<String>? = nil
        var markedUnreadChatIDs: Set<String>? = nil
        var hiddenMessageIDs: Set<String>? = nil
    }

    private let defaults: UserDefaults
    private let storageKey = "whatsapp-translator-ios-preferences-v1"
    private var starredMessageIDs: [String: Set<String>]
    private var conversations: [String: ConversationPresentationPreferences]
    /// Chat list organisation kept on this device, like WhatsApp's mute, archive and mark as unread.
    private(set) var mutedChatIDs: Set<String>
    private(set) var archivedChatIDs: Set<String>
    private(set) var markedUnreadChatIDs: Set<String>
    /// Messages removed with WhatsApp's "Delete for me", which only ever affects this device.
    private(set) var hiddenMessageIDs: Set<String>

    var theme: AppTheme { didSet { persist() } }
    var colorMode: AppColorMode { didSet { persist() } }
    var wallpaper: AppWallpaper { didSet { persist() } }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        let state = defaults.data(forKey: storageKey)
            .flatMap { try? JSONDecoder().decode(StoredState.self, from: $0) }
            ?? StoredState()
        starredMessageIDs = state.starredMessageIDs
        conversations = state.conversations
        mutedChatIDs = state.mutedChatIDs ?? []
        archivedChatIDs = state.archivedChatIDs ?? []
        markedUnreadChatIDs = state.markedUnreadChatIDs ?? []
        hiddenMessageIDs = state.hiddenMessageIDs ?? []
        theme = state.theme
        colorMode = state.colorMode
        wallpaper = state.wallpaper ?? .classic
    }

    func isStarred(messageID: String, contactID: String) -> Bool {
        starredMessageIDs[contactID]?.contains(messageID) == true
    }

    func toggleStar(messageID: String, contactID: String) {
        var values = starredMessageIDs[contactID, default: []]
        if values.contains(messageID) { values.remove(messageID) } else { values.insert(messageID) }
        starredMessageIDs[contactID] = values
        persist()
    }

    func hide(messageIDs: [String]) {
        hiddenMessageIDs.formUnion(messageIDs)
        persist()
    }

    /// The messages a chat shows, without those deleted for me.
    func visible(_ messages: [ChatMessage]) -> [ChatMessage] {
        hiddenMessageIDs.isEmpty ? messages : messages.filter { !hiddenMessageIDs.contains($0.id) }
    }

    func isMuted(_ contactID: String) -> Bool { mutedChatIDs.contains(contactID) }
    func isArchived(_ contactID: String) -> Bool { archivedChatIDs.contains(contactID) }
    func isMarkedUnread(_ contactID: String) -> Bool { markedUnreadChatIDs.contains(contactID) }

    /// Muted chats are also kept here as a plain list, which the notification delegate reads off
    /// the main actor to keep a muted chat's notifications quiet while the app is open.
    static let mutedChatsDefaultsKey = "whatsapp-translator-muted-chat-ids"

    func setMuted(_ muted: Bool, for contactID: String) {
        mutedChatIDs = Self.updated(mutedChatIDs, contactID, muted)
        defaults.set(Array(mutedChatIDs), forKey: Self.mutedChatsDefaultsKey)
        persist()
    }

    func setArchived(_ archived: Bool, for contactID: String) {
        archivedChatIDs = Self.updated(archivedChatIDs, contactID, archived)
        persist()
    }

    func setMarkedUnread(_ marked: Bool, for contactID: String) {
        guard markedUnreadChatIDs.contains(contactID) != marked else { return }
        markedUnreadChatIDs = Self.updated(markedUnreadChatIDs, contactID, marked)
        persist()
    }

    private static func updated(_ set: Set<String>, _ contactID: String, _ included: Bool) -> Set<String> {
        var set = set
        if included { set.insert(contactID) } else { set.remove(contactID) }
        return set
    }

    func conversationPreferences(for contactID: String) -> ConversationPresentationPreferences {
        conversations[contactID] ?? .empty
    }

    func setConversationPreferences(_ preferences: ConversationPresentationPreferences, for contactID: String) {
        conversations[contactID] = preferences
        persist()
    }

    func nickname(for contactID: String) -> String? {
        conversations[contactID]?.nickname?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfBlank
    }

    private func persist() {
        let state = StoredState(
            starredMessageIDs: starredMessageIDs,
            conversations: conversations,
            theme: theme,
            colorMode: colorMode,
            wallpaper: wallpaper,
            mutedChatIDs: mutedChatIDs,
            archivedChatIDs: archivedChatIDs,
            markedUnreadChatIDs: markedUnreadChatIDs,
            hiddenMessageIDs: hiddenMessageIDs
        )
        guard let data = try? JSONEncoder().encode(state) else { return }
        defaults.set(data, forKey: storageKey)
    }
}

private extension String {
    var nilIfBlank: String? { isEmpty ? nil : self }
}
