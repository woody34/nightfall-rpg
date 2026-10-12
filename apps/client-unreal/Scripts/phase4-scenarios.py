#!/usr/bin/env python3
"""Offline grammar/plan contract audit. Never claims C++ compatibility or live PASS."""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import shlex
import sys

ROOT = Path(__file__).resolve().parents[3]
SCRIPTS = Path(__file__).resolve().parent

def load(name, file):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / file)
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module

grammar = load('scenario_p4_grammar', 'phase4-grammar.py')
client = load('scenario_p4_client', 'sim-parser-check.py')
fixtures = load('scenario_p4_fixtures', 'phase4-fixtures.py')


def validate(paths, project=SCRIPTS.parent):
    failures = []
    planned_commands = {k.lower() for k in grammar.COMMANDS}
    for failure in client.validate(paths, project):
        message = failure['error']
        if message.startswith('unregistered command ') and message.split()[-1].lower() in planned_commands: continue
        if message.startswith('unregistered predicate ') and message.split()[-1] in grammar.PREDICATES: continue
        failures.append(failure)
    for path in paths:
        if fixtures.PHASE4.isdisjoint(re.findall(r'^# fixture: (\S+)$', path.read_text(), re.M)):
            failures.append(dict(scenario=path.name, error='missing Phase4 fixture contract'))
        waits = 0
        assertions = 0
        for line, source in enumerate(path.read_text().splitlines(), 1):
            tokens = shlex.split(source.split('#', 1)[0])
            if not tokens: continue
            valid = grammar.command(tokens)
            if tokens[0].lower() in ('nf.expect', 'nf.waitfor'):
                pred = tokens[1:-1] if tokens[0].lower() == 'nf.waitfor' else tokens[1:]
                valid = grammar.predicate(pred)
                assertions += 1
                if tokens[0].lower() == 'nf.waitfor': waits += int(tokens[-1])
            if valid is False: failures.append(dict(scenario=path.name, line=line, error='invalid published Phase4 syntax'))
        budgets = re.findall(r'^nf.Within (\d+)$', path.read_text(), re.M)
        if len(budgets) != 1 or int(budgets[0]) < waits + 30 or assertions < 2:
            failures.append(dict(scenario=path.name, error='missing nonvacuous assertions/wait budget'))
        if not re.findall(r'^# item-gate: \w+$', path.read_text(), re.M):
            failures.append(dict(scenario=path.name, error='missing replay oracle gates'))
    return failures


def matrix(directory=SCRIPTS.parent / 'Scenarios'):
    plan = (ROOT / 'docs/plans/phase-4-items-and-equipment.md').read_text()
    rows = re.findall(r'\| `(4-[^`]+\.nfs)` \| `(# covers: [^`]+)` \| (phase4-[a-z-]+):', plan)
    if len(rows) != 23: raise ValueError('canonical Phase4 plan matrix changed')
    paths = []
    for name, header, pack in rows:
        path = directory / name
        lines = path.read_text().splitlines()
        if lines[:2] != [header, '# fixture: ' + pack]: raise ValueError('canonical scenario header/fixture mismatch: '+name)
        paths.append(path)
    if set(directory.glob('4-*.nfs')) != set(paths): raise ValueError('orphan/missing canonical scenario')
    return paths


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('paths', nargs='*', type=Path)
    args = parser.parse_args()
    failures = validate(args.paths or matrix())
    print(json.dumps(dict(contract_valid=not failures, live_compatibility='pending E5 registration', failures=failures), indent=2))
    sys.exit(bool(failures))
