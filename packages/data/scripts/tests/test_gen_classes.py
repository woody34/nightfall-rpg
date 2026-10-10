"""Source provenance admission boundaries without mutating a shared source checkout."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "gen_classes.py"
SPEC = importlib.util.spec_from_file_location("gen_classes", SCRIPT)
GEN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GEN)


class SourceProvenanceTests(unittest.TestCase):
    def test_pinned_head_does_not_authorize_modified_source_inputs(self):
        responses = [SimpleNamespace(stdout=GEN.DATAPACK_REVISION),
                     SimpleNamespace(stdout=" M src/main/resources/data/skillTrees/classSkillTree.xml\n")]
        with patch.object(GEN.subprocess, "run", side_effect=responses) as git:
            with self.assertRaisesRegex(GEN.SourceError, "source inputs are dirty"):
                GEN.Datapack(Path("/unused/local/source"))
            command = git.call_args.args[0]
            self.assertIn("--untracked-files=all", command)
            self.assertIn("--ignored=matching", command)
            self.assertIn(GEN.LEARNING_SOURCE, command)
            self.assertIn(GEN.DATA_STATS_SKILLS, command)
            self.assertIn(GEN.DATA_STATS_CHARS, command)

    def test_clean_pinned_sources_are_admitted(self):
        responses = [SimpleNamespace(stdout=GEN.DATAPACK_REVISION),SimpleNamespace(stdout="")]
        with patch.object(GEN.subprocess, "run", side_effect=responses):
            self.assertEqual(GEN.Datapack(Path("/unused/local/source")).root,Path("/unused/local/source"))

    def test_other_source_revision_is_refused_before_working_tree_checks(self):
        with patch.object(GEN.subprocess, "run", return_value=SimpleNamespace(stdout="f"*40)) as git:
            with self.assertRaisesRegex(GEN.SourceError, "expected pinned"):
                GEN.Datapack(Path("/unused/local/source"))
            self.assertEqual(git.call_count,1)


if __name__ == "__main__":
    unittest.main()
