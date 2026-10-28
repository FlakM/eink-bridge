"""Contract tests for the /eink-pr Neovim global-mark helper.

Marks persist in shada, so the failure mode that matters is a *silent* miss being
read back as a stale mark from an earlier session. These tests assert the helper
verifies what it set and fails loudly otherwise.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / "harness/skills/eink-pr/scripts/nvim_marks.py"
NEED = ["nvim"]


@unittest.skipUnless(all(shutil.which(tool) for tool in NEED), "requires nvim")
class MarksTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.root = Path(self.dir.name)
        self.file = self.root / "code.txt"
        self.file.write_text("".join(f"line {i}\n" for i in range(1, 11)))
        self.socket = str(self.root / "nvim.sock")
        self.nvim = subprocess.Popen(
            ["nvim", "--headless", "--listen", self.socket, str(self.file)],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(100):
            if os.path.exists(self.socket):
                break
            time.sleep(0.05)
        else:
            self.nvim.kill()
            self.fail("nvim did not open its listen socket")

    def tearDown(self):
        self.nvim.terminate()
        self.nvim.wait(timeout=5)
        if self.nvim.poll() is None:
            self.nvim.kill()
        self.dir.cleanup()

    def run_helper(self, *args):
        return subprocess.run(["python3", str(HELPER), "--socket", self.socket, *args],
                              capture_output=True, text=True, timeout=30)

    def nvim_expr(self, expression):
        return subprocess.run(["nvim", "--server", self.socket, "--remote-expr", expression],
                              capture_output=True, text=True, timeout=30).stdout.strip()

    def extmark_details(self, namespace, accessor):
        lua = (
            "vim.inspect(vim.tbl_map(function(m) return " + accessor + " end, "
            'vim.api.nvim_buf_get_extmarks(vim.fn.bufnr("' + str(self.file) + '"), '
            'vim.api.nvim_create_namespace("' + namespace + '"), 0, -1, {details=true})))'
        )
        return self.nvim_expr("luaeval('" + lua + "')")

    def test_sets_global_marks_and_reports_positions(self):
        result = self.run_helper("--marks", f"A:{self.file}:4,B:{self.file}:7", "--verify", "A,B")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("set 2 mark(s)", result.stdout)
        self.assertIn(f"A={self.file}:4", result.stdout)
        self.assertIn(f"B={self.file}:7", result.stdout)

    def test_mark_beyond_end_of_file_is_a_loud_failure(self):
        result = self.run_helper("--marks", f"C:{self.file}:999")
        self.assertNotEqual(result.returncode, 0)
        # Rejected up front (range validation) or reported by verification; either
        # way it must fail loudly rather than silently reading back a stale mark.
        self.assertRegex(result.stderr, r"cannot mark line 999|marks not set")

    def test_missing_socket_is_a_loud_failure(self):
        result = subprocess.run(["python3", str(HELPER), "--socket", str(self.root / "gone.sock"),
                                 "--marks", f"D:{self.file}:1"], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("socket not found", result.stderr)

    def test_invalid_letter_is_rejected(self):
        result = self.run_helper("--marks", f"AB:{self.file}:1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("single letter", result.stderr)

    def test_a_comment_is_a_gutter_sign_that_reads_back(self):
        result = self.run_helper("--marks", f"A:{self.file}:4",
                                 "--comment", "A:this trusts the header",
                                 "--verify-comments", "A")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("marked 1 line(s): A", result.stdout)
        self.assertIn("A=this trusts the header", result.stdout)
        # Quiet by default: a sign in the gutter, no extra rows over the code.
        marks = self.extmark_details("eink-review-notes", "m[4].sign_text")
        self.assertIn("▶", marks)
        self.assertNotIn("virt_lines", marks)

    def test_inline_draws_the_note_above_the_line_when_asked(self):
        self.run_helper("--marks", f"A:{self.file}:4", "--comment", "A:check the header")
        result = self.run_helper("--inline", "A")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("inlined 1 note(s)", result.stdout)
        marks = self.extmark_details("eink-review-inline", "m[4].virt_lines[1][3][1]")
        self.assertIn("check the header", marks)

    def test_commenting_works_while_the_reader_is_typing(self):
        # The editor is in insert mode: a <Cmd> sent as keys would be swallowed, and the
        # note would silently not appear. Everything goes through an expression instead.
        subprocess.run(["nvim", "--server", self.socket, "--remote-send", "i"], check=False)
        result = self.run_helper("--marks", f"B:{self.file}:6",
                                 "--comment", "B:works while typing",
                                 "--verify-comments", "B")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("B=works while typing", result.stdout)
        self.assertEqual(self.nvim_expr("getline('.')"), "line 1", "text was typed into the buffer")

    def test_a_comment_on_an_unset_mark_fails_loudly(self):
        result = self.run_helper("--comment", "Z:nowhere")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("mark Z is not set", result.stderr)

    def test_clearing_comments_removes_signs_and_notes(self):
        self.run_helper("--marks", f"A:{self.file}:4", "--comment", "A:temporary",
                        "--inline", "A")
        result = self.run_helper("--clear-comments")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cleared", result.stdout)
        read = self.run_helper("--verify-comments", "A")
        self.assertIn("A=", read.stdout)
        self.assertNotIn("temporary", read.stdout)


if __name__ == "__main__":
    unittest.main()
