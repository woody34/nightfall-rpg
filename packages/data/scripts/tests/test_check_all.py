"""Complete generator gate and ownership regression checks."""

from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from packages.data.scripts import check_all, gen_classes


class CompleteCheckTests(unittest.TestCase):
    def test_every_check_runs_and_any_failure_fails_the_gate(self):
        for failed in range(7):
            with self.subTest(failed=failed), patch.object(
                    check_all.sys, "argv", ["check_all", "--datapack", "/dp", "--game", "/game"]), patch.object(
                    check_all.subprocess, "run", side_effect=[
                        SimpleNamespace(returncode=int(i == failed)) for i in range(7)]) as run:
                self.assertEqual(check_all.main(), 1)
                commands = [call.args[0] for call in run.call_args_list]
                self.assertEqual([Path(c[2]).name for c in commands], [
                    "gen_tables.py", "gen_classes.py", "gen_class_phase3.py", "gen_skills.py",
                    "skill_oracle.py", "skill_oracle.py", "oracle.py"])
                self.assertTrue(all(c[-1] == "--check" for c in commands))
                self.assertIn("--class-phase3", commands[5])

    def test_all_successes_pass_the_gate(self):
        with patch.object(check_all.sys, "argv", ["check_all", "--datapack", "/dp", "--game", "/game"]), patch.object(
                check_all.subprocess, "run", return_value=SimpleNamespace(returncode=0)) as run:
            self.assertEqual(check_all.main(), 0)
            self.assertEqual(run.call_count, 7)

    def test_class_generator_outputs_preserve_bytes_and_exclude_catalogue(self):
        data = Path(gen_classes.__file__).resolve().parent.parent
        dp = gen_classes.Datapack(Path("/home/matt-woodruff/.cache/nightfall/l2j/l2j-dp"))
        files = gen_classes.generate_all(dp)
        self.assertNotIn("skill_catalog.toml", files)
        self.assertEqual(len(files), 183)
        for rel, content in files.items():
            with self.subTest(path=rel):
                self.assertEqual(content.encode(), (data / rel).read_bytes())
        inventory = data.parents[1] / "apps/api/src/infrastructure/class_data_embedded.rs"
        self.assertEqual(gen_classes.embedded_inventory(files).encode(), inventory.read_bytes())


if __name__ == "__main__":
    unittest.main()
