import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    /// Returns whether a page was fetched and applied.
    @discardableResult
    func loadMessages(for contactID: String, older: Bool = false, quietly: Bool = false) async -> Bool {
        if demoMode { return false }
        do {
            let current = messages[contactID] ?? []
            let oldest = older ? current.first : nil
            let response = try await api.messages(
                contactID: contactID,
                before: oldest?.timestamp,
                beforeID: oldest?.id
            )
            // Merge an older page into the list as it is now, so messages that arrived during the
            // request are kept rather than rolled back to the list captured before it.
            messages[contactID] = older
                ? normalizeMessages(response.messages + (messages[contactID] ?? current))
                : normalizeMessages(response.messages)
            messageHistoryHasMore[contactID] = response.hasMore
            await persistCache()
            await refreshMessageTopics(response.messages)
            return true
        } catch {
            guard !quietly, !Self.isExpectedCancellation(error) else { return false }
            presentError("Couldn’t load messages", error)
            return false
        }
    }

    func loadAllMessages(for contactID: String) async {
        guard !demoMode, !loadingCompleteHistoryContactIDs.contains(contactID) else { return }
        loadingCompleteHistoryContactIDs.insert(contactID)
        defer { loadingCompleteHistoryContactIDs.remove(contactID) }
        do {
            let response = try await api.messages(contactID: contactID, limit: 0)
            messages[contactID] = normalizeMessages(response.messages + (messages[contactID] ?? []))
            messageHistoryHasMore[contactID] = false
            await persistCache()
            await refreshMessageTopics(response.messages)
        } catch {
            guard !Self.isExpectedCancellation(error) else { return }
            presentError("Couldn’t load the full conversation", error)
        }
    }

    /// Shows the profile picture saved on this device straight away, then checks the server
    /// once per launch and saves a changed picture for next time.
    func loadAvatar(for contactID: String) async {
        guard !demoMode, !avatarRequests.contains(contactID) else { return }
        if refreshedAvatarIDs.contains(contactID) {
            // Photos saved later can evict the saved picture; fetch it again if so.
            guard let url = avatarURLs[contactID], url.isFileURL,
                  !FileManager.default.fileExists(atPath: url.path) else { return }
            refreshedAvatarIDs.remove(contactID)
            avatarURLs.removeValue(forKey: contactID)
        }
        avatarRequests.insert(contactID)
        defer { avatarRequests.remove(contactID) }
        // Signing out or switching server starts a new epoch and clears the media cache. Check it
        // after every wait so a late answer can't put the old account's picture back.
        let epoch = topicAccountEpoch
        var current: Bool { topicAccountEpoch == epoch && !Task.isCancelled }
        let key = "avatar-\(contactID)"
        let cachedURL = try? await mediaCache.cachedURL(for: key)
        guard current else { return }
        if avatarURLs[contactID] == nil, let cachedURL { avatarURLs[contactID] = cachedURL }
        let remote: URL?
        do { remote = try await api.avatar(contactID: contactID) } catch { return }
        // The server also answers with no picture while WhatsApp reconnects or is slow, so keep
        // the saved one and check again next time rather than treat it as removed.
        guard current, let remote else { return }
        let download = try? await URLSession.shared.data(from: remote)
        // A row that scrolled away cancels the download; it is checked again when shown.
        guard current else { return }
        refreshedAvatarIDs.insert(contactID)
        guard let data = download?.0,
              (download?.1 as? HTTPURLResponse)?.statusCode == 200,
              PlatformImage(data: data) != nil else {
            if cachedURL == nil { avatarURLs[contactID] = remote }
            return
        }
        if let cachedURL {
            let saved = await Task.detached(priority: .utility) { try? Data(contentsOf: cachedURL) }.value
            guard current else {
                // Not compared or saved, so check this contact again when it is shown.
                if topicAccountEpoch == epoch { refreshedAvatarIDs.remove(contactID) }
                return
            }
            if saved == data { return }
        }
        let storedURL = try? await mediaCache.store(data, messageID: key, fileExtension: "jpg")
        guard topicAccountEpoch == epoch else {
            // Sign-out cleared the cache while this was being written.
            try? await mediaCache.remove(messageID: key)
            return
        }
        // A file that changed keeps its path, which the picture view wouldn't reload, so this
        // launch shows the new picture from the server and the next one from the saved file.
        avatarURLs[contactID] = cachedURL == nil ? storedURL ?? remote : remote
    }

    func togglePin(_ contact: Contact) async {
        do {
            let pinned: Bool
            if demoMode {
                pinned = contact.pinnedAt == nil
            } else {
                pinned = try await api.togglePin(contactID: contact.id)
            }
            guard let index = contacts.firstIndex(where: { $0.id == contact.id }) else { return }
            let current = contacts[index]
            contacts[index] = Contact(
                id: current.id,
                name: current.name,
                phone: current.phone,
                type: current.type,
                lastMessageTime: current.lastMessageTime,
                unreadCount: current.unreadCount,
                pinnedAt: pinned ? Int64(Date().timeIntervalSince1970 * 1_000) : nil,
                lastMessagePreview: current.lastMessagePreview
            )
            contacts = Self.orderedContacts(contacts)
            await persistCache()
        } catch {
            presentError(
                contact.pinnedAt == nil ? "Couldn’t pin conversation" : "Couldn’t unpin conversation",
                error
            )
        }
    }

    func send(text: String, to contactID: String, reply: MessageReplyTarget? = nil, replyOnlyIfNotLatest: Bool = false) async -> Bool {
        guard checkReplyOnly(contactID, reply: reply) else { return false }
        let clean = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return false }
        sendingContactIDs.insert(contactID)
        defer { sendingContactIDs.remove(contactID) }

        if demoMode {
            var quotedReply = reply
            if replyOnlyIfNotLatest, let reply {
                guard let target = ((messages[contactID] ?? []) + Array(feedByID.values)).first(where: { $0.id == reply.messageID && $0.contactId == contactID }) else { return false }
                if !feedReplyNeedsQuote(target) { quotedReply = nil }
            }
            let message = ChatMessage.demoOutgoing(contactID: contactID, text: clean, reply: quotedReply)
            messages[contactID, default: []].append(message)
            feedByID[message.id] = message
            updateContactPreview(contactID: contactID, preview: "You: \(clean)", timestamp: message.timestamp)
            return true
        }

        do {
            let response = try await api.send(contactID: contactID, text: clean, reply: reply, replyOnlyIfNotLatest: replyOnlyIfNotLatest)
            if let contact = contacts.first(where: { $0.id == contactID }) {
                MessagingIntentDonor.donateOutgoing(
                    originalText: clean,
                    response: response,
                    contact: contact,
                    displayName: displayName(for: contact)
                )
            }
            await loadMessages(for: contactID)
            await loadFeed()
            await refresh()
            return true
        } catch {
            presentError("Couldn’t send message", error)
            return false
        }
    }

    func sendAttachment(_ attachment: OutgoingAttachment, caption: String?, to contactID: String, reply: MessageReplyTarget?, replyOnlyIfNotLatest: Bool = false) async -> Bool {
        guard checkReplyOnly(contactID, reply: reply) else { return false }
        guard !attachment.data.isEmpty, attachment.data.count <= OutgoingAttachment.maximumBytes,
              !sendingContactIDs.contains(contactID) else { return false }
        sendingContactIDs.insert(contactID)
        defer { sendingContactIDs.remove(contactID) }
        if demoMode {
            let quote = replyOnlyIfNotLatest ? conditionalDemoReply(reply, contactID: contactID) : reply
            let message = demoAttachmentMessage(kind: attachment.kind, data: attachment.data, mimeType: attachment.mimeType, fileName: attachment.fileName, caption: caption, contactID: contactID, reply: quote)
            messages[contactID, default: []].append(message)
            feedByID[message.id] = message
            await storeMedia(attachment.data, mimeType: attachment.mimeType, for: message)
            return true
        }
        do {
            _ = try await api.sendAttachment(attachment, contactID: contactID, caption: caption, reply: reply, replyOnlyIfNotLatest: replyOnlyIfNotLatest)
            await loadMessages(for: contactID)
            await loadFeed()
            await refresh()
            return true
        } catch {
            presentError("Couldn’t send attachment", error)
            return false
        }
    }

    func conditionalDemoReply(_ reply: MessageReplyTarget?, contactID: String) -> MessageReplyTarget? {
        guard let reply, let target = unifiedMessages.first(where: { $0.id == reply.messageID && $0.contactId == contactID }) else { return nil }
        return feedReplyNeedsQuote(target) ? reply : nil
    }

    func demoAttachmentMessage(kind: String, data: Data, mimeType: String, fileName: String?, caption: String?, contactID: String, reply: MessageReplyTarget?) -> ChatMessage {
        var content: [String: Any] = ["type": kind, "mime_type": mimeType, "media_data": data.base64EncodedString(), "file_size": data.count]
        content["file_name"] = fileName
        content["caption"] = caption
        if let reply { content["reply_context"] = ["messageId": reply.messageID, "senderName": reply.senderName, "text": reply.text] }
        let json: [String: Any] = ["id": UUID().uuidString, "contactId": contactID, "timestamp": Int64(Date().timeIntervalSince1970 * 1000), "isFromMe": true, "isForwarded": false, "chatType": "private", "contentType": kind.capitalized, "isTranslated": false, "deliveryStatus": "sent", "content": content]
        return try! JSONDecoder().decode(ChatMessage.self, from: JSONSerialization.data(withJSONObject: json))
    }
}

