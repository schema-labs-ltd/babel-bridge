import SwiftUI

struct ConversationView: View {
    @Environment(AppSession.self) var session
    @Environment(\.translatorPalette) private var palette
    #if os(iOS)
    @Environment(\.horizontalSizeClass) var horizontalSizeClass
    #endif
    let contact: Contact
    @State var draft = ""
    @State var replyTarget: MessageReplyTarget?
    @State private var focusedReplyMessage: ChatMessage?
    @State var showSettings = ProcessInfo.processInfo.arguments.contains("-demoConversationSettings")
    @State var showCost = ProcessInfo.processInfo.arguments.contains("-demoCost")
    @State var showSearch = ProcessInfo.processInfo.arguments.contains("-demoSearch")
    @State var showGallery = false
    @State var messageSearch = ""
    @State var starredOnly = false
    @State private var selectedTopicID: String?
    @State var showTopicManagement = false
    @State var usage: UsageSummary?
    private var selectedTopic: ChatTopic? { session.topics(for: contact.id).first { $0.id == selectedTopicID } }

    #if os(iOS)
    var conversationTitleButton: some View {
        Button { showSettings = true } label: {
            HStack(spacing: 9) {
                ContactAvatar(contact: contact, url: session.avatarURLs[contact.id], size: 38)
                VStack(alignment: .leading, spacing: 1) {
                    Text(session.displayName(for: contact))
                        .font(.headline)
                        .lineLimit(1)
                    Text(conversationHeaderSubtitle)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .platformCompactControlTypography()
        }
        .buttonStyle(.plain)
        .accessibilityHint("Opens nickname, timezone and translation settings")
    }

    @ViewBuilder
    var conversationBarActions: some View {
        Button("Gallery", systemImage: "photo.on.rectangle") { showGallery = true }
            .accessibilityIdentifier("open-chat-gallery")
        Menu("Conversation", systemImage: "ellipsis") { conversationActions }
    }

    @ViewBuilder
    private var conversationActions: some View {
        Button("Search messages", systemImage: "magnifyingglass") {
            Task { await activateSearch() }
        }
        .keyboardShortcut("f", modifiers: .command)
        Button(
            starredOnly ? "Show all messages" : "Show starred only",
            systemImage: starredOnly ? "star.fill" : "star"
        ) {
            Task { await toggleStarredFilter() }
        }
        // The topic filter sits with the starred filter, which keeps the bar and the header clear.
        TopicFilterMenu(contactID: contact.id, selection: $selectedTopicID, showManagement: $showTopicManagement)
        Divider()
        Button("Conversation cost", systemImage: "dollarsign.circle") { showCost = true }
        Button("Conversation settings", systemImage: "slider.horizontal.3") { showSettings = true }
    }
    #endif

    var messages: [ChatMessage] {
        if let selectedTopicID { return session.topicPages[selectedTopicID]?.messages ?? [] }
        return session.messages[contact.id] ?? []
    }
    var activePhotoSend: PhotoSendProgress? {
        session.photoSendProgress.values
            .filter { $0.contactID == contact.id }
            .sorted { $0.id > $1.id }
            .first
    }
    private var visibleMessages: [ChatMessage] {
        let query = messageSearch.trimmingCharacters(in: .whitespacesAndNewlines)
        return messages.filter { message in
            let starMatches = !starredOnly || session.preferences.isStarred(messageID: message.id, contactID: contact.id)
            let searchMatches = query.isEmpty
                || message.displayText.localizedCaseInsensitiveContains(query)
                || (message.alternateText?.localizedCaseInsensitiveContains(query) ?? false)
                || (message.senderName?.localizedCaseInsensitiveContains(query) ?? false)
            return starMatches && searchMatches
        }
    }

    var body: some View {
        FoldableLayoutReader { layout in
            conversationContent(layout: layout)
        }
    }

    var messageTimeline: some View {
        ScrollViewReader { proxy in
            ScrollView {
                #if os(macOS)
                VStack(spacing: 6) {
                    messageTimelineContent
                }
                .padding(.horizontal, MacChatLayoutMetrics.timelineHorizontalPadding / 2)
                .padding(.vertical, 14)
                #else
                LazyVStack(spacing: 6) {
                    messageTimelineContent
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 14)
                #endif
            }
            .platformDismissesKeyboard()
            .platformSwipeDownDismissesKeyboard()
            .defaultScrollAnchor(.bottom)
            .onChange(of: messages.count) {
                guard messageSearch.isEmpty, !starredOnly else { return }
                withAnimation(.snappy) { proxy.scrollTo("conversation-bottom", anchor: .bottom) }
            }
        }
    }

    var platformNavigationTitle: String {
        #if os(macOS)
        "Babel Bridge"
        #else
        ""
        #endif
    }

    private var conversationHeaderSubtitle: String {
        let translation = session.savedConversationSettings[contact.id]?.translationEnabled == true ? "Translation on" : "Translation off"
        return session.localTimeDescription(for: contact.id).map { "\(translation) · \($0)" } ?? translation
    }

    @ViewBuilder
    private var messageTimelineContent: some View {
        if let selectedTopicID {
            TopicPageControls(topicID: selectedTopicID)
        }
        if selectedTopicID == nil, session.messageHistoryHasMore[contact.id] == true, messageSearch.isEmpty, !starredOnly {
            Button("Load earlier messages") {
                Task { await session.loadMessages(for: contact.id, older: true) }
            }
            .font(.caption.weight(.semibold))
            .buttonStyle(.bordered)
            .padding(.vertical, 10)
        }

        if visibleMessages.isEmpty {
            ContentUnavailableView(
                starredOnly ? "No starred messages" : "No messages found",
                systemImage: starredOnly ? "star" : "magnifyingglass",
                description: Text(starredOnly ? "Touch and hold a message to star it." : "Try another search.")
            )
            .padding(.top, 80)
        }

        let items = timelineItems
        ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
            if shouldShowDate(before: index) {
                DatePill(date: item.date)
                    .padding(.vertical, 10)
            }
            switch item {
            case let .message(message):
                messageBubble(message: message)
                    .id(message.id)
                    .modifier(MessageReadVisibility(messages: [message], enabled: canRecordViews))
                    .task {
                        await loadTimelineMedia(for: [message])
                        await prefetchImages(before: index, in: items)
                    }
            case let .photoAlbum(album):
                messageBubble(message: album.primaryMessage, albumMessages: album.messages)
                    .id(item.id)
                    .modifier(MessageReadVisibility(messages: Array(album.messages.prefix(PhotoGalleryLayout.previewLimit)), enabled: canRecordViews))
                    .task(id: album.messages.map(\.id)) {
                        await loadTimelineMedia(for: Array(album.messages.prefix(PhotoGalleryLayout.previewLimit)))
                        await prefetchImages(before: index, in: items)
                    }
            }
        }
        // Album grouping can replace the last image's ID; keep a stable scroll target.
        Color.clear.frame(height: 1).id("conversation-bottom")
    }

    private var canRecordViews: Bool {
        session.mainTab == .chats && session.selectedContactID == contact.id
            && !showSettings && !showCost && !showGallery && focusedReplyMessage == nil
    }

    private var timelineItems: [ConversationTimelineItem] {
        let visibleIDs = Set(visibleMessages.map(\.id))
        return ConversationTimelineBuilder.items(from: messages).flatMap { item in
            ConversationTimelineBuilder.items(from: item.messages.filter { visibleIDs.contains($0.id) })
        }
    }

    func messageBubble(message: ChatMessage, albumMessages: [ChatMessage] = []) -> some View {
        MessageBubble(
                message: message,
                isStarred: session.preferences.isStarred(messageID: message.id, contactID: contact.id),
                isBusy: session.activeMessageActionIDs.contains(message.id),
                image: session.messageImages[message.id],
                mediaURL: session.messageMediaURLs[message.id],
                mediaIsLoading: session.mediaLoadingIDs.contains(message.id),
                mediaFailed: session.mediaErrorIDs.contains(message.id),
                linkPreviews: message.extractedURLs.compactMap { session.linkPreviews[$0] },
                reply: { selectReply(message) },
                translate: { Task { await session.translate(message) } },
                aiReply: { generateAIReply(to: message) },
                toggleStar: { session.preferences.toggleStar(messageID: message.id, contactID: contact.id) },
                react: { emoji in Task { await session.react(to: message, emoji: emoji) } },
                retryMedia: { Task { await session.retryMedia(for: message) } },
                albumMessages: albumMessages,
                albumImages: session.messageImages,
                albumLoadingIDs: session.mediaLoadingIDs,
                albumFailedIDs: session.mediaErrorIDs,
                retryAlbumMedia: { albumMessage in Task { await session.retryMedia(for: albumMessage) } },
                albumReply: selectReply,
                albumAIReply: generateAIReply
            )
    }

    /// Loads the photos and stickers just above a bubble as it appears, so they are usually ready
    /// before they scroll into view when reading back through history.
    private func prefetchImages(before index: Int, in items: [ConversationTimelineItem]) async {
        let upcoming = items[max(0, index - Self.imagePrefetchDistance)..<min(index, items.count)].reversed()
        for item in upcoming {
            guard !Task.isCancelled else { return }
            let messages: [ChatMessage] = switch item {
            case let .message(message): [message]
            case let .photoAlbum(album): Array(album.messages.prefix(PhotoGalleryLayout.previewLimit))
            }
            for message in messages where message.mediaKind == .image || message.mediaKind == .sticker {
                await session.loadMedia(for: message)
            }
        }
    }

    private static let imagePrefetchDistance = 6

    private func loadTimelineMedia(for messages: [ChatMessage]) async {
        for message in messages {
            await session.loadMedia(for: message)
            for url in message.extractedURLs { await session.loadLinkPreview(for: url) }
        }
    }

    private func shouldShowDate(before index: Int) -> Bool {
        guard index > 0 else { return true }
        return !Calendar.current.isDate(timelineItems[index - 1].date, inSameDayAs: timelineItems[index].date)
    }

    private func selectReply(_ message: ChatMessage) {
        guard !session.sendingContactIDs.contains(contact.id) else { return }
        replyTarget = session.replyTarget(for: message)
        focusedReplyMessage = message
    }

    func cancelReply() {
        replyTarget = nil
        focusedReplyMessage = nil
    }

    func send() {
        let value = draft
        let reply = replyTarget
        Task {
            if await session.send(text: value, to: contact.id, reply: reply) {
                draft = ""
                cancelReply()
            }
        }
    }

    func sendImages(_ images: [OutgoingImage], _ caption: String?) -> Bool {
        let reply = replyTarget
        if session.startPhotoSend(images, caption: caption, to: contact.id, reply: reply) {
            cancelReply()
            return true
        }
        return false
    }

    private func generateAIReply(to message: ChatMessage) {
        Task {
            guard let suggestion = await session.generateAIReply(to: message) else { return }
            selectReply(message)
            draft = suggestion
        }
    }

    func activateSearch() async {
        await session.loadAllMessages(for: contact.id)
        showSearch = true
    }

    func toggleStarredFilter() async {
        if starredOnly {
            starredOnly = false
            return
        }
        await session.loadAllMessages(for: contact.id)
        starredOnly = true
    }
}

