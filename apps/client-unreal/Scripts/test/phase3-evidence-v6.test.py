#!/usr/bin/env python3
"""T17 (D114): the Phase 3 C11 consumer over a real V6 recording; no cargo needed.

Fixture: the i3 gate's 1-late-entry-b recording (V6, snapshot 10 / BinaryV6, e76b739) and its
C11 export trimmed to the manifest fields the late-entry gate reads (rules/classes emptied,
phase3_rules -> regen; every fact kept). The export came from nightfall-replay e76b739 with the
exporter's BinaryV5-only observer_combat guard removed in a scratch build (zone-lane finding:
the V6 exporter must emit observer_combat; see zone-inbox), since the C11 gates read those facts.
"""
import copy
import gzip
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('phase3_evidence_v6', SCRIPTS / 'phase3-evidence.py')
evidence = importlib.util.module_from_spec(spec); spec.loader.exec_module(evidence)
FIXTURE = SCRIPTS / 'test/fixtures/phase3-v6'
OWNER = '01a13d64-63cf-736d-b05d-e98943461a6d'


class V6Recordings(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        raw = Path(self.tmp.name) / 'raw.jsonl'
        raw.write_bytes(gzip.decompress((FIXTURE / '1-late-entry-b.raw.jsonl.gz').read_bytes()))
        self.rows = evidence.read_jsonl(raw)
        self.gates = evidence.requirements(SCRIPTS.parent / 'Scenarios/1-late-entry-b.nfs')

    def evaluate(self, rows):
        return evidence.evaluate(rows, FIXTURE / '1-late-entry-b.nfr', OWNER, self.gates)

    def test_v6_late_entry_recording_passes_c11_exactly(self):
        meta = self.rows[0]['snapshot_meta']
        self.assertEqual((meta['schema_version'], meta['digest_version']), (10, 'BinaryV6'))
        result = self.evaluate(self.rows)
        self.assertTrue(result['passed'], result)
        self.assertEqual([(g['kind'], g['ticks'], g['errors']) for g in result['gates']],
                         [('late_entry_hp_continuity', [589], [])])

    def test_version_pairings_and_policy_still_refuse(self):
        for pairing in ((10, 'BinaryV5'), (9, 'BinaryV6'), (8, 'BinaryV4'), (11, 'BinaryV7')):
            rows = copy.deepcopy(self.rows)
            rows[0]['snapshot_meta'].update(schema_version=pairing[0], digest_version=pairing[1])
            with self.subTest(pairing), self.assertRaisesRegex(ValueError, 'recorded V5 or V6'):
                self.evaluate(rows)
        rows = copy.deepcopy(self.rows); rows[0]['phase3_rules'] = None
        with self.assertRaisesRegex(ValueError, 'recorded V5 or V6'):
            self.evaluate(rows)
        self.assertEqual(evidence.C11_RECORDINGS, {(9, 'BinaryV5'), (10, 'BinaryV6')})

    def test_v6_without_observer_combat_facts_does_not_pass(self):
        # The unpatched V6 exporter emits no observer_combat: the gate must fail, not pass.
        rows = [r for r in copy.deepcopy(self.rows) if r['kind'] != 'observer_combat']
        for index, row in enumerate(rows[1:-1]): row['sequence'] = index
        rows[-1]['facts'] = len(rows) - 2
        result = self.evaluate(rows)
        self.assertFalse(result['passed'])
        self.assertEqual(result['gates'][0]['errors'], ['missing entity/tick raw replay fact'])

    def test_additive_v6_fields_are_tolerated_but_hp_continuity_is_exact(self):
        rows = copy.deepcopy(self.rows)
        row = next(r for r in rows[1:-1] if r['sequence'] == 336)
        self.assertTrue(any('phase4' in e for e in row['data']['events']))  # V6 enrichment present
        spawn = next(e for e in row['data']['events'] if e['type'] == 'entity_spawn'
                     and e['kind'] == 'npc' and e['combat']['template'] == 'keltir_durable')
        spawn['combat']['hp'] -= 1  # a re-entry baseline one HP off must not pass
        result = self.evaluate(rows)
        self.assertFalse(result['passed'])
        self.assertTrue(result['gates'][0]['errors'])


class V6AdditiveFields(unittest.TestCase):
    """T17 facet 2: the V6 producer adds `shots` to damage_formula (state_runtime_damage.rs)."""
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        raw = Path(self.tmp.name) / 'raw.jsonl'
        raw.write_bytes(gzip.decompress((FIXTURE / '1-late-entry-b.raw.jsonl.gz').read_bytes()))
        self.rows = evidence.read_jsonl(raw)
        # The real V6 calcMagicDam fact from the i3 3-aggro-skill-hate export (shots 0).
        self.fact = json.loads((FIXTURE / '3-aggro-skill-hate.damage_formula.json').read_text())

    def validate(self, fact, v5=False):
        rows = copy.deepcopy(self.rows)
        if v5: rows[0]['snapshot_meta'].update(schema_version=9, digest_version='BinaryV5')
        fact = copy.deepcopy(fact); fact['tick'] = rows[-2]['tick']  # inside the recording scope
        rows.insert(len(rows) - 1, fact)
        for index, row in enumerate(rows[1:-1]): row['sequence'] = index
        rows[-1]['facts'] = len(rows) - 2
        return evidence.validate(rows, FIXTURE / '1-late-entry-b.nfr')

    def test_real_v6_damage_formula_with_shots_validates(self):
        self.assertEqual(self.fact['data']['shots'], 0)
        self.assertEqual(evidence.V6_ADDITIVE, {'damage_formula': {'shots'}})
        self.validate(self.fact)

    def test_shots_is_exact_and_version_bound(self):
        missing = copy.deepcopy(self.fact); del missing['data']['shots']
        charged = copy.deepcopy(self.fact); charged['data']['shots'] = 1
        unknown = copy.deepcopy(self.fact); unknown['data']['bonus'] = 1
        with self.assertRaisesRegex(ValueError, 'unknown/missing damage_formula'): self.validate(missing)
        with self.assertRaisesRegex(ValueError, 'charged shot'): self.validate(charged)
        with self.assertRaisesRegex(ValueError, 'unknown/missing damage_formula'): self.validate(unknown)
        with self.assertRaisesRegex(ValueError, 'unknown/missing damage_formula'): self.validate(self.fact, v5=True)
        self.validate(missing, v5=True)  # the V5 field set is unchanged


if __name__ == '__main__':
    unittest.main()
