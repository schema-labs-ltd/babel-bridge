import assert from "node:assert/strict";
import { test } from "node:test";
import { redactor } from "../verify/lib/client.mjs";
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
