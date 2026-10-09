#!/usr/bin/env python3
"""Generate an offline trace and attach its relative path to scenario JUnit output."""
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import xml.etree.ElementTree as ET

recording, report = map(Path, sys.argv[1:3])
out = recording.with_suffix('.trace.html')
command = shlex.split(os.environ.get('SIM_TRACE_CMD', 'cargo run --quiet --bin nightfall-replay --'))
command += ['trace', '--file', str(recording), '--out', str(out)]
tree = ET.parse(report) if report.exists() else None
if tree is not None:
    own = tree.find(".//property[@name='own_entity_id']")
    if own is not None and own.get('value'):
        command += ['--session', own.get('value')]
if tree is not None:
    tick = tree.find(".//property[@name='failure_tick']")
    if tick is not None and tick.get('value', '').isdigit():
        command += ['--fail-tick', tick.get('value')]
replay_log = recording.with_suffix('.replay.log')
if replay_log.exists():
    divergence = re.search(r'\bdivergence at tick (\d+)', replay_log.read_text())
    if divergence and '--fail-tick' not in command:
        command += ['--fail-tick', divergence.group(1)]
with recording.with_suffix('.trace.log').open('w') as log:
    subprocess.run(command, cwd=Path(__file__).resolve().parents[3], stdout=log, stderr=subprocess.STDOUT, check=True)
if tree is not None:
    root = tree.getroot()
    suites = [root] if root.tag == 'testsuite' else root.findall('.//testsuite')
    for suite in suites:
        props = suite.find('properties')
        if props is None:
            props = ET.SubElement(suite, 'properties')
        ET.SubElement(props, 'property', name='trace', value=out.name)
        for failure in suite.findall('.//failure'):
            failure.text = (failure.text or '') + '\nTrace: ' + out.name
        output = suite.find('system-out')
        if output is None:
            output = ET.SubElement(suite, 'system-out')
        output.text = (output.text or '') + '\n[[ATTACHMENT|' + out.name + ']]\n'
    tree.write(report, encoding='utf-8', xml_declaration=True)
