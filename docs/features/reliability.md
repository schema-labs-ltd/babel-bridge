# Connection, delivery and data repair

How the server and apps recover from dropped connections, retries and older data.

## WhatsApp reconnection

Temporary connection drops reconnect using the saved WhatsApp session. If WhatsApp removes the linked device, the backend restarts linking automatically and the iPhone, iPad, Mac, and web apps display a fresh QR code. Expired codes are replaced automatically; successful linking returns to the inbox. Saved chats and server credentials are retained. The native apps also check status on returning to the foreground and periodically while open, so a missed live event does not leave the app stuck.

To scan the code, use WhatsApp on the account's main phone under **Settings → Linked Devices → Link a Device**. When using Babel Bridge on that same phone, display the code on a computer or tablet.

## Delivery and connection recovery

Incoming text is stored immediately. In enabled conversations it is then translated by two background workers.
The persistent queue holds up to 1,000 pending jobs and retries failed jobs up to
three attempts. Queue overflow and exhausted retries retain the original message;
manual translation remains available. Completed translations update the existing
message without creating another unread message.

WhatsApp edits replace the original message in both conversation and unified-feed
views, preserving its send time, reactions, and attachments. The latest edit is
persisted even when it arrives before its target during history recovery. Edits
clear outdated translations and queue the corrected text in enabled conversations;
late translation results and older message snapshots cannot undo a newer edit.
Native and web messages show an Edited label beside the original send time.

Native and browser clients reconnect with bounded backoff and refresh the open
conversation to recover messages missed while disconnected. Outgoing actions use
persisted idempotency keys. A lost response or server restart retains the same
operation identity; retrying that operation does not blindly send another message.
Pending sends remain visible as uncertain until confirmation arrives. A late
WhatsApp acknowledgement reconciles the stored message and operation result.
Check an uncertain conversation before composing a new send. These protections
cannot prove delivery when WhatsApp never returns an acknowledgement.

Keep the backend data volume and client app/browser storage intact for recovery.
Logout clears server recovery records. MCP send claims and outcomes also persist
across backend restarts. Deploy the updated backend before distributing the new
native clients. Network-accessible startup requires a nonblank password; explicit
password-free development binds only to loopback. Password verification allows
15 attempts per minute across the single-user service, including OAuth approval.

## Device-address repair

Private chats use the WhatsApp account address. Device suffixes such as `:17`
and `:22` are removed after WhatsApp resolves an LID to a phone address; an
unresolved LID stays in its own namespace. The same rule applies to history
imports, cached conversation links, replies and saved voice preparations.

On first startup after this update, the server repairs older split conversations
in one SQLite transaction. It preserves message IDs, contents, timestamps,
attachments, unread totals and the existing account conversation's preferences.
Before merging, it saves a private, consistent `messages-before-device-jid-repair-*.db`
snapshot beside `messages.db`, using SQLite's page backup API with a 4 MiB
destination cache. Copy progress is logged every five seconds; interrupted starts
reuse only the incomplete destination, while completed snapshots are retained.
Keep completed snapshots for recovery and provide space for another copy of the
database. Railway allows up to ten minutes for this one-time startup work. A failed
snapshot or merge stops startup without applying a partial repair. Successful startup logs the repaired conversation and
message counts without logging phone numbers or message contents.
