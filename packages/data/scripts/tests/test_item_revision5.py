"""Review-4 missing behaviors: source arithmetic and fail-closed mutations."""
from copy import deepcopy
from fractions import Fraction as F
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

from packages.data.scripts import item_oracle as o
from packages.data.scripts.item_coverage import required_cases, verify_case


class Revision5Tests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.src = o.Source()
        cls.vectors = o.make_vectors(cls.src)
        cls.by_id = {v['id']: v for v in cls.vectors}
        cls.required = required_cases(cls.src)

    def test_each_new_family_and_individual_boundary_is_mandatory(self):
        prefixes = ['bow-launch-', 'bow-refill-history-', 'bow-mp-', 'resurrection-scroll-',
                    'dormant-starter-use-', 'enchant-safe-', 'enchant-quantized-', 'load-modifier-',
                    'grade-slot-', 'bow-reuse-speed-', 'dual-plan-', 'pole-count-', 'blow-boundary-',
                    'blow-cap-rounding-', 'retail-draw-contract-', 'augmentation-provider-',
                    'learned-item-consume-', 'item-regen-status-', 'consumable-escape-timing-']
        for prefix in prefixes:
            names = [n for n in self.required if n.startswith(prefix)]
            self.assertTrue(names, prefix)
            with self.subTest(family=prefix), self.assertRaisesRegex(ValueError, 'missing vectors'):
                o.verify_coverage(self.src, [v for v in self.vectors if not v['id'].startswith(prefix)])
            for name in names:
                with self.subTest(boundary=name), self.assertRaisesRegex(ValueError, 'missing vectors'):
                    o.verify_coverage(self.src, [v for v in self.vectors if v['id'] != name])
                # Preserving identity cannot hide dropped or changed state inputs.
                row = self.by_id[name]
                for key in self.required[name]['inputs']:
                    for action in ['delete', 'replace']:
                        inputs = deepcopy(row['inputs'])
                        if action == 'delete':
                            del inputs[key]
                        else:
                            inputs[key] = {'invalid': True}
                        with self.subTest(case=name, dimension=key, action=action), self.assertRaisesRegex(ValueError, 'input dimension'):
                            verify_case(name, self.required[name], dict(row, inputs=inputs))
            # Exercise the full gate for each new family's mutated state as well.
            name = names[0]
            row = self.by_id[name]
            inputs = dict(row['inputs']); inputs[next(iter(self.required[name]['inputs']))] = None
            with self.assertRaisesRegex(ValueError, 'input dimension'):
                o.verify_coverage(self.src, [dict(v, inputs=inputs) if v['id'] == name else v for v in self.vectors])

    def test_registry_does_not_consult_either_vector_builder(self):
        required_cases.cache_clear()
        with patch.object(o, 'make_vectors', side_effect=AssertionError('not authority')), \
             patch.object(o.behavior, 'add_behavior_vectors', side_effect=AssertionError('not authority')), \
             patch.object(o.behavior, 'add_audit_vectors', side_effect=AssertionError('not authority')):
            self.assertEqual(required_cases(self.src), self.required)

    def test_bow_launch_independent_source_arithmetic(self):
        # Parse each bound weapon/arrow back from XML, not the oracle helpers.
        for name, row in self.by_id.items():
            if not name.startswith(('bow-launch-', 'bow-mp-')):
                continue
            x = o.decode_rationals(row['inputs']); y = o.decode_rationals(row['expected'])
            props = {n.get('name'): n.get('val') for n in self.src.items[x['weapon']['template_id']][2].findall('set')}
            self.assertEqual(x['weapon']['mp_cost'], int(props.get('mp_consume', '0')))
            chance, reduced = map(int, props.get('reduced_mp_consume', '0,0').split(','))
            self.assertEqual((x['weapon']['reduced_chance'], x['weapon']['reduced_cost']), (chance, reduced))
            grade = lambda g: 'S' if g in ['S80', 'S84'] else g
            arrow = next((a for a in x['inventory'] if a['instance_id'] == x['lhand']), None) if x['lhand'] is not None else next(
                (a for a in x['inventory'] if grade(a['grade']) == grade(props.get('crystal_type', 'NONE')) and a['kind'] == 'ARROW'), None)
            for a in x['inventory']:
                ap = {n.get('name'): n.get('val') for n in self.src.items[a['template_id']][2].findall('set')}
                self.assertEqual(a['grade'], ap.get('crystal_type', 'NONE'))
                self.assertEqual(a['kind'], ap.get('etcitem_type', 'NONE'))
            draws = []
            cost = 0
            launched = False
            reason = 'arrows'
            if arrow is not None:
                reason = 'reuse' if x['reuse_remaining'] > 0 else 'mp'
                if x['reuse_remaining'] == 0:
                    cost = int(props.get('mp_consume', '0'))
                    if reduced > 0:
                        draws.append(dict(site='bow_mp_reduction', bound=100, residue=x['reduction_residue']))
                        if x['reduction_residue'] < chance:
                            cost = reduced
                    cost = int(F(cost)*x['mp_modifier'])
                    launched = x['mp'] >= cost
                    if launched:
                        reason = 'launched'
                        draws.append(dict(site='hit', bound=1000, residue=x['hit_residue']))
                        # D88: a hit iff the residue is strictly below the chance.
                        self.assertEqual(x['miss'], x['hit_residue'] >= x['hit_chance'], name)
                        if not x['miss']:
                            # D87: the caller's live residues, not a neutral 999 / radius.
                            draws.append(dict(site='critical', bound=1000, residue=x['critical_residue']))
                            radius = int(props.get('random_damage', '0'))
                            if radius:
                                draws.append(dict(site='spread', bound=2*radius+1, residue=x['spread_residue']))
            self.assertEqual(y['accepted'], launched, name)
            self.assertEqual(y['reason'], reason, name)
            self.assertEqual(y['mp_paid'], cost if launched else 0, name)
            self.assertEqual(y['mp'], x['mp']-(cost if launched else 0), name)
            self.assertEqual(y['mp_q'], int(y['mp']*1000000), name)
            self.assertEqual(y['draws'], draws, name)
            self.assertEqual(row['draws'], draws, name)
            self.assertEqual(y['arrows_consumed'], int(launched), name)
            self.assertEqual(sum(i['count'] for i in y['inventory']), sum(i['count'] for i in x['inventory'])-int(launched), name)
            if launched:
                self.assertEqual(y['hit'], x['hit_residue'] < x['hit_chance'], name)
                self.assertEqual(y['lhand'], arrow['instance_id'] if arrow['count'] > 1 else None, name)

    def test_exhaustion_refill_and_empty_skip_payment(self):
        for grade in self.src.grades:
            y = o.decode_rationals(self.by_id['bow-refill-history-'+grade]['expected'])
            self.assertEqual([r['accepted'] for r in y['trace']], [True, False, True, False])
            self.assertEqual([r['hit'] for r in y['trace']], [True, None, False, None])
            self.assertEqual([r['lhand'] for r in y['trace']], [None]*4)
            self.assertEqual(y['consumed'], 2)
            self.assertEqual(y['mp_paid'], 2*int(self.src.properties(self.by_id['bow-refill-history-'+grade]['inputs']['weapon']['template_id'])['mp_consume']))

    def test_scroll_offer_accept_decline_expiry_and_death_fence(self):
        for name, row in self.by_id.items():
            if not name.startswith('resurrection-scroll-'):
                continue
            x = o.decode_rationals(row['inputs']); y = o.decode_rationals(row['expected'])
            skill = self.src.skills[2014][1]
            sets = {n.get('name'): n.get('val') for n in skill.findall('set')}
            cost = int(sets['itemConsumeCount'])
            self.assertEqual(sets['targetType'], 'PC_BODY')
            self.assertEqual(x['restore_percent'], 0)  # absent power defaults to zero; no invented XP restoration
            self.assertEqual(y['inventory'], x['available']-(cost if x['available'] >= cost else 0), name)
            self.assertEqual(y['consumed'], cost if x['available'] >= cost else 0, name)
            self.assertEqual(y['draws'], [], name)
            if not y['accepted']:
                continue
            self.assertEqual(y['completion_consumed'], 0)
            self.assertEqual(y['trace'][1]['tick'], 10+(int(sets['hitTime'])+99)//100)
            events = [r['event'] for r in y['trace']]
            if 'accepted' in events:
                self.assertFalse(y['dead']); self.assertEqual(y['incarnation'], 8)
                self.assertEqual(y['hp_q'], int(F(101)*F(65, 100)*1000000))
                self.assertEqual(y['recoverable_xp'], 0)
                self.assertIsNone(y['offer'])
            else:
                self.assertEqual(y['dead'], x['dead']); self.assertEqual(y['incarnation'], x['current_incarnation'])
                self.assertEqual(y['hp'], x['hp'])
            self.assertEqual(y['mp'], x['mp']); self.assertEqual(y['cp'], x['cp'])
            if y['offer'] is not None:
                self.assertEqual(y['offer']['expires_at']-y['offer']['created_at'], 600)
        before = o.decode_rationals(self.by_id['resurrection-scroll-accept-expiry--1']['expected'])
        at = o.decode_rationals(self.by_id['resurrection-scroll-accept-expiry-0']['expected'])
        self.assertFalse(before['dead']); self.assertTrue(at['dead'])
        retained = o.decode_rationals(self.by_id['resurrection-scroll-first-valid']['expected'])
        self.assertEqual(retained['xp_refund'], 50)
        self.assertEqual([r['event'] for r in retained['trace']][-2:], ['accepted', 'NO_RESURRECT_OFFER'])

    def test_dormant_starter_rejection_preserves_every_state_dimension(self):
        kits = self.src.xml(o.D+'stats/initialEquipment.xml').findall('equipment')
        expected = {(int(k.get('classId')), int(n.get('id'))) for k in kits if self.src.races[int(k.get('classId'))] != 'KAMAEL'
                    for n in k.findall('item') if int(n.get('id')) in [5588, 10650, 12753]}
        actual = set()
        for name, row in self.by_id.items():
            if not name.startswith('dormant-starter-use-'):
                continue
            cid, sid = map(int, name.removeprefix('dormant-starter-use-').split('-')); actual.add((cid, sid))
            x, y = row['inputs'], row['expected']
            self.assertFalse(y['accepted']); self.assertEqual(y['reason'], 'ITEM_CAPABILITY_DEFERRED')
            self.assertEqual(y['consumed'], {}); self.assertEqual(y['draws'], [])
            for key in ['inventory', 'paperdoll', 'allocator_next', 'reuse']:
                self.assertEqual(y[key], x[key])
            with self.assertRaisesRegex(ValueError, 'not a dormant'):
                o.behavior.dormant_use(**dict(x, properties={'handler': 'ItemSkills', 'item_skill': '2013-1'}))
        self.assertEqual(actual, expected)

    def test_source_option_routing_and_fraction_fold(self):
        kinds = set()
        for name, row in self.by_id.items():
            if not name.startswith('augmentation-provider-'):
                continue
            x = o.decode_rationals(row['inputs']); y = o.decode_rationals(row['expected'])
            node = ET.fromstring(x['option_xml']); kinds.update(n.tag for n in node if n.tag != 'for')
            install = x['event'] not in ['unequip', 'cancel']
            values = dict(x['stat_baseline'])
            if install:
                for n in node.findall('for/*'):
                    values[n.get('stat')] += F(n.get('val'))
            self.assertEqual(y['stats'], values)
            self.assertEqual(y['stats_q'], {k: int(v*1000000) for k, v in values.items()})
            for field, tags in [('granted_active', ['active_skill']), ('system_passive', ['passive_skill']),
                                ('triggers', ['attack_skill', 'critical_skill', 'magic_skill'])]:
                self.assertEqual(y[field], [dict({k: f'{F(v).numerator}/{F(v).denominator}' if '.' in v else v  # D98 n/d
                                    for k, v in n.attrib.items()}, kind=n.tag) for n in node if n.tag in tags] if install else [])
            self.assertEqual(y['other_provider'], 99); self.assertEqual(y['hp'], 75)
            self.assertFalse(y['on_start_replayed']); self.assertEqual(y['draws'], [])
        self.assertEqual(kinds, {'active_skill', 'passive_skill', 'attack_skill', 'critical_skill', 'magic_skill'})

    def test_quantized_success_sets_exhaust_all_residues(self):
        for base, bonus, support in [(F(0), F(0), F(0)), (F(6667, 100), F(0), F(0)), (F(100), F(0), F(0)), (F(0), F(10), F(5))]:
            y = self.by_id[f'enchant-quantized-{base}-{bonus}-{support}']['expected']
            chance = min(F(100), base+bonus+support)
            self.assertEqual(y['successful_residues'], sum(100*F(r, 100000) < chance for r in range(100000)))
            self.assertEqual(y['mismatches'], 0); self.assertEqual(y['compared_residues'], 100000)
        for full in [False, True]:
            for e in [2, 3, 4, 5]:
                y = self.by_id[f'enchant-safe-{full}-{e}']['expected']
                self.assertEqual(y['success'], e < (4 if full else 3))
                self.assertEqual(y['draws'], [dict(site='enchant', bound=100000, residue=99999)])

    def test_learned_cost_lowest_id_once_even_canceled(self):
        for phase in ['complete', 'cancel']:
            for available in [0, 1, 2]:
                y = self.by_id[f'learned-item-consume-{phase}-{available}']['expected']
                self.assertEqual(y['accepted'], available > 0)
                self.assertEqual(y['consumed'], {'1': 1} if available else {})
                if available:
                    self.assertEqual(y['completion_consumed'], {}); self.assertEqual(y['repeated_consumed'], {})
                self.assertEqual(y['draws'], [])

    def test_dual_stores_second_and_keeps_source_draw_order(self):
        for hits in [[False, False], [True, False], [False, True], [True, True]]:
            for valid in [False, True]:
                row = self.by_id[f'dual-plan-{hits}-{valid}']; y = row['expected']
                planned = [24 if hits[0] else 0, 50 if hits[1] else 0]
                self.assertEqual(y['plan'], planned); self.assertEqual(y['stored_second'], planned[1])
                self.assertEqual(y['applied'], [planned[0], planned[1] if valid else None])
                sites = ['hit1', 'hit2']
                for i, hit in enumerate(hits, 1):
                    if hit:
                        sites.extend([f'critical{i}', f'spread{i}'])
                self.assertEqual([d['site'] for d in y['draws']], sites)
                self.assertEqual(row['draws'], y['draws'])
                self.assertFalse(y['second_recomputed']); self.assertEqual(y['refunded_draws'], [])

    def test_escape_timings_and_resources_match_each_source_skill(self):
        for sid in [736, 1538]:
            for speed in [333, 1500]:
                row = self.by_id[f'consumable-escape-timing-{sid}-{speed}']; x = row['inputs']; y = row['expected']
                skill_id = int(self.src.properties(sid)['item_skill'].split('-')[0])
                sets = {n.get('name'): n.get('val') for n in self.src.skills[skill_id][1].findall('set')}
                ms = int(sets['hitTime'])
                self.assertEqual(sets['isMagic'], '2')
                self.assertEqual(y['animation_ms'], ms)
                # P3:126 and L2Character:1643,4577: 200ms launches now, impacts +400ms.
                self.assertEqual(ms, 20000 if sid == 736 else 200)
                self.assertEqual(y['completion_tick'], 210 if sid == 736 else 14)
                self.assertEqual(y['launch_tick'], 10+(max(0, ms-400)+99)//100)
                self.assertEqual(y['interrupt_cutoff_tick'], 10+max(0, ms//100-2))
                self.assertEqual(y['consumed'], 1); self.assertEqual(y['completion_consumed'], 0)
                self.assertEqual(y['resources'], x['resources']); self.assertEqual(y['incarnation'], 7)
                self.assertFalse(y['town_respawn']); self.assertEqual(y['destination'], [126, 126])
        self.assertNotEqual(self.by_id['consumable-escape-timing-736-333']['expected']['animation_ms'],
                            self.by_id['consumable-escape-timing-1538-333']['expected']['animation_ms'])

    def test_cast_sequence_stages_blessed_and_preserves_normal_and_zero_time(self):
        for sid, ms, completion in [(736, 20000, 210), (1538, 200, 14), (1539, 0, 11)]:
            definition = o.consumable_definition(self.src, sid)
            self.assertEqual(definition['skills'][0]['hit_time_ms'], ms)
            starts = [dict(tick=10, cast_id=1, definition=definition, available={sid:2},
                           condition_context={'canEscape':True, 'inside_zone_ids':[]})]
            result = o.consumable_cast_sequence(starts, 0, 100)
            self.assertEqual(result['trace'][0]['cast']['completion_tick'], completion)
            self.assertEqual(result['trace'][1]['tick'], completion)
            self.assertEqual(result['total_consumed'], {sid:1})
            self.assertEqual(result['completion_consumed'], {})
            self.assertEqual(result['inventory'], {sid:1})
        self.assertEqual([e['tick'] for e in self.by_id['consumable-same-tick-lanes']['expected']['trace']],
                         [10, 209, 210, 210])

    def test_blow_draws_adaptation_and_trace_mutations(self):
        count = 0
        for name, row in self.by_id.items():
            if not name.startswith(('blow-front-', 'blow-side-', 'blow-back-', 'blow-boundary-')):
                continue
            count += 1
            x = o.decode_rationals(row['inputs'])
            rate = F(x['chance'])*F(x['dex'])*{'front':1, 'side':F(3, 2), 'back':2}[x['proximity']]
            self.assertEqual(row['expected'], F(x['residue'], 10) < rate)
            trace = [dict(site='blow', bound=1000, residue=x['residue'])]
            self.assertEqual(row['draws'], trace)
            citations = ['G/model/stats/Formulas.java:1621-1642',
                         'docs/plans/phase-4-zone-design.md:269 (§5; A8): integer100 to per-mille; roll1000*Q < rate_q*10']
            self.assertTrue(all(c in row['sources'] for c in citations))
            mutations = [dict(row, draws=[])]
            missing = dict(row); del missing['draws']; mutations.append(missing)
            for field, wrong in [('site', 'hit'), ('bound', 100), ('residue', x['residue']-1)]:
                for action in ['delete', 'replace']:
                    draw = dict(trace[0])
                    if action == 'delete':
                        del draw[field]
                    else:
                        draw[field] = wrong
                    mutations.append(dict(row, draws=[draw]))
            for mutation in mutations:
                with self.subTest(case=name, draws=mutation.get('draws')), self.assertRaisesRegex(ValueError, 'draw trace'):
                    verify_case(name, self.required[name], mutation)
            for citation in citations:
                with self.assertRaisesRegex(ValueError, 'source citation'):
                    verify_case(name, self.required[name], dict(row, sources=[c for c in row['sources'] if c != citation]))
            with self.assertRaisesRegex(ValueError, 'draw trace'):
                o.verify_coverage(self.src, [dict(v, draws=[]) if v['id'] == name else v for v in self.vectors])
        self.assertEqual(count, 23)  # 15 positional + 8 zero/certain/cap boundary draws.

    def test_regen_precedes_item_restore_with_source_status_guards(self):
        for name, row in self.by_id.items():
            if not name.startswith('item-regen-status-'):
                continue
            x = o.decode_rationals(row['inputs']); y = o.decode_rationals(row['expected'])
            rate = max(F(1), x['base']*x['level_modifier']*x['stat_bonus'])*x['posture_modifier']*x['regen_modifier']
            self.assertEqual(y['rate'], rate)
            current, remainder = x['current'], x['remainder_q']
            for step, credit in zip(y['trace'], [rate, F(435)]):
                blocked = x['dead'] or step['site'] == 'item_restore' and (x.get('invulnerable', False) or x.get('hp_blocked', False))
                whole, carry = divmod(remainder+int(credit*1000000), 1000000)
                granted = 0 if blocked else min(whole, x['maximum']-current)
                current += granted
                if not blocked:
                    remainder = 0 if current >= x['maximum'] else carry
                self.assertEqual(step['granted'], granted); self.assertEqual(step['remainder_q'], remainder)
            self.assertEqual(y['current'], current); self.assertEqual(y['remainder_q'], remainder)
            self.assertEqual(y['new_use_allowed'], not x['dead'] and not x['stunned'])


if __name__ == '__main__':
    unittest.main()
