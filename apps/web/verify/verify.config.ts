import { defineConfig, devices } from "@playwright/test";

// web:verify (issue #138, section 4): the shipped web app in real browsers,
// against the production image that tools/verify/web.mjs starts. The file is
// not named playwright.config.ts, so @nx/playwright does not infer an e2e
// target from it and `make check` does not run it without a container.
//
// tools/verify/web.mjs runs one profile at a time, each against a fresh
// container, and sets VERIFY_WEB_URL, VERIFY_WEB_PASSWORD,
// VERIFY_WEB_CONTAINER and VERIFY_WEB_OUTPUT.

const baseURL = process.env.VERIFY_WEB_URL;
if (!baseURL) throw new Error("run web:verify through tools/verify/web.mjs");

export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.ts",
  // The tests share one container and run in file order: the restart and
  // sign-out journeys change its state for whatever follows.
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  reporter: [["list"]],
  outputDir: process.env.VERIFY_WEB_OUTPUT ?? "../../output/verify/web",
  use: {
    baseURL,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    { name: "chromium-desktop", use: { ...devices["Desktop Chrome"] } },
    { name: "chromium-pixel", use: { ...devices["Pixel 7"] } },
    { name: "webkit-iphone", use: { ...devices["iPhone 15"] } },
  ],
});
