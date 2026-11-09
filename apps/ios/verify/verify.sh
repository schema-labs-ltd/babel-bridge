#!/usr/bin/env bash
# ios:verify (issue #138, section 6): the Apple apps as they ship.
#
#   archive-ios   Release archive of WhatsAppTranslator, unsigned, with the
#                 embedded watch app, its complications and the three
#                 extensions (Notification Service, Send Activity, Intents)
#   archive-mac   Release archive of BabelBridgeMac, unsigned
#   ui-ios        ConversationTranslationUITests on an iPhone and an iPad
#   ui-watch      BabelBridgeWatchUITests on an Apple Watch
#
# A Release archive is what Xcode Cloud builds for TestFlight, so code that
# compiles only in Debug fails here on the pull request instead of after the
# merge. VERIFY_IOS_PART picks one part (apple.yml runs one per job); unset, it
# runs them all. Needs macOS with Xcode; DEVELOPER_DIR picks the Xcode.
set -euo pipefail

root=$(cd "$(dirname "$0")/../../.." && pwd)
project="${root}/apps/ios/WhatsAppTranslator.xcodeproj"
out="${VERIFY_IOS_OUTPUT:-${root}/output/verify/ios}"
derived="${VERIFY_IOS_DERIVED_DATA:-${root}/.deriveddata/verify}"
ios_runtime="${VERIFY_IOS_RUNTIME:-26.5}"
mkdir -p "$out"

# Unsigned, as apple.yml's build job: no team, no profiles, no entitlements.
unsigned=(
  CODE_SIGNING_ALLOWED=NO
  CODE_SIGNING_REQUIRED=NO
  "CODE_SIGN_IDENTITY="
  "DEVELOPMENT_TEAM="
  CODE_SIGN_STYLE=Manual
  "CODE_SIGN_ENTITLEMENTS="
  COMPILER_INDEX_STORE_ENABLE=NO
  ENABLE_PREVIEWS=NO
)

step() { printf '\n▶ %s\n' "$*"; }
fail() {
  printf '✗ %s\n' "$*" >&2
  exit 1
}

# Runs xcodebuild with its log in $out, and prints the errors on failure.
xcb() {
  local name=$1
  shift
  local log="${out}/${name}.log"
  local started=$SECONDS
  set +e
  xcodebuild "$@" >"$log" 2>&1
  local status=$?
  set -e
  if [ "$status" -ne 0 ]; then
    grep -E "error:|fatal error:|\*\* .* FAILED \*\*|Test Case .* failed|failed \(" "$log" | tail -n 80 >&2 || true
    tail -n 30 "$log" >&2
    fail "xcodebuild ${name} failed with status ${status} (log: ${log})"
  fi
  printf '✓ %s (%d s)\n' "$name" "$((SECONDS - started))"
}

# The simulator UDID of the first available device named $2 (or, with $3 set
# to "prefix", whose name starts with $2), on the runtime whose identifier
# contains $1 (such as iOS-26-5).
simulator() {
  xcrun simctl list devices available -j | python3 -c '
import json, sys
runtime, wanted, mode = sys.argv[1], sys.argv[2], sys.argv[3]
matches = (lambda name: name.startswith(wanted)) if mode == "prefix" else (lambda name: name == wanted)
devices = json.load(sys.stdin)["devices"]
for key in sorted(devices, reverse=True):
    if runtime not in key:
        continue
    for device in devices[key]:
        if device["isAvailable"] and matches(device["name"]):
            print(device["udid"], device["name"], key.rsplit(".", 1)[-1], sep="\t")
            sys.exit(0)
sys.exit(1)
' "$1" "$2" "${3:-exact}"
}

# Checks that an archived app bundle exists and holds the given bundles.
expect_bundles() {
  local app=$1
  shift
  [ -d "$app" ] || fail "the archive has no ${app##*/}"
  for bundle in "$@"; do
    [ -d "${app}/${bundle}" ] || fail "${app##*/} does not embed ${bundle}"
    echo "  ${app##*/} embeds ${bundle}"
  done
}

# Fails when the tests ran none, so a filter that matches nothing can't pass.
expect_tests() {
  local name=$1
  local count
  count=$(grep -Eo "Executed [0-9]+ tests?" "${out}/${name}.log" | tail -n 1 | grep -Eo "[0-9]+" || true)
  [ -n "$count" ] && [ "$count" -gt 0 ] || fail "${name} ran no tests"
  echo "  ${count} tests"
}

