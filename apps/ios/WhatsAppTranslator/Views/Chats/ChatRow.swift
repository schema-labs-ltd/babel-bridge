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
                    Text(contact.lastMessageTime.chatListDate)
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
                    Text(contact.lastMessageTime.chatListDate)
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
            Text(contact.lastMessagePreview ?? "No messages yet")
                .foregroundStyle(Color.platformSecondaryLabel)
                .lineLimit(lineLimit)
                .truncationMode(.tail)
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
