# Messaging features

How the apps behave for everyday messaging. Setup and deployment are in the [README](../../README.md).

## Read receipts

Incoming messages send WhatsApp read receipts only after they remain visible for 0.7 seconds in the active app or browser tab. This applies to conversations, combined Messages, and the selected full-screen media item. Loading, syncing, translating, and offscreen messages do not send receipts. Viewed IDs are queued offline and retried after reconnecting; group receipts preserve the original sender, and WhatsApp read-receipt privacy settings still apply.

## Translation

Translation defaults **off for every person and group**, including existing conversations. Open **Conversation settings → Translate messages** to enable it for a particular chat. The server saves the choice by conversation ID and shares it across web, iOS/iPadOS, macOS, and unified **Messages**. In Messages, long-press or right-click a message's conversation heading to open those same settings. Disabled conversations send text and captions as written and voice notes as original recordings, without language detection or other translation AI calls. Disabling also cancels queued incoming translations and releases pending notifications; saved language, style, and original-follow-up preferences are retained.

For enabled conversations, incoming text and media captions translate into `WA_DEFAULT_LANGUAGE`. Outgoing text, captions, and voice use a conversation override first, then the replied-to incoming message's language, then the predominant recent incoming language. Unknown chat language is resolved from recent incoming text before sending. Language-neutral messages stay unchanged. Translation failures prevent outgoing sends and incoming jobs retry with backoff; opening an enabled conversation queues previously undetected text and captions.

## Reply only

Enable **Conversation settings → Reply only** for any person or group to prevent unprompted sends. Choose an incoming message and tap **Reply** before sending text, photos, files, videos or voice notes. The choice is saved on the server and applies across iOS/iPadOS, macOS, web and connected tools. Combined **Messages** requires an explicit reply selection; replying to the latest message still includes its quote. Your own, deleted or another conversation’s messages cannot unlock sending. Turn the setting off to resume normal sending. Reactions remain available.

## Topics

Open **Topics** on the web, or choose **Manage topics** from the native topic filter (the filter button in the unified **Messages** feed, or **Filter by topic** in a conversation's menu), to enable automatic organisation for individual chats. Topic classification uses the existing OpenAI key and shared model settings, independently of translation. It starts with the latest 200 text messages and captions, then processes new messages and edits in background batches. Topic labels appear in the configured default language; matching labels from different people and groups combine into one category in Messages. Each message retains its source chat; a topic selected inside a conversation still shows only that conversation. The AI prefers broad categories of 1–3 words, such as Hospital, Shopping, Photos and Birthday wishes. Hospital support, delays, surgery and recovery belong under Hospital. Existing detailed labels are simplified in a bounded background pass using only their titles; messages stay visible in their current categories until each validated merge completes. Categories are reused across chats when the subject fits. **All messages** always restores the full timeline, and replies and reactions still address the original WhatsApp message. Incoming replies retain WhatsApp’s embedded original quote even when its message is absent from loaded history. A later history sync can restore quotes previously dropped by older builds; originals that WhatsApp does not resend cannot be reconstructed from the stored text alone. Classifications are stored on the server, survive restarts, and refresh across clients. Disabling topics stops new classification and hides that chat's topic views while retaining saved assignments. Failed batches retry up to three times; re-enabling or choosing retry starts another attempt. Enabling topics adds AI usage and does not alter anyone else's WhatsApp interface.

For an initial import, choose **Organise last 7 days from all chats** in the Topics panel (**Manage topics** in the native apps). The app previews the number of unorganised text messages and captions before you start. It uses only messages already stored on the server, excludes Updates and older history, skips existing topic assignments, and enables ongoing organisation for the included chats. Classification runs in background batches and the Topics panel shows progress; repeating the import safely skips completed messages.

## Translated voice notes

In conversations with **Translate messages** enabled, incoming voice notes are transcribed, translated into your default language, and
spoken by an AI-generated voice. The original remains available beside the
translation. Older recordings can be translated on demand.

Use the microphone button in a conversation to record up to three minutes,
listen to the translated preview, then send it as a WhatsApp voice note. Set the
conversation language first if it has not yet been detected. The existing
original-follow-up setting also applies to recordings. Translation failures do
not send the original as a fallback. Prepared recordings expire after 15 minutes;
if a send reports an uncertain delivery, check the conversation before recording
and sending again.

Choose Automatic, Masculine, Feminine, or Neutral in app settings for your
outgoing voice, or conversation settings for a contact's translated voice.
Automatic approximately matches acoustic pitch and falls back to Neutral when
uncertain; it does not clone the speaker or identify their gender. Each choice
has an audible sample. Changing a setting applies to the next prepared recording.

Original recording preparation requires microphone permission and FFmpeg. With translation off, recordings are not sent to OpenAI. Translated voice processing additionally requires an OpenAI key with
access to `gpt-4o-mini-transcribe` and `gpt-4o-mini-tts`. Browser recording requires
HTTPS or localhost. Audio and transcripts are sent to OpenAI for processing;
cached recordings are stored in the private application database. Audio-service
charges are not currently included in the app's text-model usage totals.

## Pasting images

Copy an image, then use **Paste** in the message box on iPhone or iPad, or **⌘V** on Mac. Pasted images open the photo preview, where you can add a caption, cancel, or send. Multiple copied images (up to 30) and image files copied in Finder are supported. Your existing text draft is preserved, and ordinary text paste still works. This is available in both individual chats and the unified Messages view.

## Chat gallery

On iPhone, iPad, and Mac, open a conversation and choose **Gallery** in the header. Photos and videos appear newest first in a square grid grouped by date, including older media saved on the server. Open any item full screen, then swipe left or right (or use the arrow buttons) to move between photos and videos. Photos support zooming; videos play in place and pause when you move away. Older items load as you browse. Deleted messages, stickers, audio, and documents are excluded.

## App icons

Native **Settings → Appearance → App icon** offers ten original image-generated designs, alongside the existing icon: Paper Bridge, Neon Orbit, Sunrise, Porcelain, Botanical, Pixel Passport, Ribbon, Monochrome, Mosaic, and Cosmic Whale. iPhone and iPad use the selected Home Screen icon; macOS remembers a local Dock icon choice and reapplies it while the app runs, leaving the signed app and Finder icon unchanged. Select **Original** to reset. No server connection is needed to choose an icon.
