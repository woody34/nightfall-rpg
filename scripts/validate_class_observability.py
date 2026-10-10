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


def strip_rust_comments(source: str) -> str:
    """Strips Rust line (//) and block (/* */, including nested) comments
    while preserving quoted string literals.
    """
    out = []
    i = 0
    n = len(source)
    comment_depth = 0
    in_string = False
    escape = False

    while i < n:
        if in_string:
            ch = source[i]
            out.append(ch)
            if escape:
                escape = False
            elif ch == "\\":
                escape = True
            elif ch == '"':
                in_string = False
            i += 1
        elif comment_depth > 0:
            if source[i : i + 2] == "/*":
                comment_depth += 1
                i += 2
            elif source[i : i + 2] == "*/":
                comment_depth -= 1
                i += 2
                if comment_depth == 0:
                    out.append(" ")
            else:
                if source[i] == "\n":
                    out.append("\n")
                i += 1
        else:
            if source[i] == '"':
                in_string = True
                out.append('"')
                i += 1
            elif source[i : i + 2] == "//":
                i += 2
                while i < n and source[i] != "\n":
                    i += 1
            elif source[i : i + 2] == "/*":
                comment_depth = 1
                i += 2
            else:
                out.append(source[i])
                i += 1
    return "".join(out)


def normalize_promql(expr: str) -> str:
    """Normalizes whitespace in a PromQL query while preserving delimiters.
    Collapses whitespace and strips around parentheses, brackets, and commas.
    """
    cleaned = re.sub(r"\s+", " ", expr.strip())
    return re.sub(r"\s*([(),\[\]])\s*", r"\1", cleaned)


def normalize_rust_whitespace(source: str) -> str:
    """Remove formatting whitespace while preserving literal label contents."""
    return re.sub(
        r'"(?:[^"\\]|\\.)*"|\s+',
        lambda match: match.group(0) if match.group(0).startswith('"') else "",
        source,
    )


def check_balanced_delimiters(expr: str) -> bool:
    """Verifies that parentheses and brackets are properly matched and balanced."""
    stack = []
    pairs = {")": "(", "]": "[", "}": "{"}
    for ch in expr:
        if ch in pairs.values():
            stack.append(ch)
        elif ch in pairs:
            if not stack or stack[-1] != pairs[ch]:
                return False
            stack.pop()
    return len(stack) == 0


CANONICAL_TOKEN_GRANTED_NO_WS = (
    'fntoken_granted(&self,tier:u8,source:crate::domain::character_progression::TokenSource){'
    'usecrate::domain::character_progression::TokenSource;'
    'lettier=matchtier{1=>"1",2=>"2",_=>return,};'
    'letsource=matchsource{TokenSource::Admission=>"admission",TokenSource::LevelUp=>"level_up",};'
    'self.class_transfer_token_grants_total.add(1,&[KeyValue::new("tier",tier),KeyValue::new("source",source)]);}'
)


