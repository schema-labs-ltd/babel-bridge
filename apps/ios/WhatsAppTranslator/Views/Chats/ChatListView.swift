import SwiftUI

struct ChatListView: View {
    @Environment(AppSession.self) private var session
    @Environment(\.translatorPalette) private var palette
    #if os(iOS)
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    #endif
    @State private var showSettings = ProcessInfo.processInfo.arguments.contains("-demoSettings")

    @State private var filter: ChatFilter = .all
    @State private var showArchived = false

    private var listedContacts: [Contact] {
        // Like WhatsApp, a search also finds archived chats.
        let searching = !session.searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        return session.filteredContacts.filter { contact in
            let matchesFilter = filter.includes(contact)
                || (filter == .unread && session.preferences.isMarkedUnread(contact.id))
            return matchesFilter && (searching || session.preferences.isArchived(contact.id) == showArchived)
        }
    }

    private var archivedCount: Int {
        session.contacts.filter { $0.showsInChatList && session.preferences.isArchived($0.id) }.count
    }

    var body: some View {
        @Bindable var session = session

        NavigationSplitView {
            FoldableBarReader { verticalBarEdge in
                sidebarContent(selection: $session.selectedContactID, searchText: $session.searchText)
                #if os(iOS)
                .toolbar {
                    if horizontalSizeClass == .compact {
                        MainNavigationToolbar(showSettings: $showSettings)
                    }
                    ToolbarItem(placement: .topBarLeading) {
                        updatesToolbarButton(inVerticalBar: verticalBarEdge != nil)
                    }
                    .foldableVisibilityPriority(.high)
                }
                #endif
            }
            #if os(macOS)
            .navigationSplitViewColumnWidth(
                min: MacChatLayoutMetrics.minimumSidebarWidth,
                ideal: MacChatLayoutMetrics.idealSidebarWidth,
                max: MacChatLayoutMetrics.maximumSidebarWidth
            )
            #endif
            .navigationTitle("Chats")
            .platformInlineNavigationTitle()
            .platformChatNavigationBackground()
            .overlay {
                if session.contacts.isEmpty {
                    ContentUnavailableView(
                        "No chats yet",
                        systemImage: "message",
                        description: Text("Conversations will appear when your translator receives them.")
                    )
                }
            }
        } detail: {
            if let id = session.selectedContactID,
               let contact = session.contacts.first(where: { $0.id == id }) {
                ConversationView(contact: contact)
                    .platformChatNavigationBackground()
                    .id(id)
                    #if os(iOS)
                    .toolbar { MainNavigationToolbar(showSettings: $showSettings) }
                    #endif
            } else {
                EmptyConversationView()
                    .platformChatNavigationBackground()
                    #if os(iOS)
                    .toolbar { MainNavigationToolbar(showSettings: $showSettings) }
                    #endif
            }
        }
        .navigationSplitViewStyle(.balanced)
        #if os(macOS)
        .toolbar {
            MainNavigationToolbar(showSettings: $showSettings)
            ToolbarItem(placement: .navigation) { updatesToolbarButton(inVerticalBar: false) }
        }
        #endif
        .sheet(isPresented: $showSettings) {
            AppSettingsView()
        }
    }

    /// The Updates shortcut. A vertical bar on iPhone Duo only shows items made of a title and a
    /// symbol, so there the unread count becomes a system badge instead of the drawn one.
    @ViewBuilder
    private func updatesToolbarButton(inVerticalBar: Bool) -> some View {
        if let updates = session.updatesContact {
            if inVerticalBar {
                Button("Updates", systemImage: updates.unreadCount > 0 ? "bell.badge.fill" : "bell") {
                    session.selectedContactID = updates.id
                }
                .badge(updates.unreadCount)
                .accessibilityValue(
                    updates.unreadCount == 0
                        ? "No unread updates"
                        : "\(updates.unreadCount) unread"
                )
                .help("Updates")
            } else {
                Button {
                    session.selectedContactID = updates.id
                } label: {
                    UpdatesToolbarIcon(unreadCount: updates.unreadCount)
                }
                .accessibilityLabel("Updates")
                .accessibilityValue(
                    updates.unreadCount == 0
                        ? "No unread updates"
                        : "\(updates.unreadCount) unread"
                )
                .help("Updates")
            }
        }
    }

