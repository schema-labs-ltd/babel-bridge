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
