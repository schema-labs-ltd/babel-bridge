import Foundation
import Observation
import OSLog
import UniformTypeIdentifiers

extension AppSession {
    func loadDemoData() {
        configuration = try? ServerConfiguration.make(address: "https://translator.example.com", password: "demo")
        backendStatus = BackendStatus(connected: true, phone: "447853803055", name: "Simon Skinner")
        contacts = Contact.demoContacts
        messages = ChatMessage.demoMessages
        if ProcessInfo.processInfo.arguments.contains("-demoChatList") {
            contacts = (1...20).map { index in
                Contact(id: "demo-chat-\(index)@g.us", name: "Community group \(index)", phone: nil, type: "group", lastMessageTime: 1_789_540_000_000 - Int64(index * 60_000), unreadCount: index == 1 ? 2 : 0, pinnedAt: nil, lastMessagePreview: "Alex: Looking forward to seeing everyone this weekend.")
            }
            messages = [:]
        }
        if ProcessInfo.processInfo.arguments.contains("-demoReadReceipts") {
            let chat = "receipts@g.us"
            contacts = [Contact(id: chat, name: "Receipt test", phone: nil, type: "group", lastMessageTime: 1_800_000_000_000, unreadCount: 40, pinnedAt: nil, lastMessagePreview: "Visible messages only")]
            messages = [chat: (0..<40).map { index in
                .demo(id: "receipt-\(index)", contactID: chat, timestamp: 1_800_000_000_000 + Int64(index * 60_000), fromMe: false, body: "Message \(index): This receipt is sent only after you see this message in the active app.", translated: nil, sender: "Alex", chatType: "group")
            }]
            mainTab = .messages
        }
        if ProcessInfo.processInfo.arguments.contains("-demoUnifiedFeed") {
            let base: Int64 = 1_783_940_000_000
            contacts = [
                Contact(id: "weekend@g.us", name: "Weekend plans", phone: nil, type: "group", lastMessageTime: base + 240_000, unreadCount: 2, pinnedAt: nil, lastMessagePreview: "Alex: Meet by the café?"),
                Contact(id: "studio@g.us", name: "Studio team", phone: nil, type: "group", lastMessageTime: base + 300_000, unreadCount: 1, pinnedAt: nil, lastMessagePreview: "Sam: The new draft is ready."),
                Contact(id: "jordan@s.whatsapp.net", name: "Jordan", phone: nil, type: "private", lastMessageTime: base + 180_000, unreadCount: 1, pinnedAt: nil, lastMessagePreview: "See you tomorrow!")
            ]
            messages = [
                "weekend@g.us": [
                    .demo(id: "weekend-first", contactID: "weekend@g.us", timestamp: base, fromMe: false, body: "Shall we head out at ten?", translated: nil, sender: "Alex", chatType: "group"),
                    .demo(id: "weekend-latest", contactID: "weekend@g.us", timestamp: base + 240_000, fromMe: false, body: "Meet by the café?", translated: nil, sender: "Alex", chatType: "group")
                ],
                "studio@g.us": [
                    .demo(id: "studio-first", contactID: "studio@g.us", timestamp: base + 60_000, fromMe: false, body: "Could you take a look at the colours?", translated: nil, sender: "Sam", chatType: "group"),
                    .demo(id: "studio-latest", contactID: "studio@g.us", timestamp: base + 300_000, fromMe: false, body: "The new draft is ready.", translated: nil, sender: "Sam", chatType: "group")
                ],
                "jordan@s.whatsapp.net": [.demo(id: "jordan-latest", contactID: "jordan@s.whatsapp.net", timestamp: base + 180_000, fromMe: false, body: "See you tomorrow!", translated: nil, sender: "Jordan")]
            ]
        }
        if ProcessInfo.processInfo.arguments.contains("-demoTopics") {
            let base: Int64 = 1_789_540_000_000
            contacts = [
                Contact(id: "family@g.us", name: "Family", phone: nil, type: "group", lastMessageTime: base + 4000, unreadCount: 0, pinnedAt: nil, lastMessagePreview: "Alex: Bring a picnic on Saturday."),
                Contact(id: "friends@g.us", name: "Friends", phone: nil, type: "group", lastMessageTime: base + 5000, unreadCount: 0, pinnedAt: nil, lastMessagePreview: "Sam: Sunday brunch at eleven?")
            ]
            let picnic = ChatMessage.demo(id: "topic-picnic", contactID: "family@g.us", timestamp: base, fromMe: false, body: "Shall we have a picnic on Saturday?", translated: nil, sender: "Alex", chatType: "group")
            let football = ChatMessage.demo(id: "topic-football", contactID: "family@g.us", timestamp: base + 1000, fromMe: false, body: "What a goal in last night’s match!", translated: nil, sender: "Jamie", chatType: "group")
            let brunch = ChatMessage.demo(id: "topic-brunch", contactID: "friends@g.us", timestamp: base + 2000, fromMe: false, body: "Sunday brunch at eleven?", translated: nil, sender: "Sam", chatType: "group")
            let picnicReply = ChatMessage.demo(id: "topic-picnic-reply", contactID: "family@g.us", timestamp: base + 4000, fromMe: false, body: "I’ll bring the picnic blanket and sandwiches.", translated: nil, sender: "Alex", chatType: "group", reply: MessageReplyTarget(messageID: "unloaded-original", senderJID: nil, senderName: "Sam", text: "Could someone bring lunch?"))
            let footballReply = ChatMessage.demo(id: "topic-football-reply", contactID: "family@g.us", timestamp: base + 5000, fromMe: ProcessInfo.processInfo.arguments.contains("-demoOutgoingTopic"), body: "The replay is brilliant too.", translated: nil, sender: "Jamie", chatType: "group")
            messages = ["family@g.us": [picnic, football, picnicReply, footballReply], "friends@g.us": [brunch]]
            topicCatalog = TopicCatalog(topics: [
                ChatTopic(id: "family-weekend", contactId: "family@g.us", contactName: "Family", title: "Weekend plans", messageCount: 2, lastMessageTime: base + 4000, categoryId: "category:weekend"),
                ChatTopic(id: "family-football", contactId: "family@g.us", contactName: "Family", title: "Football", messageCount: 2, lastMessageTime: base + 5000, categoryId: "category:football"),
                ChatTopic(id: "friends-weekend", contactId: "friends@g.us", contactName: "Friends", title: "Weekend plans", messageCount: 1, lastMessageTime: base + 2000, categoryId: "category:weekend")
            ], settings: contacts.map { TopicSetting(contactId: $0.id, enabled: true, pendingCount: 0, failedCount: 0) }, available: true)
            topicPages = ["family-weekend": TopicPage(messages: [picnic, picnicReply]), "family-football": TopicPage(messages: [football, footballReply]), "friends-weekend": TopicPage(messages: [brunch])]
            demoTopicCatalog = topicCatalog
            demoTopicPages = topicPages
            feedByID = Dictionary(uniqueKeysWithValues: messages.values.joined().map { ($0.id, $0) })
            feedHasLoaded = true
        }
        if ProcessInfo.processInfo.arguments.contains("-demoWhatsAppLayout") {
            let id = "virag@s.whatsapp.net"
            messages[id] = [
                .demo(id: "compact-in", contactID: id, timestamp: 1_783_939_620_000, fromMe: false, body: "Are you coming along?", translated: nil, sender: nil),
                .demo(id: "compact-out", contactID: id, timestamp: 1_783_939_740_000, fromMe: true, body: "I do…", translated: nil, sender: nil, deliveryStatus: "read"),
                .demoImage(contactID: id, id: "portrait", timestamp: 1_783_940_100_000, caption: "A lovely afternoon in the sunshine. ☀️"),
                .demo(id: "compact-last", contactID: id, timestamp: 1_783_940_400_000, fromMe: false, body: "See you soon", translated: nil, sender: nil)
            ]
        }
        if ProcessInfo.processInfo.arguments.contains("-demoPhotoAlbum"),
           var conversation = messages["virag@s.whatsapp.net"],
           let imageIndex = conversation.firstIndex(where: \.isImage) {
            let album = (0..<4).map { index in
                ChatMessage.demoImage(
                    contactID: "virag@s.whatsapp.net",
                    id: "album-photo-\(index)",
                    timestamp: 1_783_940_100_000 + Int64(index),
                    albumID: "demo-album",
                    albumIndex: index,
                    caption: index == 0 ? "A few photos from this afternoon." : nil
                )
            }
            conversation.replaceSubrange(imageIndex...imageIndex, with: album)
            messages["virag@s.whatsapp.net"] = conversation
        }
        for imageMessage in messages["virag@s.whatsapp.net"]?.filter(\.isImage) ?? [] {
            messageImages[imageMessage.id] = demoPhoto()
        }
        if demoConversationMode {
            mainTab = .chats
            selectedContactID = contacts.first(where: \.showsInChatList)?.id
            if ProcessInfo.processInfo.arguments.contains("-demoSending"),
               let selectedContactID {
                sendingContactIDs.insert(selectedContactID)
            }
        }
        phase = .ready
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-demoVideo"),
           let url = Bundle.main.url(forResource: "video-playback-fixture", withExtension: "mp4") {
            let chat = "video-preview@g.us"
            let value: [String: Any] = [
                "id": "video-preview", "contactId": chat, "timestamp": 1_789_661_000_000,
                "isFromMe": false, "isForwarded": false, "senderName": "Alex", "contactName": "Video preview",
                "chatType": "group", "contentType": "Video", "isTranslated": false,
                "content": ["type": "video", "mimeType": "video/mp4", "hasMedia": true,
                            "caption": "A sample video to try full screen and picture in picture."]
            ]
            if let data = try? JSONSerialization.data(withJSONObject: value),
               let video = try? JSONDecoder().decode(ChatMessage.self, from: data) {
                contacts = [Contact(id: chat, name: "Video preview", phone: nil, type: "group", lastMessageTime: video.timestamp, unreadCount: 0, pinnedAt: nil, lastMessagePreview: "Video")]
                messages = [chat: [video]]
                messageMediaURLs[video.id] = url
                feedByID = [video.id: video]
                feedHasLoaded = true
            }
        }
        if ProcessInfo.processInfo.arguments.contains("-demoPhotoGallery") {
            loadPhotoGalleryDemo()
        }
        if ProcessInfo.processInfo.arguments.contains("-demoChatGallery") {
            loadChatGalleryDemo()
        }
        ClipboardImages.prepareDemoClipboard()
        if ProcessInfo.processInfo.arguments.contains("-demoLiveReactions") {
            startLiveReactionDemo()
        }
        if ProcessInfo.processInfo.arguments.contains("-demoLiveEdits") {
            startLiveEditDemo()
        }
        #endif
    }

