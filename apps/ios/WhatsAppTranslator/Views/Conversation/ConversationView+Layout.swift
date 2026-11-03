import SwiftUI

extension ConversationView {
    func conversationContent(layout: FoldableLayoutContext) -> some View {
        // On a partially open iPhone Duo the timeline reads on one pane and the composer rests on the other.
        let arrangement = FoldAwareChatLayout.arrangement(for: layout.division, in: layout.size)
        let bottomSafeArea = layout.safeAreaInsets.bottom
        #if os(iOS)
        // iPhone Duo presents bars vertically on the outer display and in landscape on the inner display.
        // A custom principal item never joins a vertical bar, so the header then lives in the content,
        // exactly as it does at compact widths.
        let showsInlineHeader = horizontalSizeClass == .compact || layout.usesVerticalBars
        #endif
        return ZStack {
            ChatWallpaper()
                .blur(radius: focusedReplyMessage == nil ? 0 : 8)
            VStack(spacing: 0) {
                #if os(macOS)
                MacConversationHeader(
                    contact: contact,
                    displayName: session.displayName(for: contact),
                    avatarURL: session.avatarURLs[contact.id],
                    subtitle: conversationHeaderSubtitle,
                    translationEnabled: session.savedConversationSettings[contact.id]?.translationEnabled == true,
                    usage: usage,
                    starredOnly: starredOnly,
                    showContactSettings: { showSettings = true },
                    search: { Task { await activateSearch() } },
                    showGallery: { showGallery = true },
                    toggleStarred: { Task { await toggleStarredFilter() } },
                    showCost: { showCost = true },
                    showConversationSettings: { showSettings = true },
                    toggleTranslation: toggleTranslation
                )
                TopicFilterBar(contactID: contact.id, selection: $selectedTopicID)
                #endif
                if showSearch {
                    HStack(spacing: 10) {
                        Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                        TextField("Search messages", text: $messageSearch)
                            .platformUncapitalizedInput()
                        Button("Close search", systemImage: "xmark.circle.fill") {
                            messageSearch = ""
                            showSearch = false
                        }
                        .labelStyle(.iconOnly)
                        .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .translatorGlass(in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                    .padding(.horizontal, 12)
                    .padding(.top, 8)
                }
                if starredOnly {
                    FilterPill(title: "Starred messages", systemImage: "star.fill") {
                        starredOnly = false
                    }
                    .padding(.top, 8)
                }
                #if os(iOS)
                if let selectedTopic {
                    FilterPill(title: selectedTopic.title, systemImage: "number") {
                        selectedTopicID = nil
                    }
                    .padding(.top, 8)
                }
                #endif
                if session.loadingCompleteHistoryContactIDs.contains(contact.id) {
                    HStack(spacing: 7) {
                        ProgressView().controlSize(.small)
                        Text("Loading the full conversation…")
                    }
                    .font(.caption.weight(.semibold))
                    .padding(.horizontal, 12)
                    .padding(.vertical, 7)
                    .translatorGlass(in: Capsule())
                    .padding(.top, 8)
                }
                ZStack {
                    messageTimeline
                        .environment(\.chatBubbleMaxWidth, arrangement.bubbleMaxWidth)
                        .blur(radius: focusedReplyMessage == nil ? 0 : 8)
                        .opacity(focusedReplyMessage == nil ? 1 : 0.35)
                        .allowsHitTesting(focusedReplyMessage == nil)
                        .accessibilityHidden(focusedReplyMessage != nil)
                    if let focusedReplyMessage {
                        FocusedReplyOverlay(destination: session.displayName(for: contact), isSending: session.sendingContactIDs.contains(contact.id), cancel: cancelReply) {
                            messageBubble(message: session.messages[focusedReplyMessage.contactId]?.first(where: { $0.id == focusedReplyMessage.id }) ?? focusedReplyMessage, opensQuotes: false)
                                .modifier(ViewedMessageTask(messages: [focusedReplyMessage], enabled: !showSettings && !showCost && !showGallery))
                        }
                    }
                }
                #if os(macOS)
                composerPane(arrangement: arrangement, bottomSafeArea: bottomSafeArea)
                #endif
            }
        }
        #if os(iOS)
        // The composer floats in the bottom safe area, so messages scroll behind its glass controls
        // instead of stopping at its top edge, as they already do in the unified Messages feed.
        .safeAreaInset(edge: .bottom, spacing: 0) {
            composerPane(arrangement: arrangement, bottomSafeArea: bottomSafeArea)
        }
        #endif
        .navigationTitle(platformNavigationTitle)
        .task(id: contact.id) { _ = try? await session.conversationSettings(for: contact.id) }
        .task {
            let arguments = ProcessInfo.processInfo.arguments
            guard arguments.contains("-demoPhotoSendProgress") || arguments.contains("-demoPhotoTransferProgress") else { return }
            session.photoSendProgress["demo-photo-send"] = PhotoSendProgress(
                id: "demo-photo-send", contactID: contact.id,
                stage: arguments.contains("-demoPhotoTransferProgress") ? .transferring : .sending,
                completed: 4, total: 10, error: nil
            )
        }
        .platformInlineNavigationTitle()
        #if os(iOS)
        .platformChatNavigationBackground()
        .platformTopSafeAreaBar {
            if showsInlineHeader {
                conversationTitleButton
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                    .modifier(InlineHeaderBackground())
            }
        }
        #endif
        .toolbar {
            #if os(iOS)
            if !showsInlineHeader {
                ToolbarItem(placement: .principal) { conversationTitleButton }
            }
            // Gallery and the conversation menu are their own group, apart from Messages, Chats and Settings.
            // In a vertical bar on iPhone Duo a leading item gets its own glass button above the
            // destinations, as the Messages feed's topic filter does; a trailing one joins their capsule.
            // In a horizontal bar they stay at the right of the header, split off by a spacer.
            if layout.usesVerticalBars {
                ToolbarItemGroup(placement: .topBarLeading) { conversationBarActions }
                    .foldableVisibilityPriority(.high)
            } else {
                if #available(iOS 26.0, *) {
                    ToolbarSpacer(.fixed, placement: .topBarTrailing)
                }
                ToolbarItemGroup(placement: .topBarTrailing) { conversationBarActions }
                    .foldableVisibilityPriority(.high)
            }
            #endif
        }
        .task {
            draft = ProcessInfo.processInfo.arguments.contains("-demo") ? "" : session.draftStore.text(for: contact.id)
            // Opening a chat however it was reached clears a manual "mark as unread".
            session.preferences.setMarkedUnread(false, for: contact.id)
            // The picture may need a download; don't hold the messages back for it.
            async let avatar: Void = session.loadAvatar(for: contact.id)
            // Only a page that actually arrived can stand in for the rows reaching the chat's latest time.
            if await session.loadMessages(for: contact.id) { newestPageLoaded = true }
            markUnread()
            await avatar
            await session.loadTopics()
            usage = try? await session.conversationUsage(for: contact.id)
        }
        .onChange(of: draft) { _, newValue in
            session.draftStore.save(newValue, for: contact.id)
        }
        .task(id: selectedTopicID) {
            if let selectedTopicID { await session.loadTopicMessages(selectedTopicID) }
        }
        .onChange(of: selectedTopicID) { _, _ in cancelReply() }
        .onChange(of: session.topicCatalog.topics) { _, topics in
            if let selectedTopicID, !topics.contains(where: { $0.id == selectedTopicID && $0.contactId == contact.id }) { self.selectedTopicID = nil }
        }
        .sheet(isPresented: $showSettings) {
            ConversationSettingsView(contact: contact)
                .foldableTrailingSheetPlacement()
        }
        .sheet(isPresented: $showCost) {
            ConversationCostView(contact: contact)
                .foldableTrailingSheetPlacement()
        }
        #if os(iOS)
        .sheet(isPresented: $showTopicManagement) {
            TopicManagementView(contactID: contact.id)
                .modifier(ReadReceiptCover())
                // Inside a conversation the sheet belongs beside that conversation, not over the hinge.
                .foldableTrailingSheetPlacement()
        }
        .fullScreenCover(isPresented: $showGallery) { ChatMediaGallery(contact: contact).environment(session) }
        #else
        .sheet(isPresented: $showGallery) { ChatMediaGallery(contact: contact).environment(session) }
        #endif
    }

    private func composerPane(arrangement: FoldAwareChatLayout.Arrangement, bottomSafeArea: CGFloat) -> some View {
        VStack(spacing: 0) {
            if let progress = activePhotoSend {
                PhotoSendProgressView(progress: progress)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
            }
            ComposerView(
                contactID: contact.id,
                text: $draft,
                reply: replyTarget,
                showReplyPreview: focusedReplyMessage == nil,
                isSending: session.sendingContactIDs.contains(contact.id),
                cancelReply: cancelReply,
                sendImages: sendImages,
                send: send
            )
            .padding(.bottom, ComposerLayout.bottomAdjustment(for: bottomSafeArea))
        }
        .foldAwareComposerPane(arrangement)
    }
}

extension ConversationView {
    /// Loads the next page of history. The timeline's bottom anchor keeps what you're reading in
    /// place as rows arrive above it; if you're still near the top afterwards, the next page follows.
    func loadOlderMessages() async {
        guard !loadingOlder, selectedTopicID == nil, messageSearch.isEmpty, !starredOnly,
              session.messageHistoryHasMore[contact.id] == true else { return }
        let oldestBefore = session.messages[contact.id]?.first?.id
        loadingOlder = true
        await session.loadMessages(for: contact.id, older: true)
        loadingOlder = false
        markUnread()
        // Only follow on when the page actually added older messages; a failed request leaves the
        // cursor where it was, and the "Load earlier messages" button stays for a manual retry.
        guard !Task.isCancelled, session.messages[contact.id]?.first?.id != oldestBefore else { return }
        if isNearTop, session.messageHistoryHasMore[contact.id] == true { await loadOlderMessages() }
    }

