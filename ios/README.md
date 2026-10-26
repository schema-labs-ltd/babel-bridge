# Babel Bridge for Apple platforms

Translation is off by default for every person and group. Use **Conversation settings → Translate messages** to enable a chat. The same server-saved choice applies in Chats and unified Messages on iOS, iPadOS, macOS, and web; long-press or right-click a Messages conversation heading to open its settings. Disabled chats send original text, captions, and voice recordings without waiting for translation AI. Language, style, and original-follow-up choices are preserved when switched off.

Native SwiftUI clients for the Babel Bridge translation backend. The Xcode
project contains separate iOS and macOS app targets that share the messaging,
translation, caching and API layers while using each platform's native app
lifecycle and interaction patterns.

## Message ringtones

Open **Settings → Notifications → Message ringtone** for the global choice,
or a chat’s **Conversation settings → Notifications → Message ringtone** for
an override. Aurora, Bamboo, Bloom, Droplet, Glass and Orbit are original,
bundled notification tones. Selecting a tone previews it; the separate play
button previews without changing the selection. **Save** commits the choice,
while **Cancel** leaves the saved setting unchanged. Conversation choices include
**Use global ringtone**, **System default**, and **Silent**. Silent keeps visible
notifications and badges without requesting audio.

Choices are stored on the connected server, shared across native and web
clients, and applied to the APNs payload before delivery, including when the app
is closed. Deploy the matching `/api/settings/message-tone` backend before
installing native build 55. Older app builds fall back to the system sound if
they do not contain the selected custom asset. Normal system sound permissions,
silent mode, Focus, and Watch/CarPlay audio policies still apply.

The canonical sound assets and catalog are in `web/public/sounds`; Xcode copies
them into the iOS app, notification extension, and macOS app. Re-render the
original 16-bit mono PCM WAVs with `scripts/generate-message-tones.py`. All clips
are under two seconds. Shared native tests decode and play every bundled clip,
and backend tests verify persistence, inheritance, exact conversation routing,
validation, and silent APNs payloads.

## Generate and open

The committed `WhatsAppTranslator.xcodeproj` is generated from `project.yml`.
Xcode Cloud regenerates it after clone with `ios/ci_scripts/ci_post_clone.sh`,
which installs XcodeGen 2.45.4. Regenerate locally with that same release so the
project stays aligned with CI:

```bash
WORKDIR=$(mktemp -d)
curl -fsSL -o "$WORKDIR/xcodegen.zip" \
  https://github.com/yonaskolb/XcodeGen/releases/download/2.45.4/xcodegen.zip
unzip -q "$WORKDIR/xcodegen.zip" -d "$WORKDIR"
cd ios
"$WORKDIR/xcodegen/bin/xcodegen" generate
open WhatsAppTranslator.xcodeproj
```

Use the `WhatsAppTranslator` scheme for iPhone and iPad, or the
`BabelBridgeMac` scheme for the native Mac app. The Mac target is built with the
macOS SDK and SwiftUI/AppKit; it is not Mac Catalyst or an iPad compatibility
build.

`WhatsAppTranslator` still runs unit tests and `ConversationTranslationUITests`.
`WhatsAppTranslatorUnitTests` runs `WhatsAppTranslatorTests` only.
`BabelBridgeMac` runs `BabelBridgeMacTests` only. `BabelBridgeWatch` runs the
watch UI tests.

## Demo launch arguments

Debug builds accept launch arguments that load canned data instead of
connecting to a server, which is the quickest way to reach a screen in the
simulator, a preview, or a UI test. Add them under **Edit Scheme → Run →
Arguments Passed On Launch**, or pass them in `XCUIApplication().launchArguments`.
Most need `-demo` as well, since `-demo` is what switches the session to demo
data. They are read with `ProcessInfo.processInfo.arguments` in the files named
below, so search for the argument to find what it does.

