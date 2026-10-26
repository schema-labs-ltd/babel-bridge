package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"sync/atomic"
	"time"

	"go.mau.fi/whatsmeow/types"
)

// Fake mode stands in for WhatsApp so the server and apps can run without a
// phone or a linked account: WA_FAKE_BRIDGE=true connects straight away with a
// few canned chats, and WA_FAKE_BRIDGE=qr shows a pairing QR code first.
// Sends are acknowledged, then reported delivered and read. Nothing is sent
// anywhere and the session database is not touched.

const fakeSelfJID = "447700900000@s.whatsapp.net"

var fakeMessageCounter atomic.Int64

func fakeContact(phone, name string) Contact {
	return Contact{JID: phone + "@s.whatsapp.net", Phone: phone, Name: name}
}

func fakeTextMessage(id string, from Contact, chat Chat, body string, at time.Time, fromMe bool) Message {
	return Message{
		ID:        id,
		Timestamp: at.Unix(),
		From:      from,
		Chat:      chat,
		Content:   MessageContent{Type: "text", Body: body},
		IsFromMe:  fromMe,
		PushName:  from.Name,
	}
}

// fakeHistory returns the canned chats, oldest message first.
func fakeHistory(now time.Time) []Message {
	me := Contact{JID: fakeSelfJID, Phone: "447700900000", Name: "Me"}
	lucia := fakeContact("34600000001", "Lucía")
	joao := fakeContact("351910000002", "João")
	sam := fakeContact("447700900003", "Sam")
	luciaChat := Chat{Type: "private", JID: lucia.JID, Name: lucia.Name}
	samChat := Chat{Type: "private", JID: sam.JID, Name: sam.Name}
	members := 3
	family := Chat{Type: "group", JID: "120363000000000001@g.us", Name: "Family", ParticipantCount: &members}

	at := func(minutesAgo int) time.Time { return now.Add(-time.Duration(minutesAgo) * time.Minute) }
	unread := uint32(1)
	messages := []Message{
		fakeTextMessage("FAKE-L1", lucia, luciaChat, "¡Hola! ¿Qué tal el viaje?", at(180), false),
		fakeTextMessage("FAKE-L2", me, luciaChat, "Muy bien, gracias. Llegamos anoche.", at(175), true),
		fakeTextMessage("FAKE-L3", lucia, luciaChat, "¿Quedamos el sábado para cenar?", at(30), false),
		fakeTextMessage("FAKE-F1", joao, family, "Bom dia a todos!", at(120), false),
		fakeTextMessage("FAKE-F2", sam, family, "Morning! Photos from yesterday coming soon.", at(118), false),
		fakeTextMessage("FAKE-F3", joao, family, "Alguém sabe a que horas é o almoço?", at(60), false),
		fakeTextMessage("FAKE-S1", sam, samChat, "Can you send me the address?", at(90), false),
		fakeTextMessage("FAKE-S2", me, samChat, "Sure, sending it now.", at(85), true),
	}
	latitude, longitude := 51.5007, -0.1246
	location := fakeTextMessage("FAKE-S3", me, samChat, "", at(84), true)
	location.Content = MessageContent{Type: "location", Latitude: &latitude, Longitude: &longitude, LocationName: "Big Ben", Address: "London SW1A 0AA"}
	messages = append(messages, location)
	for i := range messages {
		messages[i].IsHistory = true
	}
	// The first message of a history-synced chat carries WhatsApp's unread count.
	messages[0].UnreadCount = &unread
	return messages
}

func fakeLiveMessage(now time.Time) Message {
	lucia := fakeContact("34600000001", "Lucía")
	return fakeTextMessage(
		"FAKE-L4",
		lucia,
		Chat{Type: "private", JID: lucia.JID, Name: lucia.Name},
		"Te espero en la estación a las ocho.",
		now,
		false,
	)
}

func nextFakeMessageID() string {
	return fmt.Sprintf("FAKE-OUT-%d", fakeMessageCounter.Add(1))
}

