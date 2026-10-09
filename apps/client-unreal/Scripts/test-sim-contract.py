#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import tempfile
import unittest
import json

spec = importlib.util.spec_from_file_location('contract', Path(__file__).with_name('sim-contract.py'))
contract = importlib.util.module_from_spec(spec)
spec.loader.exec_module(contract)


def fixture(respawn):
    return {'schema_version': 1, 'client_frames': '2', 'server_frames': '3',
            'wire_numbers': {'intents': {'respawn': 7, 'keep_alive': 8}},
            'counts': {'intents': {'respawn': str(respawn), 'keep_alive': '1'},
                       'payloads': {'event': '1'}, 'events': {'respawned': '1'},
                       'reasons': {'REJECT_REASON_INVALID': '0'}, 'close_codes': {'4409': '0'}}}


class ContractTests(unittest.TestCase):
    def test_removing_unique_respawn_scenario_regresses(self):
        with tempfile.TemporaryDirectory() as directory:
            first, second = (Path(directory) / name for name in ('only-respawn.json', 'login.json'))
            first.write_text(json.dumps(fixture(1)))
            second.write_text(json.dumps(fixture(0)))
            baseline = contract.promote(contract.merge([first, second]))
            current = contract.merge([second])
            self.assertEqual(contract.regressions(current, baseline), ['intents.respawn'])

    def test_counts_reduce_without_presence_regression(self):
        self.assertEqual(contract.regressions(fixture(1), fixture(100)), [])

    def test_promotion_never_erases_historical_positive(self):
        self.assertEqual(contract.promote(fixture(0), fixture(1))['counts']['intents']['respawn'], '1')

    def test_unseen_descriptor_case_is_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'run.json'
            path.write_text(json.dumps(fixture(0)))
            result = contract.merge([path])
            self.assertEqual(result['summary']['intents'], {'covered': 1, 'total': 2, 'missing': ['respawn']})

    def test_empty_or_mismatched_schema_cannot_pass(self):
        with self.assertRaises(ValueError):
            contract.merge([])
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'empty.json'
            path.write_text('{}')
            with self.assertRaises(ValueError):
                contract.merge([path])

    def test_unknown_reason_is_unioned_without_descriptor_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            first, second = (Path(directory) / name for name in ('known.json', 'unknown.json'))
            data = fixture(0)
            first.write_text(json.dumps(data))
            data['counts']['reasons']['unknown_99'] = '1'
            second.write_text(json.dumps(data))
            self.assertEqual(contract.merge([first, second])['counts']['reasons']['unknown_99'], '1')

    def test_uint64_counters_are_exact(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'large.json'
            path.write_text(json.dumps(fixture(2**64 - 1)))
            self.assertEqual(contract.merge([path])['counts']['intents']['respawn'], str(2**64 - 1))


if __name__ == '__main__':
    unittest.main()
