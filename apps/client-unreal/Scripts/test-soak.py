#!/usr/bin/env python3
"""Host-only checks for shared-zone isolation and non-vacuous soak reporting."""
import importlib.util
import io
import json
import math
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


fixtures, reporting, compose = load('soak-fixtures'), load('soak-report'), load('soak-compose')


class SoakTests(unittest.TestCase):
    def test_compose_does_not_join_development_network_or_volumes(self):
        original = {'name': 'dev', 'services': {'db': {'ports': [{'published': '5432', 'target': 5432}]}},
                    'networks': {'default': {'name': 'dev_default'}},
                    'volumes': {'data': {'name': 'dev_data'}}}
        isolated = compose.isolate(original, 10000)
        self.assertNotIn('name', isolated)
        self.assertEqual(isolated['networks']['default'], {})
        self.assertEqual(isolated['volumes']['data'], {})
        self.assertEqual(isolated['services']['db']['ports'][0]['published'], '15432')
        self.assertEqual(original['networks']['default']['name'], 'dev_default')
        original['networks']['default']['external'] = True
        with self.assertRaises(ValueError):
            compose.isolate(original, 10000)

    def test_lanes_keep_assertions_and_dont_cross_aggro(self):
        source = (HERE.parent / 'Scenarios/1-kill-one-monster.nfs').read_text()
        asserts = [s for s in source.splitlines() if s.startswith(('nf.Expect', 'nf.WaitFor')) and 'own_at' not in s]
        for count in (1, 2, 8):
            with tempfile.TemporaryDirectory() as tmp:
                out = Path(tmp)
                fixtures.prepare(REPO, out, count)
                zone = tomllib.loads((out / 'data/zones/test_zone.toml').read_text())
                homes = [s['home'] for s in zone['spawn_slots']]
                self.assertEqual(len(homes), count)
                for i, home in enumerate(homes):
                    scenario = (out / f'client-{i+1:02}/Scenarios/1-kill-one-monster.nfs').read_text()
                    actual = [s for s in scenario.splitlines() if s.startswith(('nf.Expect', 'nf.WaitFor')) and 'own_at' not in s]
                    self.assertEqual(actual, asserts)
                    pos = (0, 0)
                    for move in [s for s in scenario.splitlines() if s.startswith('nf.ClickMove')]:
                        end = tuple(map(int, move.split()[1:]))
                        self.assertLessEqual(math.dist(pos, end), 64)
                        pos = end
                    self.assertLess(math.dist(pos, home), 5)
                    for j, other in enumerate(homes):
                        if i != j:
                            self.assertGreater(math.dist(home, other), 2 * math.sqrt(2) * 9.375 + 8)

    def test_missing_clients_and_metrics_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            with patch('urllib.request.urlopen', side_effect=OSError('offline')):
                self.assertEqual(reporting.report(Path(tmp), 2, 120, 10, 150, 0), 1)
            self.assertEqual(len(json.loads((Path(tmp) / 'report.json').read_text())['errors']), 3)

    def test_duration_and_p99_gates(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            role = out / 'client-01'
            role.mkdir()
            (role / '1-kill-one-monster.xml').write_text('''<testsuites><testsuite time="125"><properties>
              <property name="loop_iterations" value="3"/><property name="exit_code" value="0"/>
              </properties><testcase name="kill"/></testsuite></testsuites>''')
            for value, expected in [('0.012', 0), ('0.051', 1), ('NaN', 1)]:
                payload = json.dumps({'data': {'result': [{'value': [150, value]}]}}).encode()
                with patch('urllib.request.urlopen', side_effect=lambda *a, **kw: io.BytesIO(payload)):
                    self.assertEqual(reporting.report(out, 1, 120, 10, 150, 0), expected)
            with patch('urllib.request.urlopen', side_effect=lambda *a, **kw: io.BytesIO(json.dumps({'data': {'result': [{'value': [150, '0.012']}]}}).encode())):
                self.assertEqual(reporting.report(out, 1, 130, 10, 150, 0), 1)
                (out / 'runner-errors.txt').write_text('API shutdown forced; final metrics unavailable')
                self.assertEqual(reporting.report(out, 1, 120, 10, 150, 0), 1)


if __name__ == '__main__':
    unittest.main()
