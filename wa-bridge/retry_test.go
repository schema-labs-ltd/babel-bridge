package main

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"syscall"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/socket"
	"go.mau.fi/whatsmeow/types/events"
)

// fakeClock records waits and moves its own time forward. Tests never sleep.
type fakeClock struct {
	now    time.Time
	sleeps []time.Duration
}

func (c *fakeClock) sleep(ctx context.Context, delay time.Duration) bool {
	if ctx.Err() != nil {
		return false
	}
	c.sleeps = append(c.sleeps, delay)
	c.now = c.now.Add(delay)
	return true
}

func TestConnectRetryDelaysGrowAndCap(t *testing.T) {
	var retries connectRetries
	var previous time.Duration
	capped := false
	for attempt := 1; attempt <= 12; attempt++ {
		plan := retries.next("network", connectFailureRetryable, 1)
		if !plan.Retry {
			t.Fatal("network failures must be retried")
		}
		if plan.Attempt != attempt {
			t.Fatalf("attempt = %d, want %d", plan.Attempt, attempt)
		}
		if plan.Delay > connectRetryCap {
			t.Fatalf("delay %s exceeds cap", plan.Delay)
		}
		if plan.Delay == connectRetryCap {
			capped = true
		} else if attempt > 1 && plan.Delay <= previous {
			t.Fatalf("delay did not grow: attempt %d delay %s previous %s", attempt, plan.Delay, previous)
		}
		if attempt == 1 && plan.Delay != time.Second {
			t.Fatalf("initial delay = %s", plan.Delay)
		}
		previous = plan.Delay
	}
	if !capped {
		t.Fatal("expected the delay to reach the cap")
	}
	if previous != connectRetryCap {
		t.Fatalf("final delay = %s, want cap %s", previous, connectRetryCap)
	}
}

func TestConnectRetryJitterStaysUnderCapAndStillGrows(t *testing.T) {
	var full connectRetries
	var reduced connectRetries
	var previous time.Duration
	for attempt := 1; attempt <= 8; attempt++ {
		fullPlan := full.next("network", connectFailureRetryable, 1)
		plan := reduced.next("network", connectFailureRetryable, 0)
		if plan.Delay > connectRetryCap || plan.Delay > fullPlan.Delay {
			t.Fatalf("jittered delay %s escaped full delay %s", plan.Delay, fullPlan.Delay)
		}
		if plan.Delay < fullPlan.Delay*8/10-time.Millisecond {
			t.Fatalf("jitter removed more than 20%%: got %s full %s", plan.Delay, fullPlan.Delay)
		}
		if attempt > 1 && previous < connectRetryCap*8/10 && plan.Delay <= previous {
			t.Fatalf("jittered delay did not grow: attempt %d delay %s previous %s", attempt, plan.Delay, previous)
		}
		previous = plan.Delay
	}
}

func TestConnectRetryResetsAfterSuccess(t *testing.T) {
	var retries connectRetries
	first := retries.next("network", connectFailureRetryable, 1)
	second := retries.next("disconnected", connectFailureRetryable, 1)
	if second.Delay <= first.Delay {
		t.Fatalf("second delay %s did not grow past %s", second.Delay, first.Delay)
	}
	retries.success()
	reset := retries.next("network", connectFailureRetryable, 1)
	if reset.Attempt != 1 || reset.Delay != first.Delay {
		t.Fatalf("delay after a successful connect = attempt %d %s, want attempt 1 %s", reset.Attempt, reset.Delay, first.Delay)
	}
}

func TestConnectRetryLoopUsesFakeClockAndStopsOnPermanentFailure(t *testing.T) {
	secret := "dial-error-must-stay-out-of-the-log"
	steps := []error{
		fmt.Errorf("dial %s: %w", secret, socket.ErrDialFailed),
		fmt.Errorf("connect: %w", syscall.ECONNREFUSED),
		fmt.Errorf("linking failed: err-client-outdated (%s)", secret),
		fmt.Errorf("dial %s: %w", secret, socket.ErrDialFailed),
	}
	index := 0
	clock := &fakeClock{}
	var plans []connectRetryPlan
	ctx := context.Background()
	maintainWhatsAppConnection(ctx,
		func(context.Context) error {
			if index >= len(steps) {
				t.Fatal("dialed after a permanent failure")
			}
			err := steps[index]
			index++
			return err
		},
		func(context.Context) (watchedConnection, bool) {
			t.Fatal("watch should not run when every dial fails")
			return watchedConnection{}, false
		},
		func() float64 { return 1 },
		clock.sleep,
		func(plan connectRetryPlan) {
			plans = append(plans, plan)
			line := connectAttemptLog(plan)
			if strings.Contains(line, secret) {
				t.Fatalf("log contains the dial error: %s", line)
			}
		},
	)
	if index != 3 {
		t.Fatalf("dialed %d times, want 3 (two retries then a permanent stop)", index)
	}
	if len(clock.sleeps) != 2 || len(plans) != 3 {
		t.Fatalf("sleeps=%d plans=%d, want 2 sleeps and 3 plans", len(clock.sleeps), len(plans))
	}
	if clock.sleeps[0] >= clock.sleeps[1] {
		t.Fatalf("delays did not grow: %s then %s", clock.sleeps[0], clock.sleeps[1])
	}
	var waited time.Duration
	for i, delay := range clock.sleeps {
		if delay != plans[i].Delay {
			t.Fatalf("slept %s, planned %s", delay, plans[i].Delay)
		}
		waited += delay
		if plans[i].Code != "network" || !plans[i].Retry {
			t.Fatalf("retry plan = %+v", plans[i])
		}
	}
	if clock.now.Sub(time.Time{}) != waited {
		t.Fatalf("fake clock at %s, waits sum to %s", clock.now.Sub(time.Time{}), waited)
	}
	stop := plans[2]
	if stop.Retry || stop.Code != "client_outdated" {
		t.Fatalf("permanent plan = %+v", stop)
	}
	if !strings.Contains(connectAttemptLog(stop), "will not help") {
		t.Fatalf("permanent log is unclear: %s", connectAttemptLog(stop))
	}
}

