import CryptoKit
import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    /// The sticker tray, as in WhatsApp: stickers sent and received in the loaded chats and feed,
    /// newest first, each sticker once.
    func recentStickers(limit: Int = 60) -> [ChatMessage] {
        var seen = Set<String>()
        var result: [ChatMessage] = []
        // Only stickers whose file can be had: from the server, inline, or already on this device.
        for sticker in stickerCandidates() where result.count < limit && stickerIsAvailable(sticker) {
            // The same sticker sent again is a new message with the same file.
            if seen.insert(stickerDigests[sticker.id] ?? sticker.id).inserted { result.append(sticker) }
        }
        return result
    }

    /// A sticker whose last load failed is left out until a later tray opening loads it again,
    /// so a passing network error doesn't hide it for good.
    private func stickerIsAvailable(_ sticker: ChatMessage) -> Bool {
        guard !mediaErrorIDs.contains(sticker.id) else { return false }
        return sticker.content?.hasMedia == true || sticker.content?.mediaData != nil
            || messageImages[sticker.id] != nil || stickerDigests[sticker.id] != nil
    }

    /// Loads the tray's stickers and fingerprints their files, so repeats collapse into one.
    func prepareStickerTray() async {
        for sticker in stickerCandidates() where stickerDigests[sticker.id] == nil {
            await loadMedia(for: sticker)
            await fingerprintSticker(sticker)
        }
    }

    /// Records a sticker's file digest once its file is on this device. Whichever of the tray
    /// and a thumbnail downloads the file records it, so a shared download is never missed.
    func fingerprintSticker(_ sticker: ChatMessage) async {
        let epoch = topicAccountEpoch
        guard stickerDigests[sticker.id] == nil,
              let url = try? await mediaCache.cachedURL(for: sticker.id),
              let data = try? Data(contentsOf: url), !data.isEmpty,
              topicAccountEpoch == epoch else { return }
        stickerDigests[sticker.id] = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    /// Recent WebP stickers that can be sent again; animated Lottie ones can't. Failed loads are
    /// kept here, so preparing the tray tries them again.
    private func stickerCandidates(limit: Int = 150) -> [ChatMessage] {
        let all = (messages.values.flatMap { $0 } + Array(feedByID.values)).filter {
            $0.mediaKind == .sticker && !$0.isReaction
                && ($0.content?.mimeType.map { $0.lowercased() == "image/webp" } ?? true)
        }
        var ids = Set<String>()
        return Array(all.sorted { $0.timestamp > $1.timestamp }.filter { ids.insert($0.id).inserted }.prefix(limit))
    }

    /// Sends a sticker from the tray again, here.
    func sendSticker(_ sticker: ChatMessage, to contactID: String, reply: MessageReplyTarget?) async -> Bool {
        guard !sendingContactIDs.contains(contactID) else {
            presentError("Couldn’t send sticker", "Wait for the message that’s sending to finish, then try again.")
            return false
        }
        // A server change while the file loads must not send it through the new account.
        let epoch = topicAccountEpoch
        var current: Bool { topicAccountEpoch == epoch && phase == .ready && !Task.isCancelled }
        var loaded: Data?
        if let url = try? await mediaCache.cachedURL(for: sticker.id) { loaded = try? Data(contentsOf: url) }
        if loaded == nil, let base64 = sticker.content?.mediaData { loaded = Data(base64Encoded: base64) }
        if loaded == nil, !demoMode {
            do {
                let response = try await api.media(messageID: sticker.id)
                loaded = Data(base64Encoded: response.mediaData)
            } catch {
                if current { presentError("Couldn’t send sticker", error) }
                return false
            }
        }
        guard current else { return false }
        guard let data = loaded, Self.isWebP(data) else {
            presentError("Couldn’t send sticker", "This sticker isn’t in a format WhatsApp accepts.")
            return false
        }
        // Another window may have started sending here while the file loaded.
        guard !sendingContactIDs.contains(contactID) else {
            presentError("Couldn’t send sticker", "Wait for the message that’s sending to finish, then try again.")
            return false
        }
        let attachment = OutgoingAttachment(data: data, mimeType: "image/webp", fileName: "sticker.webp", kind: "sticker")
        return await sendAttachment(attachment, caption: nil, to: contactID, reply: reply)
    }

    /// WhatsApp stickers are WebP files.
    nonisolated static func isWebP(_ data: Data) -> Bool {
        data.count >= 12 && data.prefix(4) == Data("RIFF".utf8) && data.dropFirst(8).prefix(4) == Data("WEBP".utf8)
    }

    func loadMedia(for message: ChatMessage) async {
        guard message.mediaKind != nil,
              messageImages[message.id] == nil,
              messageMediaURLs[message.id] == nil,
              !mediaRequests.contains(message.id) else { return }
        mediaErrorIDs.remove(message.id)
        if let cachedURL = try? await mediaCache.cachedURL(for: message.id) {
            if await restoreCachedMedia(from: cachedURL, for: message) {
                return
            }
            try? await mediaCache.remove(messageID: message.id)
        }
        if message.mediaKind != .audio || demoMode, let base64 = message.content?.mediaData, let data = Data(base64Encoded: base64) {
            await storeMedia(data, mimeType: message.content?.mimeType, for: message)
            return
        }
        guard message.content?.hasMedia == true || message.content?.mediaData != nil, !demoMode else { return }
        mediaRequests.insert(message.id)
        mediaLoadingIDs.insert(message.id)
        defer {
            mediaRequests.remove(message.id)
            mediaLoadingIDs.remove(message.id)
        }
        do {
            let response = try await api.media(messageID: message.id)
            guard let data = Data(base64Encoded: response.mediaData) else {
                throw APIError.decoding("The media payload was invalid.")
            }
            await storeMedia(data, mimeType: response.mimeType, for: message)
        } catch {
            // A prefetch or bubble that scrolled away cancels its load; it retries when shown.
            guard !Task.isCancelled else { return }
            mediaErrorIDs.insert(message.id)
        }
    }

    func galleryPage(contactID: String, before: ChatMessage? = nil) async throws -> MessagesResponse {
        if demoMode {
            let all = galleryDemoMessages.isEmpty ? (messages[contactID] ?? []) : galleryDemoMessages
            let matching = ChatMediaGalleryModel.ordered(all, contactID: contactID).filter {
                guard let before else { return true }
                return $0.timestamp < before.timestamp || ($0.timestamp == before.timestamp && $0.id < before.id)
            }
            return MessagesResponse(messages: Array(matching.prefix(60)), hasMore: matching.count > 60)
        }
        let server = configuration?.baseURL
        let response = try await api.gallery(contactID: contactID, before: before?.timestamp, beforeID: before?.id)
        guard phase == .ready, configuration?.baseURL == server, !Task.isCancelled else { throw CancellationError() }
        return MessagesResponse(messages: normalizeMessages(response.messages), hasMore: response.hasMore)
    }

    func galleryThumbnail(for message: ChatMessage) async throws -> PlatformImage {
        if let image = messageImages[message.id] { return image }
        if demoMode, let url = messageMediaURLs[message.id] {
            return try await GalleryVideoPreview.image(url: url)
        }
        let server = configuration?.baseURL
        let key = "gallery-preview-\(server?.absoluteString ?? "")-\(message.id)-\(message.editRevision)"
        if let url = try? await mediaCache.cachedURL(for: key),
           let data = try? Data(contentsOf: url), let image = PlatformImage(data: data) { return image }
        // Opening the gallery asks for a whole screen of previews at once. The server renders
        // two at a time, so an unthrottled burst left the first tiles queued past the request
        // timeout and they stayed blank. Asking a few at a time keeps every request short.
        await acquireGalleryThumbnailSlot()
        defer { releaseGalleryThumbnailSlot() }
        try Task.checkCancellation()
        let response = try await api.mediaThumbnail(messageID: message.id)
        guard phase == .ready, configuration?.baseURL == server, !Task.isCancelled else { throw CancellationError() }
        guard let data = Data(base64Encoded: response.mediaData), let image = PlatformImage(data: data) else {
            throw APIError.decoding("This preview could not be opened.")
        }
        _ = try? await mediaCache.store(data, messageID: key, fileExtension: "jpg")
        return image
    }

    private func acquireGalleryThumbnailSlot() async {
        guard activeGalleryThumbnailRequests >= Self.maximumGalleryThumbnailRequests else {
            activeGalleryThumbnailRequests += 1
            return
        }
        // The releasing request hands its slot straight to the next waiter, oldest first, so the
        // tiles on screen when the gallery opened are fetched before ones further down.
        await withCheckedContinuation { galleryThumbnailWaiters.append($0) }
    }

    private func releaseGalleryThumbnailSlot() {
        if galleryThumbnailWaiters.isEmpty {
            activeGalleryThumbnailRequests -= 1
        } else {
            galleryThumbnailWaiters.removeFirst().resume()
        }
    }

    func retryMedia(for message: ChatMessage) async {
        messageImages.removeValue(forKey: message.id)
        messageMediaURLs.removeValue(forKey: message.id)
        mediaErrorIDs.remove(message.id)
        try? await mediaCache.remove(messageID: message.id)
        await loadMedia(for: message)
    }

    func loadLinkPreview(for url: URL) async {
        guard linkPreviews[url] == nil, !linkPreviewRequests.contains(url) else { return }
        linkPreviewRequests.insert(url)
        defer { linkPreviewRequests.remove(url) }
        if demoMode {
            linkPreviews[url] = LinkPreview(
                url: url,
                title: "A place worth visiting this weekend",
                description: "A quick guide with opening times, directions, and the best things to see.",
                imageURL: nil,
                siteName: url.host(),
                error: nil
            )
            return
        }
        if let preview = try? await api.linkPreview(for: url), preview.error == nil {
            linkPreviews[url] = preview
        }
    }

    /// Pixel size to reserve for a photo or sticker before it loads: the server's dimensions when
    /// it sent them, otherwise the size remembered from the last time the image was shown.
    func reservedImageSize(for message: ChatMessage) -> CGSize? {
        if let width = message.content?.width, let height = message.content?.height, width > 0, height > 0 {
            return CGSize(width: width, height: height)
        }
        return rememberedImageSizes[message.id]
    }

    private func restoreCachedMedia(from url: URL, for message: ChatMessage) async -> Bool {
        switch message.mediaKind {
        case .image, .sticker:
            // Read the file away from the main thread so scrolling back stays smooth.
            let data = await Task.detached(priority: .userInitiated) { try? Data(contentsOf: url) }.value
            guard let data, let image = await Self.displayReadyImage(from: data) else {
                return false
            }
            showImage(image, for: message)
            return true
        case .audio:
            guard url.pathExtension == "mp3" else { return false }
            messageMediaURLs[message.id] = url
            return true
        case .video, .document:
            messageMediaURLs[message.id] = url
            return true
        case nil:
            return false
        }
    }

    private func showImage(_ image: PlatformImage, for message: ChatMessage) {
        messageImages[message.id] = image
        guard message.content?.width == nil || message.content?.height == nil, !demoMode else { return }
        let size = image.size
        guard size.width > 0, size.height > 0, rememberedImageSizes[message.id] != size else { return }
        rememberedImageSizes[message.id] = size
        Task { await mediaCache.rememberImageSize(size, messageID: message.id) }
    }

    /// Decodes an image and, on iOS, prepares it for display in the background, so the first
    /// frame that shows it doesn't stall the main thread while the bubble scrolls into view.
    private static func displayReadyImage(from data: Data) async -> PlatformImage? {
        guard let image = PlatformImage(data: data) else { return nil }
        #if canImport(UIKit) && !os(watchOS)
        return await image.byPreparingForDisplay() ?? image
        #else
        return image
        #endif
    }

    func storeMedia(_ data: Data, mimeType: String?, for message: ChatMessage) async {
        let preferredExtension = (message.mediaKind == .audio && mimeType == "audio/mpeg" ? "mp3" : nil) ?? message.content?.fileName
            .flatMap { URL(fileURLWithPath: $0).pathExtension.nilIfBlank }
            ?? mimeType.flatMap { UTType(mimeType: $0)?.preferredFilenameExtension }
            ?? defaultMediaExtension(for: message.mediaKind)

        switch message.mediaKind {
        case .image, .sticker:
            guard let image = await Self.displayReadyImage(from: data) else {
                mediaErrorIDs.insert(message.id)
                return
            }
            showImage(image, for: message)
            do {
                _ = try await mediaCache.store(
                    data,
                    messageID: message.id,
                    fileExtension: preferredExtension
                )
            } catch {
                AppLog.media.error("Media cache write failed: \(AppLog.kind(of: error), privacy: .public)")
            }
        case .video, .audio, .document:
            do {
                let url = try await mediaCache.store(
                    data,
                    messageID: message.id,
                    fileExtension: preferredExtension
                )
                messageMediaURLs[message.id] = url
            } catch {
                mediaErrorIDs.insert(message.id)
            }
        case nil:
            break
        }
    }

    private func defaultMediaExtension(for kind: MessageMediaKind?) -> String {
        switch kind {
        case .image: "jpg"
        case .sticker: "webp"
        case .video: "mp4"
        case .audio: "m4a"
        case .document, nil: "bin"
        }
    }
}

private extension String {
    var nilIfBlank: String? {
        trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : self
    }

}
