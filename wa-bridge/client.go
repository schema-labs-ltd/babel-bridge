package main

import (
	"context"
	"fmt"
	"io"
	"os"
	"sync"
	"time"

	"github.com/rs/zerolog"
	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types/events"
	waLog "go.mau.fi/whatsmeow/util/log"

	_ "github.com/mattn/go-sqlite3"
)

const albumUploadConcurrency = 3

func mapAlbumUploads[Input any, Output any](
	ctx context.Context,
	inputs []Input,
	concurrency int,
	work func(context.Context, Input) (Output, error),
	progress func(int),
) ([]Output, error) {
	if concurrency < 1 {
		concurrency = 1
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	outputs := make([]Output, len(inputs))
	jobs := make(chan int)
	var workers sync.WaitGroup
	var state sync.Mutex
	var firstError error
	completed := 0

	workerCount := min(concurrency, len(inputs))
	workers.Add(workerCount)
	for range workerCount {
		go func() {
			defer workers.Done()
			for index := range jobs {
				output, err := work(ctx, inputs[index])
				if err != nil {
					state.Lock()
					if firstError == nil {
						firstError = err
						cancel()
					}
					state.Unlock()
					continue
				}
				outputs[index] = output
				state.Lock()
				completed++
				done := completed
				state.Unlock()
				if progress != nil {
					progress(done)
				}
			}
		}()
	}

	for index := range inputs {
		select {
		case jobs <- index:
		case <-ctx.Done():
			close(jobs)
			workers.Wait()
			state.Lock()
			defer state.Unlock()
			if firstError == nil {
				firstError = ctx.Err()
			}
			return nil, firstError
		}
	}
	close(jobs)
	workers.Wait()
	state.Lock()
	defer state.Unlock()
	if firstError == nil && ctx.Err() != nil {
		firstError = ctx.Err()
	}
	return outputs, firstError
}

// Client wraps the whatsmeow client and handles events
type Client struct {
	client        *whatsmeow.Client
	container     *sqlstore.Container
	dbPath        string
	verbose       bool
	ctx           context.Context
	closed        bool
	connectedNote chan struct{}
	lossNote      chan connectionLoss
}

// stderrLogger creates a logger that writes to stderr (not stdout)
// This prevents log output from mixing with our JSON protocol on stdout
func stderrLogger(module string, verbose bool) waLog.Logger {
	return newBridgeLogger(module, verbose, os.Stderr)
}

func newBridgeLogger(module string, verbose bool, out io.Writer) waLog.Logger {
	// whatsmeow debug lines include push names and message payloads.
	// Verbose mode still records event types on the JSON protocol; the
	// library logger stays at warn, and warn lines are redacted.
	_ = verbose
	logger := zerolog.New(zerolog.ConsoleWriter{
		Out:        &redactingWriter{out: out},
		TimeFormat: "15:04:05.000",
	}).Level(zerolog.WarnLevel).With().Str("module", module).Timestamp().Logger()

	return waLog.Zerolog(logger)
}

// NewClient creates a new WhatsApp client
func NewClient(ctx context.Context, dataDir string, verbose bool) (*Client, error) {
	// Set up logging to stderr (not stdout, which is reserved for JSON protocol)
	dbLog := stderrLogger("Database", verbose)

	// Create database container for session storage
	dbPath := fmt.Sprintf("%s/session.db", dataDir)
	container, err := sqlstore.New(ctx, "sqlite3", fmt.Sprintf("file:%s?_foreign_keys=on", dbPath), dbLog)
	if err != nil {
		return nil, fmt.Errorf("failed to create database: %w", err)
	}

	// Get or create device store
	deviceStore, err := container.GetFirstDevice(ctx)
	if err != nil {
		return nil, fmt.Errorf("failed to get device: %w", err)
	}

	// Create client with stderr logger
	clientLog := stderrLogger("Client", verbose)
	client := whatsmeow.NewClient(deviceStore, clientLog)
	// This process owns reconnect timing. whatsmeow's built-in loop has no cap
	// and does not retry a failed initial dial.
	client.EnableAutoReconnect = false

	c := &Client{
		client:        client,
		container:     container,
		dbPath:        dbPath,
		verbose:       verbose,
		ctx:           ctx,
		connectedNote: make(chan struct{}, 1),
		lossNote:      make(chan connectionLoss, 1),
	}

	// Register event handler
	client.AddEventHandler(c.handleEvent)

	return c, nil
}

// Connect initiates the connection to WhatsApp
func (c *Client) Connect(ctx context.Context) error {
	if c.client.Store.ID == nil {
		// No existing session, need to pair with QR code
		qrChan, err := c.client.GetQRChannel(ctx)
		if err != nil {
			return fmt.Errorf("failed to start linking: %w", err)
		}
		err = c.client.Connect()
		if err != nil {
			return fmt.Errorf("failed to connect: %w", err)
		}

		// Wait for QR code or successful login
		for evt := range qrChan {
			switch evt.Event {
			case "code":
				// Send QR code to Rust CLI
				SendEvent(NewQREvent(evt.Code))
			case "success":
				// Pairing precedes the authenticated Connected event.
				return nil
			case "timeout":
				return fmt.Errorf("QR code scan timed out")
			case "error", "err-unexpected-event", "err-unexpected-state", "err-client-outdated", "err-scanned-without-multidevice":
				return fmt.Errorf("linking failed: %s", evt.Event)
			}
		}
		return fmt.Errorf("linking channel closed before completion")
	} else {
		// Existing session, just connect
		err := c.client.Connect()
		if err != nil {
			return fmt.Errorf("failed to connect: %w", err)
		}
	}

	return nil
}

// MaintainConnection dials WhatsApp and, after a retryable failure, waits and
// dials again. A logout or an outdated client stops the loop; the process stays
// up until the parent cancels ctx.
func (c *Client) MaintainConnection(ctx context.Context) {
	dropSocket := false
	maintainWhatsAppConnection(ctx,
		func(ctx context.Context) error {
			if dropSocket {
				c.Disconnect()
				dropSocket = false
			}
			c.drainRetryableSignals()
			if loss, ok := c.takeStopLoss(); ok {
				return stopError{loss: loss}
			}
			err := c.Connect(ctx)
			if loss, ok := c.takeStopLoss(); ok {
				return stopError{loss: loss}
			}
			return err
		},
		c.watchConnection,
		randomConnectJitter,
		sleepContext,
		func(plan connectRetryPlan) {
			if plan.Retry {
				dropSocket = true
			}
			reportConnectPlan(plan)
		},
	)
}

func (c *Client) watchConnection(ctx context.Context) (watchedConnection, bool) {
	if c == nil || c.client == nil {
		return watchedConnection{}, false
	}
	reset := c.client.IsLoggedIn()
	select {
	case <-c.connectedNote:
		reset = true
	default:
	}
	for {
		select {
		case <-ctx.Done():
			return watchedConnection{}, false
		case <-c.connectedNote:
			reset = true
		case loss := <-c.lossNote:
			return watchedConnection{reset: reset, loss: loss}, true
		}
	}
}

func (c *Client) observeConnection(evt interface{}) {
	if c == nil {
		return
	}
	if _, ok := evt.(*events.Connected); ok {
		c.signalConnected()
	}
	if loss, ok := classifyConnectionEvent(evt, time.Now()); ok {
		c.signalLoss(loss)
	}
}

func (c *Client) signalConnected() {
	if c == nil || c.connectedNote == nil {
		return
	}
	select {
	case c.connectedNote <- struct{}{}:
	default:
	}
}

func (c *Client) signalLoss(loss connectionLoss) {
	if c == nil || c.lossNote == nil {
		return
	}
	select {
	case c.lossNote <- loss:
		return
	default:
	}
	if loss.class == connectFailureRetryable {
		return
	}
	select {
	case <-c.lossNote:
	default:
	}
	select {
	case c.lossNote <- loss:
	default:
	}
}

func (c *Client) drainRetryableSignals() {
	if c == nil {
		return
	}
	if c.connectedNote != nil {
		select {
		case <-c.connectedNote:
		default:
		}
	}
	if c.lossNote == nil {
		return
	}
	select {
	case loss := <-c.lossNote:
		if loss.class != connectFailureRetryable {
			c.signalLoss(loss)
		}
	default:
	}
}

func (c *Client) takeStopLoss() (connectionLoss, bool) {
	if c == nil || c.lossNote == nil {
		return connectionLoss{}, false
	}
	select {
	case loss := <-c.lossNote:
		if loss.class == connectFailureRetryable {
			c.signalLoss(loss)
			return connectionLoss{}, false
		}
		return loss, true
	default:
		return connectionLoss{}, false
	}
}

// Disconnect gracefully disconnects from WhatsApp
func (c *Client) Disconnect() {
	c.client.Disconnect()
}

// Close closes the database connection. A second call is a no-op.
func (c *Client) Close() error {
	if c == nil || c.container == nil {
		return nil
	}
	err := c.container.Close()
	c.container = nil
	return err
}

// Logout clears the session and disconnects
func (c *Client) Logout(ctx context.Context) error {
	return c.client.Logout(ctx)
}

// IsConnected returns whether the client is connected
func (c *Client) IsConnected() bool {
	return c.client.IsConnected()
}

// IsLoggedIn returns whether the client is logged in
func (c *Client) IsLoggedIn() bool {
	return c.client.IsLoggedIn()
}
