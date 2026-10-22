#!/usr/bin/env python3
"""Lane-gate contract smoke: the built nightfall-replay against every committed V6 fixture, in minutes.

For each fixture under apps/api/fixtures (items/*.nfr, phase3-v6/*.nfr, sessions/t11-*.nfr,
sessions/t19-*.nfr) the script runs `check` (byte-identical replay), `coverage` (schema 4; every
item counter label must be a sim-gates.py V4_LABELS name) and `export --items-evidence-out`.
A closed-epoch fixture (items/, phase3-v6/) must export, and phase4-evidence.py validate() must
accept the export; a live capture (T11/T19 sessions) must be refused by the exporter with its
live-capture message. Every closed export must carry the same phase4_rules_hash (and the
--rules-hash value when given). Exit 1 on any mismatch, 2 on a usage error.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys

SCRIPTS = Path(__file__).resolve().parent
REPO = SCRIPTS.parents[2]
LIVE_REFUSAL = 'refuses partial/live captures'
COMMAND_TIMEOUT = 600


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_').replace('.py', ''), SCRIPTS / name)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


gates = module('sim-gates.py')
evidence = module('phase4-evidence.py')


def fixtures(repo):
    """(recording, kind) for every committed V6 fixture; kind is closed (exportable) or live."""
    root = Path(repo) / 'apps/api/fixtures'
    rows = [(path, 'closed') for folder in ('items', 'phase3-v6') for path in sorted((root / folder).glob('*.nfr'))]
    rows += [(path, 'live') for prefix in ('t11-', 't19-') for path in sorted((root / 'sessions').glob(prefix + '*.nfr'))]
    if not any(kind == 'closed' for _, kind in rows):
        raise ValueError(f'no closed V6 fixtures under {root}')
    return rows


def run(binary, arguments, log):
    result = subprocess.run([str(binary), *arguments], text=True, capture_output=True, timeout=COMMAND_TIMEOUT)
    Path(log).write_text(result.stdout + result.stderr)
    return result


def coverage_labels(path):
    """Schema-4 coverage with every item counter present and every label a V4_LABELS name."""
    data = gates.read_transitions(path)  # refuses unknown schema-4 labels and malformed tables
    if data['schema_version'] != 4:
        raise ValueError(f"coverage schema {data['schema_version']} is not the V6 schema 4")
    missing = [key for key in (*gates.V4_COUNTERS, *gates.V4_SCALARS) if key not in data]
    if missing:
        raise ValueError('coverage lacks schema-4 item facts: ' + ', '.join(missing))
    unknown = {key: sorted(set(data[key]) - gates.V4_LABELS[key]) for key in gates.V4_LABELS if set(data[key]) - gates.V4_LABELS[key]}
    if unknown:
        raise ValueError(f'coverage labels outside V4_LABELS: {unknown}')
    return {key: sorted(data[key]) for key in gates.V4_COUNTERS}


def smoke_fixture(binary, recording, kind, out, rules_hash):
    name = recording.stem
    row = {'fixture': str(recording), 'kind': kind, 'problems': []}
    check = run(binary, ['check', '--file', str(recording)], out / f'{name}.check.log')
    if check.returncode != 0:
        row['problems'].append(f'check exit {check.returncode}: {(check.stdout + check.stderr).strip().splitlines()[-1:] or [""]}'[:400])
    coverage_path = out / f'{name}.coverage.transitions.json'
    coverage = run(binary, ['coverage', '--file', str(recording), '--out', str(coverage_path)], out / f'{name}.coverage.log')
    if coverage.returncode != 0 or not coverage_path.is_file():
        row['problems'].append(f'coverage exit {coverage.returncode}')
    else:
        try:
            row['labels'] = coverage_labels(coverage_path)
        except (ValueError, KeyError, TypeError, OSError) as error:
            row['problems'].append(f'coverage contract: {error}')
    raw = out / f'{name}.phase4.raw.jsonl'
    export = run(binary, ['export', '--file', str(recording), '--items-evidence-out', str(raw)], out / f'{name}.export.log')
    if kind == 'live':
        if export.returncode == 0 or LIVE_REFUSAL not in export.stdout + export.stderr:
            row['problems'].append(f'live capture export: expected refusal "{LIVE_REFUSAL}", got exit {export.returncode}')
        return row
    if export.returncode != 0 or not raw.is_file():
        row['problems'].append(f'items export exit {export.returncode}')
        return row
    try:
        rows = evidence.read_jsonl(raw)
        manifest = rows[0] if rows and isinstance(rows[0], dict) else {}
        row['rules_hash'] = manifest.get('phase4_rules_hash')
        if not isinstance(row['rules_hash'], str):
            raise ValueError('export manifest names no phase4_rules_hash')
        if rules_hash and row['rules_hash'] != rules_hash:
            raise ValueError(f"export rules hash {row['rules_hash'][:12]}… differs from --rules-hash {rules_hash[:12]}…")
        evidence.validate(rows, recording, row['rules_hash'])
        row['facts'] = len(rows) - 2
    except (ValueError, KeyError, TypeError, IndexError, OSError) as error:
        row['problems'].append(f'items evidence contract: {error}')
    return row


def smoke(binary, repo, out, rules_hash=None):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    rows = [smoke_fixture(binary, recording, kind, out, rules_hash) for recording, kind in fixtures(repo)]
    hashes = sorted({row['rules_hash'] for row in rows if row.get('rules_hash')})
    if len(hashes) > 1:
        for row in rows:
            if row.get('rules_hash'):
                row['problems'].append(f'closed fixtures disagree on phase4_rules_hash: {[h[:12] for h in hashes]}')
    report = {'schema_version': 1, 'replay': str(binary), 'repo': str(repo), 'rules_hash': hashes[0] if len(hashes) == 1 else None,
              'fixtures': rows, 'passed': all(not row['problems'] for row in rows)}
    (out / 'contract-smoke.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--replay', type=Path, required=True, help='built nightfall-replay binary (release for V6 bundles)')
    parser.add_argument('--repo', type=Path, default=REPO, help='repository root whose apps/api/fixtures are checked')
    parser.add_argument('--out', type=Path, required=True, help='directory for logs, exports and contract-smoke.json')
    parser.add_argument('--rules-hash', help='pin the compiled Phase 4 runtime rules hash the exports must carry')
    args = parser.parse_args()
    if not args.replay.is_file():
        parser.error(f'{args.replay} is not a file')
    try:
        report = smoke(args.replay.resolve(), args.repo.resolve(), args.out.resolve(), args.rules_hash)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        parser.error(str(error))
    for row in report['fixtures']:
        status = 'PASS' if not row['problems'] else 'FAIL'
        print(f"{status} {Path(row['fixture']).name} ({row['kind']})" + ''.join('\n  ' + p for p in row['problems']), flush=True)
    print(f"contract smoke: {sum(not r['problems'] for r in report['fixtures'])}/{len(report['fixtures'])} fixtures agree; "
          f"rules hash {report['rules_hash'] or '(disagree/none)'}; report {args.out / 'contract-smoke.json'}", flush=True)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
