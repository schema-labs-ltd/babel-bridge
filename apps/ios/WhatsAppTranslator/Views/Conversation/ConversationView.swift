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
    @State var replyJump: ReplyJump?
    @State var isAtBottom = true
    /// Messages that arrived while the user was reading further up, shown on the scroll-down button.
    @State var unseenCount = 0
    @State var loadingOlder = false
    @State var isNearTop = false
    /// The chat's unread count when it opened, before reading clears it.
    @State var openingUnreadCount: Int?
    /// The messages already viewed here when the chat opened, so later views don't move the bar.
    @State var openingViewedIDs: Set<String> = []
    /// The chat's latest message time when it opened; later arrivals are never among the unread.
    @State var openingLastMessageTime: Int64?
    @State var newestPageLoaded = false
    /// The first message that was unread when the chat opened, where WhatsApp's "unread messages" bar goes.
    @State var unreadMarkerID: String?
    @State var unreadMarkerCount = 0
    @State var pendingScrollID: String?
    /// Older pages loaded so the unread bar can reach the first unread message.
    @State var unreadHistoryPages = 0
    @State var quoteJumpToken = UUID()
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
            // Keyed on the chat's newest second, so loading older history, a full search or switching
            // topic pages never counts as new, while a same-second arrival sorted before the last row does.
            .onChange(of: TimelineTail(session.messages[contact.id] ?? [])) { old, new in
                markUnread()
                // Older pages never change the newest second, so a change while one loads is a real arrival.
                guard selectedTopicID == nil, messageSearch.isEmpty, !starredOnly,
                      let old, new != nil else { return }
                // Like WhatsApp, a new message only pulls you down if you were already at the bottom
                // or sent it yourself; otherwise the scroll-down button counts it. A highlighted quote
                // keeps its place until the highlight fades.
                let arrived = old.arrivals(in: messages) { session.liveMessageIDs.contains($0) }
                if (isAtBottom && replyJump == nil) || arrived.contains(where: \.isFromMe) {
                    withAnimation(.snappy) { proxy.scrollTo("conversation-bottom", anchor: .bottom) }
                } else {
                    unseenCount += arrived.filter { !$0.isFromMe && !$0.isReaction }.count
                }
            }
            // A refresh can replace the cached list and drop the marked row; place the bar again.
            .onChange(of: session.messages[contact.id]?.first?.id) { markUnread() }
            .onScrollGeometryChange(for: Bool.self) { geometry in
                // The composer and keyboard sit in the bottom inset, so they don't count as distance.
                geometry.contentOffset.y + geometry.containerSize.height - geometry.contentInsets.bottom
                    >= geometry.contentSize.height - 120
            } action: { _, atBottom in
                isAtBottom = atBottom
                if atBottom { unseenCount = 0 }
            }
            .onScrollGeometryChange(for: Bool.self) { geometry in
                geometry.contentOffset.y + geometry.contentInsets.top < 300
            } action: { _, nearTop in
                isNearTop = nearTop
                if nearTop { Task { await loadOlderMessages() } }
            }
            .onChange(of: pendingScrollID) { _, id in
                guard let id else { return }
                // Just below the top, so the pinned day header doesn't cover the unread bar.
                proxy.scrollTo(id, anchor: UnitPoint(x: 0.5, y: 0.12))
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
            .task { markUnread() }
            .onChange(of: replyJump) { _, jump in
                guard let jump else { return }
                withAnimation(.snappy) { proxy.scrollTo(jump.itemID, anchor: .center) }
                Task {
                    try? await Task.sleep(for: .seconds(1.4))
                    withAnimation(.easeOut(duration: 0.4)) { if replyJump == jump { replyJump = nil } }
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
        // Earlier history loads by itself as you scroll near the top, as in WhatsApp. The button
        // stays as a fallback for a chat too short to scroll, and for VoiceOver.
        if loadingOlder {
            ProgressView()
                .controlSize(.small)
                .padding(.vertical, 10)
        } else if selectedTopicID == nil, session.messageHistoryHasMore[contact.id] == true, messageSearch.isEmpty, !starredOnly {
            Button("Load earlier messages") {
                Task { await loadOlderMessages() }
            }
            .font(.caption.weight(.semibold))
            .buttonStyle(.borderless)
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
                            .id(UnreadMessagesDivider.scrollID)
                    }
                    timelineRow(entry.item, at: entry.index, in: items, firstOfDay: entry.index == day.indices.lowerBound)
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
    private func timelineRow(_ item: ConversationTimelineItem, at index: Int, in items: [ConversationTimelineItem], firstOfDay: Bool) -> some View {
        let startsRun = TimelineGrouping.startsRun(items, at: index)
        let highlighted = replyJump?.itemID == item.id
        Group {
            switch item {
            case let .message(message):
                messageBubble(message: message, startsRun: startsRun, isHighlighted: highlighted)
                    .id(message.id)
                    .modifier(MessageReadVisibility(messages: [message], enabled: canRecordViews))
                    .task {
                        await loadTimelineMedia(for: [message])
                        await prefetchImages(before: index, in: items)
                    }
            case let .photoAlbum(album):
                messageBubble(message: album.primaryMessage, albumMessages: album.messages, startsRun: startsRun, isHighlighted: highlighted)
                    .id(item.id)
                    .modifier(MessageReadVisibility(messages: Array(album.messages.prefix(PhotoGalleryLayout.previewLimit)), enabled: canRecordViews))
                    .task(id: album.messages.map(\.id)) {
                        await loadTimelineMedia(for: Array(album.messages.prefix(PhotoGalleryLayout.previewLimit)))
                        await prefetchImages(before: index, in: items)
                    }
            }
        }
        .padding(.top, startsRun && !firstOfDay ? 8 : 2)
    }

    private var canRecordViews: Bool {
        session.mainTab == .chats && session.selectedContactID == contact.id
            && !showSettings && !showCost && !showGallery && focusedReplyMessage == nil
    }

    var timelineItems: [ConversationTimelineItem] {
        let visibleIDs = Set(visibleMessages.map(\.id))
        return ConversationTimelineBuilder.items(from: messages).flatMap { item in
            ConversationTimelineBuilder.items(from: item.messages.filter { visibleIDs.contains($0.id) })
        }
    }

    func messageBubble(
        message: ChatMessage,
        albumMessages: [ChatMessage] = [],
        startsRun: Bool = true,
        isHighlighted: Bool = false,
        opensQuotes: Bool = true
    ) -> some View {
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
                isHighlighted: isHighlighted,
                openQuotedMessage: opensQuotes ? { id in Task { await jumpToQuotedMessage(id) } } : nil
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

