import SwiftUI

#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

struct MessageBubble: View {
    @Environment(AppSession.self) var session
    @Environment(\.translatorPalette) var palette
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @Environment(\.chatBubbleMaxWidth) private var foldBubbleMaxWidth
    let message: ChatMessage
    let isStarred: Bool
    let isBusy: Bool
    let image: PlatformImage?
    let mediaURL: URL?
    let mediaIsLoading: Bool
    let mediaFailed: Bool
    let linkPreviews: [LinkPreview]
    let reply: () -> Void
    let translate: () -> Void
    let aiReply: () -> Void
    let toggleStar: () -> Void
    let react: (String) -> Void
    let retryMedia: () -> Void
    let albumMessages: [ChatMessage]
    let albumImages: [String: PlatformImage]
    let albumLoadingIDs: Set<String>
    let albumFailedIDs: Set<String>
    let retryAlbumMedia: (ChatMessage) -> Void
    var albumReply: ((ChatMessage) -> Void)? = nil
    var albumAIReply: ((ChatMessage) -> Void)? = nil
    @State var showAlternate = false
    @State var showActions = false
    @State var showReactionPicker = false
    @State private var demoSwipeOffset: CGFloat = 0
    @State private var swipeTranslation: CGSize = .zero

    private var swipeOffset: CGFloat {
        max(demoSwipeOffset, MessageSwipeReply.offset(translation: swipeTranslation))
    }

    private var usesInlineText: Bool {
        message.mediaKind == nil && ["text", "conversation", "extendedTextMessage"].contains(message.normalizedContentType)
            && linkPreviews.isEmpty && albumMessages.count <= 1
    }

    private var isStandaloneEmoji: Bool { message.standaloneEmojiText != nil }

