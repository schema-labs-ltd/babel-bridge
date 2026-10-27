#!/usr/bin/env bash
# Post-deploy smoke check for the hosted server, run by
# .github/workflows/post-deploy-smoke.yml after each push to main.
#
# 1. Waits for /api/health to answer ok.
# 2. Polls /api/diagnostics until it reports EXPECTED_SHA as its commit
#    (Railway sets RAILWAY_GIT_COMMIT_SHA on each deploy).
# 3. Watches the new deploy for WATCH_SECONDS and fails if it logs an ERROR,
#    stops answering, or loses the WhatsApp connection the old deploy had.
#
# Environment:
#   SMOKE_URL        base URL of the server, e.g. https://example.up.railway.app
#   SMOKE_PASSWORD   the server's web password (WEB_PASSWORD); empty if none
#   EXPECTED_SHA     commit the new deploy must report
#   DEPLOY_TIMEOUT   seconds to wait for the commit to appear (default 2700)
#   WATCH_SECONDS    seconds to watch the new deploy (default 600)
#   POLL_SECONDS     seconds between polls (default 30)
#   IGNORE_ERRORS    optional extended regex; matching ERROR lines are ignored
#
# Exit status: 0 healthy, 1 the deploy is bad (revert it), 2 the deploy never
# reported EXPECTED_SHA (nothing to judge, so no revert).

set -euo pipefail

: "${SMOKE_URL:?SMOKE_URL is required}"
: "${EXPECTED_SHA:?EXPECTED_SHA is required}"
SMOKE_URL="${SMOKE_URL%/}"
SMOKE_PASSWORD="${SMOKE_PASSWORD:-}"
DEPLOY_TIMEOUT="${DEPLOY_TIMEOUT:-2700}"
WATCH_SECONDS="${WATCH_SECONDS:-600}"
POLL_SECONDS="${POLL_SECONDS:-30}"
IGNORE_ERRORS="${IGNORE_ERRORS:-}"
MAX_MISSED_POLLS=3
# Polls in a row that may reach the old instance before the new one counts as gone.
MAX_STALE_POLLS=10

# An invalid pattern would otherwise hide every error.
if [ -n "$IGNORE_ERRORS" ]; then
  status=0
  grep -Eq -- "$IGNORE_ERRORS" <<<"" || status=$?
  if [ "$status" -gt 1 ]; then
    echo "IGNORE_ERRORS is not a valid extended regex: $IGNORE_ERRORS"
    exit 2
  fi
fi

token=""
diagnostics=""

log() { printf '%s %s\n' "$(date -u +%H:%M:%S)" "$*"; }

sign_in() {
  token=""
  [ -n "$SMOKE_PASSWORD" ] || return 0
  local response
  response="$(jq -n --arg password "$SMOKE_PASSWORD" '{password: $password}' |
    curl -fsS --max-time 15 -H 'Content-Type: application/json' --data @- \
      "$SMOKE_URL/api/auth")" || return 1
  token="$(jq -r '.token // empty' <<<"$response")"
  if [ -n "$token" ] && [ -n "${GITHUB_ACTIONS:-}" ]; then
    echo "::add-mask::$token"
  fi
  if [ -z "$token" ] && [ "$(jq -r .success <<<"$response")" != "true" ]; then
    log "the server rejected SMOKE_PASSWORD; fix the secret"
    exit 2
  fi
}

# Sets $diagnostics. Signs in again on 401, because session tokens live in
# memory and every deploy starts with none. Returns 2 when the server still
# answers 401, which during a handover means sign-in and diagnostics reached
# different instances.
fetch_diagnostics() {
  local attempt status body
  body="$(mktemp)"
  for attempt in 1 2; do
    local auth=()
    [ -z "$token" ] || auth=(-H "Authorization: Bearer $token")
    status="$(curl -sS --max-time 15 -o "$body" -w '%{http_code}' "${auth[@]}" \
      "$SMOKE_URL/api/diagnostics" || echo 000)"
    if [ "$status" = "200" ] && jq -e '.recentProblems' "$body" >/dev/null 2>&1; then
      diagnostics="$(cat "$body")"
      rm -f "$body"
      return 0
    fi
    if [ "$status" = "401" ] && [ "$attempt" = 1 ] && sign_in; then
      continue
    fi
    break
  done
  rm -f "$body"
  log "diagnostics answered HTTP $status"
  [ "$status" != "401" ] || return 2
  return 1
}

