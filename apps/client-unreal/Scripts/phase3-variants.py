#!/usr/bin/env python3
"""Validate named scenario assertions and required actor/adapter evidence links.

Declarations are not execution. --evidence checks externally produced, hash-linked
precision reports; missing evidence fails. This does not execute or replace the
server tests, the native parser, or recording/replay gates.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys

SCENARIOS = Path(__file__).resolve().parents[1] / 'Scenarios'


def read_contract(root=SCENARIOS):
    data = json.loads((root / 'phase3-variants.json').read_text())
    if data.get('schema_version') != 1:
        raise ValueError('unsupported variant schema')
    spec = importlib.util.spec_from_file_location('variant_packs', Path(__file__).with_name('phase3-fixtures.py'))
    fixtures = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixtures)
    seen, roles = set(), set()
    for group in ('runtime_cases', 'precision_cases'):
        if not isinstance(data.get(group), list) or not data[group]:
            raise ValueError('missing variant case group ' + group)
        for case in data[group]:
            name = case.get('id')
            if not isinstance(name, str) or not re.fullmatch(r'[a-z0-9-]+', name) or name in seen:
                raise ValueError('duplicate/invalid variant id')
            seen.add(name)
            canonical = case['canonical']
            if Path(canonical).name != canonical or not (root / canonical).is_file():
                raise ValueError('missing canonical matrix scenario')
            header = (root / canonical).read_text().splitlines()[0]
            assertions = case['assertions']
            if not isinstance(assertions, list) or not assertions or len(set(assertions)) != len(assertions) or any(not isinstance(a, str) or not a for a in assertions):
                raise ValueError('variant needs nonempty unique authoritative assertions')
            if group == 'runtime_cases':
                if case['fixture'] not in fixtures.PHASE3 or not case['scenarios']:
                    raise ValueError('unknown fixture or missing runtime role')
                for role in case['scenarios']:
                    if role in roles or Path(role).is_absolute() or '..' in Path(role).parts:
                        raise ValueError('duplicate/escaping scenario role')
                    roles.add(role)
                    source = (root / role).read_text()
                    lines = source.splitlines()
                    if lines[0] != header or '# fixture: ' + case['fixture'] not in lines or '# variant: ' + name not in lines:
                        raise ValueError('variant header/fixture mismatch')
                    if any(a not in lines or not a.startswith(('nf.Expect ', 'nf.WaitFor ')) for a in assertions):
                        raise ValueError('variant assertion missing from scenario body')
                    waits = sum(int(l.split('#', 1)[0].split()[-1]) for l in lines if l.startswith('nf.WaitFor '))
                    budget = re.findall(r'^nf.Within ([0-9]+)$', source, re.M)
                    if len(budget) != 1 or int(budget[0]) < waits + 30:
                        raise ValueError('variant timeout lacks source wait/grace budget')
            else:
                if case.get('layer') != 'actor-adapter' or not case.get('setup') or not case.get('owner') or not case.get('requested_test'):
                    raise ValueError('precision variant missing owner/setup/test link')
                roll = case.get('deterministic_roll_fixture')
                if roll:
                    bounds = (0, 99) if roll.get('site') in ('calcEffectSuccess', 'death_penalty') else None
                    if (bounds is None or (roll.get('minimum'), roll.get('maximum')) != bounds
                            or type(roll.get('value')) is not int or roll['value'] not in bounds
                            or roll.get('require_full_ordered_draw_trace') is not True):
                        raise ValueError('invalid deterministic draw fixture')
                if case.get('input_fixture'):
                    path = Path(case['input_fixture'])
                    if path.is_absolute() or '..' in path.parts or hashlib.sha256((root / path).read_bytes()).hexdigest() != case.get('input_fixture_sha256'):
                        raise ValueError('precision input fixture missing or changed')
    files = {str(p.relative_to(root)) for p in (root / 'variants').glob('3-*.nfs')}
    if roles != files:
        raise ValueError('orphan/unlisted runtime variant')
    return data


def case_hash(case):
    return hashlib.sha256(json.dumps(case, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def evidence_errors(data, directory):
    errors = []
    for case in data['precision_cases']:
        try:
            report = json.loads((directory / (case['id'] + '.json')).read_text())
            if (report.get('schema_version') != 1 or report.get('case_id') != case['id']
                    or report.get('case_sha256') != case_hash(case) or report.get('passed') is not True
                    or report.get('test_name') != case['requested_test']
                    or set(report.get('assertions', {})) != set(case['assertions'])
                    or any(v is not True for v in report['assertions'].values())):
                raise ValueError('case/fixture/test/assertion verdict mismatch')
            files = report.get('artifacts')
            if not isinstance(files, dict) or not files:
                raise ValueError('no retained test/trace artifacts')
            for name, digest in files.items():
                if Path(name).is_absolute() or '..' in Path(name).parts or not re.fullmatch(r'[0-9a-f]{64}', digest):
                    raise ValueError('unsafe artifact reference')
                if hashlib.sha256((directory / name).read_bytes()).hexdigest() != digest:
                    raise ValueError('artifact hash mismatch')
            roll = case.get('deterministic_roll_fixture')
            if roll:
                draws = report.get('ordered_draw_trace')
                if not isinstance(draws, list) or not draws or not any(d.get('site') == roll['site'] and d.get('value') == roll['value'] and d.get('minimum') == roll['minimum'] and d.get('maximum') == roll['maximum'] for d in draws):
                    raise ValueError('missing actual ordered injected RNG trace')
        except (OSError, ValueError, KeyError, TypeError) as exc:
            errors.append(case['id'] + ': ' + str(exc))
    return errors


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--scenarios', type=Path, default=SCENARIOS)
    parser.add_argument('--evidence', type=Path)
    args = parser.parse_args()
    try:
        data = read_contract(args.scenarios)
        errors = evidence_errors(data, args.evidence) if args.evidence else []
        print(f"{len(data['runtime_cases'])} runtime source variants; {len(data['precision_cases'])} required actor/adapter cases")
        print('No execution/acceptance attestation from source validation.')
        for error in errors:
            print(error, file=sys.stderr)
        raise SystemExit(int(bool(errors)))
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.error(str(error))
