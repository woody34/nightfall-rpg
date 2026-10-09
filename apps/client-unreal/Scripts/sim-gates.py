#!/usr/bin/env python3
"""Final wrapper gates. Unknown report classifications and coverage schemas fail closed."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import time
import xml.etree.ElementTree as ET

BOT_FAILURE_KINDS = frozenset({'bot_assertion', 'bot_expectation', 'bot_scenario'})
TABLES = ('npc_intentions', 'player_attack_states')
COUNTERS = ('life_incarnations', 'deaths', 'respawns', 'intent_rejected')


def write_json(path, value):
    path = Path(path)
    temporary = path.with_name(path.name + '.tmp')
    temporary.write_text(json.dumps(value, indent=2) + '\n')
    temporary.replace(path)


def read_report(path):
    """Check exact per-suite counters, including failure elements and outer totals."""
    root = ET.parse(path).getroot()
    if root.tag not in ('testsuite', 'testsuites'):
        raise ValueError('invalid JUnit root')
    suites = [root] if root.tag == 'testsuite' else list(root)
    if not suites or any(s.tag != 'testsuite' for s in suites):
        raise ValueError('missing or nested JUnit suites')
    totals = dict.fromkeys(('tests', 'failures', 'errors', 'skipped'), 0)
    for suite in suites:
        cases = suite.findall('testcase')
        if not cases:
            raise ValueError('empty JUnit suite')
        actual = {'tests': len(cases)}
        for tag, key in (('failure', 'failures'), ('error', 'errors'), ('skipped', 'skipped')):
            actual[key] = sum(bool(case.findall(tag)) for case in cases)
        for case in cases:
            if sum(len(case.findall(tag)) for tag in ('failure', 'error', 'skipped')) > 1:
                raise ValueError('contradictory JUnit testcase outcomes')
        for key, count in actual.items():
            if int(suite.get(key, '0')) != count:
                raise ValueError(f'inconsistent JUnit {key}')
            totals[key] += count
    if root.tag == 'testsuites':
        for key, count in totals.items():
            if int(root.get(key, '0')) != count:
                raise ValueError(f'inconsistent outer JUnit {key}')
    if len(root.findall('.//testcase')) != totals['tests']:
        raise ValueError('nested or uncounted JUnit testcase')
    return root


def bot_kinds(root, allow_pipeline=False):
    kinds, infrastructure = set(), []
    for case in root.findall('.//testcase'):
        for failure in case.findall('failure') + case.findall('error'):
            if allow_pipeline and case.get('name') == 'pipeline':
                continue
            kind = failure.get('type')
            if failure.tag == 'failure' and kind in BOT_FAILURE_KINDS:
                kinds.add(kind)
            else:
                infrastructure.append('report_failure:' + (kind or 'unclassified'))
    for prop in root.findall('.//property'):
        if prop.get('name') == 'sentinel_unallowed' and prop.get('value') != '0':
            infrastructure.append('log_or_ensure_sentinel')
    return sorted(kinds), infrastructure


def scenario_state(report, name, code, infrastructure):
    infrastructure = list(infrastructure)
    kinds = []
    try:
        root = read_report(report)
        suites = [root] if root.tag == 'testsuite' else list(root)
        if any(s.get('name') not in (None, name) for s in suites):
            infrastructure.append('report_scenario_attribution')
        kinds, report_errors = bot_kinds(root)
        infrastructure.extend(report_errors)
        failed = bool(root.findall('.//failure') or root.findall('.//error'))
        if all(case.find('skipped') is not None for case in root.findall('.//testcase')):
            infrastructure.append('no_executed_testcases')
        if (code == 0 and failed) or (code == 1 and not failed):
            infrastructure.append('exit_report_contradiction')
        for prop in root.findall(".//property[@name='exit_code']"):
            if prop.get('value') != str(code):
                infrastructure.append('reported_exit_contradiction')
    except (OSError, ValueError, ET.ParseError) as error:
        infrastructure.append('missing_unreadable_inconsistent_report:' + str(error))
    if code not in (0, 1):
        infrastructure.append(f'bot_process_exit:{code}')
    return {'scenario': name, 'failure_kinds': kinds, 'process_exit_code': code,
            'infrastructure_failures': sorted(set(infrastructure))}


def finalize(folder, names, code, infrastructure, group=None):
    rows = []
    infrastructure = list(infrastructure)
    for name in names:
        kinds = []
        try:
            row = json.loads((folder / name / 'pipeline-state.json').read_text())
            if row['scenario'] != name:
                raise ValueError('scenario state attribution mismatch')
            kinds = row['failure_kinds']
            if not isinstance(kinds, list) or any(k not in BOT_FAILURE_KINDS for k in kinds):
                raise ValueError('invalid scenario state classification')
            infrastructure.extend(f'{name}:{item}' for item in row['infrastructure_failures'])
            root = read_report(folder / name / f'{name}.xml')
            actual, report_errors = bot_kinds(root, allow_pipeline=True)
            infrastructure.extend(f'{name}:{item}' for item in report_errors)
            if actual != kinds:
                raise ValueError('final report classification changed')
        except (OSError, ValueError, KeyError, TypeError, ET.ParseError) as error:
            infrastructure.append(f'{name}:final_report_or_state:{error}')
        rows.append({'scenario': name, 'failure_kinds': kinds})
    if group:
        try:
            root = read_report(group)
            actual = sorted(ET.tostring(case) for case in root.findall('.//testcase'))
            expected = sorted(ET.tostring(case) for name in names
                              for case in read_report(folder / name / f'{name}.xml').findall('.//testcase'))
            if actual != expected:
                raise ValueError('incomplete group merge')
        except (OSError, ValueError, ET.ParseError) as error:
            infrastructure.append('group_junit_merge:' + str(error))
    failed = bool(infrastructure) or any(row['failure_kinds'] for row in rows)
    if bool(code) != failed:
        infrastructure.append('wrapper_exit_contradiction')
    return {'schema_version': 1, 'completed': True, 'exit_code': code,
            'infrastructure_failures': sorted(set(infrastructure)), 'scenarios': rows}


def claim(folder, names):
    """Never delete prior evidence. CI may pre-open only its orchestration log."""
    folder = Path(folder)
    if folder.is_symlink():
        raise ValueError('artifact destination cannot be a symlink')
    folder.mkdir(parents=True, exist_ok=True)
    allowed = {'orchestration.log'} if (os.environ.get('SIM_REQUIRE_FRESH_ARTIFACTS') == '1'
        and os.environ.get('SIM_PIPELINE_VERDICT') == str(folder.resolve() / 'pipeline-verdict.json')) else set()
    verdict = os.environ.get('SIM_PIPELINE_VERDICT')
    if verdict and (Path(verdict).exists() or Path(verdict).is_symlink()):
        raise ValueError('pipeline verdict destination must be new')
    entries = list(folder.iterdir())
    if any(p.name not in allowed or p.is_symlink() or not p.is_file() for p in entries):
        raise ValueError('--artifacts must be empty; select a new destination to retain previous evidence')
    if len(names) != len(set(names)):
        raise ValueError('duplicate scenario artifact names')
    # Exclusive claim prevents two wrappers from accepting an initially empty destination.
    with (folder / '.sim-run.json').open('x') as owner:
        json.dump({'schema_version': 1, 'pid': os.getpid(), 'started_ns': time.time_ns(),
                   'scenarios': names}, owner)
    for name in names:
        (folder / name).mkdir()


def count(value):
    if type(value) is not int or value < 0:
        raise ValueError('coverage counts must be nonnegative integers')
    return value


def read_transitions(path):
    data = json.loads(Path(path).read_text())
    if not isinstance(data, dict) or type(data.get('schema_version')) is not int or data['schema_version'] != 1:
        raise ValueError(f'{path}: unsupported transition schema')
    for key in TABLES:
        rows = data.get(key)
        if not isinstance(rows, list) or not rows:
            raise ValueError(f'{path}: missing transition table {key}')
        pairs = set()
        states = set()
        for row in rows:
            before, after = row['from'], row['to']
            if not all(isinstance(s, str) and s for s in (before, after)):
                raise ValueError('invalid transition state')
            pair = (before, after)
            if pair in pairs or type(row['reachable']) is not bool:
                raise ValueError('duplicate transition or invalid reachability')
            pairs.add(pair)
            states.update(pair)
            amount = count(row['count'])
            if before == after and row['reachable']:
                raise ValueError('self transitions cannot be reachable')
            if amount and (not row['reachable'] or before == after):
                raise ValueError('observed unreachable or self transition')
        if len(states) < 2 or not any(row['reachable'] for row in rows):
            raise ValueError('empty transition domain or no reachable transitions')
        if pairs != {(a, b) for a in states for b in states}:
            raise ValueError('incomplete transition catalogue')
    for key in COUNTERS:
        values = data.get(key)
        if not isinstance(values, dict):
            raise ValueError(f'missing counters {key}')
        for name, amount in values.items():
            if not isinstance(name, str) or not name:
                raise ValueError('invalid coverage counter name')
            count(amount)
            if key == 'life_incarnations':
                match = re.fullmatch(r'(player|npc):(\d+)->(\d+)', name)
                if not match or int(match[3]) <= int(match[2]):
                    raise ValueError('invalid life-incarnation change')
        if key in ('deaths', 'respawns') and set(values) != {'player', 'npc'}:
            raise ValueError('missing player/NPC lifecycle counters')
    return data


def transition_summary(data):
    result = {}
    for key in TABLES:
        rows = data[key]
        label = lambda row: row['from'] + ' -> ' + row['to']
        result[key] = {'covered': sum(r['reachable'] and r['count'] > 0 for r in rows),
                       'total': sum(r['reachable'] for r in rows),
                       'never_seen': [label(r) for r in rows if r['reachable'] and not r['count']],
                       'unreachable': [label(r) for r in rows if not r['reachable']]}
    return result


def merge_transitions(paths, unique_recordings=False):
    if not paths:
        raise ValueError('no current-run transition coverage inputs')
    output = None
    sources, seen = [], {}
    for path in paths:
        data = read_transitions(path)
        if unique_recordings:
            digest = data.get('recording_sha256')
            if not isinstance(digest, str) or not re.fullmatch(r'[0-9a-f]{64}', digest):
                raise ValueError('missing recording identity for unit coverage')
            facts = {key: data[key] for key in ('schema_version', *TABLES, *COUNTERS)}
            if digest in seen:
                if seen[digest] != facts:
                    raise ValueError('identical recordings have contradictory coverage')
                continue
            seen[digest] = json.loads(json.dumps(facts))
        sources.append(str(path))
        if output is None:
            output = {key: data[key] for key in ('schema_version', *TABLES, *COUNTERS)}
            continue
        for key in TABLES:
            signature = lambda rows: {(r['from'], r['to']): r['reachable'] for r in rows}
            if signature(output[key]) != signature(data[key]):
                raise ValueError('transition catalogues differ between recordings')
            amounts = {(r['from'], r['to']): r['count'] for r in data[key]}
            for row in output[key]:
                row['count'] += amounts[(row['from'], row['to'])]
        for key in COUNTERS:
            for name, amount in data[key].items():
                output[key][name] = output[key].get(name, 0) + amount
    output['sources'] = sources
    output['summary'] = transition_summary(output)
    return output


def print_transitions(data):
    for name, row in transition_summary(data).items():
        print(f"transitions {name}: {row['covered']}/{row['total']} reachable pairs covered")
        print('  never seen: ' + (', '.join(row['never_seen']) or '(none)'))
        print('  unreachable: ' + (', '.join(row['unreachable']) or '(none)'))


def coverage(recording, out):
    command = os.environ.get('SIM_COVERAGE_CMD')
    if command is not None:
        command = shlex.split(command)
        if not command:
            raise ValueError('SIM_COVERAGE_CMD is empty')
    elif os.environ.get('SIM_REPLAY_BIN'):
        command = [os.environ['SIM_REPLAY_BIN']]
    else:
        command = ['cargo', 'run', '--quiet', '-p', 'nightfall-api', '--bin', 'nightfall-replay', '--']
    with out.with_suffix('.log').open('w') as log:
        subprocess.run(command + ['coverage', '--file', str(recording), '--out', str(out)],
                       cwd=Path(__file__).resolve().parents[3], stdout=log,
                       stderr=subprocess.STDOUT, check=True)
    data = read_transitions(out)
    data['recording_source'] = str(recording)
    data['recording_sha256'] = hashlib.sha256(recording.read_bytes()).hexdigest()
    write_json(out, data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('claim')
    p.add_argument('folder', type=Path)
    p.add_argument('names', nargs='+')
    p = sub.add_parser('scenario')
    p.add_argument('--report', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--scenario', required=True)
    p.add_argument('--code', required=True, type=int)
    p.add_argument('infrastructure', nargs='*')
    p = sub.add_parser('finalize')
    p.add_argument('--folder', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--code', required=True, type=int)
    p.add_argument('--group', type=Path)
    p.add_argument('--infrastructure', action='append', default=[])
    p.add_argument('names', nargs='+')
    p = sub.add_parser('coverage')
    p.add_argument('--file', required=True, type=Path)
    p.add_argument('--out', required=True, type=Path)
    p = sub.add_parser('merge-transitions')
    p.add_argument('--unique-recordings', action='store_true')
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('inputs', nargs='+', type=Path)
    args = parser.parse_args()
    try:
        if args.command == 'claim':
            claim(args.folder, args.names)
        elif args.command == 'scenario':
            row = scenario_state(args.report, args.scenario, args.code, args.infrastructure)
            write_json(args.out, row)
            return int(bool(row['failure_kinds'] or row['infrastructure_failures']))
        elif args.command == 'finalize':
            verdict = finalize(args.folder, args.names, args.code, args.infrastructure, args.group)
            write_json(args.out, verdict)
            if args.code == 0 and verdict['infrastructure_failures']:
                return 1
        elif args.command == 'coverage':
            coverage(args.file, args.out)
        else:
            result = merge_transitions(args.inputs, args.unique_recordings)
            write_json(args.out, result)
            print_transitions(result)
    except (OSError, ValueError, KeyError, TypeError, ET.ParseError, subprocess.SubprocessError) as error:
        print(f'sim gate failed: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
