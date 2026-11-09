import assert from "node:assert/strict";
import { test } from "node:test";
import { execFileSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { eventProblems, OPTIONAL_KEYS } from "../verify/bridge.mjs";
import { redactor } from "../verify/lib/client.mjs";
import {
  checkLogs,
  redactEvidence,
  redactText,
  tokensIn,
} from "../verify/lib/evidence.mjs";
import { referencedAssets } from "../verify/lib/journeys.mjs";
import { shapeProblems } from "../verify/lib/shape.mjs";

// Unit tests for the helpers tools/verify uses to judge a live server.

test("accepts live JSON with the fixture's keys and types", () => {
  const fixture = { ok: true, service: "x", nested: { n: 1 }, maybe: null };
  assert.deepEqual(
    shapeProblems(fixture, {
      ok: false,
      service: "y",
      nested: { n: 2 },
      extra: 1,
    }),
    [],
  );
  assert.deepEqual(shapeProblems({ a: "x" }, { a: null }), []);
});

test("reports a renamed or retyped field", () => {
  assert.deepEqual(shapeProblems({ connected: true }, { isConnected: true }), [
    "$.connected: missing",
  ]);
  assert.deepEqual(shapeProblems({ n: 1 }, { n: "1" }), [
    "$.n: expected number, got string",
  ]);
  assert.deepEqual(shapeProblems({ messages: [] }, { messages: null }), [
    "$.messages: expected array, got null",
  ]);
  assert.deepEqual(shapeProblems([{ a: 1 }], {}), [
    "$: expected array, got object",
  ]);
});

test("lets a message's content leave out fields", () => {
  const fixture = { content: { type: "text", receipt_sender_jid: "a" } };
  assert.deepEqual(shapeProblems(fixture, { content: { type: "text" } }), []);
  assert.deepEqual(shapeProblems(fixture, { content: { type: 1 } }), [
    "$.content.type: expected string, got number",
  ]);
});

test("matches each array item against any fixture item", () => {
  const fixture = [
    { type: "a", a: 1 },
    { type: "b", b: "x" },
  ];
  assert.deepEqual(shapeProblems(fixture, [{ type: "b", b: "y" }]), []);
  assert.deepEqual(shapeProblems(fixture, [{ type: "c" }]), [
    "$[0].a: missing",
  ]);
});

test("lists the local assets index.html references", () => {
  const html = `<link rel="manifest" href="manifest.json">
    <link href="https://fonts.example/x.css" rel="stylesheet">
    <script src="./app.js?v=2"></script><a href="#top"></a>
    <img src="data:image/png;base64,AA"><script src="/app/feed.js"></script>`;
  assert.deepEqual(referencedAssets(html), [
    "/app.js",
    "/app/feed.js",
    "/manifest.json",
  ]);
});

test("redacts every secret from logged text", () => {
  const secrets = ["s3cret-password", "token-123456"];
  const redact = redactor(() => [...secrets, "", "short"]);
  assert.equal(
    redact("pw=s3cret-password t=token-123456 short"),
    "pw=[verify-secret] t=[verify-secret] short",
  );
});

test("checks a bridge event against every fixture of its type", () => {
  const fixtures = [
    { type: "send_result", request_id: 1, success: true, message_id: "a" },
    { type: "send_result", request_id: 2, success: false, error: "x" },
  ];
  const ok = {
    type: "send_result",
    request_id: 3,
    success: true,
    message_id: "b",
  };
  assert.deepEqual(eventProblems(fixtures, ok), []);
  assert.deepEqual(
    eventProblems(fixtures, { type: "send_result", success: true }),
    ["send_result.request_id: missing"],
  );
  // A wrong type fails even though another fixture lacks the key.
  assert.deepEqual(eventProblems(fixtures, { ...ok, message_id: 42 }), [
    "send_result.message_id: expected string, got number",
  ]);
  assert.deepEqual(eventProblems(fixtures, { ...ok, surprise: 1 }), [
    "send_result.surprise: not in any fixture",
  ]);
  assert.deepEqual(eventProblems(fixtures, { type: "nope" }), [
    "no fixture for event type nope",
  ]);
});

test("each optional bridge key is omitempty in the Go source", () => {
  const source = readdirSync("apps/wa-bridge")
    .filter((name) => name.endsWith(".go") && !name.endsWith("_test.go"))
    .map((name) => readFileSync(`apps/wa-bridge/${name}`, "utf8"))
    .join("\n");
  for (const [type, keys] of Object.entries(OPTIONAL_KEYS)) {
    for (const key of keys) {
      assert.match(
        source,
        new RegExp(`json:"${key},omitempty"`),
        `${type}.${key}`,
      );
    }
  }
});

test("finds and redacts session tokens in recorded requests", () => {
  const token = "AbCdEfGhIjKlMnOpQrStUv_-12";
  const text = `Authorization: Bearer ${token} /ws?token=${token} {"token":"${token}"}`;
  assert.deepEqual(tokensIn(text), [token, token, token]);
  assert.equal(
    redactText(text, []),
    'Authorization: Bearer [verify-secret] /ws?token=[verify-secret] {"token":"[verify-secret]"}',
  );
  assert.equal(redactText("pw hunter22", ["hunter22"]), "pw [verify-secret]");
});

test("container log check accepts only the known lines it is given", () => {
  const known = "ERROR Failed to write command to bridge: Broken pipe";
  assert.throws(() => checkLogs(`INFO ok\n${known}`), /an ERROR line/);
  checkLogs(`INFO ok\n${known}`, { allow: [/Broken pipe/] });
  assert.throws(
    () => checkLogs(`${known}\nERROR other`, { allow: [/Broken pipe/] }),
    /an ERROR line/,
  );
});

test("evidence redaction reaches inside trace zips", async () => {
  const dir = mkdtempSync(join(tmpdir(), "verify-spec-"));
  const token = "ZyXwVuTsRqPoNmLkJiHg_9876";
  const inside = join(dir, "unpacked");
  mkdirSync(inside);
  writeFileSync(join(inside, "0-trace.network"), `{"token":"${token}"}`);
  writeFileSync(join(inside, "test.trace"), `fill hunter22 then ${token}`);
  execFileSync("zip", ["-q", "-r", join(dir, "trace.zip"), "."], {
    cwd: inside,
  });
  execFileSync("rm", ["-r", inside]);
  writeFileSync(join(dir, "steps.log"), "typed hunter22");
  assert.deepEqual(await redactEvidence(dir, ["hunter22"]), []);
  const zipped = execFileSync("unzip", ["-p", join(dir, "trace.zip")], {
    encoding: "utf8",
  });
  assert.ok(!zipped.includes(token) && !zipped.includes("hunter22"));
  assert.match(zipped, /\[verify-secret\]/);
  assert.equal(
    readFileSync(join(dir, "steps.log"), "utf8"),
    "typed [verify-secret]",
  );
});
