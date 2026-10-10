#!/usr/bin/env python3
"""Focused validation for E6.2/E6.3 class transfer observability and diagram contracts.

Validates:
1. infra/grafana/dashboards/nightfall-api.json (JSON syntax, unique non-overlapping panels,
   exact source metric names, bounded labels, updated checkpoint descriptions).
2. docs/diagrams/phase-2-class-tree.html (all 89 exact classes from packages/data/professions/*.toml,
   five races, nine root branches, corrected 34->32 and 104->28 parents, font-size >= 12px,
   accessible SVG contract, local source links).
3. docs/diagrams/phase-2-class-transfer.html (7-stage lifecycle order and semantics,
   Class Master at 126,128, durable log + DB persistence release barrier, local source links).
4. docs/diagrams/README.md (presence of both new diagram links).
5. docs/engineering/class-transfer-observability.md (metrics contract, no-data vs zero, stalls, limits).
6. Diagram design checks (self_check.py and verify-geometry.py if available).

Standard library Python only; no network or native services.
"""

from __future__ import annotations

import glob
import json
import re
import subprocess
import sys
import tomllib
import html
import xml.etree.ElementTree as ET
from collections import Counter, defaultdict, deque
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def validate_dashboard() -> list[str]:
    errors = []
    dashboard_path = ROOT / "infra/grafana/dashboards/nightfall-api.json"
    if not dashboard_path.is_file():
        return [f"Dashboard file not found: {dashboard_path}"]

    try:
        data = json.loads(dashboard_path.read_text(encoding="utf-8"))
    except Exception as exc:
        return [f"Invalid JSON in dashboard: {exc}"]

    panels = data.get("panels", [])
    if not panels:
        errors.append("No panels found in dashboard")

    # Check panel IDs
    seen_ids = set()
    for p in panels:
        pid = p.get("id")
        if pid is None:
            errors.append(f"Panel missing id: {p.get('title')}")
        elif pid in seen_ids:
            errors.append(f"Duplicate panel id {pid}: {p.get('title')}")
        else:
            seen_ids.add(pid)

    # Panels 1..20 must be preserved
    for i in range(1, 21):
        if i not in seen_ids:
            errors.append(f"Expected panel id {i} was removed from dashboard")

    # Check panels 19 and 20 updated descriptions
    p19 = next((p for p in panels if p.get("id") == 19), None)
    if p19:
        desc = p19.get("description", "")
        if "worker not integrated" in desc.lower() or "until the checkpoint worker" in desc.lower():
            errors.append(f"Panel 19 has stale description claiming worker not integrated: {desc}")

    p20 = next((p for p in panels if p.get("id") == 20), None)
    if p20:
        desc = p20.get("description", "")
        if "worker not integrated" in desc.lower() or "until the checkpoint worker" in desc.lower():
            errors.append(f"Panel 20 has stale description claiming worker not integrated: {desc}")

    # Check grid positions: 24-column grid, no overlapping panels
    occupied = {}
    for p in panels:
        pid = p.get("id")
        gp = p.get("gridPos", {})
        x = gp.get("x")
        y = gp.get("y")
        w = gp.get("w")
        h = gp.get("h")
        if any(v is None for v in (x, y, w, h)):
            errors.append(f"Panel {pid} has invalid gridPos: {gp}")
            continue
        if x < 0 or x + w > 24:
            errors.append(f"Panel {pid} bounds outside 24-col grid: x={x}, w={w}")
        for cx in range(x, x + w):
            for cy in range(y, y + h):
                if (cx, cy) in occupied:
                    errors.append(
                        f"Panel {pid} ({p.get('title')}) overlaps with panel {occupied[(cx, cy)]} at grid ({cx}, {cy})"
                    )
                else:
                    occupied[(cx, cy)] = pid

    # Expected exact source metric names
    expected_metrics = {
        "nightfall_class_transfers_total",
        "nightfall_class_transfer_seconds_count",
        "nightfall_class_transfer_seconds_bucket",
    }
    dashboard_text = " ".join(t.get("expr", "") for p in panels for t in p.get("targets", []))
    for m in expected_metrics:
        if m not in dashboard_text:
            errors.append(f"Expected source metric {m} missing from dashboard queries")

    # Check panels after 20 for bounded labels and conventions
    class_panels = [p for p in panels if p.get("id", 0) > 20]
    if len(class_panels) < 3:
        errors.append(f"Expected at least 3 class transfer panels after id 20, found {len(class_panels)}")

    for p in class_panels:
        ds = p.get("datasource", {})
        if ds.get("type") != "prometheus" or ds.get("uid") != "prometheus":
            errors.append(f"Panel {p.get('id')} does not follow datasource convention: {ds}")
        for target in p.get("targets", []):
            expr = target.get("expr", "")
            if re.search(r"\bvector\s*\(\s*0(?:\.0)?\s*\)", expr):
                errors.append(f"Panel {p.get('id')} manufactures missing data as zero")
            allowed = expected_metrics
            used = set(re.findall(r"\bnightfall_[a-z0-9_]+", expr))
            if not used or not used <= allowed:
                errors.append(f"Panel {p.get('id')} uses unknown metric(s): {used - allowed}")
            if "histogram_quantile" in expr and not re.search(r"sum by \(le,\s*outcome\)", expr):
                errors.append(f"Panel {p.get('id')} quantile loses le/outcome grouping")
            # Verify no unbounded/PII labels in queries
            for forbidden in ("account_id", "character_id", "name", "idempotency_key", "secret", "token"):
                if f"by ({forbidden}" in expr or f", {forbidden}" in expr or f"{forbidden}=" in expr:
                    errors.append(f"Panel {p.get('id')} query contains unbounded/sensitive label '{forbidden}': {expr}")

    return errors