    /// Places WhatsApp's "unread messages" bar above the first message that was unread when the
    /// chat opened, and starts the chat there. It waits for the newest page, or a cache that already
    /// reaches the chat's latest message, so a stale cache can't put the bar in the wrong place. (A
    /// trailing reaction moves the chat's time but isn't a row, so the rows alone can't show that.)
    func markUnread() {
        let openingCount = openingUnreadCount ?? contact.unreadCount
        if openingUnreadCount == nil {
            openingUnreadCount = openingCount
            openingViewedIDs = session.viewedReceiptMessageIDs
            openingLastMessageTime = contact.lastMessageTime
        }
        let boundary = openingLastMessageTime ?? contact.lastMessageTime
        let loaded = session.messages[contact.id] ?? []
        if let marker = unreadMarkerID, !loaded.contains(where: { $0.id == marker }) { unreadMarkerID = nil }
        // Wait for any page that is loading; whichever load finishes calls this again.
        guard !loadingOlder, openingCount > 0, unreadMarkerID == nil, selectedTopicID == nil,
              let newest = loaded.last, newestPageLoaded || newest.timestamp >= boundary else { return }
        let viewed = openingViewedIDs
        // Messages that arrived after the chat opened were never part of the unread count.
        let opening = loaded.filter { $0.timestamp <= boundary }
        // The first page may not reach back to the first unread message, so load a few more pages
        // first; past that the bar goes above the oldest loaded unread message.
        if UnreadMarker.candidates(in: opening, viewed: viewed).count < openingCount,
           session.messageHistoryHasMore[contact.id] == true, unreadHistoryPages < 5 {
            unreadHistoryPages += 1
            let oldestBefore = loaded.first?.id
            // Set before the task starts, so another call in this update waits instead of loading too.
            loadingOlder = true
            Task {
                await session.loadMessages(for: contact.id, older: true)
                loadingOlder = false
                // A failed page stops here rather than retrying; the bar then uses what is loaded.
                if session.messages[contact.id]?.first?.id == oldestBefore { unreadHistoryPages = 5 }
                markUnread()
            }
            return
        }
        guard let marker = UnreadMarker.firstUnreadID(in: opening, unreadCount: openingCount, viewed: viewed) else { return }
        unreadMarkerID = marker
        unreadMarkerCount = openingCount
        pendingScrollID = UnreadMessagesDivider.scrollID
    }

