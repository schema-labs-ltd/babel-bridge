package main

import (
	"encoding/base64"
	"fmt"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/proto/waHistorySync"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
)

// handleEvent processes events from whatsmeow
func (c *Client) handleEvent(evt interface{}) {
	c.observeConnection(evt)
	// Log all event types for debugging (enable with verbose mode)
	if c.verbose {
		SendEvent(NewLogEvent("debug", fmt.Sprintf("Event received: %T", evt)))
	}
	if updates := connectionEvents(evt); updates != nil {
		for _, update := range updates {
			SendEvent(update)
		}
		return
	}

	switch v := evt.(type) {
	case *events.Connected:
		c.sendConnectedEvent()

	case *events.KeepAliveRestored:
		if c.client.IsConnected() && c.client.IsLoggedIn() {
			c.sendConnectedEvent()
		}

	case *events.Message:
		c.handleMessage(v)

	case *events.Receipt:
		if v.Type == types.ReceiptTypeDelivered || v.Type == types.ReceiptTypeRead || v.Type == types.ReceiptTypePlayed {
			SendEvent(NewReceiptEvent(v.MessageIDs, v.Type))
		}
		if c.verbose {
			SendEvent(NewLogEvent("debug", fmt.Sprintf("Receipt: %s", v.Type)))
		}

	case *events.Presence:
		// Online/offline status - could be useful in future
		if c.verbose {
			SendEvent(NewLogEvent("debug", fmt.Sprintf("Presence: unavailable=%t", v.Unavailable)))
		}

	case *events.ChatPresence:
		// Typing/recording indicators
		state := "paused"
		switch v.State {
		case types.ChatPresenceComposing:
			// Check if it's audio recording vs text typing
			if v.Media == types.ChatPresenceMediaAudio {
				state = "recording"
			} else {
				state = "typing"
			}
		case types.ChatPresencePaused:
			state = "paused"
		}
		SendEvent(NewLogEvent("info", chatPresenceLog(fmt.Sprint(v.State), fmt.Sprint(v.Media))))
		// Send chat presence event
		SendEvent(NewChatPresenceEvent(normalizeAddress(v.Chat, types.JID{}).String(), normalizeAddress(v.Sender, types.JID{}).String(), state))

	case *events.HistorySync:
		// History sync - import historical messages
		SendEvent(NewLogEvent("info", fmt.Sprintf("Received history sync: %d conversations", len(v.Data.Conversations))))
		c.processHistorySync(v.Data)

	case *events.UndecryptableMessage:
		// Message couldn't be decrypted
		SendEvent(NewLogEvent("error", fmt.Sprintf("Undecryptable message: mode=%v", v.DecryptFailMode)))

	case *events.PushName:
		// Contact name updates
		if c.verbose {
			SendEvent(NewLogEvent("debug", "PushName update"))
		}

	case *events.MarkChatAsRead:
		// Chat marked as read from another device (e.g., phone)
		if v.Action != nil && v.Action.GetRead() {
			SendEvent(NewMarkAsReadEvent(normalizeAddress(v.JID, types.JID{}).String()))
			SendEvent(NewLogEvent("debug", "Chat marked as read"))
		}

	case *events.OfflineSyncCompleted:
		// Offline sync completed - all pending messages delivered
		SendEvent(NewLogEvent("info", "Offline sync completed"))
	}
}

// sendConnectedEvent sends the connected event with device info
func (c *Client) sendConnectedEvent() {
	phone := ""
	name := ""
	platform := ""

	if c.client.Store.ID != nil {
		phone = c.client.Store.ID.User
	}

	if c.client.Store.PushName != "" {
		name = c.client.Store.PushName
	}

	if c.client.Store.Platform != "" {
		platform = c.client.Store.Platform
	}

	SendEvent(NewConnectedEvent(phone, name, platform))
}

// handleMessage processes incoming messages
func (c *Client) handleMessage(evt *events.Message) {
	msg := Message{
		ID:        evt.Info.ID,
		Timestamp: evt.Info.Timestamp.Unix(),
		IsFromMe:  evt.Info.IsFromMe,
	}

	// Check for forwarding info in the message itself
	if evt.Message != nil && evt.Message.ExtendedTextMessage != nil {
		if evt.Message.ExtendedTextMessage.ContextInfo != nil {
			msg.IsForwarded = evt.Message.ExtendedTextMessage.ContextInfo.IsForwarded != nil &&
				*evt.Message.ExtendedTextMessage.ContextInfo.IsForwarded
		}
	}

	// Set push name
	if evt.Info.PushName != "" {
		msg.PushName = evt.Info.PushName
	}

	// Set sender info
	msg.From = c.buildContact(evt.Info.Sender, evt.Info.SenderAlt)

	// Set chat info
	msg.Chat = c.buildChat(evt.Info)

	// Set message content (with media download)
	if id, editedAt, content, ok := c.extractMessageEdit(evt); ok {
		msg.ID = id
		msg.Content = c.buildMessageContent(content)
		msg.ReplyContext = c.incomingReplyContext(content)
		SendEvent(NewMessageEditEvent(msg, editedAt))
		return
	}
	msg.Content = c.buildMessageContent(evt.Message)
	msg.ReplyContext = c.incomingReplyContext(evt.Message)

	// Skip protocol messages and unknown types - these shouldn't be displayed
	if msg.Content.Type == "protocol" || msg.Content.Type == "unknown" {
		return
	}

	// Download media for image/video/audio/document messages
	if evt.Message != nil {
		c.downloadMediaForMessage(evt.Message, &msg.Content)
	}

	// Send the message event
	SendEvent(NewMessageEvent(msg))
}

