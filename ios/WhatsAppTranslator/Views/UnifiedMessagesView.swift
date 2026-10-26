import SwiftUI

struct UnifiedMessagesView: View {
    @Environment(AppSession.self) var session
    @Environment(\.translatorPalette) var palette
    @Binding var replyDraft: UnifiedReplyDraft
    @Binding var sending: Bool
    @State var showSettings = false
    @State var settingsContact: Contact?
    @State var selectedTopicID: String?
    @State var showTopicManagement = false
    var selectedTopic: ChatTopic? { session.topics().first { $0.id == selectedTopicID } }
    var displayedMessages: [ChatMessage] {
        if let selectedTopicID { return session.topicPages[selectedTopicID]?.messages ?? [] }
        return session.unifiedMessages
    }
    @State var atBottom = true
    @State var composerFocused = false
    @State private var imagePaste = ImagePasteController()

    @State private var settingsLoadFailed = false
    private var waitingForSettings: Bool {
        guard let target = composerTarget else { return false }
        return session.savedConversationSettings[target.contactId] == nil
    }
    private var composerUnavailable: Bool { replyBlocked || waitingForSettings }
    private func loadComposerSettings() async {
        guard let id = composerTarget?.contactId else { return }
        settingsLoadFailed = false
        do { _ = try await session.conversationSettings(for: id) }
        catch { if !Task.isCancelled { settingsLoadFailed = true } }
    }

    private var composerTarget: ChatMessage? { replyDraft.selected ?? displayedMessages.last }
    private var replyBlocked: Bool {
        guard let target = composerTarget, session.isReplyOnly(target.contactId) else { return false }
        return !replyDraft.isExplicitReply || !session.canSend(to: target.contactId, reply: session.replyTarget(for: target))
    }

    private var draft: Binding<String> {
        Binding(get: { replyDraft.text }, set: { value in
            guard !sending, !composerUnavailable else { return }
            replyDraft.updateText(value, latestMessage: displayedMessages.last)
        })
    }

    var body: some View {
        FoldableLayoutReader { layout in
            messagesContent(layout: layout)
        }
        .modifier(PastedImagePresentation(controller: imagePaste))
        .task(id: composerTarget?.contactId) { await loadComposerSettings() }
    }

    var composer: some View {
        VStack(alignment: .leading, spacing: 8) {
            if waitingForSettings {
                if settingsLoadFailed {
                    Button("Couldn’t load conversation settings. Retry") { Task { await loadComposerSettings() } }
                        .font(.caption)
                } else {
                    Text("Loading conversation settings…").font(.caption).foregroundStyle(.secondary)
                }
            } else if replyBlocked {
                Label("Reply only · Choose an incoming message and tap Reply", systemImage: "arrowshape.turn.up.left")
                    .font(.caption).foregroundStyle(.secondary)
                    .accessibilityIdentifier("reply-only-hint")
            }
            if let selected = replyDraft.selected, !replyDraft.isFocused {
                HStack(alignment: .top, spacing: 9) {
                    RoundedRectangle(cornerRadius: 2).fill(palette.accent).frame(width: 3)
                    VStack(alignment: .leading, spacing: 3) {
                        Text("To: \(name(selected))").font(.subheadline.weight(.semibold)).foregroundStyle(palette.deepAccent)
                        Text(session.feedReplyNeedsQuote(selected) ? "Reply with quote" : "Send as a normal message")
                            .font(.caption).foregroundStyle(.secondary)
                        Text(selected.displayText).font(.caption).lineLimit(1).foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                    Button("Cancel reply", systemImage: "xmark.circle.fill", action: cancelReply)
                        .labelStyle(.iconOnly).foregroundStyle(.secondary).disabled(sending)
                }.fixedSize(horizontal: false, vertical: true)
                    .padding(12)
                    .translatorGlass(in: RoundedRectangle(cornerRadius: 20))
            } else if replyDraft.selected == nil && displayedMessages.isEmpty {
                Text("Waiting for messages")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if let progress = session.photoSendProgress.values.max(by: { $0.startedAt < $1.startedAt }) {
                PhotoSendProgressView(progress: progress)
            }
            ComposerGlassGroup {
                HStack(alignment: .bottom, spacing: 7) {
                    UnifiedMediaControls(
                        disabled: composerUnavailable || sending || session.sendingContactIDs.contains((replyDraft.selected ?? displayedMessages.last)?.contactId ?? "") || (replyDraft.selected == nil && displayedMessages.isEmpty),
                        begin: {
                            guard !composerUnavailable, let target = replyDraft.beginAttachment(latestMessage: displayedMessages.last) else { return nil }
                            composerFocused = false
                            return UnifiedMediaContext(message: target, reply: session.replyTarget(for: target), destination: name(target))
                        },
                        onSent: { target in replyDraft.finishMediaSending(to: target); composerFocused = false }
                    )
                    MessageComposerTextInput(text: draft, focus: $composerFocused, allowsImagePaste: !composerUnavailable && !sending && !imagePaste.isLoading) { providers in
                        guard !sending, !composerUnavailable,
                              let target = replyDraft.beginAttachment(latestMessage: displayedMessages.last),
                              !session.sendingContactIDs.contains(target.contactId) else { return }
                        composerFocused = false
                        let reply = session.replyTarget(for: target)
                        imagePaste.begin(providers, reply: reply, destination: name(target)) { images, caption in
                            guard session.startPhotoSend(images, caption: caption, to: target.contactId, reply: reply, replyOnlyIfNotLatest: true) else { return false }
                            replyDraft.finishMediaSending(to: target)
                            composerFocused = false
                            return true
                        }
                    }
                        .textFieldStyle(.plain)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 12)
                        .frame(minHeight: 46)
                        .translatorGlassControl(in: RoundedRectangle(cornerRadius: 24))
                        .disabled(composerUnavailable || sending || (replyDraft.selected == nil && displayedMessages.isEmpty))
                    Button(action: send) {
                        Group {
                            if sending { ProgressView().tint(.white) }
                            else { Image(systemName: "arrow.up").font(.system(size: 19, weight: .semibold)) }
                        }.frame(width: 46, height: 46).foregroundStyle(.white)
                            .contentShape(Circle())
                            .translatorGlassControl(in: Circle(), tint: palette.accent)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(replyDraft.selected.map { "Send to \(name($0))" } ?? "Send message")
                    .disabled(composerUnavailable || sending || replyDraft.selected == nil || draft.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
        .frame(maxWidth: 900)
        .padding(.horizontal, 14).padding(.vertical, 10)
        .frame(maxWidth: .infinity)
    }

    func cancelReply() {
        replyDraft.cancelSelection()
        composerFocused = false
    }

    private func send() {
        guard let target = replyDraft.selected, !sending, !composerUnavailable else { return }
        let text = replyDraft.drafts[target.contactId] ?? ""
        sending = true
        Task {
            let sent = await session.send(text: text, to: target.contactId, reply: session.replyTarget(for: target), replyOnlyIfNotLatest: true)
            sending = false
            if sent {
                replyDraft.finishSending(to: target)
                composerFocused = false
            }
        }
    }
}