healthy() {
  curl -fsS --max-time 15 "$SMOKE_URL/api/health" 2>/dev/null | jq -e '.ok == true' >/dev/null
}

error_lines() {
  local lines
  lines="$(jq -r '.recentProblems[]? | select(.line | startswith("ERROR")) | "\(.at) \(.line)"' <<<"$diagnostics")"
  if [ -n "$IGNORE_ERRORS" ] && [ -n "$lines" ]; then
    lines="$(grep -Ev -- "$IGNORE_ERRORS" <<<"$lines" || true)"
  fi
  printf '%s' "$lines"
}

# Whatever is live before the new deploy says whether WhatsApp should be connected.
was_connected=false
if fetch_diagnostics; then
  was_connected="$(jq -r '.connected' <<<"$diagnostics")"
  log "before deploy: commit $(jq -r '.commit // "unknown"' <<<"$diagnostics"), connected $was_connected"
fi

log "waiting for $SMOKE_URL to report $EXPECTED_SHA"
deadline=$((SECONDS + DEPLOY_TIMEOUT))
while :; do
  if healthy && fetch_diagnostics; then
    if [ "$(jq -r '.commit // empty' <<<"$diagnostics")" = "$EXPECTED_SHA" ]; then
      break
    fi
    # Still the old deploy. Once it has been seen connected, a drop during
    # the handover must not lower the bar for the new deploy.
    if [ "$(jq -r '.connected' <<<"$diagnostics")" = "true" ]; then
      was_connected=true
    fi
  fi
  if [ "$SECONDS" -ge "$deadline" ]; then
    log "the server never reported $EXPECTED_SHA within ${DEPLOY_TIMEOUT}s"
    exit 2
  fi
  sleep "$POLL_SECONDS"
done
log "deploy is live; watching it for ${WATCH_SECONDS}s"

missed=0
stale=0
ours="$diagnostics"
deadline=$((SECONDS + WATCH_SECONDS))
while :; do
  status=0
  fetch_diagnostics || status=$?
  if [ "$status" -eq 2 ]; then
    # A sign-in that landed on the other instance; judge it like a stale poll.
    stale=$((stale + 1))
    if [ "$stale" -ge "$MAX_STALE_POLLS" ]; then
      log "the new deploy kept refusing a fresh sign-in"
      exit 1
    fi
  elif [ "$status" -eq 0 ]; then
    missed=0
    commit="$(jq -r '.commit // empty' <<<"$diagnostics")"
    # During a rolling handover the old instance can still answer for a
    # while; a newer push cancels this run. Only a fresh answer from the
    # new deploy counts, and if the old one keeps answering, the new one
    # has gone (Railway rolled it back or it stopped serving).
    if [ "$commit" != "$EXPECTED_SHA" ]; then
      stale=$((stale + 1))
      log "this poll reached $commit, not $EXPECTED_SHA ($stale in a row)"
      if [ "$stale" -ge "$MAX_STALE_POLLS" ]; then
        log "the new deploy stopped serving"
        exit 1
      fi
      diagnostics="$ours"
    else
      stale=0
    fi
    ours="$diagnostics"
    errors="$(error_lines)"
    if [ -n "$errors" ]; then
      log "the new deploy logged errors:"
      printf '%s\n' "$errors"
      exit 1
    fi
  else
    missed=$((missed + 1))
    if [ "$missed" -ge "$MAX_MISSED_POLLS" ]; then
      log "the new deploy stopped answering diagnostics"
      exit 1
    fi
  fi
  # Past the deadline, keep polling until the new deploy answers again or fails a limit.
  if [ "$SECONDS" -ge "$deadline" ] && [ "$missed" -eq 0 ] && [ "$stale" -eq 0 ]; then
    break
  fi
  sleep "$POLL_SECONDS"
done

if [ "$was_connected" = "true" ] && [ "$(jq -r '.connected' <<<"$diagnostics")" != "true" ]; then
  log "WhatsApp was connected before the deploy and is not connected now"
  exit 1
fi
log "the new deploy is healthy"
