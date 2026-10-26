import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    func loadMessages(for contactID: String, older: Bool = false) async {
        if demoMode { return }
        do {
            let current = messages[contactID] ?? []
            let oldest = older ? current.first : nil
            let response = try await api.messages(
                contactID: contactID,
                before: oldest?.timestamp,
                beforeID: oldest?.id
            )
            messages[contactID] = older ? normalizeMessages(response.messages + current) : normalizeMessages(response.messages)
            messageHistoryHasMore[contactID] = response.hasMore
            await persistCache()
            await refreshMessageTopics(response.messages)
        } catch {
            guard !Self.isExpectedCancellation(error) else { return }
            presentError("Couldn’t load messages", error)
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

    func loadAvatar(for contactID: String) async {
        guard !demoMode,
              avatarURLs[contactID] == nil,
              !avatarRequests.contains(contactID) else { return }
        avatarRequests.insert(contactID)
        defer { avatarRequests.remove(contactID) }
        if let url = try? await api.avatar(contactID: contactID) {
            avatarURLs[contactID] = url
        }
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
