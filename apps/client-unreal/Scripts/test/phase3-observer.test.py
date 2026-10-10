#!/usr/bin/env python3
"""Correlated observer source/vector contract; no native client or engine pass."""
import copy
import importlib.util
import json
from pathlib import Path
import re
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
SCENARIOS = SCRIPTS.parent / 'Scenarios'
REPO = SCRIPTS.parents[2]
spec = importlib.util.spec_from_file_location('observer_packs', SCRIPTS / 'phase3-fixtures.py')
packs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packs)


class ObserverContracts(unittest.TestCase):
    def test_both_roles_wait_for_wind_identity_not_generic_buff_result(self):
        owner = (SCENARIOS / '3-cast-observer-a.nfs').read_text()
        peer = (SCENARIOS / '3-cast-observer-b.nfs').read_text()
        self.assertIn('nf.WaitFor cast_result_for own 1177 12', owner)
        wait = 'nf.WaitFor cast_result_for ' + packs.OWNER + ' 1177 30'
        self.assertIn(wait, peer)
        self.assertNotIn('nf.WaitFor observed_cast_results ', peer)
        self.assertNotIn('nf.WaitFor cast_results ', owner)
        self.assertLess(peer.index(wait), peer.index('nf.Cast 1068 self'))
        self.assertIn('nf.WaitFor entity_effect_active ' + packs.OBSERVER + ' 1068 30', owner)
        profile = packs.profiles('phase3-trainer', REPO, paired=True)['observer']
        self.assertEqual(profile['learned_ranks']['1068'], 1)

    def test_late_entry_really_crosses_visibility_boundary_and_requires_baseline(self):
        profiles = packs.profiles('phase3-observer-late', REPO, paired=True)
        self.assertEqual(profiles['owner']['position'], [110, 100])
        self.assertEqual(profiles['observer']['position'], [65, 100])
        cells = [(x // 32, 100 // 32) for x in (110, 65, 63, 65)]
        self.assertTrue(abs(cells[0][0] - cells[1][0]) <= 1)
        self.assertTrue(abs(cells[0][0] - cells[2][0]) > 1)
        self.assertTrue(abs(cells[0][0] - cells[3][0]) <= 1)
        owner = (SCENARIOS / 'variants/3-cast-observer-late-a.nfs').read_text()
        peer = (SCENARIOS / 'variants/3-cast-observer-late-b.nfs').read_text()
        self.assertLess(owner.index('nf.WaitFor entity_not_in_view'), owner.index('nf.Cast 1177'))
        self.assertLess(peer.index('nf.ClickMove 63 100'), peer.index('nf.ClickMove 65 100'))
        self.assertIn('nf.WaitFor observed_cast_baseline_for ' + packs.OWNER + ' 1177 12', peer)
        with self.assertRaises(ValueError): packs.profiles('phase3-observer-late', REPO)

    def validate_wire_fixture(self, vector):
        proto = (REPO / 'packages/proto/nightfall/v1/world.proto').read_text()
        known, casts, terminals = {}, {}, {}
        hydrated = False
        for message in vector['messages']:
            name, fields = message['event'], message['fields']
            body = re.search(r'message ' + name + r'\s*\{(.*?)\n\}', proto, re.S)[1]
            allowed = set(re.findall(r'\b([a-z_]+)\s*=\s*\d+', body))
            self.assertLessEqual(set(fields), allowed, name)
            if name == 'EntitySpawn':
                self.assertFalse(hydrated, 'all identities must precede hydration')
                known[fields['entity_id']] = fields['life_incarnation']
            elif name == 'SkillCastStarted':
                hydrated = True
                self.assertEqual(known[fields['caster']], fields['caster_incarnation'])
                self.assertEqual(known[fields['target']], fields['target_incarnation'])
                self.assertIs(fields['baseline'], True)
                self.assertLess(fields['start_tick'], fields['baseline_tick'])
                self.assertLess(fields['baseline_tick'], fields['complete_tick'])
                key = tuple(fields[k] for k in ('caster', 'caster_incarnation', 'cast_id', 'skill_id'))
                casts[key] = fields
            elif name == 'SkillCastResult':
                key = tuple(fields[k] for k in ('caster', 'caster_incarnation', 'cast_id', 'skill_id'))
                if key in casts:
                    terminals[key] = terminals.get(key, 0) + 1
        self.assertEqual(len(casts), 2)
        self.assertEqual(set(terminals), set(casts))
        self.assertTrue(all(value == 1 for value in terminals.values()))

    def test_mutual_reference_fixture_matches_proto_and_excludes_might_terminal(self):
        vector = json.loads((SCENARIOS / 'fixtures/phase3-observer-mutual.json').read_text())
        self.validate_wire_fixture(vector)
        for mutate in (
            lambda d: d['messages'].insert(1, d['messages'].pop(2)),
            lambda d: d['messages'][2]['fields'].update(target_incarnation=1),
            lambda d: d['messages'][2]['fields'].update(baseline=False),
            lambda d: d['messages'][2]['fields'].update(baseline_tick=1080),
            lambda d: d['messages'][5]['fields'].update(cast_id=6999),
            lambda d: d['messages'][5]['fields'].update(skill_id=1068),
        ):
            changed = copy.deepcopy(vector)
            mutate(changed)
            with self.assertRaises((AssertionError, KeyError)):
                self.validate_wire_fixture(changed)


if __name__ == '__main__':
    unittest.main()