func TestConnectRetryLoopResetsDelayAfterConnected(t *testing.T) {
	type step struct {
		err   error
		watch *watchedConnection
	}
	loss := connectionLoss{code: "disconnected", class: connectFailureRetryable}
	steps := []step{
		{err: fmt.Errorf("%w", socket.ErrDialFailed)},
		{err: fmt.Errorf("%w", socket.ErrDialFailed)},
		{watch: &watchedConnection{reset: true, loss: loss}},
		{err: fmt.Errorf("%w", socket.ErrDialFailed)},
		{err: context.Canceled},
	}
	index := 0
	clock := &fakeClock{}
	ctx := context.Background()
	maintainWhatsAppConnection(ctx,
		func(context.Context) error {
			current := steps[index]
			if current.watch != nil {
				return nil
			}
			index++
			return current.err
		},
		func(context.Context) (watchedConnection, bool) {
			current := steps[index]
			index++
			return *current.watch, true
		},
		func() float64 { return 1 },
		clock.sleep,
		func(connectRetryPlan) {},
	)
	if len(clock.sleeps) != 4 {
		t.Fatalf("sleeps = %v, want 4", clock.sleeps)
	}
	if clock.sleeps[1] <= clock.sleeps[0] {
		t.Fatalf("delays did not grow before connect: %v", clock.sleeps)
	}
	if clock.sleeps[2] != clock.sleeps[0] {
		t.Fatalf("delay after a successful connect = %s, want reset %s", clock.sleeps[2], clock.sleeps[0])
	}
	if clock.sleeps[3] != clock.sleeps[1] {
		t.Fatalf("delay did not grow again after the reset: %v", clock.sleeps)
	}
	if clock.sleeps[3] == clock.sleeps[1]*2 {
		t.Fatalf("backoff kept climbing across the successful connect: %v", clock.sleeps)
	}
}

func TestPermanentConnectionEventsDoNotRetry(t *testing.T) {
	now := time.Unix(1_700_000_000, 0)
	cases := []interface{}{
		&events.LoggedOut{Reason: events.ConnectFailureLoggedOut},
		&events.ClientOutdated{},
		&events.TemporaryBan{},
		&events.CATRefreshError{Error: errors.New("refresh failed")},
		&events.ConnectFailure{Reason: events.ConnectFailureBadUserAgent},
		&events.ConnectFailure{Reason: events.ConnectFailureClientOutdated},
	}
	for _, event := range cases {
		loss, ok := classifyConnectionEvent(event, now)
		if !ok || loss.class != connectFailurePermanent {
			t.Fatalf("%T classified as retryable (%+v)", event, loss)
		}
		plan := (&connectRetries{}).next(loss.code, loss.class, 1)
		if plan.Retry {
			t.Fatalf("%T produced a retry plan", event)
		}
	}
	replaced, ok := classifyConnectionEvent(&events.StreamReplaced{}, now)
	if !ok || replaced.class != connectFailureHandoff {
		t.Fatal("a replaced stream must stop this loop without looking like a network retry")
	}
	retryable, ok := classifyConnectionEvent(&events.Disconnected{}, now)
	if !ok || retryable.class != connectFailureRetryable || retryable.code != "disconnected" {
		t.Fatalf("drop = %+v", retryable)
	}
	server := classifyConnectFailureReason(events.ConnectFailureServiceUnavailable)
	if server.class != connectFailureRetryable || server.code != "server_unavailable" {
		t.Fatalf("503 = %+v", server)
	}
}

func TestKeepAliveTimeoutRetriesOnlyAfterTheGiveUp(t *testing.T) {
	now := time.Unix(1_800_000_000, 0)
	recent := &events.KeepAliveTimeout{ErrorCount: 1, LastSuccess: now.Add(-time.Second)}
	if _, ok := classifyConnectionEvent(recent, now); ok {
		t.Fatal("a fresh keepalive miss must stay on the open socket")
	}
	dead := &events.KeepAliveTimeout{ErrorCount: 4, LastSuccess: now.Add(-connectKeepAliveGiveUp)}
	loss, ok := classifyConnectionEvent(dead, now)
	if !ok || loss.class != connectFailureRetryable || loss.code != "keepalive" {
		t.Fatalf("dead keepalive = %+v ok=%t", loss, ok)
	}
}
