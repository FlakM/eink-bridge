#!/usr/bin/env python3
"""Exercise the tablet -> agent return path against a live e-ink server.

Sends a brief, posts a handwritten note that names a mark letter, resolves that letter
back to a file:line through the same code the skill uses, acknowledges the note, and
checks the server agrees the feedback is consumed. This is the half that unit tests
cannot cover: the real HTTP contract and the real acknowledgement semantics.

Usage:
  reader_loop.py --server http://localhost:3333
  reader_loop.py --server http://localhost:3333 --annotation "B looks wrong here"
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys
import urllib.error
import urllib.request

SCRIPTS = Path(__file__).resolve().parent.parent / "skills/eink-pr/scripts"
spec = importlib.util.spec_from_file_location("eink_review", SCRIPTS / "review.py")
review = importlib.util.module_from_spec(spec)
spec.loader.exec_module(review)

BRIEF = """# PR 88: Add pagination to list sessions

## Important pieces

- **A** — The new field is required and breaks old clients. Mark it optional.
- **B** — The pagination path has no test. Cover the empty and last page.

## Marks

- A → proto/api.proto:12
- B → server/tests/health_test.rs:40
"""
MARKS = {"A": "proto/api.proto:12", "B": "server/tests/health_test.rs:40"}


def request(server, path, body=None, method=None, timeout=15):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(server.rstrip("/") + path, data=data,
                                 headers={"Content-Type": "application/json"}, method=method)
    with urllib.request.urlopen(req, timeout=timeout) as response:
        raw = response.read()
        return json.loads(raw) if raw else None


def run_loop(server, annotation="B is wrong", brief=BRIEF, marks=None, log=print):
    marks = marks if marks is not None else MARKS
    steps = []

    created = request(server, "/api/sessions", {
        "title": "PR 88: Add pagination to list sessions",
        "content": brief,
        "tags": {"pr": "88", "sha": "abcdef123456"},
    })
    session_id, version = created["id"], 1
    steps.append(("created", session_id))
    log(f"created session {session_id} at version {version}")

    request(server, f"/api/sessions/{session_id}/request_update", {
        "document_version": version,
        "annotations": [{
            "anchor": None,
            "strokes": [[[1.0, 1.0], [2.0, 2.0]]],
            "recognized_text": annotation,
        }],
    }, "POST")
    steps.append(("posted", annotation))
    log(f"posted note {annotation!r}")

    pending = request(server, f"/api/sessions/{session_id}/feedback")["pending"]
    feedback_id = pending["feedback_id"]
    recognized = pending["annotations"][0]["recognized_text"]
    steps.append(("pending", feedback_id))
    log(f"server stored feedback {feedback_id}")

    letter, spot = review.resolve_reference(marks, recognized)
    steps.append(("resolved", spot))
    log(f"note names mark {letter} -> {spot}")

    updated = request(server, f"/api/sessions/{session_id}/content", {
        "content": brief + "\nThe reader flagged the test gap.\n",
        "expected_version": version,
        "feedback_id": feedback_id,
        "consumed_annotations": [0],
    }, "PUT")
    steps.append(("acknowledged", updated["version"]))
    log(f"acknowledged; document now version {updated['version']}")

    after = request(server, f"/api/sessions/{session_id}/feedback")
    steps.append(("cleared", after["pending"]))
    log(f"pending feedback after ack: {after['pending']}")

    request(server, f"/api/sessions/{session_id}", method="DELETE")
    steps.append(("cancelled", session_id))
    log(f"cancelled {session_id}")

    return {"session": session_id, "feedback_id": feedback_id, "letter": letter,
            "spot": spot, "version": updated["version"], "pending_after": after["pending"],
            "steps": steps, "recognized": recognized}


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument("--server", default=os.environ.get("EINK_SERVER_URL", "http://localhost:3333"))
    parser.add_argument("--annotation", default="B is wrong")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)

    try:
        result = run_loop(args.server, args.annotation, log=(lambda *_: None) if args.json else print)
    except urllib.error.URLError as error:
        print(f"e-ink server not reachable at {args.server}: {error}")
        return 2

    if args.json:
        print(json.dumps(result, indent=2))

    problems = []
    if result["letter"] != "B" or result["spot"] != MARKS["B"]:
        problems.append(f"mark resolution wrong: {result['letter']} -> {result['spot']}")
    if result["pending_after"] is not None:
        problems.append("acknowledged feedback is still pending")
    for problem in problems:
        print(f"FAIL {problem}")
    if not problems:
        print("PASS reader loop: note resolved to a marked line and was acknowledged exactly once")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