    @ViewBuilder
    private func sidebarContent(
        selection: Binding<String?>,
        searchText: Binding<String>
    ) -> some View {
        #if os(macOS)
        VStack(spacing: 0) {
            HStack {
                Text("Chats").font(.largeTitle.bold())
                Spacer()
            }
            .padding(.horizontal, 18)
            .padding(.top, 16)
            .padding(.bottom, 4)
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                TextField("Search chats", text: searchText)
                    .textFieldStyle(.plain)
                if !searchText.wrappedValue.isEmpty {
                    Button("Clear search", systemImage: "xmark.circle.fill") {
                        searchText.wrappedValue = ""
                    }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 11)
            .padding(.vertical, 8)
            .background(.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 9))
            .padding(.horizontal, 10)
            .padding(.vertical, 9)

            filterBar
            contactList(selection: selection)
        }
        #else
        VStack(spacing: 0) {
            filterBar
            contactList(selection: selection)
                .refreshable { await session.refresh(reportTransientFailures: true) }
        }
        .searchable(text: searchText, prompt: "Search chats")
        #endif
    }

    private var filterBar: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) {
                filterButtons
            }
            .fixedSize(horizontal: true, vertical: false)
            VStack(alignment: .leading, spacing: 4) { filterButtons }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 14)
        .padding(.bottom, 6)
    }

    private var filterButtons: some View {
        ForEach(ChatFilter.allCases, id: \.self) { item in
            Button { filter = item } label: {
                Text(item.rawValue)
                    .font(.subheadline.weight(.semibold))
                    .fixedSize()
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                    .foregroundStyle(filter == item ? palette.deepAccent : Color.secondary)
                    .background(filter == item ? palette.accent.opacity(0.14) : Color.primary.opacity(0.045), in: Capsule())
                    .frame(minHeight: 44)
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(filter == item ? .isSelected : [])
        }
    }

    private func contactList(selection: Binding<String?>) -> some View {
        List(selection: selection) {
            archiveRow
            ForEach(listedContacts) { contact in
                // A tagged row drives the split view's selection like a NavigationLink would,
                // without the disclosure chevron WhatsApp's chat list doesn't have.
                ChatRow(
                    contact: contact,
                    displayName: session.displayName(for: contact),
                    draft: session.draftStore.text(for: contact.id),
                    avatarURL: session.avatarURLs[contact.id],
                    isMuted: session.preferences.isMuted(contact.id),
                    isMarkedUnread: session.preferences.isMarkedUnread(contact.id),
                    latestMessage: session.preferences.visible(session.messages[contact.id] ?? []).last { !$0.isReaction },
                    newestIsHidden: newestIsHidden(contact)
                )
                .tag(Optional(contact.id))
                .accessibilityAddTraits(.isButton)
                // WhatsApp's swipes: right for read state and pin, left for mute and archive.
                .swipeActions(edge: .leading, allowsFullSwipe: true) {
                    readStateButton(for: contact)
                    pinButton(for: contact)
                }
                .swipeActions(edge: .trailing, allowsFullSwipe: true) {
                    archiveButton(for: contact)
                    muteButton(for: contact)
                }
                .contextMenu {
                    readStateButton(for: contact)
                    pinButton(for: contact)
                    muteButton(for: contact)
                    archiveButton(for: contact)
                }
                .task { await session.loadAvatar(for: contact.id) }
                #if os(iOS)
                .listRowInsets(EdgeInsets(top: 0, leading: 16, bottom: 0, trailing: 16))
                #endif
                .listRowSeparator(.hidden)
                .listRowBackground(
                    session.selectedContactID == contact.id
                        ? palette.accent.opacity(0.14)
                        : Color.clear
                )
            }
        }
        .listStyle(.plain)
        .onChange(of: session.selectedContactID) { _, id in
            if let id { session.preferences.setMarkedUnread(false, for: id) }
        }
        .overlay {
            if !session.contacts.isEmpty && listedContacts.isEmpty && !showArchived
                && !(archivedCount > 0 && filter == .all && session.searchText.isEmpty) {
                ContentUnavailableView("No matching chats", systemImage: "bubble.left.and.bubble.right", description: Text("Try another filter or search."))
            }
        }
    }
}

