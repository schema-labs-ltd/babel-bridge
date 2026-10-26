import SwiftUI

/// The topic filter choices: the full timeline, every topic in scope and topic management.
struct TopicFilterMenuItems: View {
    @Environment(AppSession.self) private var session
    let contactID: String?
    @Binding var selection: String?
    @Binding var showManagement: Bool
    /// Whether organising progress and catalog errors are listed too, where no bar shows them.
    var showsStatus = true

    private var topics: [ChatTopic] { session.topics(for: contactID) }
    private var pending: Int { session.topicCatalog.pendingCount(contactID: contactID) }

    var body: some View {
        Button { selection = nil } label: {
            Label("All messages", systemImage: selection == nil ? "checkmark" : "text.bubble")
        }
        if !topics.isEmpty {
            Divider()
            ForEach(topics) { topic in
                Button { selection = topic.id } label: {
                    Label(topic.title, systemImage: selection == topic.id ? "checkmark" : "number")
                }
            }
        }
        if showsStatus, pending > 0 || session.topicCatalogError != nil {
            Divider()
            if pending > 0 {
                Text("Organising \(pending) messages…")
            }
            if let error = session.topicCatalogError {
                Text(error)
                Button("Retry loading topics", systemImage: "arrow.clockwise") { Task { await session.loadTopics() } }
            }
        }
        Divider()
        Button("Manage topics", systemImage: "slider.horizontal.3") { showManagement = true }
            .accessibilityIdentifier("manage-topics")
    }
}

/// The topic filter as a bar item, so the header above the timeline stays clear like a WhatsApp chat.
/// In the vertical bar on iPhone Duo it sits beside the other bar items; in an overflow menu it is a submenu.
struct TopicFilterMenu: View {
    @Environment(AppSession.self) private var session
    let contactID: String?
    @Binding var selection: String?
    @Binding var showManagement: Bool

    private var selected: ChatTopic? { session.topics(for: contactID).first { $0.id == selection } }

    var body: some View {
        Menu {
            TopicFilterMenuItems(contactID: contactID, selection: $selection, showManagement: $showManagement)
        } label: {
            Label(
                "Filter by topic",
                systemImage: selection == nil ? "line.3.horizontal.decrease.circle" : "line.3.horizontal.decrease.circle.fill"
            )
        }
        .accessibilityValue(selected?.title ?? "All messages")
        .accessibilityIdentifier("topic-filter")
    }
}

#if os(macOS)
struct TopicFilterBar: View {
    @Environment(AppSession.self) private var session
    let contactID: String?
    @Binding var selection: String?
    @State private var showManagement = false

    private var topics: [ChatTopic] { session.topics(for: contactID) }
    private var selected: ChatTopic? { topics.first { $0.id == selection } }
    private var pending: Int { session.topicCatalog.pendingCount(contactID: contactID) }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 10) {
                Menu {
                    TopicFilterMenuItems(contactID: contactID, selection: $selection, showManagement: $showManagement, showsStatus: false)
                } label: {
                    Label(selected?.title ?? "All messages", systemImage: "line.3.horizontal.decrease.circle")
                }
                .accessibilityIdentifier("topic-filter")
                Spacer(minLength: 4)
                if pending > 0 {
                    ProgressView().controlSize(.mini)
                    Text("Organising \(pending)").font(.caption2).foregroundStyle(.secondary)
                }
            }
            if let error = session.topicCatalogError {
                HStack {
                    Text(error).font(.caption2).foregroundStyle(.secondary)
                    Button("Retry") { Task { await session.loadTopics() } }.font(.caption)
                }
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 8)
        .background(.regularMaterial)
        .sheet(isPresented: $showManagement) {
            TopicManagementView(contactID: contactID)
                .modifier(ReadReceiptCover())
        }
    }
}
#endif

struct TopicPageControls: View {
    @Environment(AppSession.self) private var session
    let topicID: String
    var body: some View {
        VStack(spacing: 8) {
            if session.topicLoading.contains(topicID) { ProgressView("Loading topic…").font(.caption) }
            if let error = session.topicPages[topicID]?.error {
                Text(error).font(.caption).foregroundStyle(.secondary)
                Button("Retry") { Task { await session.loadTopicMessages(topicID) } }
            }
            if session.topicPages[topicID]?.hasMore == true {
                Button("Load earlier topic messages") { Task { await session.loadTopicMessages(topicID, older: true) } }
                    .disabled(session.topicLoading.contains(topicID))
                    .font(.caption.weight(.semibold)).buttonStyle(.bordered)
            }
        }
    }
}

