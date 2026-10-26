package main

import (
	"encoding/hex"
	"fmt"
	"strings"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
)

func normalizeAddress(primary types.JID, alternate types.JID) types.JID {
	if primary.Server == types.HiddenUserServer && alternate.User != "" &&
		(alternate.Server == types.DefaultUserServer || alternate.Server == types.LegacyUserServer) {
		primary = alternate
	}
	// SenderAlt also carries the sender's device number. Conversation/contact
	// identity must refer to the account, including after resolving LID -> phone.
	switch primary.Server {
	case types.DefaultUserServer, types.LegacyUserServer, types.HiddenUserServer:
		return primary.ToNonAD()
	}
	return primary
}

func normalizePrivateChatJID(info types.MessageInfo) types.JID {
	if info.IsGroup || strings.HasSuffix(info.Chat.Server, "broadcast") || info.Chat.Server == "status@broadcast" {
		return info.Chat
	}

	if info.IsFromMe {
		return normalizeAddress(info.Chat, info.RecipientAlt)
	}

	return normalizeAddress(info.Chat, info.SenderAlt)
}

// buildContact creates a Contact from a JID
func (c *Client) buildContact(jid types.JID, alternate types.JID) Contact {
	jid = normalizeAddress(jid, alternate)
	contact := Contact{
		JID: jid.String(),
	}

	// Only set phone if it's a real phone number (not a LID)
	// LIDs have server "lid" and the user part is not a phone number
	if jid.Server != "lid" && jid.User != "" {
		contact.Phone = jid.User
	}

	// Try to get contact name from store
	contactInfo, err := c.client.Store.Contacts.GetContact(c.ctx, jid)
	if err == nil && contactInfo.Found {
		if contactInfo.FullName != "" {
			contact.Name = contactInfo.FullName
		} else if contactInfo.PushName != "" {
			contact.Name = contactInfo.PushName
		} else if contactInfo.BusinessName != "" {
			contact.Name = contactInfo.BusinessName
		}
	}

	return contact
}

func (c *Client) buildMentions(jids []string) []Mention {
	mentions := make([]Mention, 0, len(jids))
	for _, jidString := range jids {
		jid, err := types.ParseJID(jidString)
		if err != nil || jid.User == "" {
			continue
		}

		originalContact := c.buildContact(jid, types.JID{})
		contact := originalContact
		if jid.Server == "lid" && c.client.Store.LIDs != nil {
			if phoneJID, err := c.client.Store.LIDs.GetPNForLID(c.ctx, jid); err == nil && phoneJID.User != "" {
				phoneContact := c.buildContact(phoneJID, types.JID{})
				contact.Phone = phoneJID.User
				if phoneContact.Name != "" {
					contact.Name = phoneContact.Name
				} else if contact.Name == "" {
					contact.Name = originalContact.Name
				}
			}
		}

		mentions = append(mentions, Mention{
			JID:   jidString,
			Phone: contact.Phone,
			Name:  contact.Name,
		})
	}
	return mentions
}

func mentionTokenUser(jid string) string {
	if parsed, err := types.ParseJID(jid); err == nil {
		return parsed.User
	}
	if user, _, found := strings.Cut(jid, "@"); found {
		return user
	}
	return jid
}

func replaceMentionTokens(text string, mentions []Mention) string {
	for _, mention := range mentions {
		user := mentionTokenUser(mention.JID)
		if user == "" {
			continue
		}

		displayName := mention.Name
		if displayName == "" {
			displayName = mention.Phone
		}
		if displayName == "" || displayName == user {
			continue
		}

		token := "@" + user
		for start := 0; start < len(text); {
			relativeIndex := strings.Index(text[start:], token)
			if relativeIndex < 0 {
				break
			}
			index := start + relativeIndex
			end := index + len(token)
			if end < len(text) && text[end] >= '0' && text[end] <= '9' {
				start = end
				continue
			}
			text = text[:index] + "@" + displayName + text[end:]
			start = index + len(displayName) + 1
		}
	}
	return text
}

func (c *Client) resolveMentions(text string, contextInfo *waE2E.ContextInfo) (string, []Mention) {
	if contextInfo == nil || len(contextInfo.GetMentionedJID()) == 0 {
		return text, nil
	}
	mentions := c.buildMentions(contextInfo.GetMentionedJID())
	return replaceMentionTokens(text, mentions), mentions
}