    #if DEBUG
    private func loadChatGalleryDemo() {
        let chat = "chat-gallery-preview@g.us"
        let base: Int64 = 1_789_770_000_000
        contacts = [Contact(id: chat, name: "Weekend adventures", phone: nil, type: "group", lastMessageTime: base, unreadCount: 0, pinnedAt: nil, lastMessagePreview: "Photos and videos from our trip")]
        galleryDemoMessages = (0..<75).compactMap { index in
            let video = index % 7 == 1
            let value: [String: Any] = [
                "id": "chat-gallery-\(String(format: "%03d", index))", "contactId": chat,
                "timestamp": base - Int64(index / 12) * 86_400_000 - Int64(index % 12) * 60_000,
                "isFromMe": index % 3 == 0, "isForwarded": false, "senderName": "Alex",
                "chatType": "group", "contentType": video ? "Video" : "Image", "isTranslated": false,
                "content": ["type": video ? "video" : "image", "has_media": true,
                            "duration_seconds": 6, "caption": "A moment from our trip"]
            ]
            guard let data = try? JSONSerialization.data(withJSONObject: value),
                  let message = try? JSONDecoder().decode(ChatMessage.self, from: data) else { return nil }
            if video { messageMediaURLs[message.id] = Bundle.main.url(forResource: "video-playback-fixture", withExtension: "mp4") }
            else { messageImages[message.id] = DemoImageFactory.landscape(size: CGSize(width: 640, height: index.isMultiple(of: 3) ? 800 : 420)) }
            return message
        }
        messages = [chat: Array(galleryDemoMessages.prefix(12).reversed())]
        mainTab = .chats
        selectedContactID = chat
    }

