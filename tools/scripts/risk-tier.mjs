import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Picks a PR's risk tier from the files it changes. The tiers and what each
// needs before merge are described in AGENTS.md ("Risk Tiers & Merging").
// risk-gate.yml runs this from the base branch, so a PR cannot loosen the
// rules that classify it. Agents may raise a PR's tier, never lower it.

// Any match makes the PR high risk, so a person approves it before merge.
export const HIGH_RISK_PATTERNS = [
  // The gates themselves: CI, agent rules, hooks and verification scripts.
  ".github/**",
  ".claude/**",
  "**/AGENTS.md",
  "**/CLAUDE.md",
  "REVIEW.md",
  "tools/scripts/**",
  "scripts/check-*",
  "Makefile",
  // Nx task definitions, which hold the commands CI and `make check` run.
  "nx.json",
  "**/project.json",
  ".nxignore",
  "lefthook.yml",
  ".gitattributes",
  "**/.commitlintrc*",
  "**/commitlint.config.*",
  // Dependencies, toolchains and deploy configuration.
  "**/Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  "**/go.mod",
  "**/go.sum",
  "**/package.json",
  "**/package-lock.json",
  "**/Package.swift",
  "**/Package.resolved",
  "**/Dockerfile",
  ".dockerignore",
  "railway.toml",
  "**/.env*",
  // Storage schema and migrations, which run on existing databases at start-up.
  "apps/server/src/storage/schema.rs",
  "apps/server/src/storage/schema/**",
  // Sign-in, OAuth and MCP permissions, and which routes need them.
  "apps/server/src/access.rs",
  "apps/server/src/oauth.rs",
  "apps/server/src/web/auth.rs",
  "apps/server/src/web/oauth.rs",
  "apps/server/src/web/oauth/**",
  "apps/server/src/storage/oauth.rs",
  "apps/server/src/web/server.rs",
  "apps/server/src/web/mcp_http.rs",
  "apps/server/src/mcp.rs",
  "apps/server/src/chatgpt/sign_in.rs",
  "apps/server/src/chatgpt/tokens.rs",
  // Private content: log redaction and APNs push.
  "apps/server/src/log_privacy.rs",
  "apps/server/src/push.rs",
  "apps/server/src/push/**",
  // The API the shipped Apple apps decode.
  "fixtures/api/**",
  // Apple signing and release.
  "apps/ios/project.yml",
  "apps/ios/*.xcodeproj/**",
  "apps/ios/ci_scripts/**",
  "**/*.entitlements",
];

// Tests only check the code beside them, so they never make a PR high risk,
// even in a high-risk path.
export const TEST_PATTERNS = [
  "**/*.spec.ts",
  "**/*.test.js",
  "**/*.test.mjs",
  "**/*_test.go",
  "apps/server/src/**/tests.rs",
  "apps/server/src/**/*_tests.rs",
  "apps/server/src/**/tests/**",
  "apps/server/src/**/*_tests/**",
  "apps/web/tests/**",
  "libs/web-extension/tests/**",
  "apps/ios/*Tests/**",
];

// A PR is low risk only when every file it changes matches one of these.
export const LOW_RISK_PATTERNS = [
  ...TEST_PATTERNS,
  "docs/**",
  "**/*.md",
  "**/*.png",
  "**/*.jpg",
  "**/*.jpeg",
];

// Turns a glob into a RegExp. `**/` matches any number of directories, `**`
// anything, and `*` anything within one path segment. The `s` flag lets
// both match line terminators such as U+2028, which paths may contain.
export function globToRegExp(glob) {
  let source = "";
  for (let i = 0; i < glob.length; i++) {
    const char = glob[i];
    if (glob.startsWith("**/", i)) {
      source += "(?:.*/)?";
      i += 2;
    } else if (glob.startsWith("**", i)) {
      source += ".*";
      i += 1;
    } else if (char === "*") {
      source += "[^/]*";
    } else {
      source += char.replace(/[.+?^${}()|[\]\\]/g, "\\$&");
    }
  }
  return new RegExp(`^${source}$`, "s");
}

const matcher = (patterns) => {
  const regexps = patterns.map(globToRegExp);
  return (file) => regexps.some((regexp) => regexp.test(file));
};

const isTest = matcher(TEST_PATTERNS);
const matchesHighRisk = matcher(HIGH_RISK_PATTERNS);
const isHighRisk = (file) => !isTest(file) && matchesHighRisk(file);
const isLowRisk = matcher(LOW_RISK_PATTERNS);

export function classify(files) {
  const high = files.filter(isHighRisk);
  if (high.length > 0) return { tier: "high", reasons: high };
  if (files.length > 0 && files.every(isLowRisk)) {
    return { tier: "low", reasons: [] };
  }
  return { tier: "medium", reasons: [] };
}

// Reads changed file paths, one per line, from the file given or stdin.
function main([input = "/dev/stdin"]) {
  const files = readFileSync(input, "utf8")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  const { tier, reasons } = classify(files);
  return [`tier=${tier}`, ...reasons.map((file) => `high-risk: ${file}`)].join(
    "\n",
  );
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    console.log(main(process.argv.slice(2)));
  } catch (error) {
    console.error(`Risk tier failed: ${error.message}`);
    process.exitCode = 1;
  }
}
