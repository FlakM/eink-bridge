"""Unit tests for the deterministic PR-review core (filter / render / lint).

These pin the rules that keep the tablet readable: only relevant feedback survives,
and a brief cannot be delivered if it is verbose, hedged, or non-actionable.
"""
from pathlib import Path
import tempfile
import unittest

from eink_harness import review


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
        "refutation": "no comment or test explains the skipped cleanup",
        "verified": "confirmed",
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

    def test_a_finding_must_say_how_its_reviewer_tried_to_refute_it(self):
        kept, dropped = review.filter_findings([finding(refutation="")])
        self.assertEqual(kept, [])
        self.assertIn("missing refutation", dropped[0][1])

    def test_an_unverified_finding_is_dropped_not_trusted(self):
        unverified = finding()
        del unverified["verified"]
        kept, dropped = review.filter_findings([unverified])
        self.assertEqual(kept, [])
        self.assertIn("unverified", dropped[0][1])

    def test_a_refuted_finding_is_dropped(self):
        kept, dropped = review.filter_findings([finding(verified="refuted")])
        self.assertEqual(kept, [])
        self.assertIn("refuted", dropped[0][1])

    def test_a_plausible_finding_survives(self):
        kept, _ = review.filter_findings([finding(verified="plausible")])
        self.assertEqual(len(kept), 1)


DIFF = """\
diff --git a/server/src/app.rs b/server/src/app.rs
--- a/server/src/app.rs
+++ b/server/src/app.rs
@@ -8,4 +8,6 @@ fn expire_loop() {
@@ -40 +42 @@ fn other() {
diff --git a/server/src/gone.rs b/server/src/gone.rs
--- a/server/src/gone.rs
+++ /dev/null
@@ -1,3 +0,0 @@
"""


class DiffAnchorTests(unittest.TestCase):
    def test_parses_new_side_ranges_and_skips_deleted_files(self):
        self.assertEqual(review.parse_diff_hunks(DIFF), {"server/src/app.rs": [(8, 13), (42, 42)]})

    def test_a_finding_inside_a_hunk_is_kept(self):
        hunks = review.parse_diff_hunks(DIFF)
        kept, _ = review.filter_findings([finding(line=10), finding(line=42, claim="other")],
                                         hunks=hunks)
        self.assertEqual(len(kept), 2)

    def test_a_finding_on_an_unchanged_file_is_dropped(self):
        kept, dropped = review.filter_findings(
            [finding(file="server/src/untouched.rs")], hunks=review.parse_diff_hunks(DIFF))
        self.assertEqual(kept, [])
        self.assertIn("not in the diff", dropped[0][1])

    def test_a_finding_outside_the_changed_hunks_is_dropped(self):
        kept, dropped = review.filter_findings([finding(line=30)],
                                               hunks=review.parse_diff_hunks(DIFF))
        self.assertEqual(kept, [])
        self.assertIn("outside the changed hunks", dropped[0][1])

    def test_a_broken_caller_may_sit_outside_the_diff(self):
        kept, _ = review.filter_findings(
            [finding(file="server/src/caller.rs", tags=["caller-broken"])],
            hunks=review.parse_diff_hunks(DIFF))
        self.assertEqual(len(kept), 1)

    def test_without_a_diff_nothing_is_dropped_for_its_anchor(self):
        kept, _ = review.filter_findings([finding(file="server/src/untouched.rs")])
        self.assertEqual(len(kept), 1)


