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



class V6Equipment(unittest.TestCase):
    """D121 (T17 facet 3): V6 starter-kit item installs are proven against their templates.

    Real i3c facts (e76b739): the C11 damage rows of 3-aggro-skill-hate, 3-cast-observer-a and
    3-debuff-status and the stat row of 3-buff-expires, each with only the facts and manifest
    parts the check reads (d121-equipment.json.gz)."""
    def setUp(self):
        self.bundle = json.loads(gzip.decompress((FIXTURE / 'd121-equipment.json.gz').read_bytes()))

    def check(self, unit, change=None):
        case = copy.deepcopy(self.bundle[unit])
        facts = case['facts']
        row = next(r for r in facts if r['sequence'] == case['sequence'])
        if change:
            change(row, facts)
        return getattr(evidence, case['function'])(row, case['manifest'], facts)

    @staticmethod
    def context(row, facts):
        if row['kind'] == 'damage_formula':
            resolution = next(r for r in facts if r['kind'] == 'formula_resolution' and r['tick'] == row['tick'])
            return resolution['data']['inputs']['caster']
        return row['data']['after']

    def test_real_v6_damage_and_stat_rows_pass_with_proven_kit(self):
        for unit in ('3-aggro-skill-hate', '3-cast-observer-a', '3-debuff-status', '3-buff-expires'):
            with self.subTest(unit):
                self.check(unit)
        row = next(r for r in self.bundle['3-buff-expires']['facts'] if r['sequence'] == 39)
        kit = evidence.equipped_kit(self.bundle['3-buff-expires']['facts'], row['entity'], 39)
        self.assertEqual(kit, [6, 425, 461])  # Apprentice's Wand, Tunic, Stockings (equip order)
        self.assertEqual(evidence.item_installs(row['data']['after'], kit), {1, 2, 3})

    def test_wrong_modifier_value_refuses(self):
        def wrong(row, facts):
            for f in self.context(row, facts)['effective']['calculator']:
                if f['key']['install_sequence'] == 1 and f['key']['stat'] == 'mAtk':
                    f['operation'] = {'Set': 7_000_001}
        for unit in ('3-aggro-skill-hate', '3-buff-expires'):
            with self.subTest(unit), self.assertRaisesRegex(ValueError, 'differs from the functions of equipped template 6'):
                self.check(unit, wrong)

    def test_extra_install_without_an_equipped_item_refuses(self):
        def extra(row, facts):
            calculator = self.context(row, facts)['effective']['calculator']
            calculator.append({'conditions': [], 'key': {'function_ordinal': 0, 'handler_ordinal': 0, 'install_sequence': 9,
                                                        'priority': 30, 'stat': 'maxMp'}, 'operation': {'Add': 1_000_000}})
        for unit in ('3-debuff-status', '3-buff-expires'):
            with self.subTest(unit), self.assertRaisesRegex(ValueError, 'missing or extra install'):
                self.check(unit, extra)

    def test_missing_install_refuses(self):
        def missing(row, facts):
            context = self.context(row, facts)
            context['effective']['calculator'] = [f for f in context['effective']['calculator'] if f['key']['install_sequence'] != 3]
        for unit in ('3-cast-observer-a', '3-buff-expires'):
            with self.subTest(unit), self.assertRaisesRegex(ValueError, 'missing or extra install'):
                self.check(unit, missing)
        # A kit item the admission load does not equip is likewise unbacked.
        def unequipped(row, facts):
            load = next(r for r in facts if r['kind'] == 'admission_reconstruction' and r['entity'] == row['entity'])
            load['data']['load']['phase4']['pending'][0]['materialization']['manifest']['items'][2]['equipped'] = False
        with self.assertRaisesRegex(ValueError, 'missing or extra install'):
            self.check('3-buff-expires', unequipped)

    def test_folded_values_stay_exact(self):
        def p_def(row, facts):
            row['data']['after']['effective']['physical']['p_def'] += 1
        def builtin_sub(row, facts):
            for f in row['data']['after']['effective']['calculator']:
                if f['key']['stat'] == 'pDef' and f['key']['install_sequence'] == 0 and 'Sub' in f['operation']:
                    f['operation'] = {'Sub': 15_000_000}  # chest only: legs default not replaced
        def m_atk(row, facts):
            row['data']['after']['effective']['m_atk_q'] += 1  # 10872846 = V5 rounding, not the V6 truncation
        for name, change in (('pDef', p_def), ('slot default', builtin_sub), ('mAtk', m_atk)):
            with self.subTest(name), self.assertRaises(ValueError):
                self.check('3-buff-expires', change)

    def test_retained_item_checkpoint_load_refuses(self):
        def retained(row, facts):
            load = next(r for r in facts if r['kind'] == 'admission_reconstruction' and r['entity'] == row['entity'])
            load['data']['load']['phase4']['checkpoint'] = {'items': []}
        with self.assertRaisesRegex(ValueError, 'retained-item equipment'):
            self.check('3-buff-expires', retained)


if __name__ == '__main__':
    unittest.main()
