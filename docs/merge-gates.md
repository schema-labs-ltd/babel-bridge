# Merge gate setup

`risk` and `review-gate` are commit statuses. Their names alone are not a
security boundary: a same-repository PR can add a workflow with
`statuses: write` and post either name through `GITHUB_TOKEN`.

Both gates therefore publish through a dedicated GitHub App. The code,
credential isolation, and required-check source settings below are all
necessary. Do not treat the migration as complete until the rollout checks
pass. Missing App configuration deliberately fails; there is no fallback to
`GITHUB_TOKEN` or a repository secret.

## Owner configuration before merging the migration

1. Register a dedicated GitHub App and install it only on this repository.
   Grant repository permissions: Contents **read**, Issues **write**, Pull
   requests **read**, and Commit statuses **write**. Do not grant Contents
   write, Workflows write, or Administration. The workflows request narrower
   installation tokens for their respective tasks.
2. Create an environment named **merge-gates**. Under deployment branches
   and tags, choose **Selected branches and tags** and add a **branch** rule
   for `main` only (the current default branch). Do not add tag rules or PR
   refs. Do not use “Protected branches only”: without branch protection that
   option can permit every branch. A job's `if` condition is not a substitute
   for this server-enforced environment restriction.
3. In that environment, store `MERGE_GATES_APP_CLIENT_ID` as a variable and
   `MERGE_GATES_APP_PRIVATE_KEY` as a secret. Keep the private key exclusively
   in this environment, with no repository/organization secret copy and no
   other installation-token distribution to PR-controlled jobs. Restrict who
   can change the App, the environment, and branch protection settings.
4. Plan the source switch before merging. Keep automatic merging disabled
   during the transition. After the new trusted workflows publish initial
   statuses, require both `risk` and `review-gate` on `main` and choose this
   dedicated App as the expected source for **each** context, preserving all
   other required checks. Do not select “any source” or GitHub Actions. The
   App may appear in the selector only after it has published a status.

The private key is never available to the PR self-test job. Gate jobs use
default-branch workflows and scripts, and ordinary `GITHUB_TOKEN` permissions
remain read-only. App tokens default to the current repository and are revoked
by the token action after the job. Do not add a PR-controlled trigger to a job
that can access this environment.

## Trigger behavior

- `risk` still checks new pushes and reevaluates created, edited, or deleted
  `/approve` comments. It sets `pending` before fetching files or comments,
  so a subsequent fetch failure cannot leave an earlier success standing.
- `review-gate` polls a new head for up to 15 minutes. Later bot reviews and
  reactions are picked up by the ten-minute schedule or a manual run from
  `main`. There is no `pull_request_review` execution of a PR workflow with
  gate credentials. A review arriving after polling may wait for the next
  scheduled run; GitHub can delay scheduled runs.
- Manual runs must select the default branch. Neither gate posts for closed
  PRs or PRs into another base branch. A new default branch requires updating
  the environment branch rule before using the gates there.
- Review status deduplication accepts only a matching **latest** status from
  `<dedicated-app-slug>[bot]`. A newer foreign status forces a fresh App
  publication even if an older App status has identical text. Repeated
  unchanged App results still deduplicate to avoid exhausting status limits.

## Rollout checks requiring owner access

1. Run both workflows from `main` on a disposable PR. Confirm the statuses'
   creator is the dedicated App and matches the action's `app-slug` output.
2. Verify environment access is rejected for both a same-repository PR
   workflow and a manual dispatch from a non-default branch. Never expose the
   real private key to a test workflow; use a harmless environment probe.
3. On the disposable PR, confirm an unapproved high-risk change stays blocked
   even when a different producer posts successful `risk` and `review-gate`
   statuses. Check the required status source in the merge box; do not merge
   the probe. Confirm legitimate App statuses can satisfy both requirements.
4. Approve that disposable high-risk head **by hand**, confirm success, then
   edit/delete the approval and confirm risk is reevaluated and blocked.
   A new head must require a new manual approval and a current-head Copilot
   review. Agents must never post `/approve` themselves.
5. Confirm delayed Codex evidence is picked up by polling/schedule, all other
   required CI checks still pass, and then restore the intended auto-merge
   policy. If any configuration or negative check is unverified, keep the
   migration draft and do not claim the gate-spoofing finding is closed.

Local focused checks (Bash, jq, Python 3, and Git required):

```sh
bash -n .github/scripts/pr-risk.sh .github/scripts/review-gate.sh
bash .github/scripts/pr-risk.sh selftest
bash .github/scripts/review-gate.sh selftest
python3 .github/scripts/test-merge-gates.py
```

The Python suite stubs `gh` locally and never writes real statuses or approval
comments. Its three successful risk-evaluation cases require Bash process
substitution (`/dev/fd`); they explicitly skip if the host lacks it. GitHub's
Ubuntu runners provide it. The PR's `risk / selftest` runs both rule self-tests
and the regression suite without App credentials.

Primary references: [environment branch restrictions](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments),
[required-check App sources](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches#require-status-checks-before-merging),
and [GitHub App token action](https://github.com/actions/create-github-app-token).
