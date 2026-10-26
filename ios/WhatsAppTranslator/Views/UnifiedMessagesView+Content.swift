import SwiftUI

extension UnifiedMessagesView {
    func messagesContent(layout: FoldableLayoutContext) -> some View {
        let sections = UnifiedFeedSectionBuilder.sections(from: ConversationTimelineBuilder.items(from: displayedMessages))
        // On a partially open iPhone Duo the feed reads on one pane and the composer rests on the other.
        let arrangement = FoldAwareChatLayout.arrangement(for: layout.division, in: layout.size)
        let bottomSafeArea = layout.safeAreaInsets.bottom
        return NavigationStack {
            ZStack {
                ChatWallpaper()
                    .blur(radius: replyDraft.isFocused ? 8 : 0)
                ScrollViewReader { proxy in
                    ScrollView {
                        // Each chat's heading is a pinned section header, so the person being read stays
                        // in view at the top of the feed while their messages scroll beneath it.
                        LazyVStack(spacing: 6, pinnedViews: [.sectionHeaders]) {
                            if let selectedTopicID { TopicPageControls(topicID: selectedTopicID) }
                            if selectedTopicID == nil, session.feedHasMore {
                                Button("Load earlier messages") {
                                    let anchor = session.unifiedMessages.first?.id
                                    Task {
                                        await session.loadFeed(older: true)
                                        if let anchor { proxy.scrollTo(anchor, anchor: .top) }
                                    }
                                }.disabled(session.feedLoading)
                            }
                            if selectedTopicID == nil, session.feedLoading { ProgressView().padding() }
                            if selectedTopicID == nil, let error = session.feedError {
                                VStack(spacing: 8) {
                                    Text(error).font(.caption).foregroundStyle(.secondary)
                                    Button("Retry") { Task { await session.loadFeed() } }
                                }.padding()
                            }
                            if displayedMessages.isEmpty && !session.feedLoading && session.feedError == nil {
                                ContentUnavailableView(selectedTopicID == nil ? "All your messages, together" : "No messages in this topic", systemImage: "text.bubble", description: Text(selectedTopicID == nil ? "Messages from your chats will appear here. Swipe a message to choose where your reply goes." : "New messages will appear here after they’re organised. All messages remains available."))
                            }
                            ForEach(sections) { section in
                                Section {
                                    ForEach(section.items) { item in
                                        bubble(item.primaryMessage, albumMessages: item.messages.count > 1 ? item.messages : [])
                                            .modifier(MessageReadVisibility(messages: Array(item.messages.prefix(PhotoGalleryLayout.previewLimit)), enabled: session.mainTab == .messages && !showSettings && settingsContact == nil && !replyDraft.isFocused))
                                            .id(item.id)
                                            .task(id: item.messages.map(\.id)) {
                                                for photo in item.messages.prefix(PhotoGalleryLayout.previewLimit) {
                                                    await session.loadMedia(for: photo)
                                                    for url in photo.extractedURLs { await session.loadLinkPreview(for: url) }
                                                }
                                            }
                                    }
                                } header: {
                                    source(section.primaryMessage)
                                }
                            }
                            Color.clear.frame(height: 1).id("feed-bottom")
                                .onAppear { atBottom = true }
                                .onDisappear { atBottom = false }
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 14)
                        .frame(maxWidth: 900)
                        .frame(maxWidth: .infinity)
                    }
                    #if os(iOS)
                    // Keep scrolled messages behind the native navigation-bar material.
                    .scrollClipDisabled()
                    #endif
                    .platformDismissesKeyboard()
                    .platformSwipeDownDismissesKeyboard()
                    .defaultScrollAnchor(.bottom)
                    .refreshable {
                        await session.loadTopics()
                        if let selectedTopicID { await session.loadTopicMessages(selectedTopicID) }
                        else { await session.loadFeed() }
                    }
                    .onChange(of: displayedMessages.last?.id) { _, _ in
                        if atBottom { withAnimation { proxy.scrollTo("feed-bottom", anchor: .bottom) } }
                    }
                }
                .environment(\.chatBubbleMaxWidth, arrangement.bubbleMaxWidth)
                .blur(radius: replyDraft.isFocused ? 8 : 0)
                .opacity(replyDraft.isFocused ? 0.35 : 1)
                .allowsHitTesting(!replyDraft.isFocused)
                .accessibilityHidden(replyDraft.isFocused)
                if replyDraft.isFocused, let selected = replyDraft.selected {
                    FocusedReplyOverlay(destination: name(selected), isSending: sending, cancel: cancelReply) {
                        bubble(session.unifiedMessages.first(where: { $0.id == selected.id && $0.contactId == selected.contactId }) ?? selected)
                            .modifier(ViewedMessageTask(messages: [selected], enabled: !showSettings && settingsContact == nil))
                    }
                }
            }
            #if os(macOS)
            // The Mac keeps the title as its window name. On iPhone and iPad the filled Messages
            // button already shows where you are, so the feed runs up to the bar with no heading.
            .navigationTitle("Messages")
            #endif
            .platformInlineNavigationTitle()
            .platformChatNavigationBackground()
            .toolbar {
                MainNavigationToolbar(showSettings: $showSettings)
                #if os(iOS)
                // The filter is a bar item, so the title sits on the wallpaper with no bar of its own
                // beneath it, and iPhone Duo keeps the filter in its vertical bar beside Messages and Chats.
                ToolbarItem(placement: .topBarLeading) {
                    TopicFilterMenu(contactID: nil, selection: $selectedTopicID, showManagement: $showTopicManagement)
                }
                .foldableVisibilityPriority(.high)
                #endif
            }
            .platformTopSafeAreaBar {
                #if os(macOS)
                TopicFilterBar(contactID: nil, selection: $selectedTopicID)
                #else
                if let selectedTopic {
                    FilterPill(title: selectedTopic.title, systemImage: "number") { selectedTopicID = nil }
                        .padding(.vertical, 8)
                }
                #endif
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                composer
                    .padding(.bottom, ComposerLayout.bottomAdjustment(for: bottomSafeArea))
                    .foldAwareComposerPane(arrangement)
            }
            .sheet(isPresented: $showSettings) { AppSettingsView() }
            .sheet(item: $settingsContact) { ConversationSettingsView(contact: $0) }
            .sheet(isPresented: $showTopicManagement) {
                TopicManagementView(contactID: nil)
                    .modifier(ReadReceiptCover())
            }
        }
        .task {
            if !session.feedHasLoaded { await session.loadFeed() }
            await session.loadTopics()
        }
        .task(id: selectedTopicID) {
            if let selectedTopicID { await session.loadTopicMessages(selectedTopicID) }
        }
        .onChange(of: selectedTopicID) { _, _ in cancelReply() }
        .onChange(of: session.topicCatalog.topics) { _, topics in
            if let selectedTopicID, !session.topics().contains(where: { $0.id == selectedTopicID }) { self.selectedTopicID = nil }
        }
        .onChange(of: session.mainTab) { _, tab in
            if tab != .messages { composerFocused = false }
        }
    }