    var body: some View {
        HStack(alignment: .bottom, spacing: 8) {
            if message.isFromMe { Spacer(minLength: bubbleEdgeInset) }
            ZStack(alignment: .leading) {
                Image(systemName: "arrowshape.turn.up.left.fill")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(palette.deepAccent)
                    .frame(width: 34, height: 34)
                    .background(.ultraThinMaterial, in: Circle())
                    .offset(x: 6)
                    .scaleEffect(0.75 + min(swipeOffset / MessageSwipeReply.triggerDistance, 1) * 0.25)
                    .opacity(min(swipeOffset / 32, 1))
                    .accessibilityHidden(true)

                VStack(alignment: .leading, spacing: 6) {
                    if !message.isFromMe, message.chatType == "group", let sender = message.senderName {
                        Text(sender)
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(palette.deepAccent)
                            .padding(.horizontal, isStandaloneEmoji ? 9 : 0)
                            .padding(.vertical, isStandaloneEmoji ? 4 : 0)
                            .background {
                                if isStandaloneEmoji {
                                    Capsule().fill(.ultraThinMaterial)
                                }
                            }
                    }
                    if let reply = message.content?.replyContext {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(reply.senderName ?? "Reply")
                                .font(.caption.weight(.semibold))
                            Text(reply.text ?? "")
                                .font(.caption)
                                .lineLimit(2)
                        }
                        .foregroundStyle(.secondary)
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 9))
                    }

                    if let emoji = message.standaloneEmojiText {
                        Text(emoji)
                            .font(.system(size: standaloneEmojiFontSize))
                            .fixedSize(horizontal: true, vertical: true)
                            .textSelection(.enabled)
                            .accessibilityLabel("Emoji: \(emoji)")
                    } else if usesInlineText {
                        CompactMessageLayout {
                            Text(MessageTextLinkifier.attributedString(from: showAlternate ? (message.alternateText ?? message.displayText) : message.displayText))
                                .font(.body)
                                .textSelection(.enabled)
                            messageMetadata
                        }
                    } else if albumMessages.count > 1 {
                        PhotoAlbumGrid(
                            messages: albumMessages,
                            showAlternate: showAlternate,
                            images: albumImages,
                            loadingIDs: albumLoadingIDs,
                            failedIDs: albumFailedIDs,
                            retry: retryAlbumMedia,
                            reply: albumReply ?? { _ in reply() },
                            aiReply: albumAIReply ?? { _ in aiReply() }
                        )
                    } else {
                        RichMessageContentView(
                            message: message,
                            displayText: showAlternate ? (message.alternateText ?? message.displayText) : message.displayText,
                            image: image,
                            mediaURL: mediaURL,
                            isLoading: mediaIsLoading,
                            failed: mediaFailed,
                            retry: retryMedia,
                            reservedImageSize: session.reservedImageSize(for: message)
                        )
                    }

                    ForEach(linkPreviews, id: \.url) { preview in
                        LinkPreviewCard(preview: preview)
                    }

                    if let reactions = message.reactions, !reactions.isEmpty {
                        HStack(spacing: 4) {
                            ForEach(reactions.keys.sorted(), id: \.self) { emoji in
                                Text("\(emoji) \(reactions[emoji]?.count ?? 0)")
                                    .font(.caption2.weight(.medium))
                                    .padding(.horizontal, 7)
                                    .padding(.vertical, 3)
                                    .background(.ultraThinMaterial, in: Capsule())
                            }
                        }
                    }

                    if isStandaloneEmoji {
                        standaloneEmojiMetadata
                    } else if !usesInlineText {
                        messageMetadata
                            .frame(maxWidth: .infinity, alignment: .trailing)
                    }

                    if showActions {
                        visibleActionStrip
                            .transition(.move(edge: .bottom).combined(with: .opacity))
                    }
                }
                .frame(maxWidth: bubbleContentMaxWidth, alignment: .leading)
                .padding(.horizontal, isStandaloneEmoji ? 4 : (message.mediaKind == .image ? 4 : 9))
                .padding(.vertical, isStandaloneEmoji ? 2 : 5)
                .background {
                    if !isStandaloneEmoji {
                        UnevenRoundedRectangle(
                            topLeadingRadius: message.isFromMe ? 13 : 4,
                            bottomLeadingRadius: 13,
                            bottomTrailingRadius: 13,
                            topTrailingRadius: message.isFromMe ? 4 : 13,
                            style: .continuous
                        )
                        .fill(message.isFromMe ? palette.outgoingBubble : palette.incomingBubble)
                        .shadow(color: .black.opacity(0.06), radius: 1, y: 1)
                    }
                }
                .contextMenu { actionMenu }
                .menuActionDismissBehavior(.enabled)
                .offset(x: swipeOffset)
            }
            #if os(macOS)
            .frame(
                maxWidth: MacChatLayoutMetrics.maximumBubbleWidth,
                alignment: message.isFromMe ? .trailing : .leading
            )
            #endif
            .contentShape(Rectangle())
            .modifier(
                MessageSwipeReplyModifier(translation: $swipeTranslation) {
                    performReplyFeedback()
                    reply()
                }
            )
            .accessibilityAction(named: "Reply") { reply() }
            .accessibilityAction(named: "Message actions") { showActions.toggle() }
            if !message.isFromMe { Spacer(minLength: bubbleEdgeInset) }
        }
        .frame(maxWidth: .infinity)
        .sheet(isPresented: $showReactionPicker) {
            MessageReactionPicker(selected: message.ownReactionEmoji, choose: react).modifier(ReadReceiptCover())
        }
        .task {
            if ProcessInfo.processInfo.arguments.contains("-demoActions"), message.id == "4" {
                showActions = true
            }
            if ProcessInfo.processInfo.arguments.contains("-demoSwipeReplyReveal"), message.id == "4" {
                demoSwipeOffset = MessageSwipeReply.triggerDistance
            }
        }
    }

    var usesCompactMessageChrome: Bool {
        NativeLayoutPolicy.usesCompactMessageChrome(for: dynamicTypeSize)
    }

    private var bubbleEdgeInset: CGFloat {
        usesCompactMessageChrome || message.mediaKind != nil ? 18 : 36
    }

}

