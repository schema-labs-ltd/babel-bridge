#!/bin/bash
# Prepares a Claude Code cloud session so `make check` runs without first-use
# downloads: ffmpeg for the Rust tests, web/ dependencies, Playwright's
# Chromium, crates, and the Go toolchain and modules that wa-bridge/go.mod
# pins. Local sessions are left alone.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel)}"

if ! command -v ffmpeg >/dev/null 2>&1; then
  echo "Installing ffmpeg for the Rust tests"
  if [ "$(id -u)" = 0 ]; then sudo_cmd=""; else sudo_cmd="sudo -n"; fi
  $sudo_cmd apt-get update -qq && $sudo_cmd apt-get install -y -qq ffmpeg >/dev/null \
    || echo "warning: could not install ffmpeg; cargo test will fail until it is installed" >&2
fi

echo "Installing web/ dependencies"
make --no-print-directory web-deps
make --no-print-directory extension-browser \
  || echo "warning: Playwright Chromium is missing; make extension-test will try again" >&2

echo "Fetching Rust crates"
cargo fetch --locked --quiet

echo "Fetching the Go toolchain and modules"
(cd wa-bridge && go mod download)

echo "Session ready. Run 'make check' before pushing, or 'make check-fast' while iterating."
