#!/usr/bin/env python3
"""Narrow helper for Phase2 token milestone and admission backfill fixtures.

Provides independent pinned-source oracle constants, temporary combat overlay
creation, template and zone validation, nonsecret oracle manifest generation,
and exact before-admission seeder manifest validation.
"""
import hashlib
import json
from pathlib import Path
import re
import shutil
import tomllib

OWNER = "01970000-0000-7000-8000-000000000020"
OBSERVER = "01970000-0000-7000-8000-000000000040"

OLD_PHASE2 = frozenset(("phase2-transfer", "phase2-transfer-observer", "phase2-transfer-missing-token"))
TOKEN_CROSSING = frozenset(("phase2-token-level20", "phase2-token-level40", "phase2-token-jump19-40"))
TOKEN_PROFILES = TOKEN_CROSSING | frozenset(("phase2-token-backfill",))
PHASE2 = OLD_PHASE2 | TOKEN_PROFILES

# Independent pinned source XP, death, and token constants agreed with native fixtures.
# No client runtime gameplay formulas are evaluated.
PINNED_ORACLES = {
    "phase2-token-level20": {
        "seed": {
            "level": 19,
            "xp": 835861,
            "tokens": [0, 0],
            "milestone_claimed_mask": 0,
            "seed_threshold_xp": 835862,
            "destination_level": 20,
            "threshold_xp": 835862,
            "xp_margin": 1,
        },
        "oracle": {
            "raw_xp": 10235,
            "human_kill_xp": 10746,
            "first_kill_level": 20,
            "first_kill_xp": 846607,
            "first_kill_tokens": [1, 0],
            "death_loss": 14329,
            "death_level": 19,
            "death_xp": 832278,
            "death_tokens": [1, 0],
            "respawn_level": 19,
            "respawn_xp": 832278,
            "respawn_tokens": [1, 0],
            "second_kill_level": 20,
            "second_kill_xp": 843024,
            "second_kill_tokens": [1, 0],
        },
        "transfer": {
            "transfers": [1],
            "first_transfer_class": 1,
            "first_transfer_consumed_tokens": [0, 0],
            "granted_skill_keys": 0,
        },
        "post_consumption": {
            "second_death_loss": 14329,
            "death_loss": 14329,
            "death_level": 19,
            "death_xp": 828695,
            "death_tokens": [0, 0],
            "respawn_level": 19,
            "respawn_xp": 828695,
            "respawn_tokens": [0, 0],
            "respawn_unchanged": True,
            "third_kill_xp_gain": 10746,
            "third_kill_level": 20,
            "third_kill_xp": 839441,
            "third_kill_tokens": [0, 0],
            "tokens": [0, 0],
            "current_class": 1,
            "class_id": 1,
        },
    },
    "phase2-token-level40": {
        "seed": {
            "level": 39,
            "xp": 15422928,
            "tokens": [1, 0],
            "milestone_claimed_mask": 1,
            "seed_threshold_xp": 15422929,
            "destination_level": 40,
            "threshold_xp": 15422929,
            "xp_margin": 1,
        },
        "oracle": {
            "raw_xp": 62751,
            "human_kill_xp": 65888,
            "first_kill_level": 40,
            "first_kill_xp": 15488816,
            "first_kill_tokens": [1, 1],
            "death_loss": 87851,
            "death_level": 39,
            "death_xp": 15400965,
            "death_tokens": [1, 1],
            "respawn_level": 39,
            "respawn_xp": 15400965,
            "respawn_tokens": [1, 1],
            "second_kill_level": 40,
            "second_kill_xp": 15466853,
            "second_kill_tokens": [1, 1],
        },
        "transfer": {
            "transfers": [1, 2],
            "first_transfer_class": 1,
            "first_transfer_consumed_tokens": [0, 1],
            "second_transfer_class": 2,
            "second_transfer_consumed_tokens": [0, 0],
            "granted_skill_keys": 0,
        },
        "post_consumption": {
            "second_death_loss": 87851,
            "death_loss": 87851,
            "death_level": 39,
            "death_xp": 15379002,
            "death_tokens": [0, 0],
            "respawn_level": 39,
            "respawn_xp": 15379002,
            "respawn_tokens": [0, 0],
            "respawn_unchanged": True,
            "third_kill_xp_gain": 65888,
            "third_kill_level": 40,
            "third_kill_xp": 15444890,
            "third_kill_tokens": [0, 0],
            "tokens": [0, 0],
            "current_class": 2,
            "class_id": 2,
        },
    },
    "phase2-token-jump19-40": {
        "seed": {
            "level": 19,
            "xp": 835861,
            "tokens": [0, 0],
            "milestone_claimed_mask": 0,
            "seed_threshold_xp": 835862,
            "destination_level": 40,
            "threshold_xp": 15422929,
            "xp_margin": 1,
        },
        "oracle": {
            "raw_xp": 13892446,
            "human_kill_xp": 14587068,
            "first_kill_level": 40,
            "first_kill_xp": 15422929,
            "first_kill_tokens": [1, 1],
        },
        "transfer": {
            "transfers": [1, 2],
            "first_transfer_class": 1,
            "first_transfer_consumed_tokens": [0, 1],
            "second_transfer_class": 2,
            "second_transfer_consumed_tokens": [0, 0],
            "granted_skill_keys": 0,
        },
    },
    "phase2-token-backfill": {
        "seed": {
            "level": 40,
            "xp": 15422929,
            "tokens": [0, 0],
            "milestone_claimed_mask": 0,
        },
        "visible": {
            "level": 40,
            "xp": 15422929,
            "tokens": [1, 1],
            "no_xp_level_change": True,
            "reconnect_never_duplicates": True,
        },
        "transfer": {
            "transfers": [1, 2],
            "first_transfer_class": 1,
            "first_transfer_consumed_tokens": [0, 1],
            "first_transfer_granted_skill_keys": 4,
            "first_transfer_granted_keys": ["l2.skill.1320", "l2.skill.1322", "l2.skill.194", "l2.skill.239"],
            "second_transfer_class": 2,
            "second_transfer_consumed_tokens": [0, 0],
            "second_transfer_granted_skill_keys": 0,
        },
    },
}