// buildChat creates a Chat from message info
func (c *Client) buildChat(info types.MessageInfo) Chat {
	chatJID := normalizePrivateChatJID(info)
	chat := Chat{
		JID: chatJID.String(),
	}

	if info.IsGroup {
		chat.Type = "group"
		// Try to get group info
		groupInfo, err := c.client.GetGroupInfo(c.ctx, info.Chat)
		if err == nil {
			chat.Name = groupInfo.Name
			count := len(groupInfo.Participants)
			chat.ParticipantCount = &count
		}
	} else if strings.HasSuffix(info.Chat.Server, "broadcast") {
		chat.Type = "broadcast"
	} else if info.Chat.Server == "status@broadcast" {
		chat.Type = "status"
	} else {
		chat.Type = "private"
		// For private chats, get the contact name from the chat JID (the other person)
		contactInfo, err := c.client.Store.Contacts.GetContact(c.ctx, chatJID)
		if err == nil && contactInfo.Found {
			if contactInfo.FullName != "" {
				chat.Name = contactInfo.FullName
			} else if contactInfo.PushName != "" {
				chat.Name = contactInfo.PushName
			} else if contactInfo.BusinessName != "" {
				chat.Name = contactInfo.BusinessName
			}
		}
	}

	return chat
}

// buildMessageContent extracts content from a WhatsApp message
func (c *Client) buildMessageContent(msg *waE2E.Message) MessageContent {
	if msg == nil {
		return MessageContent{Type: "unknown", RawType: "nil"}
	}

	// Text message
	if msg.Conversation != nil && *msg.Conversation != "" {
		return MessageContent{
			Type: "text",
			Body: *msg.Conversation,
		}
	}

	// Extended text message (with URL preview, etc.)
	if msg.ExtendedTextMessage != nil {
		body := ""
		if msg.ExtendedTextMessage.Text != nil {
			body = *msg.ExtendedTextMessage.Text
		}
		body, mentions := c.resolveMentions(body, msg.ExtendedTextMessage.ContextInfo)
		return MessageContent{
			Type:     "text",
			Body:     body,
			Mentions: mentions,
		}
	}

	// Image message
	if msg.ImageMessage != nil {
		content := MessageContent{
			Type:     "image",
			MimeType: getString(msg.ImageMessage.Mimetype),
			FileSize: getUint64(msg.ImageMessage.FileLength),
			Width:    msg.ImageMessage.GetWidth(),
			Height:   msg.ImageMessage.GetHeight(),
		}
		if msg.ImageMessage.Caption != nil {
			content.Caption, content.Mentions = c.resolveMentions(*msg.ImageMessage.Caption, msg.ImageMessage.ContextInfo)
		}
		if msg.ImageMessage.FileSHA256 != nil {
			content.FileHash = hex.EncodeToString(msg.ImageMessage.FileSHA256)
		}
		applyAlbumAssociation(msg, &content)
		return content
	}

	// Video message
	if msg.VideoMessage != nil {
		content := MessageContent{
			Type:     "video",
			MimeType: getString(msg.VideoMessage.Mimetype),
			FileSize: getUint64(msg.VideoMessage.FileLength),
		}
		if msg.VideoMessage.Caption != nil {
			content.Caption, content.Mentions = c.resolveMentions(*msg.VideoMessage.Caption, msg.VideoMessage.ContextInfo)
		}
		if msg.VideoMessage.Seconds != nil {
			dur := *msg.VideoMessage.Seconds
			content.DurationSeconds = &dur
		}
		return content
	}

	// Audio message
	if msg.AudioMessage != nil {
		content := MessageContent{
			Type:     "audio",
			MimeType: getString(msg.AudioMessage.Mimetype),
			FileSize: getUint64(msg.AudioMessage.FileLength),
		}
		if msg.AudioMessage.Seconds != nil {
			dur := *msg.AudioMessage.Seconds
			content.DurationSeconds = &dur
		}
		if msg.AudioMessage.PTT != nil {
			content.IsVoiceNote = *msg.AudioMessage.PTT
		}
		return content
	}

	// Document message
	if msg.DocumentMessage != nil {
		content := MessageContent{
			Type:     "document",
			MimeType: getString(msg.DocumentMessage.Mimetype),
			FileSize: getUint64(msg.DocumentMessage.FileLength),
		}
		if msg.DocumentMessage.FileName != nil {
			content.FileName = *msg.DocumentMessage.FileName
		}
		if msg.DocumentMessage.Caption != nil {
			content.Caption, content.Mentions = c.resolveMentions(*msg.DocumentMessage.Caption, msg.DocumentMessage.ContextInfo)
		}
		return content
	}

	// Sticker message
	if msg.StickerMessage != nil {
		content := MessageContent{
			Type:     "sticker",
			MimeType: getString(msg.StickerMessage.Mimetype),
			Width:    msg.StickerMessage.GetWidth(),
			Height:   msg.StickerMessage.GetHeight(),
		}
		if msg.StickerMessage.IsAnimated != nil {
			content.IsAnimated = *msg.StickerMessage.IsAnimated
		}
		return content
	}

	// Location message
	if msg.LocationMessage != nil {
		content := MessageContent{
			Type: "location",
		}
		if msg.LocationMessage.DegreesLatitude != nil {
			lat := *msg.LocationMessage.DegreesLatitude
			content.Latitude = &lat
		}
		if msg.LocationMessage.DegreesLongitude != nil {
			lng := *msg.LocationMessage.DegreesLongitude
			content.Longitude = &lng
		}
		if msg.LocationMessage.Name != nil {
			content.LocationName = *msg.LocationMessage.Name
		}
		if msg.LocationMessage.Address != nil {
			content.Address = *msg.LocationMessage.Address
		}
		return content
	}

	// Contact message
	if msg.ContactMessage != nil {
		return MessageContent{
			Type:        "contact",
			DisplayName: getString(msg.ContactMessage.DisplayName),
			VCard:       getString(msg.ContactMessage.Vcard),
		}
	}

	// Reaction message
	if msg.ReactionMessage != nil {
		return MessageContent{
			Type:              "reaction",
			Emoji:             getString(msg.ReactionMessage.Text),
			TargetMessageID:   msg.ReactionMessage.Key.GetID(),
			SenderTimestampMS: msg.ReactionMessage.GetSenderTimestampMS(),
		}
	}

	// Protocol message (includes revoked/deleted messages, ephemeral settings, etc.)
	if msg.ProtocolMessage != nil {
		if msg.ProtocolMessage.Type != nil {
			switch *msg.ProtocolMessage.Type {
			case waE2E.ProtocolMessage_REVOKE:
				return MessageContent{Type: "revoked"}
			default:
				// Other protocol messages (ephemeral settings, history sync, etc.)
				// should be ignored - return special type that will be filtered out
				return MessageContent{Type: "protocol", RawType: msg.ProtocolMessage.Type.String()}
			}
		}
	}

	// Sender key distribution message (group encryption setup) - ignore
	if msg.SenderKeyDistributionMessage != nil {
		return MessageContent{Type: "protocol", RawType: "sender_key_distribution"}
	}

	// Message context info only (no actual content) - ignore
	if msg.MessageContextInfo != nil && msg.Conversation == nil && msg.ExtendedTextMessage == nil {
		return MessageContent{Type: "protocol", RawType: "context_info_only"}
	}

	// Poll creation message
	if msg.PollCreationMessage != nil {
		content := MessageContent{
			Type: "poll",
		}
		if msg.PollCreationMessage.Name != nil {
			content.Question = *msg.PollCreationMessage.Name
		}
		for _, opt := range msg.PollCreationMessage.Options {
			if opt.OptionName != nil {
				content.Options = append(content.Options, *opt.OptionName)
			}
		}
		return content
	}

	// Unknown message type
	return MessageContent{
		Type:    "unknown",
		RawType: fmt.Sprintf("%T", msg),
	}
}

func applyAlbumAssociation(msg *waE2E.Message, content *MessageContent) {
	association := msg.GetMessageContextInfo().GetMessageAssociation()
	if association.GetAssociationType() != waE2E.MessageAssociation_MEDIA_ALBUM {
		return
	}
	parentID := association.GetParentMessageKey().GetID()
	if parentID == "" {
		return
	}
	content.AlbumID = parentID
	index := association.GetMessageIndex()
	content.AlbumIndex = &index
}

// Helper functions

func getString(s *string) string {
	if s == nil {
		return ""
	}
	return *s
}

func getUint64(u *uint64) uint64 {
	if u == nil {
		return 0
	}
	return *u
}
