import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    /// Returns whether a page was fetched and applied.
    @discardableResult
    func loadMessages(for contactID: String, older: Bool = false) async -> Bool {
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
            guard !Self.isExpectedCancellation(error) else { return false }
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
    /// does; several photos go as one album. Returns how many chats received all of them.
    @discardableResult
    func forward(_ originals: [ChatMessage], to contactIDs: [String]) async -> Int {
        let items = originals.filter(Self.canForward).sorted { $0.timestamp < $1.timestamp }
        guard !items.isEmpty else { return 0 }
        var media: [String: (data: Data, mimeType: String)] = [:]
        for message in items where message.mediaKind != nil {
            do {
                // Media already on this device, or carried in the message, needs no download.
                let mimeType = message.content?.mimeType?.nilIfBlank
                if let mimeType, let url = try? await mediaCache.cachedURL(for: message.id),
                   let data = try? Data(contentsOf: url), !data.isEmpty {
                    media[message.id] = (data, mimeType)
                    continue
                }
                if let mimeType, let base64 = message.content?.mediaData, let data = Data(base64Encoded: base64) {
                    media[message.id] = (data, mimeType)
                    continue
                }
                let response = try await api.media(messageID: message.id)
                guard let data = Data(base64Encoded: response.mediaData) else {
                    throw APIError.decoding("The media payload was invalid.")
                }
                media[message.id] = (data, response.mimeType)
            } catch {
                presentError("Couldn’t forward", error)
                return 0
            }
        }
        var delivered = 0
        for contactID in contactIDs {
            var sentAll = true
            if items.count > 1, items.allSatisfy({ $0.mediaKind == .image }) {
                let images = items.compactMap { media[$0.id].map { OutgoingImage(data: $0.data, mimeType: $0.mimeType) } }
                let caption = items.lazy.compactMap { $0.content?.caption?.nilIfBlank }.first
                sentAll = await sendImages(images, caption: caption, to: contactID)
            } else {
                for message in items where sentAll {
                    let caption = message.content?.caption?.nilIfBlank
                    switch message.mediaKind {
                    case .image:
                        guard let file = media[message.id] else { sentAll = false; break }
                        sentAll = await sendImage(data: file.data, mimeType: file.mimeType, caption: caption, to: contactID)
                    case .video, .document:
                        guard let file = media[message.id] else { sentAll = false; break }
                        let isVideo = message.mediaKind == .video
                        let attachment = OutgoingAttachment(
                            data: file.data,
                            mimeType: file.mimeType,
                            fileName: message.content?.fileName?.nilIfBlank ?? (isVideo ? "video.mp4" : "Document"),
                            kind: isVideo ? "video" : "document"
                        )
                        sentAll = await sendAttachment(attachment, caption: caption, to: contactID, reply: nil)
                    default:
                        sentAll = await send(text: Self.forwardedText(message), to: contactID)
                    }
                }
            }
            if sentAll { delivered += 1 }
        }
        if delivered < contactIDs.count, errorMessage == nil {
            presentError("Couldn’t forward", "The message didn’t reach every chat you chose. Try again in a moment.")
        }
        return delivered
    }

    /// The words as they were written: an incoming message's original, not this app's translation
    /// of it, so the chosen chat's own translation setting applies as for anything you send.
    nonisolated static func forwardedText(_ message: ChatMessage) -> String {
        guard !message.isFromMe else { return message.displayText }
        return message.content?.body?.nilIfBlank ?? message.originalText?.nilIfBlank ?? message.displayText
    }
}

private extension String {
    var nilIfBlank: String? { trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : self }
}
