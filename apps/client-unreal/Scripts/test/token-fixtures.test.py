#!/usr/bin/env python3
"""Focused unit tests for Phase2 token fixtures and overlay orchestration."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
REPO = SCRIPTS.parents[2]


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


token_fixtures = load_module("token_fixtures", SCRIPTS / "token-fixtures.py")


class TokenFixturesTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.owner = "01970000-0000-7000-8000-000000000020"
        self.observer = "01970000-0000-7000-8000-000000000040"

    def test_pinned_constants_match_seeder_and_table_contracts(self):
        xp_rows = tomllib.loads((REPO / "packages/data/tables/experience.toml").read_text())["to_level"]
        xp_table = {r["level"]: r["xp"] for r in xp_rows}
        penalties_rows = tomllib.loads((REPO / "packages/data/tables/penalties.toml").read_text())["death_xp_loss"]
        fraction_q = {r["level"]: r["fraction_q"] for r in penalties_rows}

        for profile, expected in token_fixtures.PINNED_ORACLES.items():
            with self.subTest(profile=profile):
                if profile == "phase2-token-backfill":
                    self.assertEqual(expected["seed"]["level"], 40)
                    self.assertEqual(expected["seed"]["xp"], xp_table[40])
                    self.assertEqual(expected["seed"]["tokens"], [0, 0])
                    self.assertEqual(expected["visible"]["tokens"], [1, 1])
                    continue

                seed_cfg = expected["seed"]
                oracle_cfg = expected["oracle"]
                lvl = seed_cfg["level"]
                dest = seed_cfg["destination_level"]
                seed_thresh = seed_cfg["seed_threshold_xp"]

                # Seed XP must be threshold - 1
                self.assertEqual(seed_cfg["xp"], xp_table[dest if profile != "phase2-token-jump19-40" else 20] - 1)
                self.assertEqual(seed_thresh, xp_table[dest if profile != "phase2-token-jump19-40" else 20])

                # Human adaptable racial trait is +5% bonus: floor(raw * 1.05)
                raw = oracle_cfg["raw_xp"]
                kill_xp = oracle_cfg["human_kill_xp"]
                self.assertEqual(int(raw * 105 // 100), kill_xp)

                # Kill lands precisely at or above destination threshold
                self.assertEqual(seed_cfg["xp"] + kill_xp, oracle_cfg["first_kill_xp"])
                self.assertEqual(oracle_cfg["first_kill_level"], dest)
                self.assertGreaterEqual(oracle_cfg["first_kill_xp"], xp_table[dest])

                if profile in ("phase2-token-level20", "phase2-token-level40"):
                    # Death penalty formula: round((xp[dest+1] - xp[dest]) * fraction_q / 1e6)
                    loss = round((xp_table[dest + 1] - xp_table[dest]) * fraction_q[dest] / 1_000_000)
                    self.assertEqual(oracle_cfg["death_loss"], loss)
                    self.assertEqual(oracle_cfg["first_kill_xp"] - loss, oracle_cfg["death_xp"])
                    self.assertEqual(oracle_cfg["death_level"], dest - 1)
                    # Respawn XP unchanged
                    self.assertEqual(oracle_cfg["respawn_xp"], oracle_cfg["death_xp"])
                    # Second kill reaches destination level
                    self.assertEqual(oracle_cfg["respawn_xp"] + kill_xp, oracle_cfg["second_kill_xp"])
                    self.assertEqual(oracle_cfg["second_kill_level"], dest)

                    # Post-consumption re-crossing verification (independent pinned source confirmation)
                    self.assertIn("post_consumption", expected)
                    post_cfg = expected["post_consumption"]
                    self.assertEqual(post_cfg["second_death_loss"], loss)
                    self.assertEqual(post_cfg["death_loss"], loss)
                    self.assertEqual(post_cfg["death_xp"], oracle_cfg["second_kill_xp"] - loss)
                    self.assertEqual(post_cfg["death_level"], dest - 1)
                    self.assertEqual(post_cfg["respawn_xp"], post_cfg["death_xp"])
                    self.assertEqual(post_cfg["respawn_level"], dest - 1)
                    self.assertTrue(post_cfg["respawn_unchanged"])
                    self.assertEqual(post_cfg["third_kill_xp_gain"], kill_xp)
                    self.assertEqual(post_cfg["third_kill_xp"], post_cfg["respawn_xp"] + kill_xp)
                    self.assertEqual(post_cfg["third_kill_level"], dest)
                    self.assertEqual(post_cfg["tokens"], [0, 0])
                    self.assertEqual(post_cfg["third_kill_tokens"], [0, 0])
                    target_class = 1 if profile == "phase2-token-level20" else 2
                    self.assertEqual(post_cfg["current_class"], target_class)
                    self.assertEqual(post_cfg["class_id"], target_class)

    def test_overlay_creation_and_contents(self):
        for profile in token_fixtures.TOKEN_CROSSING:
            with self.subTest(profile=profile):
                dest = self.root / f"overlay_{profile}"
                meta = token_fixtures.create_overlay(dest, profile, REPO)

                self.assertTrue(Path(meta["rules_dir"]).is_dir())
                self.assertTrue(Path(meta["zone_file"]).is_file())

                # Validate zone
                zone = tomllib.loads(Path(meta["zone_file"]).read_text())
                self.assertEqual(zone["bounds"], {"min": [0, 0], "max": [256, 256]})
                self.assertEqual(zone["safe_point"], {"pos": [126, 126]})
                self.assertEqual(len(zone["npcs"]), 1)
                self.assertEqual(zone["npcs"][0], {"name": "Class Master", "attackable": False, "pos": [126, 128], "speed": 0})
                self.assertEqual(len(zone["spawn_slots"]), 2)
                slots = {s["id"]: s for s in zone["spawn_slots"]}
                self.assertEqual(slots["reward_spawn"]["home"], [126, 125])
                self.assertEqual(slots["reward_spawn"]["template"], "token_oracle")
                self.assertEqual(slots["sentinel_spawn"]["home"], [130, 126])
                self.assertEqual(slots["sentinel_spawn"]["template"], "token_sentinel")

                # Validate oracle template
                oracle = tomllib.loads((dest / "npcs/token_oracle.toml").read_text())
                self.assertEqual(oracle["id"], "token_oracle")
                self.assertEqual(oracle["max_hp"], 1)
                self.assertGreater(oracle["move_speed"], 0)
                self.assertFalse(oracle["aggressive"])
                self.assertEqual(oracle["clan_help_range"], 0)
                self.assertEqual(oracle["corpse_decay_ticks"], 10)
                self.assertEqual(oracle["respawn_delay_secs"], 1)
                self.assertEqual(oracle["respawn_random_secs"], 0)
                self.assertEqual(oracle["xp_reward"], token_fixtures.PINNED_ORACLES[profile]["oracle"]["raw_xp"])

                # Validate sentinel template
                sentinel = tomllib.loads((dest / "npcs/token_sentinel.toml").read_text())
                self.assertEqual(sentinel["id"], "token_sentinel")
                self.assertEqual(sentinel["max_hp"], 1000000)
                self.assertGreater(sentinel["move_speed"], 0)
                self.assertFalse(sentinel["aggressive"])
                self.assertEqual(sentinel["clan_help_range"], 0)
                self.assertEqual(sentinel["p_atk"], "10000.0")

                # Verify sentinel lethal damage
                # Formula: damage = floor(76 * p_atk / p_def)
                # Level 40 Human Fighter unarmored P.Def ~ 150
                damage = int(76 * float(sentinel["p_atk"]) / 150)
                self.assertGreaterEqual(damage, 5000)

    def test_malformed_template_and_zone_rejection(self):
        # Template validation checks
        oracle_content = token_fixtures.oracle_template_content(10235)
        temp_oracle = self.root / "token_oracle.toml"

        # Valid oracle passes
        temp_oracle.write_text(oracle_content)
        self.assertIsNotNone(token_fixtures.validate_template(temp_oracle))

        # Float p_atk rejected
        temp_oracle.write_text(oracle_content.replace('p_atk = "1.0"', "p_atk = 1.0"))
        with self.assertRaises(ValueError):
            token_fixtures.validate_template(temp_oracle)

        # Negative p_atk rejected
        temp_oracle.write_text(oracle_content.replace('p_atk = "1.0"', 'p_atk = "-1.0"'))
        with self.assertRaises(ValueError):
            token_fixtures.validate_template(temp_oracle)

        # 0 move speed rejected (no exception for stationary NPCs)
        temp_oracle.write_text(oracle_content.replace("move_speed = 400", "move_speed = 0"))
        with self.assertRaises(ValueError):
            token_fixtures.validate_template(temp_oracle)

        # 0 max_hp rejected
        temp_oracle.write_text(oracle_content.replace("max_hp = 1", "max_hp = 0"))
        with self.assertRaises(ValueError):
            token_fixtures.validate_template(temp_oracle)

        # Unknown field rejected
        temp_oracle.write_text(oracle_content + "\nunknown_field = 123\n")
        with self.assertRaises(ValueError):
            token_fixtures.validate_template(temp_oracle)

        # Empty clan_id rejected
        temp_oracle.write_text(oracle_content + '\nclan_id = ""\n')
        with self.assertRaises(ValueError):
            token_fixtures.validate_template(temp_oracle)

        # Zone validation checks
        zone_content = token_fixtures.overlay_zone_content()
        temp_zone = self.root / "test_zone.toml"
        npcs_dir = self.root / "npcs"
        npcs_dir.mkdir(parents=True, exist_ok=True)
        (npcs_dir / "token_oracle.toml").write_text(oracle_content)
        (npcs_dir / "token_sentinel.toml").write_text(token_fixtures.sentinel_template_content())

        # Valid zone passes
        temp_zone.write_text(zone_content)
        self.assertIsNotNone(token_fixtures.validate_zone(temp_zone, npcs_dir))

        # Wrong bounds rejected
        temp_zone.write_text(zone_content.replace("max = [256, 256]", "max = [512, 512]"))
        with self.assertRaises(ValueError):
            token_fixtures.validate_zone(temp_zone, npcs_dir)

        # Wrong safe point rejected
        temp_zone.write_text(zone_content.replace("pos = [126, 126]", "pos = [0, 0]"))
        with self.assertRaises(ValueError):
            token_fixtures.validate_zone(temp_zone, npcs_dir)

        # Attackable NPC in npcs rejected
        temp_zone.write_text(zone_content.replace("attackable = false", "attackable = true"))
        with self.assertRaises(ValueError):
            token_fixtures.validate_zone(temp_zone, npcs_dir)

        # Slot outside bounds rejected
        temp_zone.write_text(zone_content.replace("home = [126, 125]", "home = [300, 300]"))
        with self.assertRaises(ValueError):
            token_fixtures.validate_zone(temp_zone, npcs_dir)

    def test_seed_manifest_validation_and_tamper_rejection(self):
        # 1. Old3 profiles pass
        old_transfers = {
            "phase2-transfer": {
                "fixture_enabled": True, "fixture": "phase2-transfer", "dry_run": False,
                "owner": {"account_id": self.owner, "character_id": self.owner, "level": 40,
                          "class_id": 0, "sex": "female", "tokens": [1, 1], "milestone_claimed_mask": 3},
                "position": [126, 126], "transfers": [1, 2],
            },
            "phase2-transfer-missing-token": {
                "fixture_enabled": True, "fixture": "phase2-transfer-missing-token", "dry_run": False,
                "owner": {"account_id": self.owner, "character_id": self.owner, "level": 20,
                          "class_id": 0, "sex": "female", "tokens": [0, 0], "milestone_claimed_mask": 1},
                "position": [126, 126], "transfers": [],
            },
            "phase2-transfer-observer": {
                "fixture_enabled": True, "fixture": "phase2-transfer-observer", "dry_run": False,
                "owner": {"account_id": self.owner, "character_id": self.owner, "level": 40,
                          "class_id": 0, "sex": "female", "tokens": [1, 1], "milestone_claimed_mask": 3},
                "observer": {"account_id": self.observer, "character_id": self.observer, "level": 1,
                             "class_id": 0, "sex": "male"},
                "position": [126, 126], "transfers": [1, 2],
            },
        }
        for profile, manifest in old_transfers.items():
            obs = self.observer if profile == "phase2-transfer-observer" else None
            token_fixtures.validate_seed_manifest(manifest, profile, self.owner, obs)

        # 2. Backfill profile passes
        backfill_manifest = {
            "fixture_enabled": True, "fixture": "phase2-token-backfill", "dry_run": False,
            "owner": {"account_id": self.owner, "character_id": self.owner, "level": 40,
                      "class_id": 0, "sex": "female", "tokens": [0, 0], "milestone_claimed_mask": 0,
                      "race": "human", "xp": 15422929, "hp": None, "mp": None, "cp": 0, "sp": 0,
                      "position": [126, 126]},
            "position": [126, 126], "transfers": [],
        }
        token_fixtures.validate_seed_manifest(backfill_manifest, "phase2-token-backfill", self.owner)

        # 3. Crossing profiles pass
        crossing_manifests = {}
        for profile, (lvl, toks, mask, xp, dest, raw, award, thresh, seed_thresh) in [
            ("phase2-token-level20", (19, [0, 0], 0, 835861, 20, 10235, 10746, 835862, 835862)),
            ("phase2-token-level40", (39, [1, 0], 1, 15422928, 40, 62751, 65888, 15422929, 15422929)),
            ("phase2-token-jump19-40", (19, [0, 0], 0, 835861, 40, 13892446, 14587068, 15422929, 835862)),
        ]:
            crossing_manifest = {
                "fixture_enabled": True, "fixture": profile, "dry_run": False,
                "owner": {"account_id": self.owner, "character_id": self.owner, "level": lvl,
                          "class_id": 0, "sex": "female", "tokens": toks, "milestone_claimed_mask": mask,
                          "race": "human", "xp": xp, "hp": None, "mp": None, "cp": 0, "sp": 0,
                          "position": [126, 126]},
                "position": [126, 126], "transfers": [],
                "crossing": {"level": dest, "threshold_xp": thresh, "seed_threshold_xp": seed_thresh,
                             "xp_margin": 1, "npc_raw_xp": raw, "kill_xp": award,
                             "expected_level": dest, "expected_xp": xp + award},
            }
            crossing_manifests[profile] = crossing_manifest
            token_fixtures.validate_seed_manifest(crossing_manifest, profile, self.owner)

        # 4. Actual real seeders dry-run validation (preserves all 7 profiles including old3 contract)
        seeder = REPO / "infra/scripts/seed-phase2-transfer-fixture.py"
        seeder_env = dict(os.environ, AUTH_DEV_TOKENS="1", NIGHTFALL_PHASE2_FIXTURE="1",
                          DATABASE_URL="postgres://test:test@127.0.0.1:26433/nf_phase2_fixture_contract")
        real_manifests = {}
        for profile in sorted(token_fixtures.PHASE2):
            cmd = [sys.executable, str(seeder), "--fixture", profile, "--owner-account", self.owner, "--dry-run"]
            obs = self.observer if profile == "phase2-transfer-observer" else None
            if obs:
                cmd += ["--observer-account", obs]
            res = subprocess.run(cmd, env=seeder_env, text=True, capture_output=True, check=True, timeout=10)
            real_m = json.loads(res.stdout.splitlines()[-1])
            real_m["dry_run"] = False
            real_manifests[profile] = (real_m, obs)
            with self.subTest(real_seeder=profile):
                token_fixtures.validate_seed_manifest(real_m, profile, self.owner, obs)

        # 5. Tampered structural manifests are rejected
        # Extra key in root
        tampered_root = copy.deepcopy(backfill_manifest)
        tampered_root["extra_root_key"] = "bad"
        with self.assertRaises(ValueError):
            token_fixtures.validate_seed_manifest(tampered_root, "phase2-token-backfill", self.owner)

        # Extra key in owner
        tampered_owner = copy.deepcopy(backfill_manifest)
        tampered_owner["owner"]["extra_owner_key"] = "bad"
        with self.assertRaises(ValueError):
            token_fixtures.validate_seed_manifest(tampered_owner, "phase2-token-backfill", self.owner)

        # Extra key in crossing
        jump_manifest = crossing_manifests["phase2-token-jump19-40"]
        tampered_crossing = copy.deepcopy(jump_manifest)
        tampered_crossing["crossing"]["extra_crossing_key"] = "bad"
        with self.assertRaises(ValueError):
            token_fixtures.validate_seed_manifest(tampered_crossing, "phase2-token-jump19-40", self.owner)

        # Wrong token count
        tampered_tokens = copy.deepcopy(jump_manifest)
        tampered_tokens["owner"]["tokens"] = [1, 1]
        with self.assertRaises(ValueError):
            token_fixtures.validate_seed_manifest(tampered_tokens, "phase2-token-jump19-40", self.owner)

        # Non-empty transfers for solo new4
        tampered_transfers = copy.deepcopy(jump_manifest)
        tampered_transfers["transfers"] = [1]
        with self.assertRaises(ValueError):
            token_fixtures.validate_seed_manifest(tampered_transfers, "phase2-token-jump19-40", self.owner)

        # Unexpected observer in solo profile
        tampered_obs = copy.deepcopy(jump_manifest)
        tampered_obs["observer"] = {"account_id": self.observer, "character_id": self.observer, "level": 1, "class_id": 0, "sex": "male"}
        with self.assertRaises(ValueError):
            token_fixtures.validate_seed_manifest(tampered_obs, "phase2-token-jump19-40", self.owner)

        # 6. Strict type tampering on actual valid manifests
        # Build combined set of all 7 valid profile manifests for tampering tests
        all_manifests = dict(real_manifests)

        # 6a. Tamper boolean token counts (reproduced across all 7 profiles)
        for profile, (base_m, obs) in all_manifests.items():
            for bad_tokens in ([False, False], [True, 0], [0, False], [True, True], [1, True], [0.0, 0]):
                m = copy.deepcopy(base_m)
                m["owner"]["tokens"] = bad_tokens
                with self.subTest(tamper="bool_token_count", profile=profile, bad_tokens=bad_tokens):
                    with self.assertRaises(ValueError):
                        token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

        # 6b. Tamper boolean class_id (reproduced across all 7 profiles)
        for profile, (base_m, obs) in all_manifests.items():
            for bad_class in (False, True, 0.0):
                m = copy.deepcopy(base_m)
                m["owner"]["class_id"] = bad_class
                with self.subTest(tamper="bool_class_id", profile=profile, bad_class=bad_class):
                    with self.assertRaises(ValueError):
                        token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

        # 6c. Tamper float level (reproduced across all 7 profiles)
        for profile, (base_m, obs) in all_manifests.items():
            m = copy.deepcopy(base_m)
            m["owner"]["level"] = float(m["owner"]["level"])
            with self.subTest(tamper="float_level", profile=profile):
                with self.assertRaises(ValueError):
                    token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

        # 6d. Tamper float xp (all new profiles: backfill + crossing)
        for profile in ("phase2-token-backfill", "phase2-token-level20", "phase2-token-level40", "phase2-token-jump19-40"):
            base_m, obs = all_manifests[profile]
            m = copy.deepcopy(base_m)
            m["owner"]["xp"] = float(m["owner"]["xp"])
            with self.subTest(tamper="float_xp", profile=profile):
                with self.assertRaises(ValueError):
                    token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

        # 6e. Tamper float crossing numbers (all crossing profiles)
        for profile in token_fixtures.TOKEN_CROSSING:
            base_m, obs = all_manifests[profile]
            crossing_fields = ["level", "threshold_xp", "seed_threshold_xp", "npc_raw_xp", "kill_xp", "expected_level", "expected_xp"]
            for field in crossing_fields:
                m = copy.deepcopy(base_m)
                m["crossing"][field] = float(m["crossing"][field])
                with self.subTest(tamper=f"float_crossing_{field}", profile=profile):
                    with self.assertRaises(ValueError):
                        token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)
            # xp_margin float and bool
            for bad_margin in (1.0, True, 0):
                m = copy.deepcopy(base_m)
                m["crossing"]["xp_margin"] = bad_margin
                with self.subTest(tamper="crossing_xp_margin", profile=profile, bad_margin=bad_margin):
                    with self.assertRaises(ValueError):
                        token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

        # 6f. Tamper float / bool coordinates (root position across all 7 profiles, and owner position)
        for profile, (base_m, obs) in all_manifests.items():
            for bad_pos in ([126.0, 126], [126, 126.0], [True, 126], [126, False]):
                m = copy.deepcopy(base_m)
                m["position"] = bad_pos
                with self.subTest(tamper="root_coordinate", profile=profile, bad_pos=bad_pos):
                    with self.assertRaises(ValueError):
                        token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

            if "position" in base_m["owner"]:
                for bad_pos in ([126.0, 126], [126, 126.0], [True, 126], [126, False]):
                    m = copy.deepcopy(base_m)
                    m["owner"]["position"] = bad_pos
                    with self.subTest(tamper="owner_coordinate", profile=profile, bad_pos=bad_pos):
                        with self.assertRaises(ValueError):
                            token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

        # 6g. Tamper wrong types for null fields (hp / mp in backfill and crossing)
        for profile in token_fixtures.TOKEN_PROFILES:
            base_m, obs = all_manifests[profile]
            for null_field in ("hp", "mp"):
                for bad_null in (0, 0.0, False, "null", ""):
                    m = copy.deepcopy(base_m)
                    m["owner"][null_field] = bad_null
                    with self.subTest(tamper=f"null_{null_field}", profile=profile, bad_null=bad_null):
                        with self.assertRaises(ValueError):
                            token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

        # 6h. Tamper wrong types for string fields
        for profile, (base_m, obs) in all_manifests.items():
            for str_field in ("account_id", "character_id", "sex"):
                for bad_str in (123, 123.0, True, None):
                    m = copy.deepcopy(base_m)
                    m["owner"][str_field] = bad_str
                    with self.subTest(tamper=f"owner_{str_field}_str", profile=profile, bad_str=bad_str):
                        with self.assertRaises(ValueError):
                            token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)
            # root fixture string
            m = copy.deepcopy(base_m)
            m["fixture"] = 123
            with self.subTest(tamper="root_fixture_str", profile=profile):
                with self.assertRaises(ValueError):
                    token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

            if "race" in base_m["owner"]:
                for bad_str in (123, True, None):
                    m = copy.deepcopy(base_m)
                    m["owner"]["race"] = bad_str
                    with self.subTest(tamper="owner_race_str", profile=profile, bad_str=bad_str):
                        with self.assertRaises(ValueError):
                            token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

        # 6i. Tamper milestone_claimed_mask (bool/float equivalence)
        for profile, (base_m, obs) in all_manifests.items():
            for bad_mask in (True, False, float(base_m["owner"]["milestone_claimed_mask"])):
                m = copy.deepcopy(base_m)
                m["owner"]["milestone_claimed_mask"] = bad_mask
                with self.subTest(tamper="claimed_mask", profile=profile, bad_mask=bad_mask):
                    with self.assertRaises(ValueError):
                        token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

        # 6j. Tamper transfers (bool/float equivalence or wrong types)
        for profile in ("phase2-transfer", "phase2-transfer-observer"):
            base_m, obs = all_manifests[profile]
            for bad_transfers in ([True, 2], [1.0, 2], [1, 2.0], [1, True]):
                m = copy.deepcopy(base_m)
                m["transfers"] = bad_transfers
                with self.subTest(tamper="transfers_type", profile=profile, bad_transfers=bad_transfers):
                    with self.assertRaises(ValueError):
                        token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

        # 6k. Tamper observer fields
        obs_m, obs_id = all_manifests["phase2-transfer-observer"]
        for bad_obs in (
            {"class_id": False},
            {"class_id": 0.0},
            {"level": 1.0},
            {"level": True},
            {"sex": 0},
            {"account_id": 123},
            {"character_id": 456},
        ):
            m = copy.deepcopy(obs_m)
            m["observer"].update(bad_obs)
            with self.subTest(tamper="observer_field", bad_obs=bad_obs):
                with self.assertRaises(ValueError):
                    token_fixtures.validate_seed_manifest(m, "phase2-transfer-observer", self.owner, obs_id)

        # 6l. Tamper cp / sp in backfill and crossing profiles
        for profile in token_fixtures.TOKEN_PROFILES:
            base_m, obs = all_manifests[profile]
            for resource in ("cp", "sp"):
                for bad_val in (False, 0.0):
                    m = copy.deepcopy(base_m)
                    m["owner"][resource] = bad_val
                    with self.subTest(tamper=f"owner_{resource}", profile=profile, bad_val=bad_val):
                        with self.assertRaises(ValueError):
                            token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

        # 6m. Tamper boolean root flags
        for profile, (base_m, obs) in all_manifests.items():
            for flag, bad_val in (("fixture_enabled", 1), ("fixture_enabled", 1.0), ("fixture_enabled", "true"),
                                  ("dry_run", 0), ("dry_run", 0.0), ("dry_run", "false")):
                m = copy.deepcopy(base_m)
                m[flag] = bad_val
                with self.subTest(tamper="root_bool_flag", profile=profile, flag=flag, bad_val=bad_val):
                    with self.assertRaises(ValueError):
                        token_fixtures.validate_seed_manifest(m, profile, self.owner, obs)

    def test_oracle_manifest_generation(self):
        for profile in token_fixtures.TOKEN_PROFILES:
            with self.subTest(profile=profile):
                if profile in token_fixtures.TOKEN_CROSSING:
                    dest = self.root / f"mani_{profile}"
                    token_fixtures.create_overlay(dest, profile, REPO)
                    rules_dir = dest
                    zone_file = dest / "zones/test_zone.toml"
                else:
                    rules_dir = REPO / "packages/data"
                    zone_file = REPO / "packages/data/zones/test_zone.toml"

                manifest = token_fixtures.build_oracle_manifest(profile, rules_dir, zone_file, REPO)
                self.assertEqual(manifest["fixture"], profile)
                self.assertEqual(manifest["is_overlay"], profile in token_fixtures.TOKEN_CROSSING)
                self.assertIn("provenance", manifest)
                self.assertIn("source_tables_sha256", manifest["provenance"])
                self.assertEqual(manifest["sha256"]["zone"], token_fixtures.digest(zone_file))
                if profile in token_fixtures.TOKEN_CROSSING:
                    self.assertIn("token_oracle", manifest["sha256"])
                    self.assertIn("token_sentinel", manifest["sha256"])
                if profile in ("phase2-token-level20", "phase2-token-level40"):
                    self.assertIn("post_consumption", manifest["constants"])
                    post = manifest["constants"]["post_consumption"]
                    if profile == "phase2-token-level20":
                        self.assertEqual(post["second_death_loss"], 14329)
                        self.assertEqual(post["death_xp"], 828695)
                        self.assertEqual(post["death_level"], 19)
                        self.assertEqual(post["respawn_xp"], 828695)
                        self.assertTrue(post["respawn_unchanged"])
                        self.assertEqual(post["third_kill_xp_gain"], 10746)
                        self.assertEqual(post["third_kill_xp"], 839441)
                        self.assertEqual(post["third_kill_level"], 20)
                        self.assertEqual(post["tokens"], [0, 0])
                        self.assertEqual(post["current_class"], 1)
                    elif profile == "phase2-token-level40":
                        self.assertEqual(post["second_death_loss"], 87851)
                        self.assertEqual(post["death_xp"], 15379002)
                        self.assertEqual(post["death_level"], 39)
                        self.assertEqual(post["respawn_xp"], 15379002)
                        self.assertTrue(post["respawn_unchanged"])
                        self.assertEqual(post["third_kill_xp_gain"], 65888)
                        self.assertEqual(post["third_kill_xp"], 15444890)
                        self.assertEqual(post["third_kill_level"], 40)
                        self.assertEqual(post["tokens"], [0, 0])
                        self.assertEqual(post["current_class"], 2)

    def test_no_source_data_mutation(self):
        src = REPO / "packages/data"
        before_hashes = {p.relative_to(src): token_fixtures.digest(p) for p in src.rglob("*.toml")}

        dest = self.root / "check_mutation"
        token_fixtures.create_overlay(dest, "phase2-token-level20", REPO)

        after_hashes = {p.relative_to(src): token_fixtures.digest(p) for p in src.rglob("*.toml")}
        self.assertEqual(before_hashes, after_hashes)


if __name__ == "__main__":
    unittest.main()
