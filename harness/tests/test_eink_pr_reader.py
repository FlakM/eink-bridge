"""Live-server check of the tablet -> agent return path.

Skips when no e-ink server is running, so the suite stays runnable anywhere. When a
server is up, this is the only test that proves the real acknowledgement contract the
skill depends on.
"""
import importlib.util
import os
from pathlib import Path
import sys
import unittest
import urllib.error
import urllib.request

HARNESS = Path(__file__).resolve().parents[1]
SERVER = os.environ.get("EINK_SERVER_URL", "http://localhost:3333")

spec = importlib.util.spec_from_file_location("reader_loop", HARNESS / "eink-pr/reader_loop.py")
reader = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = reader
spec.loader.exec_module(reader)


def server_up():
    try:
        with urllib.request.urlopen(SERVER.rstrip("/") + "/api/health", timeout=2):
            return True
    except (urllib.error.URLError, OSError):
        return False


@unittest.skipUnless(server_up(), f"no e-ink server at {SERVER}")
class ReaderLoopTests(unittest.TestCase):
    def test_note_resolves_to_a_marked_line_and_is_acknowledged_once(self):
        result = reader.run_loop(SERVER, "B is wrong", log=lambda *_: None)
        self.assertEqual((result["letter"], result["spot"]), ("B", "server/tests/health_test.rs:40"))
        self.assertIsNone(result["pending_after"], "acknowledged feedback must not stay pending")
        self.assertEqual(result["version"], 2)

    def test_a_note_without_a_mark_letter_does_not_invent_a_target(self):
        result = reader.run_loop(SERVER, "this whole section is unclear", log=lambda *_: None)
        self.assertEqual((result["letter"], result["spot"]), (None, None))
        self.assertIsNone(result["pending_after"])


if __name__ == "__main__":
    unittest.main()
