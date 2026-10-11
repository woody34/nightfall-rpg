#!/usr/bin/env python3
"""Check every generated data artifact without writing outputs."""

import argparse
import hashlib
from pathlib import Path
import subprocess
import sys
import tomllib


SCRIPTS = Path(__file__).resolve().parent
DATA = SCRIPTS.parent


def check_phase3_zone(data: Path = DATA) -> str:
    """Validate hand-written V5 geometry and return its separate byte SHA-256."""
    zone = tomllib.loads((data / "zones/test_zone.toml").read_text())
    raw = (data / "zones/test_zone_phase3.toml").read_bytes()
    phase3 = tomllib.loads(raw.decode("utf-8"))
    if set(phase3) != {"schema_version", "id", "peace_zone", "provenance"}:
        raise ValueError("unexpected Phase 3 zone fields")
    if type(phase3["schema_version"]) is not int or phase3["schema_version"] != 1:
        raise ValueError("unsupported Phase 3 zone schema")
    if phase3["id"] != zone["id"]:
        raise ValueError("Phase 3 zone id must match legacy zone")
    if phase3["provenance"].get("kind") != "Nightfall configuration":
        raise ValueError("peace geometry must cite Nightfall configuration")
    peace = phase3["peace_zone"]
    if set(peace) != {"shape", "centre", "half_width_millitiles", "inclusive"}:
        raise ValueError("unexpected peace geometry fields")
    if peace["shape"] != "square" or peace["inclusive"] is not True:
        raise ValueError("peace geometry must be an inclusive square")
    centre = peace["centre"]
    if (not isinstance(centre, list) or len(centre) != 2
            or any(type(coordinate) is not int for coordinate in centre)):
        raise ValueError("peace centre must contain two integer whole-tile coordinates")
    if centre != zone["safe_point"]["pos"]:
        raise ValueError("peace centre must equal safe_point")
    width = peace["half_width_millitiles"]
    if type(width) is not int or width <= 0:
        raise ValueError("peace half-width must be a positive integer in milli-tiles")
    lower = [coordinate * 1000 - width for coordinate in centre]
    upper = [coordinate * 1000 + width for coordinate in centre]
    if any(lower[axis] < zone["bounds"]["min"][axis] * 1000
           or upper[axis] > zone["bounds"]["max"][axis] * 1000 for axis in range(2)):
        raise ValueError("peace square must lie inside zone bounds")
    masters = [npc for npc in zone["npcs"] if npc["name"] == "Class Master"]
    if len(masters) != 1 or any(
            not lower[axis] <= masters[0]["pos"][axis] * 1000 <= upper[axis]
            for axis in range(2)):
        raise ValueError("peace square must contain the Class Master (inclusive)")
    return hashlib.sha256(raw).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--datapack", type=Path, required=True)
    parser.add_argument("--game", type=Path, required=True)
    args = parser.parse_args()
    sources = ["--datapack", str(args.datapack), "--game", str(args.game)]
    checks = [
        ("gen_tables.py", sources),
        ("gen_classes.py", sources[:2]),
        ("gen_class_phase3.py", sources),
        ("gen_skills.py", sources[:2]),
        ("skill_oracle.py", []),
        ("skill_oracle.py", ["--class-phase3"]),
        ("starter_weapon_oracle.py", []),
        ("npc_phase3/skill_oracle.py", []),
        ("oracle.py", []),
    ]
    failures = []
    print("Checking hand-written Phase 3 zone geometry", flush=True)
    try:
        digest = check_phase3_zone()
        print(f"Phase 3 zone SHA-256: {digest}", flush=True)
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
        print(f"Invalid Phase 3 zone geometry: {error}", file=sys.stderr)
        failures.append("test_zone_phase3.toml")
    for script, options in checks:
        label = " ".join([script, *options, "--check"])
        print(f"Checking {label}", flush=True)
        result = subprocess.run([sys.executable, "-B", str(SCRIPTS / script), *options, "--check"])
        if result.returncode:
            failures.append(label)
    if failures:
        print("Failed checks: " + ", ".join(failures), file=sys.stderr)
        return 1
    print(f"All {len(checks)} generator checks and Phase 3 zone validation passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
