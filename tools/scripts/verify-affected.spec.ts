import assert from "node:assert/strict";
import { test } from "node:test";
import {
  affectedTasks,
  checkResults,
  scopeForEvent,
  taskKey,
} from "./verify-affected.mjs";

const SHA_A = "a".repeat(40);
const SHA_B = "b".repeat(40);

test("names a job after its project and verify target", () => {
  assert.equal(taskKey("server", "verify"), "server");
  assert.equal(taskKey("ios", "verify-live"), "ios-live");
  assert.throws(() => taskKey("ios", "test"), /not a verify target/);
  assert.throws(() => taskKey("@babel-bridge/source", "verify"), /project/);
  assert.throws(() => taskKey("a;rm", "verify"), /project/);
});

test("runs everything outside pull requests", () => {
  for (const eventName of ["merge_group", "push", "workflow_dispatch"]) {
    assert.deepEqual(
      scopeForEvent({ eventName, baseSha: SHA_A, headSha: SHA_B }),
      { all: true },
    );
  }
});

test("runs the affected set on a pull request with a base", () => {
  assert.deepEqual(
    scopeForEvent({
      eventName: "pull_request",
      baseSha: SHA_A,
      headSha: SHA_B,
    }),
    { base: SHA_A, head: SHA_B },
  );
  for (const baseSha of ["", undefined, "0".repeat(40), "main"]) {
    assert.deepEqual(
      scopeForEvent({ eventName: "pull_request", baseSha, headSha: SHA_B }),
      { all: true },
    );
  }
});

test("asks Nx for each verify target and merges the results", () => {
  const calls = [];
  const fake = (target, args) => {
    calls.push([target, ...args]);
    return target === "verify" ? ["web", "server"] : ["ios"];
  };
  assert.deepEqual(affectedTasks({ all: true }, fake), [
    "ios-live",
    "server",
    "web",
  ]);
  assert.deepEqual(calls, [["verify"], ["verify-live"]]);

  calls.length = 0;
  affectedTasks({ base: SHA_A, head: SHA_B }, fake);
  assert.deepEqual(calls[0], [
    "verify",
    "--affected",
    `--base=${SHA_A}`,
    `--head=${SHA_B}`,
  ]);

  calls.length = 0;
  affectedTasks({ files: ["docs/index.html", "Dockerfile"] }, fake);
  assert.deepEqual(calls[0], [
    "verify",
    "--affected",
    "--files=docs/index.html,Dockerfile",
  ]);
  assert.deepEqual(affectedTasks({ files: [] }, fake), []);
});

test("splits tasks between verify.yml and apple.yml", () => {
  const fake = (target) =>
    target === "verify" ? ["ios", "server"] : ["ios", "web"];
  assert.deepEqual(affectedTasks({ all: true }, fake, "verify"), [
    "server",
    "web-live",
  ]);
  assert.deepEqual(affectedTasks({ all: true }, fake, "apple"), [
    "ios",
    "ios-live",
  ]);
  assert.equal(affectedTasks({ all: true }, fake).length, 4);
});

test("refuses revisions that are not commit shas", () => {
  assert.throws(
    () => affectedTasks({ base: "main; rm -rf /", head: SHA_B }, () => []),
    /commit shas/,
  );
});

const ok = { result: "success" };
const skipped = { result: "skipped" };

test("passes when affected jobs succeed and the rest are skipped", () => {
  assert.deepEqual(
    checkResults(["server"], { changes: ok, server: ok, website: skipped }),
    [],
  );
  assert.deepEqual(checkResults([], { changes: ok, website: skipped }), []);
});

test("fails when the changes job did not succeed", () => {
  assert.deepEqual(checkResults([], { changes: { result: "failure" } }), [
    "changes: expected success, got failure",
  ]);
  assert.equal(checkResults([], {}).length, 1);
});

test("fails when an affected job failed, was skipped or is missing", () => {
  assert.deepEqual(
    checkResults(["server", "web", "website"], {
      changes: ok,
      server: { result: "failure" },
      web: skipped,
    }),
    [
      "server: expected success, got failure",
      "web: expected success, got skipped",
      "website: affected, but verify.yml has no job for it",
    ],
  );
});

test("lets helper jobs run or skip, but not fail", () => {
  assert.deepEqual(checkResults([], { changes: ok, image: ok }), []);
  assert.deepEqual(checkResults([], { changes: ok, image: skipped }), []);
  assert.deepEqual(
    checkResults(["server"], {
      changes: ok,
      image: { result: "cancelled" },
      server: ok,
    }),
    ["image: expected success or skipped, got cancelled"],
  );
});

test("refuses two tasks that would share one job", () => {
  const fake = (target) => (target === "verify" ? ["web-live"] : ["web"]);
  assert.throws(() => affectedTasks({ all: true }, fake), /share job web-live/);
});