class RenderTests(unittest.TestCase):
    def test_assigns_letters_and_builds_the_marks_legend(self):
        pr = {"number": 42, "title": "Quiet notifications", "summary": ["Documents are quieter now."]}
        markdown, marks = review.render_brief(pr, [finding(line=10), finding(line=20, claim="second thing")],
                                              links={"server/src/app.rs:10": "https://example/perm"})
        self.assertEqual(marks, {"A": "server/src/app.rs:10", "B": "server/src/app.rs:20"})
        self.assertIn("# PR 42: Quiet notifications", markdown)
        self.assertIn("- **A** `issue (blocking)` ", markdown)
        self.assertNotIn("http", markdown, "the page must stay short, not carry permalinks")
        # The place sits on the finding, not in a legend the reader has to scroll to.
        self.assertIn("server/src/app.rs:10", markdown)
        self.assertNotIn("## Marks", markdown)

    def test_the_verdict_opens_the_page_and_agrees_with_severity(self):
        blocking = review.render_brief({"title": "t"}, [finding(severity="major")])[0]
        self.assertIn("**Ready?** With fixes — ", blocking)
        critical = review.render_brief({"title": "t"}, [finding(severity="critical")])[0]
        self.assertIn("**Ready?** No — ", critical)
        clean = review.render_brief({"title": "t"}, [])[0]
        self.assertIn("**Ready?** Yes — ", clean)

    def test_a_stated_verdict_that_agrees_lends_its_reason(self):
        markdown, _ = review.render_brief(
            {"title": "t", "verdict": "with fixes", "verdict_reason": "the leak is real"},
            [finding(severity="major")])
        self.assertIn("**Ready?** With fixes — The leak is real.", markdown)

    def test_a_stated_verdict_stricter_than_the_confirmed_findings_is_overruled(self):
        markdown, _ = review.render_brief(
            {"title": "t", "verdict": "with fixes", "verdict_reason": "it is still spaghetti"},
            [finding(severity="minor")])
        self.assertIn("**Ready?** Yes — Nothing blocking", markdown)
        self.assertNotIn("spaghetti", markdown)

    def test_only_a_confirmed_finding_blocks(self):
        plausible = review.render_brief({"title": "t"},
                                        [finding(severity="major", verified="plausible")])[0]
        self.assertIn("**Ready?** Yes — ", plausible)

    def test_the_budget_line_states_what_needs_judgement(self):
        markdown, _ = review.render_brief(
            {"title": "t", "files": 12, "additions": 340, "deletions": 96,
             "judgement_lines": 310, "mechanical": 30}, [])
        self.assertIn("12 files · +340 −96 · 310 lines to judge, 30 mechanical · 62 min", markdown)

    def test_the_question_is_the_point_and_an_absent_one_is_flagged(self):
        markdown, _ = review.render_brief({"title": "t", "question": "split the migration out?"}, [])
        self.assertIn("**The question** — Split the migration out?", markdown)
        problems = review.lint_brief(markdown)
        self.assertNotIn("missing-question", {rule for _, rule, _ in problems})

    def test_no_findings_still_produces_the_opening_without_a_findings_section(self):
        markdown, marks = review.render_brief({"number": 7, "title": "Docs only"}, [])
        self.assertEqual(marks, {})
        self.assertIn("# PR 7: Docs only", markdown)
        self.assertIn("**What changed**", markdown)
        self.assertNotIn("## Read these first", markdown)
        self.assertNotIn("## Marks", markdown)

    def test_local_review_uses_a_prefix_instead_of_a_number(self):
        markdown, _ = review.render_brief({"title": "Session expiry fix"}, [finding()], title_prefix="Changes")
        self.assertIn("# Changes: Session expiry fix", markdown)


PAGE_HEAD = (
    "# PR 1: thing\n\n"
    "**Ready?** With fixes — the guard is the problem.\n\n"
    "**What changed** — Expiry clears the notifier map.\n\n"
    "**Why** — Notifier entries leaked for every expired session.\n\n"
    "**Wider system** — The expiry loop also drives metrics.\n\n"
    "**The question** — Should the socket map clear on the same timer?\n\n"
)

CHECKS = "## What I checked\n\n- [x] All green: cargo test.\n"

SET_ASIDE = "## Set aside\n\n- The rename in `session.rs` — mechanical, no behaviour.\n"


