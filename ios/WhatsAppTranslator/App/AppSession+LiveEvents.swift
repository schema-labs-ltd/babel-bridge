import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    func handle(_ event: LiveEvent) {
        guard phase == .ready else { return }
        switch event.type {
        case "conversation_settings_updated":
            if let contactID = event.chatId, let settings = event.settings {
                conversationSettingsRevision += 1
                savedConversationSettings[contactID] = settings
            }
        case "message", "reaction", "message_updated":
            if recoveryTask != nil { recoveryNeedsAnotherPass = true }
            guard let message = event.message else { return }
            let affectedID = message.isReaction ? message.content?.targetMessageId : message.id
            var conversation = messages[message.contactId] ?? []
            // A feed message can be outside the conversation's currently loaded page.
            if let affectedID, let target = feedByID[affectedID],
               target.contactId == message.contactId,
               !conversation.contains(where: { $0.id == affectedID }) {
                conversation.append(target)
            }
            let normalized = normalizeMessages(conversation + [message])
            messages[message.contactId] = normalized
            invalidateTopicMembership(for: message)
            for id in Array(topicPages.keys) {
                if let page = topicPages[id] { topicPages[id]?.messages = normalizeMessages(page.messages) }
            }
            if let affectedID,
               let updated = normalized.first(where: { $0.id == affectedID }),
               (!message.isReaction && event.type == "message") || feedByID[affectedID] != nil {
                feedByID[affectedID] = updated
                if feedLoading { feedEventsDuringLoad.insert(affectedID) }
            }
            if !message.isReaction, let updated = normalized.first(where: { $0.id == message.id }) {
                if topicCatalog.available { Task { await refreshMessageTopics([updated]) } }
                updateContactPreview(
                    contactID: updated.contactId,
                    preview: updated.displayText,
                    timestamp: updated.timestamp
                )
            }
            persistCacheSoon()
        case "topics_updated":
            Task { await loadTopics() }
        case "voice_ready":
            if let id = event.messageId { voiceReadyIDs.insert(id) }
        case "live_reconnecting":
            if !liveUpdatesReconnecting { AppLog.live.notice("Live updates lost; reconnecting") }
            liveUpdatesReconnecting = true
        case "live_restored", "resync", "send_result":
            if event.type == "live_restored" { AppLog.live.notice("Live updates restored") }
            liveUpdatesReconnecting = false
            if recoveryTask != nil { recoveryNeedsAnotherPass = true; return }
            recoveryTask = Task {
                defer { recoveryTask = nil }
                repeat {
                    recoveryNeedsAnotherPass = false
                    await refresh()
                    await loadTopics()
                    if feedHasLoaded || mainTab == .messages { await loadFeed() }
                    if mainTab == .chats, let id = selectedContactID { await loadMessages(for: id) }
                } while recoveryNeedsAnotherPass && !Task.isCancelled
            }
        case "status", "connected", "disconnected", "qr":
            if event.type == "qr", let code = event.data {
                applyBackendStatus(BackendStatus(connected: false, phone: nil, name: nil, connectionState: "linking_required"))
                whatsAppQRCode = code
                whatsAppLinkError = nil
                return
            }
            if event.type == "connected" {
                applyBackendStatus(BackendStatus(connected: true, phone: event.phone, name: event.name, connectionState: "connected"))
                Task { await refresh() }
                return
            }
            if event.type == "disconnected" {
                applyBackendStatus(BackendStatus(connected: false, phone: nil, name: nil,
                    connectionState: requiresWhatsAppLink ? "linking_required" : "reconnecting"))
                return
            }
            if let connected = event.connected {
                applyBackendStatus(BackendStatus(connected: connected, phone: event.phone, name: event.name, connectionState: event.connectionState, qr: event.qr))
            }
        case "mark_as_read":
            if let id = event.chatId, let index = contacts.firstIndex(where: { $0.id == id }) {
                contacts[index].unreadCount = 0
                #if os(iOS)
                let badgeCount = contacts.reduce(0) { $0 + max(0, $1.unreadCount) }
                Task {
                    await PushNotificationCoordinator.shared.markConversationRead(
                        contactID: id,
                        badgeCount: badgeCount
                    )
                }
                #endif
                persistCacheSoon()
            }
        case "messages_viewed":
            if let id = event.chatId, let count = event.unreadCount {
                viewedReceiptMessageIDs.formUnion(event.messageIds ?? [])
                updateViewedUnreadCount(contactID: id, count: count)
                persistCacheSoon()
            }
        case "receipt":
            guard let status = event.status?.lowercased(),
                  matchesDeliveryStatus(status),
                  let messageIDs = event.messageIds,
                  !messageIDs.isEmpty else { return }
            let receiptIDs = Set(messageIDs)
            var didUpdate = false
            for contactID in Array(messages.keys) {
                guard var conversation = messages[contactID] else { continue }
                var conversationUpdated = false
                for index in conversation.indices where conversation[index].isFromMe && receiptIDs.contains(conversation[index].id) {
                    let current = conversation[index].deliveryStatus?.lowercased()
                    guard deliveryStatusRank(status) > deliveryStatusRank(current) else { continue }
                    conversation[index].deliveryStatus = status
                    conversationUpdated = true
                }
                if conversationUpdated {
                    messages[contactID] = conversation
                    didUpdate = true
                }
            }
            if didUpdate { persistCacheSoon() }
        case "send_progress":
            guard let id = event.progressId,
                  let stageName = event.stage,
                  let stage = PhotoSendProgress.Stage(rawValue: stageName),
                  let completed = event.completed else { return }
            updatePhotoSend(id: id, stage: stage, completed: completed)
        default:
            break
        }
    }

    private func matchesDeliveryStatus(_ status: String) -> Bool {
        status == "delivered" || status == "read" || status == "played"
    }

    private func deliveryStatusRank(_ status: String?) -> Int {
        switch status {
        case "read", "played": 3
        case "delivered": 2
        case "sent": 1
        default: 0
        }
    }
}
