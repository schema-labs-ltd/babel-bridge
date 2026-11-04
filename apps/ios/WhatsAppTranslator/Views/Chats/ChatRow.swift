import SwiftUI

struct ChatRow: View {
    @Environment(\.translatorPalette) private var palette
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let contact: Contact
    let displayName: String
    let draft: String
    let avatarURL: URL?
    var isMuted = false
    var isMarkedUnread = false
    var latestMessage: ChatMessage? = nil
    /// The chat's newest message is deleted for me, so the server's preview and time are stale.
    var newestIsHidden = false

    private var listTime: Int64 {
        newestIsHidden ? (latestMessage?.timestamp ?? contact.lastMessageTime) : contact.lastMessageTime
    }

    private var showsUnread: Bool { contact.unreadCount > 0 || isMarkedUnread }
    private var unreadTint: Color { isMuted ? Color.platformSecondaryLabel : palette.accent }

    var body: some View {
        Group {
            if NativeLayoutPolicy.usesStackedChatRow(for: dynamicTypeSize) {
                accessibilityLayout
            } else {
                standardLayout
            }
        }
        .padding(.vertical, 11)
        #if os(macOS)
        .frame(minHeight: 58)
        #endif
        .contentShape(Rectangle())
    }

    private var standardLayout: some View {
        HStack(spacing: 13) {
            ContactAvatar(contact: contact, url: avatarURL, size: avatarSize)
            VStack(alignment: .leading, spacing: 5) {
                HStack {
                    Text(displayName)
                        .font(.body.weight(.semibold))
                        .foregroundStyle(Color.platformPrimaryLabel)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .layoutPriority(1)
                    Spacer(minLength: 8)
                    Text(listTime.chatListDate)
                        .font(.caption)
                        .foregroundStyle(showsUnread && !isMuted ? palette.accent : Color.platformSecondaryLabel)
                        .fixedSize(horizontal: true, vertical: false)
                }
                HStack(spacing: 8) {
                    previewText(lineLimit: 2)
                    Spacer(minLength: 6)
                    statusIndicators
                }
                .font(.subheadline)
            }
        }
    }

