"""Tests for the comment collector: placement, payload, and the record on disk."""
import importlib.util
import json
import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "pr_comments", ROOT / "harness/skills/eink-pr/scripts/pr_comments.py")
comments = importlib.util.module_from_spec(spec)
spec.loader.exec_module(comments)

PATCH = """@@ -10,3 +10,4 @@
 context ten
-old eleven
+new eleven
+new twelve
 context twelve
"""


class ReferenceTests(unittest.TestCase):
    def test_a_number_and_a_url_both_name_the_review(self):
        self.assertEqual(comments.pr_ref("485"), (485, None))
        self.assertEqual(
            comments.pr_ref("https://github.com/coralogix/security-token-service/pull/485"),
            (485, "coralogix/security-token-service"),
        )

    def test_a_non_pull_request_url_is_refused(self):
        with self.assertRaises(SystemExit):
            comments.pr_ref("https://github.com/coralogix/security-token-service/issues/4")


class PlacementTests(unittest.TestCase):
    def test_every_line_in_a_hunk_is_commentable_on_its_side(self):
        old, new = comments.hunk_lines(PATCH)
        self.assertEqual(old, {10, 11, 12})
        self.assertEqual(new, {10, 11, 12, 13})

    def test_lines_outside_the_diff_and_unknown_files_go_to_the_body(self):
        files = [{"filename": "src/lib.rs", "patch": PATCH}]
        entries = [
            {"path": "src/lib.rs", "line": 11, "side": "RIGHT", "body": "right"},
            {"path": "src/lib.rs", "line": 11, "side": "LEFT", "body": "left"},
            {"path": "src/lib.rs", "line": 99, "side": "RIGHT", "body": "outside"},
            {"path": "src/gone.rs", "line": 3, "side": "LEFT", "body": "unknown"},
        ]
        inline, body = comments.split_comments(entries, files)
        self.assertEqual([c["body"] for c in inline], ["right", "left"])
        self.assertEqual(inline[0]["side"], "RIGHT")
        self.assertEqual(inline[1]["side"], "LEFT")
        self.assertEqual(len(body), 2)
        self.assertTrue(any("src/lib.rs:99" in line for line in body))
        self.assertTrue(any("src/gone.rs:3" in line for line in body))

    def test_a_pending_review_has_no_event_and_a_submitted_one_does(self):
        pending = comments.build_payload([], ["- `a:1` — body"], summary="")
        self.assertNotIn("event", pending)
        self.assertIn("outside the diff", pending["body"])
        submitted = comments.build_payload([], [], summary="notes", submit=True)
        self.assertEqual(submitted["event"], "COMMENT")
        self.assertEqual(submitted["body"], "notes")


class RecordingTests(unittest.TestCase):
    def test_comments_are_appended_to_one_jsonl_per_pull_request(self):
        with tempfile.TemporaryDirectory() as directory:
            state = pathlib.Path(directory)
            comments.add_comment(485, "src/lib.rs", 11, "RIGHT", "first", state=state)
            comments.add_comment(485, "src/lib.rs", 12, "LEFT", "second",
                                 source="eink", state=state)
            loaded = comments.load_comments(485, state=state)
            self.assertEqual([c["body"] for c in loaded], ["first", "second"])
            self.assertEqual(loaded[1]["source"], "eink")
            self.assertEqual(loaded[0]["side"], "RIGHT")
            self.assertTrue((state / "comments" / "pr-485.jsonl").exists())
            self.assertEqual(comments.load_comments(486, state=state), [])


if __name__ == "__main__":
    unittest.main()
