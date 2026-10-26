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
                if session.liveUpdatesReconnecting {
                    Label("Reconnecting live updates… Your messages are saved.", systemImage: "wifi.slash")
                        .font(.caption).padding(10).frame(maxWidth: .infinity)
                        .background(.orange.opacity(0.12))
                }
                #if os(macOS)
                MacConversationHeader(
                    contact: contact,
                    displayName: session.displayName(for: contact),
                    avatarURL: session.avatarURLs[contact.id],
                    localTime: session.localTimeDescription(for: contact.id),
                    translationEnabled: session.savedConversationSettings[contact.id]?.translationEnabled == true,
                    usage: usage,
                    starredOnly: starredOnly,
                    showContactSettings: { showSettings = true },
                    search: { Task { await activateSearch() } },
                    showGallery: { showGallery = true },
                    toggleStarred: { Task { await toggleStarredFilter() } },
                    showCost: { showCost = true },
                    showConversationSettings: { showSettings = true }
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
                            messageBubble(message: session.messages[focusedReplyMessage.contactId]?.first(where: { $0.id == focusedReplyMessage.id }) ?? focusedReplyMessage)
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
            await session.loadAvatar(for: contact.id)
            await session.loadMessages(for: contact.id)
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