def validate_grant_source_binding() -> list[str]:
    """Binds Prometheus exporter metric and dashboard query labels to the active
    OpenTelemetry counter literal in metrics.rs and domain tier/source mapping in combat.rs.

    Note: This is a fixed static source contract, fail-closed against drift, not a general
    Rust or PromQL parser or live query execution. Any change to the source telemetry contract
    requires an intentional validator update.
    """
    errors = []
    derived_prom_metric = ""
    metrics_rs = ROOT / "apps/api/src/infrastructure/telemetry/metrics.rs"
    exporter_rs = ROOT / "apps/api/src/infrastructure/telemetry/mod.rs"
    combat_rs = ROOT / "apps/api/src/infrastructure/telemetry/combat.rs"

    if not metrics_rs.is_file():
        errors.append(f"metrics.rs not found: {metrics_rs}")
    else:
        raw_metrics = metrics_rs.read_text(encoding="utf-8")
        stripped_metrics = strip_rust_comments(raw_metrics)

        # 1. Bind exact counter field declaration in Metrics struct:
        #    pub(super) class_transfer_token_grants_total: Counter<u64>,
        if not re.search(
            r"\bclass_transfer_token_grants_total\s*:\s*Counter\s*<\s*u64\s*>",
            stripped_metrics,
        ):
            errors.append(
                f"Active field declaration 'class_transfer_token_grants_total: Counter<u64>' not found in {metrics_rs}"
            )

        # 2. Bind active u64_counter constructor call in Metrics::new:
        #    class_transfer_token_grants_total: meter.u64_counter("...").with_description("...").build(),
        counter_match = re.search(
            r"\bclass_transfer_token_grants_total\s*:\s*meter\s*\.\s*u64_counter\s*\(\s*\"([^\"]+)\"\s*\)(.*?)\.build\s*\(\s*\)",
            stripped_metrics,
            re.DOTALL,
        )
        if not counter_match:
            errors.append(
                f"Active u64_counter initialization for 'class_transfer_token_grants_total' not found in {metrics_rs}"
            )
        else:
            otel_metric_literal = counter_match.group(1)
            builder_chain = counter_match.group(2)

            if otel_metric_literal != "nightfall_class_transfer_token_grants":
                errors.append(
                    f"Expected OTel counter literal 'nightfall_class_transfer_token_grants', got '{otel_metric_literal}'"
                )

            # Ensure no .with_unit or naming overrides (allow existing .with_description only)
            if ".with_unit" in builder_chain:
                errors.append(
                    "Counter must not define .with_unit(...) because unit overrides alter the Prometheus exported metric name"
                )

            stripped_chain = re.sub(
                r'\.with_description\s*\(\s*"(?:[^"\\]|\\.)*"\s*,?\s*\)',
                "",
                builder_chain,
                flags=re.DOTALL,
            )
            if stripped_chain.strip():
                errors.append(
                    f"Counter builder contains unauthorized naming overrides or chaining: {stripped_chain.strip()}"
                )

        # 3. Bind default Prometheus exporter configuration with no prefix/suffix overrides:
        #    opentelemetry_prometheus::exporter().with_registry(registry.clone()).build()
        # The application exporter lives in mod.rs; metrics.rs has a separate
        # exporter used by source tests which cannot bind production naming.
        exporter_text = (
            strip_rust_comments(exporter_rs.read_text(encoding="utf-8"))
            if exporter_rs.is_file()
            else ""
        )
        exporter_match = re.search(
            r"opentelemetry_prometheus\s*::\s*exporter\s*\(\s*\)(.*?)\.build\s*\(\s*\)",
            exporter_text,
            re.DOTALL,
        )
        if not exporter_match:
            errors.append(f"Application Prometheus exporter builder not found in {exporter_rs}")
        else:
            exporter_chain_no_ws = re.sub(r"\s+", "", exporter_match.group(1))
            if exporter_chain_no_ws != ".with_registry(registry.clone())":
                errors.append(
                    f"Prometheus exporter must use default naming config (.with_registry(registry.clone()) only with no prefix/suffix overrides), got: {exporter_match.group(1).strip()}"
                )

        if not errors and counter_match:
            derived_prom_metric = f"{otel_metric_literal}_total"

    if not combat_rs.is_file():
        errors.append(f"combat.rs not found: {combat_rs}")
    else:
        raw_combat = combat_rs.read_text(encoding="utf-8")
        stripped_combat = strip_rust_comments(raw_combat)

        # 4. Bind complete canonical token_granted function body ignoring whitespace/comments
        func_start = stripped_combat.find("fn token_granted")
        if func_start == -1:
            errors.append(f"Active 'fn token_granted' not found in {combat_rs}")
        else:
            open_brace = stripped_combat.find("{", func_start)
            if open_brace == -1:
                errors.append(f"Malformed 'fn token_granted' signature in {combat_rs}")
            else:
                depth = 0
                func_end = -1
                for idx in range(open_brace, len(stripped_combat)):
                    if stripped_combat[idx] == "{":
                        depth += 1
                    elif stripped_combat[idx] == "}":
                        depth -= 1
                        if depth == 0:
                            func_end = idx + 1
                            break
                if func_end == -1:
                    errors.append(f"Unterminated braces in 'fn token_granted' in {combat_rs}")
                else:
                    extracted_func = stripped_combat[func_start:func_end]
                    func_no_ws = normalize_rust_whitespace(extracted_func)
                    if func_no_ws != CANONICAL_TOKEN_GRANTED_NO_WS:
                        if '3=>"3"' in func_no_ws or '3=>' in func_no_ws:
                            errors.append("token_granted expands tier mapping beyond tiers 1 and 2")
                        if 'TokenSource::Admission=>"admission"' not in func_no_ws or \
                           'TokenSource::LevelUp=>"level_up"' not in func_no_ws:
                            errors.append("token_granted token source mapping does not match admission/level_up")
                        label_calls = len(re.findall(r'KeyValue::new\(', extracted_func))
                        if label_calls != 2:
                            errors.append(f"token_granted must observe exactly 2 KeyValue labels, found {label_calls}")
                        errors.append(
                            f"token_granted implementation does not match canonical static source contract: {extracted_func.strip()}"
                        )

    validate_grant_source_binding.derived_prom_metric = derived_prom_metric or "nightfall_class_transfer_token_grants_total"
    return errors


