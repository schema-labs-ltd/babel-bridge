import AVKit
import Foundation
import SwiftUI

enum MessageTextLinkifier {
    static func attributedString(from text: String) -> AttributedString {
        var attributedText = AttributedString(text)
        guard let detector = try? NSDataDetector(
            types: NSTextCheckingResult.CheckingType.link.rawValue
        ) else {
            return attributedText
        }

        let fullRange = NSRange(text.startIndex..<text.endIndex, in: text)
        for match in detector.matches(in: text, range: fullRange) {
            guard let url = match.url,
                  let stringRange = Range(match.range, in: text),
                  let lowerBound = AttributedString.Index(stringRange.lowerBound, within: attributedText),
                  let upperBound = AttributedString.Index(stringRange.upperBound, within: attributedText) else {
                continue
            }
            attributedText[lowerBound..<upperBound].link = url
        }

        return attributedText
    }
}

/// The frame a chat photo or sticker occupies. It is worked out from the image's pixel size
/// before the image itself has loaded, so the bubble never changes height when it arrives and
/// the timeline doesn't jump while scrolling back through history.
enum ChatImageFrame {
    static let minimumAspectRatio: CGFloat = 0.72

    static func size(pixelSize: CGSize?, maxWidth: CGFloat, maxHeight: CGFloat) -> CGSize {
        guard let pixelSize, pixelSize.width > 0, pixelSize.height > 0 else {
            // Unknown proportions: a square sits between portrait and landscape photos.
            return CGSize(width: maxWidth, height: min(maxHeight, maxWidth))
        }
        let ratio = max(pixelSize.width / pixelSize.height, minimumAspectRatio)
        return CGSize(width: maxWidth, height: min(maxHeight, maxWidth / ratio))
    }
}

struct RichMessageContentView: View {
    let message: ChatMessage
    let displayText: String
    let image: PlatformImage?
    let mediaURL: URL?
    let isLoading: Bool
    let failed: Bool
    let retry: () -> Void
    /// Pixel size known before the image loads, from the server or an earlier viewing.
    var reservedImageSize: CGSize? = nil
    @State private var showPhotoViewer = false

    var body: some View {
        Group {
            switch message.mediaKind {
            case .image:
                imageContent(maxWidth: 260, maxHeight: 360)
                caption
            case .sticker:
                imageContent(maxWidth: 180, maxHeight: 180)
            case .video:
                if let mediaURL {
                    InlineVideoPlayer(url: mediaURL)
                } else {
                    mediaPlaceholder(systemImage: "video.fill", title: "Video")
                }
                caption
            case .audio:
                if mediaURL == nil { mediaPlaceholder(systemImage: "waveform", title: "Voice note") }
                VoiceTranslationPlayer(message: message, originalURL: mediaURL)
            case .document:
                if let mediaURL {
                    DocumentMessageView(
                        url: mediaURL,
                        name: message.content?.fileName ?? "Document",
                        fileSize: message.content?.fileSize
                    )
                } else {
                    mediaPlaceholder(systemImage: "doc.fill", title: message.displayText)
                }
                caption
            case nil:
                nonMediaContent
            }
        }
    }

    @ViewBuilder
    private func imageContent(maxWidth: CGFloat, maxHeight: CGFloat) -> some View {
        let frame = ChatImageFrame.size(
            pixelSize: reservedImageSize ?? image?.size,
            maxWidth: maxWidth,
            maxHeight: maxHeight
        )
        if let image {
            Image(platformImage: image)
                .resizable()
                .scaledToFill()
                .frame(width: frame.width, height: frame.height)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .contentShape(Rectangle())
                .highPriorityGesture(
                    TapGesture().onEnded { showPhotoViewer = true }
                )
                .help("View photo full screen")
                .accessibilityLabel("View photo full screen")
                .accessibilityAddTraits(.isButton)
                .accessibilityAction { showPhotoViewer = true }
                .photoViewer(isPresented: $showPhotoViewer, image: image)
        } else {
            imagePlaceholder(
                size: frame,
                systemImage: message.mediaKind == .sticker ? "face.smiling" : "photo.fill",
                title: message.mediaKind == .sticker ? "Sticker" : "Image"
            )
        }
    }

    /// Holds the image's final frame while it loads, the way WhatsApp shows a blank tile.
    private func imagePlaceholder(size: CGSize, systemImage: String, title: String) -> some View {
        ZStack {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(.primary.opacity(0.06))
            VStack(spacing: 6) {
                if isLoading {
                    ProgressView()
                } else {
                    Image(systemName: failed ? "exclamationmark.arrow.trianglehead.2.clockwise.rotate.90" : systemImage)
                        .font(.title2)
                        .foregroundStyle(.secondary)
                }
                if failed { Text("Tap to try again").font(.caption).foregroundStyle(.secondary) }
            }
        }
        .frame(width: size.width, height: size.height)
        .contentShape(Rectangle())
        .onTapGesture { if failed { retry() } }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(failed ? "Media failed to load. Try again." : title)
    }

    @ViewBuilder
    private var caption: some View {
        if let caption = message.content?.caption?.trimmingCharacters(in: .whitespacesAndNewlines), !caption.isEmpty {
            Text(MessageTextLinkifier.attributedString(from: displayText))
                .font(.body)
                .textSelection(.enabled)
        }
    }

