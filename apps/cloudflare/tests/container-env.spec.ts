import assert from "node:assert/strict";
import { test } from "node:test";
import { configDigest, containerEnv } from "../src/container-env.ts";

test("passes listed variables through and keeps the fixed ones fixed", () => {
  const env = containerEnv({
    WA_PASSWORD: "secret",
    OPENAI_API_KEY: "sk-test",
    WA_DEFAULT_LANGUAGE: "",
    WA_DATA_DIR: "/somewhere-else",
    WA_HOST: "127.0.0.1",
    UNLISTED_SECRET: "stays in the Worker",
    BACKUPS: {},
  });
  assert.equal(env.WA_PASSWORD, "secret");
  assert.equal(env.OPENAI_API_KEY, "sk-test");
  assert.equal("WA_DEFAULT_LANGUAGE" in env, false);
  assert.equal("UNLISTED_SECRET" in env, false);
  assert.equal(env.WA_DATA_DIR, "/data");
  assert.equal(env.WA_HOST, "0.0.0.0");
  assert.equal(env.WA_PORT, "3000");
  assert.equal(env.WA_BACKUP_URL, "http://192.0.2.1");
});

test("changes the digest when any value changes, whatever the key order", async () => {
  const base = await configDigest({ A: "1", B: "2" });
  assert.match(base, /^[0-9a-f]{32}$/);
  assert.equal(await configDigest({ B: "2", A: "1" }), base);
  assert.notEqual(await configDigest({ A: "1", B: "3" }), base);
  assert.notEqual(await configDigest({ A: "1" }), base);
});
