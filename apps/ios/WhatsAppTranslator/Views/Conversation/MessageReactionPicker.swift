import SwiftUI

@MainActor
enum MessageReactionChoices {
    static let quick = ["👍", "❤️", "😂", "😮", "🙏"]
    static let all = quick + ["😢", "✅", "🥰", "😍", "😊", "😁", "😅", "🤣", "😎", "🤔", "🙌", "👏", "🔥", "🎉", "💯", "💪", "🤝", "😴", "👀", "🤗", "😇", "😬", "🤷", "💔", "✨", "🥳", "😡", "😭", "👋", "🫶", "🤞", "👌", "🚀", "🎂", "☕", "🌷"]
    private static var icons: [String: Image] = [:]

    static func icon(_ emoji: String) -> Image {
        if let cached = icons[emoji] { return cached }
        let renderer = ImageRenderer(content: Text(emoji).font(.system(size: 28)).frame(width: 36, height: 36))
        renderer.scale = 3
        #if os(iOS)
        let image = renderer.uiImage.map { Image(uiImage: $0).renderingMode(.original) }
        #else
        let image = renderer.nsImage.map { Image(nsImage: $0).renderingMode(.original) }
        #endif
        let result = image ?? Image(systemName: "face.smiling")
        icons[emoji] = result
        return result
    }

    static func isSingleEmoji(_ value: String) -> Bool {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count == 1, let character = trimmed.first else { return false }
        let scalars = character.unicodeScalars
        if scalars.count == 1, let scalar = scalars.first, scalar.isASCII { return false }
        return scalars.contains { $0.properties.isEmoji }
    }
}

struct MessageReactionPicker: View {
    let selected: String?
    let choose: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var customEmoji = ""

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 20) {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 46), spacing: 8)], spacing: 8) {
                        ForEach(MessageReactionChoices.all, id: \.self) { emoji in
                            Button {
                                select(emoji == selected ? "" : emoji)
                            } label: {
                                Text(emoji)
                                    .font(.system(size: 30))
                                    .frame(maxWidth: .infinity, minHeight: 46)
                                    .background(selected == emoji ? Color.accentColor.opacity(0.18) : .clear, in: RoundedRectangle(cornerRadius: 12))
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel("React with \(emoji)")
                            .accessibilityAddTraits(selected == emoji ? .isSelected : [])
                        }
                    }
                    HStack {
                        TextField("Other emoji", text: $customEmoji)
                            .textFieldStyle(.roundedBorder)
                            .accessibilityIdentifier("custom-reaction-emoji")
                            .onSubmit { if MessageReactionChoices.isSingleEmoji(customEmoji) { select(customEmoji.trimmingCharacters(in: .whitespacesAndNewlines)) } }
                        Button("React") { select(customEmoji.trimmingCharacters(in: .whitespacesAndNewlines)) }
                            .disabled(!MessageReactionChoices.isSingleEmoji(customEmoji))
                    }
                    if selected != nil {
                        Button("Remove reaction", systemImage: "minus.circle") { select("") }
                    }
                }
                .padding(20)
            }
            .navigationTitle("React to message")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            }
        }
        #if os(iOS)
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        #else
        .frame(width: 420, height: 430)
        #endif
    }

    private func select(_ emoji: String) {
        choose(emoji)
        dismiss()
    }
}

/// WhatsApp's Forward picker: choose up to five chats, then send.
struct ForwardMessageSheet: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    let messages: [ChatMessage]
    @State private var search = ""
    @State private var selected: [String] = []
    @State private var sending = false
    static let limit = 5

    private var chats: [Contact] {
        session.contacts
            .filter { !$0.id.hasSuffix("@broadcast") && !$0.id.hasSuffix("@newsletter") }
            .filter { search.isEmpty || session.displayName(for: $0).localizedCaseInsensitiveContains(search) }
            .sorted { $0.lastMessageTime > $1.lastMessageTime }
    }

    var body: some View {
        NavigationStack {
            List(chats) { contact in
                let isSelected = selected.contains(contact.id)
                Button { toggle(contact.id) } label: {
                    HStack(spacing: 12) {
                        ContactAvatar(contact: contact, url: session.avatarURLs[contact.id], size: 36)
                        Text(session.displayName(for: contact))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Spacer()
                        Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
                            .foregroundStyle(isSelected ? Color.accentColor : Color.secondary)
                    }
                }
                .disabled(!isSelected && selected.count >= Self.limit)
                .accessibilityAddTraits(isSelected ? .isSelected : [])
            }
            .searchable(text: $search)
            .navigationTitle("Forward to…")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }.disabled(sending)
                }
                ToolbarItem(placement: .confirmationAction) {
                    if sending {
                        ProgressView()
                    } else {
                        Button("Send") { send() }
                            .disabled(selected.isEmpty)
                    }
                }
            }
        }
        .interactiveDismissDisabled(sending)
    }

    private func toggle(_ id: String) {
        if let index = selected.firstIndex(of: id) {
            selected.remove(at: index)
        } else if selected.count < Self.limit {
            selected.append(id)
        }
    }

    private func send() {
        sending = true
        Task {
            await session.forward(messages, to: selected)
            sending = false
            dismiss()
        }
    }
}

/// WhatsApp's Message info for a message you sent: when it went and how far it has got.
struct MessageInfoSheet: View {
    @Environment(\.dismiss) private var dismiss
    let message: ChatMessage

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Text(message.displayText)
                        .lineLimit(8)
                }
                Section {
                    stage("Read", reached: message.deliveryState == .read, state: .read)
                    stage("Delivered", reached: [.delivered, .read].contains(message.deliveryState), state: .delivered)
                    LabeledContent {
                        Text(message.date.formatted(date: .abbreviated, time: .shortened))
                    } label: {
                        Label { Text("Sent") } icon: { MessageDeliveryIndicator(state: .sent) }
                    }
                }
            }
            .navigationTitle("Message info")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }

    private func stage(_ title: String, reached: Bool, state: MessageDeliveryState) -> some View {
        LabeledContent {
            Text(reached ? "Yes" : "Not yet")
                .foregroundStyle(.secondary)
        } label: {
            Label { Text(title) } icon: { MessageDeliveryIndicator(state: state) }
        }
    }
}
