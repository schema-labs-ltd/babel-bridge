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
  assert.equal(globToRegExp("**/AGENTS.md").test("apps/ios/AGENTS.md"), true);
  assert.equal(
    globToRegExp(".github/**").test(".github/workflows/a.yml"),
    true,
  );
  assert.equal(globToRegExp("**/*.md").test("docs/plans/a.md"), true);
  assert.equal(
    globToRegExp("apps/ios/*Tests/**").test("apps/ios/a/bTests/x"),
    false,
  );
  assert.equal(
    globToRegExp("Cargo.lock").test("apps/wa-bridge/Cargo.lock"),
    false,
  );
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
      "apps/ios/README.md",
      "apps/web/tests/topics.test.js",
      "apps/wa-bridge/edits_test.go",
      "apps/server/src/chatgpt/tests.rs",
      "apps/server/src/identity_tests.rs",
      "apps/server/src/web/tests/chats.rs",
      "apps/server/src/voice/integration_tests/ogg.rs",
      "libs/web-extension/tests/chats.spec.ts",
      "apps/ios/WhatsAppTranslatorTests/FoldableLayoutTests.swift",
      "assets/screenshot.png",
      "docs/images/hero.jpg",
    ]).tier,
    "low",
  );
});

test("marks ordinary product changes as medium risk", () => {
  for (const file of [
    "apps/server/src/main.rs",
    "apps/server/src/storage/messages.rs",
    "apps/server/src/web/chats.rs",
    "apps/server/src/translation.rs",
    "apps/wa-bridge/main.go",
    "apps/web/public/app.js",
    "apps/web/public/styles-base.css",
    "apps/web/public/sounds/tone.wav",
    "libs/web-extension/dist/chats.html",
    "apps/ios/WhatsAppTranslator/App/AppSession.swift",
    "apps/ios/APIContract/APIContractCheck.swift",
    "scripts/generate-message-tones.py",
    "apps/server/build.rs",
  ]) {
    assert.equal(classify([file]).tier, "medium", file);
  }
  assert.equal(
    classify([
      "apps/server/src/web/chats.rs",
      "apps/server/src/web/tests/chats.rs",
    ]).tier,
    "medium",
  );
});

test("marks any high-risk file as high risk and names it", () => {
  const result = classify([
    "docs/index.html",
    "apps/server/src/storage/schema/column_migrations.rs",
    ".github/workflows/ci.yml",
  ]);
  assert.equal(result.tier, "high");
  assert.deepEqual(result.reasons, [
    "apps/server/src/storage/schema/column_migrations.rs",
    ".github/workflows/ci.yml",
  ]);
});

test("treats agent rules as high risk even though they are Markdown", () => {
  assert.equal(classify(["AGENTS.md"]).tier, "high");
  assert.equal(classify(["apps/ios/CLAUDE.md"]).tier, "high");
  assert.equal(classify(["REVIEW.md"]).tier, "high");
  assert.equal(classify([".github/pull_request_template.md"]).tier, "high");
});

// One path for every high-risk pattern, so dropping or breaking a pattern
// fails here.
const HIGH_RISK_EXAMPLES = {
  ".github/**": ".github/workflows/risk-gate.yml",
  ".claude/**": ".claude/hooks/session-start.sh",
  "**/AGENTS.md": "apps/wa-bridge/AGENTS.md",
  "**/CLAUDE.md": "CLAUDE.md",
  "REVIEW.md": "REVIEW.md",
  "tools/scripts/**": "tools/scripts/risk-tier.mjs",
  "scripts/check-*": "scripts/check-file-size.sh",
  Makefile: "Makefile",
  "nx.json": "nx.json",
  "**/project.json": "libs/web-extension/project.json",
  ".nxignore": ".nxignore",
  "lefthook.yml": "lefthook.yml",
  ".gitattributes": ".gitattributes",
  "**/.commitlintrc*": ".commitlintrc.json",
  "**/commitlint.config.*": "commitlint.config.mjs",
  "**/Cargo.toml": "Cargo.toml",
  "Cargo.lock": "Cargo.lock",
  "rust-toolchain.toml": "rust-toolchain.toml",
  "**/go.mod": "apps/wa-bridge/go.mod",
  "**/go.sum": "apps/wa-bridge/go.sum",
  "**/package.json": "apps/web/package.json",
  "**/package-lock.json": "package-lock.json",
  "**/Package.swift": "apps/ios/Shared/Package.swift",
  "**/Package.resolved": "apps/ios/Package.resolved",
  "**/Dockerfile": "Dockerfile",
  ".dockerignore": ".dockerignore",
  "railway.toml": "railway.toml",
  "**/.env*": ".env.example",
  "apps/server/src/storage/schema.rs": "apps/server/src/storage/schema.rs",
  "apps/server/src/storage/schema/**":
    "apps/server/src/storage/schema/oauth_migrations.rs",
  "apps/server/src/access.rs": "apps/server/src/access.rs",
  "apps/server/src/oauth.rs": "apps/server/src/oauth.rs",
  "apps/server/src/web/auth.rs": "apps/server/src/web/auth.rs",
  "apps/server/src/web/oauth.rs": "apps/server/src/web/oauth.rs",
  "apps/server/src/web/oauth/**": "apps/server/src/web/oauth/tokens.rs",
  "apps/server/src/storage/oauth.rs": "apps/server/src/storage/oauth.rs",
  "apps/server/src/web/server.rs": "apps/server/src/web/server.rs",
  "apps/server/src/web/mcp_http.rs": "apps/server/src/web/mcp_http.rs",
  "apps/server/src/mcp.rs": "apps/server/src/mcp.rs",
  "apps/server/src/chatgpt/sign_in.rs": "apps/server/src/chatgpt/sign_in.rs",
  "apps/server/src/chatgpt/tokens.rs": "apps/server/src/chatgpt/tokens.rs",
  "apps/server/src/log_privacy.rs": "apps/server/src/log_privacy.rs",
  "apps/server/src/push.rs": "apps/server/src/push.rs",
  "apps/server/src/push/**": "apps/server/src/push/apns.rs",
  "fixtures/api/**": "fixtures/api/responses/chats.json",
  "apps/ios/project.yml": "apps/ios/project.yml",
  "apps/ios/*.xcodeproj/**":
    "apps/ios/WhatsAppTranslator.xcodeproj/project.pbxproj",
  "apps/ios/ci_scripts/**": "apps/ios/ci_scripts/ci_post_clone.sh",
  "**/*.entitlements": "apps/ios/BabelBridgeMac/BabelBridgeMac.entitlements",
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
    "apps/server/src/push/tests.rs",
    "apps/server/src/web/oauth/tests/tokens.rs",
    "tools/scripts/risk-tier.spec.ts",
    ".github/scripts/labels.test.mjs",
  ]) {
    assert.equal(classify([file]).tier, "low", file);
  }
  assert.equal(
    classify(["apps/server/src/push/tests.rs", "apps/server/src/push.rs"]).tier,
    "high",
  );
  assert.ok(LOW_RISK_PATTERNS.includes(TEST_PATTERNS[0]));
});

test("matches paths that contain line terminators", () => {
  for (const file of [
    ".github/workflows/\u2028publish.yml",
    "apps/server/src/web/oauth/\u2029tokens.rs",
    ".github/\rci.yml",
  ]) {
    assert.equal(classify([file]).tier, "high", JSON.stringify(file));
  }
  assert.equal(globToRegExp("docs/*").test("docs/a\u2028b"), true);
});

test("treats an empty file list as medium risk", () => {
  assert.equal(classify([]).tier, "medium");
});
