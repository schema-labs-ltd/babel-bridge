#!/bin/sh
# Xcode Cloud post-clone hook.
# Installs a pinned XcodeGen release and regenerates apps/ios/WhatsAppTranslator.xcodeproj
# from apps/ios/project.yml. Drift is reported and does not fail the build.
# Do not trace commands or print the environment.
set -eu

XCODEGEN_VERSION=2.45.4

IOS_DIR="${CI_PRIMARY_REPOSITORY_PATH}/apps/ios"
WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

curl -fsSL -o "$WORKDIR/xcodegen.zip" \
  "https://github.com/yonaskolb/XcodeGen/releases/download/${XCODEGEN_VERSION}/xcodegen.zip"
unzip -q "$WORKDIR/xcodegen.zip" -d "$WORKDIR"

cd "$IOS_DIR"
"$WORKDIR/xcodegen/bin/xcodegen" generate

if ! git diff --quiet -- WhatsAppTranslator.xcodeproj; then
  echo "warning: committed WhatsAppTranslator.xcodeproj differs from XcodeGen ${XCODEGEN_VERSION}"
  git diff --stat -- WhatsAppTranslator.xcodeproj
fi
