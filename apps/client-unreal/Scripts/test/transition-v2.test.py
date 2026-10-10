#!/usr/bin/env python3
"""Strict schema-2 gates against captured native coverage; no replay/API/container run."""
import copy
import hashlib
import os
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
FIXTURE = SCRIPTS / 'test/fixtures/phase2-native-reconnect.transitions.json'
spec = importlib.util.spec_from_file_location('sim_gates', SCRIPTS / 'sim-gates.py')
gates = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gates)


class TransitionV2Tests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.native = json.loads(FIXTURE.read_text())

    def report(self, name, data=None, digest=None):
        data = copy.deepcopy(self.native if data is None else data)
        if digest is not None:
            data.update(recording_sha256=digest, recording_source=f'captured/{name}.nfr')
        path = self.root / f'{name}.json'
        path.write_text(json.dumps(data))
        return path

    def legacy(self):
        data = copy.deepcopy(self.native)
        data['schema_version'] = 1
        for key in (*gates.V2_COUNTERS, *gates.V2_SCALARS):
            del data[key]
        return data

    def assert_native_facts(self, data, multiple=1):
        self.assertEqual(data['schema_version'], 2)
        self.assertEqual(data['class_transfer_commands'], {'applied_without_rejection': 2 * multiple})
        self.assertEqual(data['class_changes'], {'1': multiple, '2': multiple})
        self.assertEqual(data['class_transfer_effects'], {'0->1': multiple, '1->2': multiple})
        self.assertEqual(data['owner_stats_updates'], 4 * multiple)
        self.assertEqual(data['intent_rejected'], {})
        self.assertTrue(all(row['count'] == 0 for row in data['player_attack_states']))

    def test_reads_verbatim_native_tool_output_and_single_merge(self):
        data = gates.read_transitions(FIXTURE)
        self.assertEqual(data, self.native)
        self.assert_native_facts(data)
        merged = gates.merge_transitions([FIXTURE])
        self.assert_native_facts(merged)
        for key in (*gates.TABLES, *gates.COUNTERS):
            self.assertEqual(merged[key], data[key])
        out = self.report('merged', merged)
        self.assertEqual(gates.read_transitions(out), merged)

    def test_merges_every_v2_fact_and_keeps_nested_provenance(self):
        a = self.report('a', digest='a' * 64)
        b = self.report('b', digest='b' * 64)
        first = gates.merge_transitions([a, b], unique_recordings=True)
        self.assert_native_facts(first, 2)
        for key in gates.TABLES:
            self.assertEqual([row['count'] for row in first[key]],
                             [row['count'] * 2 for row in self.native[key]])
        self.assertEqual(first['sources'], [str(a), str(b)])
        self.assertEqual(first['source_provenance'][str(b)]['recording_sha256'], 'b' * 64)
        unit = self.report('unit', first)
        suite = gates.merge_transitions([unit])
        self.assert_native_facts(suite, 2)
        self.assertEqual(suite['source_provenance'][str(unit)]['source_provenance'], first['source_provenance'])

    def test_unit_dedup_preserves_v2_facts_and_all_role_provenance(self):
        a = self.report('owner', digest='a' * 64)
        b = self.report('observer', digest='a' * 64)
        result = gates.merge_transitions([a, b], unique_recordings=True)
        self.assert_native_facts(result)
        self.assertEqual(result['sources'], [str(a)])
        self.assertEqual(set(result['source_provenance']), {str(a), str(b)})
        self.assertEqual(result['source_provenance'][str(b)]['recording_source'], 'captured/observer.nfr')
        # A suite counts distinct logical units, preserving the existing v1 policy.
        self.assert_native_facts(gates.merge_transitions([a, b]), 2)

    def test_same_recording_cannot_contradict_any_v2_fact(self):
        first = self.report('first', digest='a' * 64)
        for key in (*gates.V2_COUNTERS, *gates.V2_SCALARS):
            changed = copy.deepcopy(self.native)
            if key in gates.V2_SCALARS:
                changed[key] += 1
            else:
                name = next(iter(changed[key]))
                changed[key][name] += 1
            second = self.report('second', changed, digest='a' * 64)
            for unique in (False, True):
                with self.subTest(field=key, unique=unique), self.assertRaisesRegex(ValueError, 'contradictory'):
                    gates.merge_transitions([first, second], unique_recordings=unique)
        with self.assertRaisesRegex(ValueError, 'recording identity'):
            gates.merge_transitions([FIXTURE], unique_recordings=True)

    def test_pure_v1_retains_old_facts_without_invented_v2_observations(self):
        v1 = self.legacy()
        a = self.report('old-a', v1, digest='a' * 64)
        b = self.report('old-b', v1, digest='a' * 64)
        for unique, multiple in ((False, 2), (True, 1)):
            merged = gates.merge_transitions([a, b], unique_recordings=unique)
            self.assertEqual(merged['schema_version'], 1)
            for key in (*gates.V2_COUNTERS, *gates.V2_SCALARS): self.assertNotIn(key, merged)
            for key in gates.TABLES:
                self.assertEqual([row['count'] for row in merged[key]], [row['count'] * multiple for row in v1[key]])
            for key in gates.COUNTERS:
                self.assertEqual(merged[key], {name: count * multiple for name, count in v1[key].items()})
            self.assertEqual(gates.read_transitions(self.report('old-merged', merged)), merged)

    def test_mixed_versions_explicitly_fail_in_both_orders_and_dedup_modes(self):
        v1 = self.report('legacy', self.legacy(), digest='a' * 64)
        v2 = self.report('current', digest='b' * 64)
        for paths in ([v1, v2], [v2, v1]):
            for unique in (False, True):
                with self.subTest(paths=paths, unique=unique), self.assertRaisesRegex(ValueError, 'legacy v2 facts are unknown'):
                    gates.merge_transitions(paths, unique_recordings=unique)
        # Same digest cannot make a legacy report's missing v2 observations known.
        v2 = self.report('same', digest='a' * 64)
        with self.assertRaises(ValueError): gates.merge_transitions([v1, v2], unique_recordings=True)

    def test_invalid_versions_counter_types_missing_fields_and_names_fail_closed(self):
        bad_values = (True, False, -1, 1.0, '1', None, [], {})
        for key in (*gates.V2_COUNTERS, *gates.V2_SCALARS):
            data = copy.deepcopy(self.native)
            del data[key]
            with self.subTest(missing=key), self.assertRaises(ValueError):
                gates.read_transitions(self.report('missing', data))
            for bad in bad_values:
                data = copy.deepcopy(self.native)
                if key in gates.V2_COUNTERS:
                    data[key] = {next(iter(data[key])): bad}
                else:
                    data[key] = bad
                with self.subTest(field=key, bad=bad), self.assertRaises(ValueError):
                    gates.read_transitions(self.report('invalid', data))
        for key in gates.V2_COUNTERS:
            for bad in (None, [], 1, True, '', {'': 1}):
                data = copy.deepcopy(self.native)
                data[key] = bad
                with self.assertRaises(ValueError): gates.read_transitions(self.report('map', data))
        for version in (0, 3, -1, True, '2', 2.0, None):
            data = copy.deepcopy(self.native)
            data['schema_version'] = version
            with self.assertRaises(ValueError): gates.read_transitions(self.report('version', data))
        for key, name in (('class_transfer_commands', 'not-an-outcome'), ('class_changes', '01'),
                          ('class_changes', '4294967296'), ('class_transfer_effects', '1-to-2'),
                          ('class_transfer_effects', '1->4294967296')):
            data = copy.deepcopy(self.native)
            data[key] = {name: 1}
            with self.assertRaises(ValueError): gates.read_transitions(self.report('name', data))
        data = copy.deepcopy(self.native)
        data['unknown_future_counter'] = 3
        with self.assertRaisesRegex(ValueError, 'unknown schema 2'):
            gates.read_transitions(self.report('unknown-field', data))
        data = self.legacy()
        data['owner_stats_updates'] = 0
        with self.assertRaises(ValueError): gates.read_transitions(self.report('hidden-v2', data))

    def test_v2_keeps_strict_catalogue_and_lifecycle_guards(self):
        mutations = (lambda d: d['npc_intentions'].pop(),
                     lambda d: d['npc_intentions'][0].update(count=1),
                     lambda d: d['player_attack_states'][0].update(reachable=True),
                     lambda d: d['deaths'].pop('player'),
                     lambda d: d['life_incarnations'].update({'npc:2->2': 1}),
                     lambda d: d['intent_rejected'].update({'Invalid': True}))
        for change in mutations:
            data = copy.deepcopy(self.native)
            change(data)
            with self.assertRaises(ValueError): gates.read_transitions(self.report('lifecycle', data))

    def test_json_duplicate_keys_are_rejected_before_they_can_hide_facts(self):
        for original, duplicate in (
                ('"owner_stats_updates": 4', '"owner_stats_updates": 4, "owner_stats_updates": 0'),
                ('"1": 1', '"1": 1, "1": 0'),
                ('"schema_version": 2', '"schema_version": 2, "schema_version": 1')):
            path = self.root / 'duplicate.json'
            path.write_text(FIXTURE.read_text().replace(original, duplicate, 1))
            with self.assertRaisesRegex(ValueError, 'duplicate transition JSON key'):
                gates.read_transitions(path)

    def test_coverage_adds_recording_and_capture_provenance_without_dropping_v2(self):
        recording = self.root / 'group.nfr'
        recording.write_bytes(b'unit recording identity stub')
        digest = hashlib.sha256(recording.read_bytes()).hexdigest()
        capture = {'schema_version': 1, 'recording_sha256': digest, 'roles': {'owner': {}, 'observer': {}}}
        (self.root / 'group-recording.json').write_text(json.dumps(capture))
        out = self.root / 'group.coverage.transitions.json'
        def emit(command, **kwargs):
            self.assertEqual(command[-5:], ['coverage', '--file', str(recording), '--out', str(out)])
            out.write_bytes(FIXTURE.read_bytes())
        with patch.dict(os.environ, {'SIM_COVERAGE_CMD': 'stub-coverage'}), \
                patch.object(gates.subprocess, 'run', emit):
            gates.coverage(recording, out)
        result = gates.read_transitions(out)
        self.assert_native_facts(result)
        self.assertEqual(result['recording_source'], str(recording))
        self.assertEqual(result['recording_sha256'], digest)
        self.assertEqual(result['group_capture'], capture)
        merged = gates.merge_transitions([out], unique_recordings=True)
        self.assert_native_facts(merged)
        self.assertEqual(merged['source_provenance'][str(out)]['group_capture'], capture)

    def test_merge_cli_accepts_native_v2_without_disabling_gates(self):
        out = self.root / 'cli.json'
        result = subprocess.run([sys.executable, str(SCRIPTS / 'sim-gates.py'), 'merge-transitions',
                                 '--out', str(out), str(FIXTURE)], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_native_facts(gates.read_transitions(out))
        self.assertIn('transitions player_attack_states: 0/4', result.stdout)
        self.assertEqual(set(gates.transition_summary(self.native)), set(gates.TABLES))


if __name__ == '__main__':
    unittest.main()
