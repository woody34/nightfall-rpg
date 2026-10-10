#!/usr/bin/env python3
"""Check every generated data artifact without writing outputs."""

import argparse
from pathlib import Path
import subprocess
import sys


SCRIPTS = Path(__file__).resolve().parent


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
        ("oracle.py", []),
    ]
    failures = []
    for script, options in checks:
        label = " ".join([script, *options, "--check"])
        print(f"Checking {label}", flush=True)
        result = subprocess.run([sys.executable, "-B", str(SCRIPTS / script), *options, "--check"])
        if result.returncode:
            failures.append(label)
    if failures:
        print("Failed checks: " + ", ".join(failures), file=sys.stderr)
        return 1
    print(f"All {len(checks)} generator checks passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
