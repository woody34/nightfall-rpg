#!/usr/bin/env python3
"""Run every scenario, pair coordinated roles, and retain quarantine failures as evidence."""
import argparse
import importlib.util
from datetime import date
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import xml.etree.ElementTree as ET

PROJECT = Path(__file__).resolve().parents[1]
REPO = PROJECT.parents[1]
_spec = importlib.util.spec_from_file_location('sim_gates', PROJECT / 'Scripts/sim-gates.py')
gates = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gates)
_fixture_spec = importlib.util.spec_from_file_location('phase2_fixture', PROJECT / 'Scripts/phase2-fixture.py')
phase2 = importlib.util.module_from_spec(_fixture_spec)
_fixture_spec.loader.exec_module(phase2)


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
    """Shared exact Phase2 pack/role contract, retaining legacy fixture selection."""
    return phase2.fixture(batch)


def junit_failures(folder, names):
    """Missing, inconsistent and failed reports remain blocking."""
    failures = []
    for name in names:
        try:
            root = gates.read_report(folder / name / f"{name}.xml")
            if root.findall('.//failure') or root.findall('.//error'):
                failures.append(name)
        except (OSError, ValueError, ET.ParseError):
            failures.append(name)
    return failures


def has_scenario_failure(folder, name):
    """Only actual classified bot failures qualify; wrapper cases are checked by verdict."""
    try:
        root = gates.read_report(folder / name / f"{name}.xml")
        kinds, infrastructure = gates.bot_kinds(root, allow_pipeline=True)
        return bool(kinds) and not infrastructure
    except (OSError, ValueError, ET.ParseError):
        return False


BOT_FAILURE_KINDS = frozenset({"bot_assertion", "bot_expectation", "bot_scenario"})


def pipeline_verdict(folder, names, code):
    """Validate completed wrapper evidence for both green and failing units."""
    verdict = json.loads((folder / 'pipeline-verdict.json').read_text())
    if (type(verdict.get('schema_version')) is not int or verdict['schema_version'] != 1
            or verdict.get('completed') is not True
            or type(verdict.get('exit_code')) is not int or verdict['exit_code'] != code):
        raise ValueError('incomplete or contradictory wrapper verdict')
    infrastructure = verdict.get('infrastructure_failures')
    if not isinstance(infrastructure, list) or any(not isinstance(v, str) or not v for v in infrastructure):
        raise ValueError('invalid infrastructure failure list')
    rows = verdict.get('scenarios')
    if not isinstance(rows, list) or len(rows) != len(names):
        raise ValueError('incomplete scenario verdicts')
    seen = set()
    for row in rows:
        name, kinds = row['scenario'], row['failure_kinds']
        if name not in names or name in seen or not isinstance(kinds, list):
            raise ValueError('invalid scenario verdict attribution')
        if any(not isinstance(kind, str) or kind not in BOT_FAILURE_KINDS for kind in kinds):
            raise ValueError('unknown bot failure classification')
        seen.add(name)
        root = gates.read_report(folder / name / f'{name}.xml')
        actual, report_errors = gates.bot_kinds(root, allow_pipeline=True)
        if sorted(set(kinds)) != actual or (report_errors and not infrastructure):
            raise ValueError('verdict contradicts actual failure types')
    failed = bool(infrastructure) or any(row['failure_kinds'] for row in rows)
    if (code == 0 and failed) or (code == 1 and not failed) or code not in (0, 1):
        raise ValueError('wrapper exit contradicts verdict')
    return verdict


def quarantine_verdict(folder, names, code, bad_reports, exempt):
    """Schema v1: completed=true, exit_code, infrastructure_failures, scenarios.

    SIM_PIPELINE_VERDICT=<unit>/pipeline-verdict.json is finalized after ALL gates and
    group merge. Only actual bot_assertion/bot_expectation/bot_scenario failure types
    may be exempted. Missing fields, unknown types and diagnostic retries fail closed.
    """
    try:
        verdict = pipeline_verdict(folder, names, code)
        if verdict['infrastructure_failures'] or code != 1:
            return False
        failing = {row['scenario'] for row in verdict['scenarios'] if row['failure_kinds']}
        return bool(failing) and failing == set(bad_reports) and all(
            name in exempt and has_scenario_failure(folder, name) for name in failing)
    except (OSError, ValueError, KeyError, TypeError, AttributeError, ET.ParseError):
        return False


