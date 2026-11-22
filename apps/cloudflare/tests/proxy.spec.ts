import assert from "node:assert/strict";
import { test } from "node:test";
import { forwardedRequest } from "../src/proxy.ts";

test("sends the path and query to the container and records the public host", () => {
  const request = new Request(
    "https://bridge.example.com/api/messages?limit=5",
    {
      headers: {
        forwarded: "host=evil.example",
        "x-forwarded-host": "evil.example",
        "x-forwarded-proto": "http",
        "cf-connecting-ip": "203.0.113.7",
        authorization: "Bearer token",
        upgrade: "websocket",
      },
    },
  );
  const forwarded = forwardedRequest(request);
  assert.equal(forwarded.url, "http://container/api/messages?limit=5");
  assert.equal(forwarded.headers.get("forwarded"), null);
  assert.equal(forwarded.headers.get("x-forwarded-host"), "bridge.example.com");
  assert.equal(forwarded.headers.get("x-forwarded-proto"), "https");
  assert.equal(forwarded.headers.get("x-forwarded-for"), "203.0.113.7");
  assert.equal(forwarded.headers.get("authorization"), "Bearer token");
  assert.equal(forwarded.headers.get("upgrade"), "websocket");
});

test("keeps the method and body", async () => {
  const forwarded = forwardedRequest(
    new Request("https://bridge.example.com/api/send", {
      method: "POST",
      body: "hello",
    }),
  );
  assert.equal(forwarded.method, "POST");
  assert.equal(await forwarded.text(), "hello");
});
