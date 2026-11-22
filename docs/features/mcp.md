# MCP access

How external LLM clients connect to the server. OAuth callback rules for hosted deploys are in the [README](../../README.md#required-environment-variables).

## Endpoint and scopes

The Streamable HTTP endpoint is `/mcp`. OAuth permissions are split into:

- `whatsapp.read` — status, conversation discovery, message history/search,
  and message preparation
- `whatsapp.send` — sending prepared messages and replies, reactions, and read
  receipts; this scope requires `whatsapp.read`

If an authorization request omits scope, it receives read-only access. A DCR
registration that omits scope is eligible to request both supported scopes later;
registration itself grants no access. Explicit read-only registrations remain
restricted to read-only authorization. Password consent, PKCE and exact registered
redirect matching are required before any tokens are issued. The legacy `mcp`
scope retains its existing behavior for previously registered clients.

If ChatGPT reports "Requested scope is not registered for this OAuth client",
its existing registration does not permit the requested send scope. Read-only
connection remains possible by selecting only `whatsapp.read`. For send access,
recreate the DCR connection after the corrected server release so registration can
include both supported scopes. Reauthorization alone does not update existing
registration metadata, and this release does not migrate clients or token grants.

The read workflow exposes `get_status`, `list_contacts`, `search_contacts`,
`read_messages`, `search_messages`, and `prepare_message`. External writes are
separate: `send_message`, `reply_to_message`, `react_to_message`, and
`mark_conversation_read`.

Text sends are deliberately two-step:

1. Call `prepare_message` to resolve the exact recipient, translation mode,
   target language, final text, and optional reply target.
2. Show that result to the user, then pass its short-lived preparation token
   and a unique idempotency key to `send_message` or `reply_to_message`.

When a conversation requires translation, preparation fails closed if the
translator is unavailable, errors, or returns empty text. The English source is
never sent as a fallback. Use `translation_mode: "never"` only when sending the
provided text unchanged is intentional.

## Events: new incoming messages

`/mcp` supports the [MCP Events extension](https://modelcontextprotocol.io/extensions/events/overview)
(`io.modelcontextprotocol/events`), so a client can be told about new messages
instead of polling `read_unified_messages`. The `initialize` result advertises
it in `capabilities.extensions`.

- **Event type.** `events/list` returns one type, `message.received`. It fires
  once for each new incoming message in a private or group chat, from the same
  live updates the web app shows. It does not fire for your own sent messages,
  receipts, typing, reactions, deletions, status updates, broadcast lists or
  old messages from a WhatsApp history sync.
- **Payload.** `data` has the same fields as a `read_unified_messages` message
  (`payloadSchema` is that schema). It carries whatever translation exists when
  the message arrives and does not wait for one; read the chat again for later
  translations. `eventId` is the WhatsApp message id. Message text is untrusted
  data and must not be treated as instructions.
- **Filter.** The optional `contact_id` argument limits a subscription to one
  chat. Without it a subscription covers every chat.
- **Delivery.** Poll (`events/poll`, with a recommended `nextPollMs` of 5
  seconds) and push (`events/stream`, a long-lived SSE response with
  `notifications/events/active`, `.../event` and `.../heartbeat`; the default
  heartbeat is 30 seconds, clamped to 1–60 seconds). Webhook delivery
  (`events/subscribe`) is not offered and returns `Unsupported`. At most 32
  push streams can be open at once, and at most 8 for one OAuth client;
  beyond that `events/stream` returns `ResourceExhausted`.
- **Cursors.** A `null` cursor starts from now. The server keeps the latest 500
  incoming messages in memory, so a client can resume from its last cursor
  after a short disconnect. A cursor from before a server restart, or older
  than that buffer, resumes with `truncated: true`, meaning events may have been
  missed: read the feed with `read_unified_messages` to catch up. `maxAgeMs`
  and `maxEvents` are honoured.
- **Access.** Subscribing needs `whatsapp.read` (or the legacy `mcp` scope). A
  send-only token sees no event types and gets `Forbidden`; an expired, revoked
  or unknown token gets the usual `401` OAuth challenge. A push stream checks
  the token again before every delivery and heartbeat, and ends with
  `notifications/events/terminated` (`Forbidden`) once the token is revoked or
  expires. If the check itself fails (a storage error), delivery continues
  for at most 30 seconds after the last good check and never past the token's
  expiry.
- **No side effects.** Subscribing and receiving events never marks messages
  read, queues translation or writes to the database. Event logs record only
  the method, event name, status and counts, never message text, numbers or
  chat ids.
- **Request size.** `/mcp` request bodies are limited to 4 MB.
- **Protocol era.** The server speaks the `initialize`-based protocol revision
  (2025-11-25) through rmcp 0.13, so `server/discover` is not available. The
  `Mcp-Name` header is optional; when sent it must match the event name.
