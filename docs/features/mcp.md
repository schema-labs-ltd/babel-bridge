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
