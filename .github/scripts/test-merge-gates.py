#!/usr/bin/env python3
"""Exercise the real gate scripts with a local gh stub; never contact GitHub."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
HEAD = "0123456789abcdef0123456789abcdef01234567"
CREATOR = "merge-gates-test[bot]"
DESCRIPTION = "Copilot reviewed 0123456; Codex reviewed the PR"
HAS_PROCESS_SUBSTITUTION = Path("/dev/fd").exists()

GH_STUB = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(os.environ["GATE_TEST_DIR"])
args = sys.argv[1:]
if not args or args.pop(0) != "api":
    sys.exit("unexpected gh command")
args = [arg for arg in args if arg != "--paginate"]
endpoint = args.pop(0)
fields = dict(arg.split("=", 1) for arg in args if "=" in arg)
with (root / "calls.jsonl").open("a") as log:
    log.write(json.dumps([endpoint, fields]) + "\n")
if endpoint.endswith("/statuses/" + os.environ["TEST_HEAD"]):
    if os.environ.get("FAIL_FETCH") == "pending" and fields.get("state") == "pending":
        sys.exit(73)
    (root / "state").write_text(fields["state"])
    print("{}")
elif endpoint.endswith("/pulls/109"):
    print((root / "pr.json").read_text())
elif "/files?" in endpoint:
    if os.environ.get("FAIL_FETCH") == "files":
        sys.exit(73)
    print((root / "files.json").read_text())
elif "/comments?" in endpoint:
    if os.environ.get("FAIL_FETCH") == "comments":
        sys.exit(73)
    print((root / "comments.json").read_text())
elif "/reviews?" in endpoint:
    print((root / "reviews.json").read_text())
elif "/reactions?" in endpoint:
    print("[]")
elif "/commits/" in endpoint and "/statuses?" in endpoint:
    print((root / "statuses.json").read_text())
elif "/labels" in endpoint:
    print("{}")
else:
    sys.exit("unexpected endpoint: " + endpoint)
'''


class GateTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.work = Path(self.tmp.name)
        stub = self.work / "gh"
        stub.write_text(GH_STUB)
        stub.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{self.work}{os.pathsep}{os.environ['PATH']}",
            "GATE_TEST_DIR": str(self.work),
            "TEST_HEAD": HEAD,
            "REPO": "vultuk/whatsapp-translator",
            "PR_NUMBER": "109",
            "APPROVERS": "vultuk",
            "GH_TOKEN": "local-stub-only",
            "STATUS_CREATOR": CREATOR,
            "POLL_SECONDS": "0",
            "TMPDIR": str(self.work),
            "GITHUB_STEP_SUMMARY": str(self.work / "summary"),
            "COMMENT_ID": "",
        }
        self.pr = {
            "state": "open",
            "head": {"sha": HEAD, "ref": "fix/gates"},
            "base": {"ref": "main", "repo": {"default_branch": "main"}},
            "labels": [],
        }
        self.write("pr", self.pr)
        self.write("files", [{"filename": ".github/workflows/test.yml",
                              "status": "modified", "additions": 1, "deletions": 1}])
        self.write("comments", [])
        self.write("reviews", [
            {"user": {"login": "copilot-pull-request-reviewer[bot]"}, "commit_id": HEAD},
            {"user": {"login": "chatgpt-codex-connector[bot]"}, "commit_id": HEAD},
        ])
        self.write("statuses", [])
        (self.work / "state").write_text("success")

    def write(self, name, value):
        (self.work / f"{name}.json").write_text(json.dumps(value))

    def run_gate(self, name):
        return subprocess.run(
            ["bash", str(ROOT / ".github/scripts" / name), "run"],
            cwd=ROOT, env=self.env, text=True, capture_output=True, timeout=15,
        )

    def calls(self):
        path = self.work / "calls.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def states(self):
        return [fields["state"] for _, fields in self.calls() if "state" in fields]

    def test_risk_blocks_before_files_fetch_failure(self):
        self.env["FAIL_FETCH"] = "files"
        result = self.run_gate("pr-risk.sh")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.states(), ["pending"])
        self.assertEqual((self.work / "state").read_text(), "pending")

    def test_risk_blocks_before_comments_fetch_failure(self):
        self.env["FAIL_FETCH"] = "comments"
        result = self.run_gate("pr-risk.sh")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.states(), ["pending"])
        self.assertEqual((self.work / "state").read_text(), "pending")

    def test_failed_pending_write_stops_validation(self):
        self.env["FAIL_FETCH"] = "pending"
        result = self.run_gate("pr-risk.sh")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(self.calls()), 2)
        self.assertEqual(self.states(), ["pending"])

    def test_other_base_never_writes_either_gate(self):
        self.pr["base"]["ref"] = "release"
        self.write("pr", self.pr)
        for script in ("pr-risk.sh", "review-gate.sh"):
            with self.subTest(script=script):
                result = self.run_gate(script)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.states(), [])

    def test_closed_pr_never_writes_either_gate(self):
        self.pr["state"] = "closed"
        self.write("pr", self.pr)
        for script in ("pr-risk.sh", "review-gate.sh"):
            with self.subTest(script=script):
                result = self.run_gate(script)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.states(), [])

    @unittest.skipUnless(HAS_PROCESS_SUBSTITUTION, "Bash process substitution needs /dev/fd")
    def test_high_risk_without_approval_fails(self):
        result = self.run_gate("pr-risk.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.states(), ["pending", "failure"])

    @unittest.skipUnless(HAS_PROCESS_SUBSTITUTION, "Bash process substitution needs /dev/fd")
    def test_high_risk_with_current_manual_approval_passes(self):
        self.write("comments", [{"id": 1, "body": f"/approve {HEAD}",
                                 "user": {"login": "vultuk"},
                                 "created_at": "t", "updated_at": "t",
                                 "performed_via_github_app": None}])
        result = self.run_gate("pr-risk.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.states(), ["pending", "success"])

    @unittest.skipUnless(HAS_PROCESS_SUBSTITUTION, "Bash process substitution needs /dev/fd")
    def test_low_risk_still_passes(self):
        self.write("files", [{"filename": "docs/a.md", "status": "modified",
                              "additions": 1, "deletions": 0}])
        result = self.run_gate("pr-risk.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.states(), ["pending", "success"])

    def status(self, creator, state="success", description=DESCRIPTION):
        return {"context": "review-gate", "state": state,
                "description": description, "creator": {"login": creator}}

    def test_same_text_foreign_status_cannot_suppress_app_publication(self):
        # A real older App status must not hide a newer foreign status, either.
        self.write("statuses", [self.status("github-actions[bot]"), self.status(CREATOR)])
        result = self.run_gate("review-gate.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.states(), ["success"])

    def test_same_text_human_status_cannot_suppress_app_publication(self):
        self.write("statuses", [self.status("vultuk")])
        result = self.run_gate("review-gate.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.states(), ["success"])

    def test_matching_latest_app_status_is_deduplicated(self):
        self.write("statuses", [self.status(CREATOR)])
        result = self.run_gate("review-gate.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.states(), [])

    def test_changed_app_verdict_is_published(self):
        self.write("statuses", [self.status(CREATOR, "pending", "Waiting for Codex")])
        result = self.run_gate("review-gate.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.states(), ["success"])

    def test_old_copilot_review_still_blocks(self):
        self.write("reviews", [
            {"user": {"login": "copilot-pull-request-reviewer[bot]"}, "commit_id": "old"},
            {"user": {"login": "chatgpt-codex-connector[bot]"}, "commit_id": HEAD},
        ])
        result = self.run_gate("review-gate.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.states(), ["pending"])

    def test_missing_creator_fails_before_api_requests(self):
        del self.env["STATUS_CREATOR"]
        result = self.run_gate("review-gate.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
