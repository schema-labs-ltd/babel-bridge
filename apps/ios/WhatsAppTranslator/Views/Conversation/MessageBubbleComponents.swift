import SwiftUI
#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

struct PhotoAlbumGrid: View {
    let messages: [ChatMessage]
    let showAlternate: Bool
    let images: [String: PlatformImage]
    let loadingIDs: Set<String>
    let failedIDs: Set<String>
    let retry: (ChatMessage) -> Void
    let reply: (ChatMessage) -> Void
    let aiReply: (ChatMessage) -> Void
    @State private var selectedPhoto: ChatMessage?
    private let columns = [GridItem(.flexible(), spacing: 3), GridItem(.flexible(), spacing: 3)]

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            LazyVGrid(columns: columns, spacing: 3) {
                ForEach(Array(messages.prefix(PhotoGalleryLayout.previewLimit).enumerated()), id: \.element.id) { index, message in
                    Button { selectedPhoto = message } label: {
                        ZStack {
                            Color.primary.opacity(0.05)
                            if let image = images[message.id] {
                                GeometryReader { geometry in
                                    Image(platformImage: image).resizable().scaledToFill()
                                        .frame(width: geometry.size.width, height: geometry.size.height)
                                        .clipped()
                                }
                            } else if loadingIDs.contains(message.id) {
                                ProgressView()
                            } else {
                                Image(systemName: failedIDs.contains(message.id) ? "arrow.clockwise" : "photo.fill")
                            }
                            if index == PhotoGalleryLayout.previewLimit - 1, PhotoGalleryLayout.hiddenCount(total: messages.count) > 0 {
                                Color.black.opacity(0.45)
                                Text("+\(PhotoGalleryLayout.hiddenCount(total: messages.count))")
                                    .font(.largeTitle.weight(.semibold)).foregroundStyle(.white)
                            }
                        }
                        .frame(height: 126)
                        .clipShape(RoundedRectangle(cornerRadius: 9, style: .continuous))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Open photo \(index + 1) of \(messages.count)")
                    .accessibilityHint(index == PhotoGalleryLayout.previewLimit - 1 ? "Opens the gallery with all photos" : "View photo and browse the gallery")
                }
            }
            Text("\(messages.count) photos").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            if let caption = messages.first(where: { !($0.content?.caption?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true) }) {
                Text(MessageTextLinkifier.attributedString(from: showAlternate ? (caption.alternateText ?? caption.displayText) : caption.displayText))
                    .font(.body).lineLimit(2)
            }
        }
        .frame(maxWidth: 260)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Photo gallery, \(messages.count) photos")
        #if os(macOS)
        .sheet(item: $selectedPhoto) { photo in
            PhotoGalleryViewer(messages: messages, initialPhotoID: photo.id, reply: reply, aiReply: aiReply)
                .modifier(ReadReceiptCover())
                .frame(minWidth: 800, minHeight: 650)
        }
        #else
        .fullScreenCover(item: $selectedPhoto) { photo in
            PhotoGalleryViewer(messages: messages, initialPhotoID: photo.id, reply: reply, aiReply: aiReply)
                .modifier(ReadReceiptCover())
        }
        #endif
    }
}

struct MessageDeliveryIndicator: View {
    let state: MessageDeliveryState

    var body: some View {
        HStack(spacing: -4) {
            Image(systemName: state == .uncertain ? "exclamationmark.circle" : (state == .sending ? "clock" : "checkmark"))
            if state == .uncertain { Text("Delivery uncertain").font(.caption2).padding(.leading, 6) }
            if state == .delivered || state == .read {
                Image(systemName: "checkmark")
            }
        }
        .foregroundStyle(
            state == .read
                ? Color(red: 83 / 255, green: 189 / 255, blue: 235 / 255)
                : Color.platformSecondaryLabel
        )
        .fixedSize(horizontal: true, vertical: false)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(state.accessibilityLabel)
        .help(state.accessibilityLabel)
    }
}

/// Sits beside a message of yours that hasn't reached the server: a small spinner while it sends,
/// then a resend button if it fails, as WhatsApp marks a message it couldn't send.
struct PendingSendStatus: View {
    let send: PendingSend
    let retry: () -> Void

    var body: some View {
        Group {
            if send.failed {
                Button(action: retry) {
                    Image(systemName: "arrow.clockwise.circle.fill")
                        .font(.system(size: 22, weight: .semibold))
                        .symbolRenderingMode(.palette)
                        .foregroundStyle(.white, .red)
                        .frame(width: 44, height: 44)
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Resend message")
                .accessibilityHint(send.failure ?? "")
                .help(send.failure.map { "Not sent: \($0). Tap to try again." } ?? "Tap to try again.")
                .accessibilityIdentifier("resend-message")
            } else {
                ProgressView()
                    .controlSize(.small)
                    .frame(width: 22, height: 22)
                    .padding(.bottom, 4)
                    .accessibilityLabel("Sending")
                    .accessibilityIdentifier("sending-message")
            }
        }
        .transition(.opacity.combined(with: .scale))
    }
}

struct LinkPreviewCard: View {
    @Environment(\.translatorPalette) private var palette
    let preview: LinkPreview

