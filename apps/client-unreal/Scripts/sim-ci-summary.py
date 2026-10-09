#!/usr/bin/env python3
"""Render retained JUnit failure details and available coverage reports into a job summary."""
import json
from pathlib import Path
import sys
import xml.etree.ElementTree as ET

artifacts = Path(sys.argv[1])
print("\nSimulation failures (raw headless reports, including quarantines):\n")
for path in sorted(artifacts.rglob("*.xml")):
    if "video" in path.parts or path.name in ("group.xml", "suite.xml"):
        continue
    try:
        root = ET.parse(path).getroot()
    except (OSError, ET.ParseError):
        print(f"- Unreadable JUnit: `{path.relative_to(artifacts)}`")
        continue
    for case in root.findall(".//testcase"):
        for failure in list(case.findall("failure")) + list(case.findall("error")):
            detail = (failure.get("message", "") + " " + (failure.text or "")).strip()
            # Keep arbitrary client log text out of Markdown/HTML interpretation.
            detail = detail.replace("`", "'").replace("\n", " ").replace("<", "&lt;").replace("|", "\\|")
            print(f"- `{path.parent.name}` / `{case.get('name', '')}`: {detail[:1500]}")
for name in ("coverage.contract.json", "coverage.transitions.json"):
    paths = sorted(artifacts.rglob(name))
    print(f"\n{name}: {len(paths)} report(s) retained.\n")
    for path in paths:
        data = json.loads(path.read_text())
        print(f"`{path.relative_to(artifacts)}`\n")
        print("```json\n" + json.dumps(data, indent=2)[:12000] + "\n```\n")
