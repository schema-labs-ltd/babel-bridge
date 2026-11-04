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
            // A message from someone ends their typing, as it does in WhatsApp.
            if event.type == "message", !message.isFromMe {
                typing.stop(chatID: message.contactId, userID: message.senderJID)
            }
            let normalized = normalizeMessages(conversation + [message])
            messages[message.contactId] = normalized
            // History sync also arrives as "message" events, but with old timestamps; only a message
            // sent in the last couple of minutes counts as a live arrival. Recorded after the list
            // changes, so an observer finds the message in it.
            let recent = message.timestamp >= Int64(Date().timeIntervalSince1970 * 1_000) - 120_000
            if event.type == "message", !message.isReaction, recent {
                liveArrivals[message.contactId, default: LiveArrivals()].record(message.id, fromMe: message.isFromMe)
            }
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
            scheduleConnectingIndicator()
        case "live_restored", "resync", "send_result":
            if event.type == "live_restored" { AppLog.live.notice("Live updates restored") }
            clearConnectingIndicator()
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
        case "typing":
            guard let chatID = event.chatId, let userID = event.userId, let state = event.state else { return }
            typing.apply(chatID: chatID, userID: userID, state: state)
            // WhatsApp drops a typing state it hasn't heard again in about 25 seconds, so a lost
            // "paused" can't leave the indicator on.
            if typing.isActive(in: chatID) {
                Task { [weak self] in
                    try? await Task.sleep(for: .seconds(TypingIndicators.lifetime + 1))
                    self?.typing.prune()
                }
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

extension AppSession {
    /// What the chat header and chat list show while someone in this chat is typing.
    func typingLabel(for contact: Contact) -> String? {
        typing.label(in: contact.id, isGroup: contact.isGroup) { userID in
            let sender = messages[contact.id]?.last { $0.senderJID == userID && $0.senderName?.isEmpty == false }
            if let name = sender?.senderName {
                return name.first.map { $0 == "+" || $0.isNumber } == true
                    ? name : (name.split(separator: " ").first.map(String.init) ?? name)
            }
            let user = userID.split(separator: "@").first.map(String.init) ?? ""
            return userID.hasSuffix("@s.whatsapp.net") && !user.isEmpty && user.allSatisfy(\.isNumber) ? "+\(user)" : nil
        }
    }

    func scheduleConnectingIndicator(restart: Bool = false) {
        if restart { connectingIndicatorTask?.cancel(); connectingIndicatorTask = nil }
        guard connectingIndicatorTask == nil, !showsConnectingIndicator else { return }
        let delay = connectingIndicatorDelay
        connectingIndicatorTask = Task { [weak self] in
            do { try await Task.sleep(for: delay) } catch { return }
            guard let self, !Task.isCancelled else { return }
            self.connectingIndicatorTask = nil
            if self.liveUpdatesReconnecting { self.showsConnectingIndicator = true }
        }
    }

    /// The reconnect timer and backoff keep counting while the app is suspended, so a socket that
    /// failed just before suspension would otherwise show "Connecting…" at once on return, or wait
    /// out a long backoff. Start both afresh when the app becomes active.
    func reconnectLiveEventsOnResume() async {
        guard liveUpdatesReconnecting else { return }
        showsConnectingIndicator = false
        scheduleConnectingIndicator(restart: true)
        try? await api.connectLiveEvents { [weak self] event in
            self?.handle(event)
        }
    }

    func clearConnectingIndicator() {
        liveUpdatesReconnecting = false
        connectingIndicatorTask?.cancel()
        connectingIndicatorTask = nil
        showsConnectingIndicator = false
    }
}

/// Who is typing or recording a voice note in each chat, from the bridge's chat presence events.
struct TypingIndicators: Equatable {
    static let lifetime: TimeInterval = 25

    struct Entry: Equatable {
        let userID: String
        let recording: Bool
        let until: Date
    }

    private(set) var chats: [String: [Entry]] = [:]

    mutating func apply(chatID: String, userID: String, state: String, now: Date = .now) {
        var entries = (chats[chatID] ?? []).filter { $0.userID != userID && $0.until > now }
        if state == "typing" || state == "recording" {
            entries.append(Entry(userID: userID, recording: state == "recording", until: now.addingTimeInterval(Self.lifetime)))
        }
        chats[chatID] = entries.isEmpty ? nil : entries
    }

    /// Ends one person's typing, or everyone's in the chat when the sender isn't known.
    mutating func stop(chatID: String, userID: String?, now: Date = .now) {
        guard let userID else {
            // Without a sender only a one-to-one chat's typist is known; in a group, others may
            // still be typing, so their indicators run until they stop or expire.
            if !chatID.hasSuffix("@g.us") { chats[chatID] = nil }
            return
        }
        apply(chatID: chatID, userID: userID, state: "paused", now: now)
    }

    mutating func prune(now: Date = .now) {
        let kept = chats.compactMapValues { entries -> [Entry]? in
            let live = entries.filter { $0.until > now }
            return live.isEmpty ? nil : live
        }
        if kept != chats { chats = kept }
    }

    func active(in chatID: String, now: Date = .now) -> [Entry] {
        (chats[chatID] ?? []).filter { $0.until > now }
    }

    func isActive(in chatID: String, now: Date = .now) -> Bool { !active(in: chatID, now: now).isEmpty }

    /// WhatsApp's words: "typing…" in a one-to-one chat, "Ana is typing…" in a group, and
    /// "recording audio…" for a voice note. `name` gives a group member's name from their ID.
    func label(in chatID: String, isGroup: Bool, now: Date = .now, name: (String) -> String?) -> String? {
        let entries = active(in: chatID, now: now)
        guard let latest = entries.last else { return nil }
        let action = latest.recording ? "recording audio…" : "typing…"
        guard isGroup else { return action }
        if entries.count > 1 { return "\(entries.count) people are typing…" }
        guard let who = name(latest.userID) else { return action }
        return latest.recording ? "\(who) is recording audio…" : "\(who) is typing…"
    }
}
