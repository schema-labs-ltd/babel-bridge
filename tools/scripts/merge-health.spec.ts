import assert from "node:assert/strict";
import { test } from "node:test";
import {
  codexReviewedCommit,
  formatReport,
  isRevert,
  median,
  reviewRounds,
  riskOf,
  summarise,
} from "./merge-health.mjs";

const copilot = (commit_id: string) => ({
  user: { login: "copilot-pull-request-reviewer[bot]" },
  commit_id,
});
const codex = (commit_id: string) => ({
  user: { login: "chatgpt-codex-connector[bot]" },
  commit_id,
});
const codexSummary = (sha: string | null, status = "completed") => ({
  body: `<!-- codex-pull-request-review-summary -->\n<!-- codex-review-state:v1 {"code_review":{"commit_sha":${JSON.stringify(sha)},"status":"${status}"}} -->\n\n## Codex Review Summary`,
});

const pr = (
  number: number,
  overrides: Partial<{
    title: string;
    labels: { name: string }[];
    hours: number;
    rounds: number;
  }> = {},
) => {
  const created = Date.parse("2026-10-20T09:00:00Z");
  return {
    number,
    title: overrides.title ?? `feat: change ${number}`,
    labels: overrides.labels ?? [],
    created_at: new Date(created).toISOString(),
    merged_at: new Date(
      created + (overrides.hours ?? 1) * 3_600_000,
    ).toISOString(),
    rounds: overrides.rounds ?? 1,
  };
};

test("reads the risk label and prefers the highest when several are set", () => {
  assert.equal(riskOf([{ name: "risk:low" }]), "risk:low");
  assert.equal(
    riskOf([{ name: "risk:low" }, { name: "risk:high" }]),
    "risk:high",
  );
  assert.equal(riskOf([{ name: "bug" }]), null);
});

test("spots reverts by their title", () => {
  assert.equal(isRevert('Revert "feat(web): add topics"'), true);
  assert.equal(isRevert("revert: undo widget change"), true);
  assert.equal(isRevert("fix: handle reverted edits"), false);
});

test("takes the median of odd and even lists", () => {
  assert.equal(median([]), null);
  assert.equal(median([5, 1, 3]), 3);
  assert.equal(median([4, 1, 3, 2]), 2.5);
});

test("counts one round per commit either bot reviewed", () => {
  const reviews = [
    copilot("a"),
    codex("a"),
    copilot("b"),
    { user: { login: "vultuk" }, commit_id: "c" },
  ];
  assert.equal(reviewRounds(reviews, []), 2);
});

test("counts a clean Codex run from its summary comment", () => {
  assert.equal(
    codexReviewedCommit([{ body: "hello" }, codexSummary("c")]),
    "c",
  );
  assert.equal(
    reviewRounds([copilot("a"), copilot("c")], [codexSummary("c")]),
    2,
  );
  assert.equal(reviewRounds([copilot("a")], [codexSummary("b")]), 2);
  assert.equal(reviewRounds([], [codexSummary(null)]), 0);
  assert.equal(reviewRounds([], [codexSummary("d", "running")]), 0);
});

test("summarises the week and lists PRs over three rounds, worst first", () => {
  const summary = summarise([
    pr(1, { labels: [{ name: "risk:low" }], hours: 2, rounds: 1 }),
    pr(2, { labels: [{ name: "risk:high" }], hours: 10, rounds: 5 }),
    pr(3, { title: 'Revert "feat: change 1"', hours: 1, rounds: 0 }),
    pr(4, { labels: [{ name: "risk:medium" }], hours: 4, rounds: 4 }),
  ]);

  assert.equal(summary.merged, 4);
  assert.deepEqual(summary.byRisk, {
    "risk:low": 1,
    "risk:medium": 1,
    "risk:high": 1,
    unlabelled: 1,
  });
  assert.deepEqual(
    summary.reverts.map((p) => p.number),
    [3],
  );
  assert.equal(summary.medianHours, 3);
  assert.equal(summary.medianRounds, 2.5);
  assert.equal(summary.totalRounds, 10);
  assert.deepEqual(
    summary.overLimit.map((p) => p.number),
    [2, 4],
  );
});

test("formats an empty week without failing", () => {
  const report = formatReport(summarise([]), {
    since: new Date("2026-10-19T08:00:00Z"),
    until: new Date("2026-10-26T08:00:00Z"),
  });

  assert.ok(report.includes("## Merge health, 2026-10-19 to 2026-10-26"));
  assert.ok(report.includes("| PRs merged | 0 |"));
  assert.ok(report.includes("| Median hours open to merge | n/a |"));
  assert.ok(!report.includes("### Reverts"));
});

test("lists reverts and long-running reviews in the report", () => {
  const report = formatReport(
    summarise([
      pr(7, { title: 'Revert "feat: x"' }),
      pr(8, { title: "fix: y", rounds: 6 }),
    ]),
    { since: new Date(), until: new Date() },
  );

  assert.ok(report.includes('### Reverts\n\n- #7 Revert "feat: x"'));
  assert.ok(
    report.includes("### Over 3 review rounds\n\n- #8 fix: y (6 rounds)"),
  );
});
