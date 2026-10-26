## What changed

<!-- One or two sentences on the user-visible or developer-visible change, and why. -->

## How verified

<!-- Which checks you ran (for example `make check`, `cargo test`, `go test ./...`, `npm test`, the extension tests),
     and for UI changes which screen, demo launch flag (such as `-demo -demoConversation`) or screenshot shows it. -->

- [ ] Rust (`cargo fmt --check`, `cargo test`, `cargo clippy --all-targets`)
- [ ] Go bridge (`go test ./...` in `wa-bridge/`)
- [ ] Web (`npm test` in `web/`, extension checks if `web/extensions/` changed)
- [ ] Apple (`apple / apple` check, or Xcode locally)
- [ ] Fresh-context review (`/code-review` or a reviewer subagent) before opening

## Risk

<!-- What could break, and for whom. Name anything this touches from REVIEW.md's "What counts as a real problem":
     auth or OAuth, message content in logs or diagnostics, the app API fixtures, storage schema, push, CI or deploy. -->

## How to revert

<!-- Usually "revert the squash commit". Say if reverting needs more: a schema migration that does not roll back,
     an API change the shipped apps already depend on, a setting or secret to undo. -->

## Apple impact

<!-- Does this change ios/, web/public/sounds/ or anything the apps call?
     Say whether a TestFlight build is expected on merge, and whether it was checked on a device. -->

- [ ] No Apple changes; no TestFlight build expected
- [ ] Apple changes; TestFlight build expected from Xcode Cloud on merge
- [ ] Checked on a device or simulator
