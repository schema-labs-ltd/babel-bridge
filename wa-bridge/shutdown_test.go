package main

import (
	"context"
	"database/sql"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"

	"go.mau.fi/whatsmeow/store/sqlstore"

	_ "github.com/mattn/go-sqlite3"
)

func TestSignalName(t *testing.T) {
	if signalName(syscall.SIGTERM) != "SIGTERM" {
		t.Fatalf("SIGTERM name = %s", signalName(syscall.SIGTERM))
	}
	if signalName(syscall.SIGINT) != "SIGINT" {
		t.Fatalf("SIGINT name = %s", signalName(syscall.SIGINT))
	}
	if signalName(syscall.SIGHUP) != "unknown" {
		t.Fatalf("other signal name = %s", signalName(syscall.SIGHUP))
	}
}

func TestShutdownSessionOrder(t *testing.T) {
	var order []string
	err := shutdownSession(
		func() { order = append(order, "disconnect") },
		func() error {
			order = append(order, "checkpoint")
			return nil
		},
		func() error {
			order = append(order, "close")
			return errors.New("close failed")
		},
	)
	if err == nil || !strings.Contains(err.Error(), "close failed") {
		t.Fatalf("error = %v", err)
	}
	if strings.Join(order, ",") != "disconnect,checkpoint,close" {
		t.Fatalf("order = %v", order)
	}
}

func TestShutdownSessionClosesStoreWhenCheckpointFails(t *testing.T) {
	closed := false
	err := shutdownSession(
		func() {},
		func() error { return errors.New("checkpoint failed") },
		func() error {
			closed = true
			return nil
		},
	)
	if err == nil {
		t.Fatal("expected checkpoint error")
	}
	if !closed {
		t.Fatal("store was not closed")
	}
}

func TestClientShutdownFlushesAndClosesSqlStore(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "session.db")
	container, err := sqlstore.New(context.Background(), "sqlite3", "file:"+path+"?_foreign_keys=on", nil)
	if err != nil {
		t.Fatal(err)
	}
	client := &Client{container: container, dbPath: path}

	probe, err := sql.Open("sqlite3", "file:"+path+"?_foreign_keys=on")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := probe.Exec("PRAGMA busy_timeout=5000"); err != nil {
		t.Fatal(err)
	}
	if _, err := probe.Exec("PRAGMA journal_mode=WAL"); err != nil {
		t.Fatal(err)
	}
	if _, err := probe.Exec("CREATE TABLE IF NOT EXISTS shutdown_probe (id INTEGER)"); err != nil {
		t.Fatal(err)
	}
	if _, err := probe.Exec("INSERT INTO shutdown_probe (id) VALUES (1)"); err != nil {
		t.Fatal(err)
	}
	if err := probe.Close(); err != nil {
		t.Fatal(err)
	}

	if err := client.Shutdown(); err != nil {
		t.Fatal(err)
	}
	if client.container != nil {
		t.Fatal("session store was not closed")
	}
	if err := client.Shutdown(); err != nil {
		t.Fatal(err)
	}

	// Drop the WAL so the probe row is visible only if checkpoint wrote it
	// into the main database file.
	_ = os.Remove(path + "-wal")
	_ = os.Remove(path + "-shm")

	reopened, err := sql.Open("sqlite3", "file:"+path+"?mode=ro")
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	var id int
	if err := reopened.QueryRow("SELECT id FROM shutdown_probe").Scan(&id); err != nil {
		t.Fatal(err)
	}
	if id != 1 {
		t.Fatalf("probe id = %d", id)
	}
}
