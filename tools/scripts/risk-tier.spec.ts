import assert from "node:assert/strict";
import { test } from "node:test";
import {
  classify,
  globToRegExp,
  HIGH_RISK_PATTERNS,
  LOW_RISK_PATTERNS,
  TEST_PATTERNS,
} from "./risk-tier.mjs";

test("matches globs across and within path segments", () => {
  assert.equal(globToRegExp("**/AGENTS.md").test("AGENTS.md"), true);
  assert.equal(globToRegExp("**/AGENTS.md").test("ios/AGENTS.md"), true);
  assert.equal(
    globToRegExp(".github/**").test(".github/workflows/a.yml"),
    true,
  );
  assert.equal(globToRegExp("**/*.md").test("docs/plans/a.md"), true);
  assert.equal(globToRegExp("ios/*Tests/**").test("ios/a/bTests/x"), false);
  assert.equal(globToRegExp("Cargo.lock").test("wa-bridge/Cargo.lock"), false);
  assert.equal(
    globToRegExp("scripts/check-*").test("scripts/check-a.sh"),
    true,
  );
});

test("marks docs, tests and screenshots as low risk", () => {
  assert.equal(
    classify([
      "docs/index.html",
      "README.md",
      "ios/README.md",
      "web/tests/topics.test.js",
      "wa-bridge/edits_test.go",
      "src/chatgpt/tests.rs",
      "src/identity_tests.rs",
      "src/web/tests/chats.rs",
      "src/voice/integration_tests/ogg.rs",
      "web/extensions/tests/chats.spec.ts",
      "ios/WhatsAppTranslatorTests/FoldableLayoutTests.swift",
      "assets/screenshot.png",
      "docs/images/hero.jpg",
    ]).tier,
    "low",
  );
});

test("marks ordinary product changes as medium risk", () => {
  for (const file of [
    "src/main.rs",
    "src/storage/messages.rs",
    "src/web/chats.rs",
    "src/translation.rs",
    "wa-bridge/main.go",
    "web/public/app.js",
    "web/public/styles-base.css",
    "web/public/sounds/tone.wav",
    "web/extensions/dist/chats.html",
    "ios/WhatsAppTranslator/App/AppSession.swift",
    "ios/APIContract/APIContractCheck.swift",
    "scripts/generate-message-tones.py",
    "build.rs",
  ]) {
    assert.equal(classify([file]).tier, "medium", file);
  }
  assert.equal(
    classify(["src/web/chats.rs", "src/web/tests/chats.rs"]).tier,
    "medium",
  );
});

test("marks any high-risk file as high risk and names it", () => {
  const result = classify([
    "docs/index.html",
    "src/storage/schema/column_migrations.rs",
    ".github/workflows/ci.yml",
  ]);
  assert.equal(result.tier, "high");
  assert.deepEqual(result.reasons, [
    "src/storage/schema/column_migrations.rs",
    ".github/workflows/ci.yml",
  ]);
});

test("treats agent rules as high risk even though they are Markdown", () => {
  assert.equal(classify(["AGENTS.md"]).tier, "high");
  assert.equal(classify(["ios/CLAUDE.md"]).tier, "high");
  assert.equal(classify(["REVIEW.md"]).tier, "high");
  assert.equal(classify([".github/pull_request_template.md"]).tier, "high");
});

