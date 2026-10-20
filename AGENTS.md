# WhatsApp Translator Operating Rules

## TestFlight release completion gate

- TestFlight builds of `WhatsAppTranslator` (iOS/iPadOS) and `BabelBridgeMac` (macOS) come from the Xcode Cloud workflow `Main – TestFlight (internal)` when a pull request merges to `main`.
- Agents must not bump `CURRENT_PROJECT_VERSION` and must not upload to TestFlight by hand. Xcode Cloud assigns the build number from the Next Build Number in App Store Connect. A manual upload collides with those numbers unless Next Build Number is raised first.
- A change counts as shipped when the Xcode Cloud build for its merge commit succeeds. The handoff reports that Xcode Cloud build number. App Store Connect processing and TestFlight availability checks are not required for completion.
- If the Xcode Cloud build for the merge commit does not succeed, do not describe the request as fully released; report the exact blocker and which platform remains outstanding.
- `MARKETING_VERSION` is still bumped deliberately when shipping a new version.

## Apple build and release workflow

- Pull requests should pass the Xcode Cloud workflow `PR – Build & Test`. That workflow is not a required GitHub check.
- Manual builds in the Xcode application stay appropriate for local debugging. Always use the Xcode application through Computer Use for local Apple builds, tests, archives, and signing. Do not use command-line xcodebuild or the build runner for these actions.
- Run storage preflight before building, use task-owned DerivedData and serial local tests, and preserve release archives and symbols.
