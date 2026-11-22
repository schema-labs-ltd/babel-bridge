import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Works out which production-like verify tasks a change affects, for the
// `changes` and `verify` jobs in .github/workflows/verify.yml. Nx decides what
// a change touches from each project's files, dependencies and target inputs,
// so there is no hand-written path list here. AGENTS.md ("Checks") describes
// the verify targets.

// The Nx targets verify.yml runs. Each affected (project, target) pair is one
// job in verify.yml, named by taskKey().
export const VERIFY_TARGETS = ["verify", "verify-live"];

// The verify.yml job id for a project's verify target: the project name for
// `verify`, and the project name plus the suffix for `verify-<suffix>`.
export function taskKey(project, target) {
  if (!/^[a-z0-9][a-z0-9-]*$/.test(project)) {
    throw new Error(`unexpected project name: ${JSON.stringify(project)}`);
  }
  if (target === "verify") return project;
  if (target.startsWith("verify-")) {
    return `${project}-${target.slice("verify-".length)}`;
  }
  throw new Error(`not a verify target: ${target}`);
}

const SHA = /^[0-9a-f]{40}$/i;

// With -t, Nx lists the projects whose own task is affected, judged by that
// target's inputs (which is why every verify target lists the files it needs).
// This relies on nx.json not setting `affected.legacy` and NX_LEGACY_AFFECTED
// staying unset.
function nxProjects(target, args) {
  const output = execFileSync(
    "npx",
    ["--no-install", "nx", "show", "projects", "-t", target, ...args, "--json"],
    {
      encoding: "utf8",
      env: { ...process.env, NX_DAEMON: "false", NX_NO_CLOUD: "true" },
      stdio: ["ignore", "pipe", "inherit"],
    },
  );
  const projects = JSON.parse(output);
  if (!Array.isArray(projects)) throw new Error("nx did not return a list");
  return projects;
}

// The projects whose verify tasks run in apple.yml on macOS, not in verify.yml.
export const APPLE_PROJECTS = ["ios"];

// Returns the sorted job keys to run. `scope` is one of
//   { all: true }                      every verify task
//   { base, head }                     tasks affected between two commits
//   { files: ["docs/index.html"] }     tasks affected by these files
// `workflow` keeps the tasks of one workflow: "verify", "apple" or "any".
export function affectedTasks(
  scope,
  projectsFor = nxProjects,
  workflow = "any",
) {
  const keep = (project) =>
    workflow === "any" ||
    APPLE_PROJECTS.includes(project) === (workflow === "apple");
  let args;
  if (scope.all) {
    args = [];
  } else if (scope.files) {
    if (scope.files.length === 0) return [];
    args = ["--affected", `--files=${scope.files.join(",")}`];
  } else {
    if (!SHA.test(scope.base ?? "") || !SHA.test(scope.head ?? "")) {
      throw new Error("base and head must be commit shas");
    }
    args = ["--affected", `--base=${scope.base}`, `--head=${scope.head}`];
  }
  const tasks = new Map();
  for (const target of VERIFY_TARGETS) {
    for (const project of projectsFor(target, args).filter(keep)) {
      const key = taskKey(project, target);
      const task = `${project}:${target}`;
      if (tasks.has(key) && tasks.get(key) !== task) {
        throw new Error(`${task} and ${tasks.get(key)} share job ${key}`);
      }
      tasks.set(key, task);
    }
  }
  return [...tasks.keys()].sort();
}

// Picks the scope from the GitHub event. Pull requests run what they affect.
// The merge queue, pushes to main and manual runs run everything, as does a
// pull request with no usable base.
export function scopeForEvent({ eventName, baseSha, headSha }) {
  if (eventName !== "pull_request") return { all: true };
  if (!SHA.test(baseSha ?? "") || /^0+$/.test(baseSha)) return { all: true };
  return { base: baseSha, head: headSha };
}

// Checks the results of verify.yml's jobs, as `toJSON(needs)` gives them. Every
// task in `tasks` must have a job that succeeded, and every other job must have
// been skipped. Returns the problems found; an empty list means the check passes.
export function checkResults(tasks, needs) {
  const problems = [];
  const changes = needs.changes?.result;
  if (changes !== "success") {
    problems.push(`changes: expected success, got ${changes ?? "nothing"}`);
    return problems;
  }
  for (const task of tasks) {
    const result = needs[task]?.result;
    if (result === undefined) {
      problems.push(`${task}: affected, but verify.yml has no job for it`);
    } else if (result !== "success") {
      problems.push(`${task}: expected success, got ${result}`);
    }
  }
  // Other jobs are helpers that verify jobs share, such as one image build.
  // They may run or be skipped, but must not fail.
  for (const [job, { result }] of Object.entries(needs)) {
    if (job === "changes" || tasks.includes(job)) continue;
    if (result !== "success" && result !== "skipped") {
      problems.push(`${job}: expected success or skipped, got ${result}`);
    }
  }
  return problems;
}

function writeOutput(name, value) {
  const line = `${name}=${value}\n`;
  if (process.env.GITHUB_OUTPUT)
    appendFileSync(process.env.GITHUB_OUTPUT, line);
  process.stdout.write(line);
}

function main([command = "tasks", ...rest]) {
  if (command === "tasks") {
    const files = rest.find((arg) => arg.startsWith("--files="));
    const workflow =
      rest.find((arg) => arg.startsWith("--workflow="))?.slice(11) ?? "any";
    if (!["verify", "apple", "any"].includes(workflow)) {
      throw new Error(`unknown workflow: ${workflow}`);
    }
    const scope = rest.includes("--all")
      ? { all: true }
      : files
        ? { files: files.slice("--files=".length).split(",").filter(Boolean) }
        : scopeForEvent({
            eventName: process.env.EVENT_NAME,
            baseSha: process.env.BASE_SHA,
            headSha: process.env.HEAD_SHA,
          });
    console.log(
      scope.all ? "Running every verify task." : "Running affected tasks.",
    );
    writeOutput(
      "tasks",
      JSON.stringify(affectedTasks(scope, nxProjects, workflow)),
    );
    return;
  }
  if (command === "report") {
    const tasks = JSON.parse(process.env.TASKS ?? "null");
    const needs = JSON.parse(process.env.NEEDS ?? "null");
    if (!Array.isArray(tasks) || needs === null || typeof needs !== "object") {
      throw new Error("TASKS and NEEDS must be set");
    }
    for (const [job, { result }] of Object.entries(needs)) {
      console.log(`${job}: ${result}`);
    }
    const problems = checkResults(tasks, needs);
    if (problems.length > 0) {
      for (const problem of problems) console.error(`error: ${problem}`);
      process.exitCode = 1;
      return;
    }
    console.log(
      tasks.length === 0
        ? "No verify task is affected. Passed without running one."
        : `Every affected verify task passed: ${tasks.join(", ")}.`,
    );
    return;
  }
  throw new Error(`unknown command: ${command}`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(`verify-affected failed: ${error.message}`);
    process.exitCode = 1;
  }
}
