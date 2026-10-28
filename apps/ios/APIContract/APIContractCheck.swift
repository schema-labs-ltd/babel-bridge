import Foundation

// Decodes the server's golden JSON in fixtures/api with the app's own models.
// The Rust test `api_contract` writes the responses; the requests are what the
// apps send. Run with scripts/check-api-contract.sh, which builds this file
// together with APIModels.swift on Linux or macOS.

@main
struct APIContractCheck {
    static func main() {
        guard CommandLine.arguments.count == 2 else {
            print("usage: api-contract-check <fixtures/api>")
            exit(2)
        }
        let check = Checker(root: URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true))
        checkResponses(check)
        checkLiveEvents(check)
        checkRequests(check)
        check.finish()
    }

    static let ana = "447700900123@s.whatsapp.net"
    static let family = "120363000000000001@g.us"

    static func checkResponses(_ check: Checker) {
        check.response("health", HealthResponse.self) { check.expect($0.ok, "server is healthy") }
        check.response("auth-check", AuthCheckResponse.self) { check.expect($0.required, "password is required") }
        check.response("auth", AuthResponse.self) {
            check.equal($0.success, true, "sign-in succeeds")
            check.equal($0.token, "session-token", "sign-in returns a token")
        }
        check.response("error", ErrorResponse.self) { check.expect(!$0.error.isEmpty, "error has a message") }
        check.response("status", BackendStatus.self) {
            check.equal($0.connected, false, "status is disconnected while linking")
            check.equal($0.connectionState, "linking_required", "status reports linking")
            check.equal($0.qr, "2@linking-code", "status carries the QR code")
        }
        check.response("qr", WhatsAppQRResponse.self) { check.equal($0.qr, "2@linking-code", "QR code") }
        check.response("contacts", [Contact].self) { contacts in
            check.equal(contacts.map(\.id), [family, ana], "pinned chat sorts first")
            check.equal(contacts.first?.isGroup, true, "group chat is a group")
            check.equal(contacts.first?.pinnedAt != nil, true, "pinned chat has pinnedAt")
            check.equal(contacts.last?.displayName, "Ana", "contact name")
            check.equal(contacts.last?.unreadCount, 1, "unread count")
            check.expect(contacts.allSatisfy { $0.lastMessagePreview != nil }, "every chat has a preview")
        }
        check.response("pin", PinResponse.self) { check.expect($0.success && $0.pinned, "pin succeeds") }
        check.response("messages", MessagesResponse.self) { page in
            check.equal(page.hasMore, false, "single page")
            check.equal(page.messages.map(\.id), ["ana-hello", "me-reply", "ana-reaction"], "message order")
            guard page.messages.count == 3 else { return }
            let hello = page.messages[0], reply = page.messages[1], reaction = page.messages[2]
            check.equal(hello.displayText, "Hola, ¿cómo estás?", "incoming text")
            check.equal(hello.canTranslate, true, "untranslated incoming text can be translated")
            check.equal(reply.deliveryState, .read, "receipt marks the reply read")
            check.equal(reply.isEdited, true, "edit is visible")
            check.equal(reply.editRevision, 1_767_225_700_000, "edit revision")
            check.equal(reply.content?.replyContext?.messageId, "ana-hello", "reply quotes its target")
            check.equal(reply.content?.replyContext?.senderName, "Ana", "reply quote sender")
            check.equal(reply.reactions?["👍"], ["447700900123"], "reaction summary")
            check.equal(reply.reactionStates?["447700900123"]?.id, "ana-reaction", "reaction state")
            check.equal(reaction.isReaction, true, "reaction message")
            check.equal(reaction.content?.targetMessageId, "me-reply", "reaction target")
        }
        check.response("feed", MessagesResponse.self) { page in
            let byID = Dictionary(page.messages.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            check.expect(byID["ana-reaction"] == nil, "feed leaves reactions out")
            check.equal(byID["ben-photo-0"]?.isImage, true, "photo is an image")
            check.equal(byID["ben-photo-0"]?.albumID, "ben-album", "album id")
            check.equal(byID["ben-photo-1"]?.albumIndex, 1, "album index")
            check.equal(byID["ben-photo-0"]?.content?.width, 1, "image width")
            check.equal(byID["ben-photo-0"]?.content?.hasMedia, true, "image has media")
            check.equal(byID["ben-photo-0"]?.content?.mediaData, nil, "list leaves media bytes out")
            check.equal(byID["ben-voice"]?.displayText, "Voice note", "voice note")
            check.equal(byID["ben-voice"]?.content?.durationSeconds, 7, "voice note duration")
            check.equal(byID["ben-location"]?.locationURL != nil, true, "location opens in Maps")
            check.equal(byID["ben-document"]?.displayText, "tickets.pdf", "document name")
            check.equal(byID["ben-poll"]?.content?.options, ["Pizza", "Curry"], "poll options")
            if let photos = byID["ben-photo-0"].flatMap({ first in byID["ben-photo-1"].map { [first, $0] } }) {
                let items = ConversationTimelineBuilder.items(from: photos)
                check.equal(items.count, 1, "album photos group into one timeline item")
            } else {
                check.fail("feed has both album photos")
            }
        }
        check.response("media", MediaResponse.self) {
            check.equal($0.mimeType, "image/png", "media MIME type")
            check.expect(Data(base64Encoded: $0.mediaData) != nil, "media is base64")
        }
        check.response("viewed-messages", ViewedMessagesResponse.self) {
            check.equal($0.success, true, "viewed messages are recorded")
        }
        check.response("conversation-settings", ConversationSettings.self) {
            check.equal($0, ConversationSettings(languageOverride: "Spanish", translationStyle: "casual",
                                                 sendOriginalFollowUp: true, translationEnabled: true, replyOnly: true),
                        "conversation settings round-trip")
        }
        check.response("update-conversation-settings", SuccessResponse.self) {
            check.equal($0.success, true, "conversation settings save")
        }
        check.response("openai-settings", OpenAISettings.self) {
            check.equal($0, OpenAISettings(model: "gpt-5.6-sol", reasoningEffort: "high", available: false), "OpenAI settings")
        }
        check.response("update-openai-settings", SuccessResponse.self) {
            check.equal($0.success, true, "OpenAI settings save")
        }
        check.response("chatgpt-status", ChatGPTStatus.self) {
            check.equal($0.signedIn, false, "ChatGPT signed out")
            check.equal($0.available, false, "ChatGPT unavailable")
        }
        let tone = MessageToneSettings(tone: .aurora, globalTone: .default, effectiveTone: .aurora)
        check.response("message-tone-settings", MessageToneSettings.self) { check.equal($0, tone, "message tone") }
        check.response("update-message-tone", MessageToneSettings.self) { check.equal($0, tone, "saved message tone") }
        check.response("voice-settings", VoicePreferences.self) { check.equal($0.voice, "feminine", "voice") }
        check.response("update-voice-settings", VoicePreferences.self) { check.equal($0.voice, "feminine", "saved voice") }
        check.response("usage", UsageSummary.self) {
            check.equal($0, UsageSummary(inputTokens: 120, cachedInputTokens: 40, outputTokens: 30, costUsd: 0.0125), "usage")
        }
        check.response("topics", TopicCatalog.self) { catalog in
            check.equal(catalog.available, false, "topics need OpenAI")
            check.equal(catalog.topics.first?.title, "Catching up", "topic title")
            check.equal(catalog.topics.first?.messageCount, 2, "topic message count")
            check.equal(catalog.topics.first?.categoryId?.hasPrefix("category:"), true, "topic category")
            check.equal(catalog.settings.first?.enabled, true, "topics enabled for the chat")
            check.equal(catalog.unifiedTopics.count, 1, "one unified topic")
        }
        check.response("message-topics", MessageTopicsResponse.self) {
            check.equal($0.topics.map(\.messageId), ["ana-hello", "me-reply"], "message topics")
            check.equal($0.topics.first?.menuLabel, "Catching up", "message topic label")
        }
        check.response("send-message", SendMessageResponse.self) {
            check.equal($0.messageId, "3EB0SENT", "sent message id")
            check.equal($0.isTranslated, true, "send was translated")
            check.equal($0.originalFollowUpSent, true, "original follow-up sent")
            check.equal($0.originalMessageId, "3EB0ORIGINAL", "original follow-up id")
        }
        check.response("send-image", SendImageResponse.self) { check.equal($0.messageId, "3EB0IMAGE", "sent image id") }
        check.response("reaction", ReactionResponse.self) {
            check.equal($0.success, true, "reaction sent")
            check.equal($0.reaction?.isReaction, true, "reaction message returned")
        }
        check.response("translate", TranslateMessageResponse.self) {
            check.equal($0.translatedText, "Hello, how are you?", "translation")
            check.equal($0.sourceLanguage, "Spanish", "source language")
        }
        check.response("ai-reply", AIReplyResponse.self) {
            check.equal($0.replyText, "All good here, you?", "AI reply")
            check.equal($0.costUsd, 0.0021, "AI reply cost")
        }
    }

    static func checkLiveEvents(_ check: Checker) {
        check.response("live-events", [LiveEvent].self) { events in
            let types = Set(events.map(\.type))
            let handled: Set<String> = [
                "status", "qr", "connected", "disconnected", "message", "reaction", "voice_ready",
                "message_updated", "conversation_settings_updated", "resync", "topics_updated",
                "heartbeat", "typing", "mark_as_read", "messages_viewed", "receipt", "send_progress",
                "send_result", "error",
            ]
            check.equal(types.subtracting(handled).sorted(), [], "every live event type is known to the app")
            func first(_ type: String) -> LiveEvent? { events.first { $0.type == type } }
            check.equal(first("status")?.connectionState, "linking_required", "status event state")
            check.equal(first("status")?.qr, "2@linking-code", "status event QR")
            check.equal(first("qr")?.data, "2@linking-code", "QR event")
            check.equal(first("connected")?.phone, "447700900999", "connected event phone")
            check.equal(first("message")?.message?.id, "ana-hello", "message event")
            check.equal(first("message_updated")?.message?.isEdited, true, "edit event")
            check.equal(first("reaction")?.message?.isReaction, true, "reaction event")
            check.equal(first("receipt")?.messageIds, ["me-reply"], "receipt ids")
            check.equal(first("receipt")?.status, "read", "receipt status")
            check.equal(first("typing")?.chatId, ana, "typing chat")
            check.equal(first("send_progress")?.progressId, "album-1", "send progress id")
            check.equal(first("send_progress")?.stage, "uploading", "send progress stage")
            check.equal(first("send_progress")?.completed, 1, "send progress completed")
            check.equal(first("send_progress")?.total, 2, "send progress total")
            check.equal(first("messages_viewed")?.unreadCount, 0, "viewed unread count")
            check.equal(first("messages_viewed")?.messageIds, ["ana-hello"], "viewed ids")
            check.equal(first("mark_as_read")?.chatId, family, "mark as read chat")
            check.equal(first("conversation_settings_updated")?.chatId, ana, "settings event chat")
            check.equal(first("conversation_settings_updated")?.settings?.languageOverride, "Spanish", "settings event")
            check.equal(first("voice_ready")?.messageId, "ben-voice", "voice ready id")
        }
    }

    static func checkRequests(_ check: Checker) {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
        check.request("auth", ["password": "secret"])
        check.request("send-message", SendMessageRequest(
            replyOnlyIfNotLatest: true, contactId: ana, text: "Muy bien, gracias", replyTo: "ana-hello",
            replyToSender: ana, replyToText: "Hola, ¿cómo estás?", replyToSenderName: "Ana"))
        check.request("send-image", SendImageRequest(
            mediaKind: "image", fileName: "pixel.png", contactId: ana, mediaData: png, mimeType: "image/png",
            caption: "A pixel", replyTo: nil, replyToSender: nil, replyToText: nil, replyToSenderName: nil))
        check.request("create-photo-album", CreatePhotoAlbumRequest(
            jobId: "album-1", contactId: ana, photoCount: 2, caption: "Beach day", replyTo: nil,
            replyToSender: nil, replyToText: nil, replyToSenderName: nil))
        check.request("send-reaction", SendReactionRequest(contactId: ana, messageId: "ana-hello", senderJid: ana, emoji: "❤️"))
        check.request("translate", TranslateMessageRequest(text: "Hola, ¿cómo estás?", messageId: "ana-hello", contactId: ana))
        check.request("ai-reply", AIReplyRequest(contactId: ana, messageId: "ana-hello"))
        check.request("conversation-settings", ConversationSettings(
            languageOverride: "Spanish", translationStyle: "casual", sendOriginalFollowUp: true,
            translationEnabled: true, replyOnly: true))
        check.request("viewed-messages", ViewedMessagesRequest(messageIds: ["ana-hello"]))
        check.request("openai-settings", OpenAISettings(model: "gpt-5.6-sol", reasoningEffort: "high", available: nil))
        check.request("message-tone", MessageToneUpdate(tone: .aurora))
        check.request("voice-settings", VoicePreferences(voice: "feminine"))
        check.request("message-topics", ["messageIds": ["ana-hello", "me-reply"]])
    }
}

