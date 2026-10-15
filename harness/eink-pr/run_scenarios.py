#!/usr/bin/env python3
"""Run the e-ink PR-review pipeline against repeatable scenarios.

Offline (default): deterministic. Builds a throwaway checkout from each scenario,
runs filter -> render -> lint, and asserts the outcome. No model, no network, so it
is fast enough to run on every change and to bisect a regression.

Live (--live): spawns the real agent CLI on each scenario's prompt and checks the
brief it produces passes the same lint gate. Slower and non-deterministic, so it is
opt-in and reported separately.

Usage:
  run_scenarios.py                     # offline scenarios
  run_scenarios.py --list
  run_scenarios.py --only noisy-large-pr
  run_scenarios.py --live              # agent-level run for scenarios with a prompt
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
SCENARIOS = HERE / "scenarios"

spec = importlib.util.spec_from_file_location("eink_review", SCRIPTS / "review.py")
review = importlib.util.module_from_spec(spec)
spec.loader.exec_module(review)


def load_scenarios():
    found = []
    for path in sorted(SCENARIOS.glob("*.json")):
        with open(path) as handle:
            scenario = json.load(handle)
        scenario["_path"] = path
        found.append(scenario)
    return found


def materialise_checkout(root, files):
    for relative, lines in (files or {}).items():
        target = Path(root) / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("".join(f"line {index}\n" for index in range(1, int(lines) + 1)))


class Result:
    def __init__(self, name):
        self.name = name
        self.checks = []

    def check(self, ok, label, detail=""):
        self.checks.append((bool(ok), label, detail))
        return bool(ok)

    @property
    def ok(self):
        return all(ok for ok, _, _ in self.checks)

    def report(self):
        status = "PASS" if self.ok else "FAIL"
        print(f"[{status}] {self.name}")
        for ok, label, detail in self.checks:
            if not ok:
                print(f"    x {label}: {detail}")


def run_pipeline_scenario(scenario, out_dir):
    result = Result(scenario["name"])
    with tempfile.TemporaryDirectory() as checkout:
        materialise_checkout(checkout, scenario.get("checkout"))
        kept, dropped = review.filter_findings(
            scenario["findings"], checkout=checkout, limit=scenario.get("limit", 6),
            per_file=scenario.get("per_file", 2), allow_nits=scenario.get("allow_nits", False))

        expected = scenario["expect"]
        kept_claims = [finding["claim"] for finding in kept]
        result.check(kept_claims == expected["kept_claims"], "kept findings",
                     f"got {kept_claims!r}, want {expected['kept_claims']!r}")

        reasons = " | ".join(reason for _, reason in dropped)
        for fragment in expected.get("dropped_reasons", []):
            result.check(fragment in reasons, f"drops {fragment!r}", f"drop reasons: {reasons}")

        markdown, marks = review.render_brief(
            scenario.get("pr", {}), kept, title_prefix=scenario.get("prefix", "PR"))
        (Path(out_dir) / f"{scenario['name']}.md").write_text(markdown)
        if "marks" in expected:
            result.check(marks == expected["marks"], "mark letters",
                         f"got {marks!r}, want {expected['marks']!r}")

        problems = review.lint_brief(markdown, max_words=scenario.get("max_words", 420))
        if expected.get("lint_ok", True):
            result.check(not problems, "brief lints clean", f"problems: {problems}")
        else:
            rules = {rule for _, rule, _ in problems}
            for rule in expected.get("lint_rules", []):
                result.check(rule in rules, f"lint flags {rule}", f"rules: {sorted(rules)}")
    return result


def run_lint_scenario(scenario, _out_dir):
    result = Result(scenario["name"])
    text = (HERE / scenario["brief"]).read_text()
    problems = review.lint_brief(text, max_words=scenario.get("max_words", 420))
    rules = {rule for _, rule, _ in problems}
    expected = scenario["expect"]
    result.check(rules == set(expected.get("lint_rules", [])), "lint rules",
                 f"got {sorted(rules)}, want {sorted(expected.get('lint_rules', []))}")
    return result


RUNNERS = {"pipeline": run_pipeline_scenario, "lint": run_lint_scenario}


def run_offline(selected, out_dir):
    results = []
    for scenario in selected:
        kind = scenario.get("kind", "pipeline")
        if kind == "live":
            # Agent-level scenarios have no deterministic mode.
            continue
        runner = RUNNERS[kind]
        results.append(runner(scenario, out_dir))
    return results


def run_live(selected, out_dir):
    """Spawn the agent CLI per scenario prompt and check the brief it returns."""
    cli = shutil.which("claude")
    results = []
    for scenario in selected:
        prompt = scenario.get("prompt")
        if not prompt:
            continue
        result = Result(f"{scenario['name']} (live)")
        if not cli:
            result.check(False, "agent CLI available", "claude not on PATH")
            results.append(result)
            continue
        output = Path(out_dir) / f"{scenario['name']}.live.md"
        try:
            completed = subprocess.run(
                [cli, "-p", prompt, "--output-format", "text"],
                capture_output=True, text=True, timeout=scenario.get("live_timeout", 900))
        except subprocess.TimeoutExpired:
            result.check(False, "agent finished", f"timed out after {scenario.get('live_timeout', 900)}s")
            results.append(result)
            continue
        output.write_text(completed.stdout)
        result.check(completed.returncode == 0, "agent exited 0", completed.stderr.strip()[-400:])
        expectations = scenario.get("expect", {})
        problems = review.lint_brief(completed.stdout, max_words=scenario.get("max_words", 420))
        if expectations.get("lint_ok", True):
            result.check(not problems, "live brief lints clean", f"problems: {problems}")
        else:
            rules = {rule for _, rule, _ in problems}
            for rule in expectations.get("lint_rules", []):
                result.check(rule in rules, f"lint flags {rule}", f"rules: {sorted(rules)}")
        for forbidden in expectations.get("forbidden", []):
            result.check(forbidden.lower() not in completed.stdout.lower(),
                         f"drops {forbidden!r}", "noise reached the brief")
        for required in expectations.get("required", []):
            result.check(required.lower() in completed.stdout.lower(),
                         f"keeps {required!r}", "expected content missing")
        results.append(result)
    return results


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--only", action="append", default=[], help="scenario name (repeatable)")
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--live", action="store_true", help="also run agent-level scenarios")
    parser.add_argument("--out", default=os.environ.get("EINK_PR_OUT", "/tmp/eink-pr-scenarios"))
    args = parser.parse_args(argv)

    scenarios = load_scenarios()
    if args.list:
        for scenario in scenarios:
            print(f"{scenario['name']} [{scenario.get('kind', 'pipeline')}] {scenario.get('description', '')}")
        return 0
    if args.only:
        scenarios = [s for s in scenarios if s["name"] in set(args.only)]
    os.makedirs(args.out, exist_ok=True)

    results = run_offline(scenarios, args.out)
    if args.live:
        results += run_live(scenarios, args.out)

    for result in results:
        result.report()
    failed = [r for r in results if not r.ok]
    print(f"\n{len(results) - len(failed)}/{len(results)} scenarios passed; artifacts in {args.out}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
