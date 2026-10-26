# Review guidelines

Read by the Codex and Copilot pull request reviewers and by agents running `/code-review` before they open a pull request. Most pull requests here are written by agents and merge without a human reading the diff, so a review comment costs another commit and another review round. Report what would hurt a user or break `main`, and leave the rest.

## What counts as a real problem

- **Private content leaving the process.** Message text, phone numbers, contact names, chat ids, media and tokens must never reach logs, `/api/diagnostics`, error responses, push payloads or analytics. `src/log_privacy.rs` states the rule: log lines describe what happened, never what was said or who said it.
- **Auth that fails open.** `src/web/auth.rs`, `src/web/oauth/`, `src/oauth.rs`, `src/access.rs` and the MCP HTTP routes must reject a missing, expired or revoked token. A new route under `/api` must sit behind `require_web_auth` unless the pull request says why not.
- **Breaking the shipped apps.** A change to an HTTP or WebSocket response must keep `fixtures/api/`, `ios/WhatsAppTranslator/Models/APIModels.swift` and `ios/APIContract/APIContractCheck.swift` in step (AGENTS.md, "Server and app API contract"). Removing or renaming a field the apps decode breaks every installed build until it updates.
- **Data loss in storage.** Changes under `src/storage/schema/` run on existing databases at start-up. A migration must work on a database from the previous release and must not drop user data.
- **The Rust and Go bridge drifting.** The JSON protocol in `src/bridge/` and `wa-bridge/` is pinned by shared fixtures; both sides change together.
- **CI, hooks and release config.** Changes to `.github/`, `lefthook.yml`, `scripts/check-*`, `Makefile`, `Dockerfile` and `railway.toml` decide what every later pull request is checked against. Flag anything that skips, weakens or silently passes a check.
- **Docs that lie.** AGENTS.md, CLAUDE.md, READMEs and `docs/` must match the code after the change. Flag a command, path or rule the pull request makes wrong.

## What to leave out

- Formatting and style that `rustfmt`, `gofmt`, `prettier`, clippy or `go vet` own. CI already fails on those.
- Naming preferences, optional refactors, and "consider adding a comment".
- Generated files: `web/extensions/dist/`, `ios/WhatsAppTranslator.xcodeproj`, lockfiles, `fixtures/api/responses/` (written by `UPDATE_FIXTURES=1 cargo test api_contract`).

## How to report

- Cite the file and line. Do not guess file names.
- Mark a bug the pull request did not introduce as **pre-existing**, so the author opens an issue instead of widening the pull request.
- Name the class of bug once, with every place it occurs, instead of one comment per line.
- On a second or later review round, review only the new commits and do not repeat a finding already answered.
- When there is nothing to report, say so in one line.
