package main

import (
	"context"
	"database/sql"
	"os"
	"path/filepath"
	"testing"

	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"

	_ "github.com/mattn/go-sqlite3"
)

// Schema written by whatsmeow v0.0.0-20260616120636 (store version 14).
// Versions 15 and 16 only add columns; the linked device row must survive.
const sessionSchemaV14 = `
CREATE TABLE whatsmeow_device (
	jid TEXT PRIMARY KEY,
	lid TEXT,
	facebook_uuid uuid,
	registration_id BIGINT NOT NULL CHECK ( registration_id >= 0 AND registration_id < 4294967296 ),
	noise_key    bytea NOT NULL CHECK ( length(noise_key) = 32 ),
	identity_key bytea NOT NULL CHECK ( length(identity_key) = 32 ),
	signed_pre_key     bytea   NOT NULL CHECK ( length(signed_pre_key) = 32 ),
	signed_pre_key_id  INTEGER NOT NULL CHECK ( signed_pre_key_id >= 0 AND signed_pre_key_id < 16777216 ),
	signed_pre_key_sig bytea   NOT NULL CHECK ( length(signed_pre_key_sig) = 64 ),
	adv_key             bytea NOT NULL,
	adv_details         bytea NOT NULL,
	adv_account_sig     bytea NOT NULL CHECK ( length(adv_account_sig) = 64 ),
	adv_account_sig_key bytea NOT NULL CHECK ( length(adv_account_sig_key) = 32 ),
	adv_device_sig      bytea NOT NULL CHECK ( length(adv_device_sig) = 64 ),
	platform      TEXT NOT NULL DEFAULT '',
	business_name TEXT NOT NULL DEFAULT '',
	push_name     TEXT NOT NULL DEFAULT '',
	lid_migration_ts BIGINT NOT NULL DEFAULT 0
);
CREATE TABLE whatsmeow_identity_keys (
	our_jid TEXT, their_id TEXT, identity bytea NOT NULL CHECK ( length(identity) = 32 ),
	PRIMARY KEY (our_jid, their_id),
	FOREIGN KEY (our_jid) REFERENCES whatsmeow_device(jid) ON DELETE CASCADE ON UPDATE CASCADE
);
CREATE TABLE whatsmeow_sessions (
	our_jid TEXT, their_id TEXT, session bytea,
	PRIMARY KEY (our_jid, their_id),
	FOREIGN KEY (our_jid) REFERENCES whatsmeow_device(jid) ON DELETE CASCADE ON UPDATE CASCADE
);
CREATE TABLE whatsmeow_contacts (
	our_jid TEXT, their_jid TEXT, first_name TEXT, full_name TEXT, push_name TEXT, business_name TEXT, redacted_phone TEXT,
	PRIMARY KEY (our_jid, their_jid),
	FOREIGN KEY (our_jid) REFERENCES whatsmeow_device(jid) ON DELETE CASCADE ON UPDATE CASCADE
);
CREATE TABLE whatsmeow_chat_settings (
	our_jid TEXT, chat_jid TEXT,
	muted_until BIGINT NOT NULL DEFAULT 0,
	pinned BOOLEAN NOT NULL DEFAULT false,
	archived BOOLEAN NOT NULL DEFAULT false,
	PRIMARY KEY (our_jid, chat_jid),
	FOREIGN KEY (our_jid) REFERENCES whatsmeow_device(jid) ON DELETE CASCADE ON UPDATE CASCADE
);
CREATE TABLE whatsmeow_version (version INTEGER, compat INTEGER);
`

