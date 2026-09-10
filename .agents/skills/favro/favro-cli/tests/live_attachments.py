"""Opt-in regression test against existing scratch boards; see README.md.

Uses only the Python standard library. Creates test cards and archives them on
completion, including failure. Never selects or edits pre-existing cards.
"""

import collections
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
import uuid


class LiveAttachments(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.env = dict(os.environ)
        env_file = Path(cls.env.get("FAVRO_ENV_FILE", "favro.env")).resolve()
        if env_file.is_file():
            for line in env_file.read_text().splitlines():
                if "=" in line and not line.lstrip().startswith("#"):
                    key, value = line.split("=", 1)
                    if key.strip().startswith("FAVRO_"):
                        cls.env.setdefault(key.strip(), value.strip().strip('"').strip("'"))
        required = ("FAVRO_EMAIL", "FAVRO_TOKEN", "FAVRO_TEST_COLLECTION",
                    "FAVRO_TEST_BOARD", "FAVRO_TEST_MOVE_BOARD")
        if any(not cls.env.get(key) for key in required):
            raise unittest.SkipTest("Requires credentials and explicit scratch collection/boards")
        cls.binary = str(Path(cls.env.get(
            "FAVRO_TEST_BINARY", Path(__file__).resolve().parents[1] / "target/debug/favro"
        )).resolve())
        cls.collection = cls.env["FAVRO_TEST_COLLECTION"]
        # Isolate configuration discovery and state from the invoking project.
        cls.workspace = tempfile.TemporaryDirectory(prefix="favro-attachments-")
        cls.addClassCleanup(cls.workspace.cleanup)
        for key in ("FAVRO_PROJECT_CONFIG", "FAVRO_ROLE", "FAVRO_ENV_FILE"):
            cls.env.pop(key, None)
        cls.env["FAVRO_STATE"] = str(Path(cls.workspace.name) / "state.json")
        cls.fixture = Path(cls.workspace.name) / "evidence.txt"
        cls.fixture.write_text("Favro attachment regression evidence.\n")

    def cli(self, *args):
        result = subprocess.run(
            [self.binary, "--collection", self.collection, *args],
            cwd=self.workspace.name, env=self.env, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120,
        )
        self.assertEqual(result.returncode, 0, f"{args[0]} failed: {result.stderr}")
        return result.stdout

    def card(self, common_id):
        return json.loads(self.cli("get", "--card", common_id))

    def test_attachment_survives_each_update(self):
        cases = [
            ("set-desc", "--text", "Replacement description"),
            ("set-notes", "--text", "Notes after upload"),
            ("set-result", "--text", "Evidence is attached"),
            ("set-todo", "--item", "Review the evidence"),
            ("archive",),
            ("move-board", "--board", self.env["FAVRO_TEST_MOVE_BOARD"]),
        ]
        for args in cases:
            with self.subTest(command=args[0]):
                output = self.cli("add", "--board", self.env["FAVRO_TEST_BOARD"],
                                  "--title", f"Attachment regression {uuid.uuid4().hex}")
                match = re.search(r"\[([^\[\]]+)\]\s*$", output)
                self.assertIsNotNone(match, "Created card ID missing from CLI output")
                common_id = match.group(1)
                archived = False
                try:
                    self.cli("attach", "--card", common_id, "--file", str(self.fixture))
                    before = self.card(common_id)
                    self.assertEqual(len(before.get("attachments", [])), 1)
                    self.cli(args[0], "--card", common_id, *args[1:])
                    archived = args[0] == "archive"
                    if archived:
                        # Also exercise unarchiving; get selects active instances.
                        self.cli("archive", "--card", common_id, "--undo")
                        archived = False
                    after = self.card(common_id)
                    expected = collections.Counter(a["name"] for a in before["attachments"])
                    actual = collections.Counter(a["name"] for a in after.get("attachments", []))
                    self.assertFalse(expected - actual, "Attachment disappeared")
                finally:
                    if not archived:
                        self.cli("archive", "--card", common_id)


    def test_repeated_edits_preserve_duplicate_names_and_checkboxes(self):
        output = self.cli("add", "--board", self.env["FAVRO_TEST_BOARD"],
                          "--title", f"Repeated attachment regression {uuid.uuid4().hex}")
        match = re.search(r"\[([^\[\]]+)\]\s*$", output)
        self.assertIsNotNone(match)
        common_id = match.group(1)
        try:
            # Distinct uploads with the same nontrivial filename must both survive.
            for _ in range(2):
                self.cli("attach", "--card", common_id, "--file", str(self.fixture),
                         "--name", "evidence [final] & *review* (blå).txt")
            expected = collections.Counter(a["name"] for a in self.card(common_id)["attachments"])
            self.assertEqual(sum(expected.values()), 2)
            cases = [
                ("set-desc", "--text=- [x] Human completed\n- [ ] Human pending"),
                ("set-notes", "--text", "First note"),
                ("set-notes", "--text", "Revised note"),
                ("set-result", "--text", "Evidence is attached"),
                ("set-todo", "--item", "Review evidence"),
                ("set-todo", "--clear"),
            ]
            for args in cases:
                self.cli(args[0], "--card", common_id, *args[1:])
                after = self.card(common_id)
                self.assertEqual(expected, collections.Counter(a["name"] for a in after["attachments"]))
                self.assertIn("☑ Human completed", after["detailedDescription"])
                self.assertIn("☐ Human pending", after["detailedDescription"])
            # File nodes must remain outside even an unfinished user code block.
            self.cli("set-desc", "--card", common_id, "--text", "```\nUnfinished code")
            after = self.card(common_id)
            self.assertEqual(expected, collections.Counter(a["name"] for a in after["attachments"]))
        finally:
            self.cli("archive", "--card", common_id)


if __name__ == "__main__":
    unittest.main(verbosity=2)
