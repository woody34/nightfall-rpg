#!/usr/bin/env python3
"""Check scenario coverage references against completed client-visible plan rows."""
import argparse
from pathlib import Path
import re
import sys


def check(plans, scenarios):
    stories = {}
    errors = []
    covered = {}
    for plan in sorted(plans.glob("phase-*.md")):
        phase = re.match(r"phase-([0-9]+[a-z]?)-", plan.name)
        if not phase:
            continue
        in_stories = False
        for lineno, line in enumerate(plan.read_text().splitlines(), 1):
            if line.startswith("## "):
                in_stories = bool(re.match(r"## 4\. Epics", line))
            if not in_stories:
                continue
            row = re.match(r"\|\s*(✅\s*)?([0-9]+\.[0-9]+[a-z]?)\s+([^|]+)\|", line)
            if not row:
                continue
            key = (phase[1], row[2])
            stories[key] = {"complete": bool(row[1]), "visible": "[client-visible]" in line,
                            "path": plan, "line": lineno, "title": row[3].strip()}
    for scenario in sorted(scenarios.glob("*.nfs")):
        lines = scenario.read_text().splitlines()
        first = next((line for line in lines if line.strip()), "")
        header = re.fullmatch(r"#\s*covers:\s*(.+)", first)
        if not header:
            errors.append(f"{scenario.name}: first nonempty line must be # covers: <phase> <story>, ...")
            continue
        for reference in header[1].split(","):
            match = re.fullmatch(r"\s*([0-9]+[a-z]?)\s+E?([0-9]+\.[0-9]+[a-z]?)\s*", reference)
            if not match or (match[1], match[2]) not in stories:
                errors.append(f"{scenario.name}: unknown story reference {reference.strip()!r}")
                continue
            covered.setdefault((match[1], match[2]), []).append(scenario.name)
    required = {key: value for key, value in stories.items() if value["complete"] and value["visible"]}
    missing = sorted(set(required) - set(covered))
    output = [f"Simulation traceability: {len(required) - len(missing)}/{len(required)} completed client-visible stories covered."]
    for key in missing:
        info = required[key]
        output.append(f"UNCOVERED {key[0]} E{key[1]} {info['title']} ({info['path'].name}:{info['line']})")
    for error in errors:
        output.append(f"ERROR {error}")
    if not required:
        errors.append("no completed client-visible story tags found")
        output.append("ERROR no completed client-visible story tags found; refusing vacuous success")
    return output, bool(missing or errors)


def main():
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plans", type=Path, default=project.parents[1] / "docs/plans")
    parser.add_argument("--scenarios", type=Path, default=project / "Scenarios")
    args = parser.parse_args()
    output, failed = check(args.plans, args.scenarios)
    print("\n".join(output))
    return failed


if __name__ == "__main__":
    sys.exit(main())