// One path for every high-risk pattern, so dropping or breaking a pattern
// fails here.
const HIGH_RISK_EXAMPLES = {
  ".github/**": ".github/workflows/risk-gate.yml",
  ".claude/**": ".claude/hooks/session-start.sh",
  "**/AGENTS.md": "wa-bridge/AGENTS.md",
  "**/CLAUDE.md": "CLAUDE.md",
  "REVIEW.md": "REVIEW.md",
  "tools/scripts/**": "tools/scripts/risk-tier.mjs",
  "scripts/check-*": "scripts/check-file-size.sh",
  Makefile: "Makefile",
  "nx.json": "nx.json",
  "**/project.json": "web/extensions/project.json",
  ".nxignore": ".nxignore",
  "lefthook.yml": "lefthook.yml",
  ".gitattributes": ".gitattributes",
  "**/.commitlintrc*": ".commitlintrc.json",
  "**/commitlint.config.*": "commitlint.config.mjs",
  "**/Cargo.toml": "Cargo.toml",
  "Cargo.lock": "Cargo.lock",
  "rust-toolchain.toml": "rust-toolchain.toml",
  "**/go.mod": "wa-bridge/go.mod",
  "**/go.sum": "wa-bridge/go.sum",
  "**/package.json": "web/package.json",
  "**/package-lock.json": "web/package-lock.json",
  "**/Package.swift": "ios/Shared/Package.swift",
  "**/Package.resolved": "ios/Package.resolved",
  "**/Dockerfile": "Dockerfile",
  ".dockerignore": ".dockerignore",
  "railway.toml": "railway.toml",
  "**/.env*": ".env.example",
  "src/storage/schema.rs": "src/storage/schema.rs",
  "src/storage/schema/**": "src/storage/schema/oauth_migrations.rs",
  "src/access.rs": "src/access.rs",
  "src/oauth.rs": "src/oauth.rs",
  "src/web/auth.rs": "src/web/auth.rs",
  "src/web/oauth.rs": "src/web/oauth.rs",
  "src/web/oauth/**": "src/web/oauth/tokens.rs",
  "src/storage/oauth.rs": "src/storage/oauth.rs",
  "src/web/server.rs": "src/web/server.rs",
  "src/web/mcp_http.rs": "src/web/mcp_http.rs",
  "src/mcp.rs": "src/mcp.rs",
  "src/chatgpt/sign_in.rs": "src/chatgpt/sign_in.rs",
  "src/chatgpt/tokens.rs": "src/chatgpt/tokens.rs",
  "src/log_privacy.rs": "src/log_privacy.rs",
  "src/push.rs": "src/push.rs",
  "src/push/**": "src/push/apns.rs",
  "fixtures/api/**": "fixtures/api/responses/chats.json",
  "ios/project.yml": "ios/project.yml",
  "ios/*.xcodeproj/**": "ios/WhatsAppTranslator.xcodeproj/project.pbxproj",
  "ios/ci_scripts/**": "ios/ci_scripts/ci_post_clone.sh",
  "**/*.entitlements": "ios/BabelBridgeMac/BabelBridgeMac.entitlements",
};

test("covers every high-risk pattern", () => {
  assert.deepEqual(Object.keys(HIGH_RISK_EXAMPLES), HIGH_RISK_PATTERNS);
  for (const [pattern, file] of Object.entries(HIGH_RISK_EXAMPLES)) {
    assert.equal(globToRegExp(pattern).test(file), true, pattern);
    assert.deepEqual(classify([file]), { tier: "high", reasons: [file] });
  }
});

test("does not count tests in high-risk paths as high risk", () => {
  for (const file of [
    "src/push/tests.rs",
    "src/web/oauth/tests/tokens.rs",
    "tools/scripts/risk-tier.spec.ts",
    ".github/scripts/labels.test.mjs",
  ]) {
    assert.equal(classify([file]).tier, "low", file);
  }
  assert.equal(classify(["src/push/tests.rs", "src/push.rs"]).tier, "high");
  assert.ok(LOW_RISK_PATTERNS.includes(TEST_PATTERNS[0]));
});

test("matches paths that contain line terminators", () => {
  for (const file of [
    ".github/workflows/\u2028publish.yml",
    "src/web/oauth/\u2029tokens.rs",
    ".github/\rci.yml",
  ]) {
    assert.equal(classify([file]).tier, "high", JSON.stringify(file));
  }
  assert.equal(globToRegExp("docs/*").test("docs/a\u2028b"), true);
});

test("treats an empty file list as medium risk", () => {
  assert.equal(classify([]).tier, "medium");
});
