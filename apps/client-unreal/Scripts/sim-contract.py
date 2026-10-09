#!/usr/bin/env python3
"""Aggregate descriptor-backed runtime contract coverage; gate on loss of historical presence.

Counts are decimal strings so uint64 counters survive JSON consumers. Baselines record presence,
never a run-dependent frequency. Promotion is explicit and preserves every historical positive.
"""
import argparse
import json
from pathlib import Path
import sys
import xml.etree.ElementTree as ET

GROUPS = ('intents', 'payloads', 'events', 'reasons', 'close_codes')


def read(path):
    value = json.loads(Path(path).read_text())
    if value.get('schema_version') != 1 or not isinstance(value.get('counts'), dict):
        raise ValueError(f'{path}: unsupported or missing contract schema')
    for group in GROUPS:
        entries = value['counts'].get(group)
        if not isinstance(entries, dict) or not entries:
            raise ValueError(f'{path}: missing descriptor group {group}')
        for name, count in entries.items():
            if not isinstance(name, str) or not name or isinstance(count, bool) or not str(count).isdigit():
                raise ValueError(f'{path}: invalid counter {group}.{name}')
    return value


def merge(paths):
    if not paths:
        raise ValueError('no current-run coverage inputs')
    counts = {group: {} for group in GROUPS}
    numbers = None
    frames = {'client_frames': 0, 'server_frames': 0}
    sources = []
    for path in paths:
        data = read(path)
        # Different binaries/schema catalogues cannot be silently combined.
        if numbers is None:
            numbers = data.get('wire_numbers')
        elif data.get('wire_numbers') != numbers:
            raise ValueError(f'{path}: compiled descriptor numbers differ between clients')
        for group in GROUPS:
            if counts[group] and group not in ('close_codes', 'reasons') and set(counts[group]) != set(data['counts'][group]):
                raise ValueError(f'{path}: descriptor catalogue differs for {group}')
            for name, count in data['counts'][group].items():
                counts[group][name] = counts[group].get(name, 0) + int(count)
        for frame in frames:
            frames[frame] += int(data.get(frame, 0))
        sources.append(str(Path(path)))
    summary = {}
    for group, entries in counts.items():
        missing = sorted(name for name, count in entries.items() if count == 0)
        summary[group] = {'covered': len(entries) - len(missing), 'total': len(entries), 'missing': missing}
    return {'schema_version': 1, 'catalogue_source': 'compiled protobuf descriptors',
            'sources': sources, **{k: str(v) for k, v in frames.items()},
            'counts': {g: {k: str(v) for k, v in sorted(entries.items())} for g, entries in counts.items()},
            'wire_numbers': numbers, 'summary': summary}


def regressions(current, baseline):
    return sorted(f'{group}.{name}' for group, entries in baseline['counts'].items()
                  for name, count in entries.items() if int(count) > 0
                  and int(current['counts'].get(group, {}).get(name, 0)) == 0)


def promote(candidate, previous=None):
    # Keep historical observations even when an explicitly promoted partial run has fewer cases.
    counts = {group: {name: '1' if int(count) > 0 else '0' for name, count in entries.items()}
              for group, entries in candidate['counts'].items()}
    if previous:
        for group, entries in previous['counts'].items():
            for name, count in entries.items():
                if int(count) > 0:
                    counts[group][name] = '1'
    scenarios = set(previous.get('observed_scenarios', [])) if previous else set()
    scenarios.update(Path(path).name.removesuffix('.coverage.contract.json') for path in candidate.get('sources', []))
    return {'schema_version': 1, 'baseline_kind': 'historical observed presence',
            'counts': counts, 'wire_numbers': candidate.get('wire_numbers'), 'observed_scenarios': sorted(scenarios)}


def attach(report, coverage):
    tree = ET.parse(report)
    root = tree.getroot()
    suites = [root] if root.tag == 'testsuite' else root.findall('.//testsuite')
    for suite in suites:
        props = suite.find('properties')
        if props is None:
            props = ET.SubElement(suite, 'properties')
        for group, entries in coverage['counts'].items():
            for name, count in entries.items():
                key = f'contract.{group}.{name}'
                # Replace to make aggregation idempotent.
                for old in list(props.findall('property')):
                    if old.get('name') == key:
                        props.remove(old)
                ET.SubElement(props, 'property', name=key, value=str(count))
    tree.write(report, encoding='utf-8', xml_declaration=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    gate = sub.add_parser('merge')
    gate.add_argument('--out', required=True, type=Path)
    gate.add_argument('--baseline', type=Path)
    gate.add_argument('--report', type=Path)
    gate.add_argument('inputs', nargs='+', type=Path)
    baseline = sub.add_parser('promote')
    baseline.add_argument('--candidate', required=True, type=Path)
    baseline.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    try:
        if args.command == 'promote':
            data = promote(read(args.candidate), read(args.out) if args.out.exists() else None)
        else:
            data = merge(args.inputs)
            data['regressions'] = regressions(data, read(args.baseline)) if args.baseline else []
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')
        if args.command == 'merge':
            if args.report:
                attach(args.report, data)
            for group, summary in data['summary'].items():
                print(f"contract {group}: {summary['covered']}/{summary['total']}; missing: {', '.join(summary['missing']) or '(none)'}")
            if data['regressions']:
                print('contract regression: ' + ', '.join(data['regressions']), file=sys.stderr)
                return 1
    except (OSError, ValueError, ET.ParseError, TypeError) as error:
        print(f'contract coverage failed: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
