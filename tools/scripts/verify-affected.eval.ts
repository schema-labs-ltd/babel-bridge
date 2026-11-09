import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { affectedTasks, APPLE_PROJECTS } from "./verify-affected.mjs";

// Runs the real Nx project graph to check that a change selects the verify jobs
// issue #138 expects. Each case lists jobs that must run and jobs that must
// not. A job that no project defines yet is left out of the comparison, so a
// case starts to count when its verify target is added. verify.yml's `changes`
// job runs this, and so does `make verify-mapping`.

const ALL = ["website", "server", "wa-bridge", "web", "web-extension"]
  .concat(["ios", "ios-live", "tools-scripts"])
  .sort();

const CASES = [
  {
    files: ["docs/index.html"],
    runs: ["website"],
  },
  {
    files: ["Dockerfile"],
    // wa-bridge too: its check builds the Dockerfile's go-builder stage.
    runs: ["server", "web", "web-extension", "wa-bridge"],
    skips: ["website", "ios", "tools-scripts"],
  },
  {
    files: ["railway.toml"],
    runs: ["server"],
    skips: ["website", "ios"],
  },
  {
    files: ["apps/wa-bridge/send.go"],
    runs: ["wa-bridge", "server", "web", "web-extension", "ios-live"],
    skips: ["website", "ios"],
  },
  {
    files: ["fixtures/api/responses/status.json"],
    runs: ["server", "web", "ios-live"],
    skips: ["website", "ios", "wa-bridge"],
  },
  {
    files: ["fixtures/bridge/events/qr.json"],
    runs: ["server", "wa-bridge"],
    skips: ["website", "ios"],
  },
  // The cases apple.yml checked before Nx chose its jobs.
  { files: ["apps/ios/project.yml"], runs: ["ios", "ios-live"] },
  {
    files: ["apps/ios/WhatsAppTranslator/App/AppSession.swift"],
    runs: ["ios", "ios-live"],
    skips: ["website", "server", "web"],
  },
  { files: ["apps/web/public/sounds/tone.wav"], runs: ["ios"] },
  { files: [".github/workflows/apple.yml"], runs: ["ios"] },
  { files: ["apps/server/src/main.rs"], runs: ["server"], skips: ["ios"] },
  { files: ["AGENTS.md"], runs: [], skips: ALL },
  { files: [".github/workflows/ci.yml"], skips: ["ios"] },
  {
    files: [".github/workflows/verify.yml"],
    runs: ALL.filter((task) => !task.startsWith("ios")),
    skips: ["ios", "ios-live"],
  },
];

const existing = affectedTasks({ all: true });

// The job ids under a workflow's top-level `jobs:` key.
const jobsOf = (name) => {
  const text = readFileSync(
    new URL(`../../.github/workflows/${name}`, import.meta.url),
    "utf8",
  );
  const jobs = text.split(/^jobs:\s*$/m)[1] ?? "";
  const section = jobs.split(/^\S/m)[0];
  return new Set(
    [...section.matchAll(/^ {2}([a-z0-9-]+):\s*$/gm)].map((match) => match[1]),
  );
};

test("every verify task has a job in its workflow", () => {
  const verifyJobs = jobsOf("verify.yml");
  const appleJobs = jobsOf("apple.yml");
  for (const task of existing) {
    const apple = APPLE_PROJECTS.some(
      (project) => task === project || task.startsWith(`${project}-`),
    );
    const [jobs, workflow] = apple
      ? [appleJobs, "apple.yml"]
      : [verifyJobs, "verify.yml"];
    assert.ok(jobs.has(task), `${workflow} has no job named ${task}`);
  }
  for (const task of existing) {
    assert.ok(ALL.includes(task), `add ${task} to ALL in this eval`);
  }
});

for (const { files, runs = [], skips = [] } of CASES) {
  test(`a change to ${files.join(", ")} selects the expected jobs`, () => {
    const selected = affectedTasks({ files });
    for (const task of runs.filter((task) => existing.includes(task))) {
      assert.ok(selected.includes(task), `${task} should run: ${selected}`);
    }
    for (const task of skips) {
      assert.ok(!selected.includes(task), `${task} should not run`);
    }
  });
}
