import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    func normalizeMessages(_ values: [ChatMessage]) -> [ChatMessage] {
        for reaction in values where reaction.isReaction {
            guard let targetID = reaction.content?.targetMessageId else { continue }
            let key = ReactionTarget(contactID: reaction.contactId, messageID: targetID)
            let actor = reaction.isFromMe ? "me" : (reaction.senderPhone ?? reaction.senderName ?? "unknown")
            if let previous = reactionEvents[key]?[actor],
               (previous.timestamp, previous.id) >= (reaction.timestamp, reaction.id) { continue }
            reactionEvents[key, default: [:]][actor] = MessageReactionState(id: reaction.id, timestamp: reaction.timestamp, emoji: reaction.content?.emoji ?? "")
        }

        var display: [ReactionTarget: ChatMessage] = [:]
        for var message in values where !message.isReaction {
            let key = ReactionTarget(contactID: message.contactId, messageID: message.id)
            // An in-flight page or replay may predate an edit already shown live.
            let cached = [display[key], messages[message.contactId]?.first(where: { $0.id == message.id }),
                          feedByID[message.id]].compactMap { $0 }.filter { $0.contactId == message.contactId }
            if let latest = cached.max(by: { $0.editRevision < $1.editRevision }),
               latest.editRevision > message.editRevision { message = latest }
            for (actor, state) in message.reactionStates ?? [:] {
                if let previous = reactionEvents[key]?[actor],
                   (previous.timestamp, previous.id) >= (state.timestamp, state.id) { continue }
                reactionEvents[key, default: [:]][actor] = state
            }
            // Message updates and feed pages contain message fields, not reaction events.
            // Preserve cached aggregates before applying the newest per-person changes.
            if message.reactions == nil {
                message.reactions = display[key]?.reactions
                    ?? messages[message.contactId]?.first(where: { $0.id == message.id })?.reactions
                    ?? feedByID[message.id].flatMap { $0.contactId == message.contactId ? $0.reactions : nil }
            }
            if let events = reactionEvents[key] {
                var reactions = message.reactions ?? [:]
                for actor in events.keys.sorted() {
                    for emoji in Array(reactions.keys) {
                        reactions[emoji]?.removeAll { $0 == actor }
                        if reactions[emoji]?.isEmpty == true { reactions.removeValue(forKey: emoji) }
                    }
                    if let emoji = events[actor]?.emoji, !emoji.isEmpty {
                        reactions[emoji, default: []].append(actor)
                    }
                }
                message.reactions = reactions
                message.reactionStates = events
            }
            display[key] = message
        }
        return display.values.sorted { ($0.timestamp, $0.id) < ($1.timestamp, $1.id) }
    }

    func replaceMessage(
        _ messageID: String,
        in contactID: String,
        transform: (ChatMessage) -> ChatMessage
    ) {
        guard let index = messages[contactID]?.firstIndex(where: { $0.id == messageID }),
              let current = messages[contactID]?[index] else { return }
        messages[contactID]?[index] = transform(current)
        persistCacheSoon()
    }

    var ownSenderJID: String? {
        guard let phone = backendStatus.phone?.filter(\.isNumber), !phone.isEmpty else { return nil }
        return "\(phone)@s.whatsapp.net"
    }

    func updateContactPreview(contactID: String, preview: String, timestamp: Int64) {
        guard let index = contacts.firstIndex(where: { $0.id == contactID }) else { return }
        let old = contacts[index]
        guard timestamp >= old.lastMessageTime else { return }
        contacts[index] = Contact(
            id: old.id,
            name: old.name,
            phone: old.phone,
            type: old.type,
            lastMessageTime: timestamp,
            unreadCount: old.unreadCount,
            pinnedAt: old.pinnedAt,
            lastMessagePreview: preview
        )
        contacts = Self.orderedContacts(contacts)
    }

    func persistCache() async {
        guard let snapshot = cacheSnapshot() else { return }
        await cache.save(snapshot)
    }

    func persistCacheSoon() {
        guard let snapshot = cacheSnapshot() else { return }
        Task { await cache.save(snapshot) }
    }

    private func cacheSnapshot() -> ChatCacheSnapshot? {
        guard let configuration else { return nil }
        let trimmedMessages = messages.mapValues {
            Array($0.suffix(ChatCacheSnapshot.maximumMessagesPerChat))
        }
        return ChatCacheSnapshot(
            serverBaseURL: configuration.baseURL.absoluteString,
            contacts: contacts,
            messages: trimmedMessages,
            updatedAt: Date(),
            pendingReadReceipts: pendingReadReceipts.mapValues { Array($0).sorted() }
        )
    }
}
