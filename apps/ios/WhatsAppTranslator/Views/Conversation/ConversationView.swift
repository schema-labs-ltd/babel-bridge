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
    @State var focusedReplyMessage: ChatMessage?
    @State var showSettings = ProcessInfo.processInfo.arguments.contains("-demoConversationSettings")
    @State var showCost = ProcessInfo.processInfo.arguments.contains("-demoCost")
    @State var showSearch = ProcessInfo.processInfo.arguments.contains("-demoSearch")
    @State var showGallery = false
    @State var messageSearch = ""
    @State var starredOnly = false
    @State var selectedTopicID: String?
    @State var showTopicManagement = false
    @State var usage: UsageSummary?
    /// The quoted message a tap on a reply scrolled to; it is briefly highlighted.
    @State var replyJumpTarget: String?
    @State var isAtBottom = true
    /// Messages that arrived while the user was reading further up, shown on the scroll-down button.
    @State var unseenCount = 0
    @State var loadingOlder = false
    /// The first message that was unread when the chat opened, where WhatsApp's "unread messages" bar goes.
    @State var unreadMarkerID: String?
    @State var unreadMarkerCount = 0
    @State var pendingScrollID: String?
    var selectedTopic: ChatTopic? { session.topics(for: contact.id).first { $0.id == selectedTopicID } }

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
                VStack(spacing: 0) {
                    messageTimelineContent
                }
                .padding(.horizontal, MacChatLayoutMetrics.timelineHorizontalPadding / 2)
                .padding(.vertical, 14)
                #else
                // Day headers stay pinned at the top while their day scrolls past, as in WhatsApp.
                LazyVStack(spacing: 0, pinnedViews: [.sectionHeaders]) {
                    messageTimelineContent
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 14)
                #endif
            }
            .platformDismissesKeyboard()
            .platformSwipeDownDismissesKeyboard()
            .defaultScrollAnchor(.bottom)
            // Keyed on the newest message, so loading older history or a full search never counts as new.
            .onChange(of: messages.last?.id) { oldID, newID in
                guard messageSearch.isEmpty, !starredOnly, !loadingOlder, oldID != nil, newID != nil else { return }
                // Like WhatsApp, a new message only pulls you down if you were already at the bottom
                // or sent it yourself; otherwise the scroll-down button counts it.
                if isAtBottom || messages.last?.isFromMe == true {
                    withAnimation(.snappy) { proxy.scrollTo("conversation-bottom", anchor: .bottom) }
                } else {
                    unseenCount += 1
                }
            }
            .onScrollGeometryChange(for: Bool.self) { geometry in
                geometry.contentOffset.y + geometry.containerSize.height >= geometry.contentSize.height - 120
            } action: { _, atBottom in
                isAtBottom = atBottom
                if atBottom { unseenCount = 0 }
            }
            .onScrollGeometryChange(for: Bool.self) { geometry in
                geometry.contentOffset.y + geometry.contentInsets.top < 300
            } action: { _, nearTop in
                if nearTop { Task { await loadOlderMessages(proxy: proxy) } }
            }
            .onChange(of: pendingScrollID) { _, id in
                guard let id else { return }
                proxy.scrollTo(id, anchor: .top)
                pendingScrollID = nil
            }
            .overlay(alignment: .bottomTrailing) {
                if !isAtBottom {
                    ScrollToBottomButton(unseenCount: unseenCount) {
                        withAnimation(.snappy) { proxy.scrollTo("conversation-bottom", anchor: .bottom) }
                    }
                    .padding(.trailing, 14)
                    .padding(.bottom, 10)
                    .transition(.scale.combined(with: .opacity))
                }
            }
            .animation(.snappy, value: isAtBottom)
            .onChange(of: replyJumpTarget) { _, target in
                guard let target else { return }
                withAnimation(.snappy) { proxy.scrollTo(target, anchor: .center) }
                Task {
                    try? await Task.sleep(for: .seconds(1.4))
                    withAnimation(.easeOut(duration: 0.4)) { if replyJumpTarget == target { replyJumpTarget = nil } }
                }
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
        // Like WhatsApp, a slow reconnect shows in the header, not as a banner over the chat.
        if session.showsConnectingIndicator { return "Connecting…" }
        let translation = session.savedConversationSettings[contact.id]?.translationEnabled == true ? "Translation on" : "Translation off"
        return session.localTimeDescription(for: contact.id).map { "\(translation) · \($0)" } ?? translation
    }

    @ViewBuilder
    private var messageTimelineContent: some View {
        if let selectedTopicID {
            TopicPageControls(topicID: selectedTopicID)
        }
        // Earlier history loads by itself as you scroll near the top, as in WhatsApp.
        if loadingOlder {
            ProgressView()
                .controlSize(.small)
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
        ForEach(TimelineGrouping.days(items)) { day in
            Section {
                ForEach(day.indices.map { (index: $0, item: items[$0]) }, id: \.item.id) { entry in
                    if let unreadMarkerID, entry.item.messages.contains(where: { $0.id == unreadMarkerID }) {
                        UnreadMessagesDivider(count: unreadMarkerCount)
                            .padding(.top, 10)
                    }
                    timelineRow(entry.item, at: entry.index, in: items)
                        .padding(.top, TimelineGrouping.startsRun(items, at: entry.index) && entry.index != day.indices.lowerBound ? 8 : 2)
                }
            } header: {
                DatePill(date: day.date)
                    .padding(.vertical, 8)
            }
        }
        // Album grouping can replace the last image's ID; keep a stable scroll target.
        Color.clear.frame(height: 1).id("conversation-bottom")
    }

    @ViewBuilder
    private func timelineRow(_ item: ConversationTimelineItem, at index: Int, in items: [ConversationTimelineItem]) -> some View {
        let startsRun = TimelineGrouping.startsRun(items, at: index)
        switch item {
        case let .message(message):
            messageBubble(message: message, startsRun: startsRun)
                .id(message.id)
                .modifier(MessageReadVisibility(messages: [message], enabled: canRecordViews))
                .task {
                    await loadTimelineMedia(for: [message])
                    await prefetchImages(before: index, in: items)
                }
        case let .photoAlbum(album):
            messageBubble(message: album.primaryMessage, albumMessages: album.messages, startsRun: startsRun)
                .id(item.id)
                .modifier(MessageReadVisibility(messages: Array(album.messages.prefix(PhotoGalleryLayout.previewLimit)), enabled: canRecordViews))
                .task(id: album.messages.map(\.id)) {
                    await loadTimelineMedia(for: Array(album.messages.prefix(PhotoGalleryLayout.previewLimit)))
                    await prefetchImages(before: index, in: items)
                }
        }
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

    func messageBubble(message: ChatMessage, albumMessages: [ChatMessage] = [], startsRun: Bool = true) -> some View {
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
                albumAIReply: generateAIReply,
                startsRun: startsRun,
                isHighlighted: replyJumpTarget == message.id,
                openQuotedMessage: { id in Task { await jumpToQuotedMessage(id) } }
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

    /// Scrolls to and highlights the message a reply quotes, loading the whole conversation first
    /// when the quoted message is older than what's loaded.
    func jumpToQuotedMessage(_ id: String) async {
        if !messages.contains(where: { $0.id == id }), selectedTopicID == nil {
            await session.loadAllMessages(for: contact.id)
        }
        guard messages.contains(where: { $0.id == id }) else { return }
        let target = timelineItems.first { item in item.messages.contains { $0.id == id } }?.id ?? id
        messageSearch = ""
        starredOnly = false
        replyJumpTarget = target
    }

    /// Loads the next page of history and keeps the message that was at the top in place.
    func loadOlderMessages(proxy: ScrollViewProxy) async {
        guard !loadingOlder, selectedTopicID == nil, messageSearch.isEmpty, !starredOnly,
              session.messageHistoryHasMore[contact.id] == true,
              let anchor = timelineItems.first?.id else { return }
        loadingOlder = true
        await session.loadMessages(for: contact.id, older: true)
        proxy.scrollTo(anchor, anchor: .top)
        loadingOlder = false
    }

    /// Places WhatsApp's "unread messages" bar above the first message that was unread when the
    /// chat opened, and starts the chat there instead of at the bottom.
    func markUnread(openingCount: Int) {
        guard openingCount > 0, unreadMarkerID == nil,
              let marker = UnreadMarker.firstUnreadID(in: messages, unreadCount: openingCount) else { return }
        unreadMarkerID = marker
        unreadMarkerCount = openingCount
        pendingScrollID = timelineItems.first { item in item.messages.contains { $0.id == marker } }?.id ?? marker
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

