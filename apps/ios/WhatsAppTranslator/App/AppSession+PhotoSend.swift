import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    func sendImage(
        data: Data,
        mimeType: String,
        caption: String? = nil,
        to contactID: String,
        reply: MessageReplyTarget? = nil
    ) async -> Bool {
        await sendImages(
            [OutgoingImage(data: data, mimeType: mimeType)],
            caption: caption,
            to: contactID,
            reply: reply
        )
    }

    func sendImages(
        _ images: [OutgoingImage],
        caption: String? = nil,
        to contactID: String,
        reply: MessageReplyTarget? = nil
    ) async -> Bool {
        guard checkReplyOnly(contactID, reply: reply) else { return false }
        guard !images.isEmpty, images.count <= 30 else {
            presentError("Couldn’t send photos", "Choose between 1 and 30 photos.")
            return false
        }
        guard images.allSatisfy({ !$0.data.isEmpty && $0.data.count <= 16 * 1_024 * 1_024 }) else {
            presentError("Couldn’t send photos", "One of the photos couldn’t be reduced enough for WhatsApp.")
            return false
        }
        guard images.reduce(0, { $0 + $1.data.count }) <= 64 * 1_024 * 1_024 else {
            presentError("Couldn’t send photos", "The selected photos must be smaller than 64 MB combined.")
            return false
        }
        sendingContactIDs.insert(contactID)
        defer { sendingContactIDs.remove(contactID) }

        if demoMode {
            for image in images {
                let message = ChatMessage.demoImage(contactID: contactID)
                messages[contactID, default: []].append(message)
                await storeMedia(image.data, mimeType: image.mimeType, for: message)
            }
            return true
        }

        do {
            _ = try await api.sendImages(contactID: contactID, images: images, caption: caption, reply: reply)
            await loadMessages(for: contactID)
            await refresh()
            return true
        } catch {
            presentError("Couldn’t send photos", error)
            return false
        }
    }

    @discardableResult
    func startPhotoSend(
        _ images: [OutgoingImage],
        caption: String? = nil,
        to contactID: String,
        reply: MessageReplyTarget? = nil,
        replyOnlyIfNotLatest: Bool = false
    ) -> Bool {
        guard checkReplyOnly(contactID, reply: reply) else { return false }
        guard !sendingContactIDs.contains(contactID) else { return false }
        guard !images.isEmpty, images.count <= 30 else {
            presentError("Couldn’t send photos", "Choose between 1 and 30 photos.")
            return false
        }
        let id = UUID().uuidString
        photoSendProgress = photoSendProgress.filter { $0.value.contactID != contactID }
        photoSendProgress[id] = PhotoSendProgress(
            id: id, contactID: contactID, stage: .preparing,
            completed: 0, total: images.count, error: nil
        )
        #if os(iOS)
        PhotoSendActivityCoordinator.shared.start(photoSendProgress[id]!)
        #endif
        sendingContactIDs.insert(contactID)
        Task { await runPhotoSend(id: id, rawImages: images, caption: caption, contactID: contactID, reply: reply, replyOnlyIfNotLatest: replyOnlyIfNotLatest) }
        return true
    }

    private func runPhotoSend(
        id: String,
        rawImages: [OutgoingImage],
        caption: String?,
        contactID: String,
        reply: MessageReplyTarget?,
        replyOnlyIfNotLatest: Bool
    ) async {
        #if os(iOS)
        let backgroundLease = PhotoSendBackgroundLease(name: "Prepare photo album")
        defer { backgroundLease.end() }
        #endif
        defer { sendingContactIDs.remove(contactID) }
        let maximumBytes = PhotoUploadPreparer.maximumBytes(forPhotoCount: rawImages.count)
        var prepared: [OutgoingImage] = []
        for (index, raw) in rawImages.enumerated() {
            let result = await Task.detached(priority: .userInitiated) {
                guard let image = PlatformImage(data: raw.data) else { return nil as OutgoingImage? }
                return PhotoUploadPreparer.prepare(
                    data: raw.data, mimeType: raw.mimeType, image: image, maximumBytes: maximumBytes
                )
            }.value
            guard let result else {
                failPhotoSend(id: id, message: "Photo \(index + 1) couldn’t be prepared for sending.")
                return
            }
            prepared.append(result)
            updatePhotoSend(id: id, stage: .preparing, completed: index + 1)
        }

        updatePhotoSend(id: id, stage: .transferring, completed: 0)
        if demoMode {
            let quoted = replyOnlyIfNotLatest ? conditionalDemoReply(reply, contactID: contactID) : reply
            for (index, image) in prepared.enumerated() {
                let message = demoAttachmentMessage(kind: "image", data: image.data, mimeType: image.mimeType, fileName: nil, caption: caption, contactID: contactID, reply: quoted)
                messages[contactID, default: []].append(message)
                feedByID[message.id] = message
                await storeMedia(image.data, mimeType: image.mimeType, for: message)
                updatePhotoSend(id: id, stage: .sending, completed: index + 1)
            }
            completePhotoSend(id: id)
            return
        }

        do {
            _ = try await api.sendImages(
                contactID: contactID, images: prepared, progressID: id,
                caption: caption, reply: reply, replyOnlyIfNotLatest: replyOnlyIfNotLatest,
                transferProgress: { [weak self] completed, _ in
                    await self?.updatePhotoSend(id: id, stage: .transferring, completed: completed)
                }
            )
            completePhotoSend(id: id)
            await loadMessages(for: contactID)
            await loadFeed()
            await refresh()
        } catch {
            failPhotoSend(id: id, message: error.localizedDescription)
            presentError("Couldn’t send photos", error)
        }
    }

    func updatePhotoSend(id: String, stage: PhotoSendProgress.Stage, completed: Int) {
        guard var progress = photoSendProgress[id] else { return }
        progress.stage = stage
        progress.completed = completed
        photoSendProgress[id] = progress
        #if os(iOS)
        PhotoSendActivityCoordinator.shared.update(progress)
        #endif
    }

    private func completePhotoSend(id: String) {
        guard var progress = photoSendProgress[id] else { return }
        progress.stage = .complete
        progress.completed = progress.total
        photoSendProgress[id] = progress
        #if os(iOS)
        PhotoSendActivityCoordinator.shared.end(progress)
        #endif
        Task {
            try? await Task.sleep(for: .seconds(5))
            if photoSendProgress[id]?.stage == .complete { photoSendProgress[id] = nil }
        }
    }

    private func failPhotoSend(id: String, message: String) {
        guard var progress = photoSendProgress[id] else { return }
        progress.stage = .failed
        progress.error = message
        photoSendProgress[id] = progress
        #if os(iOS)
        PhotoSendActivityCoordinator.shared.end(progress)
        #endif
    }
}