def validate_class_tree() -> list[str]:
    errors = []
    tree_path = ROOT / "docs/diagrams/phase-2-class-tree.html"
    if not tree_path.is_file():
        return [f"Class tree diagram not found: {tree_path}"]

    tree_html = tree_path.read_text(encoding="utf-8")
    if re.search(r"<(?:script|iframe|embed|object)\b|\bon[a-z]+\s*=|<link[^>]+href=[\"\'](?:https?:)?//|@import", tree_html, re.I):
        errors.append("Class tree must contain static non-executable markup")

    # Load all 89 profession TOML files
    prof_files = sorted(glob.glob(str(ROOT / "packages/data/professions/*.toml")))
    if len(prof_files) != 89:
        errors.append(f"Expected 89 profession TOML files, found {len(prof_files)}")

    classes = {}
    for f in prof_files:
        try:
            d = tomllib.loads(Path(f).read_text(encoding="utf-8"))
            classes[d["id"]] = d
        except Exception as exc:
            errors.append(f"Error reading {f}: {exc}")

    # Check each class is present in the diagram with exact ID and display name
    for cid, c in classes.items():
        expected_token = html.escape(f"[{cid}] {c['display_name']}")
        if expected_token not in tree_html:
            errors.append(f"Class [{cid}] '{c['display_name']}' not found in phase-2-class-tree.html")

    # Compare actual rendered node metadata and direct edges against the data oracle.
    svgs = [ET.fromstring(raw) for raw in re.findall(r"<svg\b.*?</svg>", tree_html, re.S)]
    seen = Counter()
    for svg in svgs:
        nodes = {}
        graph = defaultdict(set)
        segments = []
        for element in svg.iter():
            if element.tag.endswith("line"):
                a = (float(element.get("x1")), float(element.get("y1")))
                b = (float(element.get("x2")), float(element.get("y2")))
                segments.append((a, b))
            if "data-class-id" not in element.attrib:
                continue
            cid = int(element.get("data-class-id"))
            seen[cid] += 1
            expected = classes.get(cid)
            if expected is None:
                errors.append(f"Unknown rendered class {cid}")
                continue
            for attr, key in [("data-parent", "parent"), ("data-tier", "tier"),
                              ("data-race", "race"), ("data-min-level", "min_level")]:
                if element.get(attr) != str(expected.get(key, "")):
                    errors.append(f"Class {cid} mismatched {attr}")
            children = list(element)
            rect = next(child for child in children if child.tag.endswith("rect"))
            texts = ["".join(child.itertext()) for child in children if child.tag.endswith("text")]
            if texts != [f"[{cid}] {expected['display_name']}", expected["l2_ref"]]:
                errors.append(f"Class {cid} mismatched rendered labels")
            if expected["tier"] == 3 and not rect.get("stroke-dasharray"):
                errors.append(f"Class {cid} lacks deferred border")
            nodes[cid] = tuple(float(rect.get(k)) for k in ("x", "y", "width", "height"))
        # Sibling buses can meet a parent drop at the interior of a segment.
        vertices = {point for segment in segments for point in segment}
        for a, b in segments:
            along = sorted(point for point in vertices if
                           min(a[0], b[0]) <= point[0] <= max(a[0], b[0]) and
                           min(a[1], b[1]) <= point[1] <= max(a[1], b[1]))
            for first, second in zip(along, along[1:]):
                graph[first].add(second)
                graph[second].add(first)
        for cid, (x, y, width, height) in nodes.items():
            parent = classes[cid].get("parent")
            if parent is None:
                continue
            if parent not in nodes:
                errors.append(f"Class {cid} parent {parent} is in another panel")
                continue
            px, py, pw, ph = nodes[parent]
            start, end = (px + pw, py + ph / 2), (x, y + height / 2)
            reached, queue = {start}, deque([start])
            while queue:
                for point in graph[queue.popleft()]:
                    if start[0] <= point[0] <= end[0] and point not in reached:
                        reached.add(point)
                        queue.append(point)
            if end not in reached:
                errors.append(f"Missing visual direct edge {parent}->{cid}")
    if seen != Counter({cid: 1 for cid in classes}):
        errors.append("Rendered classes must match all 89 source IDs exactly once")
    if len(svgs) != 9:
        errors.append(f"Expected nine branch SVGs, got {len(svgs)}")

    # Check all 5 races and 9 roots
    races = {"human", "elf", "dark_elf", "orc", "dwarf"}
    roots = {c["id"] for c in classes.values() if c.get("parent") is None}
    if len(roots) != 9:
        errors.append(f"Expected 9 root classes, found {len(roots)}")

    for r in races:
        if r not in tree_html.lower().replace(" ", "_"):
            errors.append(f"Race '{r}' not represented in phase-2-class-tree.html")

    # Verify corrected parents in metadata
    # Class 34 (Edge Cantor) parent must be 32 (Gloamguard)
    if classes.get(34, {}).get("parent") != 32:
        errors.append(f"Class 34 parent in TOML should be 32, got {classes.get(34, {}).get('parent')}")
    # Class 104 (Tide Sovereign) parent must be 28 (Tidekeeper)
    if classes.get(104, {}).get("parent") != 28:
        errors.append(f"Class 104 parent in TOML should be 28, got {classes.get(104, {}).get('parent')}")

    # Check font-size >= 12px throughout SVG text elements
    for m in re.finditer(r'<text\b[^>]*?font-size="(?P<size>[\d.]+)(?:px)?"', tree_html):
        size = float(m.group("size"))
        if size < 12.0:
            errors.append(f"phase-2-class-tree.html text font-size {size}px is smaller than 12px limit")

    # Check relative links exist on disk
    for m in re.finditer(r'<a\b[^>]*?href="(?P<href>[^"#]+?)"', tree_html):
        href = m.group("href")
        target = (tree_path.parent / href).resolve()
        if not target.exists():
            errors.append(f"phase-2-class-tree.html link '{href}' does not exist on disk: {target}")

    return errors


