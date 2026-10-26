# Claude Code notes

The repository rules live in AGENTS.md, so Codex and Claude Code follow the same ones. They are loaded here:

@AGENTS.md

## Claude Code specifics

- In a cloud session, `.claude/hooks/session-start.sh` runs at start-up. It installs ffmpeg when it is missing, installs `web/` dependencies and Playwright's Chromium, fetches the Rust crates and the Go toolchain and modules, so `make check` needs no first-use downloads, and installs the file-size pre-commit hook. It does nothing in a local session.
- Run `make check-fast` while iterating and `make check` before pushing. The first `cargo test` in a fresh session compiles everything and takes a few minutes.
- A cloud session cannot use Xcode or Computer Use. Follow the cloud-session rule under "Apple build and release workflow" above, and read Apple failures from the `apple / apple` job logs.
- Never bump `CURRENT_PROJECT_VERSION` or upload to TestFlight. A change ships when the Xcode Cloud build for its merge commit succeeds, as the TestFlight release completion gate above describes.
