#!/bin/sh
# Xcode Cloud post-xcodebuild hook.
# After an archive, writes apps/ios/TestFlight/WhatToTest.en-US.txt from git history
# so TestFlight shows "What to Test" notes for the build. The newest commit that
# touches the app paths leads, with its first body paragraph, followed by the
# subjects of the commits before it. Problems are reported and never fail the
# build. Do not trace commands or print the environment.
set -u

if [ "${CI_XCODEBUILD_ACTION:-}" != "archive" ]; then
  exit 0
fi

REPO="${CI_PRIMARY_REPOSITORY_PATH:-}"
if [ -z "$REPO" ] || [ ! -e "$REPO/.git" ]; then
  echo "warning: no repository checkout; skipping What to Test notes"
  exit 0
fi

OUT_DIR="$REPO/apps/ios/TestFlight"
OUT_FILE="$OUT_DIR/WhatToTest.en-US.txt"
# Same paths that start the Main – TestFlight (internal) workflow.
APP_PATHS="apps/ios apps/web/public/sounds"
EARLIER_COUNT=4
# TestFlight accepts up to 4000 characters.
MAX_BYTES=3900

cd "$REPO" || exit 0

if [ "$(git rev-parse --is-shallow-repository 2>/dev/null)" = "true" ]; then
  git fetch --quiet --deepen=50 origin >/dev/null 2>&1 \
    || echo "warning: shallow clone could not be deepened; notes may be short"
fi

# shellcheck disable=SC2086
commits=$(git log -n $((EARLIER_COUNT + 1)) --format=%H HEAD -- $APP_PATHS 2>/dev/null)
if [ -z "$commits" ]; then
  commits=$(git rev-parse HEAD 2>/dev/null)
fi
if [ -z "$commits" ]; then
  echo "warning: no commits found; skipping What to Test notes"
  exit 0
fi

# Drops a trailing pull request number such as " (#70)".
subject_of() {
  git log -1 --format=%s "$1" | sed -E 's/[[:space:]]*\(#[0-9]+\)$//'
}

# First body paragraph, unwrapped onto one line, unless it is only trailers.
summary_of() {
  git log -1 --format=%b "$1" | awk '
    /^[[:space:]]*$/ { if (text != "") exit; next }
    {
      line = $0
      sub(/^[[:space:]]*\*[[:space:]]+/, "", line)
      if (line !~ /^[A-Z][A-Za-z]*(-[A-Za-z]+)*: /) trailers_only = 0
      else if (text == "") trailers_only = 1
      text = (text == "") ? line : text " " line
    }
    END { if (text != "" && !trailers_only) print text }
  '
}

latest=$(printf '%s\n' "$commits" | head -n 1)
earlier=$(printf '%s\n' "$commits" | sed '1d')

notes=$(
  subject_of "$latest"
  summary=$(summary_of "$latest")
  if [ -n "$summary" ]; then
    echo
    echo "$summary"
  fi
  if [ -n "$earlier" ]; then
    echo
    echo "Earlier changes:"
    for sha in $earlier; do
      echo "- $(subject_of "$sha")"
    done
  fi
  echo
  echo "Commit $(git rev-parse --short "${CI_COMMIT:-HEAD}" 2>/dev/null || echo "${CI_COMMIT:-unknown}")"
)

mkdir -p "$OUT_DIR" || exit 0
printf '%s\n' "$notes" | head -c "$MAX_BYTES" > "$OUT_FILE" || {
  echo "warning: could not write $OUT_FILE"
  exit 0
}
echo "Wrote What to Test notes for build ${CI_BUILD_NUMBER:-unknown}:"
cat "$OUT_FILE"
exit 0