| Argument | Shows |
| --- | --- |
| `-demo` | Demo mode: canned contacts and messages, no server. CarPlay and the watch app read it too. |
| `-demoConversation` | Opens the first chat in the Chats tab. |
| `-demoSending` | With `-demoConversation`, shows that chat as sending. |
| `-demoOnboarding` | The server setup screen. Does not need `-demo`. |
| `-demoRelinking` | Two seconds after start, the WhatsApp link drops and the QR relinking flow appears. |
| `-demoChatList` | Twenty group chats in the chat list. |
| `-demoReadReceipts` | One group with 40 unread messages on the Messages tab, with a banner of the IDs counted as viewed. |
| `-demoUnifiedFeed` | Three chats with recent messages for the unified Messages view. |
| `-demoTopics` | Two groups with topic grouping (Weekend plans, Football). |
| `-demoOutgoingTopic` | With `-demoTopics`, makes the last Football message your own. |
| `-demoWhatsAppLayout` | A short one-to-one chat with a tall photo, in light mode, for comparing layout with WhatsApp. |
| `-demoPhotoAlbum` | With `-demoWhatsAppLayout`, replaces the photo with a four-photo album. |
| `-demoPhotoGallery` | The photo gallery with demo photos. |
| `-demoChatGallery` | The per-chat gallery with demo media. |
| `-demoVideo` | A chat with a playable video (Debug only). |
| `-demoLiveReactions` | Reactions arriving live on a message. |
| `-demoLiveEdits` | A message being edited live. |
| `-demoSettings` | The chat list with Settings open. |
| `-demoConversationSettings` | Conversation settings open when a conversation opens (pair with `-demoConversation`). The rows below that start in a conversation work the same way. |
| `-demoTimezonePicker` | The time zone picker open inside conversation settings (pair with `-demoConversation` and `-demoConversationSettings`). |
| `-demoCost` | A conversation with the translation cost sheet open. |
| `-demoSearch` | A conversation with search open. |
| `-demoActions` | The actions menu open on demo message 4. |
| `-demoSwipeReplyReveal` | Demo message 4 swiped to the reply threshold. |
| `-demoImageComposer` | The composer with one photo attached. |
| `-demoImageAlbumComposer` | The composer with four photos attached. |
| `-demoOptimizedPhotoComposer` | Currently the same as `-demoImageComposer`: one photo attached. |
| `-demoPhotoSendProgress` | A photo send in its sending stage. |
| `-demoPhotoTransferProgress` | A photo send in its transfer stage. |
| `-demoClipboardText` | Puts two lines of text on the clipboard, for paste tests. |
| `-demoClipboardImage` | Puts two images on the clipboard, for paste tests. |
| `-demoNotification` | Posts a sample message notification after asking for permission. |
| `-demoAppIcons` | With `-demo`, shows the alternate app icon picker. |

The data comes from `AppSession.loadDemoData()` in
`ios/WhatsAppTranslator/App/AppSession.swift`; the view-level flags are in the
conversation, chat list, settings, and root views.

## GitHub Actions

Pull requests and pushes to `main` run `.github/workflows/apple.yml`. The check
to require is `apple / apple`. It passes without a build when the diff does not
touch `ios/`, `web/public/sounds/`, or that workflow file. When those paths
change, the `macos-26` runner selects Xcode 26.6 (Swift 6), installs XcodeGen
2.45.4, and regenerates the project. The job fails if
`WhatsAppTranslator.xcodeproj` differs from `project.yml`. It builds
`WhatsAppTranslator` for the iPhone 17 simulator on iOS 26.5, including the
embedded watch app and extensions, and builds `BabelBridgeMac` for macOS arm64.
Signing is disabled. Tests are `WhatsAppTranslatorUnitTests` and
`BabelBridgeMacTests` only. A generated project that differs from the committed
`WhatsAppTranslator.xcodeproj` fails this check. The Xcode Cloud hook prints a
warning for the same drift and continues.

From a machine with Xcode 26.6, the same build and test is:

