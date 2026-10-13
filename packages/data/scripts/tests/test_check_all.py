"""Complete generator gate and ownership regression checks."""

from pathlib import Path
import hashlib
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from packages.data.scripts import check_all, gen_classes


class CompleteCheckTests(unittest.TestCase):
    def test_geometry_failure_fails_gate_but_generators_still_run(self):
        with patch.object(check_all.sys, "argv", ["check_all", "--datapack", "/dp", "--game", "/game"]), patch.object(
                check_all, "check_phase3_zone", side_effect=ValueError("invalid geometry")), patch.object(
                check_all.subprocess, "run", return_value=SimpleNamespace(returncode=0)) as run:
            self.assertEqual(check_all.main(), 1)
            self.assertEqual(run.call_count, 11)

    def test_every_check_runs_and_any_failure_fails_the_gate(self):
        for failed in range(11):
            with self.subTest(failed=failed), patch.object(
                    check_all.sys, "argv", ["check_all", "--datapack", "/dp", "--game", "/game"]), patch.object(
                    check_all.subprocess, "run", side_effect=[
                        SimpleNamespace(returncode=int(i == failed)) for i in range(11)]) as run:
                self.assertEqual(check_all.main(), 1)
                commands = [call.args[0] for call in run.call_args_list]
                self.assertEqual([Path(c[2]).name for c in commands], [
                    "gen_tables.py", "gen_classes.py", "gen_class_phase3.py", "gen_skills.py",
                    "gen_economy.py", "skill_oracle.py", "skill_oracle.py", "starter_weapon_oracle.py", "skill_oracle.py", "oracle.py", "economy_oracle.py"])
                self.assertTrue(all(c[-1] == "--check" for c in commands))
                self.assertIn("--class-phase3", commands[6])

    def test_all_successes_pass_the_gate(self):
        with patch.object(check_all.sys, "argv", ["check_all", "--datapack", "/dp", "--game", "/game"]), patch.object(
                check_all.subprocess, "run", return_value=SimpleNamespace(returncode=0)) as run:
            self.assertEqual(check_all.main(), 0)
            self.assertEqual(run.call_count, 11)

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


class Phase3ZoneTests(unittest.TestCase):
    def setUp(self):
        self.legacy = (check_all.DATA / "zones/test_zone.toml").read_bytes()
        self.geometry = (check_all.DATA / "zones/test_zone_phase3.toml").read_text()

    def validate(self, geometry, legacy=None):
        with tempfile.TemporaryDirectory() as directory:
            data = Path(directory)
            (data / "zones").mkdir()
            (data / "zones/test_zone.toml").write_bytes(self.legacy if legacy is None else legacy)
            (data / "zones/test_zone_phase3.toml").write_text(geometry)
            return check_all.check_phase3_zone(data)

    def test_shipped_policy_and_separate_hash_preserve_legacy_bytes(self):
        parsed = check_all.tomllib.loads(self.geometry)
        self.assertEqual(parsed["peace_zone"], {
            "shape": "square", "centre": [126, 126],
            "half_width_millitiles": 10000, "inclusive": True})
        self.assertEqual(check_all.check_phase3_zone(), hashlib.sha256(self.geometry.encode()).hexdigest())
        self.assertEqual(hashlib.sha256(self.legacy).hexdigest(),
                         "230dba10e1eadac901494658d794dee9a92a4f951e12a5c1d1295ea886ae678a")
        self.assertNotEqual(self.validate(self.geometry),
                            self.validate(self.geometry.replace("10000", "2000")))

    def test_inclusive_master_and_zone_bounds(self):
        # Class Master is exactly on the north boundary at half-width 2 tiles.
        self.validate(self.geometry.replace("10000", "2000"))
        # West/south edges equal the legacy zone minimum at half-width 126 tiles.
        self.validate(self.geometry.replace("10000", "126000"))

    def test_invalid_geometry_is_rejected(self):
        cases = [
            ("[126, 126]", "[125, 126]", "safe_point"),
            ("[126, 126]", '["126", 126]', "integer"),
            ("10000", "127000", "bounds"),
            ("10000", "1999", "Class Master"),
            ("10000", "0", "positive integer"),
            ("10000", "true", "positive integer"),
            ("10000", "10.0", "positive integer"),
            ('"square"', '"circle"', "inclusive square"),
            ("inclusive = true", "inclusive = false", "inclusive square"),
            ('"Nightfall configuration"', '"L2J"', "Nightfall configuration"),
            ('id = "test_zone"', 'id = "another_zone"', "zone id"),
        ]
        for old, new, message in cases:
            with self.subTest(new=new), self.assertRaisesRegex(ValueError, message):
                self.validate(self.geometry.replace(old, new))
        with self.assertRaises(check_all.tomllib.TOMLDecodeError):
            self.validate(self.geometry + "\ninvalid TOML")

    def test_missing_master_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Class Master"):
            self.validate(self.geometry, self.legacy.replace(b"Class Master", b"Other NPC"))


if __name__ == "__main__":
    unittest.main()
