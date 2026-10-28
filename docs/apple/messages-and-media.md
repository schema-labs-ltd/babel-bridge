# Messages and media on Apple platforms

See the [Apple platforms README](../../apps/ios/README.md) for building, testing and releasing the apps.

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

## Unified feed attachments

The Messages composer supports photo albums (up to 30 photos), videos, files, and translated voice notes on iOS, iPadOS, and macOS through a single **+** menu, without a separate microphone shortcut. Tapping **+** captures the selected message, or the latest message in the feed when no reply is selected, before opening the picker or recorder. The destination and message remain fixed while media is prepared. Cancel the reply to choose a different destination.

Photos use the existing optimization and album progress flow. Videos are prepared as MP4; videos and files are limited to 64 MB per attachment. Captions and voice notes use the captured conversation's translation settings. Sending media preserves any separate text draft.

Deploy the matching backend before updating native clients: `/api/send-media` adds confirmed video/file sends, and `replyOnlyIfNotLatest` now applies to photos, staged albums, and prepared voice notes. The server rechecks the captured message in its destination chat immediately before preparing the send; only older messages receive a quote. The existing duplicate-safe delivery tracking also covers video/file sends.