class LintTests(unittest.TestCase):
    def brief(self, body, head=True, checks=True, set_aside=True):
        parts = [PAGE_HEAD if head else "# PR 1: thing\n\n", body + "\n"]
        if checks:
            parts.append(CHECKS)
        if set_aside:
            parts.append(SET_ASIDE)
        return "\n".join(parts)

    def rules(self, body, **kwargs):
        return {rule for _, rule, _ in review.lint_brief(self.brief(body, **kwargs))}

    def test_context_opening_is_mandatory(self):
        rules = self.rules("- **A** `issue` Check the guard. app.rs:1\n",
                           head=False, checks=False, set_aside=False)
        self.assertIn("missing-opening", rules)
        self.assertIn("missing-checks", rules)

    def test_the_answer_comes_before_the_evidence(self):
        without = self.brief("", head=False)
        without = without.replace("**Ready?** With fixes — the guard is the problem.\n\n", "")
        self.assertIn("missing-verdict", {rule for _, rule, _ in review.lint_brief(without)})

    def test_a_finding_must_say_what_kind_of_finding_it_is(self):
        self.assertIn("unlabelled", self.rules("- **A** Expiry skips cleanup. Clear it. app.rs:1\n"))
        self.assertIn("unknown-label",
                      self.rules("- **A** `blocker` Expiry skips cleanup. Clear it. app.rs:1\n"))
        self.assertEqual(self.rules("- **A** `issue (blocking)` Expiry skips cleanup. Clear it. app.rs:1\n"), set())

    def test_findings_without_a_set_aside_section_are_rejected(self):
        self.assertIn("missing-set-aside",
                      self.rules("## Read these first\n\n- **A** `issue` Check the guard. app.rs:1\n",
                                 set_aside=False))

    def test_answers_on_consecutive_lines_are_flagged(self):
        """Three lines of markdown join into one paragraph, and one paragraph is one anchor."""
        merged = (
            "# PR 1: thing\n\n"
            "**What changed** — Expiry clears the map.\n"
            "**Why** — It leaked.\n\n"
            "## What I checked\n\n- [x] All green.\n"
        )
        self.assertIn("merged-opening", {rule for _, rule, _ in review.lint_brief(merged)})

    def test_a_long_opening_line_is_rejected(self):
        problems = review.lint_brief(self.brief(
            "**What changed** — " + " ".join(["clause"] * 40) + ".\n"
            "**Why** — Reason.\n**Wider system** — Callers.\n"))
        self.assertIn("long-opening", {rule for _, rule, _ in problems})

    def test_checks_report_a_failure_rather_than_claiming_green(self):
        markdown, _ = review.render_brief(
            {"number": 1, "title": "t", "what": "w", "why": "y", "wider": "x",
             "checks": [{"name": "cargo test", "status": "pass"},
                        {"name": "CI", "status": "fail", "detail": "proto lint"}]}, [])
        self.assertIn("## What I checked", markdown)
        self.assertIn("**CI** fail", markdown)
        self.assertNotIn("All green", markdown)

    def test_checks_say_green_in_one_line_when_everything_passed(self):
        markdown, _ = review.render_brief(
            {"number": 1, "title": "t", "what": "w", "why": "y", "wider": "x",
             "checks": [{"name": "cargo test", "status": "pass"}, {"name": "clippy", "status": "pass"}]}, [])
        self.assertIn("- [x] All green: cargo test, clippy.", markdown)

    def test_what_was_not_checked_is_a_line_of_its_own(self):
        markdown, _ = review.render_brief(
            {"number": 1, "title": "t",
             "checked": ["Every caller of `expire()` (4)."],
             "not_checked": ["the Lua plugin path"]}, [])
        self.assertIn("- [x] Every caller of `expire()` (4).", markdown)
        self.assertIn("- [ ] Not checked: the Lua plugin path.", markdown)

    def test_set_aside_reaches_the_page(self):
        markdown, _ = review.render_brief(
            {"number": 1, "title": "t",
             "set_aside": [{"what": "The rename in session.rs", "why": "mechanical"}]}, [])
        self.assertIn("## Set aside", markdown)
        self.assertIn("- The rename in session.rs — mechanical.", markdown)

    def test_the_change_surface_is_a_table_of_surfaces(self):
        markdown, _ = review.render_brief(
            {"number": 1, "title": "t",
             "surface": [{"surface": "HTTP API", "effect": "none"},
                         {"surface": "`session.json`", "effect": "one additive field"}]}, [])
        self.assertIn("## Change surface", markdown)
        self.assertIn("| surface | effect |", markdown)
        self.assertIn("| `session.json` | one additive field |", markdown)

    def test_an_unstated_why_is_admitted_not_invented(self):
        markdown, _ = review.render_brief({"number": 1, "title": "t"}, [])
        self.assertIn("**Why** — Not stated in the description", markdown)

    def test_a_tight_actionable_brief_passes(self):
        problems = review.lint_brief(self.brief(
            "## Read these first\n\n"
            "- **A** `issue (blocking)` Expiry skips notifier cleanup. Clear it when a session expires. app.rs:170\n"
            "- **B** `suggestion` Version check trusts the client. Verify the version before accept. session.rs:118\n"))
        self.assertEqual(problems, [], problems)

    def test_a_long_url_is_rejected(self):
        self.assertIn("long-link", self.rules(
            "- **A** `issue` See https://github.com/acme/repo/blob/deadbeef1234567890/server/src/app.rs#L170-L176 for the context. Check it. app.rs:1\n"))

    def test_a_short_reference_is_fine(self):
        problems = review.lint_brief(self.brief(
            "- **A** `issue` See server/src/app.rs:170. Check the guard.\n"))
        self.assertEqual(problems, [], problems)

    def test_common_words_are_not_mistaken_for_filler(self):
        """"every" must not trip the "very" rule, "adjust" must not trip "just"."""
        problems = review.lint_brief(self.brief(
            "## Read these first\n\n"
            "- **A** `issue` Every session adjusts the map. Verify the guard. app.rs:1\n"))
        self.assertEqual(problems, [], problems)

    def test_accepts_natural_imperative_phrasing(self):
        """A real agent wrote this shape; the gate must not reject good reviews."""
        problems = review.lint_brief(self.brief(
            "## Read these first\n\n"
            "- **A** `issue (blocking)` New pagination field is required, which breaks old clients. Make it optional or version the message. api.proto:9\n"
            "- **B** `suggestion` Last-page path has no test. Cover the empty page and the last page. pager.rs:44\n"))
        self.assertEqual(problems, [], problems)

    def test_necessity_phrasing_counts_as_an_action(self):
        problems = review.lint_brief(self.brief(
            "- **A** `issue` The cursor is never validated and the server must reject it. cache.rs:12\n"))
        self.assertEqual(problems, [], problems)

    def test_filler_and_hedges_are_rejected(self):
        rules = self.rules("- **A** `issue` This basically works. Maybe check the lock. app.rs:1\n")
        self.assertIn("filler", rules)
        self.assertIn("hedge", rules)

    def test_long_bullet_is_rejected(self):
        long_words = " ".join(["word"] * 40)
        self.assertIn("long-bullet", self.rules(f"- **A** `issue` {long_words} check it. app.rs:1\n"))

    def test_long_sentence_is_rejected(self):
        long_sentence = " ".join(["clause"] * 30) + " verify it."
        self.assertIn("long-sentence", self.rules(f"## Impact\n\n- {long_sentence}\n"))

    def test_trailing_summary_heading_is_rejected(self):
        self.assertIn("summary-stack", self.rules("## Summary\n\n- **A** `issue` Check the guard. app.rs:1\n"))

    def test_diff_dumps_are_rejected(self):
        self.assertIn("diff-dump", self.rules(
            "```diff\n- old\n+ new\n```\n\n@@ -1,3 +1,4 @@\n"))

    def test_finding_without_an_action_is_rejected(self):
        self.assertIn("not-actionable", self.rules(
            "- **A** `issue` The session handler diverges from the older behaviour. session.rs:9\n"))

    def test_overall_word_budget_is_enforced(self):
        bullets = "\n".join(
            f"- **{review.MARK_LETTERS[i]}** `issue` Check clause {i} and confirm the guard. app.rs:{i}\n"
            for i in range(20))
        problems = review.lint_brief(self.brief("## Read these first\n\n" + bullets), max_words=100)
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