    func jumpToQuotedMessage(_ id: String) async {
        // A newer tap wins: an older one that is still loading history must not scroll back.
        let token = UUID()
        quoteJumpToken = token
        // A topic only shows its own page, so leave it when the quoted message is not in it.
        if selectedTopicID != nil, !messages.contains(where: { $0.id == id }) {
            selectedTopicID = nil
        }
        if !messages.contains(where: { $0.id == id }) {
            await session.loadAllMessages(for: contact.id)
            // An earlier tap or a search may already be loading the history, in which case the
            // call above returns at once; wait for that load rather than give up on the quote.
            while session.loadingCompleteHistoryContactIDs.contains(contact.id), quoteJumpToken == token {
                try? await Task.sleep(for: .milliseconds(100))
            }
        }
        guard quoteJumpToken == token, messages.contains(where: { $0.id == id }) else { return }
        // Clear filters first: the timeline only holds rows that match them.
        messageSearch = ""
        starredOnly = false
        guard let target = timelineItems.first(where: { item in item.messages.contains { $0.id == id } })?.id else { return }
        replyJump = ReplyJump(itemID: target)
    }
}

extension ConversationView {
    var conversationHeaderSubtitle: String {
        // Like WhatsApp, a slow reconnect shows in the header, not as a banner over the chat.
        if session.showsConnectingIndicator { return "Connecting…" }
        if let localTime = session.localTimeDescription(for: contact.id) { return localTime }
        return contact.isGroup ? ConversationHeaderSummary.groupMembers(in: session.messages[contact.id] ?? []) : "tap here for contact info"
    }

    var translationOn: Bool { session.savedConversationSettings[contact.id]?.translationEnabled == true }

    func toggleTranslation() {
        // Apply what the button offered when tapped, even if the settings were still loading, and
        // never carry one server's settings over to another picked meanwhile.
        guard !savingTranslation else { return }
        let enable = !translationOn
        let epoch = session.topicAccountEpoch
        savingTranslation = true
        Task {
            defer { savingTranslation = false }
            do {
                var settings = try await session.conversationSettings(for: contact.id)
                guard session.topicAccountEpoch == epoch, settings.translationEnabled != enable else { return }
                settings.translationEnabled = enable
                try await session.saveConversationSettings(settings, for: contact.id)
            } catch {
                guard session.topicAccountEpoch == epoch, !AppSession.isExpectedCancellation(error) else { return }
                session.presentError("Couldn’t change translation", error)
            }
        }
    }
}
