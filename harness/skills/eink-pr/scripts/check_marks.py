#!/usr/bin/env python3
"""Check that every mark in a review brief resolves to a real line in a checkout.

A brief's Marks section is a list like `- A → src/lib.rs:120`. The renderer turns each
`path:line` into a tappable reference, so a wrong path, a line past the end of the file, or
a stale worktree produces a pill that shows no code. Run this before delivering; it is
cheap, and it catches the class rather than the instance.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

MARK = re.compile(r"^\s*[-*]?\s*([A-F])\s*(?:→|->)\s*(\S+?):(\d+)\s*$", re.M)


def marks(brief: str) -> list[tuple[str, str, int]]:
    return [(letter, ref, int(line)) for letter, ref, line in MARK.findall(brief)]


def check(brief: str, roots: list[str]) -> list[tuple[str, str, int, str | None]]:
    results = []
    for letter, ref, line in marks(brief):
        reason = "no checkout contains the file"
        for root in roots:
            path = Path(root) / ref
            if not path.is_file():
                continue
            total = sum(1 for _ in path.open(errors="replace"))
            if line <= total:
                results.append((letter, ref, line, None))
                break
            reason = f"{path} has {total} lines"
        else:
            results.append((letter, ref, line, reason))
    return results


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("brief", help="markdown file, or - for stdin")
    parser.add_argument("--worktree", action="append", default=[], required=True,
                        help="a review checkout (repeat for a batch)")
    args = parser.parse_args(argv)

    text = sys.stdin.read() if args.brief == "-" else Path(args.brief).read_text()
    found = marks(text)
    if not found:
        print("no marks (A → path:line) found in the brief", file=sys.stderr)
        return 1

    failures = 0
    for letter, ref, line, reason in check(text, args.worktree):
        if reason is None:
            print(f"  ok   {letter}  {ref}:{line}")
        else:
            failures += 1
            print(f"  FAIL {letter}  {ref}:{line}  ({reason})")
    if failures:
        print(f"\n{failures} of {len(found)} marks do not resolve", file=sys.stderr)
        return 1
    print(f"\nall {len(found)} marks resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main())
