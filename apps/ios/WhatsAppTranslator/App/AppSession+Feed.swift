import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    var unifiedMessages: [ChatMessage] {
        var result = feedByID
        for message in messages.values.joined() where result[message.id] != nil {
            result[message.id] = message
        }
        return result.values.filter { !$0.isReaction && $0.contactId != "status@broadcast" }.sorted {
            $0.timestamp == $1.timestamp ? $0.id < $1.id : $0.timestamp < $1.timestamp
        }
    }

    func feedReplyNeedsQuote(_ message: ChatMessage) -> Bool {
        if isReplyOnly(message.contactId) { return true }
        let conversation = (messages[message.contactId] ?? []) + feedByID.values.filter { $0.contactId == message.contactId }
        return conversation.contains {
            !$0.isReaction && ($0.timestamp > message.timestamp || ($0.timestamp == message.timestamp && $0.id > message.id))
        }
    }

    func loadFeed(older: Bool = false) async {
        guard !feedLoading, !older || feedHasMore else { return }
        if demoMode {
            feedByID = Dictionary(messages.values.joined().map { ($0.id, $0) }, uniquingKeysWith: { _, new in new })
            feedHasLoaded = true
            return
        }
        feedLoading = true
        feedEventsDuringLoad = []
        feedError = nil
        let generation = feedGeneration
        defer { if generation == feedGeneration { feedLoading = false } }
        do {
            let cursor = older ? feedCursor : nil
            let response = try await api.feed(before: cursor?.timestamp, beforeID: cursor?.id)
            guard generation == feedGeneration else { return }
            // Refresh starts a contiguous window; older pages extend only this server cursor.
            var next = older ? feedByID : [:]
            for message in normalizeMessages(response.messages) { next[message.id] = message }
            // Events received while the request was in flight are newer than its snapshot.
            for id in feedEventsDuringLoad {
                if let live = feedByID[id] { next[id] = live }
            }
            feedByID = next
            feedHasLoaded = true
            feedCursor = response.messages.first
            feedHasMore = response.hasMore
            let pageMessages = response.messages.map { next[$0.id] ?? $0 }
            for (id, page) in Dictionary(grouping: pageMessages, by: \.contactId) {
                messages[id] = normalizeMessages((messages[id] ?? []) + page)
            }
            await persistCache()
            await refreshMessageTopics(response.messages)
        } catch {
            guard generation == feedGeneration, !Self.isExpectedCancellation(error) else { return }
            feedError = error.localizedDescription
        }
    }

    func recordMessageViews(_ values: [ChatMessage]) async {
        let eligible = values.filter(\.canSendReadReceipt)
        for message in eligible where !viewedReceiptMessageIDs.contains(message.id) {
            pendingReadReceipts[message.contactId, default: []].insert(message.id)
            viewedReceiptMessageIDs.insert(message.id)
        }
        if demoMode { return }
        await persistCache()
        startReadReceiptRetry()
    }

    func startReadReceiptRetry() {
        guard readReceiptTask == nil, !pendingReadReceipts.isEmpty, phase == .ready, !demoMode else { return }
        let epoch = topicAccountEpoch
        readReceiptTask = Task { [weak self] in
            guard let self else { return }
            defer { if self.topicAccountEpoch == epoch { self.readReceiptTask = nil } }
            while !Task.isCancelled, self.topicAccountEpoch == epoch, self.phase == .ready, !self.pendingReadReceipts.isEmpty {
                for (contactID, values) in self.pendingReadReceipts {
                    let ids = Array(values.prefix(200))
                    do {
                        let response = try await self.api.markViewedMessages(contactID: contactID, messageIDs: ids)
                        guard self.topicAccountEpoch == epoch, !Task.isCancelled else { return }
                        if response.success {
                            self.pendingReadReceipts[contactID]?.subtract(ids)
                            if self.pendingReadReceipts[contactID]?.isEmpty == true { self.pendingReadReceipts.removeValue(forKey: contactID) }
                            self.updateViewedUnreadCount(contactID: contactID, count: response.unreadCount)
                            await self.persistCache()
                        }
                    } catch { /* Keep already-seen IDs for retry. No receipt originates in loading. */ }
                }
                if !self.pendingReadReceipts.isEmpty {
                    do { try await Task.sleep(for: .seconds(10)) } catch { return }
                }
            }
        }
    }

    func updateViewedUnreadCount(contactID: String, count: Int) {
        if let index = contacts.firstIndex(where: { $0.id == contactID }) { contacts[index].unreadCount = max(0, count) }
        #if os(iOS)
        // Preserve notifications for other, unseen messages in the same chat.
        if count == 0 {
            Task { await PushNotificationCoordinator.shared.markConversationRead(contactID: contactID, badgeCount: self.contacts.reduce(0) { $0 + max(0, $1.unreadCount) }) }
        }
        #endif
    }
}

