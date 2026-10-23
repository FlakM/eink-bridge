"""Every flag a skill tells an agent to run must exist on the CLI.

The skills are the interface; the CLI is the implementation. A flag that is renamed in
one place and not the other fails at the worst moment — mid-review, on the tablet, in
front of the reader — so this reads the skills and asks the binary.
"""
import re
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BIN = ROOT / "server" / "target" / "debug" / "eink-review"
COMMANDS = {"push", "star", "unstar", "update", "content", "source", "watch",
            "result", "cancel", "remove", "list", "image"}
GLOBAL_FLAGS = {"--server"}


def cli(*args):
    return subprocess.run([str(BIN), *args], capture_output=True, text=True, timeout=60)


class SkillCliFlagTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not BIN.exists():
            subprocess.run(["cargo", "build", "--bin", "eink-review"],
                           cwd=ROOT / "server", check=True)

    def invocations(self):
        """Yield (skill, command, flags) for every eink-review call in every skill."""
        for path in sorted((ROOT / "harness" / "skills").glob("*/SKILL.md")):
            lines = path.read_text().splitlines()
            index = 0
            while index < len(lines):
                line = lines[index]
                index += 1
                if "eink-review" not in line:
                    continue
                # A long command is wrapped with a trailing backslash; check the whole
                # invocation, not just the line that mentions the binary.
                while line.rstrip().endswith("\\") and index < len(lines):
                    line += " " + lines[index].strip()
                    index += 1
                segment = line.split("eink-review", 1)[1]
                # Stop at the end of the invocation: prose often names another command
                # on the same line (`... fails: systemctl --user start ...`), and its
                # flags belong to that command, not to ours.
                for delimiter in ("`", "#", ";", "&&", "||", ")"):
                    segment = segment.split(delimiter, 1)[0]
                flags = set(re.findall(r"--[a-z][a-z0-9-]*", segment))
                words = re.findall(r"\b([a-z][a-z0-9-]*)\b", segment)
                command = next((word for word in words if word in COMMANDS), None)
                if command:
                    yield path.name, command, flags

    def test_every_flag_in_a_skill_exists_on_that_command(self):
        seen = 0
        failures = []
        for skill, command, flags in self.invocations():
            seen += 1
            help_text = cli(command, "--help").stdout
            for flag in sorted(flags - GLOBAL_FLAGS - {"--help"}):
                if flag not in help_text:
                    failures.append(f"{skill}: `eink-review {command} {flag}` does not exist")
        self.assertGreaterEqual(seen, 5, "the skills stopped being readable")
        self.assertEqual(failures, [], "\n".join(failures))

    def test_every_global_flag_exists_at_the_top_level(self):
        help_text = cli("--help").stdout
        for flag in sorted(GLOBAL_FLAGS):
            self.assertIn(flag, help_text)
