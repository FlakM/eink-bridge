"""Unit tests for the deterministic PR-review core (filter / render / lint).

These pin the rules that keep the tablet readable: only relevant feedback survives,
and a brief cannot be delivered if it is verbose, hedged, or non-actionable.
"""
import importlib.util
from pathlib import Path
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1] / "skills/eink-pr/scripts"
spec = importlib.util.spec_from_file_location("eink_review", SCRIPTS / "review.py")
review = importlib.util.module_from_spec(spec)
spec.loader.exec_module(review)


def finding(**overrides):
    base = {
        "lens": "correctness",
        "severity": "major",
        "confidence": 0.8,
        "file": "server/src/app.rs",
        "line": 10,
        "claim": "expiry never clears notifiers",
        "evidence": "the loop calls expire_stale only",
        "action": "call notify_and_cleanup for each expired id",
        "tags": [],
    }
    base.update(overrides)
    return base


class FilterTests(unittest.TestCase):
    def test_drops_structurally_invalid_findings(self):
        kept, dropped = review.filter_findings([
            finding(file=""),
            finding(line=0),
            finding(claim=""),
            finding(action=""),
            finding(severity="blocker"),
            finding(confidence=3.0),
            finding(evidence=""),
        ])
        self.assertEqual(kept, [])
        self.assertEqual(len(dropped), 7)
        reasons = " ".join(reason for _, reason in dropped)
        self.assertIn("invalid", reasons)
        self.assertIn("no evidence", reasons)

    def test_drops_nits_but_keeps_one_when_nothing_else_survives(self):
        nit = finding(severity="nit", claim="rename this helper")
        kept, _ = review.filter_findings([nit])
        self.assertEqual(len(kept), 1, "an empty page is worse than one nit")

        kept, dropped = review.filter_findings([nit, finding()])
        self.assertEqual(len(kept), 1)
        self.assertEqual(kept[0]["severity"], "major")
        self.assertTrue(any("nit" in r for _, r in dropped))

    def test_two_nits_keep_only_the_best_one(self):
        kept, dropped = review.filter_findings([
            finding(severity="nit", confidence=0.5, claim="rename this"),
            finding(severity="nit", confidence=0.9, claim="reorder fields"),
        ])
        self.assertEqual(len(kept), 1)
        self.assertEqual(kept[0]["claim"], "reorder fields")
        self.assertTrue(any("nit" in r for _, r in dropped))

    def test_allow_nits_lets_them_through(self):
        kept, _ = review.filter_findings([finding(severity="nit")], allow_nits=True)
        self.assertEqual(len(kept), 1)

    def test_low_confidence_dropped_unless_critical(self):
        kept, dropped = review.filter_findings([
            finding(confidence=0.2, claim="mild worry"),
            finding(severity="critical", confidence=0.3, claim="deletes user data"),
        ])
        self.assertEqual([f["severity"] for f in kept], ["critical"])
        self.assertTrue(any("low confidence" in r for _, r in dropped))

    def test_stale_anchor_is_dropped_against_a_checkout(self):
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / "server/src/app.rs"
            target.parent.mkdir(parents=True)
            target.write_text("one\ntwo\nthree\n")
            kept, dropped = review.filter_findings(
                [finding(line=3), finding(line=999, claim="points past the file")], checkout=root)
            self.assertEqual([f["line"] for f in kept], [3])
            self.assertTrue(any("stale anchor" in r for _, r in dropped))

    def test_deduplicates_near_identical_findings_keeping_the_stronger(self):
        weaker = finding(severity="minor", confidence=0.6, line=11)
        stronger = finding(severity="critical", confidence=0.9, line=10)
        kept, dropped = review.filter_findings([weaker, stronger])
        self.assertEqual(len(kept), 1)
        self.assertEqual(kept[0]["severity"], "critical")
        self.assertTrue(any("duplicate" in r for _, r in dropped))

    def test_distinct_findings_in_the_same_file_are_not_deduped(self):
        kept, _ = review.filter_findings([
            finding(line=10, claim="expiry never clears notifiers"),
            finding(line=40, claim="pagination returns every session at once"),
        ])
        self.assertEqual(len(kept), 2)

    def test_ranks_by_severity_then_confidence_then_blast_radius(self):
        kept, _ = review.filter_findings([
            finding(severity="minor", file="a.rs", claim="cosmetic thing", tags=["public-api"]),
            finding(severity="major", confidence=0.6, file="b.rs", claim="slow under load"),
            finding(severity="major", confidence=0.95, file="c.rs", claim="lock held across await"),
            finding(severity="critical", confidence=0.51, file="d.rs", claim="drops the response body"),
        ])
        self.assertEqual([f["claim"] for f in kept], [
            "drops the response body",
            "lock held across await",
            "slow under load",
            "cosmetic thing",
        ])

    def test_blast_radius_breaks_a_tie_within_a_severity(self):
        kept, _ = review.filter_findings([
            finding(file="a.rs", claim="local detail"),
            finding(file="b.rs", claim="changes the wire format", tags=["public-api"]),
        ], per_file=1)
        self.assertEqual(kept[0]["claim"], "changes the wire format")

    def test_caps_per_file_then_total(self):
        findings = [
            finding(file=f"src/file_{index}.rs", line=1, claim=f"problem number {index}")
            for index in range(10)
        ]
        kept, dropped = review.filter_findings(findings, limit=4, per_file=1)
        self.assertEqual(len(kept), 4)
        self.assertTrue(any("total cap" in r for _, r in dropped))

        same_file = [finding(line=line, claim=f"problem number {line}") for line in range(1, 11)]
        kept, dropped = review.filter_findings(same_file, limit=6, per_file=2)
        self.assertEqual(sum(1 for f in kept if f["file"] == "server/src/app.rs"), 2)
        self.assertTrue(any("per-file cap" in r for _, r in dropped))

    def test_findings_without_evidence_are_not_worth_a_high_severity(self):
        kept, dropped = review.filter_findings([finding(severity="major", evidence="")])
        self.assertEqual(kept, [])
        self.assertTrue(any("no evidence" in r for _, r in dropped))

    def test_minor_findings_survive_without_evidence(self):
        kept, _ = review.filter_findings([finding(severity="minor", evidence="")])
        self.assertEqual(len(kept), 1)


