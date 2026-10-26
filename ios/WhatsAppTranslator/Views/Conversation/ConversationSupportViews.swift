import SwiftUI

struct PhotoSendProgressView: View {
    @Environment(\.translatorPalette) private var palette
    let progress: PhotoSendProgress

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: progress.stage == .complete ? "checkmark.circle.fill" : "photo.stack.fill")
                .font(.title3)
                .foregroundStyle(progress.stage == .failed ? .red : palette.accent)
            VStack(alignment: .leading, spacing: 6) {
                Text(progress.statusText)
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(2)
                ProgressView(value: progress.fractionCompleted)
                    .tint(progress.stage == .failed ? .red : palette.accent)
            }
        }
        .padding(12)
        .translatorGlass(in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .accessibilityElement(children: .combine)
        .accessibilityLabel(progress.statusText)
    }
}

struct ChatWallpaper: View {
    @Environment(AppSession.self) private var session
    @Environment(\.translatorPalette) private var palette

    var body: some View {
        palette.chatBackground
            .overlay { WallpaperPattern(wallpaper: session.preferences.wallpaper) }
            .ignoresSafeArea()
    }
}

struct WallpaperPattern: View {
    @Environment(\.colorScheme) private var colorScheme
    let wallpaper: AppWallpaper
    var tileSize: CGFloat = 360

    var body: some View {
        Group {
            if let asset = wallpaper.assetName {
                // White source pixels become transparent; only the fine artwork is tinted.
                // One mask therefore follows every palette in both light and dark appearance.
                (colorScheme == .dark ? Color.white : Color.black)
                    .opacity(colorScheme == .dark ? 0.17 : 0.12)
                    .mask {
                        Canvas { context, size in
                            let pattern = context.resolve(Image(asset))
                            for row in 0..<max(1, Int(ceil(size.height / tileSize))) {
                                for column in 0..<max(1, Int(ceil(size.width / tileSize))) {
                                    context.draw(pattern, in: CGRect(x: CGFloat(column) * tileSize, y: CGFloat(row) * tileSize, width: tileSize, height: tileSize))
                                }
                            }
                        }
                        .colorInvert()
                        .luminanceToAlpha()
                    }
            } else {
                Canvas { context, size in
                    for row in 0..<max(0, Int(size.height / 64 + 1)) {
                        for column in 0..<max(0, Int(size.width / 64 + 1)) {
                            let index = (row * 3 + column) % 8
                            if let symbol = context.resolveSymbol(id: index) {
                                var tile = context
                                tile.translateBy(x: CGFloat(column) * 64 + (row.isMultiple(of: 2) ? 8 : 36), y: CGFloat(row) * 64 + 20)
                                tile.rotate(by: .degrees(Double((row + column) % 3 - 1) * 22))
                                tile.draw(symbol, at: .zero)
                            }
                        }
                    }
                } symbols: {
                    ForEach(Array(["cup.and.saucer", "paperplane", "heart", "camera", "sun.max", "leaf", "globe.europe.africa", "sparkles"].enumerated()), id: \.offset) { index, symbol in
                        Image(systemName: symbol)
                            .font(.system(size: 32, weight: .ultraLight))
                            .foregroundStyle(Color.secondary.opacity(0.14))
                            .tag(index)
                    }
                }

            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

#if os(iOS)
/// A frosted fade behind the inline header, like the one under a WhatsApp chat header. The frost is
/// strongest at the top of the display and eases through the header, so a scrolled bubble shows only
/// faintly behind the name, then dissolves over the row beneath the header's bottom edge: the message
/// just under the header is about half visible, the next one is clear, and there is no bar edge.
struct InlineHeaderBackground: ViewModifier {
    /// The frost's strength at the header's bottom edge, as a fraction of full strength.
    private static let edgeStrength = 0.68
    /// How far below the header's bottom edge the frost reaches before it is gone.
    private static let dissolveHeight: CGFloat = 60

    func body(content: Content) -> some View {
        content.background {
            Rectangle()
                .fill(.ultraThinMaterial)
                .mask {
                    VStack(spacing: 0) {
                        // Full strength at the top of the display, easing to the edge strength at the
                        // header's bottom edge, so the header never reads as a solid band.
                        LinearGradient(
                            colors: [.black, .black.opacity(Self.edgeStrength)],
                            startPoint: .top,
                            endPoint: .bottom
                        )
                        // Beneath the header the frost falls away quickly at first and then tails off,
                        // which is softer than a straight dissolve of the same height.
                        LinearGradient(
                            stops: [
                                .init(color: .black.opacity(Self.edgeStrength), location: 0),
                                .init(color: .black.opacity(0.36), location: 0.3),
                                .init(color: .black.opacity(0.14), location: 0.6),
                                .init(color: .clear, location: 1)
                            ],
                            startPoint: .top,
                            endPoint: .bottom
                        )
                        .frame(height: Self.dissolveHeight)
                    }
                }
                .padding(.bottom, -Self.dissolveHeight)
                .ignoresSafeArea(edges: .top)
                .allowsHitTesting(false)
        }
    }
}
#endif

struct DatePill: View {
    let date: Date
    var body: some View {
        Text(date.formatted(.dateTime.weekday().day().month()))
            .font(.caption.weight(.semibold))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .translatorGlass(in: Capsule())
    }
}

struct FilterPill: View {
    let title: String
    let systemImage: String
    let clear: () -> Void

    var body: some View {
        HStack(spacing: 7) {
            Image(systemName: systemImage)
            Text(title)
            Button("Clear filter", systemImage: "xmark.circle.fill", action: clear)
                .labelStyle(.iconOnly)
        }
        .font(.caption.weight(.semibold))
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .translatorGlass(in: Capsule())
    }
}

struct ConversationCostView: View {
    @Environment(AppSession.self) private var session
    @Environment(\.dismiss) private var dismiss
    let contact: Contact
    @State private var usage: UsageSummary?
    @State private var error: String?

    var body: some View {
        NavigationStack {
            List {
                if let usage {
                    Section("Conversation cost") {
                        costRow("Total", usage.costUsd.formatted(.currency(code: "USD").precision(.fractionLength(4))))
                        costRow("Input tokens", usage.inputTokens.formatted())
                        costRow("Cached input", usage.cachedInputTokens.formatted())
                        costRow("Output tokens", usage.outputTokens.formatted())
                    }
                } else if let error {
                    VStack(spacing: 14) {
                        ContentUnavailableView("Couldn’t load cost", systemImage: "exclamationmark.triangle", description: Text(error))
                        Button("Try again", systemImage: "arrow.clockwise") {
                            Task { await load() }
                        }
                        .buttonStyle(.borderedProminent)
                    }
                } else {
                    HStack { Spacer(); ProgressView(); Spacer() }
                }
            }
            .navigationTitle(session.displayName(for: contact))
            .platformInlineNavigationTitle()
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
            .task { await load() }
        }
        #if os(macOS)
        .platformSheetSize(
            minWidth: MacChatLayoutMetrics.costSheetMinimumWidth,
            minHeight: MacChatLayoutMetrics.costSheetMinimumHeight
        )
        #endif
    }

    private func load() async {
        error = nil
        do { usage = try await session.conversationUsage(for: contact.id) }
        catch { self.error = error.localizedDescription }
    }

    private func costRow(_ label: String, _ value: String) -> some View {
        LabeledContent(label) {
            Text(value)
                .foregroundStyle(.secondary)
                .monospacedDigit()
        }
    }
}