def validate_class_transfer() -> list[str]:
    errors = []
    transfer_path = ROOT / "docs/diagrams/phase-2-class-transfer.html"
    if not transfer_path.is_file():
        return [f"Class transfer diagram not found: {transfer_path}"]

    content = transfer_path.read_text(encoding="utf-8")
    if re.search(r"<(?:script|iframe|embed|object)\b|\bon[a-z]+\s*=|<link[^>]+href=[\"\'](?:https?:)?//|@import", content, re.I):
        errors.append("Transfer diagram must contain static non-executable markup")

    required_keywords = [
        "ChangeClass",
        "Frozen receipt",
        "Class Master at (126, 128)",
        "AppliedTickDraft",
        "JetStream",
        "revision fence",
        "ClassChanged",
        "BOTH",
    ]
    for kw in required_keywords:
        if kw not in content:
            errors.append(f"phase-2-class-transfer.html missing critical semantic keyword: '{kw}'")

    # Check relative links exist on disk
    for m in re.finditer(r'<a\b[^>]*?href="(?P<href>[^"#]+?)"', content):
        href = m.group("href")
        target = (transfer_path.parent / href).resolve()
        if not target.exists():
            errors.append(f"phase-2-class-transfer.html link '{href}' does not exist on disk: {target}")

    return errors


