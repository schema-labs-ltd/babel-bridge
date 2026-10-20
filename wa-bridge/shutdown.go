package main

import (
	"database/sql"
	"errors"
	"fmt"
	"os"
	"syscall"
)

func signalName(sig os.Signal) string {
	switch sig {
	case syscall.SIGTERM:
		return "SIGTERM"
	case syscall.SIGINT:
		return "SIGINT"
	default:
		return "unknown"
	}
}

// shutdownSession disconnects WhatsApp, flushes the session database, then
// closes it. A checkpoint failure still closes the store.
func shutdownSession(disconnect func(), checkpoint func() error, closeStore func() error) error {
	if disconnect != nil {
		disconnect()
	}
	var errs []error
	if checkpoint != nil {
		if err := checkpoint(); err != nil {
			errs = append(errs, err)
		}
	}
	if closeStore != nil {
		if err := closeStore(); err != nil {
			errs = append(errs, err)
		}
	}
	return errors.Join(errs...)
}

func checkpointSQLite(path string) error {
	if path == "" {
		return nil
	}
	db, err := sql.Open("sqlite3", fmt.Sprintf("file:%s?_foreign_keys=on", path))
	if err != nil {
		return err
	}
	defer db.Close()
	if _, err := db.Exec("PRAGMA busy_timeout=1000"); err != nil {
		return err
	}
	_, err = db.Exec("PRAGMA wal_checkpoint(TRUNCATE)")
	return err
}

// Shutdown disconnects the WhatsApp socket and flushes the session database.
// A second call is a no-op so every exit path can defer it.
func (c *Client) Shutdown() error {
	if c == nil || c.closed {
		return nil
	}
	c.closed = true
	return shutdownSession(
		func() {
			if c.client != nil {
				c.client.Disconnect()
				c.client = nil
			}
		},
		func() error { return checkpointSQLite(c.dbPath) },
		func() error { return c.Close() },
	)
}
