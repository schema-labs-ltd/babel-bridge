#!/usr/bin/env bash
# Classifies a pull request as risk:low, risk:standard or risk:high from the
# paths it changes, labels it, and posts the `risk` commit status. A high-risk
# PR passes only after an approver comments `/approve <head sha>`.
#
#   pr-risk.sh selftest   check the path rules and the approval rules
#   pr-risk.sh run        classify PR_NUMBER in REPO and post the status
#
# .github/workflows/risk.yml runs this file from the default branch, so a PR
# cannot change how it is classified. Change the tiers here and add a matching
# expect_tier line to selftest.
set -euo pipefail

MAX_LINES=800

# Prints high, low or standard for one path. High wins over low, so a test
# under a high-risk path (src/push/tests.rs) is high.
tier_for_path() {
  case "$1" in
    .github/* | .claude/* | lefthook.yml | scripts/check-* | Makefile | .gitattributes | \
      AGENTS.md | */AGENTS.md | CLAUDE.md | */CLAUDE.md | REVIEW.md | */REVIEW.md | \
      src/web/auth.rs | src/web/oauth.rs | src/web/oauth/* | src/oauth.rs | src/access.rs | \
      src/log_privacy.rs | src/push* | src/storage/schema* | fixtures/api/* | \
      Dockerfile | */Dockerfile | railway.toml | rust-toolchain.toml | \
      Cargo.toml | */Cargo.toml | Cargo.lock | go.mod | */go.mod | go.sum | */go.sum | \
      package.json | */package.json | package-lock.json | */package-lock.json | \
      Package.swift | */Package.swift | Package.resolved | */Package.resolved | \
      ios/project.yml | ios/*.xcodeproj/* | *.entitlements)
      echo high
      ;;
    docs/* | *.md | fixtures/* | web/public/*.css | web/public/*.html)
      echo low
      ;;
    *)
      if is_test_path "$1"; then echo low; else echo standard; fi
      ;;
  esac
}

is_test_path() {
  case "$1" in
    */tests/* | tests/* | */tests.rs | *_tests.rs | */*_tests/* | *_test.go | *.test.js | \
      *.test.mjs | ios/*Tests/* | ios/Fixtures/*)
      return 0
      ;;
    *) return 1 ;;
  esac
}

is_generated() {
  [ "$(git check-attr linguist-generated -- "$1" | sed 's/.*: //')" = "set" ]
}

# Reads a pulls/{n}/files JSON array and sets TIER and REASONS.
classify() {
  local files_json="$1" status additions deletions file previous marketing tier lines=0
  TIER=low
  REASONS=()
  # Fields are joined with US (0x1f): IFS whitespace like tab would merge the
  # empty previous_filename into its neighbour.
  while IFS=$'\x1f' read -r status additions deletions file previous marketing; do
    for path in "$file" "$previous"; do
      [ -z "$path" ] && continue
      tier=$(tier_for_path "$path")
      if [ "$tier" = high ]; then
        TIER=high
        REASONS+=("high-risk path: $path")
      elif [ "$tier" = standard ] && [ "$TIER" = low ]; then
        TIER=standard
      fi
    done
    if [ "$status" = removed ] && is_test_path "$file"; then
      TIER=high
      REASONS+=("deletes a test: $file")
    fi
    if [ "$marketing" = 1 ]; then
      TIER=high
      REASONS+=("changes MARKETING_VERSION: $file")
    fi
    if ! is_generated "$file"; then
      lines=$((lines + additions + deletions))
    fi
  done < <(jq -r '.[] | [
      .status, .additions, .deletions, .filename, (.previous_filename // ""),
      (if ((.patch // "") | test("(?m)^[-+].*MARKETING_VERSION")) then 1 else 0 end)
    ] | map(tostring) | join("\u001f")' "$files_json")
  if [ "$lines" -gt "$MAX_LINES" ]; then
    TIER=high
    REASONS+=("changes $lines lines (limit $MAX_LINES, generated files excluded)")
  fi
}

# Reads an issues/{n}/comments JSON array and prints one line per comment that
# starts with /approve: its id, then "ok" or why it does not count.
approvals() {
  local comments_json="$1" head="$2" approvers="$3"
  jq -r --arg head "$head" --arg approvers "$approvers" '
    ($approvers | split(",")) as $allowed
    | .[]
    | select((.body // "") | startswith("/approve"))
    | (((.body | split("\n")[0] | rtrimstr("\r")
        | capture("^/approve\\s+(?<sha>[0-9a-fA-F]{7,40})\\s*$"))? // {sha: ""}).sha | ascii_downcase) as $sha
    | [ .id,
        if (.user.login as $u | $allowed | index($u)) == null then "not from an approver (\(.user.login))"
        elif .performed_via_github_app != null then "posted through the \(.performed_via_github_app.slug) app, not by hand"
        elif .created_at != .updated_at then "edited after posting; post a new comment"
        elif $sha == "" then "first line is not /approve <head sha>"
        elif ($head | startswith($sha)) | not then "names \($sha), not the head commit \($head[0:7])"
        else "ok" end
      ] | @tsv' "$comments_json"
}

selftest() {
  local failed=0
  expect_tier() {
    local got
    got=$(tier_for_path "$1")
    if [ "$got" != "$2" ]; then
      echo "error: risk tier mismatch for $1 (expected $2, got $got)"
      failed=1
    fi
  }
  expect_tier ".github/workflows/ci.yml" high
  expect_tier ".github/scripts/pr-risk.sh" high
  expect_tier ".github/pull_request_template.md" high
  expect_tier ".claude/hooks/session-start.sh" high
  expect_tier "lefthook.yml" high
  expect_tier "scripts/check-file-size.sh" high
  expect_tier "Makefile" high
  expect_tier "AGENTS.md" high
  expect_tier "CLAUDE.md" high
  expect_tier "REVIEW.md" high
  expect_tier "src/web/auth.rs" high
  expect_tier "src/web/oauth/token.rs" high
  expect_tier "src/oauth.rs" high
  expect_tier "src/access.rs" high
  expect_tier "src/log_privacy.rs" high
  expect_tier "src/push.rs" high
  expect_tier "src/push/tests.rs" high
  expect_tier "src/storage/schema.rs" high
  expect_tier "src/storage/schema/v12.sql" high
  expect_tier "fixtures/api/responses/chats.json" high
  expect_tier "Dockerfile" high
  expect_tier "railway.toml" high
  expect_tier "rust-toolchain.toml" high
  expect_tier "Cargo.toml" high
  expect_tier "Cargo.lock" high
  expect_tier "wa-bridge/go.mod" high
  expect_tier "wa-bridge/go.sum" high
  expect_tier "web/package.json" high
  expect_tier "web/package-lock.json" high
  expect_tier "ios/project.yml" high
  expect_tier "ios/WhatsAppTranslator.xcodeproj/project.pbxproj" high
  expect_tier "ios/BabelBridgeMac/BabelBridgeMac.entitlements" high
  expect_tier "docs/index.html" low
  expect_tier "README.md" low
  expect_tier "ios/README.md" low
  expect_tier "fixtures/bridge/message.json" low
  expect_tier "web/public/styles-base.css" low
  expect_tier "web/public/index.html" low
  expect_tier "web/tests/topics.test.js" low
  expect_tier "web/read-receipts.test.js" low
  expect_tier "web/extensions/tests/chats.test.mjs" low
  expect_tier "wa-bridge/edits_test.go" low
  expect_tier "src/chatgpt/tests.rs" low
  expect_tier "src/identity_tests.rs" low
  expect_tier "src/web/tests/chats.rs" low
  expect_tier "src/voice/integration_tests/ogg.rs" low
  expect_tier "ios/WhatsAppTranslatorTests/FoldableLayoutTests.swift" low
  expect_tier "src/main.rs" standard
  expect_tier "src/storage/messages.rs" standard
  expect_tier "src/web/chats.rs" standard
  expect_tier "wa-bridge/main.go" standard
  expect_tier "web/public/app.js" standard
  expect_tier "web/public/markup/chat-shell.js" standard
  expect_tier "web/public/sounds/tone.wav" standard
  expect_tier "ios/WhatsAppTranslator/App/AppSession.swift" standard
  expect_tier "scripts/generate-message-tones.py" standard
  expect_tier "build.rs" standard

  local tmp
  tmp=$(mktemp -d)
  expect_files() {
    printf '%s' "$2" > "$tmp/files.json"
    classify "$tmp/files.json"
    if [ "$TIER" != "$3" ]; then
      echo "error: $1 classified $TIER, expected $3 (${REASONS[*]:-no reasons})"
      failed=1
    fi
  }
  expect_files "docs only" '[{"filename":"docs/a.md","status":"modified","additions":5,"deletions":1}]' low
  expect_files "docs and code" '[{"filename":"docs/a.md","status":"modified","additions":5,"deletions":1},
    {"filename":"src/main.rs","status":"modified","additions":5,"deletions":1}]' standard
  expect_files "deleted test" '[{"filename":"web/tests/topics.test.js","status":"removed","additions":0,"deletions":40}]' high
  expect_files "renamed test" '[{"filename":"web/tests/b.test.js","previous_filename":"web/tests/a.test.js","status":"renamed","additions":0,"deletions":0}]' low
  expect_files "moved out of a high path" '[{"filename":"src/auth_old.rs","previous_filename":"src/web/auth.rs","status":"renamed","additions":0,"deletions":0}]' high
  expect_files "large change" '[{"filename":"src/main.rs","status":"modified","additions":700,"deletions":101}]' high
  expect_files "large generated file" '[{"filename":"web/extensions/dist/chats.html","status":"modified","additions":900,"deletions":900}]' standard
  expect_files "version bump" '[{"filename":"ios/WhatsAppTranslator/Info.plist","status":"modified","additions":1,"deletions":1,
    "patch":"@@ -1 +1 @@\n-MARKETING_VERSION = 1.0.2\n+MARKETING_VERSION = 1.0.3"}]' high

  local head=0123456789abcdef0123456789abcdef01234567 base='"created_at":"t","updated_at":"t","performed_via_github_app":null'
  expect_approval() {
    local got
    printf '[%s]' "$2" > "$tmp/comments.json"
    got=$(approvals "$tmp/comments.json" "$head" vultuk | cut -f2)
    if [ -z "$3" ] && [ -n "$got" ] || [ -n "$3" ] && [ "${got#"$3"}" = "$got" ]; then
      echo "error: approval '$1' gave '$got', expected '$3'"
      failed=1
    fi
  }
  expect_approval "full sha" "{\"id\":1,\"body\":\"/approve $head\",\"user\":{\"login\":\"vultuk\"},$base}" ok
  expect_approval "short sha" "{\"id\":1,\"body\":\"/approve 0123456\\r\\nlooks fine\",\"user\":{\"login\":\"vultuk\"},$base}" ok
  expect_approval "old sha" "{\"id\":1,\"body\":\"/approve fedcba9\",\"user\":{\"login\":\"vultuk\"},$base}" "names fedcba9"
  expect_approval "no sha" "{\"id\":1,\"body\":\"/approve\",\"user\":{\"login\":\"vultuk\"},$base}" "first line"
  expect_approval "other user" "{\"id\":1,\"body\":\"/approve $head\",\"user\":{\"login\":\"someone\"},$base}" "not from an approver"
  expect_approval "via app" "{\"id\":1,\"body\":\"/approve $head\",\"user\":{\"login\":\"vultuk\"},\"created_at\":\"t\",\"updated_at\":\"t\",\"performed_via_github_app\":{\"slug\":\"claude\"}}" "posted through the claude app"
  expect_approval "edited" "{\"id\":1,\"body\":\"/approve $head\",\"user\":{\"login\":\"vultuk\"},\"created_at\":\"t\",\"updated_at\":\"u\",\"performed_via_github_app\":null}" "edited"
  expect_approval "other comment" "{\"id\":1,\"body\":\"looks good\",\"user\":{\"login\":\"vultuk\"},$base}" ""
  rm -rf "$tmp"

  if [ "$failed" -ne 0 ]; then
    exit 1
  fi
  echo "risk rules self-test passed"
}

# Prints N when the PR's head branch is revert/pr-N and its diff exactly undoes
# merged PR N's commit on the default branch, so a revert of a high-risk PR
# can merge without waiting for /approve. Needs the full default-branch history.
clean_revert_of() {
  local ref n merge base ours theirs
  ref=$(jq -r .head.ref "$WORK/pr.json")
  [[ $ref =~ ^revert/pr-([0-9]+)$ ]] || return 1
  n=${BASH_REMATCH[1]}
  [ "$(jq -r .head.repo.full_name "$WORK/pr.json")" = "$REPO" ] || return 1
  gh api "repos/$REPO/pulls/$n" > "$WORK/reverted.json" || return 1
  [ "$(jq -r .merged "$WORK/reverted.json")" = true ] || return 1
  merge=$(jq -r .merge_commit_sha "$WORK/reverted.json")
  base=$(jq -r .base.sha "$WORK/pr.json")
  git -c "http.extraheader=AUTHORIZATION: basic $(printf 'x-access-token:%s' "$GH_TOKEN" | base64 -w0)" \
    fetch -q --no-tags origin "$HEAD_SHA" || return 1
  git merge-base --is-ancestor "$merge" "$base" || return 1
  base=$(git merge-base "$base" "$HEAD_SHA") || return 1
  ours=$(git diff --binary "$base" "$HEAD_SHA" | git patch-id --verbatim | cut -d' ' -f1)
  theirs=$(git diff --binary "$merge" "$merge^" | git patch-id --verbatim | cut -d' ' -f1)
  # --verbatim keeps whitespace, which --stable would let an edit hide in.
  [ -n "$ours" ] && [ "$ours" = "$theirs" ] || return 1
  echo "$n"
}

post_status() {
  gh api "repos/$REPO/statuses/$HEAD_SHA" -f state="$1" -f context=risk \
    -f description="${2:0:140}" -f target_url="${RUN_URL:-}" > /dev/null
}

set_label() {
  local want="risk:$1" color description label
  case "$1" in
    low) color=0e8a16 description="Docs, tests or styles only" ;;
    standard) color=fbca04 description="Merges when checks and review-gate pass" ;;
    high) color=d93f0b description="Needs /approve <head sha> from the owner" ;;
  esac
  gh api "repos/$REPO/labels" -f name="$want" -f color="$color" -f description="$description" \
    > /dev/null 2>&1 || true
  for label in $(jq -r '.labels[].name | select(startswith("risk:"))' "$WORK/pr.json"); do
    if [ "$label" != "$want" ]; then
      gh api -X DELETE "repos/$REPO/issues/$PR_NUMBER/labels/${label/:/%3A}" > /dev/null ||
        echo "::warning::could not remove $label"
    fi
  done
  gh api "repos/$REPO/issues/$PR_NUMBER/labels" -f "labels[]=$want" > /dev/null ||
    echo "::warning::could not add $want"
}

react() {
  [ -n "${COMMENT_ID:-}" ] || return 0
  gh api "repos/$REPO/issues/comments/$COMMENT_ID/reactions" -f content="$1" > /dev/null ||
    echo "::warning::could not react to comment $COMMENT_ID"
}

run() {
  : "${REPO:?}" "${PR_NUMBER:?}" "${APPROVERS:?}"
  WORK=$(mktemp -d)
  gh api "repos/$REPO/pulls/$PR_NUMBER" > "$WORK/pr.json"
  if [ "$(jq -r .state "$WORK/pr.json")" != open ]; then
    echo "PR #$PR_NUMBER is not open; nothing to do."
    return 0
  fi
  HEAD_SHA=$(jq -r .head.sha "$WORK/pr.json")
  gh api --paginate "repos/$REPO/pulls/$PR_NUMBER/files?per_page=100" | jq -s 'add // []' > "$WORK/files.json"
  gh api --paginate "repos/$REPO/issues/$PR_NUMBER/comments?per_page=100" | jq -s 'add // []' > "$WORK/comments.json"

  classify "$WORK/files.json"
  if [ "$(jq length "$WORK/files.json")" -ge 3000 ]; then
    TIER=high
    REASONS+=("3000 or more files changed")
  fi
  local reverted
  if [ "$TIER" != low ] && reverted=$(clean_revert_of); then
    TIER=low
    REASONS=("clean revert of #$reverted")
  fi
  echo "PR #$PR_NUMBER at $HEAD_SHA is risk:$TIER"
  printf '  %s\n' "${REASONS[@]}"
  set_label "$TIER"

  local approved="" id verdict
  while IFS=$'\t' read -r id verdict; do
    [ -z "$id" ] && continue
    echo "/approve comment $id: $verdict"
    if [ "$verdict" = ok ]; then
      approved=$id
    fi
    if [ "$id" = "${COMMENT_ID:-}" ]; then
      if [ "$verdict" = ok ]; then react "+1"; else react confused; fi
    fi
  done < <(approvals "$WORK/comments.json" "$HEAD_SHA" "$APPROVERS")

  local summary="${REASONS[0]:-}"
  if [ "${#REASONS[@]}" -gt 1 ]; then
    summary="$summary (+$((${#REASONS[@]} - 1)) more)"
  fi
  {
    echo "## risk:$TIER"
    echo
    printf -- '- %s\n' "${REASONS[@]}"
  } >> "${GITHUB_STEP_SUMMARY:-/dev/null}"

  if [ "$TIER" != high ]; then
    post_status success "risk:$TIER"
  elif [ -n "$approved" ]; then
    post_status success "risk:high, approved for ${HEAD_SHA:0:7} by comment $approved"
  else
    post_status failure "Needs /approve ${HEAD_SHA:0:7}. $summary"
  fi
}

case "${1:-}" in
  selftest) selftest ;;
  run) run ;;
  *)
    echo "usage: $0 selftest|run" >&2
    exit 2
    ;;
esac