    func name(_ message: ChatMessage) -> String {
        if let contact = session.contacts.first(where: { $0.id == message.contactId }) {
            return session.displayName(for: contact)
        }
        return message.contactName ?? message.contactPhone ?? message.contactId
    }

    private func source(_ message: ChatMessage) -> some View {
        let contact = session.contacts.first(where: { $0.id == message.contactId }) ?? Contact(
            id: message.contactId, name: name(message), phone: message.contactPhone,
            type: message.chatType.lowercased(), lastMessageTime: message.timestamp,
            unreadCount: 0, pinnedAt: nil, lastMessagePreview: nil
        )
        // Glass capsules keep the heading legible while it is pinned over the messages scrolling under it.
        return HStack(spacing: 8) {
            Button {
                session.selectedContactID = message.contactId
                session.mainTab = .chats
            } label: {
                HStack(spacing: 7) {
                    ContactAvatar(contact: contact, url: session.avatarURLs[message.contactId], size: 26)
                        .accessibilityHidden(true)
                    Text(name(message)).fontWeight(.semibold).lineLimit(1)
                    Image(systemName: "chevron.right").font(.system(size: 9, weight: .bold))
                }
                .font(.caption)
                .foregroundStyle(palette.deepAccent)
                .padding(.leading, 4)
                .padding(.trailing, 11)
                .padding(.vertical, 4)
                .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            .translatorGlass(in: Capsule())
            .contextMenu {
                Button("Conversation settings", systemImage: "slider.horizontal.3") { settingsContact = contact }
            }
            .accessibilityLabel("Open chat: \(name(message))")
            Spacer(minLength: 8)
            Text(message.date, format: .dateTime.day().month(.abbreviated))
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .translatorGlass(in: Capsule())
        }
        .padding(.top, 6)
        .padding(.bottom, 2)
        .task { await session.loadAvatar(for: message.contactId) }
    }

    private func select(_ message: ChatMessage) {
        guard !sending else { return }
        replyDraft.select(message)
        composerFocused = true
    }

    private func bubble(_ message: ChatMessage, albumMessages: [ChatMessage] = []) -> some View {
        MessageBubble(
            message: message,
            isStarred: session.preferences.isStarred(messageID: message.id, contactID: message.contactId),
            isBusy: session.activeMessageActionIDs.contains(message.id),
            image: session.messageImages[message.id],
            mediaURL: session.messageMediaURLs[message.id],
            mediaIsLoading: session.mediaLoadingIDs.contains(message.id),
            mediaFailed: session.mediaErrorIDs.contains(message.id),
            linkPreviews: message.extractedURLs.compactMap { session.linkPreviews[$0] },
            reply: { select(message) },
            translate: { Task { await session.translate(message) } },
            aiReply: {
                select(message)
                Task {
                    if let suggestion = await session.generateAIReply(to: message), replyDraft.selected?.id == message.id, !sending {
                        replyDraft.drafts[message.contactId] = suggestion
                    }
                }
            },
            toggleStar: { session.preferences.toggleStar(messageID: message.id, contactID: message.contactId) },
            react: { emoji in Task { await session.react(to: message, emoji: emoji) } },
            retryMedia: { Task { await session.retryMedia(for: message) } },
            albumMessages: albumMessages, albumImages: session.messageImages,
            albumLoadingIDs: session.mediaLoadingIDs, albumFailedIDs: session.mediaErrorIDs,
            retryAlbumMedia: { message in Task { await session.retryMedia(for: message) } },
            albumReply: select,
            albumAIReply: { photo in
                select(photo)
                Task {
                    if let suggestion = await session.generateAIReply(to: photo), replyDraft.selected?.id == photo.id, !sending {
                        replyDraft.drafts[photo.contactId] = suggestion
                    }
                }
            }
        )
    }
}