def digest(path: Path) -> str:
    with open(path, "rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def oracle_template_content(raw_xp: int) -> str:
    return f"""# Custom combat fixture template: Token Oracle.
# HP 1, tiny harmless attack, fast deterministic respawn and short corpse decay.
# Units match packages/data/npcs: milli-tiles, milli-tiles/tick, whole HP and seconds.
id = "token_oracle"
name = "Token Oracle"
attackable = true
level = 1
p_atk = "1.0"
p_def = "26.9"
max_hp = 1
max_mp = 20
attack_speed = 253
attack_range = 1500
move_speed = 400
collision_radius = 300
xp_reward = {raw_xp}
aggressive = false
aggro_range = 0
clan_help_range = 0
leash_radius = 15000
corpse_decay_ticks = 10
respawn_delay_secs = 1
respawn_random_secs = 0
"""


def sentinel_template_content() -> str:
    return """# Custom combat fixture template: Token Sentinel.
# High HP and audited lethal P.Atk, positive movement speed, passive.
# Attacking triggers normal retaliation/death, not scripted damage.
# Units match packages/data/npcs: milli-tiles, milli-tiles/tick, whole HP and seconds.
id = "token_sentinel"
name = "Token Sentinel"
attackable = true
level = 80
p_atk = "10000.0"
p_def = "1000.0"
max_hp = 1000000
max_mp = 20
attack_speed = 253
attack_range = 1500
move_speed = 400
collision_radius = 300
xp_reward = 0
aggressive = false
aggro_range = 0
clan_help_range = 0
leash_radius = 15000
corpse_decay_ticks = 100
respawn_delay_secs = 5
respawn_random_secs = 0
"""


def overlay_zone_content() -> str:
    return """# Temporary overlay test zone for Phase2 token combat crossings.
# Preserves Class Master semantics at (126,128) noncombat, safe point (126,126).
# Dedicated reward spawn at (126,125) and sentinel spawn at (130,126).
id = "test_zone"
zone_id = 1
name = "Test Zone"

[bounds]
min = [0, 0]
max = [256, 256]

[[npcs]]
name = "Class Master"
attackable = false
pos = [126, 128]
speed = 0

[safe_point]
pos = [126, 126]

[[spawn_slots]]
id = "reward_spawn"
template = "token_oracle"
home = [126, 125]
count = 1
respawn_delay_secs = 1
respawn_random_secs = 0

[[spawn_slots]]
id = "sentinel_spawn"
template = "token_sentinel"
home = [130, 126]
count = 1
respawn_delay_secs = 5
respawn_random_secs = 0
"""


def validate_template(path: Path) -> dict:
    stem = path.stem
    content = path.read_text(encoding="utf-8")
    data = tomllib.loads(content)
    allowed_fields = {
        "id", "name", "attackable", "level", "p_atk", "p_def", "max_hp", "max_mp",
        "attack_speed", "attack_range", "move_speed", "collision_radius", "xp_reward",
        "aggressive", "aggro_range", "clan_id", "clan_help_range", "leash_radius",
        "corpse_decay_ticks", "respawn_delay_secs", "respawn_random_secs"
    }
    unknown = set(data.keys()) - allowed_fields
    if unknown:
        raise ValueError(f"template {stem} contains unknown fields: {unknown}")

    if data.get("id") != stem:
        raise ValueError(f"template id {data.get('id')!r} must match filename stem {stem!r}")
    if not isinstance(data.get("name"), str) or not data["name"].strip():
        raise ValueError(f"template {stem} name must not be empty")
    if data.get("attackable") is not True:
        raise ValueError(f"template {stem} attackable must be true")

    lvl = data.get("level")
    if not isinstance(lvl, int) or not (1 <= lvl <= 65535):
        raise ValueError(f"template {stem} level must be 1..=65535")

    for field in ("max_hp", "attack_speed", "attack_range", "move_speed", "collision_radius", "leash_radius"):
        val = data.get(field)
        if not isinstance(val, int) or val <= 0:
            raise ValueError(f"template {stem} {field} must be positive, got {val}")

    for field in ("p_atk", "p_def"):
        val = data.get(field)
        if isinstance(val, (int, str)):
            s = str(val).strip()
            if not re.fullmatch(r"[0-9]+(\.[0-9]+)?", s):
                raise ValueError(f"template {stem} {field} invalid exact decimal or integer: {val}")
        else:
            raise ValueError(f"template {stem} {field} must be integer or exact decimal string: {val}")

    if not isinstance(data.get("aggressive"), bool):
        raise ValueError(f"template {stem} aggressive must be a bool")
    if data["aggressive"] and data.get("aggro_range", 0) <= 0:
        raise ValueError(f"template {stem} aggressive requires positive aggro_range")

    if data.get("leash_radius", 0) < data.get("aggro_range", 0):
        raise ValueError(f"template {stem} leash_radius must be at least aggro_range")

    if data.get("clan_id") == "":
        raise ValueError(f"template {stem} clan_id must not be empty string")
    if data.get("clan_help_range", 0) > 0 and not data.get("clan_id"):
        raise ValueError(f"template {stem} clan_help_range requires clan_id")

    delay = data.get("respawn_delay_secs")
    if not isinstance(delay, int) or not (1 <= delay <= 4294967295):
        raise ValueError(f"template {stem} respawn_delay_secs must be 1..=u32::MAX")
    random_s = data.get("respawn_random_secs")
    if not isinstance(random_s, int) or not (0 <= random_s <= 4294967294):
        raise ValueError(f"template {stem} respawn_random_secs must be 0..=u32::MAX-1")

    return data


def validate_zone(path: Path, npcs_dir: Path) -> dict:
    content = path.read_text(encoding="utf-8")
    data = tomllib.loads(content)
    if data.get("id") != "test_zone" or data.get("name") != "Test Zone":
        raise ValueError(f"zone {path.name} invalid id or name")
    bounds = data.get("bounds", {})
    if bounds.get("min") != [0, 0] or bounds.get("max") != [256, 256]:
        raise ValueError(f"zone {path.name} bounds must be 0..256")
    safe = data.get("safe_point", {})
    if safe.get("pos") != [126, 126]:
        raise ValueError(f"zone {path.name} safe point must be [126, 126]")

    npcs = data.get("npcs", [])
    if len(npcs) != 1 or npcs[0].get("name") != "Class Master":
        raise ValueError(f"overlay zone must contain exactly Class Master NPC")
    cm = npcs[0]
    if cm.get("attackable") is not False or cm.get("pos") != [126, 128] or cm.get("speed") != 0:
        raise ValueError(f"Class Master must be noncombat at [126, 128] with speed 0")

    slots = data.get("spawn_slots", [])
    slot_ids = {s.get("id") for s in slots}
    if slot_ids != {"reward_spawn", "sentinel_spawn"}:
        raise ValueError(f"overlay zone spawn slots must be reward_spawn and sentinel_spawn: {slot_ids}")

    for s in slots:
        template_name = s.get("template")
        template_file = npcs_dir / f"{template_name}.toml"
        if not template_file.is_file():
            raise ValueError(f"spawn slot {s.get('id')} references missing template {template_name}")
        home = s.get("home", [])
        if len(home) != 2 or not (0 <= home[0] <= 256 and 0 <= home[1] <= 256):
            raise ValueError(f"spawn slot {s.get('id')} home outside bounds: {home}")
        if s.get("count") != 1:
            raise ValueError(f"spawn slot {s.get('id')} count must be 1")

    return data


def create_overlay(dest_dir: Path, profile: str, repo_root: Path) -> dict:
    if profile not in TOKEN_CROSSING:
        raise ValueError(f"overlay creation requested for non-crossing profile: {profile}")

    src_data = repo_root / "packages/data"
    shutil.copytree(src_data, dest_dir, dirs_exist_ok=True)

    raw_xp = PINNED_ORACLES[profile]["oracle"]["raw_xp"]
    oracle_path = dest_dir / "npcs/token_oracle.toml"
    sentinel_path = dest_dir / "npcs/token_sentinel.toml"
    zone_path = dest_dir / "zones/test_zone.toml"

    oracle_path.write_text(oracle_template_content(raw_xp), encoding="utf-8")
    sentinel_path.write_text(sentinel_template_content(), encoding="utf-8")
    zone_path.write_text(overlay_zone_content(), encoding="utf-8")

    validate_template(oracle_path)
    validate_template(sentinel_path)
    validate_zone(zone_path, dest_dir / "npcs")

    return {
        "rules_dir": str(dest_dir),
        "zone_file": str(zone_path),
        "sha256": {
            "zone": digest(zone_path),
            "token_oracle": digest(oracle_path),
            "token_sentinel": digest(sentinel_path),
        },
    }


def build_oracle_manifest(profile: str, rules_dir: Path, zone_file: Path, repo_root: Path) -> dict:
    if profile not in TOKEN_PROFILES:
        raise ValueError(f"oracle manifest requested for unsupported profile: {profile}")

    is_overlay = profile in TOKEN_CROSSING
    tables_dir = repo_root / "packages/data/tables"
    manifest = {
        "schema_version": 1,
        "fixture": profile,
        "is_overlay": is_overlay,
        "provenance": {
            "description": (
                "Independent pinned-source oracle constants derived from High Five tables "
                "(experience.toml, penalties.toml, and human adaptable racial trait 1.05x multiplier). "
                "No client runtime gameplay formulas are evaluated."
            ),
            "source_tables_sha256": {
                "experience": digest(tables_dir / "experience.toml"),
                "penalties": digest(tables_dir / "penalties.toml"),
                "formulas": digest(tables_dir / "formulas.toml"),
            },
        },
        "constants": PINNED_ORACLES[profile],
        "data_sources": {
            "rules_dir": str(rules_dir),
            "zone_file": str(zone_file),
        },
        "sha256": {
            "zone": digest(zone_file),
        },
    }

    if is_overlay:
        npcs_dir = rules_dir / "npcs"
        manifest["sha256"]["token_oracle"] = digest(npcs_dir / "token_oracle.toml")
        manifest["sha256"]["token_sentinel"] = digest(npcs_dir / "token_sentinel.toml")

    return manifest


def _assert_strict_equal(actual: object, expected: object, path: str = "manifest") -> None:
    """Strictly assert type and value equivalence between actual and expected JSON structures.

    Rejects:
    - bools where ints are expected (e.g., False == 0, True == 1)
    - floats where ints are expected (e.g., 20.0 == 20)
    - ints/floats where bools are expected (e.g., 1 == True)
    - non-None where None is expected (e.g., 0, False, "")
    - non-str where str is expected
    - unexpected or missing keys in dictionaries
    - mismatched list lengths or element types
    """
    if expected is None:
        if actual is not None:
            raise ValueError(f"{path}: expected null (None), got {type(actual).__name__}: {actual!r}")
        return

    if type(expected) is bool:
        if type(actual) is not bool:
            raise ValueError(f"{path}: expected bool, got {type(actual).__name__}: {actual!r}")
        if actual != expected:
            raise ValueError(f"{path}: expected bool {expected!r}, got {actual!r}")
        return

    if type(expected) is int:
        if type(actual) is not int:
            raise ValueError(f"{path}: expected integer, got {type(actual).__name__}: {actual!r}")
        if actual != expected:
            raise ValueError(f"{path}: expected integer {expected!r}, got {actual!r}")
        return

    if type(expected) is float:
        if type(actual) is not float:
            raise ValueError(f"{path}: expected float, got {type(actual).__name__}: {actual!r}")
        if actual != expected:
            raise ValueError(f"{path}: expected float {expected!r}, got {actual!r}")
        return

    if type(expected) is str:
        if type(actual) is not str:
            raise ValueError(f"{path}: expected str, got {type(actual).__name__}: {actual!r}")
        if actual != expected:
            raise ValueError(f"{path}: expected str {expected!r}, got {actual!r}")
        return

    if type(expected) is list:
        if type(actual) is not list:
            raise ValueError(f"{path}: expected list, got {type(actual).__name__}: {actual!r}")
        if len(actual) != len(expected):
            raise ValueError(f"{path}: expected list of length {len(expected)}, got length {len(actual)}")
        for idx, (act_item, exp_item) in enumerate(zip(actual, expected)):
            _assert_strict_equal(act_item, exp_item, f"{path}[{idx}]")
        return

    if type(expected) is dict:
        if type(actual) is not dict:
            raise ValueError(f"{path}: expected dict, got {type(actual).__name__}: {actual!r}")
        actual_keys = set(actual.keys())
        expected_keys = set(expected.keys())
        if actual_keys != expected_keys:
            unexpected = actual_keys - expected_keys
            missing = expected_keys - actual_keys
            parts = []
            if unexpected:
                parts.append(f"unexpected keys: {sorted(unexpected)}")
            if missing:
                parts.append(f"missing keys: {sorted(missing)}")
            raise ValueError(f"{path}: key mismatch ({'; '.join(parts)})")
        for key in expected:
            _assert_strict_equal(actual[key], expected[key], f"{path}.{key}")
        return

    if type(actual) is not type(expected) or actual != expected:
        raise ValueError(f"{path}: expected {type(expected).__name__} {expected!r}, got {type(actual).__name__}: {actual!r}")


def validate_seed_manifest(seed: dict, selected: str, owner: str, observer: str | None = None) -> None:
    """Validate that the seeder manifest strictly conforms to the expected profile schema and values.

    Enforces exact JSON types recursively (no bool/int or float/int equivalence) for all
    seven profile manifests.
    """
    if not isinstance(seed, dict) or type(seed) is not dict:
        raise ValueError("seeder manifest must be a dict")
    if selected not in PHASE2:
        raise ValueError(f"unknown fixture: {selected}")
    if seed.get("fixture_enabled") is not True or seed.get("dry_run") is not False:
        raise ValueError("seeder manifest indicates fixture not enabled or dry_run")
    if seed.get("fixture") != selected:
        raise ValueError(f"seeder manifest fixture mismatch: {seed.get('fixture')} != {selected}")
    if selected == "phase2-transfer-observer" and observer is None:
        raise ValueError("phase2-transfer-observer requires an observer account")
    if selected != "phase2-transfer-observer" and "observer" in seed:
        raise ValueError("solo fixture unexpectedly provisioned an observer")

    if selected in OLD_PHASE2:
        missing = selected == "phase2-transfer-missing-token"
        expected_owner = {
            "account_id": owner,
            "character_id": OWNER,
            "level": 20 if missing else 40,
            "class_id": 0,
            "sex": "female",
            "tokens": [0, 0] if missing else [1, 1],
            "milestone_claimed_mask": 1 if missing else 3,
        }
        transfers = [1, 2] if selected in ("phase2-transfer", "phase2-transfer-observer") else []
    elif selected == "phase2-token-backfill":
        expected_owner = {
            "account_id": owner,
            "character_id": OWNER,
            "level": 40,
            "class_id": 0,
            "sex": "female",
            "tokens": [0, 0],
            "milestone_claimed_mask": 0,
            "race": "human",
            "xp": 15422929,
            "hp": None,
            "mp": None,
            "cp": 0,
            "sp": 0,
            "position": [126, 126],
        }
        transfers = []
    elif selected in TOKEN_CROSSING:
        constants = PINNED_ORACLES[selected]
        lvl = constants["seed"]["level"]
        tokens = constants["seed"]["tokens"]
        mask = constants["seed"]["milestone_claimed_mask"]
        xp = constants["seed"]["xp"]

        expected_owner = {
            "account_id": owner,
            "character_id": OWNER,
            "level": lvl,
            "class_id": 0,
            "sex": "female",
            "tokens": list(tokens),
            "milestone_claimed_mask": mask,
            "race": "human",
            "xp": xp,
            "hp": None,
            "mp": None,
            "cp": 0,
            "sp": 0,
            "position": [126, 126],
        }
        dest = constants["seed"]["destination_level"]
        raw_xp = constants["oracle"]["raw_xp"]
        kill_xp = constants["oracle"]["human_kill_xp"]
        thresh_xp = constants["seed"]["threshold_xp"]
        seed_thresh_xp = constants["seed"]["seed_threshold_xp"]

        expected_crossing = {
            "level": dest,
            "threshold_xp": thresh_xp,
            "seed_threshold_xp": seed_thresh_xp,
            "xp_margin": 1,
            "npc_raw_xp": raw_xp,
            "kill_xp": kill_xp,
            "expected_level": dest,
            "expected_xp": xp + kill_xp,
        }
        transfers = []
    else:
        raise ValueError(f"unknown fixture: {selected}")

    expected_manifest = {
        "fixture_enabled": True,
        "fixture": selected,
        "dry_run": False,
        "owner": expected_owner,
        "position": [126, 126],
        "transfers": transfers,
    }
    if selected == "phase2-transfer-observer":
        expected_manifest["observer"] = {
            "account_id": observer,
            "character_id": OBSERVER,
            "level": 1,
            "class_id": 0,
            "sex": "male",
        }
    if selected in TOKEN_CROSSING:
        expected_manifest["crossing"] = expected_crossing

    _assert_strict_equal(seed, expected_manifest, "seeder manifest")
