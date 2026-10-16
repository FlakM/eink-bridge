#!/usr/bin/env python3
"""Deterministic core of the e-ink PR review pipeline.

The model stages (parallel reviewers, the `ihaveadhd` pass) emit and consume JSON;
this module owns every rule that must hold regardless of the model:

  filter   raw findings -> only the relevant, actionable ones
  render   findings -> tablet brief + Neovim mark letters
  lint     a brief -> brevity/humanness violations (non-zero exit blocks delivery)
  validate a findings file against the schema

Pure functions, no network, so the whole pipeline is testable without a model.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys

SEVERITIES = ("critical", "major", "minor", "nit")
SEVERITY_WEIGHT = {"critical": 4, "major": 3, "minor": 2, "nit": 1}
# Tags that widen the blast radius: a wrong call here hurts more than a local nit.
TAG_BOOST = {
    "public-api": 3.0,
    "behavior-change": 2.0,
    "migration": 3.0,
    "security": 3.0,
    "data-loss": 4.0,
    "concurrency": 2.0,
}
MARK_LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
REQUIRED_FINDING_FIELDS = ("file", "line", "claim", "action")

# Single words are matched on word boundaries: a substring test flagged "every" as the
# filler "very", and "adjust" as "just".
BANNED_WORDS = (
    "basically", "essentially", "simply", "just", "very", "really", "actually",
    "leverage", "utilize", "robust", "seamless", "delve", "landscape",
)
BANNED_PHRASES = (
    "note that", "it should be noted", "in order to", "as you can see",
    "make sure to", "please note", "it is important to",
)
BANNED_WORD_PATTERN = re.compile(r"\b(" + "|".join(BANNED_WORDS) + r")\b")
HEDGE_WORDS = ("might", "may", "maybe", "possibly", "perhaps", "arguably")
HEDGE_PHRASES = ("i think",)
HEDGE_WORD_PATTERN = re.compile(r"\b(" + "|".join(HEDGE_WORDS) + r")\b")
ACTION_VERBS = (
    "check", "verify", "confirm", "decide", "change", "add", "remove", "delete",
    "drop", "clear", "clean", "test", "cover", "guard", "clamp", "bound", "cap",
    "reject", "return", "handle", "fix", "reuse", "cache", "cancel", "retry",
    "wait", "assert", "fail", "log", "validate", "parse", "decode", "encode",
    "rename", "move", "split", "merge", "replace", "reset", "close", "expire",
    "notify", "surface", "expose", "hide", "document", "measure", "profile",
    "migrate", "rollback", "gate", "skip", "raise", "throw", "allow", "deny",
    "block", "serialize", "persist", "escape", "trim", "normalise", "normalize",
    "update", "mark", "name", "index", "borrow", "prefer", "enforce", "restrict",
    "scope", "wire", "plumb", "hoist", "factor", "extract", "inline", "convert",
    "switch", "gate", "bound", "flush", "drain", "seed", "stub", "mock", "instrument",
    # Natural imperative phrasing a human reviewer actually writes. Missing these made
    # the gate reject good briefs written as "Make it optional", "Avoid the clone".
    "make", "use", "avoid", "keep", "set", "write", "read", "send", "receive",
    "open", "accept", "ignore", "watch", "treat", "wrap", "unwrap", "forward",
    "propagate", "dedupe", "sanitize", "throttle", "batch", "debounce", "schedule",
    "await", "spawn", "join", "abort", "shutdown", "render", "mount", "coerce",
    "cast", "rework", "simplify", "tighten", "clarify", "explain", "justify",
)
# Necessity phrasing carries an implicit action without an imperative verb.
NECESSITY = ("must ", "should ", "needs to", "has to", "cannot ", "can't ", "won't ")
ACTION_PATTERN = re.compile(r"\b(" + "|".join(ACTION_VERBS) + r")\b")
# Claims this severe need a stated reason to be worth the reader's attention.
EVIDENCE_REQUIRED = ("major", "critical")
SUMMARY_HEADINGS = ("summary", "tl;dr", "tldr", "in conclusion", "conclusion")


# ── loading and validation ────────────────────────────────────────────────────

def load_document(path):
    with open(path) as handle:
        data = json.load(handle)
    if isinstance(data, list):
        return {}, data
    return data.get("pr", {}), data.get("findings", [])


def validate_finding(finding, index=0):
    """Return a list of human-readable problems. Empty means valid."""
    problems = []
    if not isinstance(finding, dict):
        return [f"finding[{index}] is not an object"]
    for field in REQUIRED_FINDING_FIELDS:
        if not str(finding.get(field, "")).strip():
            problems.append(f"finding[{index}] missing {field}")
    try:
        if int(finding.get("line", 0)) < 1:
            problems.append(f"finding[{index}] line must be >= 1")
    except (TypeError, ValueError):
        problems.append(f"finding[{index}] line {finding.get('line')!r} is not an integer")
    severity = finding.get("severity")
    if severity not in SEVERITIES:
        problems.append(f"finding[{index}] severity {severity!r} not in {SEVERITIES}")
    confidence = finding.get("confidence", 1.0)
    if not isinstance(confidence, (int, float)) or not 0.0 <= float(confidence) <= 1.0:
        problems.append(f"finding[{index}] confidence {confidence!r} not in 0..1")
    return problems


def validate_document(pr, findings):
    problems = []
    for index, finding in enumerate(findings):
        problems.extend(validate_finding(finding, index))
    return problems


def file_line_count(checkout, relative_path):
    if not checkout:
        return None
    full = os.path.join(checkout, relative_path)
    if not os.path.isfile(full):
        return None
    with open(full, "rb") as handle:
        return handle.read().count(b"\n") + 1


# ── filter ────────────────────────────────────────────────────────────────────

STOPWORDS = frozenset(
    "the a an and or but if then of to in on for with without is are was were be been "
    "this that these those it its as at by from into than too not no".split()
)


def stem(word):
    """A light, consistent plural strip so 'notifiers' and 'notifier' agree.

    Consistency matters more than correctness here: both sides of a comparison use
    the same rule, so a shared stem is enough to recognise the same finding worded
    twice. Only a single trailing 's' is dropped, so 'changes' -> 'change' without
    mangling 'status' into something unrecognisable across both sides.
    """
    if word.endswith("s") and len(word) > 4:
        return word[:-1]
    return word


def tokens(text):
    words = re.findall(r"[a-z0-9_]+", str(text).lower())
    return {stem(w) for w in words if w not in STOPWORDS and len(w) > 2}


def claim_similar(a, b, threshold=0.6):
    left, right = tokens(a), tokens(b)
    if not left or not right:
        return False
    union = left | right
    return len(left & right) / len(union) >= threshold


def rank_key(finding):
    severity = SEVERITY_WEIGHT.get(finding.get("severity"), 0)
    confidence = float(finding.get("confidence", 0.5))
    boost = sum(TAG_BOOST.get(tag, 0.0) for tag in finding.get("tags", []) or [])
    return (severity, confidence, boost)


def filter_findings(findings, checkout=None, limit=6, per_file=2, allow_nits=False):
    """Return (kept, dropped) where dropped is a list of (finding, reason)."""
    dropped = []
    valid = []
    nits = []
    for finding in findings:
        problems = validate_finding(finding)
        if problems:
            dropped.append((finding, "invalid: " + "; ".join(problems)))
            continue
        severity = finding.get("severity")
        confidence = float(finding.get("confidence", 1.0))
        if severity == "nit":
            if allow_nits:
                valid.append(finding)
            else:
                # Held aside rather than discarded: an empty page is worse than one nit.
                nits.append(finding)
            continue
        if confidence < 0.5 and severity != "critical":
            dropped.append((finding, f"low confidence {confidence}"))
            continue
        if severity in EVIDENCE_REQUIRED and not str(finding.get("evidence", "")).strip():
            dropped.append((finding, "high severity with no evidence"))
            continue
        count = file_line_count(checkout, finding["file"])
        if count is not None and int(finding["line"]) > count:
            dropped.append((finding, f"stale anchor: {finding['file']} has {count} lines"))
            continue
        valid.append(finding)

    valid.sort(key=rank_key, reverse=True)

    # Deduplicate near-identical findings, best-ranked first.
    deduped = []
    for finding in valid:
        duplicate = None
        for kept in deduped:
            same_spot = (
                kept["file"] == finding["file"]
                and abs(int(kept["line"]) - int(finding["line"])) <= 3
            )
            if same_spot and claim_similar(kept["claim"], finding["claim"]):
                duplicate = kept
                break
        if duplicate:
            dropped.append((finding, f"duplicate of {duplicate['file']}:{duplicate['line']}"))
        else:
            deduped.append(finding)

    # If nothing else survived, keep the best nit: something beats an empty page.
    if not deduped:
        if nits:
            nits.sort(key=rank_key, reverse=True)
            for extra in nits[1:]:
                dropped.append((extra, "nit (noise unless nothing else survives)"))
            return nits[:1], dropped
        if not allow_nits:
            return [], dropped
    for nit in nits:
        dropped.append((nit, "nit (noise unless nothing else survives)"))

    per_file_count = {}
    capped = []
    for finding in deduped:
        used = per_file_count.get(finding["file"], 0)
        if used >= per_file:
            dropped.append((finding, f"per-file cap ({per_file}) reached for {finding['file']}"))
            continue
        per_file_count[finding["file"]] = used + 1
        if len(capped) >= limit:
            dropped.append((finding, f"total cap ({limit}) reached"))
            continue
        capped.append(finding)
    return capped, dropped


# ── render ────────────────────────────────────────────────────────────────────

def opening_text(pr, key):
    """The opening lines come from the PR description and commits; a fallback states the
    gap rather than inventing context."""
    value = pr.get(key)
    if isinstance(value, list):
        value = " ".join(str(v) for v in value)
    if value and str(value).strip():
        return ensure_sentence(value)
    fallbacks = {
        "what": "The changed files and their diff.",
        "why": "Not stated in the description — infer it from the linked issue or commits.",
        "wider": "Callers and adjacent modules this touches.",
    }
    return fallbacks[key]


def render_opening(pr):
    return [f"**{label}** — {opening_text(pr, key)}" for label, key in OPENING]


def render_checks(pr):
    """Background checks are evidence: report failures, and say so briefly when clean."""
    checks = pr.get("checks") or []
    if not checks:
        return ["## Checks", "", "- Not run — say so rather than implying the change is tested."]
    interesting = [c for c in checks if str(c.get("status", "")).lower() not in ("pass", "passed", "green", "ok")]
    if not interesting:
        names = ", ".join(str(c.get("name")) for c in checks[:4])
        return ["## Checks", "", f"- All green: {names}."]
    lines = ["## Checks", ""]
    for check in interesting[:4]:
        detail = str(check.get("detail", "")).strip()
        lines.append(f"- **{check.get('name')}** {check.get('status')}." + (f" {ensure_sentence(detail)}" if detail else ""))
    return lines


def ensure_sentence(text):
    text = " ".join(str(text).split())
    if text and text[0].islower():
        text = text[0].upper() + text[1:]
    if text and text[-1] not in ".!?":
        text += "."
    return text


# The three questions the reader needs answered before anything else. The brief always
# opens with them, because a finding without its context is a guess the reader must
# re-derive from the diff.
OPENING = (("What changed", "what"), ("Why", "why"), ("Wider system", "wider"))


def render_brief(pr, findings, links=None, title_prefix="PR"):
    """Return (markdown, marks). links maps "file:line" -> permalink."""
    links = links or {}
    number = pr.get("number")
    title = pr.get("title") or "untitled"
    heading = f"{title_prefix} {number}: {title}" if number else f"{title_prefix}: {title}"

    lines = [f"# {heading}", ""]
    lines += render_opening(pr)
    lines.append("")
    lines += render_checks(pr)
    lines.append("")

    marks = {}
    if findings:
        lines.append("## Important pieces")
        lines.append("")
        for index, finding in enumerate(findings):
            letter = MARK_LETTERS[index]
            spot = f"{finding['file']}:{finding['line']}"
            marks[letter] = spot
            claim = ensure_sentence(finding["claim"]).rstrip(".")
            action = ensure_sentence(finding["action"])
            lines.append(f"- **{letter}** — {claim}. {action}")
        lines.append("")


    if marks:
        lines.append("## Marks")
        lines.append("")
        for letter, spot in marks.items():
            # Short references only: the reader navigates by pressing the letter in the
            # editor, so a full permalink is noise. Long links belong in the PR comment.
            lines.append(f"- {letter} → {spot}")
        lines.append("")

    return "\n".join(lines).rstrip() + "\n", marks


# ── lint ──────────────────────────────────────────────────────────────────────

def lint_brief(markdown, max_words=420, max_bullet_words=30, max_sentence_words=24):
    """Return a list of problems. Empty means the brief is deliverable."""
    problems = []
    body_lines = markdown.splitlines()
    headings = []
    in_diff_fence = False
    total_words = 0

    for number, raw in enumerate(body_lines, start=1):
        line = raw.rstrip()
        stripped = line.strip()
        if stripped.startswith("```"):
            in_diff_fence = not in_diff_fence
            if stripped.lower().startswith("```diff"):
                problems.append((number, "diff-dump", "fenced diff block belongs in the editor, not the tablet"))
            continue
        if in_diff_fence:
            continue
        if stripped.startswith("@@"):
            problems.append((number, "diff-dump", "raw hunk header"))
            continue
        if stripped.startswith("#"):
            headings.append(stripped.lstrip("# ").strip().lower())
            continue
        if not stripped:
            continue

        words = stripped.split()
        total_words += len(words)
        lowered = stripped.lower()
        for url in re.findall(r"https?://\S+", stripped):
            if len(url) > 60:
                problems.append((number, "long-link", f"{len(url)}-char URL belongs in the PR, not the tablet"))
        for phrase in BANNED_PHRASES:
            if phrase in lowered:
                problems.append((number, "filler", f"drop {phrase.strip()!r}"))
        for word in set(BANNED_WORD_PATTERN.findall(lowered)):
            problems.append((number, "filler", f"drop {word!r}"))
        for phrase in HEDGE_PHRASES:
            if phrase in lowered:
                problems.append((number, "hedge", f"commit or cut: {phrase!r}"))
        for word in set(HEDGE_WORD_PATTERN.findall(lowered)):
            problems.append((number, "hedge", f"commit or cut: {word!r}"))
        for sentence in re.split(r"[.!?]+\s+", stripped):
            if len(sentence.split()) > max_sentence_words:
                problems.append((number, "long-sentence", f"{len(sentence.split())} words"))

        for label, _key in OPENING:
            if stripped.startswith(f"**{label}**"):
                body = stripped.split("**", 2)[2]
                if len(body.split()) > max_bullet_words:
                    problems.append((number, "long-opening", f"{label}: {len(body.split())} words"))
        if stripped.startswith("-"):
            if len(words) > max_bullet_words:
                problems.append((number, "long-bullet", f"{len(words)} words (max {max_bullet_words})"))
            if mark_bullet(stripped):
                actionable = bool(ACTION_PATTERN.search(lowered)) or any(
                    phrase in lowered for phrase in NECESSITY)
                if not actionable:
                    problems.append((number, "not-actionable", "important piece has no action verb"))

    if markdown.strip():
        for label, _key in OPENING:
            if f"**{label}**" not in markdown:
                problems.append((0, "missing-opening", f"brief must state '{label}' before findings"))
        if "## Checks" not in markdown:
            problems.append((0, "missing-checks", "brief must say what was checked in the background"))
    for heading in headings:
        if heading in SUMMARY_HEADINGS:
            problems.append((0, "summary-stack", f"'{heading}' heading duplicates the overview"))
    if total_words > max_words:
        problems.append((0, "too-long", f"{total_words} words (max {max_words})"))
    return problems


def mark_bullet(line):
    """True for the '**A** — ...' important-piece bullets."""
    return re.match(r"^-\s+\*\*[A-Z]\*\*\s", line) is not None


def resolve_reference(marks, text):
    """Map a handwritten note back to a marked location.

    The reader annotates next to a letter ("B", "mark B", "->B"), so find the first
    standalone capital that is a real mark. Returns (letter, spot) or (None, None).
    """
    if not marks or not text:
        return None, None
    candidates = re.findall(r"\*\*([A-Z])\*\*|\b([A-Z])\b", text)
    for pair in candidates:
        letter = pair[0] or pair[1]
        if letter in marks:
            return letter, marks[letter]
    return None, None


def split_spot(spot):
    """'a/b.py:12' -> ('a/b.py', 12)."""
    path, _, line = spot.rpartition(":")
    return path, int(line)


# ── CLI ───────────────────────────────────────────────────────────────────────

def cmd_filter(args):
    pr, findings = load_document(args.findings)
    kept, dropped = filter_findings(
        findings, checkout=args.checkout, limit=args.max, per_file=args.per_file,
        allow_nits=args.allow_nits)
    payload = {"pr": pr, "findings": kept, "dropped": [
        {"file": f.get("file"), "line": f.get("line"), "claim": f.get("claim"), "reason": r}
        for f, r in dropped]}
    if args.out:
        with open(args.out, "w") as handle:
            json.dump(payload, handle, indent=2)
            handle.write("\n")
    print(f"kept {len(kept)} of {len(findings)} ({len(dropped)} dropped)")
    for finding, reason in dropped:
        print(f"  drop {finding.get('file')}:{finding.get('line')} — {reason}")
    return 0


def cmd_render(args):
    pr, findings = load_document(args.findings)
    links = json.load(open(args.links)) if args.links else {}
    markdown, marks = render_brief(pr, findings, links=links, title_prefix=args.prefix)
    if args.out:
        with open(args.out, "w") as handle:
            handle.write(markdown)
    else:
        sys.stdout.write(markdown)
    if args.marks_out:
        with open(args.marks_out, "w") as handle:
            json.dump(marks, handle, indent=2)
            handle.write("\n")
    return 0


def cmd_lint(args):
    with open(args.file) as handle:
        markdown = handle.read()
    problems = lint_brief(markdown, max_words=args.max_words,
                          max_bullet_words=args.max_bullet_words,
                          max_sentence_words=args.max_sentence_words)
    if not problems:
        print("brief ok")
        return 0
    for number, rule, message in problems:
        print(f"line {number}: {rule}: {message}")
    return 1


def cmd_validate(args):
    pr, findings = load_document(args.findings)
    problems = validate_document(pr, findings)
    for problem in problems:
        print(problem)
    return 1 if problems else 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    watch = sub.add_parser("filter", help="keep only relevant, actionable findings")
    watch.add_argument("--findings", required=True)
    watch.add_argument("--checkout", help="worktree root, to drop stale anchors")
    watch.add_argument("--out")
    watch.add_argument("--max", type=int, default=6)
    watch.add_argument("--per-file", type=int, default=2)
    watch.add_argument("--allow-nits", action="store_true")
    watch.set_defaults(func=cmd_filter)

    render = sub.add_parser("render", help="findings -> brief + mark letters")
    render.add_argument("--findings", required=True)
    render.add_argument("--links", help="JSON map file:line -> permalink")
    render.add_argument("--out")
    render.add_argument("--marks-out")
    render.add_argument("--prefix", default="PR")
    render.set_defaults(func=cmd_render)

    lint = sub.add_parser("lint", help="check a brief for brevity/humanness")
    lint.add_argument("--file", required=True)
    lint.add_argument("--max-words", type=int, default=420)
    lint.add_argument("--max-bullet-words", type=int, default=30)
    lint.add_argument("--max-sentence-words", type=int, default=24)
    lint.set_defaults(func=cmd_lint)

    validate = sub.add_parser("validate", help="check findings against the schema")
    validate.add_argument("--findings", required=True)
    validate.set_defaults(func=cmd_validate)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
