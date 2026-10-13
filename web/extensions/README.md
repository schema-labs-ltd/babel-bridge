# WhatsApp conversations in ChatGPT

The existing Rust MCP server registers `ui://whatsapp-translator/chats` as a real
`text/html;profile=mcp-app` resource. `open_chats` accepts `{}` and declares both a
global sidebar entrypoint and a thread panel entrypoint. `open_chat` takes a known
`contact_id`. Resource metadata prefers `fullscreen`, supports `inline` and
`fullscreen`, and denies external resource/connect origins. In the Extensions
spec, both sidebar apps and conversation panels use `fullscreen`; there is no
`sidebar` display-mode enum. A supported inline host offers an expand control.

The frontend uses the official MCP Apps `App` and OpenAI `OpenAIExtensions` bridge.
It consumes the initial tool result without repeating the opener; subsequent
reads and actions call the same authenticated server through `callServerTool`.
Deep links `/chats/<percent-encoded-contact-id>` resolve through `open_chat` and
receive the same server validation. Unknown IDs never create a chat. Theme changes
follow host context. Context sharing identifies the selected chat without sending
message contents. UI data is rendered as text and draft state is held only in
memory, cleared on teardown.

## Authentication and actions

This service is **one WhatsApp account per instance**, not a multitenant service.
The existing OAuth token database binds access to that instance. No tenant/user ID
is accepted from tool arguments and no new scopes or clients are configured by this
change. `whatsapp.read` is required for resources, openers, listing, message history,
search and preparation. `whatsapp.send` is still required for sends, replies,
reactions and marking read; new `translate_message` requires both scopes because
it changes the locally stored translation and may incur AI usage. The legacy
`mcp` scope retains its existing behavior. The iframe never receives a bearer token
or bypasses the host via web API calls. Account reset continues to revoke tokens.

ChatGPT setup uses the existing `/mcp` URL with OAuth and Dynamic Client
Registration. CIMD is not advertised or implemented. The callback allowlist accepts
only OpenAI's documented HTTPS paths on `chatgpt.com` and existing loopback
callbacks. Password approval, exact registered redirect matching and S256 PKCE
remain required. Explicit OAuth `resource` values must match this instance's
canonical `/mcp` URL; existing clients that omit it remain compatible because the
opaque tokens are bound to this instance's token database and single MCP resource.

DCR scope metadata describes client eligibility, separately from user consent.
When DCR omits scope, the client may later request both `whatsapp.read` and
`whatsapp.send`. An authorization request that omits scope still defaults to read
only. Explicit registration limits and legacy behavior are preserved. Send access
requires an explicit read/send authorization request, password consent and PKCE.
Existing registrations limited to read are not automatically widened: use read
only now or recreate the ChatGPT DCR connection after the corrected release to
register for send access. Existing tokens and grants are unchanged.

Existing standalone tools remain available without opening the UI:
`get_status`, `list_contacts`, `search_contacts`, `read_messages`, `search_messages`,
`prepare_message`, `send_message`, `reply_to_message`, `react_to_message`,
`mark_conversation_read`. Incoming `translate_message` reuses the web app's
translation handler, preserving opt-in settings, source-text validation and edit
revision protection. History includes original/translated text, replies, delivery
state and media type placeholders. Media playback and uploads remain in the native
and web apps; this Extension presents the text chat workflow.

Opening, searching and paging never mark chats read. Sending requires a short-lived
preparation token, recipient/final-text review and an explicit confirmation.
Retries reuse the idempotency key. Switching chats or editing a draft invalidates
preparation; late read/search/prepare responses are discarded. Translation errors
remain errors rather than sending untranslated fallback text. Read-only tokens
hide action controls and are also enforced on the server. Existing tool annotations
are preserved and openers are annotated as read-only, non-destructive and idempotent.

## Unified Messages

Choose **Messages** in the view navigation or conversation header to read the
latest stored messages across individuals and groups. The timeline shows source
conversation headings, direction, group senders, timestamps, stored translations
and originals, quotes, delivery state and media placeholders. Source names fall
back to the stored phone or exact conversation ID even when that contact is
outside the loaded Chats pages. A source heading opens its exact conversation
through `open_chat`.

Messages is view only, including with a send-enabled connection. It has no
composer, replies, reactions, translation action, read-receipt tracking, media
playback or uploads. Feed reads, refresh, paging, original-text disclosures and
source navigation do not share model context. Entering Messages invalidates a
prepared chat message while retaining its draft. Existing actions remain in
Chats, where normal selected-chat navigation retains its existing context sharing.