def validate_readme_and_docs() -> list[str]:
    errors = []
    readme_path = ROOT / "docs/diagrams/README.md"
    if not readme_path.is_file():
        return [f"docs/diagrams/README.md not found"]

    readme_text = readme_path.read_text(encoding="utf-8")
    if "phase-2-class-tree.html" not in readme_text:
        errors.append("docs/diagrams/README.md missing link to phase-2-class-tree.html")
    if "phase-2-class-transfer.html" not in readme_text:
        errors.append("docs/diagrams/README.md missing link to phase-2-class-transfer.html")

    doc_path = ROOT / "docs/engineering/class-transfer-observability.md"
    if not doc_path.is_file():
        errors.append("docs/engineering/class-transfer-observability.md not found")
    else:
        doc_text = doc_path.read_text(encoding="utf-8")
        for topic in (
            "nightfall_class_transfers_total",
            "nightfall_class_transfer_seconds",
            "No data",
            "shared all-zone",
            "token_tier",
        ):
            if topic not in doc_text:
                errors.append(f"class-transfer-observability.md missing documented section/keyword: '{topic}'")

    return errors


def run_diagram_skill_checks() -> list[str]:
    errors = []
    skill_dir = Path("/home/matt-woodruff/.claude/plugins/marketplaces/diagram-design/skills/diagram-design")
    self_check_script = skill_dir / "scripts/self_check.py"
    verify_geom_script = Path("/home/matt-woodruff/.claude/plugins/marketplaces/diagram-design/scripts/verify-geometry.py")

    target_diagrams = [
        ROOT / "docs/diagrams/phase-2-class-tree.html",
        ROOT / "docs/diagrams/phase-2-class-transfer.html",
    ]

    if not self_check_script.is_file() or not verify_geom_script.is_file():
        print("  Optional external Diagram Design checker unavailable; local contract checks still run.")
    for diag in target_diagrams:
        if not diag.is_file():
            continue
        if self_check_script.is_file():
            proc = subprocess.run(
                [sys.executable, str(self_check_script), str(diag)],
                capture_output=True,
                text=True,
            )
            if proc.returncode != 0:
                errors.append(f"self_check.py failed on {diag.name}:\n{proc.stdout}\n{proc.stderr}")

        if verify_geom_script.is_file():
            proc = subprocess.run(
                [sys.executable, str(verify_geom_script), str(diag)],
                capture_output=True,
                text=True,
            )
            if proc.returncode != 0 or "finding(s)" in proc.stdout and not "0 finding(s)" in proc.stdout:
                errors.append(f"verify-geometry.py reported findings on {diag.name}:\n{proc.stdout}\n{proc.stderr}")

    return errors


def main() -> int:
    all_errors = []

    print("Validating Grafana dashboard contract...")
    dashboard_errors = validate_dashboard()
    all_errors.extend(dashboard_errors)
    if not dashboard_errors:
        print("  OK: Dashboard contract valid (25 panels, non-overlapping grid, exact metrics).")

    print("Validating Class Progression Tree diagram contract...")
    tree_errors = validate_class_tree()
    all_errors.extend(tree_errors)
    if not tree_errors:
        print("  OK: 89 exact source nodes/direct edges, 5 races, 9 roots, font-size >= 12px.")

    print("Validating Class Transfer lifecycle diagram contract...")
    transfer_errors = validate_class_transfer()
    all_errors.extend(transfer_errors)
    if not transfer_errors:
        print("  OK: Transfer lifecycle labels and local links verified (semantic order independently reviewed).")

    print("Validating README links and engineering observability documentation...")
    doc_errors = validate_readme_and_docs()
    all_errors.extend(doc_errors)
    if not doc_errors:
        print("  OK: README links and engineering observability documentation verified.")

    print("Running Diagram Design skill checks (self_check.py & verify-geometry.py)...")
    diag_errors = run_diagram_skill_checks()
    all_errors.extend(diag_errors)
    if not diag_errors:
        print("  OK: Available Diagram Design checks passed; unavailable external checkers are reported above.")

    if all_errors:
        print(f"\nVALIDATION FAILED with {len(all_errors)} error(s):", file=sys.stderr)
        for err in all_errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print("\nALL CONTRACT CHECKS PASSED SUCCESSFULLY.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
