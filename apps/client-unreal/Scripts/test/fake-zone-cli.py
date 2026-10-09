#!/usr/bin/env python3
"""Rust CLI-shaped fake: each live export advances one shared epoch prefix."""
import importlib.util
import json
import os
from pathlib import Path
import sys
import xml.etree.ElementTree as ET

spec = importlib.util.spec_from_file_location('fixtures', Path(__file__).with_name('fake-artifacts.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)
command, *args = sys.argv[1:]
flags = {}
while args:
    key = args.pop(0)
    flags[key] = True if key in ('--latest', '--live') else args.pop(0)
calls = Path(os.environ['FAKE_ZONE_CALLS'])
with calls.open('a') as log:
    log.write(json.dumps({'command': command, 'flags': flags}) + '\n')
if command == 'export':
    assert '--session' not in flags, 'canonical export must not select a role'
    assert flags['--latest'] and flags['--live']
    if os.environ.get('FAKE_EXPORT_FAIL') == '1':
        sys.exit(2)
    exports = sum(json.loads(line)['command'] == 'export' and
                  json.loads(line)['flags']['--out'] == flags['--out']
                  for line in calls.read_text().splitlines())
    sessions = []
    claim = json.loads((Path(flags['--out']).parent / '.sim-run.json').read_text())
    for name in claim['scenarios']:
        report = Path(os.environ['SIM_SAVED_DIR']) / f'{name}.xml'
        if not report.exists():
            continue
        if report.stem == os.environ.get('FAKE_SESSION_OMIT'):
            continue
        own = ET.parse(report).find(".//property[@name='own_entity_id']")
        if own is not None:
            sessions.append(own.get('value'))
    # Realistic prefix overlap: another durable tick arrives between any two exports.
    value = {'zone': int(flags['--zone']), 'epoch': 1, 'sessions': sessions,
             'records': [{'tick': tick} for tick in range(1, 4 + exports)]}
    Path(flags['--out']).write_text(json.dumps(value))
else:
    value = json.loads(Path(flags['--file']).read_text())
    if command == 'check':
        if '--session' in flags and flags['--session'] not in value['sessions']:
            sys.exit(2)
        if '--session' not in flags and os.environ.get('FAKE_REPLAY_FAIL') == '1':
            sys.exit(1)
    elif command == 'coverage':
        Path(flags['--out']).write_text(json.dumps(fixtures.transition_fixture(len(value['records']))))
    elif command == 'trace':
        if os.environ.get('FAKE_TRACE_FAIL') == '1':
            sys.exit(2)
        if '--session' in flags and flags['--session'] not in value['sessions']:
            sys.exit(2)
        Path(flags['--out']).write_text('<!doctype html><title>Offline zone trace</title>')
    else:
        sys.exit(2)