func runFakeBridge(ctx context.Context, cancel context.CancelFunc, mode string) {
	SendEvent(NewLogEvent("info", "Fake bridge: no WhatsApp connection is made"))
	SendEvent(NewConnectionStateEvent("connecting"))
	if mode == "qr" {
		SendEvent(NewQREvent("2@FAKE-QR-CODE,fake-bridge,not-a-real-pairing-code"))
		select {
		case <-ctx.Done():
			return
		case <-time.After(5 * time.Second):
		}
	}
	SendEvent(NewConnectedEvent("447700900000", "Me", "fake"))
	SendEvent(NewConnectionStateEvent("connected"))
	now := time.Now()
	for _, msg := range fakeHistory(now) {
		SendEvent(NewMessageEvent(msg))
	}

	go func() {
		select {
		case <-ctx.Done():
		case <-time.After(10 * time.Second):
			SendEvent(NewMessageEvent(fakeLiveMessage(time.Now())))
			SendEvent(NewChatPresenceEvent("34600000001@s.whatsapp.net", "34600000001@s.whatsapp.net", "paused"))
		}
	}()

	go readFakeCommands(ctx, cancel)
	<-ctx.Done()
	SendEvent(NewLogEvent("info", "Disconnecting"))
}

func readFakeCommands(ctx context.Context, cancel context.CancelFunc) {
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 64*1024), maxCommandBytes)
	for scanner.Scan() {
		line := scanner.Bytes()
		if len(line) == 0 {
			continue
		}
		var cmd Command
		if err := json.Unmarshal(line, &cmd); err != nil {
			SendEvent(NewLogEvent("warn", fmt.Sprintf("failed to parse command: %v", err)))
			continue
		}
		handleFakeCommand(ctx, cancel, cmd)
	}
	SendEvent(NewLogEvent("info", "stdin closed, shutting down"))
	cancel()
}

// reportFakeReceipts marks a sent message delivered, then read, like a phone
// that is online and has the chat open.
func reportFakeReceipts(ctx context.Context, ids []string) {
	messageIDs := make([]types.MessageID, len(ids))
	for i, id := range ids {
		messageIDs[i] = types.MessageID(id)
	}
	go func() {
		for _, step := range []struct {
			delay   time.Duration
			receipt types.ReceiptType
		}{{time.Second, types.ReceiptTypeDelivered}, {3 * time.Second, types.ReceiptTypeRead}} {
			select {
			case <-ctx.Done():
				return
			case <-time.After(step.delay):
				SendEvent(NewReceiptEvent(messageIDs, step.receipt))
			}
		}
	}()
}

func handleFakeCommand(ctx context.Context, cancel context.CancelFunc, cmd Command) {
	switch cmd.Type {
	case "disconnect":
		cancel()
	case "logout":
		SendEvent(NewLoggedOutEvent("fake bridge logout"))
		cancel()
	case "send", "send_audio", "send_media", "send_image", "send_reaction":
		id := nextFakeMessageID()
		SendEvent(NewSendResultEvent(cmd.RequestID, true, id, Now(), ""))
		if cmd.Type != "send_reaction" {
			reportFakeReceipts(ctx, []string{id})
		}
	case "send_images":
		ids := make([]string, len(cmd.Images))
		timestamps := make([]int64, len(cmd.Images))
		for i := range cmd.Images {
			ids[i] = nextFakeMessageID()
			timestamps[i] = Now()
			if cmd.ProgressID != "" {
				SendEvent(NewSendProgressEvent(cmd.RequestID, cmd.ProgressID, "uploading", i+1, len(cmd.Images)))
			}
		}
		if len(ids) == 0 {
			SendEvent(NewSendResultEvent(cmd.RequestID, false, "", 0, "no images"))
			return
		}
		SendEvent(NewAlbumSendResultEvent(cmd.RequestID, ids[0], timestamps[0], ids, timestamps))
		reportFakeReceipts(ctx, ids)
	case "mark_read":
		SendEvent(ReadReceiptResultEvent{Type: "read_receipt_result", MessageID: cmd.MessageID, Success: true})
	case "get_profile_picture":
		SendEvent(NewProfilePictureEvent(cmd.RequestID, cmd.To, "", "", ""))
	default:
		SendEvent(NewLogEvent("warn", fmt.Sprintf("Fake bridge ignored command: %s", cmd.Type)))
	}
}
