#!/usr/bin/env python3
"""Collect review comments from the local editor and the e-ink notes, and post them.

A review produces comments in two places:

  * Neovim: `<leader>gc` at a line, which appends to the PR's JSONL;
  * the e-ink tablet: an annotation that is about the code, recorded here by the agent.

`list` shows what would be posted. `post` creates a GitHub **pending review** (the default,
so it is finished in the GitHub UI) or submits it as a comment. A comment on a line that
is not in the diff cannot be inline; it goes into the review body, and the script says so.

State lives beside the launcher's registry:

    ~/.local/state/eink-bridge/comments/pr-<n>.jsonl
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

STATE = Path(os.environ.get("EINK_PR_STATE", Path.home() / ".local/state/eink-bridge"))
GH_PR = re.compile(r"github\.com/([^/\s]+)/([^/\s]+?)(?:\.git)?/pull/(\d+)")


def comments_path(pr: int, state: Path | None = None) -> Path:
    return (state or STATE) / "comments" / f"pr-{pr}.jsonl"


def load_comments(pr: int, state: Path | None = None) -> list[dict]:
    path = comments_path(pr, state)
    if not path.exists():
        return []
    comments = []
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip():
            continue
        try:
            comments.append(json.loads(line))
        except json.JSONDecodeError as error:
            raise SystemExit(f"{path}:{number}: not JSON: {error}")
    return comments


def add_comment(pr: int, path: str, line: int, side: str, body: str,
                source: str = "nvim", worktree: str = "",
                state: Path | None = None) -> dict:
    if line < 1:
        raise SystemExit("line must be >= 1")
    if not body.strip():
        raise SystemExit("a comment needs text")
    entry = {
        "pr": pr,
        "path": path,
        "line": line,
        "side": side.upper(),
        "body": body.strip(),
        "source": source,
        "worktree": worktree,
        "created_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    }
    target = comments_path(pr, state)
    target.parent.mkdir(parents=True, exist_ok=True)
    with target.open("a") as handle:
        handle.write(json.dumps(entry) + "\n")
    return entry


def pr_ref(value: str, repo: str | None = None) -> tuple[int, str | None]:
    """Accept a number or a GitHub URL, and return (number, repo-or-None)."""
    if value.isdigit():
        return int(value), repo
    match = GH_PR.search(value)
    if not match:
        raise SystemExit(f"expected a pull-request number or URL, got {value!r}")
    return int(match.group(3)), f"{match.group(1)}/{match.group(2)}"


def repo_from_registry(pr: int, state: Path | None = None) -> str | None:
    """A PR number alone is enough when the launcher recorded its worktree."""
    registry = (state or STATE) / "pr-reviews.json"
    try:
        rows = json.loads(registry.read_text()).get("reviews", [])
    except (FileNotFoundError, json.JSONDecodeError):
        return None
    for row in rows:
        if str(row.get("pr")) != str(pr):
            continue
        worktree = row.get("worktree")
        if not worktree:
            continue
        origin = subprocess.run(
            ["git", "-C", worktree, "remote", "get-url", "origin"],
            capture_output=True, text=True,
        ).stdout.strip()
        match = re.search(r"[:/]([^/:]+)/([^/]+?)(?:\.git)?$", origin)
        if match:
            return f"{match.group(1)}/{match.group(2)}"
    return None


def hunk_lines(patch: str) -> tuple[set[int], set[int]]:
    """Line numbers a GitHub inline comment may use, (old, new), for one file patch.

    Every line in a hunk — added, removed, and context — is commentable; the side says
    which file's numbering. Lines outside the hunks are not.
    """
    old, new = set(), set()
    old_line = new_line = 0
    for raw in patch.splitlines():
        header = re.match(r"^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@", raw)
        if header:
            old_line, new_line = int(header.group(1)), int(header.group(2))
            continue
        if raw.startswith("\\"):
            continue
        if raw.startswith("+"):
            new.add(new_line)
            new_line += 1
        elif raw.startswith("-"):
            old.add(old_line)
            old_line += 1
        elif raw.startswith(" "):
            old.add(old_line)
            new.add(new_line)
            old_line += 1
            new_line += 1
    return old, new


def split_comments(comments: list[dict], files: list[dict]) -> tuple[list[dict], list[str]]:
    """Inline comments GitHub can place, and body lines for the rest."""
    by_path = {entry.get("filename"): entry for entry in files}
    inline: list[dict] = []
    body: list[str] = []
    for comment in comments:
        path = comment["path"]
        side = comment.get("side", "RIGHT").upper()
        file_entry = by_path.get(path)
        if file_entry is not None:
            old, new = hunk_lines(file_entry.get("patch") or "")
            placeable = comment["line"] in (new if side == "RIGHT" else old)
        else:
            placeable = False
        if placeable:
            inline.append({
                "path": path,
                "line": comment["line"],
                "side": side,
                "body": comment["body"],
            })
        else:
            body.append(f"- `{path}:{comment['line']}` — {comment['body']}")
    return inline, body


def build_payload(inline: list[dict], unplaceable: list[str],
                  summary: str = "", submit: bool = False) -> dict:
    body = summary.strip()
    if unplaceable:
        block = "Comments on lines outside the diff:\n" + "\n".join(unplaceable)
        body = f"{body}\n\n{block}" if body else block
    payload: dict = {"body": body, "comments": inline}
    if submit:
        payload["event"] = "COMMENT"
    return payload


def pull_files(repo: str, pr: int) -> list[dict]:
    """The changed files with their patches, from the API GitHub will validate against."""
    result = subprocess.run(
        ["gh", "api", "--paginate", f"repos/{repo}/pulls/{pr}/files", "--jq", ".[]"],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        raise SystemExit(f"gh could not read the PR files: {result.stderr.strip()}")
    files = []
    for line in result.stdout.splitlines():
        if line.strip():
            files.append(json.loads(line))
    return files


def post_review(repo: str, pr: int, payload: dict) -> dict:
    result = subprocess.run(
        ["gh", "api", "-X", "POST", f"repos/{repo}/pulls/{pr}/reviews", "--input", "-"],
        input=json.dumps(payload), capture_output=True, text=True,
    )
    if result.returncode != 0:
        raise SystemExit(f"gh could not create the review: {result.stderr.strip()}")
    return json.loads(result.stdout)


def cmd_add(args) -> int:
    pr, _ = pr_ref(args.pr, args.repo)
    entry = add_comment(pr, args.path, args.line, args.side, args.body,
                        source=args.source, worktree=args.worktree)
    print(f"recorded {entry['path']}:{entry['line']} ({entry['side']}) for PR {pr}")
    return 0


def cmd_list(args) -> int:
    pr, _ = pr_ref(args.pr, args.repo)
    comments = load_comments(pr)
    if args.json:
        print(json.dumps(comments, indent=2))
        return 0
    if not comments:
        print(f"no comments for PR {pr}")
        return 0
    for index, comment in enumerate(comments, 1):
        print(f"{index:2}. [{comment.get('source', '?')}] "
              f"{comment['path']}:{comment['line']} ({comment.get('side', 'RIGHT')})")
        for line in comment["body"].splitlines():
            print(f"    {line}")
    return 0


def cmd_post(args) -> int:
    pr, repo = pr_ref(args.pr, args.repo)
    repo = repo or repo_from_registry(pr)
    if not repo:
        raise SystemExit("pass --repo owner/name (no registry entry names this PR)")
    comments = load_comments(pr)
    if not comments:
        raise SystemExit(f"no comments for PR {pr}; nothing to post")
    files = pull_files(repo, pr)
    inline, unplaceable = split_comments(comments, files)
    payload = build_payload(inline, unplaceable, args.body, submit=args.submit)
    print(f"PR {repo}#{pr}: {len(inline)} inline, {len(unplaceable)} in the body"
          + (", submitted" if args.submit else ", pending"))
    if args.dry_run:
        print(json.dumps(payload, indent=2))
        return 0
    if not args.yes:
        raise SystemExit("refusing to post without --yes (ask the reviewer first)")
    review = post_review(repo, pr, payload)
    url = review.get("html_url") or review.get("url") or "(no URL returned)"
    print(f"review: {url}")
    # A successful post must not be repeated by a retry or a second agent.
    target = comments_path(pr)
    posted = target.with_name(
        f"{target.stem}.posted-{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')}.jsonl"
    )
    target.replace(posted)
    print(f"moved the posted comments to {posted.name}")
    return 0


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    add = sub.add_parser("add", help="record one comment")
    add.add_argument("--pr", required=True, help="number or GitHub URL")
    add.add_argument("--repo", help="owner/name when --pr is a number")
    add.add_argument("--path", required=True, help="file path, relative to the checkout")
    add.add_argument("--line", type=int, required=True)
    add.add_argument("--side", choices=["RIGHT", "LEFT", "right", "left"], default="RIGHT")
    add.add_argument("--body", required=True)
    add.add_argument("--source", choices=["nvim", "eink", "agent"], default="agent")
    add.add_argument("--worktree", default="")
    add.set_defaults(func=cmd_add)

    listing = sub.add_parser("list", help="show the comments for a PR")
    listing.add_argument("--pr", required=True)
    listing.add_argument("--repo")
    listing.add_argument("--json", action="store_true")
    listing.set_defaults(func=cmd_list)

    post = sub.add_parser("post", help="create a pending (or submitted) GitHub review")
    post.add_argument("--pr", required=True)
    post.add_argument("--repo")
    post.add_argument("--body", default="", help="summary shown above the comments")
    post.add_argument("--submit", action="store_true",
                      help="submit the review now instead of leaving it pending")
    post.add_argument("--dry-run", action="store_true")
    post.add_argument("--yes", action="store_true")
    post.set_defaults(func=cmd_post)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
