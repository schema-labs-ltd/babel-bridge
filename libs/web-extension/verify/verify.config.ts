import { chromium, defineConfig, devices } from "@playwright/test";
import { existsSync } from "node:fs";

// web-extension:verify (issue #138, section 5): the MCP Apps UI the server
// actually serves, rendered in verify/host.html with every tool call going to
// the real /mcp. tools/verify/web-extension.mjs signs in through OAuth and
// sets VERIFY_MCP_URL and VERIFY_MCP_TOKEN. The file is not named
// playwright.config.ts, so @nx/playwright infers no e2e target from it.

if (!process.env.VERIFY_MCP_URL) {
  throw new Error(
    "run web-extension:verify through tools/verify/web-extension.mjs",
  );
}

// As playwright.config.ts: CHROMIUM_PATH, then Playwright's own Chromium,
// then a system one.
const bundledChromium = chromium.executablePath();
const executablePath =
  process.env.CHROMIUM_PATH ||
  (existsSync(bundledChromium) ? bundledChromium : "/usr/bin/chromium");

export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.ts",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  reporter: [["list"]],
  outputDir:
    process.env.VERIFY_MCP_OUTPUT ?? "../../../output/verify/web-extension",
  use: {
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    viewport: { width: 1080, height: 800 },
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"], launchOptions: { executablePath } },
    },
  ],
});
