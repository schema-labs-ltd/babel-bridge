import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    func messageTopicLabel(for message: ChatMessage) -> String {
        if demoMode {
            if let topic = demoTopicCatalog.topics.first(where: { topic in
                demoTopicPages[topic.id]?.messages.contains(where: { $0.id == message.id && $0.contactId == message.contactId }) == true
            }) { return "\(topic.title)" }
            return "Not categorised"
        }
        if let status = messageTopicStates[message.id], status.contactId == message.contactId, status.revision == message.editRevision {
            return status.menuLabel
        }
        if messageTopicFailures.contains(message.id) { return "Unavailable" }
        return "Checking…"
    }

    func refreshMessageTopics(_ values: [ChatMessage]) async {
        guard !demoMode else { return }
        let unique = Dictionary(values.filter { !$0.isReaction }.map { ($0.id, $0) }, uniquingKeysWith: { a, b in a.editRevision > b.editRevision ? a : b })
        let ids = unique.keys.sorted()
        let epoch = topicAccountEpoch
        let request = UUID()
        for id in ids { messageTopicRequests[id] = request }
        for start in stride(from: 0, to: ids.count, by: 200) {
            let batch = Array(ids[start..<min(start + 200, ids.count)])
            do {
                let response = try await api.messageTopics(ids: batch)
                guard epoch == topicAccountEpoch else { return }
                for status in response.topics {
                    guard messageTopicRequests[status.messageId] == request,
                          let message = unique[status.messageId], status.contactId == message.contactId,
                          status.revision == message.editRevision else { continue }
                    messageTopicStates[status.messageId] = status
                    messageTopicFailures.remove(status.messageId)
                }
            } catch {
                guard epoch == topicAccountEpoch else { return }
                for id in batch where messageTopicRequests[id] == request { messageTopicFailures.insert(id) }
            }
        }
    }

    func topics(for contactID: String? = nil) -> [ChatTopic] {
        guard let contactID else { return topicCatalog.unifiedTopics }
        return topicCatalog.topics.filter { $0.contactId == contactID }
    }

    func topicSetting(for contactID: String) -> TopicSetting {
        topicCatalog.settings.first { $0.contactId == contactID }
            ?? TopicSetting(contactId: contactID, enabled: false, pendingCount: 0, failedCount: 0)
    }

    func applyTopicCatalog(_ catalog: TopicCatalog) {
        topicCatalog = catalog
        let ids = catalog.allTopicIDs
        topicPages = topicPages.filter { ids.contains($0.key) }
        for id in topicPages.keys {
            let contacts = catalog.contactIDs(for: id)
            topicPages[id]?.messages.removeAll { !contacts.contains($0.contactId) }
        }
        topicRevision = UUID()
    }

    func loadTopics() async {
        guard !demoMode else { return }
        let request = UUID()
        topicCatalogRequest = request
        do {
            let catalog = try await api.topics()
            guard topicCatalogRequest == request else { return }
            applyTopicCatalog(catalog)
            topicCatalogError = nil
            await refreshMessageTopics(Array(feedByID.values) + messages.values.flatMap { $0 } + topicPages.values.flatMap { $0.messages })
            for id in Set(topicPages.keys).union(topicLoading) {
                await loadTopicMessages(id)
            }
        } catch {
            guard topicCatalogRequest == request, !Self.isExpectedCancellation(error) else { return }
            topicCatalogError = "Couldn’t load topics. Check your connection and try again."
        }
    }

    func setTopicsEnabled(_ enabled: Bool, contactID: String) async throws {
        topicCatalogRequest = UUID()
        if demoMode {
            topicCatalog.settings.removeAll { $0.contactId == contactID }
            topicCatalog.settings.append(TopicSetting(contactId: contactID, enabled: enabled, pendingCount: 0, failedCount: 0))
            let retained = topicCatalog.topics.filter { $0.contactId != contactID }
            let restored = enabled ? demoTopicCatalog.topics.filter { $0.contactId == contactID } : []
            applyTopicCatalog(TopicCatalog(topics: retained + restored, settings: topicCatalog.settings, available: true))
            for topic in restored { topicPages[topic.id] = demoTopicPages[topic.id] }
            return
        }
        let accountEpoch = topicAccountEpoch
        _ = try await api.setTopicsEnabled(enabled, contactID: contactID)
        guard topicAccountEpoch == accountEpoch else { return }
        await loadTopics()
    }

    func topicImportPreview() async throws -> TopicImportSummary {
        if demoMode { return TopicImportSummary(days: 7, chatCount: demoTopicCatalog.settings.count, messageCount: demoTopicPages.values.reduce(0) { $0 + $1.messages.count }) }
        return try await api.topicImportPreview()
    }

    func importRecentTopics() async throws -> TopicImportSummary {
        if demoMode {
            let summary = try await topicImportPreview()
            applyTopicCatalog(demoTopicCatalog)
            topicPages = demoTopicPages
            return summary
        }
        let accountEpoch = topicAccountEpoch
        let summary = try await api.importRecentTopics()
        guard topicAccountEpoch == accountEpoch else { throw CancellationError() }
        await loadTopics()
        return summary
    }

    func loadTopicMessages(_ id: String, older: Bool = false) async {
        guard topicCatalog.allTopicIDs.contains(id) else { return }
        if demoMode {
            let members = topicCatalog.topics.filter { $0.id == id || $0.categoryId == id }
            topicPages[id] = TopicPage(messages: normalizeMessages(members.flatMap { demoTopicPages[$0.id]?.messages ?? [] }))
            return
        }
        guard !topicLoading.contains(id) else {
            if !older { topicReloadAfterLoad.insert(id) }
            return
        }
        guard !demoMode, !older || topicPages[id]?.hasMore == true else { return }
        topicLoading.insert(id)
        let revision = topicRevision
        defer {
            topicLoading.remove(id)
            if topicReloadAfterLoad.remove(id) != nil { Task { await loadTopicMessages(id) } }
        }
        do {
            let cursor = older ? topicPages[id]?.messages.first : nil
            let response = try await api.topicMessages(id: id, before: cursor?.timestamp, beforeID: cursor?.id)
            guard revision == topicRevision else {
                if topicCatalog.allTopicIDs.contains(id) { topicReloadAfterLoad.insert(id) }
                return
            }
            let contacts = topicCatalog.contactIDs(for: id)
            guard !contacts.isEmpty, response.messages.allSatisfy({ contacts.contains($0.contactId) }) else { return }
            let incoming = normalizeMessages(response.messages)
            // An edit received during this HTTP request invalidates the old assignment.
            let valid = incoming.filter { updated in response.messages.contains { $0.id == updated.id && $0.editRevision == updated.editRevision } }
            topicPages[id] = TopicPage(messages: normalizeMessages((older ? topicPages[id]?.messages ?? [] : []) + valid), hasMore: response.hasMore)
            for message in valid {
                messages[message.contactId] = normalizeMessages((messages[message.contactId] ?? []) + [message])
            }
            await refreshMessageTopics(response.messages)
        } catch {
            guard revision == topicRevision, !Self.isExpectedCancellation(error) else { return }
            var page = topicPages[id] ?? TopicPage()
            page.error = "Couldn’t load this topic. Try again."
            topicPages[id] = page
        }
    }

    func invalidateTopicMembership(for message: ChatMessage) {
        for id in Array(topicPages.keys) {
            topicPages[id]?.messages.removeAll { $0.id == message.id && $0.contactId == message.contactId && $0.editRevision < message.editRevision }
        }
    }
}
