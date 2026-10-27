import { appendFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Builds the weekly merge health report for merge-health.yml: PRs merged in
// the last week by risk label, reverts, time from open to merge and how many
// rounds of bot review each PR needed. Posts it as a comment on one tracking
// issue, or prints it with --dry-run.

export const RISK_LABELS = ["risk:low", "risk:medium", "risk:high"];
export const REVIEW_BOTS = [
  "copilot-pull-request-reviewer[bot]",
  "Copilot",
  "chatgpt-codex-connector[bot]",
];
export const ROUND_LIMIT = 3;
export const ISSUE_TITLE = "Weekly merge health";
const DAY_MS = 24 * 60 * 60 * 1000;

export function riskOf(labels) {
  const names = labels.map((label) => label.name ?? label);
  // A PR carrying several risk labels counts at its highest.
  return (
    [...RISK_LABELS].reverse().find((risk) => names.includes(risk)) ?? null
  );
}

export function isRevert(title) {
  return /^revert\b/i.test(title.trim());
}

export function median(values) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

export function hoursOpen(pr) {
  return (Date.parse(pr.merged_at) - Date.parse(pr.created_at)) / 3_600_000;
}

// The commit a clean Codex run reviewed lives only in its summary comment,
// because Codex posts a review only when it has findings. That comment is
// edited in place, so only the latest clean run is visible.
export function codexReviewedCommit(comments) {
  for (const comment of comments) {
    const match = comment.body?.match(/codex-review-state:v1 (\{.*?\}) -->/);
    if (!match) continue;
    try {
      const review = JSON.parse(match[1]).code_review;
      return review?.status === "completed"
        ? (review.commit_sha ?? null)
        : null;
    } catch {
      return null;
    }
  }
  return null;
}

// A round is one commit that Copilot or Codex reviewed, so a bot looking at
// the same commit twice, or both bots reviewing it, is still one round.
// Earlier clean Codex runs leave no trace, so this is a lower bound; Copilot
// reviews every push here, which keeps the gap small.
export function reviewRounds(reviews, comments) {
  const commits = new Set(
    reviews
      .filter((review) => REVIEW_BOTS.includes(review.user?.login))
      .map((review) => review.commit_id),
  );
  const codexCommit = codexReviewedCommit(comments);
  if (codexCommit) commits.add(codexCommit);
  return commits.size;
}

export function summarise(prs) {
  const byRisk = Object.fromEntries(
    [...RISK_LABELS, "unlabelled"].map((risk) => [risk, 0]),
  );
  for (const pr of prs) byRisk[riskOf(pr.labels) ?? "unlabelled"] += 1;
  const rounds = prs.map((pr) => pr.rounds);
  return {
    merged: prs.length,
    byRisk,
    reverts: prs.filter((pr) => isRevert(pr.title)),
    medianHours: median(prs.map(hoursOpen)),
    medianRounds: median(rounds),
    totalRounds: rounds.reduce((sum, value) => sum + value, 0),
    overLimit: prs
      .filter((pr) => pr.rounds > ROUND_LIMIT)
      .sort((a, b) => b.rounds - a.rounds || a.number - b.number),
  };
}

const fixed = (value) => (value === null ? "n/a" : value.toFixed(1));
const prLine = (pr, extra = "") =>
  `- #${pr.number} ${pr.title.replace(/\s+/g, " ").trim()}${extra}`;

export function formatReport(summary, { since, until }) {
  const day = (date) => date.toISOString().slice(0, 10);
  const lines = [
    `## Merge health, ${day(since)} to ${day(until)}`,
    "",
    "| Measure | Value |",
    "| --- | --- |",
    `| PRs merged | ${summary.merged} |`,
    ...RISK_LABELS.map((risk) => `| \`${risk}\` | ${summary.byRisk[risk]} |`),
    `| No risk label | ${summary.byRisk.unlabelled} |`,
    `| Reverts | ${summary.reverts.length} |`,
    `| Median hours open to merge | ${fixed(summary.medianHours)} |`,
    `| Median review rounds per PR | ${fixed(summary.medianRounds)} |`,
    `| Review rounds in total | ${summary.totalRounds} |`,
    `| PRs over ${ROUND_LIMIT} rounds | ${summary.overLimit.length} |`,
  ];
  if (summary.reverts.length) {
    lines.push(
      "",
      "### Reverts",
      "",
      ...summary.reverts.map((pr) => prLine(pr)),
    );
  }
  if (summary.overLimit.length) {
    lines.push(
      "",
      `### Over ${ROUND_LIMIT} review rounds`,
      "",
      ...summary.overLimit.map((pr) => prLine(pr, ` (${pr.rounds} rounds)`)),
    );
  }
  lines.push(
    "",
    "<sub>A review round is a commit that Copilot or Codex reviewed. Codex keeps only its latest clean run, so round counts are a lower bound. Hours run from when the PR was opened, drafts included.</sub>",
  );
  return lines.join("\n");
}

function client({ token, repository, apiUrl }) {
  async function request(path, init = {}) {
    const url = path.startsWith("http") ? path : `${apiUrl}${path}`;
    const response = await fetch(url, {
      ...init,
      headers: {
        accept: "application/vnd.github+json",
        authorization: `Bearer ${token}`,
        "x-github-api-version": "2022-11-28",
        ...init.headers,
      },
    });
    if (!response.ok) {
      throw new Error(
        `${init.method ?? "GET"} ${url}: ${response.status} ${await response.text()}`,
      );
    }
    return { data: await response.json(), next: nextLink(response) };
  }

  async function all(path, pick = (data) => data) {
    const items = [];
    for (let url = path; url;) {
      const { data, next } = await request(url);
      items.push(...pick(data));
      url = next;
    }
    return items;
  }

  const repo = `/repos/${repository}`;
  return {
    mergedSince: (since) =>
      all(
        `/search/issues?per_page=100&q=${encodeURIComponent(
          `repo:${repository} is:pr is:merged merged:>=${since.toISOString().replace(/\.\d+Z$/, "Z")}`,
        )}`,
        (data) => data.items,
      ),
    reviews: (number) => all(`${repo}/pulls/${number}/reviews?per_page=100`),
    comments: (number) => all(`${repo}/issues/${number}/comments?per_page=100`),
    openIssuesByActions: () =>
      all(
        `${repo}/issues?state=open&creator=${encodeURIComponent("github-actions[bot]")}&per_page=100`,
      ),
    createIssue: (body) =>
      request(`${repo}/issues`, { method: "POST", body: JSON.stringify(body) }),
    comment: (number, body) =>
      request(`${repo}/issues/${number}/comments`, {
        method: "POST",
        body: JSON.stringify({ body }),
      }),
  };
}

function nextLink(response) {
  const match = response.headers.get("link")?.match(/<([^>]+)>;\s*rel="next"/);
  return match ? match[1] : null;
}

async function trackingIssue(github) {
  const existing = (await github.openIssuesByActions()).find(
    (issue) => !issue.pull_request && issue.title === ISSUE_TITLE,
  );
  if (existing) return existing.number;
  const { data } = await github.createIssue({
    title: ISSUE_TITLE,
    body: "The merge health workflow (`.github/workflows/merge-health.yml`) comments here every Monday with the previous week's numbers. Keep this issue open so the reports stay in one place.",
  });
  return data.number;
}

async function main(argv) {
  const dryRun = argv.includes("--dry-run");
  const { GITHUB_TOKEN, GITHUB_REPOSITORY, GITHUB_STEP_SUMMARY } = process.env;
  if (!GITHUB_TOKEN || !GITHUB_REPOSITORY) {
    throw new Error("GITHUB_TOKEN and GITHUB_REPOSITORY must be set");
  }
  const github = client({
    token: GITHUB_TOKEN,
    repository: GITHUB_REPOSITORY,
    apiUrl: process.env.GITHUB_API_URL ?? "https://api.github.com",
  });
  const until = new Date();
  const since = new Date(until.getTime() - 7 * DAY_MS);

  const prs = [];
  const merged = (await github.mergedSince(since)).filter(
    (item) => Date.parse(item.pull_request.merged_at) >= since.getTime(),
  );
  for (const item of merged) {
    const [reviews, comments] = await Promise.all([
      github.reviews(item.number),
      github.comments(item.number),
    ]);
    prs.push({
      number: item.number,
      title: item.title,
      labels: item.labels,
      created_at: item.created_at,
      merged_at: item.pull_request.merged_at,
      rounds: reviewRounds(reviews, comments),
    });
  }

  const report = formatReport(summarise(prs), { since, until });
  console.log(report);
  if (GITHUB_STEP_SUMMARY) appendFileSync(GITHUB_STEP_SUMMARY, `${report}\n`);
  if (dryRun) return;
  const issue = await trackingIssue(github);
  await github.comment(issue, report);
  console.error(`Posted to issue #${issue}`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(`merge-health failed: ${error.message}`);
    process.exitCode = 1;
  });
}