extension AppSession {
    /// Whether WhatsApp's Forward can resend this message from here: text, photos, videos and files.
    nonisolated static func canForward(_ message: ChatMessage) -> Bool {
        guard !message.isReaction else { return false }
        switch message.mediaKind {
        case .image, .video, .document: return true
        case .audio, .sticker: return false
        case nil:
            return ["text", "conversation", "extendedtextmessage"].contains(message.normalizedContentType)
                && !message.displayText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    /// Sends the messages to each chosen chat in their original order, as WhatsApp's Forward
    /// does; up to 30 photos with at most a first caption go as one album. Returns how many
    /// chats received all of them.
    @discardableResult
    func forward(_ originals: [ChatMessage], to contactIDs: [String]) async -> Int {
        let items = originals.filter(Self.canForward).sorted { $0.timestamp < $1.timestamp }
        guard !items.isEmpty, !contactIDs.isEmpty else { return 0 }
        let epoch = topicAccountEpoch
        var current: Bool { topicAccountEpoch == epoch && !Task.isCancelled }
        var media: [String: (data: Data, mimeType: String)] = [:]
        // Loads a file, unless the forward has already failed or the account changed meanwhile.
        func file(for message: ChatMessage) async -> (data: Data, mimeType: String)? {
            if let loaded = media[message.id] { return loaded }
            do {
                let loaded = try await forwardMedia(for: message)
                return current ? loaded : nil
            } catch {
                if current { presentError("Couldn’t forward", error) }
                return nil
            }
        }
        var reached = Set(contactIDs)
        // An album needs every photo at once (at most 30, within 64 MB).
        if Self.forwardsAsAlbum(items, sizes: items.map { _ in 0 }) {
            // Stop loading once the photos can't fit, so the fallback below needn't start
            // with every file in memory; it loads the rest one at a time.
            var total = 0
            for message in items {
                guard let loaded = await file(for: message) else { return 0 }
                media[message.id] = loaded
                total += loaded.data.count
                if loaded.data.count > 16 * 1_024 * 1_024 || total > 64 * 1_024 * 1_024 { break }
            }
            if Self.forwardsAsAlbum(items, sizes: items.compactMap { media[$0.id]?.data.count }) {
                let images = items.compactMap { media[$0.id].map { OutgoingImage(data: $0.data, mimeType: $0.mimeType) } }
                for contactID in contactIDs {
                    guard current else { return 0 }
                    if !(await sendImages(images, caption: Self.forwardedCaption(items[0]), to: contactID)) {
                        reached.remove(contactID)
                    }
                }
                return finishForward(items, to: contactIDs, reached: reached, current: current)
            }
        }
        // Otherwise each message goes to every chat in turn, holding only its own file, so a
        // long run never keeps all its files in memory.
        for message in items {
            guard current, !reached.isEmpty else { break }
            var loaded: (data: Data, mimeType: String)?
            if message.mediaKind != nil {
                loaded = await file(for: message)
                media[message.id] = nil
                guard loaded != nil else { return 0 }
            }
            let caption = Self.forwardedCaption(message)
            for contactID in contactIDs where reached.contains(contactID) {
                guard current else { return 0 }
                let sent: Bool
                switch (message.mediaKind, loaded) {
                case (.image, let file?):
                    sent = await sendImage(data: file.data, mimeType: file.mimeType, caption: caption, to: contactID)
                case (.video, let file?), (.document, let file?):
                    let isVideo = message.mediaKind == .video
                    let attachment = OutgoingAttachment(
                        data: file.data,
                        mimeType: file.mimeType,
                        fileName: message.content?.fileName?.nilIfBlank ?? (isVideo ? "video.mp4" : "Document"),
                        kind: isVideo ? "video" : "document"
                    )
                    sent = await sendAttachment(attachment, caption: caption, to: contactID, reply: nil)
                default:
                    sent = await send(text: Self.forwardedText(message), to: contactID)
                }
                if !sent { reached.remove(contactID) }
            }
        }
        return finishForward(items, to: contactIDs, reached: current ? reached : [], current: current)
    }

    /// Media already on this device, or carried in the message, needs no download.
    private func forwardMedia(for message: ChatMessage) async throws -> (data: Data, mimeType: String) {
        let mimeType = message.content?.mimeType?.nilIfBlank
        if let mimeType, let url = try? await mediaCache.cachedURL(for: message.id),
           let data = await Task.detached(operation: { try? Data(contentsOf: url) }).value, !data.isEmpty {
            return (data, mimeType)
        }
        if let mimeType, let base64 = message.content?.mediaData, let data = Data(base64Encoded: base64) {
            return (data, mimeType)
        }
        let response = try await api.media(messageID: message.id)
        guard let data = Data(base64Encoded: response.mediaData) else {
            throw APIError.decoding("The media payload was invalid.")
        }
        return (data, response.mimeType)
    }

    private func finishForward(_ items: [ChatMessage], to contactIDs: [String], reached: Set<String>, current: Bool) -> Int {
        guard current else { return 0 }
        // Photo sends refresh only their chat; the Messages feed needs them too.
        if !reached.isEmpty, items.contains(where: { $0.mediaKind == .image }) { Task { await loadFeed() } }
        if reached.count < contactIDs.count, errorMessage == nil {
            presentError("Couldn’t forward", "The message didn’t reach every chat you chose. Try again in a moment.")
        }
        return reached.count
    }

    /// Several photos go as one album only when the album keeps every caption: at most 30
    /// photos, and none but the first captioned. Otherwise each goes on its own.
    /// An album must also fit `sendImages`' limits: 16 MB a photo and 64 MB in all.
    nonisolated static func forwardsAsAlbum(_ items: [ChatMessage], sizes: [Int]) -> Bool {
        (2...30).contains(items.count)
            && sizes.count == items.count
            && sizes.allSatisfy { $0 <= 16 * 1_024 * 1_024 }
            && sizes.reduce(0, +) <= 64 * 1_024 * 1_024
            && items.allSatisfy { $0.mediaKind == .image }
            && items.dropFirst().allSatisfy { forwardedCaption($0) == nil }
    }

    /// The words as they were written, not a translation of them, so the chosen chat's own
    /// translation setting applies as for anything you send. `originalText` holds what you
    /// typed when your message was translated, and the sender's words for incoming messages.
    nonisolated static func forwardedText(_ message: ChatMessage) -> String {
        message.originalText?.nilIfBlank ?? message.content?.body?.nilIfBlank ?? message.displayText
    }

    nonisolated static func forwardedCaption(_ message: ChatMessage) -> String? {
        message.originalText?.nilIfBlank ?? message.content?.caption?.nilIfBlank
    }
}

private extension String {
    var nilIfBlank: String? { trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : self }
}
