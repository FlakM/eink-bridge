"""Every mark in a brief must point at a real line before the reader can tap it."""
import importlib.util
import pathlib
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "check_marks", ROOT / "harness/skills/eink-pr/scripts/check_marks.py")
check_marks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_marks)

BRIEF = """## Marks

- A → src/a.rs:3
- B → src/a.rs:11
- C → missing.rs:1
"""


class MarkTests(unittest.TestCase):
    def test_a_mark_past_the_end_and_a_missing_file_are_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "src").mkdir()
            (root / "src" / "a.rs").write_text("\n".join(f"line {i}" for i in range(1, 11)))
            results = check_marks.check(BRIEF, [directory])
            by_letter = {letter: reason for letter, _, _, reason in results}
            self.assertIsNone(by_letter["A"])
            self.assertIn("has 10 lines", by_letter["B"])
            self.assertEqual(by_letter["C"], "no checkout contains the file")

    def test_a_batch_may_resolve_a_mark_in_any_of_its_worktrees(self):
        with tempfile.TemporaryDirectory() as directory:
            first = pathlib.Path(directory) / "one"
            second = pathlib.Path(directory) / "two"
            first.mkdir(); second.mkdir()
            (second / "other.rs").write_text("a\nb\nc\n")
            brief = "- A → other.rs:3\n"
            results = check_marks.check(brief, [str(first), str(second)])
            self.assertIsNone(results[0][3], "the second checkout supplies the line")

    def test_a_brief_without_marks_is_not_silently_accepted(self):
        self.assertEqual(check_marks.marks("no marks here"), [])


if __name__ == "__main__":
    unittest.main()
