"""Concurrency and reattach tests for per-review keying.

The failure this prevents: two PR reviews that pick the same worktree, socket, tab
label or tablet session, and then overwrite each other's marks.
"""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1] / "skills/eink-pr/scripts"
spec = importlib.util.spec_from_file_location("eink_registry", SCRIPTS / "registry.py")
registry = importlib.util.module_from_spec(spec)
# dataclasses looks the module up in sys.modules while processing the class.
sys.modules[spec.name] = registry
spec.loader.exec_module(registry)


class KeyTests(unittest.TestCase):
    def test_keys_are_filesystem_and_socket_safe(self):
        self.assertEqual(registry.review_key("42"), "42")
        self.assertEqual(registry.review_key("  PR-7/x  "), "pr-7-x")
        self.assertEqual(registry.review_key("self/fix expiry"), "self-fix-expiry")

    def test_empty_key_is_rejected(self):
        with self.assertRaises(ValueError):
            registry.review_key("   ")

    def test_resources_differ_per_review(self):
        keys = [registry.review_key(p) for p in (7, 8, "self-branch")]
        worktrees = [registry.worktree_dir("/home/x/repo", p) for p in (7, 8, "self-branch")]
        sockets = [registry.socket_path(p) for p in (7, 8, "self-branch")]
        self.assertEqual(len(set(keys)), len(keys))
        self.assertEqual(len(set(worktrees)), len(worktrees))
        self.assertEqual(len(set(sockets)), len(sockets))
        self.assertTrue(worktrees[0].endswith("repo-worktrees/pr-7"))

    def test_labels_and_tags_carry_pr_and_sha(self):
        self.assertEqual(registry.tab_label(88, "Add pagination"), "PR 88: Add pagination")
        self.assertTrue(registry.tab_label("self-expiry", "Clear maps").startswith("Self:"))
        self.assertEqual(registry.session_tags(88, "abcdef1234567890"), {"pr": "88", "sha": "abcdef123456"})


class RegistryTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = Path(self.dir.name) / "pr-reviews.json"

    def tearDown(self):
        self.dir.cleanup()

    def review(self, pr, **overrides):
        base = dict(
            key=registry.review_key(pr), pr=str(pr), sha="a" * 40,
            worktree=f"/tmp/wt-{pr}", socket=registry.socket_path(pr),
            tab_label=registry.tab_label(pr, "title"))
        base.update(overrides)
        return registry.Review(**base)

    def test_registry_round_trips_and_keeps_reviews_apart(self):
        reg = registry.Registry.load(self.path)
        reg.upsert(self.review(7, title="seven"))
        reg.upsert(self.review(8, title="eight"))
        reg.save()

        reloaded = registry.Registry.load(self.path)
        self.assertEqual(sorted(reloaded.reviews), ["7", "8"])
        self.assertEqual(reloaded.find(7).title, "seven")
        self.assertEqual(reloaded.find(8).socket, "/tmp/eink-pr-8.sock")

    def test_reattach_keeps_state_the_caller_did_not_repeat(self):
        reg = registry.Registry.load(self.path)
        reg.upsert(self.review(7, session="ses-1", tab_id="wV:t2"))
        reg.save()

        # A second run learns only the sha; the session and tab must survive.
        reg.upsert(self.review(7, sha="b" * 40))
        reg.save()
        found = registry.Registry.load(self.path).find(7)
        self.assertEqual(found.sha, "b" * 40)
        self.assertEqual(found.session, "ses-1")
        self.assertEqual(found.tab_id, "wV:t2")

    def test_remove_returns_what_it_dropped(self):
        reg = registry.Registry.load(self.path)
        reg.upsert(self.review(9, title="nine"))
        dropped = reg.remove(9)
        self.assertEqual(dropped.title, "nine")
        self.assertIsNone(reg.find(9))
        self.assertIsNone(reg.remove(9))

    def test_a_corrupt_registry_does_not_block_a_review(self):
        self.path.write_text("{not json")
        reg = registry.Registry.load(self.path)
        self.assertEqual(reg.reviews, {})


if __name__ == "__main__":
    unittest.main()
