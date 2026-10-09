#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('audit', Path(__file__).with_name('sim-contract-audit.py'))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class AuditTests(unittest.TestCase):
    def coverage(self):
        return {'schema_version': 1, 'client_frames': '1', 'server_frames': '1',
                'wire_numbers': {'intents': {'respawn': 15}, 'payloads': {'rejected': 3},
                                 'events': {'attack_started': 11}, 'reasons': {'REJECT_REASON_INVALID': 6}},
                'counts': {'intents': {'respawn': '1'}, 'payloads': {'rejected': '1'},
                           'events': {'attack_started': '0'}, 'reasons': {'REJECT_REASON_INVALID': '1'},
                           'close_codes': {'4409': '1'}}}

    def records(self):
        # Append-only application audit wrappers: SessionIn.frame field7; SessionOut.frame field5.
        return [('nightfall.session.test.in', bytes.fromhex('0a01013a027a00')),
                ('nightfall.session.test.out', bytes.fromhex('0a01012a061a0408011006'))]

    def test_same_wire_frames_match_runtime(self):
        result = audit.compare(self.coverage(), self.records())
        self.assertEqual(result['sessions'], 1)
        self.assertEqual(result['differences'], [])
        self.assertIn('close_codes', result['excluded'][0])

    def test_queued_undelivered_tail_is_reported(self):
        result = audit.compare(self.coverage(), self.records() + self.records()[1:])
        self.assertEqual({row['case'] for row in result['differences']},
                         {'payloads.rejected', 'reasons.REJECT_REASON_INVALID', 'server_frames'})

    def test_truncated_wire_fails_closed(self):
        for frame in (b'\x80', b'\x0a\x02\x01', b'\x00'):
            with self.assertRaises(ValueError):
                audit.fields(frame)


if __name__ == '__main__':
    unittest.main()
