// wa-bridge:verify — the Go bridge as shipped, from issue #138, section 3.
//
// Builds the bridge exactly as the Dockerfile's go-builder stage does
// (golang:1.27-bookworm, CGO_ENABLED=1, -ldflags="-s -w"), then runs that
// binary in fake mode and speaks the JSON line protocol the Rust server uses:
// every command in fixtures/bridge/commands goes in on stdin, and every event
// that comes out on stdout must parse and match the shape of the
// fixtures/bridge/events files of its type. Real WhatsApp is out of scope;
// session_upgrade_test.go guards whatsmeow session upgrades.
//
// Environment (all optional):
//   VERIFY_BRIDGE_IMAGE  an image that already holds the binary
//   VERIFY_BRIDGE_PATH   the binary's path in that image (default /build/wa-bridge)
//   VERIFY_EVIDENCE_DIR  where the step log and event transcripts go
//                        (default: output/verify/wa-bridge)
//   VERIFY_DOCKER_BUILD_FLAGS extra `docker build` flags for this machine

import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import {
  appendFileSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { step, until } from "./lib/client.mjs";
import { docker } from "./lib/docker.mjs";
import { shapeProblems } from "./lib/shape.mjs";

const ROOT = new URL("../../", import.meta.url);
const evidence = process.env.VERIFY_EVIDENCE_DIR ?? "output/verify/wa-bridge";
const stepLog = join(evidence, "steps.log");
const log = (line) => {
  console.log(line);
  appendFileSync(stepLog, `${line}\n`);
};

const readFixtures = (dir) => {
  const path = new URL(`fixtures/bridge/${dir}/`, ROOT);
  return readdirSync(path)
    .filter((name) => name.endsWith(".json"))
    .sort()
    .map((name) => ({
      name,
      json: JSON.parse(readFileSync(new URL(name, path), "utf8")),
    }));
};

// Keys that the fixtures of an event type all carry, but that the bridge's Go
// struct for that event tags `omitempty`, so Go leaves them out when empty.
// The spec checks each entry against the Go source.
export const OPTIONAL_KEYS = {
  profile_picture: ["url", "id", "error"],
  read_receipt_result: ["error"],
};

const kindOf = (value) =>
  value === null ? "null" : Array.isArray(value) ? "array" : typeof value;

// Checks one event against the fixtures of its type. Every key that all
// fixtures of the type carry is required, except OPTIONAL_KEYS. Each key the
// event has must match, in JSON type and nested shape, at least one fixture
// that has that key; a key no fixture has is reported, as Rust may reject it.
export function eventProblems(fixtures, event) {
  if (typeof event?.type !== "string") return ["event has no type"];
  const samples = fixtures.filter((fixture) => fixture.type === event.type);
  if (samples.length === 0) return [`no fixture for event type ${event.type}`];
  const optional = new Set(OPTIONAL_KEYS[event.type] ?? []);
  const problems = [];
  for (const key of Object.keys(samples[0])) {
    const everywhere = samples.every(
      (sample) => key in sample && sample[key] !== null,
    );
    if (everywhere && !optional.has(key) && !(key in event)) {
      problems.push(`${event.type}.${key}: missing`);
    }
  }
  for (const [key, value] of Object.entries(event)) {
    const withKey = samples.filter(
      (sample) => key in sample && sample[key] !== null,
    );
    if (withKey.length === 0) {
      if (!samples.some((sample) => key in sample)) {
        problems.push(`${event.type}.${key}: not in any fixture`);
      }
      continue;
    }
    const options = withKey.map((sample) =>
      kindOf(value) === kindOf(sample[key])
        ? shapeProblems(sample[key], value, `${event.type}.${key}`)
        : [
            `${event.type}.${key}: expected ${kindOf(sample[key])}, got ${kindOf(value)}`,
          ],
    );
    if (options.every((option) => option.length > 0)) {
      options.sort((a, b) => a.length - b.length);
      problems.push(...options[0]);
    }
  }
  return problems;
}

// Runs the bridge and returns a handle that sends commands and collects events.
function startBridge(image, path, env) {
  const args = ["run", "--rm", "--interactive"];
  for (const key of Object.keys(env)) args.push("--env", key);
  // The server passes a data directory; fake mode never writes to it.
  args.push("--entrypoint", path, image, "--data-dir", "/tmp/wa-bridge-data");
  const child = spawn("docker", args, {
    env: { ...process.env, ...env },
    stdio: ["pipe", "pipe", "pipe"],
  });
  const bridge = { child, events: [], lines: [], stderr: "", exit: null };
  let buffer = "";
  child.stdout.on("data", (chunk) => {
    buffer += chunk;
    let index;
    while ((index = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, index);
      buffer = buffer.slice(index + 1);
      if (!line.trim()) continue;
      bridge.lines.push(line);
      try {
        bridge.events.push(JSON.parse(line));
      } catch {
        bridge.events.push({ type: "unparsable", line });
      }
    }
  });
  child.stderr.on("data", (chunk) => {
    bridge.stderr += chunk;
  });
  bridge.exited = new Promise((resolve) => {
    child.on("close", (code) => {
      bridge.exit = code;
      resolve(code);
    });
  });
  child.stdin.on("error", (error) => {
    bridge.stdinError = error;
  });
  bridge.send = (command) => {
    if (bridge.stdinError || bridge.exit !== null) {
      throw new Error(`the bridge is not running (exit ${bridge.exit})`);
    }
    child.stdin.write(`${JSON.stringify(command)}\n`);
  };
  bridge.of = (type) => bridge.events.filter((event) => event.type === type);
  return bridge;
}

async function buildBridgeImage(tag) {
  const flags = (process.env.VERIFY_DOCKER_BUILD_FLAGS ?? "")
    .split(/\s+/)
    .filter(Boolean);
  await docker([
    "build",
    "--target",
    "go-builder",
    "--tag",
    tag,
    ...flags,
    ".",
  ]);
}

const save = (name, bridge) =>
  writeFileSync(
    join(evidence, `${name}.jsonl`),
    `${bridge.lines.join("\n")}\n`,
  );

async function main() {
  const eventFixtures = readFixtures("events").map((fixture) => fixture.json);
  const commands = readFixtures("commands");
  let image = process.env.VERIFY_BRIDGE_IMAGE;
  const path = process.env.VERIFY_BRIDGE_PATH || "/build/wa-bridge";
  const created = [];
  if (!image) {
    image = `wa-bridge:verify-${randomBytes(3).toString("hex")}`;
    created.push(image);
    await step("build the bridge as the go-builder stage does", log, () =>
      buildBridgeImage(image),
    );
  }
  try {
    await commandsJourney(image, path, eventFixtures, commands);
    await qrJourney(image, path, eventFixtures);
    await logoutJourney(image, path, eventFixtures);
  } finally {
    for (const tag of created) {
      await docker(["image", "rm", "--force", tag], { quiet: true }).catch(
        () => {},
      );
    }
  }
}

function checkEvents(bridge, fixtures) {
  const problems = [];
  bridge.events.forEach((event, index) => {
    if (event.type === "unparsable") {
      problems.push(`line ${index + 1} is not JSON`);
      return;
    }
    for (const problem of eventProblems(fixtures, event)) {
      problems.push(`line ${index + 1}: ${problem}`);
    }
  });
  if (problems.length > 0) {
    throw new Error([...new Set(problems)].slice(0, 20).join("; "));
  }
}

const ENDING = new Set(["disconnect", "logout"]);

async function commandsJourney(image, path, fixtures, commands) {
  const bridge = startBridge(image, path, { WA_FAKE_BRIDGE: "true" });
  try {
    await step("the fake bridge connects and syncs history", log, () =>
      until("connected and history", 15_000, () => {
        const connected = bridge.of("connected").length > 0;
        const states = bridge.of("connection_state").map((e) => e.state);
        const history = bridge.of("message").filter((e) => e.is_history);
        return connected && states.includes("connected") && history.length > 0;
      }),
    );
    const sends = [];
    // set_presence has no fake-mode answer; the bridge logs that it ignored it.
    await step(
      "every fixture command is written to the bridge",
      log,
      async () => {
        for (const { name, json } of commands) {
          if (ENDING.has(json.type)) continue;
          bridge.send(json);
          if (json.type.startsWith("send")) sends.push(json);
          log(`  sent ${name}`);
        }
      },
    );
    await step("each send reports a result", log, () =>
      until("send results", 10_000, () =>
        sends.every((command) =>
          bridge
            .of("send_result")
            .some((e) => e.request_id === command.request_id && e.success),
        ),
      ),
    );
    await step("the album send reports progress", log, () =>
      until(
        "send progress",
        5_000,
        () => bridge.of("send_progress").length > 0,
      ),
    );
    await step("mark_read and get_profile_picture are answered", log, () =>
      until("answers", 5_000, () => {
        const read = bridge.of("read_receipt_result").length > 0;
        const picture = bridge.of("profile_picture").length > 0;
        return read && picture;
      }),
    );
    await step("sent messages are delivered, then read", log, () =>
      until("receipts", 10_000, () => {
        // A reaction is not a message, so it gets no receipts.
        const reactions = sends
          .filter((command) => command.type === "send_reaction")
          .map((command) => command.request_id);
        const ids = bridge
          .of("send_result")
          .filter((e) => e.success && e.message_id)
          .filter((e) => !reactions.includes(e.request_id))
          .flatMap((e) => e.message_ids ?? [e.message_id]);
        const receipts = bridge.of("receipt");
        return ids.every((id) => {
          const statuses = receipts
            .filter((r) => r.message_ids.includes(id))
            .map((r) => r.status);
          return statuses.join(",") === "delivered,read";
        });
      }),
    );
    await step(
      "the disconnect command stops the bridge with exit 0",
      log,
      async () => {
        bridge.send(commands.find((c) => c.json.type === "disconnect").json);
        const code = await Promise.race([
          bridge.exited,
          new Promise((resolve) =>
            setTimeout(() => resolve("timeout"), 10_000),
          ),
        ]);
        if (code !== 0) throw new Error(`bridge exited with ${code}`);
      },
    );
    await step("every event matches fixtures/bridge/events", log, () =>
      checkEvents(bridge, fixtures),
    );
    await step("the bridge reports no errors", log, () => {
      const errors = bridge.of("error");
      const warnings = bridge
        .of("log")
        .filter((e) => e.level === "warn" || e.level === "error")
        .map((e) => e.message)
        .filter((message) => !/ignored command: set_presence/.test(message));
      if (errors.length || warnings.length) {
        throw new Error(
          `${errors.map((e) => e.code).join(", ")} ${warnings.join(" | ")}`,
        );
      }
    });
    const types = [...new Set(bridge.events.map((e) => e.type))].sort();
    log(`  event types: ${types.join(", ")}`);
  } finally {
    save("commands", bridge);
    bridge.child.kill();
  }
}

async function qrJourney(image, path, fixtures) {
  const bridge = startBridge(image, path, { WA_FAKE_BRIDGE: "qr" });
  try {
    await step("WA_FAKE_BRIDGE=qr shows a code, then connects", log, () =>
      until("qr then connected", 15_000, () => {
        const qr = bridge.events.findIndex((e) => e.type === "qr");
        const connected = bridge.events.findIndex(
          (e) => e.type === "connected",
        );
        return connected >= 0 && { qr, connected };
      }).then(({ qr, connected }) => {
        if (qr < 0 || qr > connected) {
          throw new Error("connected before showing a QR code");
        }
      }),
    );
    await step("closing stdin stops the bridge with exit 0", log, async () => {
      bridge.child.stdin.end();
      const code = await Promise.race([
        bridge.exited,
        new Promise((resolve) => setTimeout(() => resolve("timeout"), 10_000)),
      ]);
      if (code !== 0) throw new Error(`bridge exited with ${code}`);
    });
    await step("QR mode events match fixtures/bridge/events", log, () =>
      checkEvents(bridge, fixtures),
    );
  } finally {
    save("qr", bridge);
    bridge.child.kill();
  }
}

async function logoutJourney(image, path, fixtures) {
  const bridge = startBridge(image, path, { WA_FAKE_BRIDGE: "true" });
  try {
    await step("the fake bridge connects again", log, () =>
      until("connected", 15_000, () => bridge.of("connected").length > 0),
    );
    const logout = readFixtures("commands").find(
      (c) => c.json.type === "logout",
    );
    if (!logout) throw new Error("fixtures/bridge/commands has no logout");
    await step("logout reports logged_out and exits 0", log, async () => {
      bridge.send(logout.json);
      const code = await Promise.race([
        bridge.exited,
        new Promise((resolve) => setTimeout(() => resolve("timeout"), 10_000)),
      ]);
      if (bridge.of("logged_out").length === 0)
        throw new Error("no logged_out");
      if (code !== 0) throw new Error(`bridge exited with ${code}`);
    });
    await step("logout events match fixtures/bridge/events", log, () =>
      checkEvents(bridge, fixtures),
    );
  } finally {
    save("logout", bridge);
    bridge.child.kill();
  }
}

const isMain = process.argv[1] === new URL(import.meta.url).pathname;
if (isMain) {
  mkdirSync(evidence, { recursive: true });
  writeFileSync(stepLog, "");
  const startedAt = Date.now();
  try {
    await main();
    log(
      `wa-bridge:verify passed in ${Math.round((Date.now() - startedAt) / 1000)} s`,
    );
  } catch (error) {
    log(
      `wa-bridge:verify FAILED at "${error.step ?? "setup"}": ${error.message}`,
    );
    process.exitCode = 1;
  }
}
