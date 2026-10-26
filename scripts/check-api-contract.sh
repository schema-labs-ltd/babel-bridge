#!/usr/bin/env bash
# Decodes fixtures/api with the app's APIModels*.swift. Needs a Swift 6 toolchain
# (Xcode on macOS, or swift.org's Linux release). The Rust test `api_contract`
# writes the response fixtures; see src/api_contract.rs.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if ! command -v swiftc >/dev/null 2>&1; then
  echo "error: swiftc not found; install Swift 6 from https://www.swift.org/install/" >&2
  exit 1
fi

build="$(mktemp -d)"
trap 'rm -rf "$build"' EXIT
models="$root/ios/WhatsAppTranslator/Models"
swiftc -swift-version 6 -parse-as-library \
  "$models"/APIModels*.swift \
  "$models/MessageTone.swift" \
  "$root/ios/APIContract/APIContractCheck.swift" \
  -o "$build/api-contract-check"
"$build/api-contract-check" "$root/fixtures/api"
