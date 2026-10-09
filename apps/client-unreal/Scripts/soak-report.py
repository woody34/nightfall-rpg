#!/usr/bin/env python3
"""Collect per-role JUnit, combat telemetry and the exact Grafana dashboard range."""
import argparse
import base64
import json
import math
import os
from pathlib import Path
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET


def report(out: Path, clients: int, seconds: int, start: int, end: int, uat_exit: int, metrics_at: int | None = None) -> int:
    errors = []
    metrics_at = metrics_at or end
    runner_errors = out / 'runner-errors.txt'
    if runner_errors.exists():
        errors.extend(runner_errors.read_text().splitlines())
    suites = ET.Element('testsuites')
    roles = []
    for index in range(1, clients + 1):
        name = f'client-{index:02}'
        path = out / name / '1-kill-one-monster.xml'
        try:
            root = ET.parse(path).getroot()
            suite = root.find('testsuite')
            if suite is None:
                raise ValueError('no testsuite')
            properties = {p.attrib['name']: p.attrib['value'] for p in suite.findall('./properties/property')}
            iterations = int(properties.get('loop_iterations', '0'))
            elapsed = float(suite.attrib['time'])
            if root.findall('.//failure') or root.findall('.//error') or properties.get('exit_code') != '0':
                errors.append(f'{name}: failed assertions, sentinel, or process')
            if iterations < 1 or not math.isfinite(elapsed) or elapsed < seconds:
                errors.append(f'{name}: loop ended early ({iterations} iterations, {elapsed}s)')
            suite.set('name', name)
            suites.append(suite)
            roles.append({'client': name, 'iterations': iterations, 'seconds': elapsed})
        except (OSError, ValueError, KeyError, ET.ParseError) as exc:
            errors.append(f'{name}: missing or invalid JUnit: {exc}')
    grafana = os.environ.get('SOAK_GRAFANA_URL', 'http://localhost:3300').rstrip('/')
    dashboard = f'{grafana}/d/nightfall-api?from={start * 1000}&to={metrics_at * 1000}'
    window = max(1, metrics_at - start)
    # Use the entire run window; rate over a fixed 5m window would include previous tests.
    query = f'histogram_quantile(0.99, sum by (le) (increase(nightfall_combat_tick_duration_seconds_bucket[{window}s])))'
    p99 = None
    telemetry = {}
    credentials = f"{os.environ.get('SOAK_GRAFANA_USER', 'admin')}:{os.environ.get('SOAK_GRAFANA_PASSWORD', 'admin')}"
    def fetch(expression):
        params = urllib.parse.urlencode({'query': expression, 'time': metrics_at})
        request = urllib.request.Request(f'{grafana}/api/datasources/proxy/uid/prometheus/api/v1/query?{params}')
        request.add_header('Authorization', 'Basic ' + base64.b64encode(credentials.encode()).decode())
        with urllib.request.urlopen(request, timeout=15) as response:
            return json.load(response)
    try:
        telemetry['p99'] = fetch(query)
        values = telemetry['p99']['data']['result']
        if len(values) != 1:
            raise ValueError('expected one combat tick p99 series')
        p99 = float(values[0]['value'][1])
        if not math.isfinite(p99) or p99 < 0:
            raise ValueError('combat tick p99 is unavailable')
        if p99 > .050:
            errors.append(f'combat tick p99 {p99 * 1000:.3f}ms exceeds 50ms')
        telemetry['combat'] = fetch('{__name__=~"nightfall_combat_.*"}')
    except (OSError, ValueError, KeyError, IndexError) as exc:
        if p99 is not None and not math.isfinite(p99):
            p99 = None
        errors.append(f'telemetry unavailable: {exc}')
    (out / 'combat-telemetry.json').write_text(json.dumps(telemetry, indent=2) + '\n')
    if uat_exit:
        errors.append(f'Gauntlet/UAT exited {uat_exit}')
    gate = ET.SubElement(suites, 'testsuite', name='soak-gates', tests='1', failures=str(int(bool(errors))))
    case = ET.SubElement(gate, 'testcase', name='processes, duration and combat tick p99')
    if errors:
        ET.SubElement(case, 'failure', message='; '.join(errors)).text = '\n'.join(errors)
    ET.ElementTree(suites).write(out / 'soak.xml', encoding='utf-8', xml_declaration=True)
    result = {'passed': not errors, 'clients': clients, 'requested_seconds': seconds,
              'start_unix': start, 'end_unix': end, 'metrics_end_unix': metrics_at, 'tick_p99_ms': p99 * 1000 if p99 is not None else None,
              'grafana': dashboard, 'roles': roles, 'errors': errors,
              'zone_seed': {'zone': 1, 'epoch': 1} if (out / 'soak.nfr').exists() else None}
    (out / 'report.json').write_text(json.dumps(result, indent=2) + '\n')
    lines = [f"Soak: {'PASS' if not errors else 'FAIL'}; {clients} clients, {seconds}s requested", dashboard]
    lines += [f"{r['client']}: {r['iterations']} iterations, {r['seconds']:.1f}s" for r in roles]
    lines += [f'Combat tick p99: {p99 * 1000:.3f}ms'] if p99 is not None else []
    lines += errors
    (out / 'summary.txt').write_text('\n'.join(lines) + '\n')
    print('\n'.join(lines))
    return int(bool(errors))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('out', type=Path)
    for name in ('clients', 'seconds', 'start', 'end', 'uat_exit'):
        p.add_argument(name, type=int)
    p.add_argument('--metrics-at', type=int)
    a = p.parse_args()
    raise SystemExit(report(a.out, a.clients, a.seconds, a.start, a.end, a.uat_exit, a.metrics_at))
