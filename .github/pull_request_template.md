## What changed

<!-- One or two sentences on the user-visible or developer-visible change, and why. -->

## How verified

<!-- Which checks you ran (for example `cargo test`, `go test ./...`, `npm test`, the extension tests),
     and for UI changes which screen, demo launch flag (such as `-demo -demoConversation`) or screenshot shows it. -->

- [ ] Rust (`cargo fmt --check`, `cargo test`, `cargo clippy --all-targets`)
- [ ] Go bridge (`go test ./...` in `wa-bridge/`)
- [ ] Web (`npm test` in `web/`, extension checks if `web/extensions/` changed)
- [ ] Apple (`apple / apple` check, or Xcode locally)

## Apple impact

<!-- Does this change ios/, web/public/sounds/ or anything the apps call?
     Say whether a TestFlight build is expected on merge, and whether it was checked on a device. -->

- [ ] No Apple changes; no TestFlight build expected
- [ ] Apple changes; TestFlight build expected from Xcode Cloud on merge
- [ ] Checked on a device or simulator
