// server:verify — the production container smoke from issue #138, section 2.
//
// Builds the full production image (every Dockerfile stage), runs it the way
// Railway does, and drives the HTTP and WebSocket API as a client would. It
// then restarts on the same volume, upgrades from the base revision's image,
// checks graceful shutdown and the QR pairing variant. Only WA_FAKE_BRIDGE and
// WA_FAKE_AI stand in for WhatsApp and OpenAI, so nothing leaves the machine.
//
// Environment (all optional):
//   VERIFY_IMAGE        use this image instead of building the working tree
//   VERIFY_BASE_IMAGE   image for the upgrade step; otherwise built from
//   VERIFY_BASE_REF     a git revision (default: the merge base with origin/main;
//                       "none" skips the upgrade step)
//   VERIFY_EVIDENCE_DIR where the step log and redacted container logs go
//                       (default: output/verify/server)
//   VERIFY_DOCKER_BUILD_FLAGS extra `docker build` flags for this machine

import { execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { Client, redactor, step, until } from "./lib/client.mjs";
import * as container from "./lib/docker.mjs";
import { checkLogs } from "./lib/evidence.mjs";
import {
  connectedJourney,
  diagnosticsJourney,
  persistedJourney,
  publicSurface,
  sendJourney,
  setToneJourney,
  shapesJourney,
  signInJourney,
  translateJourney,
} from "./lib/journeys.mjs";

const evidence = process.env.VERIFY_EVIDENCE_DIR ?? "output/verify/server";
mkdirSync(evidence, { recursive: true });
const stepLog = join(evidence, "steps.log");
writeFileSync(stepLog, "");

const password = randomBytes(24).toString("base64url");
const tokens = new Set();
const redact = redactor(() => [password, ...tokens]);
const log = (line) => {
  const text = redact(line);
  console.log(text);
  appendFileSync(stepLog, `${text}\n`);
};

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();
const head = git("rev-parse", "--short=12", "HEAD");
const run = `verify-${head}-${randomBytes(3).toString("hex")}`;
const created = { containers: [], volumes: [], images: [] };

function track(client) {
  const signIn = client.signIn.bind(client);
  client.signIn = async (secret) => {
    await signIn(secret);
    tokens.add(client.token);
  };
  return client;
}

// Starts a container, waits for the image's own HEALTHCHECK, and signs in.
async function boot(label, { image, volume, env }) {
  const name = `${run}-${label}`;
  created.containers.push(name);
  if (!created.volumes.includes(volume)) created.volumes.push(volume);
  const started = await step(`start the ${label} container`, log, () =>
    container.startContainer({ name, image, volume, password, env }),
  );
  await step(`the ${label} container's HEALTHCHECK reports healthy`, log, () =>
    container.waitHealthy(name),
  );
  return { ...started, label, client: track(new Client(started.url)) };
}

async function saveLogs(label, name) {
  const text = redact(await container.logs(name));
  writeFileSync(join(evidence, `${label}.log`), text);
  return text;
}

// SIGTERM, as Railway sends on every deploy: the server must drain, stop the
// bridge and close SQLite, then exit 0 inside its own 12 s budget.
async function stopCleanly(box) {
  await step(
    `SIGTERM stops the ${box.label} container cleanly`,
    log,
    async () => {
      const result = await container.stop(box.name);
      log(
        `  stopped in ${result.seconds.toFixed(1)} s, exit ${result.exitCode}`,
      );
      if (result.exitCode !== 0 || result.oomKilled) {
        throw new Error(`exit code ${result.exitCode}`);
      }
      if (result.seconds > 15) throw new Error("shutdown took too long");
    },
  );
  await step(`the ${box.label} container's logs are clean`, log, async () =>
    checkLogs(await saveLogs(box.label, box.name)),
  );
}

// The revision the upgrade step starts from. In CI it is the base branch
// commit the PR merges into; locally the merge base with origin/main.
function baseRef() {
  const wanted = process.env.VERIFY_BASE_REF;
  if (wanted === "none") return null;
  if (wanted) return git("rev-parse", "--verify", `${wanted}^{commit}`);
  try {
    return git("merge-base", "HEAD", "origin/main");
  } catch {
    return null;
  }
}

async function main() {
  const image = process.env.VERIFY_IMAGE || `whatsapp-translator:${run}`;
  if (!process.env.VERIFY_IMAGE) {
    created.images.push(image);
    await step("build the production image", log, () =>
      container.buildImage({ tag: image, ref: null, log }),
    );
  }
  const volume = `${run}-data`;

  // 1. A fresh deploy.
  const first = await boot("first", { image, volume });
  await publicSurface(first.client, log);
  await signInJourney(first.client, password, log);
  await connectedJourney(first.client, log);
  const messageId = await sendJourney(
    first.client,
    log,
    `verify ${run}: hola desde la verificación`,
  );
  await translateJourney(first.client, log);
  await setToneJourney(first.client, log, "aurora");
  await diagnosticsJourney(first.client, log, { wrongPasswordTried: true });
  await shapesJourney(first.client, log);
  await step("the bridge runs as a child process", log, async () => {
    const top = await container.processes(first.name);
    if (!/wa-bridge/.test(top)) throw new Error("no wa-bridge process");
  });
  await stopCleanly(first);

  // 2. A redeploy on the same volume keeps messages and settings. Web
  // sessions live in memory, so the client signs in again.
  const second = await boot("restart", { image, volume });
  await step("sign in to the restarted container", log, () =>
    second.client.signIn(password),
  );
  await connectedJourney(second.client, log);
  await persistedJourney(second.client, log, {
    messageId,
    tone: "aurora",
    translation: true,
  });
  await diagnosticsJourney(second.client, log);
  await shapesJourney(second.client, log);
  await stopCleanly(second);

  // 3. The upgrade path: data written by the base revision's image survives
  // this image's migrations.
  const ref = process.env.VERIFY_BASE_IMAGE ? null : baseRef();
  const baseImage =
    process.env.VERIFY_BASE_IMAGE ||
    (ref ? `whatsapp-translator:${run}-base` : null);
  if (!baseImage) {
    log("- upgrade path skipped: no base image or revision");
  } else {
    if (ref) {
      created.images.push(baseImage);
      await step(
        `build the base revision's image (${ref.slice(0, 12)})`,
        log,
        () => container.buildImage({ tag: baseImage, ref, log }),
      );
    }
    const upgradeVolume = `${run}-upgrade`;
    const old = await boot("base", { image: baseImage, volume: upgradeVolume });
    await step("sign in to the base image", log, () =>
      old.client.signIn(password),
    );
    await connectedJourney(old.client, log);
    const oldMessage = await sendJourney(
      old.client,
      log,
      `verify ${run}: before upgrade`,
    );
    await stopCleanly(old);
    const upgraded = await boot("upgraded", { image, volume: upgradeVolume });
    await step("sign in to the upgraded container", log, () =>
      upgraded.client.signIn(password),
    );
    await connectedJourney(upgraded.client, log);
    await persistedJourney(upgraded.client, log, { messageId: oldMessage });
    await diagnosticsJourney(upgraded.client, log);
    await shapesJourney(upgraded.client, log);
    await stopCleanly(upgraded);
  }

  // 4. WA_FAKE_BRIDGE=qr shows a pairing code for 5 s before connecting, so
  // this signs in as soon as HTTP answers rather than waiting for the
  // HEALTHCHECK, which it checks afterwards.
  const name = `${run}-qr`;
  created.containers.push(name);
  created.volumes.push(`${run}-qr`);
  const started = await step("start the qr container", log, () =>
    container.startContainer({
      name,
      image,
      volume: `${run}-qr`,
      password,
      env: { WA_FAKE_BRIDGE: "qr" },
    }),
  );
  const qr = {
    ...started,
    label: "qr",
    client: track(new Client(started.url)),
  };
  await step("the qr container answers HTTP", log, () =>
    until(
      "health",
      30_000,
      async () => {
        const response = await qr.client.request("GET", "/api/health", {
          auth: false,
        });
        return response.status === 200;
      },
      100,
    ),
  );
  await step("sign in to the qr container", log, () =>
    qr.client.signIn(password),
  );
  await step("/api/qr returns a code before connecting", log, () =>
    until(
      "a QR code",
      5_000,
      async () => {
        const { json } = await qr.client.json("GET", "/api/qr", {
          fixture: "qr",
        });
        return typeof json.qr === "string" && json.qr.length > 0;
      },
      100,
    ),
  );
  await connectedJourney(qr.client, log);
  await step("the qr container's HEALTHCHECK reports healthy", log, () =>
    container.waitHealthy(name),
  );
  await diagnosticsJourney(qr.client, log);
  await shapesJourney(qr.client, log);
  await stopCleanly(qr);
}

// An interrupted run still removes what it created.
let cleaning = false;
async function cleanUp() {
  if (cleaning) return;
  cleaning = true;
  await container.remove(created);
}
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.once(signal, async () => {
    log(`server:verify interrupted by ${signal}`);
    await cleanUp();
    process.exit(130);
  });
}

const startedAt = Date.now();
try {
  await main();
  log(
    `server:verify passed in ${Math.round((Date.now() - startedAt) / 1000)} s`,
  );
} catch (error) {
  log(`server:verify FAILED at "${error.step ?? "setup"}": ${error.message}`);
  for (const name of created.containers) {
    await saveLogs(name.slice(run.length + 1), name).catch(() => {});
  }
  process.exitCode = 1;
} finally {
  await cleanUp();
}
