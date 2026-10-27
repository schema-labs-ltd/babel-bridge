@AGENTS.md

## Claude Code notes

- `AGENTS.md` (imported above) is the single source of instructions, so Codex and Claude Code follow the same rules. Change rules there, not here.
- In a cloud session, `.claude/hooks/session-start.sh` runs at start-up. It installs ffmpeg when it is missing, installs `web/` dependencies and Playwright's Chromium, and fetches the Rust crates and the Go toolchain and modules, so `make check` needs no first-use downloads. It does nothing in a local session.
- Run `make check-fast` while iterating and `make check` before pushing. The first `cargo test` in a fresh session compiles everything and takes a few minutes.
- A cloud session cannot use Xcode or Computer Use. Follow the cloud-session rule under "Apple build and release workflow" above, and read Apple failures from the `apple / apple` job logs.
- Never bump `CURRENT_PROJECT_VERSION` or upload to TestFlight. A change ships when the Xcode Cloud build for its merge commit succeeds, as the TestFlight release completion gate above describes.