struct TopicManagementView: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    let contactID: String?
    @State private var saving: Set<String> = []
    @State private var error: String?
    @State private var importing = false
    @State private var importPreview: TopicImportSummary?
    @State private var showImportConfirmation = false
    @State private var importStatus: String?
    private var contacts: [Contact] {
        session.contacts.filter { !$0.isUpdates && (contactID == nil || $0.id == contactID) }
            .sorted { session.displayName(for: $0).localizedStandardCompare(session.displayName(for: $1)) == .orderedAscending }
    }
    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("Follow one discussion at a time in Chats and Messages. Topics always stay separate by chat.")
                        .font(.subheadline)
                }
                Section {
                    ForEach(contacts) { contact in
                        VStack(alignment: .leading, spacing: 6) {
                            Toggle(session.displayName(for: contact), isOn: Binding(
                                get: { session.topicSetting(for: contact.id).enabled },
                                set: { setEnabled($0, for: contact.id) }
                            ))
                            .disabled(importing || saving.contains(contact.id) || (!session.topicCatalog.available && !session.topicSetting(for: contact.id).enabled))
                            .accessibilityIdentifier("topics-enabled-\(contact.id)")
                            if saving.contains(contact.id) { ProgressView().controlSize(.small) }
                            let status = session.topicSetting(for: contact.id)
                            if status.pendingCount > 0 { Text("Organising \(status.pendingCount) messages…").font(.caption).foregroundStyle(.secondary) }
                            if status.failedCount > 0 {
                                HStack {
                                    Text("\(status.failedCount) messages couldn’t be organised.").font(.caption)
                                    Button("Retry") { setEnabled(true, for: contact.id) }.disabled(saving.contains(contact.id))
                                }
                            }
                        }
                    }
                } header: { Text("Organise by topic") } footer: {
                    Text("Off by default. Enabling sends the latest 200 text messages and captions to your configured AI, then processes new messages and edits. This adds AI usage. Existing topics are saved; everyone else continues using their normal WhatsApp chat.")
                }
                Section {
                    Button {
                        importing = true
                        error = nil
                        Task {
                            defer { importing = false }
                            do {
                                importPreview = try await session.topicImportPreview()
                                if importPreview?.messageCount == 0 { importStatus = "No unorganised text messages or captions from the last 7 days are stored on this server." }
                                else { showImportConfirmation = true }
                            } catch { self.error = error.localizedDescription }
                        }
                    } label: {
                        Label("Organise last 7 days from all chats", systemImage: "tray.and.arrow.down")
                    }
                    .accessibilityIdentifier("import-recent-topics")
                    .disabled(importing || !saving.isEmpty || !session.topicCatalog.available)
                    if importing { ProgressView("Preparing import…") }
                    if let importStatus { Text(importStatus).font(.caption).foregroundStyle(.secondary) }
                } header: { Text("Initial import") } footer: {
                    Text("Uses text and captions already stored on the server. Skips messages already organised. Enables topics for the included chats so new messages stay organised, with topics kept separate by chat.")
                }
                if !session.topicCatalog.available {
                    Text("OpenAI is unavailable on this server. Configure it before enabling topics.").font(.caption).foregroundStyle(.secondary)
                }
                if let error { Text(error).foregroundStyle(.red).font(.caption) }
                if let error = session.topicCatalogError {
                    Text(error).font(.caption)
                    Button("Retry loading topics") { Task { await session.loadTopics() } }
                }
            }
            .platformGroupedFormStyle()
            .navigationTitle("Topics")
            .platformInlineNavigationTitle()
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() }.disabled(importing || !saving.isEmpty) } }
            .task { await session.loadTopics() }
            .interactiveDismissDisabled(importing || !saving.isEmpty)
            .confirmationDialog("Organise the last 7 days?", isPresented: $showImportConfirmation, titleVisibility: .visible) {
                Button("Start import") {
                    importing = true
                    Task {
                        defer { importing = false }
                        do {
                            let result = try await session.importRecentTopics()
                            importStatus = "Queued \(result.messageCount) messages across \(result.chatCount) chats. Topics will appear as processing finishes."
                        } catch { self.error = error.localizedDescription }
                    }
                }
            } message: {
                Text("\(importPreview?.messageCount ?? 0) messages across \(importPreview?.chatCount ?? 0) chats will be sent to your configured AI. This adds AI usage and enables topics for those chats.")
            }
        }
        #if os(macOS)
        .frame(minWidth: 440, minHeight: 400)
        #endif
    }
    private func setEnabled(_ enabled: Bool, for id: String) {
        saving.insert(id)
        error = nil
        Task {
            defer { saving.remove(id) }
            do { try await session.setTopicsEnabled(enabled, contactID: id) }
            catch { self.error = error.localizedDescription }
        }
    }
}