extension ChatListView {
    /// WhatsApp's "Archived" row above the chats, and a way back from the archive.
    @ViewBuilder
    fileprivate var archiveRow: some View {
        if showArchived {
            Button { withAnimation(.snappy) { showArchived = false } } label: {
                Label("Back to chats", systemImage: "chevron.left")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(palette.deepAccent)
                    .frame(minHeight: 44)
            }
            .buttonStyle(.plain)
            .listRowSeparator(.hidden)
        } else if archivedCount > 0, filter == .all {
            Button { withAnimation(.snappy) { showArchived = true } } label: {
                HStack(spacing: 16) {
                    Image(systemName: "archivebox")
                        .frame(width: 56)
                        .foregroundStyle(Color.platformSecondaryLabel)
                    Text("Archived")
                        .font(.body.weight(.semibold))
                    Spacer()
                    Text("\(archivedCount)")
                        .font(.caption)
                        .foregroundStyle(Color.platformSecondaryLabel)
                }
                .frame(minHeight: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .listRowSeparator(.hidden)
            .accessibilityLabel("Archived chats, \(archivedCount)")
        }
    }

    /// Whether the newest loaded message is one deleted for me, and at least as new as the one
    /// the server's preview describes, so that preview must not be shown.
    fileprivate func newestIsHidden(_ contact: Contact) -> Bool {
        guard let newest = session.messages[contact.id]?.last(where: { !$0.isReaction }) else { return false }
        return newest.timestamp >= contact.lastMessageTime && session.preferences.hiddenMessageIDs.contains(newest.id)
    }

    fileprivate func readStateButton(for contact: Contact) -> some View {
        let unread = contact.unreadCount > 0 || session.preferences.isMarkedUnread(contact.id)
        return Button {
            if unread {
                Task { await session.markChatRead(contact) }
            } else {
                session.preferences.setMarkedUnread(true, for: contact.id)
            }
        } label: {
            Label(unread ? "Mark as read" : "Mark as unread", systemImage: unread ? "message" : "message.badge")
        }
        .tint(.blue)
    }

    fileprivate func pinButton(for contact: Contact) -> some View {
        Button {
            Task { await session.togglePin(contact) }
        } label: {
            Label(contact.pinnedAt == nil ? "Pin" : "Unpin", systemImage: contact.pinnedAt == nil ? "pin" : "pin.slash")
        }
        .tint(palette.accent)
    }

    fileprivate func muteButton(for contact: Contact) -> some View {
        let muted = session.preferences.isMuted(contact.id)
        return Button {
            session.preferences.setMuted(!muted, for: contact.id)
        } label: {
            Label(muted ? "Unmute" : "Mute", systemImage: muted ? "speaker.wave.2" : "speaker.slash")
        }
        .tint(.gray)
    }

    fileprivate func archiveButton(for contact: Contact) -> some View {
        let archived = session.preferences.isArchived(contact.id)
        return Button {
            withAnimation(.snappy) { session.preferences.setArchived(!archived, for: contact.id) }
        } label: {
            Label(archived ? "Unarchive" : "Archive", systemImage: archived ? "tray.and.arrow.up" : "archivebox")
        }
        .tint(.indigo)
    }
}

private struct UpdatesToolbarIcon: View {
    @Environment(\.translatorPalette) private var palette
    #if os(iOS)
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    #endif
    let unreadCount: Int

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Image(systemName: unreadCount > 0 ? "bell.fill" : "bell")
                .frame(width: 28, height: 28)

            if unreadCount > 0 {
                Text(unreadCount > 99 ? "99+" : "\(unreadCount)")
                    .font(.system(size: 9, weight: .bold, design: .rounded))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 4)
                    .frame(minWidth: 16, minHeight: 16)
                    .background(palette.accent, in: Capsule())
                    .offset(x: 7, y: -5)
            }
        }
        .padding(.trailing, unreadCount > 0 ? 7 : 0)
    }
}

private struct EmptyConversationView: View {
    @Environment(\.translatorPalette) private var palette
    #if os(iOS)
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    #endif

    var body: some View {
        ZStack {
            palette.chatBackground.ignoresSafeArea()
            VStack(spacing: 18) {
                TranslatorMark(size: 78)
                Text("Choose a chat")
                    .font(.largeTitle.bold())
                Text("Translation is off by default.\nEnable Translate messages in a conversation’s settings when you need it.")
                    .multilineTextAlignment(.center)
                    .foregroundStyle(.secondary)
            }
            .padding(32)
        }
    }
}

enum ChatFilter: String, CaseIterable {
    case all = "All"
    case unread = "Unread"
    case groups = "Groups"

    func includes(_ contact: Contact) -> Bool {
        switch self {
        case .all: true
        case .unread: contact.unreadCount > 0
        case .groups: contact.isGroup
        }
    }
}
