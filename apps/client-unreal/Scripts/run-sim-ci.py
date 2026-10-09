#!/usr/bin/env python3
"""Run every scenario, pair coordinated roles, and retain quarantine failures as evidence."""
import argparse
from datetime import date
import json
import os
from pathlib import Path
import signal
import re
import subprocess
import sys
import time
import xml.etree.ElementTree as ET

PROJECT = Path(__file__).resolve().parents[1]
REPO = PROJECT.parents[1]


def run_clients(command, log, seconds, env=None):
    """Bound the entire client/API process group, including background children."""
    process = subprocess.Popen(command, cwd=PROJECT, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    try:
        return process.wait(timeout=seconds)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        log.write("orchestration timeout; entire process group terminated\n")
        return 124


def quarantine(path, names, today=None):
    """Reject unknown, duplicate, ownerless or expired quarantines before running clients."""
    data = json.loads(path.read_text())
    if data.get("schema_version") != 1 or not isinstance(data.get("scenarios"), list):
        raise ValueError("quarantine requires schema_version=1 and scenarios array")
    result = {}
    for item in data["scenarios"]:
        name = item.get("scenario")
        if name not in names or name in result:
            raise ValueError(f"unknown or duplicate quarantine scenario: {name}")
        for field in ("owner", "reason", "issue", "expires"):
            if not isinstance(item.get(field), str) or not item[field].strip():
                raise ValueError(f"quarantine {name}: missing {field}")
        if date.fromisoformat(item["expires"]) < (today or date.today()):
            raise ValueError(f"expired quarantine {name}: {item['expires']}")
        result[name] = item
    return result


def units(scenarios):
    """A/B roles are one unit; an orphan is a configuration failure, never a single run."""
    paths = {path.stem: path for path in scenarios}
    result = []
    for name, path in sorted(paths.items()):
        if name.endswith("-b"):
            if name[:-2] + "-a" not in paths:
                raise ValueError(f"orphan scenario role: {name}")
            continue
        if name.endswith("-a"):
            peer = name[:-2] + "-b"
            if peer not in paths:
                raise ValueError(f"orphan scenario role: {name}")
            result.append([path, paths[peer]])
        else:
            result.append([path])
    return result


def fixture(batch):
    """Scenario roles must agree on one explicitly supported deterministic fixture."""
    fixtures = []
    for path in batch:
        values = re.findall(r"^#\s*fixture:\s*(\S+)\s*$", path.read_text(), re.MULTILINE)
        if len(values) > 1:
            raise ValueError(f"duplicate fixture header: {path.name}")
        value = values[0] if values else "default"
        if value not in ("default", "phase1a-social-aggro"):
            raise ValueError(f"unsupported fixture {value}: {path.name}")
        fixtures.append(value)
    if len(set(fixtures)) != 1:
        raise ValueError("scenario roles must request the same fixture")
    return fixtures[0]


def junit_failures(folder, names):
    """A successful process cannot conceal an absent, invalid or failed report."""
    failures = []
    for name in names:
        try:
            root = ET.parse(folder / name / f"{name}.xml").getroot()
            suites = [root] if root.tag == "testsuite" else list(root.iter("testsuite"))
            if not suites:
                raise ValueError("no testsuite")
            if any(int(s.get("failures", 0)) or int(s.get("errors", 0)) for s in suites):
                failures.append(name)
            elif root.findall(".//failure") or root.findall(".//error"):
                failures.append(name)
        except (OSError, ValueError, ET.ParseError):
            failures.append(name)
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenarios", type=Path, default=PROJECT / "Scenarios")
    parser.add_argument("--artifacts", type=Path, default=PROJECT / "Saved/SimCI")
    parser.add_argument("--quarantine", type=Path, default=PROJECT / "Scenarios/quarantine.json")
    parser.add_argument("--fresh-stack", action="store_true", help="wipe this Compose project's volumes for each unit")
    parser.add_argument("--video", action="store_true", help="failure-only rendered retry for single-client cases")
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    scenarios = sorted(args.scenarios.resolve().glob("*.nfs"))
    if not scenarios:
        parser.error("no scenarios")
    try:
        exempt = quarantine(args.quarantine, {p.stem for p in scenarios})
        batches = units(scenarios)
        fixtures = [fixture(batch) for batch in batches]
        if not args.fresh_stack and any(value != "default" for value in fixtures):
            raise ValueError("scenario fixture selection requires --fresh-stack; an attached API cannot switch fixtures")
    except (ValueError, OSError) as error:
        parser.error(str(error))
    if args.fresh_stack and not os.environ.get("COMPOSE_PROJECT_NAME", "").startswith("nightfall-sim-"):
        parser.error("--fresh-stack requires an isolated COMPOSE_PROJECT_NAME=nightfall-sim-...")
    artifacts = args.artifacts.resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    report = []
    aggregate = ET.Element("testsuite", name="sim orchestration")
    for batch, fixture_name in zip(batches, fixtures):
        names = [p.stem for p in batch]
        name = names[0][:-2] if len(batch) > 1 else names[0]
        dest = artifacts / name
        dest.mkdir(parents=True, exist_ok=True)
        start = time.monotonic()
        script = "run-sim-multi.sh" if len(batch) > 1 else "run-sim.sh"
        command = ["bash", str(PROJECT / "Scripts" / script), "--api", "start" if args.fresh_stack else "attach",
                   "--artifacts", str(dest), "--timeout", str(args.timeout)]
        if args.video and len(batch) == 1:
            command.append("--video")
        command += list(map(str, batch))
        code = 2
        env = dict(os.environ)
        env.pop("ZONE_SIM_FIXTURE", None)
        if fixture_name != "default":
            env["ZONE_SIM_FIXTURE"] = fixture_name
        with (dest / "orchestration.log").open("w") as log:
            try:
                if args.fresh_stack:
                    subprocess.run(["docker", "compose", "down", "--volumes", "--remove-orphans"], cwd=REPO,
                                   stdout=log, stderr=subprocess.STDOUT, check=True, timeout=120)
                code = run_clients(command, log, args.timeout + 600, env=env)
            except (OSError, subprocess.SubprocessError) as error:
                log.write(f"orchestration failed: {error}\n")
        if args.fresh_stack:
            with (dest / "compose.log").open("w") as log:
                subprocess.run(["docker", "compose", "logs", "--no-color"], cwd=REPO,
                               stdout=log, stderr=subprocess.STDOUT, timeout=60, check=False)
        bad_reports = junit_failures(dest, names)
        failed = code != 0 or bool(bad_reports)
        # Replay, trace and infrastructure failures remain blocking even for a quarantined bot.
        log_text = (dest / "orchestration.log").read_text()
        infrastructure_failure = code not in (0, 1) or any(
            text in log_text for text in ("replay check failed", "trace generation failed", "no session recording", "FAIL replay"))
        quarantined = failed and not infrastructure_failure and bool(bad_reports) and all(n in exempt for n in bad_reports)
        # Any unexplained nonzero exit stays blocking, even if all reports look green.
        status = "QUARANTINED" if quarantined else "FAIL" if failed else "PASS"
        elapsed = round(time.monotonic() - start, 3)
        report.append({"unit": name, "scenarios": names, "status": status, "exit_code": code,
                       "seconds": elapsed, "failed_reports": bad_reports,
                       "quarantine": {n: exempt[n] for n in names if n in exempt}})
        case = ET.SubElement(aggregate, "testcase", name=name, time=str(elapsed))
        if status == "FAIL":
            ET.SubElement(case, "failure", message=f"exit={code}; failed/missing reports={bad_reports}").text = log_text[-12000:]
        elif quarantined:
            ET.SubElement(case, "skipped", message="quarantined; raw failing JUnit retained").text = json.dumps(report[-1]["quarantine"])
        print(f"{status} {name} ({elapsed}s)", flush=True)
    failures = sum(row["status"] == "FAIL" for row in report)
    aggregate.set("tests", str(len(report)))
    aggregate.set("failures", str(failures))
    aggregate.set("skipped", str(sum(row["status"] == "QUARANTINED" for row in report)))
    ET.ElementTree(aggregate).write(artifacts / "suite.xml", encoding="utf-8", xml_declaration=True)
    (artifacts / "summary.json").write_text(json.dumps({"schema_version": 1, "units": report}, indent=2) + "\n")
    summary = "| Scenario/group | Result | Seconds |\n|---|---|---|\n" + "".join(
        f"| {row['unit']} | {row['status']} | {row['seconds']} |\n" for row in report)
    (artifacts / "summary.md").write_text(summary)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as file:
            file.write(summary)
    return bool(failures)


if __name__ == "__main__":
    sys.exit(main())
