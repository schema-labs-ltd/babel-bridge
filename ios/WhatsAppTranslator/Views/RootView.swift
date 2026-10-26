import SwiftUI

struct RootView: View {
    @Environment(AppSession.self) private var session
    #if DEBUG && os(macOS)
    @State private var showsDemoIcons = ProcessInfo.processInfo.arguments.contains("-demo")
        && ProcessInfo.processInfo.arguments.contains("-demoAppIcons")
    #endif

    var body: some View {
        Group {
            switch session.phase {
            case .restoring:
                LaunchView(label: "Restoring your translator…")
            case .connecting:
                LaunchView(label: "Connecting securely…")
            case .needsConfiguration:
                ConnectionSetupView()
            case .ready:
                Group {
                    if session.requiresWhatsAppLink {
                        WhatsAppLinkView()
                    } else {
                        MainMessagesView()
                            .safeAreaInset(edge: .top, spacing: 0) {
                                if !session.backendStatus.connected {
                                    Label("Reconnecting to WhatsApp…", systemImage: "arrow.triangle.2.circlepath")
                                        .font(.footnote)
                                        .padding(10)
                                        .frame(maxWidth: .infinity)
                                        .background(.regularMaterial)
                                }
                            }
                    }
                }
                .task { await session.monitorWhatsAppConnection() }
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            if ProcessInfo.processInfo.arguments.contains("-demoReadReceipts") {
                Text("Viewed: \(session.viewedReceiptMessageIDs.sorted().joined(separator: ","))")
                    .font(.caption2).lineLimit(2).accessibilityIdentifier("demo-viewed-receipts")
            }
        }
        .animation(.snappy, value: session.phase)
        .alert(session.errorTitle, isPresented: errorPresented) {
            Button("OK") { session.errorMessage = nil }
        } message: {
            Text(session.errorMessage ?? "Something went wrong.")
        }
        #if DEBUG && os(macOS)
        .sheet(isPresented: $showsDemoIcons) {
            NavigationStack {
                AppIconPickerView()
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) {
                            Button("Done") { showsDemoIcons = false }
                        }
                    }
            }
            .frame(width: 560, height: 650)
        }
        #endif
    }

    private var errorPresented: Binding<Bool> {
        Binding(
            get: { session.errorMessage != nil },
            set: { if !$0 { session.errorMessage = nil } }
        )
    }
}

private struct LaunchView: View {
    let label: String

    var body: some View {
        ZStack {
            TranslatorBackdrop()
            VStack(spacing: 18) {
                TranslatorMark(size: 70)
                ProgressView()
                    .controlSize(.large)
                Text(label)
                    .font(.headline)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

private struct MainMessagesView: View {
    @Environment(AppSession.self) private var session
    @State private var replyDraft = UnifiedReplyDraft()
    @State private var sending = false

    var body: some View {
        Group {
            if session.mainTab == .messages {
                UnifiedMessagesView(replyDraft: $replyDraft, sending: $sending)
            } else {
                ChatListView()
            }
        }
    }
}

struct UnifiedReplyDraft {
    var selected: ChatMessage?
    private(set) var isExplicitReply = false
    var isFocused = false
    var drafts: [String: String] = [:]

    var text: String { selected.map { drafts[$0.contactId] ?? "" } ?? "" }

    mutating func updateText(_ value: String, latestMessage: ChatMessage?) {
        if selected == nil {
            guard !value.isEmpty, let latestMessage else { return }
            selected = latestMessage
            isFocused = true
        }
        if let selected { drafts[selected.contactId] = value }
    }

    @discardableResult
    mutating func beginAttachment(latestMessage: ChatMessage?) -> ChatMessage? {
        if selected == nil { selected = latestMessage }
        isFocused = selected != nil
        return selected
    }

    mutating func finishMediaSending(to target: ChatMessage) {
        // A separate photo caption or recording must not erase a text draft.
        if selected?.id == target.id { cancelSelection() }
    }

    mutating func select(_ message: ChatMessage) {
        selected = message
        isExplicitReply = true
        isFocused = true
    }

    mutating func cancelSelection() {
        selected = nil
        isExplicitReply = false
        isFocused = false
    }

    mutating func finishSending(to target: ChatMessage) {
        drafts[target.contactId] = ""
        if selected?.id == target.id { cancelSelection() }
    }
}

struct MainNavigationToolbar: ToolbarContent {
    @Environment(AppSession.self) private var session
    @Binding var showSettings: Bool

    var body: some ToolbarContent {
        // Messages, Chats and Settings are the app's primary destinations, so they stay visible
        // longest when iPhone Duo compresses a vertical bar and sends other items to the overflow menu.
        ToolbarItemGroup(placement: .primaryAction) {
            Button("Messages", systemImage: session.mainTab == .messages ? "text.bubble.fill" : "text.bubble") {
                session.mainTab = .messages
            }
            .accessibilityAddTraits(session.mainTab == .messages ? .isSelected : [])
            .help("Messages")
            Button("Chats", systemImage: session.mainTab == .chats ? "person.2.fill" : "person.2") {
                session.mainTab = .chats
            }
            .accessibilityAddTraits(session.mainTab == .chats ? .isSelected : [])
            .help("Chats")
            #if os(macOS)
            SettingsLink { Label("Settings", systemImage: "gearshape") }
                .help("Settings")
            #else
            Button("Settings", systemImage: "gearshape") { showSettings = true }
            #endif
        }
        .foldableVisibilityPriority(.high)
    }
}

