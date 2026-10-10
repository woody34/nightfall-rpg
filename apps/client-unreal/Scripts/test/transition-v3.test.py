#!/usr/bin/env python3
"""Schema3 consumer contract fixtures, not native engine execution evidence."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('gates', SCRIPTS / 'sim-gates.py')
gates = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gates)


class Coverage3Tests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.old = json.loads((SCRIPTS / 'test/fixtures/phase2-native-reconnect.transitions.json').read_text())
        self.current = copy.deepcopy(self.old)
        self.current['schema_version'] = 3
        for table, (states, reachable) in gates.V3_REACHABLE.items():
            self.current[table] = [{'from': a, 'to': b, 'reachable': (a, b) in reachable,
                                   'count': int((a, b) in reachable)} for a in sorted(states) for b in sorted(states)]
        for key in gates.V3_COUNTERS:
            self.current[key] = {'Completed': 2}

    def save(self, name, data):
        path = self.root / (name + '.json')
        path.write_text(json.dumps(data))
        return path

    def test_merge_preserves_all_v2_and_v3_facts_and_recomputes_totals(self):
        a = self.save('a', self.current)
        b = self.save('b', self.current)
        merged = gates.merge_transitions([a, b])
        for key in gates.transition_counters(self.current):
            self.assertEqual(merged[key], {name: count * 2 for name, count in self.current[key].items()})
        self.assertEqual(merged['owner_stats_updates'], self.old['owner_stats_updates'] * 2)
        self.assertEqual(merged['summary']['player_cast_states']['covered'], 2)
        self.assertEqual(merged['summary']['effect_states']['total'], 2)
        self.assertEqual(gates.read_transitions(self.save('merged', merged)), merged)
        self.assertEqual(gates.read_transitions(self.save('old', self.old)), self.old)

    def test_missing_unknown_negative_self_pair_and_resurrection_of_effect_id_fail_closed(self):
        for key in (*gates.V3_TABLES, *gates.V3_COUNTERS):
            data = copy.deepcopy(self.current)
            del data[key]
            with self.assertRaises((ValueError, KeyError)): gates.read_transitions(self.save('missing', data))
        for change in (lambda d: d.update(unknown_future_fact=1),
                       lambda d: d['effect_states'][0].update(unknown_handler_fact=1),
                       lambda d: d['regen_pulses'].update(HP=-1),
                       lambda d: d['effect_states'][0].update(count=1),
                       lambda d: d['effect_states'][-2].update(reachable=True),
                       lambda d: d['player_cast_states'][0].update(reachable=True),
                       lambda d: d['effect_pulses'].update({'uuid-123': 1})):
            data = copy.deepcopy(self.current)
            change(data)
            with self.assertRaises(ValueError): gates.read_transitions(self.save('bad', data))

    def test_mixed_schema_and_contradictory_same_recording_rejected(self):
        a = self.save('old', self.old)
        b = self.save('new', self.current)
        for paths in ([a, b], [b, a]):
            with self.assertRaises(ValueError): gates.merge_transitions(paths)
        for key in gates.V3_COUNTERS:
            original = self.current | {'recording_sha256': 'a' * 64}
            changed = copy.deepcopy(original)
            changed[key]['Completed'] += 1
            with self.assertRaisesRegex(ValueError, 'contradictory'):
                gates.merge_transitions([self.save('a', original), self.save('b', changed)], unique_recordings=True)

    def test_job_summary_reports_cast_effect_and_legacy_tables(self):
        for name, data in (('legacy', self.old), ('phase3', self.current)):
            folder = self.root / name
            folder.mkdir()
            (folder / 'coverage.transitions.json').write_text(json.dumps(data))
        result = subprocess.run([sys.executable, str(SCRIPTS / 'sim-ci-summary.py'), str(self.root)],
                                capture_output=True, text=True, check=True)
        self.assertIn('Transition schema 2', result.stdout)
        self.assertIn('Transition schema 3', result.stdout)
        self.assertIn('player_cast_states: 2/2', result.stdout)
        self.assertIn('effect_states: 2/2', result.stdout)


if __name__ == '__main__':
    unittest.main()