final class Checker {
    let root: URL
    private var failures: [String] = []
    private var checkedFiles: Set<String> = []

    init(root: URL) { self.root = root }

    func fail(_ message: String) { failures.append(message) }

    func expect(_ condition: Bool, _ what: String) {
        if !condition { fail(what) }
    }

    func equal<T: Equatable>(_ actual: T, _ expected: T, _ what: String) {
        if actual != expected { fail("\(what): expected \(expected), got \(actual)") }
    }

    /// Decodes responses/<name>.json strictly, as APIClient does.
    func response<T: Decodable>(_ name: String, _ type: T.Type, _ assertions: (T) -> Void) {
        let file = "responses/\(name).json"
        checkedFiles.insert(file)
        do {
            let data = try Data(contentsOf: root.appendingPathComponent(file))
            assertions(try JSONDecoder().decode(T.self, from: data))
        } catch {
            fail("\(file) does not decode as \(T.self): \(error)")
        }
    }

    /// The app's encoding of a request must match requests/<name>.json, which the server reads.
    func request<T: Encodable>(_ name: String, _ value: T) {
        let file = "requests/\(name).json"
        checkedFiles.insert(file)
        do {
            let fixture = try canonical(Data(contentsOf: root.appendingPathComponent(file)))
            let encoded = try canonical(JSONEncoder().encode(value))
            if fixture != encoded {
                fail("\(file) differs from the app's \(T.self):\n  fixture: \(fixture)\n  app:     \(encoded)")
            }
        } catch {
            fail("\(file): \(error)")
        }
    }

    private func canonical(_ data: Data) throws -> String {
        let object = try JSONSerialization.jsonObject(with: data)
        let sorted = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(decoding: sorted, as: UTF8.self)
    }

    func finish() {
        for directory in ["responses", "requests"] {
            let files = (try? FileManager.default.contentsOfDirectory(atPath: root.appendingPathComponent(directory).path)) ?? []
            if files.isEmpty { fail("\(directory)/ has no fixtures") }
            for file in files.sorted() where file.hasSuffix(".json") && !checkedFiles.contains("\(directory)/\(file)") {
                fail("\(directory)/\(file) has no Swift check; add one to APIContractCheck.swift")
            }
        }
        if failures.isEmpty {
            print("API contract: \(checkedFiles.count) fixtures decode with APIModels.swift")
            return
        }
        print("API contract check failed:")
        for failure in failures { print("  - \(failure)") }
        exit(1)
    }
}
