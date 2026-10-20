package main

import (
	"context"
	"errors"
	"fmt"
	"math/rand/v2"
	"net"
	"strings"
	"syscall"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/socket"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/types/events"
)

const (
	// connectRetryInitial is the delay after the first failure when jitter is 1.
	connectRetryInitial = time.Second
	// connectRetryCap is the longest pause between attempts.
	connectRetryCap = time.Minute
	// connectKeepAliveGiveUp matches whatsmeow's keepalive give-up. Shorter
	// misses stay on the open socket; a dead socket is retried by us.
	connectKeepAliveGiveUp = 3 * time.Minute
)

// connectFailureClass decides whether another dial can help.
type connectFailureClass int

const (
	connectFailureRetryable connectFailureClass = iota
	// connectFailurePermanent stops the loop. Logging in is required, or the
	// server rejected this client and another dial will fail the same way.
	connectFailurePermanent
	// connectFailureStop ends the loop because the caller cancelled it.
	connectFailureStop
	// connectFailureHandoff ends the loop because the parent process is
	// replacing the bridge, such as after a replaced stream.
	connectFailureHandoff
)

type connectionLoss struct {
	code  string
	class connectFailureClass
}

// connectRetryPlan is one logged decision. Code is a stable token, never a
// phone number, name, or message.
type connectRetryPlan struct {
	Retry   bool
	Attempt int
	Delay   time.Duration
	Code    string
	Detail  string
}

// connectRetries counts consecutive retryable failures. A successful
// authentication resets it so the next outage starts at the initial delay.
type connectRetries struct {
	failures int
}

func (r *connectRetries) success() {
	r.failures = 0
}

func (r *connectRetries) next(code string, class connectFailureClass, jitter float64) connectRetryPlan {
	if class != connectFailureRetryable {
		return connectRetryPlan{
			Retry:  false,
			Code:   code,
			Detail: permanentConnectDetail(code),
		}
	}
	r.failures++
	return connectRetryPlan{
		Retry:   true,
		Attempt: r.failures,
		Delay:   connectRetryDelay(r.failures, jitter),
		Code:    code,
		Detail:  "retrying",
	}
}

// connectRetryDelay doubles until connectRetryCap. Jitter keeps 80–100% of
// that delay, so attempts still grow until the cap and never wait longer than it.
func connectRetryDelay(attempt int, jitter float64) time.Duration {
	if attempt < 1 {
		attempt = 1
	}
	if jitter < 0 {
		jitter = 0
	} else if jitter > 1 {
		jitter = 1
	}
	exponential := connectRetryInitial
	for step := 1; step < attempt && exponential < connectRetryCap; step++ {
		next := exponential * 2
		if next <= 0 || next > connectRetryCap {
			exponential = connectRetryCap
			break
		}
		exponential = next
	}
	// 0.8+0.2*jitter is exactly 1 when jitter is 1, and at least 0.8 otherwise.
	scaled := time.Duration(float64(exponential) * (0.8 + 0.2*jitter))
	if scaled > connectRetryCap {
		scaled = connectRetryCap
	}
	if scaled < time.Millisecond {
		scaled = time.Millisecond
	}
	return scaled
}

func randomConnectJitter() float64 {
	return rand.Float64()
}

func permanentConnectDetail(code string) string {
	switch code {
	case "client_outdated":
		return "the client is outdated and retrying will not help until the bridge is updated"
	case "logged_out":
		return "the phone must be linked again"
	case "temporary_ban":
		return "the account is temporarily restricted and retrying will not help"
	case "multidevice_disabled":
		return "multi-device is off on the phone and retrying will not help"
	case "cat_refresh":
		return "the session could not be refreshed and retrying will not help"
	case "connect_rejected":
		return "the server rejected the connection and retrying will not help"
	default:
		return "retrying will not help"
	}
}

func connectAttemptLog(plan connectRetryPlan) string {
	if plan.Retry {
		return fmt.Sprintf(
			"WhatsApp connect attempt %d failed (code=%s); retrying in %s",
			plan.Attempt, plan.Code, plan.Delay,
		)
	}
	return fmt.Sprintf("WhatsApp connection stopped (code=%s); %s", plan.Code, plan.Detail)
}

func reportConnectPlan(plan connectRetryPlan) {
	if plan.Retry {
		SendEvent(NewConnectionStateEvent("reconnecting"))
		SendEvent(NewLogEvent("warn", connectAttemptLog(plan)))
		return
	}
	// "disconnected" means retries have stopped. A logout event that arrives
	// as well still wins, and a later disconnected state does not clear it.
	SendEvent(NewConnectionStateEvent("disconnected"))
	SendEvent(NewLogEvent("error", connectAttemptLog(plan)))
}

type watchedConnection struct {
	reset bool
	loss  connectionLoss
}

type stopError struct {
	loss connectionLoss
}

func (e stopError) Error() string {
	return "connect stopped"
}

func followConnectFailure(
	ctx context.Context,
	retries *connectRetries,
	code string,
	class connectFailureClass,
	jitter func() float64,
	sleep func(context.Context, time.Duration) bool,
	report func(connectRetryPlan),
) bool {
	switch class {
	case connectFailureStop, connectFailureHandoff:
		return false
	}
	plan := retries.next(code, class, jitter())
	report(plan)
	if !plan.Retry {
		return false
	}
	return sleep(ctx, plan.Delay)
}

