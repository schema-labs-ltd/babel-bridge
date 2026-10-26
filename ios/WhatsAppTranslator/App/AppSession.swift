import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

@MainActor
@Observable
final class AppSession {
    enum Phase: Equatable {
        case restoring
        case needsConfiguration
        case connecting
        case ready
    }

    var phase: Phase = .restoring
    var configuration: ServerConfiguration?
    var backendStatus = BackendStatus(connected: false, phone: nil, name: nil)
    var whatsAppQRCode: String?
    var whatsAppLinkError: String?
    var connectionRevision = UUID()
    var requiresWhatsAppLink: Bool {
        !backendStatus.connected && (backendStatus.connectionState == "linking_required" || whatsAppQRCode != nil)
    }
    var contacts: [Contact] = []
    var avatarURLs: [String: URL] = [:]
    enum MainTab { case messages, chats }
    var mainTab: MainTab = .messages
    var feedByID: [String: ChatMessage] = [:]
    var feedHasMore = false
    var feedHasLoaded = false
    var feedLoading = false
    var feedError: String?
    var feedCursor: ChatMessage?
    var feedGeneration = UUID()
    var feedEventsDuringLoad: Set<String> = []
    var messages: [String: [ChatMessage]] = [:]
    var topicCatalog = TopicCatalog.empty
    var topicPages: [String: TopicPage] = [:]
    var topicLoading: Set<String> = []
    var topicCatalogError: String?
    var topicCatalogRequest = UUID()
    var topicAccountEpoch = UUID()
    var topicRevision = UUID()
    var topicReloadAfterLoad: Set<String> = []
    var demoTopicCatalog = TopicCatalog.empty
    var demoTopicPages: [String: TopicPage] = [:]

    var messageTopicStates: [String: MessageTopic] = [:]
    var messageTopicRequests: [String: UUID] = [:]
    var messageTopicFailures: Set<String> = []

    struct ReactionTarget: Hashable {
        let contactID: String
        let messageID: String
    }
    // Keep removals too: an older page or replay must not restore an old emoji.
    var reactionEvents: [ReactionTarget: [String: MessageReactionState]] = [:]

    var messageHistoryHasMore: [String: Bool] = [:]
    var selectedContactID: String?
    var searchText = ""
    var errorTitle = "Something went wrong"
    var errorMessage: String?
    var isRefreshing = false
    var sendingContactIDs: Set<String> = []
    var photoSendProgress: [String: PhotoSendProgress] = [:]
    var voicePreferenceRevision = 0
    var messageToneRevision = 0
    var demoGlobalTone = MessageTone.default
    var demoConversationTones: [String: MessageTone] = [:]
    var voiceReadyIDs: Set<String> = []
    var activeMessageActionIDs: Set<String> = []
    var messageImages: [String: PlatformImage] = [:]
    var messageMediaURLs: [String: URL] = [:]
    /// Sizes of photos and stickers seen before, for messages the server sent without dimensions.
    private(set) var rememberedImageSizes: [String: CGSize] = [:]
    var mediaLoadingIDs: Set<String> = []
    var mediaErrorIDs: Set<String> = []
    var loadingCompleteHistoryContactIDs: Set<String> = []
    var linkPreviews: [URL: LinkPreview] = [:]

    let draftStore: DraftStore
    let preferences: AppPreferencesStore
    let appIcons = AppIconController()
    var readReceiptCovers: Set<UUID> = []
    private(set) var viewedReceiptMessageIDs: Set<String> = []
    var pendingReadReceipts: [String: Set<String>] = [:]
    var readReceiptTask: Task<Void, Never>?

    let api: APIClient
    let credentials: CredentialStore
    let cache: ChatCacheStore
    let mediaCache: MediaCacheStore
    let demoMode: Bool
    var liveUpdatesReconnecting = false
    var recoveryTask: Task<Void, Never>?
    var recoveryNeedsAnotherPass = false
    let demoConversationMode: Bool
    var avatarRequests: Set<String> = []
    var mediaRequests: Set<String> = []
    static let maximumGalleryThumbnailRequests = 4
    var activeGalleryThumbnailRequests = 0
    var galleryThumbnailWaiters: [CheckedContinuation<Void, Never>] = []
    var galleryDemoMessages: [ChatMessage] = []
    var linkPreviewRequests: Set<URL> = []
    var isConnecting = false