    var body: some View {
        if preview.error == nil {
            Link(destination: preview.url) {
                HStack(spacing: 10) {
                    if let imageURL = preview.imageURL {
                        AsyncImage(url: imageURL) { image in
                            image.resizable().scaledToFill()
                        } placeholder: {
                            Color.primary.opacity(0.05)
                        }
                        .frame(width: 64, height: 64)
                        .clipShape(RoundedRectangle(cornerRadius: 9))
                    }
                    VStack(alignment: .leading, spacing: 3) {
                        Text(preview.siteName ?? preview.url.host() ?? "Link")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(palette.deepAccent)
                        Text(preview.title ?? preview.url.absoluteString)
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(2)
                            .truncationMode(.tail)
                        if let description = preview.description {
                            Text(description)
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                        }
                    }
                }
                .padding(8)
                .frame(maxWidth: 300, alignment: .leading)
                .background(.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 11))
            }
            .buttonStyle(.plain)
        }
    }
}

/// Short messages and receipts share a line; longer text wraps without expanding short bubbles.
struct CompactMessageLayout: Layout {
    private func measurements(_ proposal: ProposedViewSize, _ subviews: Subviews) -> (CGSize, CGSize, Bool, CGFloat) {
        let footer = subviews[1].sizeThatFits(.unspecified)
        let natural = subviews[0].sizeThatFits(.unspecified)
        let available = max(1, proposal.width ?? natural.width + footer.width + 8)
        let inline = natural.width + footer.width + 8 <= available
        let text = inline ? natural : subviews[0].sizeThatFits(ProposedViewSize(width: available, height: nil))
        return (text, footer, inline, inline ? text.width + footer.width + 8 : max(text.width, footer.width))
    }
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let (text, footer, inline, width) = measurements(proposal, subviews)
        return CGSize(width: width, height: inline ? max(text.height, footer.height) : text.height + footer.height + 2)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let (text, footer, inline, _) = measurements(proposal, subviews)
        subviews[0].place(at: bounds.origin, proposal: ProposedViewSize(text))
        subviews[1].place(at: CGPoint(x: bounds.maxX - footer.width, y: inline ? bounds.maxY - footer.height : bounds.minY + text.height + 2), proposal: ProposedViewSize(footer))
    }
}

/// The quote at the top of a reply, drawn like WhatsApp's: a coloured bar and sender name over a
/// tinted box. Tapping it scrolls to the quoted message when that message can be found.
struct QuotedReplyView: View {
    @Environment(\.translatorPalette) private var palette
    let reply: ReplyContext
    let isGroup: Bool
    let open: (() -> Void)?

    // Keyed by display name, as the sender's name on their own bubbles is, so one person keeps one colour.
    private var tint: Color {
        guard isGroup, let name = reply.senderName, !name.isEmpty, name != "You" else { return palette.deepAccent }
        return SenderNameColor.color(for: name)
    }

    var body: some View {
        let quote = HStack(spacing: 0) {
            Rectangle().fill(tint).frame(width: 4)
            VStack(alignment: .leading, spacing: 2) {
                // The bar and background carry the colour; small coloured text is too faint on
                // outgoing bubbles, so the name stays in the primary text colour.
                Text(reply.senderName ?? "Reply")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.primary)
                Text(reply.text ?? "")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(tint.opacity(0.1))
        .clipShape(RoundedRectangle(cornerRadius: 7, style: .continuous))

        if let open {
            Button(action: open) { quote }
                .buttonStyle(.plain)
                .accessibilityHint("Shows the quoted message")
        } else {
            quote
        }
    }
}

/// Every reaction on a message in one pill: the emojis, most used first, and a total when more
/// than one person reacted.
struct ReactionSummaryPill: View {
    let reactions: [String: [String]]

    private var ordered: [String] {
        reactions.filter { !$0.value.isEmpty }
            .sorted { $0.value.count == $1.value.count ? $0.key < $1.key : $0.value.count > $1.value.count }
            .map(\.key)
    }

    private var total: Int { reactions.values.reduce(0) { $0 + $1.count } }

    var body: some View {
        HStack(spacing: 2) {
            Text(ordered.prefix(3).joined())
            if total > 1 {
                Text("\(total)")
                    .foregroundStyle(.secondary)
            }
        }
        .font(.footnote)
        .padding(.horizontal, 7)
        .padding(.vertical, 3)
        .background(.regularMaterial, in: Capsule())
        .overlay(Capsule().stroke(Color.primary.opacity(0.08), lineWidth: 0.5))
        .shadow(color: .black.opacity(0.08), radius: 1, y: 1)
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Reactions: \(ordered.joined(separator: ", ")), \(total) in total")
    }
}
