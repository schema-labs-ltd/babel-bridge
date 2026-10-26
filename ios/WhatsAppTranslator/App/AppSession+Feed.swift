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
