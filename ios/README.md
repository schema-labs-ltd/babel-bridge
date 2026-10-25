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

## iPhone Duo

Babel Bridge follows Apple's [Designing for iPhone Duo](https://developer.apple.com/design/human-interface-guidelines/designing-for-iphone-duo)
guidance and the [Preparing your app for iPhone Duo](https://developer.apple.com/documentation/technologyoverviews/preparing-your-app-for-iphone-duo)
overview. The same functionality is available on both displays and in every pose;
only the arrangement changes.

Outer display (device closed, compact width):

- Toolbars and the navigation controls move to the side of the display. Every
  toolbar item has a title and a symbol, so the system can present it vertically.
  Messages, Chats, Settings, Updates, Gallery and the topic filter of the
  Messages feed carry a high visibility priority and stay in the bar when it
  compresses. In a conversation, Gallery and the ellipsis menu of secondary
  actions (search, starred filter, topic filter, cost, settings) form their
  own group at the right of the header, split from Messages, Chats and Settings
  by a spacer. In a vertical bar they are leading items, so, like the Messages
  feed's topic filter, they get their own glass button above the capsule of
  destinations instead of joining it.
- Chat headers fade into the wallpaper instead of drawing a bar: iOS 26 blurs
  scrolled content softly under a clear navigation bar, the Messages feed keeps
  its topic filter in the bar rather than under the title, and the conversation
  header in the content sits on a frosted fade that is strongest at the top of
  the display, eases through the header so a scrolled bubble shows only faintly
  behind the name, and dissolves over the row beneath it, as WhatsApp's header
  does. The composer floats in the bottom safe area on both screens, so
  messages scroll behind its glass controls. An active topic shows as a
  clearable pill above the timeline, like the starred filter.
- In the unified Messages feed each chat's heading (avatar, name, date) is a
  pinned section header, so the person being read stays at the top of the feed
  while their messages scroll. The grouping rule is
  `UnifiedFeedSectionBuilder` in `APIModels.swift`, covered by
  `UnifiedFeedSectionTests`.
- The Updates bell keeps its drawn unread count in horizontal bars and switches
  to a system badge in a vertical bar, where only title-and-symbol items appear.
- The conversation header (avatar, name, translation status) lives in the
  content whenever bars are vertical, because a custom principal item never
  joins a vertical bar. Chats keeps its single-pane split view.

Inner display (device open, regular width):

- Chats shows the list and the conversation side by side, like Mail, and the
  system adapts the column widths to the hinge.
- Conversation settings, cost and topic sheets opened from a conversation use a
  trailing presentation placement on iOS 27, so they cover the conversation they
  configure rather than the centre of the display.
- Gallery uses an even number of columns when the device is folded like a book
  and widens the middle spacing into a gutter over the hinge, so no tile is
  split.

Partially open poses (the hinge is an active reserved division):

- Laptop pose (horizontal fold): the timeline reads on the upper pane and ends
  at the fold; the composer, reply preview and photo-send progress rest on the
  lower pane directly under the fold band. Chats and the unified Messages feed
  behave the same way. When the keyboard leaves no usable lower pane the
  ordinary layout returns.
- Book pose (vertical fold): every message bubble stays within one pane, so no
  message runs across the hinge, and the composer sits on the trailing pane.
- Full-screen photos and gallery media fill one pane while their captions and
  controls fill the other, following the fold's direction.

Flat displays, closed devices and every other iPhone, iPad and Mac keep the
layout they have today. Rotation, Split View multitasking and the camera
occlusions are handled by the system containers the app already uses.

Implementation notes:

- `WhatsAppTranslator/Design/FoldableLayoutModel.swift` holds the pure layout
  rules (`FoldDivision`, `FoldAwareChatLayout`). It has no SwiftUI dependency so
  `SharedTests/FoldableLayoutTests.swift` runs in both unit-test targets.
- `WhatsAppTranslator/Design/FoldableDisplaySupport.swift` wraps the iOS 27 and
  27.1 APIs: reserved regions through `GeometryProxy`, `toolbarVerticalEdge`,
  `visibilityPriority` and `presentationPlacement`. Each
  use is behind `#if compiler(>=6.4)` (Swift 6.4 ships with Xcode 27) plus a
  runtime availability check, so Xcode 26.6 compiles the same sources without
  any iPhone Duo behaviour and the deployment target stays iOS 18.
- Build with Xcode 27.1 or later to get the iPhone Duo behaviour. Apple notes
  that apps built with Xcode 26 and earlier do not extend under the status bar
  and camera on iPhone Duo, so point the Xcode Cloud workflow at a released
  Xcode 27.1 environment before shipping. The GitHub Actions check stays on
  Xcode 26.6 until the `macos-26` runner image provides Xcode 27.1; the guarded
  code paths are therefore only compiled by Xcode 27.1 builds.

Validation checklist for a Duo simulator (Xcode 27.1 Device Hub poses) or device:

- Closed, portrait and landscape: vertical bar shows Messages, Chats, Settings,
  Updates (badge), Gallery, the conversation ellipsis menu and the Messages
  feed's topic filter; the ellipsis menu lists the conversation actions, including Filter by topic;
  the conversation header is in the content.
- Open flat, portrait and landscape: two-column Chats; trailing conversation
  settings and cost sheets; Gallery grid with four or six columns.
- Book pose: bubbles and the composer stay off the hinge in Chats and
  Messages; Gallery shows a gutter over the hinge; a photo shows on the left
  pane with its actions on the right.
- Laptop pose: the timeline ends at the fold and the composer sits just below
  it; opening the keyboard restores the ordinary layout; media viewers put the
  photo or video above the fold and the controls below.
- Transitions: open and close during a conversation with a draft, a focused
  reply and a photo send in progress; nothing is lost and no control disappears.

This change was prepared without an iPhone Duo simulator or device, so the
checklist above still needs to be run and recorded.

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

## Unified Messages view

Messages is the default tab on iPhone, iPad and Mac. It combines private and group
conversations in chronological order, with a chat label on every message and
cursor-based loading of earlier history. Opening the feed does not mark every
chat as read. Chats retains the individual conversation view and its full media
composer.

Swipe or use a message's Reply action to select the destination for a text reply.
The composer names that chat and keeps a separate draft for each destination.
The backend validates the selected message belongs to the destination, then checks
again after translation: the latest message in that chat receives an ordinary
send; an older message receives a quoted reply. Activity in other chats does not
change the decision. The existing durable send route handles retries.

The feed uses authenticated `GET /api/feed?limit=50&before=…&before_id=…` and
`POST /api/send` with `replyOnlyIfNotLatest: true`. Ordinary chat replies retain
their existing explicit quote behavior. Use `-demo -demoUnifiedFeed` in Xcode's
Run arguments to inspect a synthetic mixed-chat example.

## Compact photo galleries

Consecutive photos from the same sender in the same conversation share a compact
gallery in both Messages and Chats, including photos without WhatsApp album metadata.
A different sender, chat, non-photo message, or calendar day starts a new run.
Search and starred filters retain the original conversation boundaries.

The feed shows at most four thumbnails and a remaining-photo count. Open any tile
to browse every image with swipe or Previous/Next controls, zoom, translated and
original captions, reactions, and actions for the selected photo. Only preview
images load in the timeline; the viewer loads other photos as they are selected.
Use `-demo -demoPhotoGallery` for a synthetic 12-photo group example.

## Video playback

On iPhone and iPad, tap a video to reveal Apple's playback controls, then use the
expand button for full-screen viewing. Videos can continue in picture in picture
when leaving the app, subject to device support and **Settings → General →
Picture in Picture → Start PiP Automatically**. These iOS presentations share
the same playhead.
Starting another video pauses the first; forgetting the server stops retained
video playback.

The iOS app declares background audio and activates a playback audio session only
when a video starts. AVKit manages app-background transitions. Debug launch
arguments `-demo -demoVideo` show a synthetic, bundled video for UI checks.
`WhatsAppTranslatorTests/VideoPlaybackTests.swift` exercises real playback through message
removal, full-screen/PiP transitions, restoration and account removal. The PiP UI
check explicitly skips devices where AVKit reports no PiP support.

## Native Mac app

The Mac app provides a resizable two-column conversation window, native menu
bar and Settings scene, Command-R refresh, Command-F conversation search,
compact desktop message bubbles, drag-to-reply, context menus and inline media.
It shares the same App Store bundle ID and backend configuration as the iOS app.

macOS registers its own APNs token and notification category. Incoming message
notifications can open the matching conversation or send an inline reply
through the normal translation-aware `/api/send` route. The sandboxed target
has outbound network, communication-notification and shared-Keychain
entitlements.

After authentication, the app asks for notification, announcement, CarPlay, and Siri
permission, registers its APNs token with the backend, and displays incoming
translated messages even when the app is not open. The notification service
extension presents them as communication notifications with the sender, group
name on a separate line, translated body, and contact avatar. The server carries
the real group recipient count into the communication intent; when that count is
unavailable, a standard alert preserves sender, group, and message as separate fields. Tapping one opens the matching chat; its Reply action
sends through the normal `/api/send` route, so the conversation's translation
and send-original settings are applied. The same action is mirrored to Apple
Watch for dictated or typed replies. A Debug build registers against the APNs
sandbox; an archived Release build uses the production APNs environment.

The Siri intents extension implements sending, searching, and marking messages
read. Those intents provide the voice-first message surface required by CarPlay.
The app also opts its message notification category into announcements and
CarPlay, with the search/read intent handling a tapped CarPlay alert. Existing
installations re-request the full notification option set on activation, so an
upgrade includes CarPlay while preserving the user's existing notification choices.
Apple assigned the managed CarPlay Communication App entitlement to this developer
account on 11 September 2026. It is enabled for `com.vultuk.whatsapptranslator`
and included in the iOS target as `com.apple.developer.carplay-communication`.
The account can enable it separately for other eligible communication apps.

CarPlay opens **Messages**, an interleaved timeline across conversations, with a
**Chats** tab for the grouped conversation list. Each tab shows up to 12 entries
(or the vehicle's smaller limit). Messages shows the conversation, sender and time;
Siri handles message bodies and dictation. Selecting an unread entry reads that
message; selecting a read entry starts a reply. Message rows retain the exact
conversation and message identity, so a reply to an older entry quotes it using
the same conditional-quote rule as the iPhone unified view. Chats keeps unread
conversations first. Refresh updates both tabs, including automatic refresh while
active. Account setup stays on iPhone.

Tapped alerts resolve their exact delivered notification ID and read its translated
body. An expired alert returns no message instead of reading another conversation.
Siri replies use the selected conversation's exact ID and the existing translation
and durable-send API. Read receipts carry both conversation and message identity,
so the extension can mark the correct chat read without searching recent chats.

To verify on a device, update TestFlight, open Babel Bridge once, enable Show in
CarPlay in its iPhone notification settings, then reconnect to CarPlay. Announce
Notifications and Show Previews settings govern spoken alerts. Test a translated
incoming alert, tap to read, and dictate and confirm a reply to the same chat.
Xcode's iPhone simulator with I/O → External Displays → CarPlay verifies the
conversation screen; `-demo` uses synthetic CarPlay conversations. Siri speech,
real push delivery, and the car's audio routing still require a physical device.

The bundled **Babel Bridge** Watch app requires watchOS 11 or later and the paired
iPhone app. Install it from the iPhone Watch app after updating Babel Bridge in
TestFlight (or enable automatic app installation). It opens only the unified
**Messages** view, with the latest entry first. Tap **Reply to latest** to capture
the current latest message, or swipe left on any entry and choose **Reply**. Tapping
an entry also opens its reply. Use the Watch's system text input for dictation,
Scribble, or the keyboard where supported, then tap **Send**. The destination stays
fixed while composing even if a message arrives from another chat.

To launch from a watch face, edit the face, choose a complication slot, and select
**Babel Bridge**. The launcher supports circular, corner, rectangular, and inline
slots on compatible faces; tapping it opens the Watch app's Messages view.

The Watch asks the paired iPhone for the latest 30 messages while active, every
15 seconds and on Refresh. Long messages are shortened to 1,000 characters and
the transfer is bounded to 48 KB; open the iPhone for complete history and media.
Watch Connectivity wakes the iPhone app in the background when reachable. The
phone needs access to the backend; the Watch does not store the backend password.
Transport completions are explicitly Sendable because Watch Connectivity invokes
them on a background queue; UI state resumes on MainActor after awaiting the result.
Replies use the existing translation and send-original settings for that chat.
Selected older messages receive a quote, while the latest message in that chat
does not. Deleted or unavailable targets fail without changing the recipient.

Interrupted replies keep a protected draft and submission ID. **Check delivery**
consults the phone's persistent receipt instead of issuing another send. If the
backend result itself is uncertain, check the conversation on the iPhone before
composing another reply. The Watch does not silently queue sends while disconnected.
The `BabelBridgeWatch` scheme tests both the live startup/recovery screen and
synthetic `-demo` replies. The iOS suite exercises the shared transport's success
and error callbacks from a background queue. Real paired-device messaging and
dictation still need a physical iPhone and Apple Watch.

Apple Watch also mirrors iPhone notifications. With the iPhone unlocked, Apple routes
the alert to the iPhone. To test Watch delivery, wear and unlock the Watch, lock
the iPhone, and enable this app under Watch → Notifications → Mirror iPhone Alerts
From. Focus and notification settings can still silence alerts. The app cannot
override Apple's routing to alert both devices simultaneously. See
[Apple's notification routing guide](https://support.apple.com/en-ie/108369).

Incoming message alerts also request background execution. When iOS grants it,
the app fetches the affected chat and writes a file-protected local cache before
the user opens the app. Cached chats render immediately at launch and refresh
again when the app becomes active. Background execution is best-effort on iOS,
so the foreground refresh remains the fallback if the system throttles a push or
the user has force-quit the app.

Run either app scheme with the `-demo` launch argument to preview populated
chats without a backend. Add `-demoConversation` to select a populated
conversation immediately.

Add `-demoNotification` to an iOS Debug run to preview a synthetic translated
group alert using the same communication-notification formatter as incoming
pushes. Enter each launch argument on its own row in Xcode. Allow notifications when prompted. It fires after five seconds and
does not contact the backend or send a WhatsApp message.

Add `-demoLiveReactions` alongside `-demo` to preview a reaction arriving after
20 seconds, changing emoji, being removed, and returning without navigation.
The fixture decodes the same live-event payloads as the WebSocket handler.
Reaction state is retained per conversation, target, and actor, so translation
updates, refreshes, history pages, and older event replays cannot erase a newer
reaction. Reaction events remain hidden from message lists.

Long-press a message (or open its context menu on macOS) for a row of quick
reactions beside the reply actions. The plus opens more emoji choices; selecting
an existing quick reaction again removes it. The send response contains the
confirmed reaction, which updates both views even when the live socket is delayed.

The bridge publishes successful outgoing reactions before acknowledging the send,
and preserves WhatsApp's millisecond reaction clock. Empty emoji fields omitted by
the Go bridge represent removals. Both paginated message endpoints attach current
reaction aggregates and per-actor clocks, including removals, so cached or older
pages cannot resurrect an earlier choice. The shared `ReactionDeliveryTests` and
long-press UI test cover this path without sending real WhatsApp messages.

Live text and caption alerts are persisted on the server until language detection
and any required translation finish. Translation retries and server restarts
retain pending alerts; imported history does not create alerts. Group alerts carry
separate sender, group subtitle, and translated body fields. Their communication
intent uses the actual recipient count from WhatsApp to select the group layout;
older prefixed payloads remain supported.

## Unified feed attachments

The Messages composer supports photo albums (up to 30 photos), videos, files, and translated voice notes on iOS, iPadOS, and macOS through a single **+** menu, without a separate microphone shortcut. Tapping **+** captures the selected message, or the latest message in the feed when no reply is selected, before opening the picker or recorder. The destination and message remain fixed while media is prepared. Cancel the reply to choose a different destination.

Photos use the existing optimization and album progress flow. Videos are prepared as MP4; videos and files are limited to 64 MB per attachment. Captions and voice notes use the captured conversation's translation settings. Sending media preserves any separate text draft.

Deploy the matching backend before updating native clients: `/api/send-media` adds confirmed video/file sends, and `replyOnlyIfNotLatest` now applies to photos, staged albums, and prepared voice notes. The server rechecks the captured message in its destination chat immediately before preparing the send; only older messages receive a quote. The existing duplicate-safe delivery tracking also covers video/file sends.
