// web:verify — the web app in real browsers, from issue #138, section 4.
//
// Starts the production image as server:verify does (sign-in on, a volume on
// /data, WA_FAKE_BRIDGE and WA_FAKE_AI), then runs the Playwright journeys in
// apps/web/verify on each browser profile: Chromium desktop, Chromium on a
// Pixel and WebKit on an iPhone. Each profile gets a fresh container on a
// fixed port, because the journeys restart it, sign out and change settings.
// Storybook is built first through the target's dependsOn.
//
// Environment (all optional):
//   VERIFY_IMAGE        use this image instead of building the working tree
//   VERIFY_WEB_PROFILES comma-separated profiles to run (default: all three)
//   VERIFY_EVIDENCE_DIR where the step log, container logs and Playwright
//                       results go (default: output/verify/web)
//   VERIFY_DOCKER_BUILD_FLAGS extra `docker build` flags for this machine

import { execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { appendFileSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { step } from "./lib/client.mjs";
import * as container from "./lib/docker.mjs";
import { checkLogs, redactEvidence, redactText } from "./lib/evidence.mjs";
import { FAKE } from "./lib/journeys.mjs";
import { freePort, runPlaywright } from "./lib/playwright.mjs";

export const PROFILES = ["chromium-desktop", "chromium-pixel", "webkit-iphone"];

// Signing out stops the bridge, and the server's next command to it fails
// with this before the bridge restarts. It is harmless and happens once per
// sign-out; any other occurrence still fails the check.
const SIGN_OUT_LINE = /Failed to write command to bridge: Broken pipe/;

const evidence = resolve(
  process.env.VERIFY_EVIDENCE_DIR ?? "output/verify/web",
);
rmSync(evidence, { recursive: true, force: true });
mkdirSync(evidence, { recursive: true });
const stepLog = join(evidence, "steps.log");

const password = randomBytes(24).toString("base64url");
const secrets = [password, ...FAKE.phones];
// Container logs keep the phone numbers, so checkLogs can see a leak.
const redact = (text) => redactText(text, [password]);
const log = (line) => {
  const text = redactText(line, secrets);
  console.log(text);
  appendFileSync(stepLog, `${text}\n`);
};

const head = execFileSync("git", ["rev-parse", "--short=12", "HEAD"], {
  encoding: "utf8",
}).trim();
const run = `verify-web-${head}-${randomBytes(3).toString("hex")}`;
const created = { containers: [], volumes: [], images: [] };

async function profileRun(image, profile) {
  const name = `${run}-${profile}`;
  const volume = `${name}-data`;
  created.containers.push(name);
  created.volumes.push(volume);
  const port = await freePort();
  const started = await step(`start the ${profile} container`, log, () =>
    container.startContainer({ name, image, volume, password, port }),
  );
  await step(
    `the ${profile} container's HEALTHCHECK reports healthy`,
    log,
    () => container.waitHealthy(name),
  );
  const output = join(evidence, profile);
  try {
    await step(`the browser journeys pass on ${profile}`, log, () =>
      runPlaywright({
        config: "apps/web/verify/verify.config.ts",
        project: profile,
        env: {
          VERIFY_WEB_URL: started.url,
          VERIFY_WEB_PASSWORD: password,
          VERIFY_WEB_CONTAINER: name,
          VERIFY_WEB_OUTPUT: output,
        },
        log,
      }),
    );
  } finally {
    // Redacted straight away, so a cancelled job leaves no raw trace behind.
    await secure(output);
  }
  await step(
    `SIGTERM stops the ${profile} container cleanly`,
    log,
    async () => {
      const result = await container.stop(name);
      if (result.exitCode !== 0)
        throw new Error(`exit code ${result.exitCode}`);
    },
  );
  await step(`the ${profile} container's logs are clean`, log, async () => {
    const text = redact(await container.logs(name));
    writeFileSync(join(evidence, `${profile}.log`), text);
    const signOutLines = text.match(new RegExp(SIGN_OUT_LINE, "g")) ?? [];
    if (signOutLines.length > 1) {
      throw new Error(`${signOutLines.length} broken pipes to the bridge`);
    }
    checkLogs(text, { allow: [SIGN_OUT_LINE] });
  });
}

async function main() {
  const wanted = (process.env.VERIFY_WEB_PROFILES ?? PROFILES.join(","))
    .split(",")
    .filter(Boolean);
  const unknown = wanted.filter((profile) => !PROFILES.includes(profile));
  if (unknown.length > 0) throw new Error(`unknown profiles: ${unknown}`);
  const image = process.env.VERIFY_IMAGE || `whatsapp-translator:${run}`;
  if (!process.env.VERIFY_IMAGE) {
    created.images.push(image);
    await step("build the production image", log, () =>
      container.buildImage({ tag: image, ref: null, log }),
    );
  }
  for (const profile of wanted) await profileRun(image, profile);
}

// Traces record typed text, requests, console lines and the page, so the
// password, every session token and the fake phone numbers the app shows
// are removed before the evidence is uploaded. Anything that cannot be
// cleaned is deleted and fails the run.
async function secure(dir) {
  try {
    const leaks = await redactEvidence(dir, secrets);
    if (leaks.length === 0) return;
    for (const path of leaks) rmSync(path, { force: true });
    log(`removed evidence that could not be cleaned: ${leaks.join(" ")}`);
  } catch (error) {
    rmSync(dir, { recursive: true, force: true });
    mkdirSync(dir, { recursive: true });
    log(`removed ${dir}: redaction failed (${error.message})`);
  }
  process.exitCode = 1;
}

let cleaning = false;
async function cleanUp() {
  if (cleaning) return;
  cleaning = true;
  await container.remove(created).catch(() => {});
  await secure(evidence);
}
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.once(signal, async () => {
    log(`web:verify interrupted by ${signal}`);
    try {
      await cleanUp();
    } finally {
      process.exit(130);
    }
  });
}

const startedAt = Date.now();
try {
  await main();
} catch (error) {
  log(`web:verify FAILED at "${error.step ?? "setup"}": ${error.message}`);
  for (const name of created.containers) {
    const label = name.slice(run.length + 1);
    await container
      .logs(name)
      .then((text) =>
        writeFileSync(join(evidence, `${label}.log`), redact(text)),
      )
      .catch(() => {});
  }
  process.exitCode = 1;
} finally {
  await cleanUp();
}
// Only now, since securing the evidence can still fail the run.
if (!process.exitCode) {
  log(`web:verify passed in ${Math.round((Date.now() - startedAt) / 1000)} s`);
}
