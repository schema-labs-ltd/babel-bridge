import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    func conversationUsage(for contactID: String) async throws -> UsageSummary {
        if demoMode { return UsageSummary(inputTokens: 12_840, cachedInputTokens: 8_120, outputTokens: 2_430, costUsd: 0.0842) }
        return try await api.conversationUsage(contactID: contactID)
    }

    func conversationSettings(for contactID: String) async throws -> ConversationSettings {
        if demoMode {
            let settings = savedConversationSettings[contactID] ?? ConversationSettings()
            savedConversationSettings[contactID] = settings
            return settings
        }
        let revision = conversationSettingsRevision
        let settings = try await api.conversationSettings(contactID: contactID)
        if revision != conversationSettingsRevision, let current = savedConversationSettings[contactID] { return current }
        savedConversationSettings[contactID] = settings
        return settings
    }

    func isReplyOnly(_ contactID: String) -> Bool {
        savedConversationSettings[contactID]?.replyOnly == true
    }

    func canSend(to contactID: String, reply: MessageReplyTarget?) -> Bool {
        guard isReplyOnly(contactID) else { return true }
        guard let reply else { return false }
        return ((messages[contactID] ?? []) + Array(feedByID.values) + topicPages.values.flatMap(\.messages)).contains {
            $0.id == reply.messageID && $0.contactId == contactID && !$0.isFromMe
                && !$0.isReaction && $0.contentType.lowercased() != "revoked"
        }
    }

    func checkReplyOnly(_ contactID: String, reply: MessageReplyTarget?) -> Bool {
        guard canSend(to: contactID, reply: reply) else {
            presentError("Reply only", "Choose an incoming message in this conversation and tap Reply before sending.")
            return false
        }
        return true
    }

    func saveConversationSettings(_ settings: ConversationSettings, for contactID: String) async throws {
        if !demoMode { try await api.updateConversationSettings(contactID: contactID, settings: settings) }
        conversationSettingsRevision += 1
        savedConversationSettings[contactID] = settings
    }

    func openAISettings() async throws -> OpenAISettings {
        if demoMode { return OpenAISettings(model: "gpt-5.6-terra", reasoningEffort: "medium", available: true) }
        return try await api.openAISettings()
    }

    func saveOpenAISettings(_ settings: OpenAISettings) async throws {
        guard !demoMode else { return }
        try await api.updateOpenAISettings(settings)
    }

    func chatGPTStatus() async throws -> ChatGPTStatus {
        if demoMode { return ChatGPTStatus(available: true, signedIn: false, apiKeyConfigured: true) }
        return try await api.chatGPTStatus()
    }

    func startChatGPTSignIn(redirectURI: String) async throws -> ChatGPTSignInStart {
        guard !demoMode else { throw APIError.server("Sign in with ChatGPT isn’t available in the demo.") }
        return try await api.startChatGPTSignIn(redirectURI: redirectURI)
    }

    func completeChatGPTSignIn(callbackURL: String) async throws -> ChatGPTStatus {
        try await api.completeChatGPTSignIn(callbackURL: callbackURL)
    }

    func setChatGPTModel(_ model: String) async throws -> ChatGPTStatus {
        try await api.setChatGPTModel(model)
    }

    func signOutOfChatGPT() async throws -> ChatGPTStatus {
        try await api.signOutOfChatGPT()
    }
}
