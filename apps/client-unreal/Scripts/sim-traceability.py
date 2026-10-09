#!/usr/bin/env python3
"""Check scenario declarations and reasoned scope exceptions against plan rows."""
import argparse
import json
from pathlib import Path
import re
import sys


def story_key(reference):
    if not isinstance(reference, str):
        return None
    match = re.fullmatch(r"\s*([0-9]+[a-z]?)\s+E?([0-9]+\.[0-9]+[a-z]?)\s*", reference)
    return (match[1], match[2]) if match else None


def read_exceptions(manifest, stories, covered, scenarios, errors):
    """Exceptions document scope; their evidence is not an execution/pass attestation."""
    if manifest is None:
        return {}
    try:
        data = json.loads(manifest.read_text())
    except (OSError, ValueError) as error:
        errors.append(f"{manifest.name}: cannot read exception manifest: {error}")
        return {}
    if (not isinstance(data, dict) or type(data.get("schema_version")) is not int
            or data["schema_version"] != 1 or not isinstance(data.get("exceptions"), list)):
        errors.append(f"{manifest.name}: expected schema_version 1 and an exceptions array")
        return {}
    result = {}
    seen = set()
    for index, entry in enumerate(data["exceptions"]):
        label = f"{manifest.name}: exceptions[{index}]"
        if not isinstance(entry, dict):
            errors.append(f"{label}: expected an object")
            continue
        key = story_key(entry.get("story"))
        if key not in stories:
            errors.append(f"{label}: unknown story reference {entry.get('story')!r}")
            continue
        if key in seen:
            errors.append(f"{label}: duplicate exception for {entry['story']}")
            continue
        seen.add(key)
        start = len(errors)
        if not isinstance(entry.get("reason"), str) or not entry["reason"].strip():
            errors.append(f"{label}: a nonempty reason is required")
        scope = entry.get("subscope")
        if "subscope" not in entry:
            if key in covered:
                errors.append(f"{label}: scenario tags for an exception require an explicit subscope")
        elif not isinstance(scope, dict):
            errors.append(f"{label}: subscope must be an object")
        else:
            if not isinstance(scope.get("scope"), str) or not scope["scope"].strip():
                errors.append(f"{label}: subscope needs a nonempty scope")
            names, automation = scope.get("scenarios", []), scope.get("automation", [])
            if (not isinstance(names, list) or not isinstance(automation, list)
                    or not (names or automation)):
                errors.append(f"{label}: subscope needs scenario or automation references")
            else:
                for name in names:
                    if (not isinstance(name, str) or name not in covered.get(key, [])
                            or not (scenarios / name).is_file()):
                        errors.append(f"{label}: unknown or untagged scenario reference {name!r}")
                for evidence in automation:
                    if not isinstance(evidence, dict):
                        errors.append(f"{label}: automation reference must have path and test")
                        continue
                    path, test = evidence.get("path"), evidence.get("test")
                    if not isinstance(path, str) or not isinstance(test, str) or not test.strip():
                        errors.append(f"{label}: automation reference needs path and test")
                        continue
                    source = scenarios.parent / path
                    if (Path(path).is_absolute() or not source.resolve().is_relative_to(scenarios.parent.resolve())
                            or not source.is_file()):
                        errors.append(f"{label}: unknown automation source {path!r}")
                        continue
                    declaration = r'IMPLEMENT_(?:SIMPLE|COMPLEX)_AUTOMATION_TEST\s*\(\s*\w+\s*,\s*"' + re.escape(test) + r'"'
                    if not re.search(declaration, source.read_text()):
                        errors.append(f"{label}: unknown automation test {test!r} in {path}")
            if isinstance(names, list) and set(covered.get(key, [])) - {n for n in names if isinstance(n, str)}:
                errors.append(f"{label}: subscope must list every scenario tagged for this story")
        if len(errors) == start:
            result[key] = entry
    return result


def check(plans, scenarios, manifest=None):
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
            key = story_key(reference)
            if key not in stories:
                errors.append(f"{scenario.name}: unknown story reference {reference.strip()!r}")
                continue
            covered.setdefault(key, []).append(scenario.name)
    exceptions = read_exceptions(manifest, stories, covered, scenarios, errors)
    required = {key: value for key, value in stories.items() if value["complete"] and value["visible"]}
    full = (set(required) & set(covered)) - set(exceptions)
    exempt = set(required) & set(exceptions)
    missing = sorted(set(required) - full - exempt)
    output = [f"Simulation traceability: {len(required)} completed client-visible stories: "
              f"{len(full)} covered, {len(exempt)} exception, {len(missing)} missing."]
    for key, entry in sorted(exceptions.items()):
        suffix = "" if key in required else " (outside completed client-visible totals)"
        output.append(f"EXCEPTION {key[0]} E{key[1]}{suffix}: {entry['reason'].strip()}")
        if "subscope" in entry:
            scope = entry["subscope"]
            output.append(f"  SUBSCOPE {scope['scope'].strip()}")
            for name in scope.get("scenarios", []):
                output.append(f"  SCENARIO {name}")
            for evidence in scope.get("automation", []):
                output.append(f"  AUTOMATION {evidence['test']} ({evidence['path']})")
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
    parser.add_argument("--manifest", type=Path,
                        default=project / "Scenarios/traceability-exceptions.json")
    args = parser.parse_args()
    output, failed = check(args.plans, args.scenarios, args.manifest)
    print("\n".join(output))
    return failed


if __name__ == "__main__":
    sys.exit(main())