    @ViewBuilder
    private var nonMediaContent: some View {
        switch message.normalizedContentType {
        case "location":
            if let url = message.locationURL {
                Link(destination: url) {
                    Label {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(message.content?.name ?? "Location").fontWeight(.semibold)
                            if let address = message.content?.address { Text(address).font(.caption).foregroundStyle(.secondary) }
                        }
                    } icon: {
                        Image(systemName: "map.fill").font(.title2)
                    }
                    .padding(9)
                    .background(.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 11))
                }
                .buttonStyle(.plain)
            } else {
                Label(displayText, systemImage: "map.fill")
            }
        case "contact":
            Label {
                VStack(alignment: .leading, spacing: 2) {
                    Text(message.content?.displayName ?? message.content?.name ?? "Contact").fontWeight(.semibold)
                    if let vcard = message.content?.vcard, !vcard.isEmpty {
                        Text(vcard.components(separatedBy: .newlines).first(where: { $0.hasPrefix("TEL") }) ?? "Contact card")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            } icon: {
                Image(systemName: "person.crop.circle.fill").font(.title2)
            }
            .padding(9)
            .background(.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 11))
        case "poll":
            VStack(alignment: .leading, spacing: 7) {
                Label(message.content?.question ?? "Poll", systemImage: "chart.bar.fill")
                    .font(.body.weight(.semibold))
                ForEach(message.content?.options ?? [], id: \.self) { option in
                    Label(option, systemImage: "circle")
                        .font(.callout)
                }
            }
            .padding(9)
            .background(.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 11))
        case "revoked":
            Label(displayText, systemImage: "nosign")
                .font(.body.italic())
                .foregroundStyle(.secondary)
        default:
            Text(MessageTextLinkifier.attributedString(from: displayText))
                .font(.body)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func mediaPlaceholder(systemImage: String, title: String) -> some View {
        HStack(spacing: 10) {
            if isLoading {
                ProgressView()
            } else {
                Image(systemName: failed ? "exclamationmark.arrow.trianglehead.2.clockwise.rotate.90" : systemImage)
                    .font(.title2)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.callout.weight(.semibold))
                if failed { Text("Tap to try again").font(.caption).foregroundStyle(.secondary) }
            }
        }
        .padding(12)
        .frame(minWidth: 190, minHeight: 74, alignment: .leading)
        .background(.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .contentShape(Rectangle())
        .onTapGesture { if failed { retry() } }
        .accessibilityLabel(failed ? "Media failed to load. Try again." : title)
    }
}

struct AudioMessagePlayer: View {
    let url: URL
    let title: String
    let duration: Double?
    @State private var player: AVAudioPlayer?
    @State private var isPlaying = false
    @State private var playbackError: String?

    var body: some View {
        HStack(spacing: 10) {
            Button(isPlaying ? "Pause" : "Play", systemImage: isPlaying ? "pause.fill" : "play.fill") {
                do {
                    playbackError = nil
                    if player == nil { player = try AVAudioPlayer(contentsOf: url) }
                    if isPlaying { player?.pause() } else {
                        #if os(iOS)
                        try AVAudioSession.sharedInstance().setCategory(.playback, mode: .default)
                        try AVAudioSession.sharedInstance().setActive(true)
                        #endif
                        guard player?.play() == true else { throw CocoaError(.fileReadCorruptFile) }
                    }
                    isPlaying = player?.isPlaying == true
                } catch {
                    isPlaying = false
                    playbackError = "Unable to play this voice note: \(error.localizedDescription)"
                }
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.plain)
            .font(.system(size: 24))
            .foregroundStyle(.secondary)
            .frame(width: 44, height: 44)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 2) {
                    ForEach(0..<36) { index in
                        Capsule()
                            .fill(Color.secondary.opacity(0.5))
                            .frame(width: 3, height: CGFloat(5 + ((index * 7 + 11) % 23)))
                    }
                }
                .accessibilityHidden(true)
                if let playbackError { Text(playbackError).font(.caption).foregroundStyle(.red) }
                if let duration {
                    Text(String(format: "%d:%02d", Int(duration) / 60, Int(duration) % 60))
                        .font(.caption2).foregroundStyle(.secondary)
                }
            }
        }
        .accessibilityLabel(title)
        .padding(.vertical, 2)
        .frame(width: 238, alignment: .leading)
        .task(id: url) {
            player?.stop(); player = nil; isPlaying = false; playbackError = nil
            while !Task.isCancelled {
                isPlaying = player?.isPlaying == true
                try? await Task.sleep(for: .milliseconds(200))
            }
        }
        .onDisappear { player?.pause(); isPlaying = false }
    }
}

private struct DocumentMessageView: View {
    let url: URL
    let name: String
    let fileSize: Int?

    var body: some View {
        HStack(spacing: 10) {
            Link(destination: url) {
                Label {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(name).font(.callout.weight(.semibold)).lineLimit(2)
                        if let fileSize {
                            Text(ByteCountFormatter.string(fromByteCount: Int64(fileSize), countStyle: .file))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                } icon: {
                    Image(systemName: "doc.fill").font(.title2)
                }
            }
            .buttonStyle(.plain)
            Spacer(minLength: 4)
            ShareLink(item: url) { Image(systemName: "square.and.arrow.up") }
                .labelStyle(.iconOnly)
        }
        .padding(10)
        .frame(minWidth: 230, alignment: .leading)
        .background(.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 12))
    }
}
