package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strings"
	"testing"

	"go.mau.fi/whatsmeow/types"
)

// Shared wire fixtures for the Rust <-> Go bridge protocol live in
// ../fixtures/bridge. The bridge owns events (Go writes, Rust reads) and the
// server owns commands (Rust writes, Go reads). Each side checks that it
// writes its fixtures byte for byte and can read the other side's.
//
// After an intended protocol change, regenerate the event fixtures with
// UPDATE_FIXTURES=1 go test ./... and the command fixtures with
// UPDATE_FIXTURES=1 cargo test bridge::protocol, then update the reader.

const fixtureRoot = "../fixtures/bridge"

func fixtureEvents() map[string]interface{} {
	count := 3
	albumIndex := int32(1)
	duration := uint32(7)
	unread := uint32(2)
	lat, lng := 51.5007, -0.1246
	sender := Contact{JID: "447700900001@s.whatsapp.net", Phone: "447700900001", Name: "Ana"}
	private := Chat{Type: "private", JID: "447700900001@s.whatsapp.net", Name: "Ana"}
	group := Chat{Type: "group", JID: "120363000000000001@g.us", Name: "Family", ParticipantCount: &count}
	message := func(id string, chat Chat, content MessageContent) Message {
		return Message{ID: id, Timestamp: 1767225600, From: sender, Chat: chat, Content: content, PushName: "Ana"}
	}

	text := message("3EB0TEXT", private, MessageContent{Type: "text", Body: "Olá @447700900002", Mentions: []Mention{{JID: "447700900002@s.whatsapp.net", Phone: "447700900002", Name: "Ben"}}})
	text.ReplyContext = &ReplyContext{MessageID: "3EB0QUOTED", SenderName: "Ben", Text: "Are you coming?"}
	history := message("3EB0HISTORY", private, MessageContent{Type: "text", Body: "Old message"})
	history.IsHistory = true
	history.UnreadCount = &unread
	image := message("3EB0IMAGE", group, MessageContent{Type: "image", Caption: "Beach", MimeType: "image/jpeg", FileSize: 2048, FileHash: "aGFzaA==", Width: 1280, Height: 960, AlbumID: "3EB0ALBUM", AlbumIndex: &albumIndex})
	// Ordinary audio: Go omits the false is_voice_note flag. Carries
	// media_data so the attachment payload is pinned too.
	audio := message("3EB0AUDIO", private, MessageContent{Type: "audio", MimeType: "audio/mpeg", FileSize: 5, DurationSeconds: &duration, MediaData: "YXVkaW8="})
	voice := message("3EB0VOICE", private, MessageContent{Type: "audio", MimeType: "audio/ogg; codecs=opus", FileSize: 4096, DurationSeconds: &duration, IsVoiceNote: true})
	location := message("3EB0LOCATION", private, MessageContent{Type: "location", Latitude: &lat, Longitude: &lng, LocationName: "Big Ben", Address: "London"})
	reaction := message("3EB0REACTION", group, MessageContent{Type: "reaction", Emoji: "👍", TargetMessageID: "3EB0IMAGE", SenderTimestampMS: 1767225601000})
	edited := message("3EB0IMAGE", group, MessageContent{Type: "image", Caption: "Beach at sunset"})

	return map[string]interface{}{
		"qr":                   NewQREvent("2@ABC123"),
		"connected":            NewConnectedEvent("447700900000", "Me", "android"),
		"connection_state":     NewConnectionStateEvent("reconnecting"),
		"message_text":         NewMessageEvent(text),
		"message_history":      NewMessageEvent(history),
		"message_image":        NewMessageEvent(image),
		"message_voice_note":   NewMessageEvent(voice),
		"message_audio":        NewMessageEvent(audio),
		"message_location":     NewMessageEvent(location),
		"message_reaction":     NewMessageEvent(reaction),
		"message_edit_caption": NewMessageEditEvent(edited, 1767225700000),
		"send_result_success":  NewSendResultEvent(4, true, "3EB0SENT", 1767225600, ""),
		"send_result_failure":  NewSendResultEvent(5, false, "", 0, "not connected"),
		"send_result_album":    NewAlbumSendResultEvent(6, "3EB0ALBUM1", 1767225600, []string{"3EB0ALBUM1", "3EB0ALBUM2"}, []int64{1767225600, 1767225601}),
		"send_progress":        NewSendProgressEvent(6, "progress-1", "uploading", 1, 2),
		"profile_picture":      NewProfilePictureEvent(7, "447700900001@s.whatsapp.net", "https://pps.whatsapp.net/v/t61/example.jpg", "1700000000", ""),
		"chat_presence":        NewChatPresenceEvent("120363000000000001@g.us", "447700900001@s.whatsapp.net", "typing"),
		"receipt":              NewReceiptEvent([]types.MessageID{"3EB0SENT"}, types.ReceiptTypeRead),
		"read_receipt_result":  ReadReceiptResultEvent{Type: "read_receipt_result", MessageID: "3EB0TEXT", Success: false, Error: "not connected"},
		"mark_as_read":         NewMarkAsReadEvent("447700900001@s.whatsapp.net"),
		"error":                NewErrorEvent("connection_failed", "could not reach WhatsApp"),
		"log":                  NewLogEvent("info", "Connected"),
		"logged_out":           NewLoggedOutEvent("device removed"),
	}
}

