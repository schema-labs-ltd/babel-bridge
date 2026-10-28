import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension Contact {
    static let demoContacts = [
        Contact(id: "status@broadcast", name: nil, phone: nil, type: "broadcast", lastMessageTime: 1_783_941_000_000, unreadCount: 7, pinnedAt: nil, lastMessagePreview: "A status update"),
        Contact(id: "virag@s.whatsapp.net", name: "Virág Skinner", phone: "+36 30 555 0142", type: "private", lastMessageTime: 1_783_940_400_000, unreadCount: 2, pinnedAt: 1, lastMessagePreview: "Perfect, I’ll be there just after six."),
        Contact(id: "family@g.us", name: "Sample Group", phone: nil, type: "group", lastMessageTime: 1_783_936_860_000, unreadCount: 0, pinnedAt: nil, lastMessagePreview: "Sample Sender: Photo"),
        Contact(id: "anyu@s.whatsapp.net", name: "Anyu", phone: "+36 20 555 0181", type: "private", lastMessageTime: 1_783_850_000_000, unreadCount: 1, pinnedAt: nil, lastMessagePreview: "Nagyon köszönöm ❤️"),
        Contact(id: "work@g.us", name: "AFX / RCX Hedge Funds", phone: nil, type: "group", lastMessageTime: 1_783_760_000_000, unreadCount: 0, pinnedAt: nil, lastMessagePreview: "Will check it this afternoon."),
    ]
}

extension ChatMessage {
    static let demoMessages: [String: [ChatMessage]] = [
        "virag@s.whatsapp.net": [
            demo(id: "1", contactID: "virag@s.whatsapp.net", timestamp: 1_783_939_620_000, fromMe: false, body: "Mikor érkezel?", translated: "What time will you arrive?", sender: "Virág"),
            demo(id: "2", contactID: "virag@s.whatsapp.net", timestamp: 1_783_939_740_000, fromMe: true, body: "Probably just after six — I’ll message when I leave.", translated: "Valószínűleg nem sokkal hat után — írok, amikor elindulok.", sender: nil, deliveryStatus: "read"),
            demo(id: "link", contactID: "virag@s.whatsapp.net", timestamp: 1_783_939_900_000, fromMe: false, body: "This looks lovely: https://example.com/weekend", translated: nil, sender: "Virág"),
            demoImage(contactID: "virag@s.whatsapp.net", id: "image", timestamp: 1_783_940_100_000),
            demo(id: "3", contactID: "virag@s.whatsapp.net", timestamp: 1_783_940_400_000, fromMe: false, body: "Tökéletes, akkor hat után várlak.", translated: "Perfect, I’ll be there just after six.", sender: "Virág"),
            demo(id: "4", contactID: "virag@s.whatsapp.net", timestamp: 1_783_940_520_000, fromMe: false, body: "😉", translated: nil, sender: "Virág", chatType: "group"),
        ]
    ]

    static func demoOutgoing(contactID: String, text: String, reply: MessageReplyTarget? = nil) -> ChatMessage {
        demo(id: UUID().uuidString, contactID: contactID, timestamp: Int64(Date().timeIntervalSince1970 * 1_000), fromMe: true, body: text, translated: nil, sender: nil, reply: reply)
    }

    static func demoImage(
        contactID: String,
        id: String = UUID().uuidString,
        timestamp: Int64 = Int64(Date().timeIntervalSince1970 * 1_000),
        albumID: String? = nil,
        albumIndex: Int? = nil,
        caption: String? = "The light is gorgeous here today."
    ) -> ChatMessage {
        ChatMessage(
            id: id,
            contactId: contactID,
            timestamp: timestamp,
            isFromMe: false,
            isForwarded: false,
            senderName: "Virág",
            senderPhone: "36305550142",
            contactName: "Virág Skinner",
            contactPhone: "+36 30 555 0142",
            chatType: "private",
            contentType: "Image",
            content: MessageContent(
                type: "image",
                body: nil,
                caption: caption,
                mimeType: "image/jpeg",
                hasMedia: true,
                albumId: albumID,
                albumIndex: albumIndex,
                showTranslatedPrimary: nil,
                replyContext: nil
            ),
            originalText: nil,
            translatedText: nil,
            sourceLanguage: nil,
            isTranslated: false
        )
    }

    static func demoTranslated(from message: ChatMessage) -> ChatMessage {
        ChatMessage(
            id: message.id,
            contactId: message.contactId,
            timestamp: message.timestamp,
            isFromMe: message.isFromMe,
            isForwarded: message.isForwarded,
            senderName: message.senderName,
            senderPhone: message.senderPhone,
            contactName: message.contactName,
            contactPhone: message.contactPhone,
            chatType: message.chatType,
            contentType: message.contentType,
            content: message.content,
            originalText: message.contentText,
            translatedText: "Translated: \(message.contentText ?? message.displayText)",
            sourceLanguage: "Hungarian",
            isTranslated: true,
            reactions: message.reactions,
            reactionStates: message.reactionStates
        )
    }

    static func demo(id: String, contactID: String, timestamp: Int64, fromMe: Bool, body: String, translated: String?, sender: String?, deliveryStatus: String? = nil, chatType: String = "private", reply: MessageReplyTarget? = nil) -> ChatMessage {
        ChatMessage(
            id: id,
            contactId: contactID,
            timestamp: timestamp,
            isFromMe: fromMe,
            isForwarded: false,
            senderName: sender,
            senderPhone: nil,
            contactName: nil,
            contactPhone: nil,
            chatType: chatType,
            contentType: "Text",
            content: MessageContent(type: "text", body: body, showTranslatedPrimary: nil, replyContext: reply.map { ReplyContext(messageId: $0.messageID, senderName: $0.senderName, text: $0.text) }),
            originalText: translated == nil ? nil : body,
            translatedText: translated,
            sourceLanguage: translated == nil ? nil : "Hungarian",
            isTranslated: translated != nil,
            deliveryStatus: deliveryStatus
        )
    }
}
