# WhatsApp Translator Operating Rules

## TestFlight release completion gate

- TestFlight builds of `WhatsAppTranslator` (iOS/iPadOS) and `BabelBridgeMac` (macOS) come from the Xcode Cloud workflow `Main – TestFlight (internal)` when a pull request merges to `main`.
- Agents must not bump `CURRENT_PROJECT_VERSION` and must not upload to TestFlight by hand. Xcode Cloud assigns the build number from the Next Build Number in App Store Connect. A manual upload collides with those numbers unless Next Build Number is raised first.
- A change counts as shipped when the Xcode Cloud build for its merge commit succeeds. The handoff reports that Xcode Cloud build number. App Store Connect processing and TestFlight availability checks are not required for completion.
- If the Xcode Cloud build for the merge commit does not succeed, do not describe the request as fully released; report the exact blocker and which platform remains outstanding.
- `MARKETING_VERSION` is still bumped deliberately when shipping a new version.

## Apple build and release workflow

- GitHub Actions runs the `apple` workflow on pull requests and pushes to `main`. The stable check is `apple / apple`. It builds `WhatsAppTranslator` and `BabelBridgeMac` with signing disabled and runs `WhatsAppTranslatorUnitTests` and `BabelBridgeMacTests` when `ios/`, `web/public/sounds/`, or `.github/workflows/apple.yml` changes, and passes without a build on every other change, so branch protection can require it. The commands are in `ios/README.md`. Local agent builds follow the next rule.
- Xcode Cloud workflow B, `Main – TestFlight (internal)`, archives merges to `main` for internal TestFlight. The one-time setup for that workflow is in `ios/README.md`. Pull-request build and test stays on the GitHub Actions check.
- Manual builds in the Xcode application stay appropriate for local debugging. Always use the Xcode application through Computer Use for local Apple builds, tests, archives, and signing. Do not use command-line xcodebuild or the build runner for these actions.
- Run storage preflight before building, use task-owned DerivedData and serial local tests, and preserve release archives and symbols.
- iPhone Duo behaviour lives in `ios/WhatsAppTranslator/Design/FoldableDisplaySupport.swift` behind `#if compiler(>=6.4)` and runtime availability checks, with the pure rules in `FoldableLayoutModel.swift`. Keep every use of an iOS 27 or 27.1 API inside those guards so Xcode 26.6 keeps building, keep `FoldableLayoutModel.swift` free of SwiftUI so `FoldableLayoutTests` runs in both unit-test targets, and build or archive with Xcode 27.1 or later when the change must be checked on iPhone Duo. Do not raise the GitHub Actions Xcode pin until the `macos-26` runner image provides that Xcode release.

## File size limit

- A Lefthook pre-commit hook (`lefthook.yml`, `scripts/check-file-size.sh`) blocks any commit that stages a hand-written text file over 5,000 tokens, counted as bytes / 4 (20,000 bytes). Install it once per clone with `npx lefthook install` (or `lefthook install`).
- It checks only staged files, so an existing oversized file blocks the commit only when it is edited. When that happens, split the file as the hook's message describes, in the same commit; do not bypass the hook with `--no-verify` or trim content to fit.
- Binary files and files marked `linguist-generated` in `.gitattributes` (lockfiles, `web/extensions/dist/`, Xcode project and asset catalog files) are skipped. Mark new generated files there.
- Run `scripts/check-file-size.sh $(git ls-files)` to list every file currently over the limit.