// processHistorySync processes historical messages from WhatsApp history sync
func (c *Client) processHistorySync(data *waHistorySync.HistorySync) {
	if data == nil {
		return
	}

	totalMessages := 0
	for _, conv := range data.Conversations {
		if conv == nil || conv.ID == nil {
			continue
		}

		chatJIDParsed, err := types.ParseJID(*conv.ID)
		if err != nil {
			continue
		}
		chatJIDParsed = normalizeAddress(chatJIDParsed, types.JID{})
		chatJID := chatJIDParsed.String()

		// Get unread count from conversation (only set on first message)
		var convUnreadCount *uint32
		if conv.UnreadCount != nil {
			convUnreadCount = conv.UnreadCount
		}
		firstMessageInConv := true

		for _, historyMsg := range conv.Messages {
			if historyMsg == nil || historyMsg.Message == nil {
				continue
			}

			webMsg := historyMsg.Message
			if webMsg.Key == nil {
				continue
			}

			// Parse the message
			msg := Message{
				ID:        webMsg.Key.GetID(),
				Timestamp: time.Unix(int64(webMsg.GetMessageTimestamp()), 0).Unix(),
				IsFromMe:  webMsg.Key.GetFromMe(),
			}

			// Determine sender and chat
			if webMsg.Key.GetFromMe() {
				// Outgoing message - sender is me
				if c.client.Store.ID != nil {
					msg.From = Contact{
						JID:   c.client.Store.ID.ToNonAD().String(),
						Phone: c.client.Store.ID.User,
					}
				}
			} else {
				// Incoming message
				var senderJID types.JID
				if webMsg.Key.Participant != nil && *webMsg.Key.Participant != "" {
					// Group message - participant is sender
					senderJID, _ = types.ParseJID(*webMsg.Key.Participant)
				} else if webMsg.Key.RemoteJID != nil {
					// Private message - remote JID is sender
					senderJID, _ = types.ParseJID(*webMsg.Key.RemoteJID)
				}

				// Build contact from JID
				msg.From = c.buildContact(senderJID, types.JID{})

				// If we have a push name from the message, use it as the contact name
				// This is more reliable for history messages where contact store might not have the info
				if webMsg.PushName != nil && *webMsg.PushName != "" {
					msg.From.Name = *webMsg.PushName
				}
			}

			// Set push name if available
			if webMsg.PushName != nil {
				msg.PushName = *webMsg.PushName
			}

			// Build chat info
			// Determine if it's a group
			isGroup := strings.HasSuffix(chatJID, "@g.us")

			msg.Chat = Chat{
				JID:  chatJID,
				Type: "private",
			}

			if isGroup {
				msg.Chat.Type = "group"
				// First try to get name from the conversation object (most reliable for history)
				if conv.Name != nil && *conv.Name != "" {
					msg.Chat.Name = *conv.Name
				} else if conv.DisplayName != nil && *conv.DisplayName != "" {
					msg.Chat.Name = *conv.DisplayName
				} else {
					// Fallback to GetGroupInfo (may fail for left/archived groups)
					groupInfo, err := c.client.GetGroupInfo(c.ctx, chatJIDParsed)
					if err == nil {
						msg.Chat.Name = groupInfo.Name
						count := len(groupInfo.Participants)
						msg.Chat.ParticipantCount = &count
					}
				}
			} else {
				// Private chat - first try conversation DisplayName/Name
				if conv.DisplayName != nil && *conv.DisplayName != "" {
					msg.Chat.Name = *conv.DisplayName
				} else if conv.Name != nil && *conv.Name != "" {
					msg.Chat.Name = *conv.Name
				} else {
					// Fallback to contact store
					contactInfo, err := c.client.Store.Contacts.GetContact(c.ctx, chatJIDParsed)
					if err == nil && contactInfo.Found {
						if contactInfo.FullName != "" {
							msg.Chat.Name = contactInfo.FullName
						} else if contactInfo.PushName != "" {
							msg.Chat.Name = contactInfo.PushName
						}
					}
				}
			}

			// Parse message content
			if webMsg.Message == nil {
				continue
			}

			// The message is wrapped in a WebMessageInfo, need to unwrap
			waMessage := webMsg.Message
			if parsed, err := c.client.ParseWebMessage(chatJIDParsed, webMsg); err == nil {
				if id, editedAt, content, ok := c.extractMessageEdit(parsed); ok {
					msg.ID = id
					msg.Content = c.buildMessageContent(content)
					msg.ReplyContext = c.incomingReplyContext(content)
					msg.IsHistory = true
					SendEvent(NewMessageEditEvent(msg, editedAt))
					continue
				}
				waMessage = parsed.Message
			}
			msg.Content = c.buildMessageContent(waMessage)
			msg.ReplyContext = c.incomingReplyContext(waMessage)

			// Skip protocol/unknown messages
			if msg.Content.Type == "protocol" || msg.Content.Type == "unknown" {
				continue
			}

			// Don't download media for history (too slow) - just send metadata
			// Users can see what media exists but won't have the actual data

			// Mark as history message (no translation)
			msg.IsHistory = true

			// Set unread count on first message of each conversation
			if firstMessageInConv && convUnreadCount != nil {
				msg.UnreadCount = convUnreadCount
				firstMessageInConv = false
			}

			SendEvent(NewMessageEvent(msg))
			totalMessages++
		}
	}

	SendEvent(NewLogEvent("info", fmt.Sprintf("History sync complete: imported %d messages", totalMessages)))
}

