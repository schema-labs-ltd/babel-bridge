# Code review guidelines

Automated and human reviewers (Codex, Copilot, Claude) follow this file when reviewing pull requests. It says what matters in this repository; [AGENTS.md](AGENTS.md) describes the project itself. Keep this file short, and when the same review comment keeps coming up, add it here.

## What failure looks like here

This is a self-hosted WhatsApp client that translates messages and serves them to a web app, MCP clients and native Apple apps. Look hardest for changes that could do any of the following:

- **Private content leaving the process.** Message text, phone numbers, contact names, chat ids (JIDs), media, QR codes, push tokens, and OAuth or ChatGPT tokens reaching logs, `/api/diagnostics`, error responses, push payloads or fixtures. Log lines pass through `apps/server/src/log_privacy.rs` and describe what happened, never what was said or who said it; a path that bypasses or weakens its redaction is always Important.
- **Auth that fails open.** Web sign-in (`apps/server/src/web/auth.rs`), OAuth for MCP clients (`apps/server/src/oauth.rs`, `apps/server/src/web/oauth/`), MCP read and send permissions (`apps/server/src/mcp.rs`, `apps/server/src/web/mcp_http.rs`) and the startup access policy (`apps/server/src/access.rs`) must reject a missing, expired, revoked or malformed token, and a hosted server must never run without a password. A new route in `apps/server/src/web/server.rs` must sit behind `require_web_auth` unless the PR says why not. A tool that sends a WhatsApp message without the send scope is Important.
- **Sending what the user did not ask for.** A message sent to the wrong chat, sent twice, or sent untranslated or mistranslated without the user seeing it first. Watch the outbox, send confirmations and AI compose or reply paths.
- **Data loss in storage.** Migrations in `apps/server/src/storage/schema/` run on existing SQLite databases at start-up. A migration must work on a database from the previous release, be safe to re-run, and must not drop or rewrite stored messages, contacts or OAuth grants.
- **Breaking the shipped apps.** A changed HTTP or WebSocket response must keep `fixtures/api/`, `apps/ios/WhatsAppTranslator/Models/APIModels.swift` and `apps/ios/APIContract/APIContractCheck.swift` in step (AGENTS.md, "Server and app API contract"). Removing or renaming a field the apps decode breaks every installed build until it updates.
- **The Rust server and Go bridge drifting.** The JSON protocol in `apps/server/src/bridge/` and `apps/wa-bridge/` changes on both sides together, and the server treats the bridge's input as untrusted.
- **Requests the server should not make.** Link previews (`apps/server/src/link_preview.rs`) fetch URLs from incoming messages; flag requests that can reach private, loopback or metadata addresses.
- **Docs that lie.** AGENTS.md, CLAUDE.md, READMEs and `docs/` must match the code after the change. Flag a command, path or rule the PR makes wrong.

Other correctness bugs still count. Style alone does not.

## Every finding needs evidence

- Cite the file and line for each finding, and say concretely what goes wrong (the input or state and the wrong result). Do not infer a bug from a name, a pattern, or code you have not read.
- If you cannot point to a line, do not post the finding.
- Do not repeat what CI already enforces: `rustfmt`, clippy, `gofmt`, `go vet`, prettier, the extension type check, the API contract check, the file-size limit, and the tests.

## Severity tags

Start every inline comment with one tag:

- **Important**: a bug or risk this PR introduces that should be fixed before merge.
- **Nit**: a minor improvement that does not block merge. Post at most **three** nits per review; drop the rest.
- **Pre-existing**: a real problem in code this PR touches but did not cause. Say so plainly so it is not attributed to the author, and do not block the PR on it.

## Paths to skip

Do not review or comment on:

- Lockfiles: `Cargo.lock`, `package-lock.json`, `apps/wa-bridge/go.sum`, `Package.resolved`, and other package manager lockfiles.
- Generated files: anything marked `linguist-generated` in [.gitattributes](.gitattributes), including `libs/web-extension/dist/`, `*.pbxproj` and `*.xcassets/`.
- Snapshots: `fixtures/api/responses/`, written by `UPDATE_FIXTURES=1 cargo test api_contract`. Review the change to the server or the app models that caused them instead.
- Screenshots, images and rendered sounds, such as `*.png`, `*.jpg` and `apps/web/public/sounds/`, beyond checking that they show no message content, phone numbers or secrets.

## Summary format

Begin the review summary with the count of findings by tag, for example `2 Important, 1 Nit, 0 Pre-existing`. If there is nothing to fix, say exactly that in the first line (`0 Important, 0 Nit, 0 Pre-existing: nothing to fix`) and stop; do not pad the review with praise or a restatement of the diff.