// maintainWhatsAppConnection dials until the context ends or a failure cannot
// be retried. sleep is injected so tests can advance a fake clock.
func maintainWhatsAppConnection(
	ctx context.Context,
	dial func(context.Context) error,
	watch func(context.Context) (watchedConnection, bool),
	jitter func() float64,
	sleep func(context.Context, time.Duration) bool,
	report func(connectRetryPlan),
) {
	var retries connectRetries
	for {
		if ctx.Err() != nil {
			return
		}
		err := dial(ctx)
		if ctx.Err() != nil {
			return
		}
		if err != nil {
			code, class := classifyConnectError(err)
			if !followConnectFailure(ctx, &retries, code, class, jitter, sleep, report) {
				return
			}
			continue
		}
		watched, ok := watch(ctx)
		if !ok {
			return
		}
		if watched.reset {
			retries.success()
		}
		if !followConnectFailure(ctx, &retries, watched.loss.code, watched.loss.class, jitter, sleep, report) {
			return
		}
	}
}

func sleepContext(ctx context.Context, delay time.Duration) bool {
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return false
	case <-timer.C:
		return true
	}
}

func classifyConnectError(err error) (string, connectFailureClass) {
	var stopped stopError
	if errors.As(err, &stopped) {
		return stopped.loss.code, stopped.loss.class
	}
	if err == nil || errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
		return "cancelled", connectFailureStop
	}
	text := err.Error()
	switch {
	case strings.Contains(text, "err-client-outdated"):
		return "client_outdated", connectFailurePermanent
	case strings.Contains(text, "err-scanned-without-multidevice"):
		return "multidevice_disabled", connectFailurePermanent
	case strings.Contains(text, "QR code scan timed out"), strings.Contains(text, "linking channel closed"):
		return "qr_timeout", connectFailureRetryable
	}
	if code, class, ok := classifyTransportError(err); ok {
		return code, class
	}
	if errors.Is(err, store.ErrDeviceDeleted) {
		return "logged_out", connectFailurePermanent
	}
	if errors.Is(err, whatsmeow.ErrAlreadyConnected) {
		return "already_connected", connectFailureRetryable
	}
	if strings.Contains(text, "linking failed:") {
		return "link_failed", connectFailureRetryable
	}
	return "connect", connectFailureRetryable
}

func classifyTransportError(err error) (string, connectFailureClass, bool) {
	var statusErr socket.ErrWithStatusCode
	if errors.As(err, &statusErr) {
		switch statusErr.StatusCode {
		case 408, 500, 501, 502, 503, 504:
			return "server_unavailable", connectFailureRetryable, true
		default:
			// A websocket rejection is not a dropped connection. Protocol logout
			// and an outdated client arrive as their own events.
			return "connect_rejected", connectFailurePermanent, true
		}
	}
	if errors.Is(err, socket.ErrDialFailed) {
		return "network", connectFailureRetryable, true
	}
	var netErr net.Error
	if errors.As(err, &netErr) {
		return "network", connectFailureRetryable, true
	}
	var errno syscall.Errno
	if errors.As(err, &errno) && networkErrno(errno) {
		return "network", connectFailureRetryable, true
	}
	return "", connectFailureRetryable, false
}

func networkErrno(errno syscall.Errno) bool {
	switch errno {
	case syscall.ENETDOWN, syscall.ENETUNREACH, syscall.ENETRESET,
		syscall.ECONNABORTED, syscall.ECONNRESET, syscall.ENOBUFS,
		syscall.ETIMEDOUT, syscall.ECONNREFUSED, syscall.EHOSTDOWN,
		syscall.EHOSTUNREACH, syscall.EPIPE:
		return true
	default:
		return false
	}
}

func classifyConnectionEvent(event interface{}, now time.Time) (connectionLoss, bool) {
	switch event := event.(type) {
	case *events.LoggedOut:
		return connectionLoss{code: "logged_out", class: connectFailurePermanent}, true
	case *events.ClientOutdated:
		return connectionLoss{code: "client_outdated", class: connectFailurePermanent}, true
	case *events.TemporaryBan:
		return connectionLoss{code: "temporary_ban", class: connectFailurePermanent}, true
	case *events.CATRefreshError:
		return connectionLoss{code: "cat_refresh", class: connectFailurePermanent}, true
	case *events.StreamReplaced:
		return connectionLoss{code: "stream_replaced", class: connectFailureHandoff}, true
	case *events.ConnectFailure:
		return classifyConnectFailureReason(event.Reason), true
	case *events.Disconnected:
		return connectionLoss{code: "disconnected", class: connectFailureRetryable}, true
	case *events.KeepAliveTimeout:
		if !keepaliveNeedsReconnect(event, now) {
			return connectionLoss{}, false
		}
		return connectionLoss{code: "keepalive", class: connectFailureRetryable}, true
	default:
		return connectionLoss{}, false
	}
}

func classifyConnectFailureReason(reason events.ConnectFailureReason) connectionLoss {
	switch reason {
	case events.ConnectFailureServiceUnavailable, events.ConnectFailureInternalServerError:
		return connectionLoss{code: "server_unavailable", class: connectFailureRetryable}
	case events.ConnectFailureClientOutdated, events.ConnectFailureBadUserAgent:
		return connectionLoss{code: "client_outdated", class: connectFailurePermanent}
	case events.ConnectFailureTempBanned:
		return connectionLoss{code: "temporary_ban", class: connectFailurePermanent}
	}
	if reason.IsLoggedOut() {
		return connectionLoss{code: "logged_out", class: connectFailurePermanent}
	}
	return connectionLoss{code: "connect_rejected", class: connectFailurePermanent}
}

func keepaliveNeedsReconnect(event *events.KeepAliveTimeout, now time.Time) bool {
	if event == nil || event.LastSuccess.IsZero() {
		return false
	}
	return now.Sub(event.LastSuccess) >= connectKeepAliveGiveUp
}