def group_report_matches(folder, names):
    """Missing/partial group merge is infrastructure failure even with valid role reports."""
    try:
        group = gates.read_report(folder / "group.xml")
        if group.tag != "testsuites":
            return False
        for key in ("tests", "failures", "errors"):
            if int(group.get(key, "-1")) != sum(int(s.get(key, "0")) for s in group.iter("testsuite")):
                return False
        actual = sorted(ET.tostring(case) for case in group.findall(".//testcase"))
        expected = sorted(ET.tostring(case) for name in names
                          for case in ET.parse(folder / name / f"{name}.xml").getroot().findall(".//testcase"))
        return bool(expected) and actual == expected
    except (OSError, ValueError, ET.ParseError):
        return False


def suite_coverage(artifacts, batches, baseline):
    """Aggregate current named inputs exactly once; missing evidence cannot be quarantined."""
    contract_inputs, transition_inputs = [], []
    for batch in batches:
        unit = batch[0].stem[:-2] if len(batch) > 1 else batch[0].stem
        for scenario in batch:
            dest = artifacts / unit / scenario.stem
            contract_inputs.append(dest / f'{scenario.stem}.coverage.contract.json')
        transition_inputs.append(artifacts / unit / 'coverage.transitions.json')
    errors, lines = [], []
    transitions, contract = None, None
    try:
        for batch in batches:
            unit = batch[0].stem[:-2] if len(batch) > 1 else batch[0].stem
            for scenario in batch:
                gates.read_transitions(artifacts / unit / scenario.stem / 'coverage.transitions.json')
        transitions = gates.merge_transitions(transition_inputs)
        gates.write_json(artifacts / 'coverage.transitions.json', transitions)
        for name, row in transitions['summary'].items():
            lines.extend([f"transitions {name}: {row['covered']}/{row['total']} reachable pairs covered",
                          'never seen: ' + (', '.join(row['never_seen']) or '(none)'),
                          'unreachable: ' + (', '.join(row['unreachable']) or '(none)')])
    except (OSError, ValueError, KeyError, TypeError) as error:
        errors.append(f'suite transition coverage missing or invalid: {error}')
    command = [sys.executable, str(PROJECT / 'Scripts/sim-contract.py'), 'merge',
               '--out', str(artifacts / 'coverage.contract.json'), '--baseline', str(baseline),
               *map(str, contract_inputs)]
    try:
        result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                cwd=PROJECT, timeout=60)
        (artifacts / 'coverage.contract.log').write_text(result.stdout)
        lines.extend(result.stdout.strip().splitlines())
        if result.returncode:
            errors.append(f'suite contract coverage baseline gate failed (exit {result.returncode}); see coverage.contract.log')
        if result.returncode == 0 and not (artifacts / 'coverage.contract.json').exists():
            raise ValueError('contract merger succeeded without a suite report')
        if (artifacts / 'coverage.contract.json').exists():
            contract = json.loads((artifacts / 'coverage.contract.json').read_text())
            if not isinstance(contract, dict) or not isinstance(contract.get('summary'), dict):
                raise ValueError('missing suite contract summary')
    except (OSError, ValueError, TypeError, subprocess.SubprocessError) as error:
        errors.append(f'suite contract coverage missing or invalid: {error}')
    for line in lines:
        print(line, flush=True)
    for error in errors:
        print('FAIL infrastructure: ' + error, flush=True)
    return {'infrastructure_errors': errors, 'transitions': transitions,
            'contract': contract, 'summary_lines': lines}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenarios", type=Path, default=PROJECT / "Scenarios")
    parser.add_argument("--artifacts", type=Path, default=PROJECT / "Saved/SimCI")
    parser.add_argument("--quarantine", type=Path, default=PROJECT / "Scenarios/quarantine.json")
    parser.add_argument("--contract-baseline", type=Path,
                        default=PROJECT / "Scenarios/coverage.contract.baseline.json",
                        help="historical contract presence required across this suite")
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
    if (args.fresh_stack and any(value not in phase2.PHASE2 for value in fixtures)
            and not os.environ.get("COMPOSE_PROJECT_NAME", "").startswith("nightfall-sim-")):
        parser.error("--fresh-stack requires an isolated COMPOSE_PROJECT_NAME=nightfall-sim-...")
    artifacts = args.artifacts.resolve()
    # Retain the published path layout, but never reuse evidence from an earlier invocation.
    # Do not delete artifacts: callers must select a new destination for each CI attempt.
    unit_names = [batch[0].stem[:-2] if len(batch) > 1 else batch[0].stem for batch in batches]
    if len(set(unit_names)) != len(unit_names):
        parser.error("scenario/group artifact directory names collide")
    try:
        artifacts.mkdir(parents=True, exist_ok=True)
        if any(artifacts.iterdir()):
            parser.error("--artifacts must be empty; select a new destination to retain previous evidence")
        # Exclusive claim also prevents two invocations from sharing an initially empty root.
        with (artifacts / "run.json").open("x") as owner:
            json.dump({"schema_version": 1, "pid": os.getpid(), "started_ns": time.time_ns()}, owner)
    except OSError as error:
        parser.error(str(error))
    report = []
    aggregate = ET.Element("testsuite", name="sim orchestration")
    for batch, fixture_name in zip(batches, fixtures):
        names = [p.stem for p in batch]
        name = names[0][:-2] if len(batch) > 1 else names[0]
        dest = artifacts / name
        dest.mkdir()
        start = time.monotonic()
        script = "run-sim-multi.sh" if len(batch) > 1 else "run-sim.sh"
        command = ["bash", str(PROJECT / "Scripts" / script), "--api", "start" if args.fresh_stack else "attach",
                   "--artifacts", str(dest), "--timeout", str(args.timeout)]
        if args.video and len(batch) == 1:
            command.append("--video")
        command += list(map(str, batch))
        code = 2
        env = dict(os.environ)
        env["SIM_PIPELINE_VERDICT"] = str(dest / "pipeline-verdict.json")
        env["SIM_REQUIRE_FRESH_ARTIFACTS"] = "1"
        env["SIM_REQUIRE_TRANSITION_COVERAGE"] = "1"
        if args.fresh_stack:
            env["SIM_REQUIRE_OWNED_API"] = "1"
        env.pop("ZONE_SIM_FIXTURE", None)
        env.pop("ZONE_FILE", None)
        if fixture_name == "phase1a-late-entry":
            env["ZONE_FILE"] = str(REPO / "apps/api/fixtures/phase1a-late-entry/zones/late_entry.toml")
        elif fixture_name != "default" and fixture_name not in phase2.PHASE2:
            env["ZONE_SIM_FIXTURE"] = fixture_name
        orchestration_errors = []
        with (dest / "orchestration.log").open("w") as log:
            try:
                if args.fresh_stack and fixture_name not in phase2.PHASE2:
                    subprocess.run(["docker", "compose", "down", "--volumes", "--remove-orphans"], cwd=REPO,
                                   stdout=log, stderr=subprocess.STDOUT, check=True, timeout=120)
                code = run_clients(command, log, args.timeout + 600, env=env)
            except (OSError, subprocess.SubprocessError) as error:
                log.write(f"orchestration failed: {error}\n")
                orchestration_errors.append(str(error))
        if args.fresh_stack and fixture_name not in phase2.PHASE2:
            with (dest / "compose.log").open("w") as log:
                try:
                    subprocess.run(["docker", "compose", "logs", "--no-color"], cwd=REPO,
                                   stdout=log, stderr=subprocess.STDOUT, timeout=60, check=True)
                except (OSError, subprocess.SubprocessError) as error:
                    log.write(f"compose log collection failed: {error}\n")
                    orchestration_errors.append(str(error))
        if fixture_name in phase2.PHASE2:
            # A forced wrapper/process-group timeout may interrupt its EXIT trap. Retry only
            # generated owned fixture resources, never the root Compose project.
            for state_path in dest.glob('*/fixture/state.json'):
                folder = state_path.parent
                try:
                    status_path = folder / 'cleanup-status.json'
                    cleaned = status_path.exists() and json.loads(status_path.read_text()).get('completed') is True
                    if not cleaned:
                        phase2.cleanup(folder)
                except (OSError, ValueError, KeyError, subprocess.SubprocessError):
                    orchestration_errors.append('owned Phase2 fixture cleanup failed; see fixture logs')
        bad_reports = junit_failures(dest, names)
        if len(names) > 1 and not group_report_matches(dest, names):
            orchestration_errors.append("missing, invalid or incomplete group JUnit merge")
        try:
            verdict = pipeline_verdict(dest, names, code)
            orchestration_errors.extend(verdict['infrastructure_failures'])
        except (OSError, ValueError, KeyError, TypeError, AttributeError, ET.ParseError) as error:
            orchestration_errors.append(f'missing, invalid or incomplete pipeline verdict: {error}')
        failed = code != 0 or bool(bad_reports) or bool(orchestration_errors)
        log_text = (dest / "orchestration.log").read_text()
        quarantined = failed and not orchestration_errors and quarantine_verdict(
            dest, names, code, bad_reports, exempt)
        # Any unexplained nonzero exit stays blocking, even if all reports look green.
        status = "QUARANTINED" if quarantined else "FAIL" if failed else "PASS"
        elapsed = round(time.monotonic() - start, 3)
        report.append({"unit": name, "scenarios": names, "status": status, "exit_code": code,
                       "seconds": elapsed, "failed_reports": bad_reports,
                       "infrastructure_errors": orchestration_errors,
                       "quarantine": {n: exempt[n] for n in names if n in exempt}})
        case = ET.SubElement(aggregate, "testcase", name=name, time=str(elapsed))
        if status == "FAIL":
            ET.SubElement(case, "failure", message=f"exit={code}; failed/missing reports={bad_reports}").text = log_text[-12000:]
        elif quarantined:
            ET.SubElement(case, "skipped", message="quarantined; raw failing JUnit retained").text = json.dumps(report[-1]["quarantine"])
        print(f"{status} {name} ({elapsed}s)", flush=True)
    coverage = suite_coverage(artifacts, batches, args.contract_baseline.resolve())
    failures = sum(row["status"] == "FAIL" for row in report)
    if coverage['infrastructure_errors']:
        failures += 1
        case = ET.SubElement(aggregate, 'testcase', name='suite coverage gates')
        ET.SubElement(case, 'failure', type='pipeline_infrastructure',
                      message='; '.join(coverage['infrastructure_errors']))

    aggregate.set("tests", str(len(report) + bool(coverage["infrastructure_errors"])))
    aggregate.set("failures", str(failures))
    aggregate.set("skipped", str(sum(row["status"] == "QUARANTINED" for row in report)))
    ET.ElementTree(aggregate).write(artifacts / "suite.xml", encoding="utf-8", xml_declaration=True)
    (artifacts / "summary.json").write_text(json.dumps({"schema_version": 1, "units": report, "coverage": coverage}, indent=2) + "\n")
    summary = "| Scenario/group | Result | Seconds |\n|---|---|---|\n" + "".join(
        f"| {row['unit']} | {row['status']} | {row['seconds']} |\n" for row in report)
    summary += '\n' + '\n\n'.join(coverage['summary_lines']) + '\n'
    if coverage['infrastructure_errors']:
        summary += '\n' + '\n\n'.join('FAIL infrastructure: ' + e for e in coverage['infrastructure_errors']) + '\n'
    (artifacts / "summary.md").write_text(summary)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as file:
            file.write(summary)
    return bool(failures)


if __name__ == "__main__":
    sys.exit(main())
