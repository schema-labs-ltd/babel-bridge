import { chromium, defineConfig } from "@playwright/test";
import { existsSync } from "node:fs";

// CHROMIUM_PATH wins (CI sets it); otherwise Playwright's own Chromium, then
// a system one.
const bundledChromium = chromium.executablePath();
const executablePath =
  process.env.CHROMIUM_PATH ||
  (existsSync(bundledChromium) ? bundledChromium : "/usr/bin/chromium");

export default defineConfig({
  testDir: "./tests",
  testMatch: "**/*.spec.ts",
  fullyParallel: false,
  workers: 1,
  timeout: 20000,
  reporter: "list",
  outputDir: "../../output/playwright/extensions",
  use: {
    browserName: "chromium",
    launchOptions: {
      executablePath,
      args: ["--no-sandbox"],
    },
    viewport: { width: 1080, height: 800 },
  },
});
