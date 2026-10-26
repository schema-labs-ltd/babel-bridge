import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    func voiceSample(preference: String) async throws -> VoiceSample {
        try await api.voiceSample(preference: preference)
    }

    func voicePreferences(scope: String) async throws -> VoicePreferences {
        if demoMode { return VoicePreferences() }
        return try await api.voicePreferences(scope: scope)
    }

    func messageToneSettings(contactID: String?) async throws -> MessageToneSettings {
        if demoMode {
            let tone = contactID.map { demoConversationTones[$0] } ?? demoGlobalTone
            return MessageToneSettings(tone: tone, globalTone: demoGlobalTone, effectiveTone: tone ?? demoGlobalTone)
        }
        return try await api.messageToneSettings(contactID: contactID)
    }

    func saveMessageTone(_ tone: MessageTone?, contactID: String?) async throws -> MessageToneSettings {
        guard contactID != nil || tone != nil else { throw APIError.server("Choose a global ringtone.") }
        let saved: MessageToneSettings
        if demoMode {
            if let contactID { demoConversationTones[contactID] = tone }
            else if let tone { demoGlobalTone = tone }
            saved = try await messageToneSettings(contactID: contactID)
        } else {
            saved = try await api.saveMessageTone(tone, contactID: contactID)
        }
        messageToneRevision += 1
        return saved
    }

    func saveVoicePreferences(_ preferences: VoicePreferences, scope: String) async throws {
        if !demoMode { try await api.saveVoicePreferences(preferences, scope: scope) }
        voicePreferenceRevision += 1
    }

    func translateVoice(messageID: String) async throws -> TranslatedVoiceNote {
        try await api.translateVoice(messageID: messageID)
    }

    func prepareVoice(data: Data, contactID: String, reply: MessageReplyTarget?, replyOnlyIfNotLatest: Bool = false) async throws -> TranslatedVoiceNote {
        guard checkReplyOnly(contactID, reply: reply) else { throw APIError.server("Choose an incoming message and tap Reply before recording.") }
        return try await api.prepareVoice(data: data, contactID: contactID, reply: reply, replyOnlyIfNotLatest: replyOnlyIfNotLatest)
    }

    func sendVoice(_ note: TranslatedVoiceNote) async throws -> VoiceSendResult {
        let result = try await api.sendVoice(preparationID: note.id)
        await loadMessages(for: note.contactId)
        await loadFeed()
        await refresh()
        return result
    }
}