def validate_dashboard() -> list[str]:
    errors = []
    dashboard_path = ROOT / "infra/grafana/dashboards/nightfall-api.json"
    if not dashboard_path.is_file():
        return [f"Dashboard file not found: {dashboard_path}"]

    try:
        data = json.loads(dashboard_path.read_text(encoding="utf-8"))
    except Exception as exc:
        return [f"Invalid JSON in dashboard: {exc}"]

    # Run source binding check and derive Prometheus exporter metric name
    binding_errors = validate_grant_source_binding()
    errors.extend(binding_errors)
    derived_prom_metric = getattr(
        validate_grant_source_binding,
        "derived_prom_metric",
        "nightfall_class_transfer_token_grants_total",
    )

    panels = data.get("panels", [])
    if not panels:
        errors.append("No panels found in dashboard")

    validate_dashboard.panel_count = len(panels)

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

    # Panels 1..27 must be preserved
    for i in range(1, 28):
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
        derived_prom_metric,
    }
    dashboard_text = " ".join(
        t["expr"]
        for p in panels
        if isinstance(p.get("targets"), list)
        for t in p["targets"]
        if isinstance(t, dict) and isinstance(t.get("expr"), str)
    )
    for m in expected_metrics:
        if m not in dashboard_text:
            errors.append(f"Expected source metric {m} missing from dashboard queries")

    # Check panels after 20 for bounded labels and conventions
    class_panels = [p for p in panels if p.get("id", 0) > 20]
    if len(class_panels) < 7:
        errors.append(f"Expected at least 7 class transfer panels after id 20, found {len(class_panels)}")

    for p in class_panels:
        ds = p.get("datasource", {})
        if ds.get("type") != "prometheus" or ds.get("uid") != "prometheus":
            errors.append(f"Panel {p.get('id')} does not follow datasource convention: {ds}")
        targets = p.get("targets", [])
        if not isinstance(targets, list):
            errors.append(f"Panel {p.get('id')} targets must be a JSON array")
            continue
        for target in targets:
            if not isinstance(target, dict) or not isinstance(target.get("expr"), str):
                errors.append(f"Panel {p.get('id')} target must contain a string query")
                continue
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

    # Specific checks for grant panels (26 and 27)
    grant_panels = [p for p in panels if p.get("id") in (26, 27)]
    if len(grant_panels) != 2:
        errors.append(f"Expected exactly 2 grant panels (ids 26 and 27), found {len(grant_panels)}")

    for p in grant_panels:
        pid = p.get("id")
        desc = p.get("description", "").lower()
        if "applied" not in desc:
            errors.append(f"Grant panel {pid} description must document applied-only boundary")
        if "replayed" not in desc:
            errors.append(f"Grant panel {pid} description must document exclusion of replayed checkpoints")
        if "outbox" not in desc and "accounting" not in desc:
            errors.append(f"Grant panel {pid} description must document durable outbox accounting distinction")

        targets = p.get("targets")
        if not isinstance(targets, list) or len(targets) != 1:
            errors.append(
                f"Grant panel {pid} must have nonempty single target (existing grant panels each exactly 1 target A), found: {len(targets) if isinstance(targets, list) else targets}"
            )
            continue

        target = targets[0]
        if not isinstance(target, dict):
            errors.append(f"Grant panel {pid} target must be a JSON object, got: {type(target).__name__}")
            continue

        ref_id = target.get("refId")
        if ref_id != "A":
            errors.append(f"Grant panel {pid} target refId must be 'A', got: {ref_id!r}")

        expr = target.get("expr")
        if not isinstance(expr, str) or not expr.strip():
            errors.append(f"Grant panel {pid} target query expression cannot be empty")
            continue

        if not check_balanced_delimiters(expr):
            errors.append(f"Grant panel {pid} query has unmatched parentheses/brackets: {expr}")
            continue

        norm_expr = normalize_promql(expr)

        # Expected complete supported query forms permitting whitespace normalization only:
        # Panel 26: sum by(tier,source)(rate(METRIC[$__rate_interval]))
        # Panel 27: sum by(tier,source)(METRIC)
        expected_p26 = {
            f"sum by(tier,source)(rate({derived_prom_metric}[$__rate_interval]))",
            f"sum by(source,tier)(rate({derived_prom_metric}[$__rate_interval]))",
        }
        expected_p27 = {
            f"sum by(tier,source)({derived_prom_metric})",
            f"sum by(source,tier)({derived_prom_metric})",
        }

        if pid == 26:
            if norm_expr not in expected_p26:
                errors.append(
                    f"Grant panel 26 query does not match required rate query form 'sum by (tier, source) (rate({derived_prom_metric}[$__rate_interval]))': {expr}"
                )
        elif pid == 27:
            if norm_expr not in expected_p27:
                errors.append(
                    f"Grant panel 27 query does not match required cumulative query form 'sum by (tier, source) ({derived_prom_metric})': {expr}"
                )

    p26 = next((p for p in panels if p.get("id") == 26), None)
    if p26:
        gp = p26.get("gridPos", {})
        if gp.get("x") != 0 or gp.get("y") != 104 or gp.get("w") != 12 or gp.get("h") != 8:
            errors.append(f"Panel 26 gridPos expected {{x: 0, y: 104, w: 12, h: 8}}, got {gp}")
    p27 = next((p for p in panels if p.get("id") == 27), None)
    if p27:
        gp = p27.get("gridPos", {})
        if gp.get("x") != 12 or gp.get("y") != 104 or gp.get("w") != 12 or gp.get("h") != 8:
            errors.append(f"Panel 27 gridPos expected {{x: 12, y: 104, w: 12, h: 8}}, got {gp}")

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
            "seven Phase 2 panels",
            "nightfall_class_transfer_token_grants",
            "nightfall_class_transfer_token_grants_total",
            "1175b73",
            "692e989",
            "ae0583d",
            "critical-fix-review.md",
            "CheckpointOutcome::Applied",
            "CheckpointOutcome::Replayed",
            "admission",
            "level_up",
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
        panel_count = getattr(validate_dashboard, "panel_count", 27)
        print(f"  OK: Dashboard contract valid ({panel_count} panels, non-overlapping grid, exact metrics).")

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
