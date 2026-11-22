// web-extension:verify — the MCP Apps extension through the real server, from
// issue #138, section 5.
//
// Starts the production image as server:verify does, then signs in the way an
// MCP host does: OAuth discovery, dynamic client registration and the
// authorisation-code flow with PKCE, a loopback redirect and the password.
// With that token it checks /mcp: the tool list, the UI resource against a
// fresh build of libs/web-extension/src, a read and a send. It then
// renders the served resource in libs/web-extension/verify/host.html with
// every tool call going to the real /mcp, runs the chats and feed journeys,
// and finally checks that /mcp refuses no token and a revoked one.
//
// Environment (all optional):
//   VERIFY_IMAGE        use this image instead of building the working tree
//   VERIFY_EVIDENCE_DIR where the step log, container log and Playwright
//                       results go (default: output/verify/web-extension)
//   VERIFY_DOCKER_BUILD_FLAGS extra `docker build` flags for this machine

import { execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import {
  appendFileSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { step, until } from "./lib/client.mjs";
import * as container from "./lib/docker.mjs";
import { checkLogs, redactEvidence, redactText } from "./lib/evidence.mjs";
import { FAKE } from "./lib/journeys.mjs";
import { authorize, McpSession, rpc } from "./lib/mcp.mjs";
import { freePort, runPlaywright } from "./lib/playwright.mjs";

const ROOT = new URL("../../", import.meta.url);
const RESOURCE = "ui://whatsapp-translator/chats";
// Tools the send scope adds; translate_message needs read and send.
const SEND_TOOLS = [
  "mark_conversation_read",
  "react_to_message",
  "reply_to_message",
  "send_message",
  "translate_message",
];

// Every tool the server dispatches, read from its source, so a tool that
// stops being listed (or a new one that is never listed) fails the check.
export function dispatchedTools(source) {
  return [...source.matchAll(/"([a-z_]+)" => self\.handle_/g)]
    .map((match) => match[1])
    .sort();
}

const evidence = resolve(
  process.env.VERIFY_EVIDENCE_DIR ?? "output/verify/web-extension",
);

const password = randomBytes(24).toString("base64url");
const tokens = new Set();
const secrets = () => [password, ...tokens, ...FAKE.phones];
const redact = (text) => redactText(text, [password, ...tokens]);
const log = (line) => {
  const text = redactText(line, secrets());
  console.log(text);
  appendFileSync(join(evidence, "steps.log"), `${text}\n`);
};

const created = { containers: [], volumes: [], images: [] };

async function signIn(base, scope) {
  const result = await authorize(base, { password, scope });
  if (result.token?.access_token) tokens.add(result.token.access_token);
  if (result.token?.refresh_token) tokens.add(result.token.refresh_token);
  return result;
}

async function main() {
  const head = execFileSync("git", ["rev-parse", "--short=12", "HEAD"], {
    encoding: "utf8",
  }).trim();
  const run = `verify-ext-${head}-${randomBytes(3).toString("hex")}`;
  const image = process.env.VERIFY_IMAGE || `whatsapp-translator:${run}`;
  if (!process.env.VERIFY_IMAGE) {
    created.images.push(image);
    await step("build the production image", log, () =>
      container.buildImage({ tag: image, ref: null, log }),
    );
  }
  const name = `${run}-server`;
  created.containers.push(name);
  created.volumes.push(`${name}-data`);
  const port = await freePort();
  const { url: base } = await step("start the container", log, () =>
    container.startContainer({
      name,
      image,
      volume: `${name}-data`,
      password,
      port,
    }),
  );
  await step("the container's HEALTHCHECK reports healthy", log, () =>
    container.waitHealthy(name),
  );

  await step("OAuth discovery points at this server", log, async () => {
    const server = await fetch(
      `${base}/.well-known/oauth-authorization-server`,
    ).then((response) => response.json());
    const resource = await fetch(
      `${base}/.well-known/oauth-protected-resource`,
    ).then((response) => response.json());
    if (server.issuer !== base) throw new Error(`issuer is ${server.issuer}`);
    for (const key of [
      "authorization_endpoint",
      "token_endpoint",
      "registration_endpoint",
    ]) {
      if (!server[key]?.startsWith(`${base}/`)) {
        throw new Error(`${key} is ${server[key]}`);
      }
    }
    if (!server.code_challenge_methods_supported?.includes("S256")) {
      throw new Error("PKCE S256 is not offered");
    }
    if (resource.resource !== `${base}/mcp`) {
      throw new Error(`resource is ${resource.resource}`);
    }
    if (!resource.authorization_servers?.includes(base)) {
      throw new Error("the resource names another authorization server");
    }
  });

  await step("/mcp refuses a request without a token", log, async () => {
    const reply = await rpc(base, null, "tools/list");
    const challenge = reply.response.headers.get("www-authenticate") ?? "";
    if (reply.status !== 401) throw new Error(`status ${reply.status}`);
    if (!challenge.includes("/.well-known/oauth-protected-resource")) {
      throw new Error(`WWW-Authenticate is "${challenge}"`);
    }
  });

  await step("a wrong password authorises nothing", log, async () => {
    const result = await authorize(base, {
      password: `wrong-${randomBytes(6).toString("hex")}`,
      scope: "whatsapp.read",
    });
    if (result.params.get("code")) throw new Error("a code was issued");
    if (result.params.get("error") !== "access_denied") {
      throw new Error(`error is ${result.params.get("error")}`);
    }
  });

  await step("a wrong PKCE verifier gets no token", log, async () => {
    const result = await authorize(base, {
      password,
      scope: "whatsapp.read",
      verifier: randomBytes(32).toString("base64url"),
    });
    if (result.status !== 400 || result.token.access_token) {
      throw new Error(`the token endpoint returned ${result.status}`);
    }
  });

  const full = await step(
    "the authorisation-code flow with PKCE issues a token",
    log,
    async () => {
      const result = await signIn(base, "whatsapp.read whatsapp.send");
      if (result.status !== 200 || !result.token.access_token) {
        throw new Error(`the token endpoint returned ${result.status}`);
      }
      if (result.token.scope !== "whatsapp.read whatsapp.send") {
        throw new Error(`scope is ${result.token.scope}`);
      }
      if (!result.token.refresh_token) throw new Error("no refresh token");
      return { ...result.token, clientId: result.clientId };
    },
  );
  const mcp = new McpSession(base, full.access_token);
  let readOnlyToken;

  await step(
    "tools/list returns every tool the server handles",
    log,
    async () => {
      await mcp.initialize();
      const source = readFileSync(
        new URL("apps/server/src/mcp/server_handler.rs", ROOT),
        "utf8",
      );
      const expected = dispatchedTools(source);
      if (expected.length < 10) throw new Error("could not read the tool list");
      const { tools } = await mcp.request("tools/list");
      const listed = tools.map((tool) => tool.name).sort();
      if (listed.join() !== expected.join()) {
        throw new Error(`listed ${listed} but the server handles ${expected}`);
      }
      log(`  ${listed.length} tools`);
    },
  );

  await step("a read-only token lists and runs no send tool", log, async () => {
    const result = await signIn(base, "whatsapp.read");
    if (result.status !== 200 || result.token.scope !== "whatsapp.read") {
      throw new Error(`got ${result.status} with scope ${result.token?.scope}`);
    }
    const readOnly = new McpSession(base, result.token.access_token);
    const { tools } = await readOnly.request("tools/list");
    const leaked = tools.filter((tool) => SEND_TOOLS.includes(tool.name));
    if (leaked.length > 0) {
      throw new Error(`listed ${leaked.map((tool) => tool.name)}`);
    }
    // A real preparation, so only the scope check can refuse the send.
    const prepared = await readOnly.call("prepare_message", {
      contact_id: FAKE.lucia,
      text: `verify ${randomBytes(4).toString("hex")}: read-only`,
    });
    const attempt = await rpc(base, result.token.access_token, "tools/call", {
      name: "send_message",
      arguments: {
        preparation_token: prepared.preparationToken,
        idempotency_key: `verify-ro-${randomBytes(8).toString("hex")}`,
      },
    });
    const refusal = attempt.error?.message ?? "";
    if (!refusal.includes("does not include whatsapp.send")) {
      throw new Error(
        `send_message with a read-only token was not refused for its scope: ${refusal || "it ran"}`,
      );
    }
    readOnlyToken = result.token.access_token;
  });

  await step(
    "the UI resource is byte-identical to a fresh build of src/",
    log,
    async () => {
      const { contents } = await mcp.request("resources/read", {
        uri: RESOURCE,
      });
      const committed = readFileSync(
        new URL("libs/web-extension/dist/chats.html", ROOT),
        "utf8",
      );
      if (contents.length !== 1 || contents[0].uri !== RESOURCE) {
        throw new Error("unexpected resource contents");
      }
      if (contents[0].text !== committed) {
        throw new Error(
          `served ${contents[0].text.length} characters, committed ${committed.length}`,
        );
      }
      // The committed bundle must also be what src/ builds to today, so an
      // edit to src/ without a rebuild fails here as well as in
      // web-extension:check.
      try {
        execFileSync("node", ["libs/web-extension/build.mjs", "--check"], {
          stdio: "pipe",
        });
      } catch (error) {
        throw new Error(
          `dist/chats.html is not a fresh build of src/: ${String(error.stderr).match(/Error: .*/)?.[0] ?? error.message}`,
        );
      }
    },
  );

  await step("list_contacts returns the fake chats", log, async () => {
    const { contacts } = await mcp.call("list_contacts", { limit: 50 });
    const names = contacts.map((contact) => contact.name);
    for (const wanted of FAKE.chatNames) {
      if (!names.includes(wanted)) throw new Error(`no ${wanted}`);
    }
  });

  await step("a prepared send reaches read", log, async () => {
    const text = `verify ${randomBytes(4).toString("hex")}: desde MCP`;
    const prepared = await mcp.call("prepare_message", {
      contact_id: FAKE.lucia,
      text,
    });
    const sent = await mcp.call("send_message", {
      preparation_token: prepared.preparationToken,
      idempotency_key: `verify-${randomBytes(8).toString("hex")}`,
    });
    if (sent.success !== true || !sent.messageId) {
      throw new Error("send_message did not report a message");
    }
    await until("deliveryStatus read", 20_000, async () => {
      const chat = await mcp.call("read_messages", {
        contact_id: FAKE.lucia,
        limit: 20,
      });
      const message = chat.messages.find((m) => m.id === sent.messageId);
      return message?.deliveryStatus === "read";
    });
  });

  const output = join(evidence, "playwright");
  try {
    await step(
      "the served app's chats and feed journeys pass on the real /mcp",
      log,
      () =>
        runPlaywright({
          config: "libs/web-extension/verify/verify.config.ts",
          env: {
            VERIFY_MCP_URL: base,
            VERIFY_MCP_TOKEN: full.access_token,
            VERIFY_MCP_OUTPUT: output,
          },
          log,
        }),
    );
  } finally {
    await secure(output);
  }

  // A refresh grant, as a host makes when its access token expires.
  const refresh = (token) =>
    fetch(`${base}/oauth/token`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({
        grant_type: "refresh_token",
        refresh_token: token,
        client_id: full.clientId,
        resource: `${base}/mcp`,
      }),
    });

  await step("revoking every OAuth client locks /mcp out", log, async () => {
    // First show the refresh grant works, so its refusal below means revoked.
    const renewed = await refresh(full.refresh_token);
    const fresh = await renewed.json();
    if (fresh.access_token) tokens.add(fresh.access_token);
    if (fresh.refresh_token) tokens.add(fresh.refresh_token);
    if (renewed.status !== 200 || !fresh.access_token) {
      throw new Error(`the refresh grant returned ${renewed.status}`);
    }
    const session = await fetch(`${base}/api/auth`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ password }),
    }).then((response) => response.json());
    if (!session.token) throw new Error("web sign-in failed");
    tokens.add(session.token);
    const revoked = await fetch(`${base}/api/oauth/revoke-all`, {
      method: "POST",
      headers: { authorization: `Bearer ${session.token}` },
    });
    if (!revoked.ok) throw new Error(`revoke-all returned ${revoked.status}`);
    for (const [kind, token] of [
      ["read and send", full.access_token],
      ["refreshed", fresh.access_token],
      ["read-only", readOnlyToken],
    ]) {
      const reply = await rpc(base, token, "tools/list");
      if (reply.status !== 401) {
        throw new Error(`the revoked ${kind} token got ${reply.status}`);
      }
    }
    // A surviving refresh token would mint a new access token.
    for (const token of [full.refresh_token, fresh.refresh_token]) {
      if (!token) continue;
      const again = await refresh(token);
      const body = await again.json().catch(() => ({}));
      if (body.access_token) tokens.add(body.access_token);
      if (again.ok) throw new Error("a revoked refresh token was accepted");
    }
  });

  await step("SIGTERM stops the container cleanly", log, async () => {
    const result = await container.stop(name);
    if (result.exitCode !== 0) throw new Error(`exit code ${result.exitCode}`);
  });
  await step("the container's logs are clean", log, async () => {
    const text = redact(await container.logs(name));
    writeFileSync(join(evidence, "server.log"), text);
    checkLogs(text);
  });
}

// Removes the password, every token and the fake phone numbers from the
// evidence, inside trace zips too. Anything it cannot clean is deleted and
// fails the run.
async function secure(dir) {
  try {
    const leaks = await redactEvidence(dir, secrets());
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

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  rmSync(evidence, { recursive: true, force: true });
  mkdirSync(evidence, { recursive: true });
  for (const signal of ["SIGINT", "SIGTERM"]) {
    process.once(signal, async () => {
      log(`web-extension:verify interrupted by ${signal}`);
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
    log(
      `web-extension:verify FAILED at "${error.step ?? "setup"}": ${error.message}`,
    );
    for (const name of created.containers) {
      await container
        .logs(name)
        .then((text) =>
          writeFileSync(join(evidence, "server.log"), redact(text)),
        )
        .catch(() => {});
    }
    process.exitCode = 1;
  } finally {
    await cleanUp();
  }
  // Only now, since securing the evidence can still fail the run.
  if (!process.exitCode) {
    log(
      `web-extension:verify passed in ${Math.round((Date.now() - startedAt) / 1000)} s`,
    );
  }
}
