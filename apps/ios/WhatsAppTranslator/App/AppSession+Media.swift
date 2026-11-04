import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
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