The app-only `read_unified_messages` tool requires `whatsapp.read` and uses the
existing account's storage query directly; it does not invoke the REST feed
handler, which can queue translations. No new opener, resource, scope or
credential is needed. `WA-FEED-CONTRACT-v1` accepts `limit` (1–100, default 50)
and paired optional `before_timestamp` / `before_message_id` fields. It returns
`messages` ordered by ascending timestamp and opaque message ID, `hasMore`, and
`nextCursor: { beforeTimestamp, beforeMessageId }` for the oldest retained row
when earlier messages exist, otherwise `null`. Each message includes its own
`contactId`, nullable `contactName` / `contactPhone`, and `chatType`, alongside
the stored message presentation fields. Feed responses have no `view` field;
the UI keeps its own explicit navigation state.

**Load earlier messages** preserves a visible bubble's pixel position and open
original-text disclosures. **Refresh messages** starts a new latest-page window.
There is no background polling. Delayed reads, source navigation, deep links,
errors and retries are guarded against newer navigation and requests.

## Design reference

The UI follows `ios/WhatsAppTranslator/Design/TranslatorTheme.swift`,
`Views/Chats/ChatRow.swift`, `Views/Conversation/MessageBubble.swift` and
`Views/Conversation/ComposerView.swift`: system typography, accent `#00A884`,
deep accent `#00695C`, incoming `#FFFFFF` / `#1F2C33`, outgoing `#D9FDD3` /
`#005C4B`, warm chat background `#F5F1E8` / `#0B141A`, rounded bubbles,
initial avatars, unread badges, pin markers, translation/original disclosure,
quoted replies and a rounded composer. Narrow panels use list/chat navigation.

## Versioned contract

- [OpenAI Extensions guide](https://developers.openai.com/plugins/build/extensions)
- [Specification snapshot](https://github.com/openai/mcp-extensions/blob/0ba30cd5e3aa17265685969508e0ca7e39011a3a/docs/spec.md)
- `@openai/mcp-extensions` **0.1.0**
- `@modelcontextprotocol/ext-apps` **1.7.5** and MCP TypeScript SDK **1.29.0**,
  matching the Extensions SDK peer versions
- Rust `rmcp` **0.13.0**, unchanged: server registration is implemented using its
  native tools/resources handlers and the language-independent Extensions metadata.

## Build and test

From `web/`, with Node 22:

```sh
npm ci
npm run lint:extension
npm run typecheck:extension
npm run build:extension
npm run check:extension
npm test
npx playwright install --with-deps chromium
CHROMIUM_PATH="$(node -e 'process.stdout.write(require("playwright").chromium.executablePath())')" npm run test:extension
```

`dist/chats.html` is committed and embedded with Rust `include_str!`, preserving
the existing hosting/deployment path. Rebuild it after frontend edits. CI verifies
that the committed resource matches its sources. The bundle contains no externally
loaded JavaScript, CSS, fonts or credential material. Run `cargo fmt --check`,
`cargo test --locked`, `cargo clippy --locked` and Go bridge tests from the repository.
The production Docker Rust build copies the committed resource before compilation;
CI also builds that Docker stage to verify the real build context and toolchain.

The browser suite loads the actual compiled resource inside an iframe and exercises
its real SDK handshake against a **local simulated host** with synthetic data and
mock send results. It covers pagination, switching with late responses, search,
translations, reply preparation, explicit send confirmation, same-key retries,
draft invalidation, interruption, scopes, errors/empty states, deep links, untrusted
text, strict CSP, light/dark themes and narrow layout. Screenshots are written to
`output/playwright/extension-{desktop,mobile,dark,empty}.png` and uploaded by CI.
Rust tests also cover real HTTP MCP registration, missing/invalid/expired OAuth,
read versus send scopes, separate-instance isolation and sparse/equal-timestamp
pagination. Tests never launch a WhatsApp bridge, link an account or send real
messages.

The unified-feed browser cases assert equal-timestamp paging, off-page source
identity and exact navigation, stored text/quotes/media/direction, zero write
tools or model-context updates during feed operations, stale successes and
failures, interrupted reads, repeated paging clicks, refresh/older-page races,
and pixel anchoring through the final page in desktop and narrow layouts. The
local host fixture uses small synthetic pages to make pagination assertions
explicit. Feed screenshots are saved as
`output/playwright/extension-feed-{desktop,narrow,dark,empty,loading,error}.png`.

Local browser verification is **not** verification inside ChatGPT. Before release,
use the existing plugin in an authorised ChatGPT test host to verify sidebar and
thread discovery, actual fullscreen placement, deep links, resource CSP, OAuth
proxying, host lifecycle and explicit send approvals. No plugin ID, hosting,
persistent permission or production account configuration was changed. Publishing,
merging and TestFlight releases are separate release steps.
