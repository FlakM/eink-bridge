#!/usr/bin/env python3
"""Several PR reviews at once: prove the per-review resources stay isolated.

Creates N throwaway git worktrees in a temp clone (never the user's repo), registers a
review for each, and checks that worktree, socket, tab label and session tag differ.
With --with-herdr it also opens and closes real herdr tabs, so the layout path is
exercised; without it the run stays headless and safe to repeat.

Usage:
  multi_pr.py                    # 3 reviews, worktrees + registry only
  multi_pr.py --count 5
  multi_pr.py --with-herdr       # also open/close a herdr tab per review
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
SCRIPTS = HERE.parent / "skills/eink-pr/scripts"

spec = importlib.util.spec_from_file_location("eink_registry", SCRIPTS / "registry.py")
registry = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = registry
spec.loader.exec_module(registry)

TITLES = [
    "Add pagination to list sessions",
    "Quiet notifications",
    "Rework the ingest pipeline",
    "Fix the expiry loop",
    "Split the render module",
]


def run(command, **kwargs):
    return subprocess.run(command, capture_output=True, text=True, check=True, **kwargs)


def make_repo(root):
    repo = Path(root) / "repo"
    repo.mkdir()
    run(["git", "init", "-q", "-b", "main", str(repo)])
    run(["git", "-C", str(repo), "config", "user.email", "test@example.com"])
    run(["git", "-C", str(repo), "config", "user.name", "test"])
    (repo / "file.txt").write_text("one\n")
    run(["git", "-C", str(repo), "add", "."])
    run(["git", "-C", str(repo), "commit", "-qm", "base"])
    return repo


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--count", type=int, default=3)
    parser.add_argument("--with-herdr", action="store_true")
    args = parser.parse_args(argv)

    problems = []
    tabs = []
    with tempfile.TemporaryDirectory() as root:
        repo = make_repo(root)
        register_path = Path(root) / "pr-reviews.json"
        reg = registry.Registry.load(register_path)

        for index in range(args.count):
            pr = 100 + index
            title = TITLES[index % len(TITLES)]
            worktree = registry.worktree_dir(str(repo), pr)
            run(["git", "-C", str(repo), "worktree", "add", "--detach", worktree, "HEAD"])
            review = registry.Review(
                key=registry.review_key(pr), pr=str(pr), sha="f" * 40,
                worktree=worktree, socket=registry.socket_path(pr),
                tab_label=registry.tab_label(pr, title), title=title,
                session=f"session-{pr}")
            reg.upsert(review)
            print(f"registered PR {pr}: worktree={worktree} socket={review.socket}")

        reg.save()
        reloaded = registry.Registry.load(register_path)

        resources = {
            "worktrees": [r.worktree for r in reloaded.reviews.values()],
            "sockets": [r.socket for r in reloaded.reviews.values()],
            "tabs": [r.tab_label for r in reloaded.reviews.values()],
            "sessions": [r.session for r in reloaded.reviews.values()],
        }
        for name, values in resources.items():
            if len(set(values)) != len(values):
                problems.append(f"{name} collided: {values}")
        for review in reloaded.reviews.values():
            if not Path(review.worktree).is_dir():
                problems.append(f"worktree missing for PR {review.pr}: {review.worktree}")
            if not os.environ.get("EINK_PR_REGISTRY") and not review.socket.startswith("/tmp/"):
                problems.append(f"socket not per-review: {review.socket}")

        # Reattach must not duplicate anything.
        before = len(reloaded.reviews)
        reloaded.upsert(registry.Review(key=registry.review_key(100), pr="100", sha="a" * 40,
                                        worktree="/tmp/ignored", socket="/tmp/ignored",
                                        tab_label="ignored"))
        after = registry.Registry.load(register_path)
        after.upsert(registry.Review(key=registry.review_key(100), pr="100", sha="b" * 40,
                                     worktree="/tmp/ignored", socket="/tmp/ignored",
                                     tab_label="ignored"))
        if len(after.reviews) != before:
            problems.append("reattach created a duplicate review")
        if not after.find(100).session:
            problems.append("reattach dropped the session id")

        if args.with_herdr:
            if not os.environ.get("HERDR_ENV"):
                problems.append("--with-herdr needs HERDR_ENV=1")
            else:
                for review in reloaded.reviews.values():
                    created = run(["herdr", "tab", "create", "--no-focus",
                                   "--workspace", os.environ["HERDR_WORKSPACE_ID"],
                                   "--cwd", review.worktree, "--label", review.tab_label])
                    payload = json.loads(created.stdout)
                    tabs.append(payload["result"]["tab"]["tab_id"])
                if len(set(tabs)) != len(tabs):
                    problems.append(f"herdr gave duplicate tab ids: {tabs}")
                print(f"opened {len(tabs)} herdr tabs")

        # Cleanup: only what this run created.
        for tab in tabs:
            subprocess.run(["herdr", "tab", "close", tab], capture_output=True, text=True)
        for review in registry.Registry.load(register_path).reviews.values():
            subprocess.run(["git", "-C", str(repo), "worktree", "remove", "--force", review.worktree],
                           capture_output=True, text=True)
        leftovers = [r.worktree for r in reloaded.reviews.values() if Path(r.worktree).exists()]
        if leftovers:
            problems.append(f"worktrees not cleaned up: {leftovers}")

    for problem in problems:
        print(f"FAIL {problem}")
    if not problems:
        print(f"PASS multi-PR: {args.count} reviews isolated, reattach stable, cleanup complete")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
