import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    func translate(_ message: ChatMessage) async {
        guard message.canTranslate else { return }
        activeMessageActionIDs.insert(message.id)
        defer { activeMessageActionIDs.remove(message.id) }
        if demoMode {
            replaceMessage(message.id, in: message.contactId) { current in
                ChatMessage.demoTranslated(from: current)
            }
            return
        }
        do {
            let result = try await api.translate(message)
            guard result.success else { throw APIError.server(result.error ?? "Translation failed.") }
            await loadMessages(for: message.contactId)
        } catch {
            presentError("Couldn’t translate message", error)
        }
    }

    func generateAIReply(to message: ChatMessage) async -> String? {
        guard message.canGenerateAIReply else { return nil }
        activeMessageActionIDs.insert(message.id)
        defer { activeMessageActionIDs.remove(message.id) }
        if demoMode { return "Sounds good — I’ll let you know when I’m leaving." }
        do {
            let result = try await api.generateAIReply(to: message)
            guard result.success, let reply = result.replyText?.trimmingCharacters(in: .whitespacesAndNewlines), !reply.isEmpty else {
                throw APIError.server(result.error ?? "AI reply was empty.")
            }
            return reply
        } catch {
            presentError("Couldn’t generate an AI reply", error)
            return nil
        }
    }

    func react(to message: ChatMessage, emoji: String) async {
        guard !activeMessageActionIDs.contains(message.id) else { return }
        activeMessageActionIDs.insert(message.id)
        defer { activeMessageActionIDs.remove(message.id) }
        if demoMode {
            applyConfirmedReaction(to: message, emoji: emoji)
            return
        }
        do {
            let confirmed = try await api.react(to: message, emoji: emoji, senderJID: message.isFromMe ? ownSenderJID : message.senderJID)
            if let confirmed { handle(.reaction(confirmed)) }
            else { applyConfirmedReaction(to: message, emoji: emoji) }
        } catch {
            presentError("Couldn’t react to message", error)
        }
    }

    // Older servers return success without an event. Apply that confirmed choice
    // locally too; don't rely on an unrelated later refresh or WhatsApp echo.
    private func applyConfirmedReaction(to message: ChatMessage, emoji: String) {
        let reaction = ChatMessage(
            id: "confirmed-reaction-\(UUID().uuidString)", contactId: message.contactId,
            timestamp: Int64(Date().timeIntervalSince1970 * 1_000), isFromMe: true, isForwarded: false,
            senderName: nil, senderPhone: nil, contactName: message.contactName, contactPhone: message.contactPhone,
            chatType: message.chatType, contentType: "Reaction",
            content: MessageContent(type: "reaction", body: nil, emoji: emoji, targetMessageId: message.id, showTranslatedPrimary: nil, replyContext: nil),
            originalText: nil, translatedText: nil, sourceLanguage: nil, isTranslated: false
        )
        handle(.reaction(reaction))
    }
}
