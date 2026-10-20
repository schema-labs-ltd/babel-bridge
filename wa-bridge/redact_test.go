package main

import (
	"bytes"
	"strings"
	"testing"

	"github.com/rs/zerolog"
	waLog "go.mau.fi/whatsmeow/util/log"
)

const secretBody = "zz-unique-body-449d"
const secretPhone = "15550001111"

func TestRedactLogRemovesNumbersChatsAndMessagePayloads(t *testing.T) {
	input := "Decrypting message from " + secretPhone + "@s.whatsapp.net <message><conversation>" + secretBody + "</conversation></message> size: 10485760 bytes"
	got := redactLog(input)
	if strings.Contains(got, secretPhone) || strings.Contains(got, secretBody) || strings.Contains(got, "@s.whatsapp.net") {
		t.Fatalf("log leaked private data: %s", got)
	}
	if !strings.Contains(got, "Decrypting message") || !strings.Contains(got, "10485760 bytes") {
		t.Fatalf("log lost metadata: %s", got)
	}
}

func TestBridgeLoggerOmitsIncomingPhoneAndPayload(t *testing.T) {
	var buf bytes.Buffer
	logger := testBridgeLogger(true, &buf)
	logger.Warnf("Decrypting message from %s", secretPhone+"@s.whatsapp.net")
	logger.Warnf("Node handling took 6s for <message from=%q><conversation>%s</conversation></message>", secretPhone+"@s.whatsapp.net", secretBody)
	logger.Debugf("Event received: %s", "*events.Message")
	got := buf.String()
	if strings.Contains(got, secretPhone) || strings.Contains(got, secretBody) {
		t.Fatalf("stderr leaked private data: %s", got)
	}
	if !strings.Contains(got, "Decrypting message") || !strings.Contains(got, "Node handling") {
		t.Fatalf("stderr lost metadata: %s", got)
	}
	if !strings.Contains(got, "Event received") {
		t.Fatalf("verbose metadata missing: %s", got)
	}
}

func TestProductionLoggerDropsPushNamesAndMessageText(t *testing.T) {
	const pushName = "Fixture Push"
	const contactName = "Fixture Contact"
	var buf bytes.Buffer
	logger := newBridgeLogger("Client", true, &buf)
	logger.Debugf("Got push name %s for %s in history sync", pushName, secretPhone+"@s.whatsapp.net")
	logger.Warnf("Failed to parse user ID '%s' in push name history sync: %v", secretPhone, errBadID{})
	logger.Warnf("Node handling took 6s for <message from=%q notify=%q><conversation>%s</conversation></message>", secretPhone+"@s.whatsapp.net", contactName, secretBody)
	got := buf.String()
	for _, secret := range []string{pushName, contactName, secretPhone, secretBody, "@s.whatsapp.net"} {
		if strings.Contains(got, secret) {
			t.Fatalf("stderr leaked %q: %s", secret, got)
		}
	}
	if strings.Contains(got, "Got push name") {
		t.Fatalf("debug push name line reached stderr: %s", got)
	}
	if !strings.Contains(got, "push name history sync") || !strings.Contains(got, "Node handling") {
		t.Fatalf("stderr lost metadata: %s", got)
	}
}

type errBadID struct{}

func (errBadID) Error() string { return "bad id" }

func TestChatPresenceLogIsMetadataOnly(t *testing.T) {
	line := chatPresenceLog("typing", "")
	if strings.Contains(line, "@") || strings.Contains(line, secretPhone) {
		t.Fatalf("presence log contains an identifier: %s", line)
	}
	if !strings.Contains(line, "state=typing") {
		t.Fatalf("presence log lost state: %s", line)
	}
}

func testBridgeLogger(verbose bool, out *bytes.Buffer) waLog.Logger {
	level := zerolog.WarnLevel
	if verbose {
		level = zerolog.DebugLevel
	}
	logger := zerolog.New(&redactingWriter{out: out}).Level(level).With().Timestamp().Logger()
	return waLog.Zerolog(logger)
}