class RenderTests(unittest.TestCase):
    def test_assigns_letters_and_builds_the_marks_legend(self):
        pr = {"number": 42, "title": "Quiet notifications", "summary": ["Documents are quieter now."]}
        markdown, marks = review.render_brief(pr, [finding(line=10), finding(line=20, claim="second thing")],
                                              links={"server/src/app.rs:10": "https://example/perm"})
        self.assertEqual(marks, {"A": "server/src/app.rs:10", "B": "server/src/app.rs:20"})
        self.assertIn("# PR 42: Quiet notifications", markdown)
        self.assertIn("- **A** — ", markdown)
        self.assertNotIn("http", markdown, "the legend must stay short, not carry permalinks")
        self.assertIn("- B → server/src/app.rs:20", markdown)

    def test_no_findings_still_produces_the_opening_without_a_marks_section(self):
        markdown, marks = review.render_brief({"number": 7, "title": "Docs only"}, [])
        self.assertEqual(marks, {})
        self.assertIn("# PR 7: Docs only", markdown)
        self.assertIn("**What changed**", markdown)
        self.assertNotIn("## Marks", markdown)
        self.assertNotIn("## Important pieces", markdown)

    def test_local_review_uses_a_prefix_instead_of_a_number(self):
        markdown, _ = review.render_brief({"title": "Session expiry fix"}, [finding()], title_prefix="Changes")
        self.assertIn("# Changes: Session expiry fix", markdown)


OPENING = (
    "**What changed** — Expiry clears the notifier map.\n"
    "**Why** — Notifier entries leaked for every expired session.\n"
    "**Wider system** — The expiry loop also drives metrics.\n\n"
    "## Checks\n\n- All green: cargo test.\n"
)