    private var accessibilityLayout: some View {
        HStack(alignment: .top, spacing: 12) {
            ContactAvatar(contact: contact, url: avatarURL, size: 46)
            VStack(alignment: .leading, spacing: 7) {
                Text(displayName)
                    .font(.body.weight(.semibold))
                    .foregroundStyle(Color.platformPrimaryLabel)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                previewText(lineLimit: 2)
                    .font(.subheadline)
                HStack(spacing: 9) {
                    Text(listTime.chatListDate)
                        .foregroundStyle(showsUnread && !isMuted ? palette.accent : Color.platformSecondaryLabel)
                    statusIndicators
                }
                .font(.caption)
                .platformCompactControlTypography()
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private func previewText(lineLimit: Int) -> some View {
        if !draft.isEmpty {
            Text("Draft: \(draft)")
                .foregroundStyle(.orange)
                .italic()
                .lineLimit(lineLimit)
                .truncationMode(.tail)
                .layoutPriority(1)
        } else {
            let line = ChatListPreview.line(for: contact, latest: latestMessage, newestIsHidden: newestIsHidden)
            HStack(alignment: .firstTextBaseline, spacing: 3) {
                if let delivery = line.delivery {
                    MessageDeliveryIndicator(state: delivery)
                        .font(.footnote.weight(.semibold))
                }
                Text(line.text)
                    .foregroundStyle(Color.platformSecondaryLabel)
                    .lineLimit(lineLimit)
                    .truncationMode(.tail)
            }
            .layoutPriority(1)
        }
    }

    @ViewBuilder
    private var statusIndicators: some View {
        if contact.pinnedAt != nil {
            Image(systemName: "pin.fill")
                .font(.caption2)
                .foregroundStyle(Color.platformSecondaryLabel)
        }
        if isMuted {
            Image(systemName: "speaker.slash.fill")
                .font(.caption2)
                .foregroundStyle(Color.platformSecondaryLabel)
                .accessibilityLabel("Muted")
        }
        if contact.unreadCount > 0 {
            // Like WhatsApp, a muted chat's count is grey rather than green.
            Text("\(contact.unreadCount)")
                .font(.caption2.bold())
                .foregroundStyle(.white)
                .padding(.horizontal, 7)
                .padding(.vertical, 4)
                .background(unreadTint, in: Capsule())
                .fixedSize(horizontal: true, vertical: false)
        } else if isMarkedUnread {
            Circle()
                .fill(unreadTint)
                .frame(width: 18, height: 18)
                .accessibilityLabel("Marked as unread")
        }
    }

    private var avatarSize: CGFloat {
        #if os(macOS)
        48
        #else
        56
        #endif
    }

}

private extension Int64 {
    var chatListDate: String {
        let date = Date(timeIntervalSince1970: TimeInterval(self) / 1_000)
        if Calendar.current.isDateInToday(date) {
            return date.formatted(date: .omitted, time: .shortened)
        }
        if Calendar.current.isDateInYesterday(date) { return "Yesterday" }
        return date.formatted(.dateTime.day().month(.abbreviated))
    }
}

/// The chat list's second line, written the way WhatsApp writes it: the sender's first name in
/// groups, ticks instead of "You:" for your own messages, and an icon with a word for media.
enum ChatListPreview {
    struct Line: Equatable {
        var text: String
        var delivery: MessageDeliveryState? = nil
    }

    /// Uses the newest loaded message when it is the one the server summarised, since it carries
    /// the sender and the delivery state; otherwise tidies the server's own preview.
    /// Whether a chat preview describes this message. The server writes "Name: " or "You: " in front
    /// and cuts long text with "…"; a live update stores the message's display text instead.
    static func summarises(_ preview: String, _ message: ChatMessage, label body: String) -> Bool {
        let raw = preview.trimmingCharacters(in: .whitespacesAndNewlines)
        let forms = Set([body, message.displayText, tidy(message.displayText)].filter { !$0.isEmpty })
        for text in Set([raw, tidy(raw)]) {
            let truncated = text.hasSuffix("…")
            let whole = truncated ? String(text.dropLast()).trimmingCharacters(in: .whitespaces) : text
            // The whole preview, or what follows its "Name: " or "You: " prefix, must be the message
            // itself (or, when cut, the start of it), so "No, not OK" never matches a message "OK".
            var parts = [whole]
            if let colon = whole.range(of: ": ") { parts.append(String(whole[colon.upperBound...])) }
            for part in parts where !part.isEmpty {
                if forms.contains(where: { truncated ? $0.hasPrefix(part) : $0 == part }) { return true }
            }
        }
        return false
    }

    static func line(for contact: Contact, latest: ChatMessage?, newestIsHidden: Bool = false) -> Line {
        // The server's preview would show a message deleted for me: use the newest one left.
        if newestIsHidden {
            guard let latest, !latest.isReaction else { return Line(text: "No messages yet") }
            return line(for: latest, in: contact)
        }
        let serverLine = Line(text: tidy(contact.lastMessagePreview ?? "No messages yet"))
        guard let latest, !latest.isReaction, latest.timestamp >= contact.lastMessageTime else {
            return serverLine
        }
        let body = label(for: latest)
        // Timestamps are only to the second, so a different message can share one. Trust the cached
        // message then only when the preview reads as a summary of it.
        if latest.timestamp == contact.lastMessageTime,
           !summarises(contact.lastMessagePreview ?? "", latest, label: body) {
            return serverLine
        }
        return line(for: latest, in: contact)
    }

    private static func line(for latest: ChatMessage, in contact: Contact) -> Line {
        let body = label(for: latest)
        if latest.isFromMe {
            switch latest.deliveryState {
            case .sending, .sent, .delivered, .read:
                return Line(text: body, delivery: latest.normalizedContentType == "revoked" ? nil : latest.deliveryState)
            case .none, .uncertain:
                return Line(text: "You: \(body)")
            }
        }
        if contact.isGroup, let first = latest.senderName?.split(separator: " ").first {
            return Line(text: "\(first): \(body)")
        }
        return Line(text: body)
    }

    static func label(for message: ChatMessage) -> String {
        let content = message.content
        let caption = content?.caption?.nilIfBlank == nil ? nil : flattened(message.displayText)
        return switch message.normalizedContentType {
        case "image": caption.map { "📷 \($0)" } ?? "📷 Photo"
        case "video": caption.map { "🎥 \($0)" } ?? "🎥 Video"
        case "audio": content?.isVoiceNote == true ? "🎤 Voice message" : "🎵 Audio"
        case "document": "📄 \(content?.fileName?.nilIfBlank ?? "Document")"
        case "sticker": "Sticker"
        case "location": "📍 \(content?.name?.nilIfBlank ?? "Location")"
        case "contact": "👤 \(content?.displayName?.nilIfBlank ?? content?.name?.nilIfBlank ?? "Contact")"
        case "poll": "📊 \(content?.question?.nilIfBlank ?? "Poll")"
        case "revoked": message.isFromMe ? "🚫 You deleted this message" : "🚫 This message was deleted"
        default: flattened(message.displayText)
        }
    }

    /// Rewrites the server's bracketed labels, such as "[ Image ] caption", in the same style.
    static func tidy(_ preview: String) -> String {
        var rest = flattened(preview)
        var prefix = ""
        if rest.hasPrefix("You: ") {
            prefix = "You: "
            rest.removeFirst(5)
        }
        let mapped: String
        if let caption = afterLabel(rest, "[ Image ]") {
            mapped = caption.isEmpty ? "📷 Photo" : "📷 \(caption)"
        } else if let caption = afterLabel(rest, "[ Video ]") {
            mapped = caption.isEmpty ? "🎥 Video" : "🎥 \(caption)"
        } else if rest == "[ Voice Note ]" {
            mapped = "🎤 Voice message"
        } else if rest == "[ Audio ]" {
            mapped = "🎵 Audio"
        } else if rest == "[ Sticker ]" {
            mapped = "Sticker"
        } else if rest == "[ Location ]" {
            mapped = "📍 Location"
        } else if rest == "[ Message deleted ]" {
            mapped = "🚫 This message was deleted"
        } else if rest == "[ Message ]" {
            mapped = "Message"
        } else if let name = bracketed(rest, "Document: ") {
            mapped = "📄 \(name)"
        } else if let name = bracketed(rest, "Contact: ") {
            mapped = "👤 \(name.isEmpty ? "Contact" : name)"
        } else if let question = bracketed(rest, "Poll: ") {
            mapped = "📊 \(question.isEmpty ? "Poll" : question)"
        } else {
            mapped = rest
        }
        return prefix + mapped
    }

    private static func afterLabel(_ text: String, _ label: String) -> String? {
        guard text.hasPrefix(label) else { return nil }
        return text.dropFirst(label.count).trimmingCharacters(in: .whitespaces)
    }

    private static func bracketed(_ text: String, _ label: String) -> String? {
        guard text.hasPrefix("[ \(label)"), text.hasSuffix(" ]") else { return nil }
        return String(text.dropFirst(label.count + 2).dropLast(2)).trimmingCharacters(in: .whitespaces)
    }

    private static func flattened(_ text: String) -> String {
        text.split(whereSeparator: \.isNewline).joined(separator: " ")
    }
}

private extension String {
    var nilIfBlank: String? { trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : self }
}