func TestExistingSessionSurvivesStoreUpgrade(t *testing.T) {
	dir := t.TempDir()
	historyMarker := []byte("history-marker")
	if err := os.WriteFile(filepath.Join(dir, "messages.db"), historyMarker, 0o600); err != nil {
		t.Fatal(err)
	}

	deviceJID := types.NewJID("15550001111", types.DefaultUserServer)
	deviceJID.Device = 1
	noiseKey := bytes32(1)
	identityKey := bytes32(2)
	signedPreKey := bytes32(3)
	signedPreKeySig := bytes64(4)
	accountSig := bytes64(5)
	accountSigKey := bytes32(6)
	deviceSig := bytes64(7)
	sessionBlob := []byte{9, 8, 7, 6}
	dbPath := filepath.Join(dir, "session.db")
	dsn := "file:" + dbPath + "?_foreign_keys=on"
	db, err := sql.Open("sqlite3", dsn)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(sessionSchemaV14); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`INSERT INTO whatsmeow_version (version, compat) VALUES (14, 8)`); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`
		INSERT INTO whatsmeow_device (
			jid, registration_id, noise_key, identity_key, signed_pre_key, signed_pre_key_id, signed_pre_key_sig,
			adv_key, adv_details, adv_account_sig, adv_account_sig_key, adv_device_sig, platform, push_name
		) VALUES (?, 7, ?, ?, ?, 1, ?, ?, ?, ?, ?, ?, 'fixture-platform', 'fixture-push')`,
		deviceJID.String(), noiseKey[:], identityKey[:], signedPreKey[:], signedPreKeySig[:],
		[]byte{1, 2, 3, 4}, []byte{5, 6}, accountSig[:], accountSigKey[:], deviceSig[:],
	); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`INSERT INTO whatsmeow_sessions (our_jid, their_id, session) VALUES (?, ?, ?)`, deviceJID.String(), "15550002222.0", sessionBlob); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`INSERT INTO whatsmeow_contacts (our_jid, their_jid, full_name) VALUES (?, ?, ?)`, deviceJID.String(), "15550002222@s.whatsapp.net", "Fixture"); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`INSERT INTO whatsmeow_chat_settings (our_jid, chat_jid, muted_until) VALUES (?, ?, 42)`, deviceJID.String(), "15550002222@s.whatsapp.net"); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}

	ctx := context.Background()
	container, err := sqlstore.New(ctx, "sqlite3", dsn, nil)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = container.Close() })
	device, err := container.GetFirstDevice(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if device.ID == nil || device.ID.String() != deviceJID.String() {
		t.Fatalf("linked device was not kept: %#v", device.ID)
	}
	if device.NoiseKey == nil || device.NoiseKey.Priv == nil || *device.NoiseKey.Priv != noiseKey || device.IdentityKey == nil || device.IdentityKey.Priv == nil || *device.IdentityKey.Priv != identityKey {
		t.Fatal("device identity keys changed during upgrade")
	}
	if device.PushName != "fixture-push" || device.Platform != "fixture-platform" {
		t.Fatalf("device profile changed: %q %q", device.Platform, device.PushName)
	}

	checkDB, err := sql.Open("sqlite3", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer checkDB.Close()
	var version int
	if err := checkDB.QueryRow(`SELECT version FROM whatsmeow_version`).Scan(&version); err != nil {
		t.Fatal(err)
	}
	if version != 16 {
		t.Fatalf("store version = %d, want 16", version)
	}
	var nonce, secretID string
	var muted int
	var savedSession []byte
	var contactName string
	if err := checkDB.QueryRow(`SELECT companion_meta_nonce FROM whatsmeow_device WHERE jid = ?`, deviceJID.String()).Scan(&nonce); err != nil {
		t.Fatal(err)
	}
	if err := checkDB.QueryRow(`SELECT wasa_root_secret_id, muted_until FROM whatsmeow_chat_settings WHERE our_jid = ?`, deviceJID.String()).Scan(&secretID, &muted); err != nil {
		t.Fatal(err)
	}
	if err := checkDB.QueryRow(`SELECT session FROM whatsmeow_sessions WHERE our_jid = ?`, deviceJID.String()).Scan(&savedSession); err != nil {
		t.Fatal(err)
	}
	if err := checkDB.QueryRow(`SELECT full_name FROM whatsmeow_contacts WHERE our_jid = ?`, deviceJID.String()).Scan(&contactName); err != nil {
		t.Fatal(err)
	}
	if nonce != "" || secretID != "" || muted != 42 || string(savedSession) != string(sessionBlob) || contactName != "Fixture" {
		t.Fatalf("upgraded rows changed: nonce=%q secret=%q muted=%d session=%v contact=%q", nonce, secretID, muted, savedSession, contactName)
	}
	history, err := os.ReadFile(filepath.Join(dir, "messages.db"))
	if err != nil {
		t.Fatal(err)
	}
	if string(history) != string(historyMarker) {
		t.Fatal("message history file was modified")
	}
}

func bytes32(fill byte) [32]byte {
	var out [32]byte
	for i := range out {
		out[i] = fill
	}
	return out
}

func bytes64(fill byte) [64]byte {
	var out [64]byte
	for i := range out {
		out[i] = fill
	}
	return out
}
