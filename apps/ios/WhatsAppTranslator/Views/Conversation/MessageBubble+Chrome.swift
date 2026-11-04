import SwiftUI
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

extension MessageBubble {
    func performReplyFeedback() {
        #if canImport(UIKit)
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        #elseif canImport(AppKit)
        NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .now)
        #endif
    }

    /// The text Copy puts on the pasteboard: what the bubble shows, for text and captions.
    var copyText: String? {
        // An album shows the caption of whichever photo has one, so Copy takes that one.
        let source = albumMessages.count > 1
            ? albumMessages.first { !($0.content?.caption?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true) }
            : message
        guard let source, source.normalizedContentType != "revoked",
              source.contentText != nil || Self.usesInlineTextContent(source) else { return nil }
        return showAlternate ? (source.alternateText ?? source.displayText) : source.displayText
    }

    private static func usesInlineTextContent(_ message: ChatMessage) -> Bool {
        ["text", "conversation", "extendedtextmessage"].contains(message.normalizedContentType)
    }

    func copyToPasteboard(_ text: String) {
        #if canImport(UIKit)
        UIPasteboard.general.string = text
        #elseif canImport(AppKit)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        #endif
    }

    var messageMetadata: some View {
        HStack(spacing: 6) {
            if message.isTranslated {
                Button {
                    withAnimation(.snappy) { showAlternate.toggle() }
                } label: {
                    Image(systemName: showAlternate ? "character" : "character.bubble")

                }
                .buttonStyle(.plain)
                .accessibilityLabel(showAlternate ? "Show translated message" : "Show original message")
            }
            if isBusy { ProgressView().controlSize(.mini) }
            if isStarred { Image(systemName: "star.fill").foregroundStyle(.yellow) }
            if message.isEdited { Text("Edited") }
            Text(message.date.formatted(date: .omitted, time: .shortened))
                .fixedSize(horizontal: true, vertical: false)
            if message.isFromMe {
                MessageDeliveryIndicator(state: message.deliveryState)
            }

        }
        .font(.caption2)
        .foregroundStyle(.secondary)
        .fixedSize()
        .platformCompactControlTypography()
    }

    var standaloneEmojiMetadata: some View {
        HStack(spacing: 5) {
            if isBusy { ProgressView().controlSize(.mini) }
            if isStarred { Image(systemName: "star.fill").foregroundStyle(.yellow) }
            if message.isEdited { Text("Edited") }
            Text(message.date.formatted(date: .omitted, time: .shortened))
            if message.isFromMe {
                MessageDeliveryIndicator(state: message.deliveryState)
            }
        }
        .font(.caption2)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(.ultraThinMaterial, in: Capsule())
        .fixedSize(horizontal: true, vertical: false)
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    var visibleActionStrip: some View {
        #if os(macOS)
        HStack(spacing: 6) {
            compactActionButton("Reply", systemImage: "arrowshape.turn.up.left", action: reply)
            if message.canTranslate {
                compactActionButton("Translate", systemImage: "character.bubble", action: translate)
            }
            if message.canGenerateAIReply {
                compactActionButton("AI reply", systemImage: "sparkles", action: aiReply)
            }
            compactActionButton(
                isStarred ? "Unstar" : "Star",
                systemImage: isStarred ? "star.fill" : "star",
                action: toggleStar
            )
            Menu {
                ForEach(["👍", "❤️", "😂", "😮", "😢", "🙏"], id: \.self) { emoji in
                    Button(emoji) { react(emoji) }
                }
            } label: {
                compactActionLabel(systemImage: "face.smiling")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .help("React")
            .accessibilityLabel("React")
        }
        .foregroundStyle(palette.deepAccent)
        .padding(.top, 2)
        #else
        if usesCompactMessageChrome {
            compactMobileActionStrip
        } else {
            HStack(spacing: 10) {
                actionButton("Reply", systemImage: "arrowshape.turn.up.left", action: reply)
                if message.canTranslate {
                    actionButton("Translate", systemImage: "character.bubble", action: translate)
                }
                if message.canGenerateAIReply {
                    actionButton("AI reply", systemImage: "sparkles", action: aiReply)
                }
                actionButton(isStarred ? "Unstar" : "Star", systemImage: isStarred ? "star.fill" : "star", action: toggleStar)
                Menu {
                    ForEach(["👍", "❤️", "😂", "😮", "😢", "🙏"], id: \.self) { emoji in
                        Button(emoji) { react(emoji) }
                    }
                } label: {
                    actionLabel("React", systemImage: "face.smiling")
                }
            }
            .font(.caption2.weight(.medium))
            .foregroundStyle(palette.deepAccent)
            .padding(.top, 2)
        }
        #endif
    }

    #if os(macOS)
    private func compactActionButton(
        _ title: String,
        systemImage: String,
        action: @escaping () -> Void
    ) -> some View {
        Button {
            action()
            withAnimation(.snappy) { showActions = false }
        } label: {
            compactActionLabel(systemImage: systemImage)
        }
        .buttonStyle(.plain)
        .help(title)
        .accessibilityLabel(title)
    }

    private func compactActionLabel(systemImage: String) -> some View {
        Image(systemName: systemImage)
            .font(.system(size: 13, weight: .semibold))
            .frame(width: 28, height: 26)
            .background(.primary.opacity(0.065), in: RoundedRectangle(cornerRadius: 7))
            .contentShape(RoundedRectangle(cornerRadius: 7))
    }
    #endif

    #if os(iOS)
    private var compactMobileActionStrip: some View {
        HStack(spacing: 8) {
            compactMobileActionButton("Reply", systemImage: "arrowshape.turn.up.left", action: reply)
            if message.canTranslate {
                compactMobileActionButton("Translate", systemImage: "character.bubble", action: translate)
            }
            if message.canGenerateAIReply {
                compactMobileActionButton("AI reply", systemImage: "sparkles", action: aiReply)
            }
            compactMobileActionButton(
                isStarred ? "Unstar" : "Star",
                systemImage: isStarred ? "star.fill" : "star",
                action: toggleStar
            )
            Menu {
                ForEach(["👍", "❤️", "😂", "😮", "😢", "🙏"], id: \.self) { emoji in
                    Button(emoji) { react(emoji) }
                }
            } label: {
                compactMobileActionLabel(systemImage: "face.smiling")
            }
            .accessibilityLabel("React")
        }
        .foregroundStyle(palette.deepAccent)
        .padding(.top, 2)
    }

    private func compactMobileActionButton(
        _ title: String,
        systemImage: String,
        action: @escaping () -> Void
    ) -> some View {
        Button {
            action()
            withAnimation(.snappy) { showActions = false }
        } label: {
            compactMobileActionLabel(systemImage: systemImage)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
    }

    private func compactMobileActionLabel(systemImage: String) -> some View {
        Image(systemName: systemImage)
            .font(.system(size: 17, weight: .semibold))
            .frame(width: 40, height: 38)
            .background(.primary.opacity(0.065), in: RoundedRectangle(cornerRadius: 10))
            .contentShape(RoundedRectangle(cornerRadius: 10))
    }
    #endif

    private func actionButton(_ title: String, systemImage: String, action: @escaping () -> Void) -> some View {
        Button {
            action()
            withAnimation(.snappy) { showActions = false }
        } label: {
            actionLabel(title, systemImage: systemImage)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
    }

    private func actionLabel(_ title: String, systemImage: String) -> some View {
        VStack(spacing: 2) {
            Image(systemName: systemImage).font(.body)
            Text(title).lineLimit(1)
        }
        .frame(minWidth: 42)
    }

    /// Media keeps its usual width. When iPhone Duo is folded like a book, every bubble also stays
    /// within one pane so no message runs across the hinge; 18 points cover the bubble's own padding.
    var bubbleContentMaxWidth: CGFloat? {
        let natural: CGFloat? = message.mediaKind == .image ? 260 : (message.mediaKind == .audio ? 238 : nil)
        guard let foldBubbleMaxWidth else { return natural }
        let foldLimit = max(120, foldBubbleMaxWidth - 18)
        return min(natural ?? foldLimit, foldLimit)
    }

    var standaloneEmojiFontSize: CGFloat {
        switch message.standaloneEmojiText?.count {
        case 1: 60
        case 2: 52
        default: 46
        }
    }

    @ViewBuilder
    var actionMenu: some View {
        #if os(macOS)
        Menu("React", systemImage: "face.smiling") {
            ForEach(MessageReactionChoices.quick, id: \.self) { emoji in
                Button("React with \(emoji)") { react(message.ownReactionEmoji == emoji ? "" : emoji) }
            }
            Button("More reactions", systemImage: "plus") { showReactionPicker = true }
        }
        .disabled(isBusy)
        #else
        ControlGroup {
            ForEach(MessageReactionChoices.quick, id: \.self) { emoji in
                Button {
                    react(message.ownReactionEmoji == emoji ? "" : emoji)
                } label: {
                    Label { Text("React with \(emoji)") } icon: { MessageReactionChoices.icon(emoji) }
                }
                .accessibilityLabel("React with \(emoji)")
            }
            Button("More reactions", systemImage: "plus") { showReactionPicker = true }
        }
        .controlGroupStyle(.palette)
        .menuActionDismissBehavior(.enabled)
        .disabled(isBusy)
        #endif
        Divider()
        Label(session.messageTopicLabel(for: message), systemImage: "tag")
        Divider()
        Button("Reply", systemImage: "arrowshape.turn.up.left", action: reply)
        if actionTargets.contains(where: AppSession.canForward) {
            Button("Forward", systemImage: "arrowshape.turn.up.right") { chosenTargets = actionTargets; showForward = true }
        }
        if let copyText {
            Button("Copy", systemImage: "doc.on.doc") { copyToPasteboard(copyText) }
        }
        if message.isFromMe {
            Button("Info", systemImage: "info.circle") { showInfo = true }
        }
        if message.canTranslate {
            Button("Translate", systemImage: "character.bubble", action: translate)
        }
        if message.canGenerateAIReply {
            Button("AI reply", systemImage: "sparkles", action: aiReply)
        }
        Button(isStarred ? "Remove star" : "Star", systemImage: isStarred ? "star.slash" : "star", action: toggleStar)
        if message.ownReactionEmoji != nil {
            Button("Remove reaction", systemImage: "minus.circle") { react("") }
        }
        if message.alternateText != nil {
            Divider()
            Button(showAlternate ? "Show translated" : "Show original", systemImage: "arrow.left.arrow.right") {
                showAlternate.toggle()
            }
        }
        Divider()
        // An unconfirmed send gets a new ID once confirmed, which would bring it back.
        if !actionTargets.contains(where: { [.sending, .uncertain].contains($0.deliveryState) }) {
            Button("Delete", systemImage: "trash", role: .destructive) { chosenTargets = actionTargets; confirmDelete = true }
        }
    }

    /// The messages Forward and Delete act on: every photo of an album, as in WhatsApp.
    var actionTargets: [ChatMessage] { albumMessages.count > 1 ? albumMessages : [message] }
}