class ShapeTests(unittest.TestCase):
    """The written recipe and the renderer must describe the same page.

    `references/review-document.md` is what a human (or an agent) reads to learn the
    shape; `render_brief` is what actually reaches the tablet. Two sources of truth drift,
    so the skeleton in the document is parsed and held against a fully populated render:
    rename a section in one place and this fails.
    """

    REFERENCE = (Path(__file__).resolve().parents[1]
                 / "skills/eink/references/review-document.md")

    def skeleton_headings(self):
        text = self.REFERENCE.read_text()
        fence = text.split("```markdown", 1)[1].split("```", 1)[0]
        return [line.lstrip("# ").strip() for line in fence.splitlines()
                if line.startswith("## ")]

    def full_brief(self):
        pr = {
            "number": 1, "title": "t",
            "what": "w", "why": "y", "wider": "x",
            "question": "q?", "files": 2, "additions": 10, "deletions": 3,
            "judgement_lines": 9, "checked": ["read the callers"],
            "not_checked": ["the plugin"],
            "set_aside": [{"what": "a rename", "why": "mechanical"}],
            "surface": [{"surface": "HTTP", "effect": "none"}],
            "bottom_line": "The guard belongs one layer down; nothing else here is risky.",
        }
        return review.render_brief(pr, [finding()])[0]

    def test_every_section_in_the_recipe_is_rendered(self):
        brief = self.full_brief()
        headings = self.skeleton_headings()
        self.assertGreaterEqual(len(headings), 5, "the recipe lost its skeleton")
        for heading in headings:
            self.assertIn(f"## {heading}", brief, f"the recipe names '{heading}' but the page does not have it")

    def test_the_labels_in_the_recipe_are_the_labels_the_linter_accepts(self):
        text = self.REFERENCE.read_text()
        for label in review.LABELS:
            self.assertIn(label, text, f"label {label!r} is accepted but not documented")

    """A change to a contract is what the reviewer has to agree with.

    The implementation can be read later; a proto, schema or OpenAPI change is the thing
    that other teams build against, so the brief has to show it -- and a finding on a
    contract outranks the same finding on an implementation file.
    """

    BRIEF = """**What changed** - the session cookie gains a scope claim

**Why** - the gateway reads it to authorise the websocket upgrade

**Wider system** - identity-service mints it, the workspace API consumes it

## What I checked

- not run
"""

    def test_contract_files_are_recognised(self):
        changed = [
            "services/auth/src/main.rs",
            "proto/identity/v2/session.proto",
            "api/openapi.yaml",
            "events/session.avsc",
            "models/session.thrift",
            "README.md",
        ]
        self.assertEqual(
            review.interface_files(changed),
            ["proto/identity/v2/session.proto", "api/openapi.yaml",
             "events/session.avsc", "models/session.thrift"],
        )

    def test_implementation_only_change_is_not_asked_for_interfaces(self):
        self.assertEqual(review.interface_files(["src/app.rs", "src/lib.rs"]), [])
        rules = [rule for _line, rule, _message in review.lint_brief(self.BRIEF)]
        self.assertNotIn("missing-interfaces", rules)

    def test_a_contract_change_must_be_presented(self):
        problems = review.lint_brief(self.BRIEF, interfaces=["proto/session.proto"])
        rules = {rule: message for _line, rule, message in problems}
        self.assertIn("missing-interfaces", rules)
        self.assertIn("Change surface", rules["missing-interfaces"])

    def test_the_section_is_enough_to_satisfy_it(self):
        brief = self.BRIEF.replace("## What I checked", "## Interfaces\n\n- `Session.scope: string` is new; old clients ignore it.\n\n## What I checked")
        rules = [rule for _line, rule, _message in review.lint_brief(brief, interfaces=["proto/session.proto"])]
        self.assertNotIn("missing-interfaces", rules)

    def test_a_finding_on_a_contract_outranks_one_on_the_implementation(self):
        contract = {"file": "proto/session.proto", "line": 12, "severity": "medium",
                    "confidence": 0.9, "claim": "the scope field is required but defaults to empty"}
        implementation = dict(contract, file="services/auth/src/main.rs", line=88)
        findings = [implementation, contract]
        # The caller sorts descending (worst first, contract over implementation).
        ranked = sorted(findings, key=lambda f: review.rank_key(f, interfaces=["proto/session.proto"]),
                        reverse=True)
        self.assertEqual(ranked[0]["file"], "proto/session.proto",
                         "the contract has to come first in the brief")
        # Without the interface list the two are ranked on their own merits.
        same = sorted(findings, key=review.rank_key, reverse=True)
        self.assertEqual({f["file"] for f in same[:2]},
                         {"proto/session.proto", "services/auth/src/main.rs"})
