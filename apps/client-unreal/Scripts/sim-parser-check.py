#!/usr/bin/env python3
"""Fail closed on DSL names absent from the merged client's registry sources.

This checks source registration and basic Phase3 argument grammar, not Unreal
execution or predicate truth. Missing projections still fail at runtime.
"""
import argparse
import json
import importlib.util
from pathlib import Path
import re
import shlex
import sys
import uuid

PROJECT = Path(__file__).resolve().parents[1]
_p4_spec = importlib.util.spec_from_file_location('phase4_grammar', Path(__file__).with_name('phase4-grammar.py'))
phase4 = importlib.util.module_from_spec(_p4_spec)
_p4_spec.loader.exec_module(phase4)



def clean_cpp(source):
    # Preserve strings; comments cannot turn a proposed name into a registration.
    return re.sub(r'"(?:\\.|[^"\\])*"|//[^\n]*|/\*.*?\*/',
                  lambda m: m[0] if m[0].startswith('"') else ' ', source, flags=re.S)


def registry(project=PROJECT):
    root = project / 'Source/Nightfall'
    paths = sorted(set(root.glob('Bot/*Predicates*.cpp')))
    if not paths:
        raise ValueError('client predicate registry sources are missing')
    predicates, commands = set(), set()
    for path in paths:
        source = clean_cpp(path.read_text())
        predicates.update(re.findall(r'\b(?:Register(?:Flag|Number|Equality)?\s*\(\s*\{?\s*|Number\s*\(\s*(?:R\s*,\s*)?|Custom\s*\(\s*)TEXT\("([a-z_]+)"\)', source))
        # Current client uses literal initializer lists for its grouped registrations.
        for match in re.finditer(r'for\s*\(\s*const FString& Name\s*:\s*\{(.*?)\}\s*\)', source, re.S):
            body = source[match.end():].lstrip()
            if re.match(r'(?:R\.)?(?:Register|RegisterEquality|Number)\s*\(', body):
                predicates.update(re.findall(r'FString\(TEXT\("([a-z_]+)"\)\)', match[1]))
        for match in re.finditer(r'const FString Name\s*=\s*\w+\s*\?\s*TEXT\("([a-z_]+)"\)\s*:\s*TEXT\("([a-z_]+)"\)\s*;\s*Register\(', source):
            predicates.update(match.groups())
    for path in sorted(root.rglob('*.cpp')):
        source = clean_cpp(path.read_text())
        commands.update(name.lower() for name in re.findall(
            r'(?:FAutoConsoleCommand\w*\s+\w+\s*\(|RegisterConsoleCommand\s*\()\s*TEXT\("(nf\.[A-Za-z]+)"\)', source))
    # The scenario parser owns these directives in BotSteps.cpp.
    steps = clean_cpp((root / 'Bot/BotSteps.cpp').read_text())
    for directive in ('nf.WaitFor', 'nf.Expect', 'nf.Within', 'nf.Sleep'):
        if f'TEXT("{directive}")' in steps:
            commands.add(directive.lower())
    return predicates, commands


def phase3_arguments(tokens):
    item_command = phase4.command(tokens)
    if item_command is not None:
        return item_command
    head, args = tokens[0].lower(), tokens[1:]
    def positive(value, maximum):
        return bool(re.fullmatch(r'[0-9]+', value)) and 0 < int(value) <= maximum
    def guid(value):
        try:
            uuid.UUID(value)
            return True
        except ValueError:
            return False
    if head == 'nf.cast':
        return len(args) in (1, 2) and positive(args[0], 2**32-1) and (len(args) == 1 or args[1] in ('self', 'target') or guid(args[1]))
    if head == 'nf.castif':
        return len(args) == 2 and positive(args[0], 2**32-1) and args[1] in ('target_alive', 'effect_absent')
    if head == 'nf.learn':
        return len(args) in (2, 3) and all(positive(a, 2**32-1) for a in args[:2]) and (len(args) == 2 or guid(args[2]))
    if head in ('nf.cancelcast', 'nf.learnableskills', 'nf.skillcatalog'):
        return not args
    if head == 'nf.acceptresurrect':
        return len(args) <= 2 and (not args or args[0] == 'latest' or positive(args[0], 2**64-1)) and (len(args) < 2 or args[1] in ('accept', 'decline'))
    return True


def validate(paths, project=PROJECT):
    predicates, commands = registry(project)
    unavailable = set()
    for path in (project / 'Source/Nightfall/Bot').glob('*Predicates*.cpp'):
        unavailable.update(re.findall(r'if\s*\(\s*Name\s*==\s*TEXT\("([a-z_]+)"\)\s*\)\s*return\s*\{false\s*,\s*TEXT\("unknown:', clean_cpp(path.read_text())))
    failures = []
    for path in paths:
        for line, source in enumerate(path.read_text().splitlines(), 1):
            tokens = shlex.split(source.split('#', 1)[0])
            if not tokens:
                continue
            error = None
            head = tokens[0].lower()
            if head.startswith('nf.') and head not in commands:
                error = 'unregistered command ' + tokens[0]
            elif head in ('nf.waitfor', 'nf.expect'):
                if len(tokens) < (3 if head == 'nf.waitfor' else 2):
                    error = 'missing predicate or timeout'
                elif tokens[1] not in predicates:
                    error = 'unregistered predicate ' + tokens[1]
                elif tokens[1] in unavailable:
                    error = 'unavailable received projection ' + tokens[1]
                elif phase4.predicate(tokens[1:-1] if head == 'nf.waitfor' else tokens[1:]) is False:
                    error = 'invalid Phase4 predicate arguments'
                elif head == 'nf.waitfor' and not re.fullmatch(r'(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)', tokens[-1]):
                    error = 'invalid timeout'
                elif head == 'nf.waitfor' and float(tokens[-1]) <= 0:
                    error = 'nonpositive timeout'
            elif not phase3_arguments(tokens):
                error = 'invalid command arguments'
            if error:
                failures.append({'scenario': path.name, 'line': line, 'error': error})
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', type=Path, default=PROJECT)
    parser.add_argument('--json', action='store_true')
    parser.add_argument('scenarios', nargs='+', type=Path)
    args = parser.parse_args()
    try:
        failures = validate(args.scenarios, args.project)
    except (OSError, ValueError) as error:
        print(str(error), file=sys.stderr)
        return 2
    if args.json:
        print(json.dumps({'compatible': not failures, 'failures': failures}, indent=2))
    else:
        for row in failures:
            print(f"{row['scenario']}:{row['line']}: {row['error']}", file=sys.stderr)
        print(f'{len(args.scenarios)} scenarios checked; {len(failures)} source compatibility failures')
    return int(bool(failures))


if __name__ == '__main__':
    raise SystemExit(main())