archive_ios() {
  step "Release archive of WhatsAppTranslator for iOS"
  local archive="${out}/WhatsAppTranslator.xcarchive"
  rm -rf "$archive"
  xcb archive-ios archive \
    -project "$project" -scheme WhatsAppTranslator -configuration Release \
    -destination "generic/platform=iOS" -archivePath "$archive" \
    -derivedDataPath "${derived}/archive-ios" "${unsigned[@]}"
  local app="${archive}/Products/Applications/WhatsAppTranslator.app"
  expect_bundles "$app" \
    PlugIns/NotificationService.appex \
    PlugIns/PhotoSendActivity.appex \
    PlugIns/WhatsAppTranslatorIntents.appex \
    Watch/BabelBridgeWatch.app
  expect_bundles "${app}/Watch/BabelBridgeWatch.app" \
    PlugIns/BabelBridgeWatchComplications.appex
}

archive_mac() {
  step "Release archive of BabelBridgeMac"
  local archive="${out}/BabelBridgeMac.xcarchive"
  rm -rf "$archive"
  xcb archive-mac archive \
    -project "$project" -scheme BabelBridgeMac -configuration Release \
    -destination "generic/platform=macOS" -archivePath "$archive" \
    -derivedDataPath "${derived}/archive-mac" "${unsigned[@]}" \
    ENABLE_HARDENED_RUNTIME=NO
  [ -d "${archive}/Products/Applications/Babel Bridge.app" ] ||
    fail "the archive has no Babel Bridge.app"
  echo "  Babel Bridge.app archived"
}

ui_ios() {
  local runtime="iOS-${ios_runtime//./-}"
  local iphone ipad
  iphone=$(simulator "$runtime" "iPhone 17") || fail "no iPhone 17 simulator on iOS ${ios_runtime}"
  ipad=$(simulator "$runtime" "iPad" prefix) || fail "no iPad simulator on iOS ${ios_runtime}"
  step "Build ConversationTranslationUITests once for the simulator"
  xcb ui-ios-build build-for-testing \
    -project "$project" -scheme WhatsAppTranslator -configuration Debug \
    -destination "generic/platform=iOS Simulator" \
    -derivedDataPath "${derived}/ui-ios" "${unsigned[@]}"
  for device in "$iphone" "$ipad"; do
    local udid=${device%%$'\t'*}
    local label
    label=$(printf '%s' "$device" | cut -f2 | tr -c 'A-Za-z0-9\n' '-')
    step "ConversationTranslationUITests on $(printf '%s' "$device" | cut -f2,3 | tr '\t' ' ')"
    rm -rf "${out}/ui-${label}.xcresult"
    xcb "ui-${label}" test-without-building \
      -project "$project" -scheme WhatsAppTranslator \
      -destination "id=${udid}" -derivedDataPath "${derived}/ui-ios" \
      -only-testing:ConversationTranslationUITests \
      -resultBundlePath "${out}/ui-${label}.xcresult" "${unsigned[@]}"
    expect_tests "ui-${label}"
  done
}

ui_watch() {
  local watch
  watch=$(simulator "watchOS" "Apple Watch" prefix) || fail "no Apple Watch simulator is installed"
  step "BabelBridgeWatchUITests on $(printf '%s' "$watch" | cut -f2,3 | tr '\t' ' ')"
  rm -rf "${out}/ui-watch.xcresult"
  xcb ui-watch test \
    -project "$project" -scheme BabelBridgeWatch -configuration Debug \
    -destination "id=${watch%%$'\t'*}" -derivedDataPath "${derived}/ui-watch" \
    -only-testing:BabelBridgeWatchUITests \
    -resultBundlePath "${out}/ui-watch.xcresult" "${unsigned[@]}"
  expect_tests ui-watch
}

[ "$(uname -s)" = "Darwin" ] || fail "ios:verify needs macOS with Xcode"
xcodebuild -version | head -n 1
started=$SECONDS
part="${VERIFY_IOS_PART:-all}"
case "$part" in
  archive-ios) archive_ios ;;
  archive-mac) archive_mac ;;
  ui-ios) ui_ios ;;
  ui-watch) ui_watch ;;
  all)
    archive_ios
    archive_mac
    ui_ios
    ui_watch
    ;;
  *) fail "unknown VERIFY_IOS_PART: ${part}" ;;
esac
printf '\nios:verify (%s) passed in %d s\n' "$part" "$((SECONDS - started))"
