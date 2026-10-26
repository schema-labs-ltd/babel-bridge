#!/usr/bin/env bash
# Posts the `review-gate` commit status. It passes once Copilot has reviewed
# the PR's head commit and Codex has reviewed the PR, commented on it, or left
# a +1 reaction on it.
#
#   review-gate.sh selftest   check the rules against sample API responses
#   review-gate.sh run        evaluate PR_NUMBER in REPO, or every open PR
#                             when PR_NUMBER is empty
#
# With POLL_SECONDS set, `run` re-checks every 30 seconds until the gate passes,
# the time runs out, or a newer commit replaces the head. Reactions do not
# trigger workflows, so .github/workflows/review-gate.yml also runs on a
# schedule to catch a Codex +1 that arrives after the poll ends.
set -euo pipefail

COPILOT=copilot-pull-request-reviewer[bot]
CODEX=chatgpt-codex-connector[bot]

# Reads reviews, reactions and comments JSON arrays and prints "pass" or what
# the gate is still waiting for.
evaluate() {
  local reviews="$1" reactions="$2" comments="$3" head="$4" waiting=()
  if ! jq -e --arg bot "$COPILOT" --arg head "$head" \
    'any(.[]; .user.login == $bot and .commit_id == $head)' "$reviews" > /dev/null; then
    waiting+=("Copilot on ${head:0:7}")
  fi
  if ! jq -e --arg bot "$CODEX" 'any(.[]; .user.login == $bot)' "$reviews" > /dev/null &&
    ! jq -e --arg bot "$CODEX" 'any(.[]; .user.login == $bot and .content == "+1")' "$reactions" > /dev/null &&
    ! jq -e --arg bot "$CODEX" 'any(.[]; .user.login == $bot)' "$comments" > /dev/null; then
    waiting+=("Codex")
  fi
  if [ "${#waiting[@]}" -eq 0 ]; then
    echo pass
  else
    local IFS=,
    echo "Waiting for ${waiting[*]}" | sed 's/,/ and /'
  fi
}

selftest() {
  local tmp failed=0 head=0123456789abcdef0123456789abcdef01234567
  tmp=$(mktemp -d)
  expect() {
    printf '%s' "$2" > "$tmp/reviews.json"
    printf '%s' "$3" > "$tmp/reactions.json"
    printf '%s' "$4" > "$tmp/comments.json"
    local got
    got=$(evaluate "$tmp/reviews.json" "$tmp/reactions.json" "$tmp/comments.json" "$head")
    if [ "$got" != "$5" ]; then
      echo "error: review gate '$1' gave '$got', expected '$5'"
      failed=1
    fi
  }
  local copilot="{\"user\":{\"login\":\"$COPILOT\"},\"commit_id\":\"$head\"}"
  local copilot_old="{\"user\":{\"login\":\"$COPILOT\"},\"commit_id\":\"fedcba9876543210fedcba9876543210fedcba98\"}"
  local codex_review="{\"user\":{\"login\":\"$CODEX\"},\"commit_id\":\"fedcba9876543210fedcba9876543210fedcba98\"}"
  local codex_up="{\"user\":{\"login\":\"$CODEX\"},\"content\":\"+1\"}"
  local codex_eyes="{\"user\":{\"login\":\"$CODEX\"},\"content\":\"eyes\"}"
  local codex_comment="{\"user\":{\"login\":\"$CODEX\"}}"
  local human_up='{"user":{"login":"vultuk"},"content":"+1"}'
  expect "nothing yet" '[]' '[]' '[]' "Waiting for Copilot on 0123456 and Codex"
  expect "copilot and codex review" "[$copilot,$codex_review]" '[]' '[]' pass
  expect "copilot and codex +1" "[$copilot]" "[$codex_up]" '[]' pass
  expect "copilot and codex comment" "[$copilot]" '[]' "[$codex_comment]" pass
  expect "codex still looking" "[$copilot]" "[$codex_eyes]" '[]' "Waiting for Codex"
  expect "someone else's +1" "[$copilot]" "[$human_up]" '[]' "Waiting for Codex"
  expect "copilot on an old commit" "[$copilot_old]" "[$codex_up]" '[]' "Waiting for Copilot on 0123456"
  rm -rf "$tmp"
  if [ "$failed" -ne 0 ]; then
    exit 1
  fi
  echo "review gate self-test passed"
}

current_status() {
  gh api "repos/$REPO/commits/$1/statuses?per_page=100" |
    jq -r '[.[] | select(.context == "review-gate")][0] | "\(.state // "")\t\(.description // "")"'
}

post_status() {
  local sha="$1" state="$2" description="$3" current
  current=$(current_status "$sha")
  if [ "$current" = "$state"$'\t'"$description" ]; then
    return 0
  fi
  gh api "repos/$REPO/statuses/$sha" -f state="$state" -f context=review-gate \
    -f description="${description:0:140}" -f target_url="${RUN_URL:-}" > /dev/null
}

check_pr() {
  local number="$1" work head result deadline
  work=$(mktemp -d)
  deadline=$((SECONDS + ${POLL_SECONDS:-0}))
  head=""
  while :; do
    gh api "repos/$REPO/pulls/$number" > "$work/pr.json"
    if [ "$(jq -r .state "$work/pr.json")" != open ]; then
      echo "PR #$number is not open."
      return 0
    fi
    if [ -n "$head" ] && [ "$head" != "$(jq -r .head.sha "$work/pr.json")" ]; then
      echo "PR #$number has a newer head; its own run takes over."
      return 0
    fi
    head=$(jq -r .head.sha "$work/pr.json")
    gh api --paginate "repos/$REPO/pulls/$number/reviews?per_page=100" | jq -s 'add // []' > "$work/reviews.json"
    gh api --paginate "repos/$REPO/issues/$number/reactions?per_page=100" | jq -s 'add // []' > "$work/reactions.json"
    gh api --paginate "repos/$REPO/issues/$number/comments?per_page=100" | jq -s 'add // []' > "$work/comments.json"
    result=$(evaluate "$work/reviews.json" "$work/reactions.json" "$work/comments.json" "$head")
    echo "PR #$number at ${head:0:7}: $result"
    if [ "$result" = pass ]; then
      post_status "$head" success "Copilot reviewed ${head:0:7}; Codex reviewed the PR"
      return 0
    fi
    post_status "$head" pending "$result"
    if [ "$SECONDS" -ge "$deadline" ]; then
      return 0
    fi
    sleep 30
  done
}

run() {
  : "${REPO:?}"
  if [ -n "${PR_NUMBER:-}" ]; then
    check_pr "$PR_NUMBER"
    return 0
  fi
  local number
  for number in $(gh api --paginate "repos/$REPO/pulls?state=open&per_page=100" | jq -r '.[] | select(.draft | not) | .number'); do
    check_pr "$number" || echo "::warning::could not evaluate PR #$number"
  done
}

case "${1:-}" in
  selftest) selftest ;;
  run) run ;;
  *)
    echo "usage: $0 selftest|run" >&2
    exit 2
    ;;
esac