```bash
export DEVELOPER_DIR=/Applications/Xcode_26.6.app/Contents/Developer
cd ios
# Install XcodeGen 2.45.4 and generate, using the commands in the section above.
git diff --exit-code -- WhatsAppTranslator.xcodeproj

xcodebuild build \
  -project WhatsAppTranslator.xcodeproj \
  -scheme WhatsAppTranslator \
  -configuration Debug \
  -destination 'platform=iOS Simulator,name=iPhone 17,OS=26.5' \
  CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY= \
  DEVELOPMENT_TEAM= CODE_SIGN_STYLE=Manual CODE_SIGN_ENTITLEMENTS=

xcodebuild test \
  -project WhatsAppTranslator.xcodeproj \
  -scheme WhatsAppTranslatorUnitTests \
  -configuration Debug \
  -destination 'platform=iOS Simulator,name=iPhone 17,OS=26.5' \
  -only-testing:WhatsAppTranslatorTests \
  CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY= \
  DEVELOPMENT_TEAM= CODE_SIGN_STYLE=Manual CODE_SIGN_ENTITLEMENTS=

xcodebuild build \
  -project WhatsAppTranslator.xcodeproj \
  -scheme BabelBridgeMac \
  -configuration Debug \
  -destination 'platform=macOS,arch=arm64' \
  ENABLE_HARDENED_RUNTIME=NO \
  CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY= \
  DEVELOPMENT_TEAM= CODE_SIGN_STYLE=Manual CODE_SIGN_ENTITLEMENTS=

xcodebuild test \
  -project WhatsAppTranslator.xcodeproj \
  -scheme BabelBridgeMac \
  -configuration Debug \
  -destination 'platform=macOS,arch=arm64' \
  -only-testing:BabelBridgeMacTests \
  ENABLE_HARDENED_RUNTIME=NO \
  CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY= \
  DEVELOPMENT_TEAM= CODE_SIGN_STYLE=Manual CODE_SIGN_ENTITLEMENTS=
```

The macOS commands turn the hardened runtime off for that unsigned run. The
project setting stays on for signed archives.

## Xcode Cloud

`ios/ci_scripts/ci_post_clone.sh` is the repo-side hook. It downloads XcodeGen
2.45.4 from the GitHub release zip, runs `xcodegen generate` in `ios/`, and
prints a warning plus a diff stat if the committed
`WhatsAppTranslator.xcodeproj` differs. The warning does not fail the build.
Generation failure does. The script does not print the environment.

`ios/ci_scripts/ci_post_xcodebuild.sh` fills in the TestFlight **What to Test**
notes. After each archive it writes `ios/TestFlight/WhatToTest.en-US.txt`,
which Xcode Cloud attaches to the build it sends to TestFlight. The notes lead
with the subject and first body paragraph of the newest commit that touches
`ios/` or `web/public/sounds/`, then list the subjects of the four app commits
before it, then the short commit hash. Pull request numbers are dropped from
subjects, so write pull request titles and the first paragraph of the
description for testers. The file is generated per build and ignored by git.
The script never fails the build.

Pull-request builds and unit tests are the GitHub Actions check `apple / apple`.
Xcode Cloud keeps workflow B, **Main – TestFlight (internal)**. This repository
does not define that workflow. Simon creates it in Xcode. These steps are for
workflow B only. Do not create workflow A (`PR – Build & Test`).

**Main – TestFlight (internal)** runs on merges to `main` that touch `ios/` or
`web/public/sounds/`. It archives the iOS and macOS apps and sends them to the
internal TestFlight group. Xcode Cloud assigns the build number.
`CURRENT_PROJECT_VERSION` in `project.yml` stays as the local value and is not
bumped for cloud builds. `MARKETING_VERSION` is still changed on purpose for a
new version. Signing stays with Xcode Cloud managed signing.

One-time setup for workflow B, done in Xcode and App Store Connect:

1. Start Xcode Cloud from the Report navigator (Cloud, then Get Started) and
   grant the Xcode Cloud GitHub app access to this repository.
2. Confirm an internal TestFlight tester group exists.
3. Note the highest build already uploaded for iOS and for macOS, then set Next
   Build Number to one above that highest build before workflow B runs for the
   first time. That keeps the cloud build number from colliding with an
   existing upload.
4. Create **Main – TestFlight (internal)** on a released Xcode 26 environment.
   Archive both apps and upload them to the internal group.

The Apple Developer Program includes 25 Xcode Cloud hours a month. Put
`[ci skip]` in the commit message for work-in-progress pushes that do not need
a cloud build.

The first-run screen asks for the backend web address and password. These are
stored in the system Keychain. The client authenticates against `/api/auth`,
uses the bearer-protected REST API, and receives live updates over the
authenticated `/ws` endpoint.

## More details

How the apps behave on particular devices and screens is described in [docs/apple](../docs/apple/):

- [iPhone Duo](../docs/apple/iphone-duo.md): outer and inner display layouts, postures and the hinge
- [Native Mac app](../docs/apple/mac-app.md)
- [Messages and media](../docs/apple/messages-and-media.md): the unified Messages view, compact photo galleries, video playback and feed attachments