func fixtureNames(t *testing.T, kind string) []string {
	t.Helper()
	paths, err := filepath.Glob(filepath.Join(fixtureRoot, kind, "*.json"))
	if err != nil {
		t.Fatal(err)
	}
	names := make([]string, 0, len(paths))
	for _, path := range paths {
		names = append(names, strings.TrimSuffix(filepath.Base(path), ".json"))
	}
	sort.Strings(names)
	return names
}

func TestBridgeEventsMatchSharedFixtures(t *testing.T) {
	events := fixtureEvents()
	update := os.Getenv("UPDATE_FIXTURES") == "1"
	if update {
		if err := os.MkdirAll(filepath.Join(fixtureRoot, "events"), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	for name, event := range events {
		data, err := json.MarshalIndent(event, "", "  ")
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		data = append(data, '\n')
		path := filepath.Join(fixtureRoot, "events", name+".json")
		if update {
			if err := os.WriteFile(path, data, 0o644); err != nil {
				t.Fatal(err)
			}
			continue
		}
		want, err := os.ReadFile(path)
		if err != nil {
			t.Fatalf("%s: %v (regenerate with UPDATE_FIXTURES=1 go test ./...)", name, err)
		}
		if string(want) != string(data) {
			t.Errorf("%s: bridge output changed; if intended, regenerate with UPDATE_FIXTURES=1 go test ./... and check the Rust reader\nwant:\n%s\ngot:\n%s", name, want, data)
		}
	}
	if !update {
		var want []string
		for name := range events {
			want = append(want, name)
		}
		sort.Strings(want)
		if got := fixtureNames(t, "events"); !reflect.DeepEqual(got, want) {
			t.Errorf("event fixtures on disk %v do not match fixtureEvents %v", got, want)
		}
	}
}

func readCommandFixture(t *testing.T, name string) Command {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(fixtureRoot, "commands", name+".json"))
	if err != nil {
		t.Fatalf("%s: %v (the server writes these: UPDATE_FIXTURES=1 cargo test bridge::protocol)", name, err)
	}
	var cmd Command
	if err := json.Unmarshal(data, &cmd); err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	return cmd
}

func TestBridgeReadsServerCommandFixtures(t *testing.T) {
	want := map[string]Command{
		"send":       {Type: "send", RequestID: 1, To: "447700900001@s.whatsapp.net", Text: "Olá", ReplyTo: "3EB0QUOTED", ReplyToSender: "447700900002@s.whatsapp.net", ReplyToText: "Are you coming?"},
		"send_audio": {Type: "send_audio", RequestID: 2, To: "447700900001@s.whatsapp.net", MediaData: "T2dnUw==", DurationSeconds: 7},
		"send_media": {Type: "send_media", RequestID: 3, To: "447700900001@s.whatsapp.net", MediaData: "JVBERi0=", MimeType: "application/pdf", MediaKind: "document", FileName: "menu.pdf", Caption: "Menu"},
		"send_image": {Type: "send_image", RequestID: 4, To: "120363000000000001@g.us", MediaData: "/9j/4AAQ", MimeType: "image/jpeg", Caption: "Beach"},
		"send_images": {Type: "send_images", RequestID: 6, ProgressID: "progress-1", To: "120363000000000001@g.us", Caption: "Holiday", Images: []ImagePayload{
			{MediaData: "/9j/4AAQ", MimeType: "image/jpeg"},
			{MediaData: "iVBORw0K", MimeType: "image/png"},
		}},
		"send_reaction":       {Type: "send_reaction", RequestID: 8, To: "120363000000000001@g.us", MessageID: "3EB0IMAGE", SenderJID: "447700900001@s.whatsapp.net", Emoji: "👍"},
		"mark_read":           {Type: "mark_read", To: "120363000000000001@g.us", MessageID: "3EB0IMAGE", Timestamp: 1767225700, SenderJID: "447700900001@s.whatsapp.net"},
		"get_profile_picture": {Type: "get_profile_picture", RequestID: 7, To: "447700900001@s.whatsapp.net"},
		"disconnect":          {Type: "disconnect"},
		"logout":              {Type: "logout"},
	}
	var names []string
	for name, expected := range want {
		names = append(names, name)
		if got := readCommandFixture(t, name); !reflect.DeepEqual(got, expected) {
			t.Errorf("%s: decoded %+v, want %+v", name, got, expected)
		}
	}
	sort.Strings(names)
	if got := fixtureNames(t, "commands"); !reflect.DeepEqual(got, names) {
		t.Errorf("command fixtures on disk %v do not match the commands checked here %v", got, names)
	}
}