class LintTests(unittest.TestCase):
    def brief(self, body, opening=True):
        head = "# PR 1: thing\n\n" + (OPENING if opening else "")
        return f"{head}\n{body}\n"

    def test_context_opening_is_mandatory(self):
        problems = review.lint_brief(self.brief("- **A** — Check the guard.", opening=False))
        rules = {rule for _, rule, _ in problems}
        self.assertIn("missing-opening", rules)
        self.assertIn("missing-checks", rules)

    def test_a_long_opening_line_is_rejected(self):
        problems = review.lint_brief(self.brief(
            "**What changed** — " + " ".join(["clause"] * 40) + ".\n"
            "**Why** — Reason.\n**Wider system** — Callers.\n\n## Checks\n\n- All green.\n"))
        self.assertIn("long-opening", {rule for _, rule, _ in problems})

    def test_checks_report_a_failure_rather_than_claiming_green(self):
        markdown, _ = review.render_brief(
            {"number": 1, "title": "t", "what": "w", "why": "y", "wider": "x",
             "checks": [{"name": "cargo test", "status": "pass"},
                        {"name": "CI", "status": "fail", "detail": "proto lint"}]}, [])
        self.assertIn("## Checks", markdown)
        self.assertIn("**CI** fail", markdown)
        self.assertNotIn("All green", markdown)

    def test_checks_say_green_in_one_line_when_everything_passed(self):
        markdown, _ = review.render_brief(
            {"number": 1, "title": "t", "what": "w", "why": "y", "wider": "x",
             "checks": [{"name": "cargo test", "status": "pass"}, {"name": "clippy", "status": "pass"}]}, [])
        self.assertIn("- All green: cargo test, clippy.", markdown)

    def test_an_unstated_why_is_admitted_not_invented(self):
        markdown, _ = review.render_brief({"number": 1, "title": "t"}, [])
        self.assertIn("**Why** — Not stated in the description", markdown)

    def test_a_tight_actionable_brief_passes(self):
        problems = review.lint_brief(self.brief(
            "## Important pieces\n\n"
            "- **A** — Expiry skips notifier cleanup. Clear it when a session expires.\n"
            "- **B** — Version check trusts the client. Verify the version before accept.\n"))
        self.assertEqual(problems, [], problems)

    def test_a_long_url_is_rejected(self):
        problems = review.lint_brief(self.brief(
            "- **A** — See https://github.com/acme/repo/blob/deadbeef1234567890/server/src/app.rs#L170-L176 for the context. Check it.\n"))
        self.assertIn("long-link", {rule for _, rule, _ in problems})

    def test_a_short_reference_is_fine(self):
        problems = review.lint_brief(self.brief(
            "- **A** — See server/src/app.rs:170. Check the guard.\n"))
        self.assertEqual(problems, [], problems)

    def test_common_words_are_not_mistaken_for_filler(self):
        """"every" must not trip the "very" rule, "adjust" must not trip "just"."""
        problems = review.lint_brief(self.brief(
            "## Important pieces\n\n"
            "- **A** — Every session adjusts the map. Verify the guard.\n"))
        self.assertEqual(problems, [], problems)

    def test_accepts_natural_imperative_phrasing(self):
        """A real agent wrote this shape; the gate must not reject good reviews."""
        problems = review.lint_brief(self.brief(
            "## Important pieces\n\n"
            "- **A** New pagination field is required, which breaks old clients. Make it optional or version the message.\n"
            "- **B** Last-page path has no test. Cover the empty page and the last page.\n"))
        self.assertEqual(problems, [], problems)

    def test_necessity_phrasing_counts_as_an_action(self):
        problems = review.lint_brief(self.brief(
            "- **A** The cursor is never validated and the server must reject it.\n"))
        self.assertEqual(problems, [], problems)

    def test_filler_and_hedges_are_rejected(self):
        problems = review.lint_brief(self.brief(
            "- **A** — This basically works. Maybe check the lock.\n"))
        rules = {rule for _, rule, _ in problems}
        self.assertIn("filler", rules)
        self.assertIn("hedge", rules)

    def test_long_bullet_is_rejected(self):
        long_words = " ".join(["word"] * 40)
        problems = review.lint_brief(self.brief(f"- **A** — {long_words} check it.\n"))
        self.assertIn("long-bullet", {rule for _, rule, _ in problems})

    def test_long_sentence_is_rejected(self):
        long_sentence = " ".join(["clause"] * 30) + " verify it."
        problems = review.lint_brief(self.brief(f"## Impact\n\n- {long_sentence}\n"))
        self.assertIn("long-sentence", {rule for _, rule, _ in problems})

    def test_trailing_summary_heading_is_rejected(self):
        problems = review.lint_brief(self.brief("## Summary\n\n- **A** — Check the guard.\n"))
        self.assertIn("summary-stack", {rule for _, rule, _ in problems})

    def test_diff_dumps_are_rejected(self):
        problems = review.lint_brief(self.brief(
            "```diff\n- old\n+ new\n```\n\n@@ -1,3 +1,4 @@\n"))
        self.assertIn("diff-dump", {rule for _, rule, _ in problems})

    def test_important_piece_without_an_action_is_rejected(self):
        problems = review.lint_brief(self.brief(
            "- **A** — The session handler diverges from the older behaviour.\n"))
        self.assertIn("not-actionable", {rule for _, rule, _ in problems})

    def test_overall_word_budget_is_enforced(self):
        bullets = "\n".join(
            f"- **{review.MARK_LETTERS[i]}** — Check clause {i} and confirm the guard.\n"
            for i in range(20))
        problems = review.lint_brief(self.brief("## Important pieces\n\n" + bullets), max_words=100)
        self.assertIn("too-long", {rule for _, rule, _ in problems})


class ReferenceTests(unittest.TestCase):
    def test_resolves_a_note_that_names_a_mark_letter(self):
        marks = {"A": "server/src/app.rs:10", "B": "proto/api.proto:9"}
        self.assertEqual(review.resolve_reference(marks, "B looks wrong"), ("B", "proto/api.proto:9"))
        self.assertEqual(review.resolve_reference(marks, "**A** — why is this here"), ("A", "server/src/app.rs:10"))

    def test_ignores_capitals_that_are_not_marks(self):
        marks = {"A": "server/src/app.rs:10"}
        self.assertEqual(review.resolve_reference(marks, "HTTP returns 500 OK"), (None, None))
        self.assertEqual(review.resolve_reference(marks, ""), (None, None))
        self.assertEqual(review.resolve_reference({}, "A"), (None, None))

    def test_splits_a_spot_into_path_and_line(self):
        self.assertEqual(review.split_spot("a/b.py:12"), ("a/b.py", 12))


if __name__ == "__main__":
    unittest.main()
