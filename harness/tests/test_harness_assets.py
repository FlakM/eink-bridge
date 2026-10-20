"""Structural tests for the harness assets: agents, skills, and how they wire together.

These catch the quiet failures — a skill that lost its frontmatter, an agent file that
never declares a model, a pipeline stage that stopped being referenced.
"""
import json
from pathlib import Path
import re
import unittest

HARNESS = Path(__file__).resolve().parents[1]
SKILLS = HARNESS / "skills"
AGENTS = HARNESS / "agents"


def frontmatter(path):
    text = path.read_text()
    if not text.startswith("---"):
        return None
    end = text.index("\n---", 3)
    block = text[3:end]
    fields = {}
    for line in block.splitlines():
        match = re.match(r"^([A-Za-z_-]+):\s*(.*)$", line)
        if not match:
            continue
        key, value = match.group(1), match.group(2).strip()
        if value.startswith('"'):
            try:
                value = json.loads(value)
            except json.JSONDecodeError:
                value = value.strip('"')
        fields[key] = value
    return fields


class AgentTests(unittest.TestCase):
    def test_agents_dir_has_the_attention_guard(self):
        self.assertTrue((AGENTS / "ihaveadhd.md").is_file(), "ihaveadhd agent is missing")

    def test_every_agent_declares_name_description_and_model(self):
        for path in sorted(AGENTS.glob("*.md")):
            fields = frontmatter(path)
            self.assertIsNotNone(fields, f"{path.name} has no frontmatter")
            for key in ("name", "description", "model"):
                self.assertTrue(fields.get(key), f"{path.name} missing {key}")

    def test_ihaveadhd_describes_when_to_reach_for_it(self):
        fields = frontmatter(AGENTS / "ihaveadhd.md")
        description = fields["description"].lower()
        for trigger in ("eink", "comment"):
            self.assertIn(trigger, description, f"description should mention {trigger}")

    def test_ihaveadhd_keeps_the_reader_contract(self):
        body = (AGENTS / "ihaveadhd.md").read_text().lower()
        # The rules that make it useful, not just an instruction to "be brief".
        for rule in ("file:line", "mark letters", "cut", "summary"):
            self.assertIn(rule, body, f"ihaveadhd should mention {rule}")


class SkillTests(unittest.TestCase):
    def test_skills_that_declare_frontmatter_are_complete(self):
        """A skill without frontmatter is user-invoked only; that is a valid choice.
        When it does declare one, both fields must be present to be discoverable."""
        for path in sorted(SKILLS.glob("*/SKILL.md")):
            fields = frontmatter(path)
            if fields is None:
                continue
            self.assertTrue(fields.get("name"), f"{path} declares frontmatter without a name")
            self.assertTrue(fields.get("description"), f"{path} declares frontmatter without a description")

    def test_expected_skills_are_present(self):
        names = {path.parent.name for path in SKILLS.glob("*/SKILL.md")}
        for expected in ("eink", "eink-pr", "eink-update", "jump"):
            self.assertIn(expected, names)

    def test_eink_pr_is_wired_to_the_pipeline_and_helpers(self):
        body = (SKILLS / "eink-pr" / "SKILL.md").read_text()
        for reference in ("review.py", "nvim_marks.py", "ihaveadhd", "jump", "eink"):
            self.assertIn(reference, body, f"eink-pr should reference {reference}")

    def test_jump_skill_points_at_the_socket_helper(self):
        body = (SKILLS / "jump" / "SKILL.md").read_text()
        self.assertIn("nvim_marks.py", body)
        self.assertIn("EINK_PR_SOCKET", body)


class ContinueFlowTests(unittest.TestCase):
    """`/eink continue` must survive a Claude restart.

    Subscriptions live in memory in the channel, so a restarted instance silently
    acknowledges nothing until it claims the session again. The flow has to re-subscribe
    (which also re-registers the callback) and read the durable pending feedback.
    """

    def section(self, heading, next_heading):
        text = (SKILLS / "eink" / "SKILL.md").read_text()
        start = text.index(heading)
        # Search *after* the heading: "**Portable path" also appears earlier in the file,
        # and slicing to an earlier index silently yields "".
        end = text.find(next_heading, start)
        return text[start : end if end != -1 else len(text)]

    def test_continue_resubscribes_and_reads_pending_feedback(self):
        section = self.section("## Steps for `/eink continue", "## Steps for a new push")
        self.assertIn("subscribe_session", section)
        self.assertIn("/feedback", section)

    def test_the_channel_registers_its_own_port(self):
        section = self.section("### 2. Connect delivery", "**Portable path")
        self.assertNotIn("$CHANNEL_PORT", section,
                         "a guessed port is not the port the channel is listening on")
        self.assertIn("subscribe_session", section)


class BatchReviewTests(unittest.TestCase):
    """A batch is one tablet note, however many pull requests it contains."""

    def test_a_batch_has_one_eink_session(self):
        text = (SKILLS / "eink-pr" / "SKILL.md").read_text()
        section = text[text.index("## Several PRs at once"):text.index("## Testing this workflow")]
        self.assertIn("one for the whole batch", section)
        self.assertIn("one brief and one e-ink session", section)
        self.assertNotIn("one per PR, one observer each", section)


if __name__ == "__main__":
    unittest.main()
