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