extension AppSession {
    /// WhatsApp's "Mark as read" in the chat list: sends read receipts for the chat's unread
    /// messages, loading them first if the chat hasn't been opened yet.
    func markChatRead(_ contact: Contact) async {
        preferences.setMarkedUnread(false, for: contact.id)
        guard contact.unreadCount > 0 else { return }
        let epoch = topicAccountEpoch
        // Messages that can still take a receipt: ones viewed here already can't move the count on.
        func unread() -> [ChatMessage] {
            (messages[contact.id] ?? []).filter { $0.canSendReadReceipt && !viewedReceiptMessageIDs.contains($0.id) }
        }
        // The cache can miss messages that arrived while the app was closed, or not reach back to the
        // oldest unread one, so fetch the newest pages until they cover the unread count. Each page is
        // merged into the cache as it stands after the request, so live arrivals are kept.
        // Always fetch: a live arrival can make the cache look current while hiding an offline gap.
        // The newest message the server had when this started: later arrivals were not part of the
        // unread count, so they must not push the older unread messages out of the selection.
        // Timestamps are whole seconds, so messages sharing the boundary second count only if that
        // first page had them.
        var boundary: Int64?
        var boundaryIDs: Set<String> = []
        if !demoMode {
            var oldestFetched: ChatMessage?
            var fetchedUnread = 0
            do {
                for _ in 0..<5 {
                    let response = try await api.messages(
                        contactID: contact.id, before: oldestFetched?.timestamp, beforeID: oldestFetched?.id
                    )
                    // A switch of server while this ran must not let the old server's messages in.
                    guard topicAccountEpoch == epoch else { return }
                    if boundary == nil {
                        let newest = response.messages.map(\.timestamp).max() ?? 0
                        boundary = newest
                        boundaryIDs = Set(response.messages.filter { $0.timestamp == newest }.map(\.id))
                    }
                    messages[contact.id] = normalizeMessages((messages[contact.id] ?? []) + response.messages)
                    fetchedUnread += response.messages.filter {
                        $0.canSendReadReceipt && !viewedReceiptMessageIDs.contains($0.id)
                    }.count
                    guard response.hasMore, fetchedUnread < contact.unreadCount,
                          let oldest = response.messages.min(by: { ($0.timestamp, $0.id) < ($1.timestamp, $1.id) })
                    else { break }
                    oldestFetched = oldest
                }
            } catch {
                guard topicAccountEpoch == epoch, !Self.isExpectedCancellation(error) else { return }
                presentError("Couldn’t mark the chat as read", error)
                return
            }
        }
        guard topicAccountEpoch == epoch else { return }
        let candidates = unread().filter { message in
            guard let boundary else { return true }
            return message.timestamp < boundary || (message.timestamp == boundary && boundaryIDs.contains(message.id))
        }
        await recordMessageViews(Array(candidates.suffix(contact.unreadCount)))
    }
}
