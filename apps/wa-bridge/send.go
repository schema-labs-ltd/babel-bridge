package main

import (
	"context"
	"encoding/base64"
	"fmt"
	"strings"
	"time"

	"go.mau.fi/util/random"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waCommon"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

// GetStore returns the underlying store for direct access if needed
func (c *Client) GetStore() *store.Device {
	return c.client.Store
}

func buildQuotedMessage(replyToText string) *waE2E.Message {
	if replyToText == "" {
		return nil
	}

	textCopy := replyToText
	return &waE2E.Message{
		ExtendedTextMessage: &waE2E.ExtendedTextMessage{
			Text: &textCopy,
		},
	}
}

func buildReplyContext(jid types.JID, replyToID string, replyToSender string, replyToText string) *waE2E.ContextInfo {
	if replyToID == "" {
		return nil
	}

	remote := jid.String()
	contextInfo := &waE2E.ContextInfo{
		StanzaID:  &replyToID,
		RemoteJID: &remote,
	}
	if replyToSender != "" {
		participant := replyToSender
		if !strings.Contains(replyToSender, "@") {
			participant += "@s.whatsapp.net"
		}
		contextInfo.Participant = &participant
	}
	contextInfo.QuotedMessage = buildQuotedMessage(replyToText)
	return contextInfo
}

func newMessageSecret() []byte {
	return random.Bytes(32)
}

func buildAlbumMessage(jidStr string, imageCount int, replyToID string, replyToSender string, replyToText string) (*waE2E.Message, *waCommon.MessageKey, error) {
	jid, err := types.ParseJID(jidStr)
	if err != nil {
		return nil, nil, fmt.Errorf("invalid JID: %w", err)
	}
	parentID := string(whatsmeow.GenerateMessageID())
	parentKey := &waCommon.MessageKey{
		RemoteJID: proto.String(jid.String()),
		FromMe:    proto.Bool(true),
		ID:        proto.String(parentID),
	}
	message := &waE2E.Message{
		AlbumMessage: &waE2E.AlbumMessage{
			ExpectedImageCount: proto.Uint32(uint32(imageCount)),
			ExpectedVideoCount: proto.Uint32(0),
			ContextInfo:        buildReplyContext(jid, replyToID, replyToSender, replyToText),
		},
		MessageContextInfo: &waE2E.MessageContextInfo{MessageSecret: newMessageSecret()},
	}
	return message, parentKey, nil
}

func buildAlbumChildMessage(image *waE2E.ImageMessage, parentKey *waCommon.MessageKey, index int) *waE2E.Message {
	return &waE2E.Message{
		ImageMessage: image,
		MessageContextInfo: &waE2E.MessageContextInfo{
			MessageSecret: newMessageSecret(),
			MessageAssociation: &waE2E.MessageAssociation{
				AssociationType:  waE2E.MessageAssociation_MEDIA_ALBUM.Enum(),
				ParentMessageKey: parentKey,
				MessageIndex:     proto.Int32(int32(index)),
			},
		},
	}
}

// SendTextMessage sends a text message to the specified JID.
// If replyToID is provided, the message will be a reply to that message.
func (c *Client) SendTextMessage(ctx context.Context, jidStr string, text string, replyToID string, replyToSender string, replyToText string) (string, int64, error) {
	// Parse the JID
	jid, err := types.ParseJID(jidStr)
	if err != nil {
		return "", 0, fmt.Errorf("invalid JID: %w", err)
	}

	var msg *waE2E.Message

	// Check if this is a reply
	if replyToID != "" {
		// Create context info for the reply
		contextInfo := &waE2E.ContextInfo{
			StanzaID: &replyToID,
			RemoteJID: func() *string {
				remote := jid.String()
				return &remote
			}(),
		}

		// Set the participant (sender of the quoted message)
		// Convert phone number to full JID if needed
		if replyToSender != "" {
			participant := replyToSender
			// If it's just a phone number, convert to JID format
			if !strings.Contains(replyToSender, "@") {
				participant = replyToSender + "@s.whatsapp.net"
			}
			contextInfo.Participant = &participant
		}

		if quoted := buildQuotedMessage(replyToText); quoted != nil {
			contextInfo.QuotedMessage = quoted
		}

		// Use ExtendedTextMessage for replies (required for ContextInfo)
		msg = &waE2E.Message{
			ExtendedTextMessage: &waE2E.ExtendedTextMessage{
				Text:        &text,
				ContextInfo: contextInfo,
			},
		}
	} else {
		// Simple message without reply
		msg = &waE2E.Message{
			Conversation: &text,
		}
	}

	// Send the message
	resp, err := c.client.SendMessage(ctx, jid, msg)
	if err != nil {
		return "", 0, fmt.Errorf("failed to send message: %w", err)
	}

	return resp.ID, resp.Timestamp.Unix(), nil
}

// SendAudioMessage sends a normalized Ogg/Opus recording as a WhatsApp voice note.
func (c *Client) SendAudioMessage(ctx context.Context, to, encoded string, seconds uint32, replyID, replySender, replyText string) (string, int64, error) {
	jid, err := types.ParseJID(to)
	if err != nil || to == "" {
		return "", 0, fmt.Errorf("invalid recipient")
	}
	data, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil || len(data) < 32 || len(data) > 16*1024*1024 || string(data[:4]) != "OggS" || seconds == 0 || seconds > 240 {
		return "", 0, fmt.Errorf("invalid voice note")
	}
	upload, err := c.client.Upload(ctx, data, whatsmeow.MediaAudio)
	if err != nil {
		return "", 0, fmt.Errorf("voice upload failed: %w", err)
	}
	msg := &waE2E.AudioMessage{
		Mimetype: proto.String("audio/ogg; codecs=opus"), PTT: proto.Bool(true), Seconds: &seconds,
		URL: &upload.URL, DirectPath: &upload.DirectPath, MediaKey: upload.MediaKey,
		FileEncSHA256: upload.FileEncSHA256, FileSHA256: upload.FileSHA256, FileLength: &upload.FileLength,
		ContextInfo: buildReplyContext(jid, replyID, replySender, replyText),
	}
	resp, err := c.client.SendMessage(ctx, jid, &waE2E.Message{AudioMessage: msg})
	if err != nil {
		return "", 0, err
	}
	return resp.ID, resp.Timestamp.Unix(), nil
}

// SendImageMessage sends an image message to the specified JID.
// If replyToID is provided, the message will be a reply to that message.
func (c *Client) SendImageMessage(ctx context.Context, jidStr string, mediaDataB64 string, mimeType string, caption string, replyToID string, replyToSender string, replyToText string) (string, int64, error) {
	// Parse the JID
	jid, err := types.ParseJID(jidStr)
	if err != nil {
		return "", 0, fmt.Errorf("invalid JID: %w", err)
	}

	// Decode base64 image data
	imageData, err := base64.StdEncoding.DecodeString(mediaDataB64)
	if err != nil {
		return "", 0, fmt.Errorf("failed to decode image data: %w", err)
	}

	// Default mime type if not provided
	if mimeType == "" {
		mimeType = "image/jpeg"
	}

	// Upload the image to WhatsApp
	uploadResp, err := c.client.Upload(ctx, imageData, whatsmeow.MediaImage)
	if err != nil {
		return "", 0, fmt.Errorf("failed to upload image: %w", err)
	}

	// Create the image message
	imageMsg := &waE2E.ImageMessage{
		Mimetype:      &mimeType,
		URL:           &uploadResp.URL,
		DirectPath:    &uploadResp.DirectPath,
		MediaKey:      uploadResp.MediaKey,
		FileEncSHA256: uploadResp.FileEncSHA256,
		FileSHA256:    uploadResp.FileSHA256,
		FileLength:    &uploadResp.FileLength,
	}

	if caption != "" {
		imageMsg.Caption = &caption
	}

	// Add reply context if provided
	if replyToID != "" {
		contextInfo := &waE2E.ContextInfo{
			StanzaID: &replyToID,
			RemoteJID: func() *string {
				remote := jid.String()
				return &remote
			}(),
		}
		// Convert phone number to full JID if needed
		if replyToSender != "" {
			participant := replyToSender
			if !strings.Contains(replyToSender, "@") {
				participant = replyToSender + "@s.whatsapp.net"
			}
			contextInfo.Participant = &participant
		}
		if quoted := buildQuotedMessage(replyToText); quoted != nil {
			contextInfo.QuotedMessage = quoted
		}
		imageMsg.ContextInfo = contextInfo
	}

	msg := &waE2E.Message{
		ImageMessage: imageMsg,
	}

	// Send the message
	resp, err := c.client.SendMessage(ctx, jid, msg)
	if err != nil {
		return "", 0, fmt.Errorf("failed to send image: %w", err)
	}

	return resp.ID, resp.Timestamp.Unix(), nil
}

// SendImageAlbum sends multiple images as one grouped WhatsApp album.
func (c *Client) SendImageAlbum(ctx context.Context, jidStr string, images []ImagePayload, caption string, replyToID string, replyToSender string, replyToText string, progress func(string, int, int)) (string, int64, []string, []int64, error) {
	if len(images) < 2 {
		return "", 0, nil, nil, fmt.Errorf("an album requires at least two images")
	}
	jID, err := types.ParseJID(jidStr)
	if err != nil {
		return "", 0, nil, nil, fmt.Errorf("invalid JID: %w", err)
	}

	progress("uploading", 0, len(images))
	imageIndexes := make([]int, len(images))
	for index := range images {
		imageIndexes[index] = index
	}
	children, err := mapAlbumUploads(ctx, imageIndexes, albumUploadConcurrency, func(uploadCtx context.Context, index int) (*waE2E.ImageMessage, error) {
		image := images[index]
		data, err := base64.StdEncoding.DecodeString(image.MediaData)
		if err != nil {
			return nil, fmt.Errorf("failed to decode image %d: %w", index+1, err)
		}
		upload, err := c.client.Upload(uploadCtx, data, whatsmeow.MediaImage)
		if err != nil {
			return nil, fmt.Errorf("failed to upload image %d: %w", index+1, err)
		}
		mimeType := image.MimeType
		if mimeType == "" {
			mimeType = "image/jpeg"
		}
		return &waE2E.ImageMessage{
			Mimetype:      &mimeType,
			URL:           &upload.URL,
			DirectPath:    &upload.DirectPath,
			MediaKey:      upload.MediaKey,
			FileEncSHA256: upload.FileEncSHA256,
			FileSHA256:    upload.FileSHA256,
			FileLength:    &upload.FileLength,
		}, nil
	}, func(completed int) {
		progress("uploading", completed, len(images))
	})
	if err != nil {
		return "", 0, nil, nil, err
	}
	if caption != "" {
		children[0].Caption = &caption
	}

	parent, parentKey, err := buildAlbumMessage(jidStr, len(children), replyToID, replyToSender, replyToText)
	if err != nil {
		return "", 0, nil, nil, err
	}
	parentResponse, err := c.client.SendMessage(ctx, jID, parent, whatsmeow.SendRequestExtra{ID: types.MessageID(parentKey.GetID())})
	if err != nil {
		return "", 0, nil, nil, fmt.Errorf("failed to send album: %w", err)
	}

	messageIDs := make([]string, 0, len(children))
	timestamps := make([]int64, 0, len(children))
	progress("sending", 0, len(children))
	for index, child := range children {
		response, err := c.client.SendMessage(ctx, jID, buildAlbumChildMessage(child, parentKey, index))
		if err != nil {
			return "", 0, nil, nil, fmt.Errorf("failed to send album image %d: %w", index+1, err)
		}
		messageIDs = append(messageIDs, response.ID)
		timestamps = append(timestamps, response.Timestamp.Unix())
		progress("sending", index+1, len(children))
	}

	return parentResponse.ID, parentResponse.Timestamp.Unix(), messageIDs, timestamps, nil
}

// SendReaction sends a reaction to a message
func (c *Client) SendReaction(ctx context.Context, chatJIDStr string, targetMessageID string, senderJIDStr string, emoji string) (string, int64, error) {
	// Parse the chat JID
	chatJID, err := types.ParseJID(chatJIDStr)
	if err != nil {
		return "", 0, fmt.Errorf("invalid chat JID: %w", err)
	}

	// Parse the sender JID (who sent the message we're reacting to)
	var senderJID types.JID
	if senderJIDStr != "" {
		senderJID, err = types.ParseJID(senderJIDStr)
		if err != nil {
			return "", 0, fmt.Errorf("invalid sender JID: %w", err)
		}
	} else {
		// If no sender JID provided, assume it's the chat JID (for private chats)
		senderJID = chatJID
	}

	// Build and send the reaction message
	reactionMsg := c.client.BuildReaction(chatJID, senderJID, targetMessageID, emoji)

	resp, err := c.client.SendMessage(ctx, chatJID, reactionMsg)
	if err != nil {
		return "", 0, fmt.Errorf("failed to send reaction: %w", err)
	}

	// whatsmeow does not deliver our own send as an incoming Message event.
	// Publish the confirmed payload before send_result so every consumer sees it
	// in storage and the live stream before the HTTP/MCP send is acknowledged.
	if c.client.Store.ID != nil {
		c.handleMessage(confirmedReactionEvent(chatJID, *c.client.Store.ID, resp.ID, resp.Timestamp, reactionMsg))
	}
	return resp.ID, resp.Timestamp.Unix(), nil
}

func confirmedReactionEvent(chat, sender types.JID, id string, sentAt time.Time, payload *waE2E.Message) *events.Message {
	return &events.Message{
		Info: types.MessageInfo{
			MessageSource: types.MessageSource{Chat: normalizeAddress(chat, types.JID{}), Sender: sender.ToNonAD(), IsFromMe: true, IsGroup: chat.Server == types.GroupServer},
			ID:            id, Timestamp: sentAt,
		},
		Message: payload,
	}
}

// MarkRead marks a message as read in WhatsApp.
func (c *Client) MarkRead(ctx context.Context, chatJIDStr string, messageID string, timestamp int64, senderJIDStr string) error {
	chatJID, senderJID, readAt, err := readReceiptTarget(chatJIDStr, senderJIDStr, timestamp)
	if err != nil {
		return err
	}
	if err := c.client.MarkRead(ctx, []types.MessageID{types.MessageID(messageID)}, readAt, chatJID, senderJID); err != nil {
		return fmt.Errorf("failed to mark read: %w", err)
	}
	return nil
}

func readReceiptTarget(chat, sender string, timestamp int64) (types.JID, types.JID, time.Time, error) {
	chatJID, err := types.ParseJID(chat)
	if err != nil || chatJID.User == "" || timestamp <= 0 {
		return types.JID{}, types.JID{}, time.Time{}, fmt.Errorf("invalid read receipt chat or view time")
	}
	var senderJID types.JID
	if sender != "" {
		senderJID, err = types.ParseJID(sender)
		if err != nil || senderJID.User == "" {
			return types.JID{}, types.JID{}, time.Time{}, fmt.Errorf("invalid read receipt sender")
		}
		senderJID = senderJID.ToNonAD()
	}
	if chatJID.Server == types.GroupServer && (senderJID.IsEmpty() || (senderJID.Server != types.DefaultUserServer && senderJID.Server != types.HiddenUserServer)) {
		return types.JID{}, types.JID{}, time.Time{}, fmt.Errorf("group read receipts require the message sender")
	}
	return chatJID.ToNonAD(), senderJID, time.Unix(timestamp, 0), nil
}

// GetProfilePicture fetches the profile picture URL for a JID
func (c *Client) GetProfilePicture(ctx context.Context, jidStr string) (string, string, error) {
	// Parse the JID
	jid, err := types.ParseJID(jidStr)
	if err != nil {
		return "", "", fmt.Errorf("invalid JID: %w", err)
	}

	// Get profile picture info
	params := &whatsmeow.GetProfilePictureParams{
		Preview: false, // Get full size image
	}
	pic, err := c.client.GetProfilePictureInfo(ctx, jid, params)
	if err != nil {
		return "", "", fmt.Errorf("failed to get profile picture: %w", err)
	}

	if pic == nil {
		return "", "", nil // No profile picture set
	}

	return pic.URL, pic.ID, nil
}

// SendMediaMessage sends a video or file using the same confirmed-send protocol as photos.
func (c *Client) SendMediaMessage(ctx context.Context, cmd Command) (string, int64, error) {
	jid, err := types.ParseJID(cmd.To)
	if err != nil || cmd.To == "" {
		return "", 0, fmt.Errorf("invalid recipient")
	}
	data, err := base64.StdEncoding.DecodeString(cmd.MediaData)
	if err != nil || len(data) == 0 || len(data) > 64*1024*1024 {
		return "", 0, fmt.Errorf("invalid attachment data or size")
	}
	mediaType := whatsmeow.MediaDocument
	switch cmd.MediaKind {
	case "video":
		if cmd.MimeType != "video/mp4" || len(data) < 8 || string(data[4:8]) != "ftyp" {
			return "", 0, fmt.Errorf("invalid MP4 video")
		}
		mediaType = whatsmeow.MediaVideo
	case "document":
		if cmd.FileName == "" || strings.ContainsAny(cmd.FileName, "/\\\r\n") {
			return "", 0, fmt.Errorf("invalid file name")
		}
	default:
		return "", 0, fmt.Errorf("unsupported attachment type")
	}
	uploaded, err := c.client.Upload(ctx, data, mediaType)
	if err != nil {
		return "", 0, fmt.Errorf("attachment upload failed: %w", err)
	}
	msg := buildMediaMessage(cmd, jid, uploaded)
	resp, err := c.client.SendMessage(ctx, jid, msg)
	if err != nil {
		return "", 0, fmt.Errorf("attachment send failed: %w", err)
	}
	return resp.ID, resp.Timestamp.Unix(), nil
}

func buildMediaMessage(cmd Command, jid types.JID, uploaded whatsmeow.UploadResponse) *waE2E.Message {
	context := buildReplyContext(jid, cmd.ReplyTo, cmd.ReplyToSender, cmd.ReplyToText)
	if cmd.MediaKind == "video" {
		return &waE2E.Message{VideoMessage: &waE2E.VideoMessage{
			Mimetype: proto.String(cmd.MimeType), Caption: proto.String(cmd.Caption),
			URL: &uploaded.URL, DirectPath: &uploaded.DirectPath, MediaKey: uploaded.MediaKey,
			FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
			FileLength: &uploaded.FileLength, ContextInfo: context,
		}}
	}
	return &waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{
		Mimetype: proto.String(cmd.MimeType), FileName: proto.String(cmd.FileName), Caption: proto.String(cmd.Caption),
		URL: &uploaded.URL, DirectPath: &uploaded.DirectPath, MediaKey: uploaded.MediaKey,
		FileEncSHA256: uploaded.FileEncSHA256, FileSHA256: uploaded.FileSHA256,
		FileLength: &uploaded.FileLength, ContextInfo: context,
	}}
}
