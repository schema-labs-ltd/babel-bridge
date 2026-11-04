package main

import (
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/types"
	"testing"
)

func TestMediaMessagesPreserveUploadAndReplyContext(t *testing.T) {
	jid, _ := types.ParseJID("one@g.us")
	upload := whatsmeow.UploadResponse{URL: "https://example.invalid/media", DirectPath: "/media", FileLength: 42, MediaKey: []byte{1, 2}, FileSHA256: []byte{3}, FileEncSHA256: []byte{4}}
	for _, kind := range []string{"video", "document"} {
		for _, reply := range []string{"", "captured-message"} {
			cmd := Command{MediaKind: kind, MimeType: "video/mp4", FileName: "Attachment.mp4", Caption: "Caption", ReplyTo: reply, ReplyToSender: "sam@s.whatsapp.net", ReplyToText: "Original"}
			msg := buildMediaMessage(cmd, jid, upload, nil)
			context := msg.GetVideoMessage().GetContextInfo()
			if kind == "document" {
				if msg.GetDocumentMessage().GetFileName() != cmd.FileName || msg.GetDocumentMessage().GetFileLength() != 42 {
					t.Fatal("document fields lost")
				}
				context = msg.GetDocumentMessage().GetContextInfo()
			} else if msg.GetVideoMessage().GetCaption() != "Caption" || msg.GetVideoMessage().GetFileLength() != 42 {
				t.Fatal("video fields lost")
			}
			if reply == "" {
				if context != nil {
					t.Fatal("latest message must send without a quote")
				}
			} else if context.GetStanzaID() != reply || context.GetParticipant() != cmd.ReplyToSender || context.GetRemoteJID() != jid.String() {
				t.Fatal("captured reply context lost")
			}
		}
	}
}

func TestStickersSendAsWebPStickerMessages(t *testing.T) {
	jid, _ := types.ParseJID("one@s.whatsapp.net")
	upload := whatsmeow.UploadResponse{URL: "https://example.invalid/media", DirectPath: "/media", FileLength: 42, MediaKey: []byte{1, 2}, FileSHA256: []byte{3}, FileEncSHA256: []byte{4}}
	still := []byte("RIFF\x00\x00\x00\x00WEBPVP8 \x00\x00\x00\x00")
	animated := []byte("RIFF\x00\x00\x00\x00WEBPVP8X\x0a\x00\x00\x00\x02\x00\x00\x00")
	if !isWebP(still) || isWebP([]byte("GIF89a not webp")) {
		t.Fatal("WebP detection is wrong")
	}
	msg := buildMediaMessage(Command{MediaKind: "sticker", MimeType: "image/webp"}, jid, upload, animated)
	sticker := msg.GetStickerMessage()
	if sticker == nil || sticker.GetMimetype() != "image/webp" || !sticker.GetIsAnimated() || sticker.GetFileLength() != 42 {
		t.Fatalf("sticker fields lost: %+v", sticker)
	}
	if buildMediaMessage(Command{MediaKind: "sticker"}, jid, upload, still).GetStickerMessage().GetIsAnimated() {
		t.Fatal("a still sticker was marked animated")
	}
}
