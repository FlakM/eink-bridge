#!/usr/bin/env python3
"""Per-review keying and the live-review registry.

Several PR reviews run at once, so every per-PR resource must be derived from one key
and never collide. This module is the single place that decides those names, and the
single place that records what is live, so a second run reattaches rather than opening a
duplicate tab, socket, worktree or tablet session.

Pure and file-based, so it is unit-tested without touching herdr or the network.
"""
from __future__ import annotations

import json
import os
import re
from dataclasses import asdict, dataclass, field
from pathlib import Path

DEFAULT_REGISTRY = Path(
    os.environ.get("EINK_PR_REGISTRY",
                   Path.home() / ".local/state/eink-bridge/pr-reviews.json"))


def review_key(pr) -> str:
    """A filesystem- and socket-safe key for a PR number or a self-review label."""
    text = str(pr).strip().lower()
    text = re.sub(r"[^a-z0-9._-]+", "-", text).strip("-")
    if not text:
        raise ValueError("review key would be empty")
    return text


def worktree_dir(repo_root, pr) -> str:
    parent = os.path.dirname(os.path.abspath(repo_root))
    repo = os.path.basename(os.path.abspath(repo_root))
    return os.path.join(parent, f"{repo}-worktrees", f"pr-{review_key(pr)}")


def socket_path(pr) -> str:
    return f"/tmp/eink-pr-{review_key(pr)}.sock"


def tab_label(pr, title) -> str:
    short = " ".join(str(title).split())[:48]
    return f"PR {pr}: {short}" if not str(pr).startswith("self-") else f"Self: {short}"


def session_tags(pr, sha) -> dict:
    return {"pr": str(pr), "sha": str(sha)[:12]}


@dataclass
class Review:
    key: str
    pr: str
    sha: str
    worktree: str
    socket: str
    tab_label: str
    title: str = ""
    session: str = ""
    tab_id: str = ""
    pane_id: str = ""

    def as_row(self):
        return asdict(self)


@dataclass
class Registry:
    path: Path = field(default_factory=lambda: DEFAULT_REGISTRY)
    reviews: dict = field(default_factory=dict)

    @classmethod
    def load(cls, path=None):
        path = Path(path) if path else DEFAULT_REGISTRY
        registry = cls(path=path)
        if path.is_file():
            try:
                payload = json.loads(path.read_text())
            except (json.JSONDecodeError, OSError):
                # A corrupt registry must not stop a review; start clean and keep going.
                return registry
            registry.reviews = {row["key"]: Review(**row) for row in payload.get("reviews", [])}
        return registry

    def save(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        payload = {"reviews": [row.as_row() for row in
                               sorted(self.reviews.values(), key=lambda review: review.key)]}
        temporary = self.path.with_suffix(".tmp")
        temporary.write_text(json.dumps(payload, indent=2) + "\n")
        temporary.replace(self.path)

    def upsert(self, review: Review) -> Review:
        """Reattach if we already know this review; keep fields the caller omitted."""
        existing = self.reviews.get(review.key)
        merged = {**(existing.as_row() if existing else {}),
                  **{key: value for key, value in review.as_row().items() if value}}
        merged["key"] = review.key
        row = Review(**merged)
        self.reviews[review.key] = row
        return row

    def find(self, pr) -> Review | None:
        return self.reviews.get(review_key(pr))

    def remove(self, pr) -> Review | None:
        return self.reviews.pop(review_key(pr), None)