// downloadMediaForMessage downloads media data and adds it to the content
func (c *Client) downloadMediaForMessage(waMsg *waE2E.Message, content *MessageContent) {
	var data []byte
	var err error

	// Size limit for media downloads (50MB)
	const maxMediaSize uint64 = 50 * 1024 * 1024

	SendEvent(NewLogEvent("info", fmt.Sprintf("Attempting media download for type: %s, size: %d", content.Type, content.FileSize)))

	switch content.Type {
	case "image":
		if waMsg.ImageMessage != nil {
			if content.FileSize > 0 && content.FileSize > maxMediaSize {
				SendEvent(NewLogEvent("warn", fmt.Sprintf("Image too large to download: %d bytes", content.FileSize)))
				return
			}
			SendEvent(NewLogEvent("info", "Downloading image..."))
			data, err = c.client.Download(c.ctx, waMsg.ImageMessage)
		} else {
			SendEvent(NewLogEvent("warn", "Image content type but ImageMessage is nil"))
		}
	case "video":
		if waMsg.VideoMessage != nil {
			if content.FileSize > 0 && content.FileSize > maxMediaSize {
				SendEvent(NewLogEvent("warn", fmt.Sprintf("Video too large to download: %d bytes", content.FileSize)))
				return
			}
			SendEvent(NewLogEvent("info", "Downloading video..."))
			data, err = c.client.Download(c.ctx, waMsg.VideoMessage)
		} else {
			SendEvent(NewLogEvent("warn", "Video content type but VideoMessage is nil"))
		}
	case "audio":
		if waMsg.AudioMessage != nil {
			if content.FileSize > 0 && content.FileSize > maxMediaSize {
				SendEvent(NewLogEvent("warn", fmt.Sprintf("Audio too large to download: %d bytes", content.FileSize)))
				return
			}
			SendEvent(NewLogEvent("info", "Downloading audio..."))
			data, err = c.client.Download(c.ctx, waMsg.AudioMessage)
		} else {
			SendEvent(NewLogEvent("warn", "Audio content type but AudioMessage is nil"))
		}
	case "document":
		if waMsg.DocumentMessage != nil {
			if content.FileSize > 0 && content.FileSize > maxMediaSize {
				SendEvent(NewLogEvent("warn", fmt.Sprintf("Document too large to download: %d bytes", content.FileSize)))
				return
			}
			SendEvent(NewLogEvent("info", "Downloading document..."))
			data, err = c.client.Download(c.ctx, waMsg.DocumentMessage)
		} else {
			SendEvent(NewLogEvent("warn", "Document content type but DocumentMessage is nil"))
		}
	case "sticker":
		if waMsg.StickerMessage != nil {
			SendEvent(NewLogEvent("info", "Downloading sticker..."))
			data, err = c.client.Download(c.ctx, waMsg.StickerMessage)
		} else {
			SendEvent(NewLogEvent("warn", "Sticker content type but StickerMessage is nil"))
		}
	default:
		return
	}

	if err != nil {
		SendEvent(NewLogEvent("warn", fmt.Sprintf("Failed to download media: %v", err)))
		return
	}

	if len(data) > 0 {
		SendEvent(NewLogEvent("info", fmt.Sprintf("Media downloaded successfully: %d bytes", len(data))))
		content.MediaData = base64.StdEncoding.EncodeToString(data)
	} else {
		SendEvent(NewLogEvent("warn", "Media download returned empty data"))
	}
}