    private func startLiveEditDemo() {
        let chat = "edit-preview@g.us"
        let timestamp = Int64(Date().timeIntervalSince1970 * 1_000) - 60_000
        let original = ChatMessage.demo(id: "edit-preview-original", contactID: chat, timestamp: timestamp, fromMe: false, body: "Get", translated: nil, sender: "Alex", chatType: "group")
        let reply = ChatMessage.demo(id: "edit-preview-reply", contactID: chat, timestamp: timestamp + 30_000, fromMe: true, body: "Get what?", translated: nil, sender: nil, deliveryStatus: "read", chatType: "group")
        contacts = [Contact(id: chat, name: "Edit preview", phone: nil, type: "group", lastMessageTime: reply.timestamp, unreadCount: 0, pinnedAt: nil, lastMessagePreview: reply.displayText)]
        messages = [chat: [original, reply]]
        feedByID = [original.id: original, reply.id: reply]
        feedHasLoaded = true
        mainTab = .messages
        selectedContactID = nil
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(8))
            guard let data = try? JSONEncoder().encode(original),
                  var value = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return }
            value["content"] = ["type": "text", "body": "Grr", "edited_at_ms": timestamp + 45_000]
            value["originalText"] = "Grr"
            guard let data = try? JSONSerialization.data(withJSONObject: ["type": "message_updated", "message": value]),
                  let event = try? JSONDecoder().decode(LiveEvent.self, from: data) else { return }
            handle(event)
        }
    }

    private func loadPhotoGalleryDemo() {
        let contactID = "gallery-preview@g.us"
        let base = Int64(Date().timeIntervalSince1970 * 1_000) - 120_000
        contacts = [Contact(id: contactID, name: "Weekend photos", phone: nil, type: "group", lastMessageTime: base + 40_000, unreadCount: 0, pinnedAt: nil, lastMessagePreview: "Alex: 12 photos")]
        var photos: [ChatMessage] = []
        for index in 0..<12 {
            let payload: [String: Any] = [
                "id": "gallery-preview-\(index)", "contactId": contactID,
                "timestamp": base + Int64(index) * 1_000, "isFromMe": false,
                "isForwarded": false, "senderName": "Alex", "senderPhone": "447700900123",
                "contactName": "Weekend photos", "chatType": "group", "contentType": "Image",
                "content": ["type": "image", "caption": "Photo \(index + 1) from our afternoon walk.", "has_media": true],
                "isTranslated": false,
            ]
            guard let data = try? JSONSerialization.data(withJSONObject: payload),
                  var photo = try? JSONDecoder().decode(ChatMessage.self, from: data) else { continue }
            if index == 7 { photo.reactions = ["❤️": ["447700900456"]] }
            photos.append(photo)
            messageImages[photo.id] = DemoImageFactory.landscape(size: CGSize(width: 640, height: index.isMultiple(of: 2) ? 420 : 800))
        }
        messages = [contactID: photos]
        feedByID = Dictionary(uniqueKeysWithValues: photos.map { ($0.id, $0) })
        feedHasLoaded = true
        mainTab = .messages
        selectedContactID = contactID
    }

    private func startLiveReactionDemo() {
        let contactID = "reaction-preview@g.us"
        let timestamp = Int64(Date().timeIntervalSince1970 * 1_000)
        contacts = [Contact(id: contactID, name: "Weekend plans", phone: nil, type: "group", lastMessageTime: timestamp, unreadCount: 0, pinnedAt: nil, lastMessagePreview: "You: I’ll be there at six.")]
        let target = ChatMessage.demo(id: "reaction-preview-target", contactID: contactID, timestamp: timestamp, fromMe: true, body: "I’ll be there at six.", translated: nil, sender: nil, deliveryStatus: "read", chatType: "group")
        messages = [contactID: [target]]
        feedByID = [target.id: target]
        Task { [weak self] in
            for (index, emoji) in ["❤️", "👍", "", "❤️"].enumerated() {
                try? await Task.sleep(for: .seconds(index == 0 ? 20 : 10))
                guard let self, !Task.isCancelled else { return }
                let payload: [String: Any] = ["type": "reaction", "message": [
                    "id": "reaction-preview-\(index)", "contactId": contactID,
                    "timestamp": timestamp + Int64(index + 1), "isFromMe": false,
                    "isForwarded": false, "senderName": "Alex", "senderPhone": "447700900123",
                    "chatType": "group", "contentType": "Reaction", "isTranslated": false,
                    "content": ["type": "reaction", "target_message_id": target.id, "emoji": emoji]
                ]]
                if let data = try? JSONSerialization.data(withJSONObject: payload),
                   let event = try? JSONDecoder().decode(LiveEvent.self, from: data) {
                    handle(event)
                    AppLog.live.debug("Live reaction preview step \(index + 1, privacy: .public)")
                }
            }
        }
    }
    #endif

    private func demoPhoto() -> PlatformImage {
        DemoImageFactory.landscape(size: CGSize(width: 640, height: ProcessInfo.processInfo.arguments.contains("-demoWhatsAppLayout") ? 1400 : 420))
    }
}