extension AppSession {
    /// Shows the message in its chat straight away and sends it in the background; sends to one
    /// chat go one at a time so they arrive in the order they were written.
    @discardableResult
    func startSend(text: String, to contactID: String, reply: MessageReplyTarget? = nil, replyOnlyIfNotLatest: Bool = false) -> Bool {
        guard checkReplyOnly(contactID, reply: reply) else { return false }
        let clean = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return false }
        var quote = reply
        if replyOnlyIfNotLatest, let reply {
            let target = ((messages[contactID] ?? []) + Array(feedByID.values)).first { $0.id == reply.messageID && $0.contactId == contactID }
            if let target, !feedReplyNeedsQuote(target) { quote = nil }
        }
        let contact = contacts.first { $0.id == contactID }
        var send = PendingSend(
            id: PendingSend.idPrefix + UUID().uuidString, contactID: contactID, text: clean,
            reply: reply, replyOnlyIfNotLatest: replyOnlyIfNotLatest,
            timestamp: Int64(Date().timeIntervalSince1970 * 1_000), quote: quote,
            contactName: contact?.name, chatType: contact?.isGroup == true ? "group" : "private"
        )
        send.earlierIDs = Set(loadedMessages(in: [contactID]).map(\.id))
        pendingSends[send.id] = send
        // Saved at once, so a message the composer no longer holds survives the app closing.
        persistCacheSoon()
        deliver(send.id)
        return true
    }

    func retrySend(_ id: String) {
        guard pendingSends[id]?.failed == true else { return }
        // The server may already have it, for one restored after the app closed mid-send.
        if settle(id) { persistCacheSoon(); return }
        pendingSends[id]?.failure = nil
        deliver(id)
        persistCacheSoon()
    }

    /// Stops waiting sends after signing out or switching server, so a new send to the same
    /// chat doesn't queue behind a request to the old one.
    func cancelSendQueues() {
        sendQueues.values.forEach { $0.cancel() }
        sendQueues = [:]
    }

    func discardSend(_ id: String) {
        guard pendingSends[id]?.failed == true else { return }
        pendingSends[id] = nil
        persistCacheSoon()
    }

    /// The chat's sends still waiting for the server's copy, as messages to show. For the
    /// Messages feed (no chat given) only the feed's own copy counts, as only that shows there.
    func pendingMessages(for contactID: String?) -> [ChatMessage] {
        let sends = pendingSends.values.filter { contactID == nil || $0.contactID == contactID }
        guard !sends.isEmpty else { return [] }
        let contactIDs = Set(sends.map(\.contactID))
        let loaded = contactID == nil ? feedByID.values.filter { contactIDs.contains($0.contactId) } : loadedMessages(in: contactIDs)
        return PendingSend.unconfirmed(Array(sends), loaded: loaded).map(\.message)
    }

    private func loadedMessages(in contactIDs: Set<String>) -> [ChatMessage] {
        contactIDs.flatMap { messages[$0] ?? [] } + feedByID.values.filter { contactIDs.contains($0.contactId) }
    }

    /// Settles a send once its chat has reloaded: the server's copy, sent or marked uncertain,
    /// takes its place in the chat and the Messages feed alike, so the feed has no gap until it
    /// next loads. Returns whether a copy was found.
    private func settle(_ id: String) -> Bool {
        guard let send = pendingSends[id] else { return false }
        let chatSends = pendingSends.values.filter { $0.contactID == send.contactID }
        guard let copy = PendingSend.copies(of: Array(chatSends), in: loadedMessages(in: [send.contactID]))[id] else { return false }
        resolve(id, with: copy)
        return true
    }

    /// Replaces a pending send with its server copy. That copy belongs to this send alone,
    /// never to another one with the same words.
    private func resolve(_ id: String, with copy: ChatMessage, alsoReserving others: [String] = []) {
        guard let send = pendingSends.removeValue(forKey: id) else { return }
        place(copy)
        for (otherID, other) in pendingSends where other.contactID == send.contactID {
            pendingSends[otherID]?.earlierIDs.formUnion([copy.id] + others)
        }
    }

    /// Settles a send the server accepted by the message its reply names, rather than by its
    /// words. The row stored while it sent is replaced, and if the chat didn't reload the
    /// message shows as the server confirmed it until the next load brings the server's copy.
    private func confirm(_ id: String, response: SendMessageResponse) {
        guard let send = pendingSends[id] else { return }
        let contactID = send.contactID
        let loaded = messages[contactID] ?? []
        let stored = send.copy(in: loaded).flatMap { $0.id.hasPrefix("pending_") && $0.id != response.messageId ? $0 : nil }
        let confirmed = loaded.first { $0.id == response.messageId } ?? send.confirmedMessage(response)
        if let stored {
            messages[contactID]?.removeAll { $0.id == stored.id }
            feedByID[stored.id] = nil
        }
        if !loaded.contains(where: { $0.id == confirmed.id }) {
            messages[contactID] = normalizeMessages((messages[contactID] ?? []) + [confirmed])
        }
        resolve(id, with: confirmed, alsoReserving: [response.originalMessageId, stored?.id].compactMap { $0 })
    }

    /// Puts a sent message in the Messages feed, kept through a feed load already under way
    /// as a live message is.
    private func place(_ message: ChatMessage) {
        guard feedHasLoaded, feedByID[message.id] == nil else { return }
        feedByID[message.id] = message
        if feedLoading { feedEventsDuringLoad.insert(message.id) }
    }

    private func deliver(_ id: String) {
        guard let contactID = pendingSends[id]?.contactID else { return }
        let previous = sendQueues[contactID]
        sendQueues[contactID] = Task {
            await previous?.value
            // Signing out or switching server drops what was waiting.
            guard let send = pendingSends[id] else { return }
            let outcome = await transmit(send)
            guard pendingSends[id] != nil else { return }
            defer { persistCacheSoon() }
            switch outcome {
            case .failed(let failure):
                pendingSends[id]?.failure = failure
                // A send that timed out may still be on the server, marked as uncertain; showing
                // that copy instead of a resend button avoids sending the message twice. Offline,
                // the resend button says enough without an alert as well.
                await loadMessages(for: contactID, quietly: true)
                _ = settle(id)
            case .sent(let response):
                // Saved before anything else, as the idempotency key is already gone.
                pendingSends[id]?.confirmed = true
                await persistCache()
                await loadMessages(for: contactID)
                // Demo mode has no reply; it adds the message itself.
                if let response { confirm(id, response: response) } else { pendingSends[id] = nil }
                Task { await loadFeed(); await refresh() }
            }
        }
    }

    private enum SendOutcome {
        /// The server's answer; nil in demo mode, which adds the message itself.
        case sent(SendMessageResponse?)
        case failed(String)
    }

    private func transmit(_ send: PendingSend) async -> SendOutcome {
        if demoMode {
            try? await Task.sleep(for: .seconds(1.5))
            if demoSendsFail { return .failed("The demo server is offline.") }
            let message = ChatMessage.demoOutgoing(contactID: send.contactID, text: send.text, reply: send.quote)
            messages[send.contactID, default: []].append(message)
            feedByID[message.id] = message
            updateContactPreview(contactID: send.contactID, preview: "You: \(send.text)", timestamp: message.timestamp)
            return .sent(nil)
        }
        do {
            let response = try await api.send(contactID: send.contactID, text: send.text, reply: send.reply, replyOnlyIfNotLatest: send.replyOnlyIfNotLatest, operation: send.id)
            if let contact = contacts.first(where: { $0.id == send.contactID }) {
                MessagingIntentDonor.donateOutgoing(originalText: send.text, response: response, contact: contact, displayName: displayName(for: contact))
            }
            return .sent(response)
        } catch {
            AppLog.ui.error("Couldn’t send message: \(AppLog.kind(of: error), privacy: .public)")
            return .failed(error.localizedDescription)
        }
    }
}