    init(
        api: APIClient = APIClient(),
        credentials: CredentialStore = CredentialStore(),
        cache: ChatCacheStore = .shared,
        mediaCache: MediaCacheStore = .shared,
        draftStore: DraftStore = DraftStore(),
        preferences: AppPreferencesStore = AppPreferencesStore(),
        demoMode: Bool = ProcessInfo.processInfo.arguments.contains("-demo")
    ) {
        self.api = api
        self.credentials = credentials
        self.cache = cache
        self.mediaCache = mediaCache
        self.draftStore = draftStore
        self.preferences = preferences
        self.demoMode = demoMode
        self.demoConversationMode = ProcessInfo.processInfo.arguments.contains("-demoConversation")
    }

    var filteredContacts: [Contact] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        let ordered = Self.orderedContacts(contacts.filter(\.showsInChatList))
        guard !query.isEmpty else { return ordered }
        return ordered.filter {
            displayName(for: $0).localizedCaseInsensitiveContains(query)
                || ($0.lastMessagePreview?.localizedCaseInsensitiveContains(query) ?? false)
        }
    }

    var updatesContact: Contact? {
        contacts.first(where: \.showsAsUpdatesToolbarItem)
    }

    static func orderedContacts(_ contacts: [Contact]) -> [Contact] {
        contacts.sorted { first, second in
            switch (first.pinnedAt, second.pinnedAt) {
            case let (firstPinned?, secondPinned?):
                if firstPinned != secondPinned { return firstPinned < secondPinned }
            case (.some, .none):
                return true
            case (.none, .some):
                return false
            case (.none, .none):
                break
            }
            if first.lastMessageTime != second.lastMessageTime {
                return first.lastMessageTime > second.lastMessageTime
            }
            return first.id < second.id
        }
    }

    func displayName(for contact: Contact) -> String {
        if contact.isUpdates { return contact.displayName }
        return preferences.nickname(for: contact.id) ?? contact.displayName
    }

    func localTimeDescription(for contactID: String) -> String? {
        guard let identifier = preferences.conversationPreferences(for: contactID).timezoneIdentifier,
              let timezone = TimeZone(identifier: identifier) else { return nil }
        var format = Date.FormatStyle(date: .omitted, time: .shortened)
        format.timeZone = timezone
        let city = identifier.split(separator: "/").last?.replacingOccurrences(of: "_", with: " ") ?? identifier
        return "\(Date().formatted(format)) in \(city)"
    }

    func replyTarget(for message: ChatMessage) -> MessageReplyTarget {
        let target = message.replyTarget
        guard message.isFromMe, let senderJID = ownSenderJID else { return target }
        return MessageReplyTarget(
            messageID: target.messageID,
            senderJID: senderJID,
            senderName: target.senderName,
            text: target.text
        )
    }

    /// Delays between quiet retries when a refresh hits a dropped or unavailable connection.
    var transientRefreshRetryDelays: [Duration] = [.seconds(1), .seconds(3)]

    private(set) var savedConversationSettings: [String: ConversationSettings] = [:]
    var conversationSettingsRevision = 0

    func presentError(_ title: String, _ error: Error) {
        AppLog.ui.error("\(title, privacy: .public): \(AppLog.kind(of: error), privacy: .public) \(error.localizedDescription)")
        showError(title, error.localizedDescription)
    }

    func presentError(_ title: String, _ message: String) {
        AppLog.ui.error("\(title, privacy: .public): \(message)")
        showError(title, message)
    }

    private func showError(_ title: String, _ message: String) {
        errorTitle = title
        errorMessage = message
    }

    nonisolated static func isExpectedCancellation(_ error: Error) -> Bool {
        if error is CancellationError {
            return true
        }
        let error = error as NSError
        if error.domain == NSURLErrorDomain, error.code == URLError.cancelled.rawValue {
            return true
        }
        guard let underlying = error.userInfo[NSUnderlyingErrorKey] as? Error else {
            return false
        }
        return isExpectedCancellation(underlying)
    }

    /// Network failures that usually clear up by themselves, such as a connection dropped
    /// while the device was locked or a request made before the network came back.
    nonisolated static func isTransientNetworkError(_ error: Error) -> Bool {
        let error = error as NSError
        if error.domain == NSURLErrorDomain {
            let transient: Set<URLError.Code> = [
                .networkConnectionLost, .notConnectedToInternet, .timedOut,
                .cannotConnectToHost, .cannotFindHost, .dnsLookupFailed,
                .dataNotAllowed, .internationalRoamingOff, .callIsActive,
                .backgroundSessionWasDisconnected,
            ]
            if transient.contains(URLError.Code(rawValue: error.code)) { return true }
        }
        guard let underlying = error.userInfo[NSUnderlyingErrorKey] as? Error else {
            return false
        }
        return isTransientNetworkError(underlying)
    }

}

