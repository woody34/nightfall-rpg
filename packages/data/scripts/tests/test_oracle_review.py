"""Independent regression checks for DATA review 1 oracle corrections."""
from fractions import Fraction as F
import struct
import unittest

from packages.data.scripts.skill_oracle import (
    accumulation_trace, binary_nearest, generate, ieee_normal, ticks_multiplier,
)


class BinaryRoundingTests(unittest.TestCase):
    def test_binary32_multiplier_matches_struct_reference(self):
        # Independent standard-library IEEE binary32 rounder. Golden generator
        # uses integer arithmetic only, while this test checks the encoding.
        def f32(x):
            return struct.unpack('!f', struct.pack('!f', x))[0]
        for ticks in [1, 3, 5, 7, 31, 101, 255, 25000, 100000]:
            expected = F(*f32(f32(ticks * 666) / f32(1000)).as_integer_ratio())
            self.assertEqual(ticks_multiplier(ticks), expected)
        self.assertEqual(ticks_multiplier(5), F(1745879, 524288))

    def test_nearest_ties_even_sign_and_zero(self):
        self.assertEqual(ieee_normal(F(0), 24), 0)
        self.assertEqual(binary_nearest(F(1) + F(1, 2**24), 24), 1)
        self.assertEqual(binary_nearest(F(1) + F(3, 2**24), 24), F(1) + F(1, 2**22))
        self.assertEqual(ieee_normal(-F(13, 10), 24), -binary_nearest(F(13, 10), 24))


class OracleReviewTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.formulas = generate()[0]['formulas']

    def test_whole_dot_conservation_before_subtraction(self):
        for vector in self.formulas['TickHp']:
            inputs, result = vector['inputs'], vector['rounded_result']
            if F(inputs['power']) < 0:
                self.assertEqual(result['amount'], result['cp_removed'] + result['hp_removed'])
                self.assertEqual(result['final_hp'], inputs['current_hp'] - result['hp_removed'])
                self.assertEqual(result['final_cp'], inputs['current_cp'] - result['cp_removed'])
                self.assertGreaterEqual(result['final_hp'], 1)
                self.assertEqual(result['restoration_remainder_q'], 0)
        v = next(v for v in self.formulas['TickHp'] if v['inputs']['power'] == '-10'
                 and v['inputs']['ticks'] == 1 and v['inputs']['current_hp'] == 100 and v['inputs']['current_cp'] == 0)
        self.assertEqual(v['rounded_result']['amount'], 6)
        self.assertEqual(v['rounded_result']['final_hp'], 94)
        self.assertLess(F(v['source_fractional_result']['final_hp']), 94)
        self.assertGreater(F(v['source_fractional_result']['final_hp']), 93)

    def test_cp_absorption_after_hp_nonlethal_clamp(self):
        def case(hp, cp, player=True):
            return next(v for v in self.formulas['TickHp'] if v['inputs']['current_hp'] == hp
                        and v['inputs']['current_cp'] == cp and v['inputs']['player_source'] == player)['rounded_result']
        full = case(50, 100)
        self.assertEqual((full['amount'], full['cp_removed'], full['final_hp'], full['final_cp']), (49, 49, 50, 51))
        partial = case(50, 10)
        self.assertEqual((partial['cp_removed'], partial['hp_removed'], partial['final_hp']), (10, 39, 11))
        hp_one = case(1, 100)
        self.assertEqual((hp_one['amount'], hp_one['final_hp'], hp_one['final_cp']), (0, 1, 100))
        npc_source = case(50, 100, False)
        self.assertEqual((npc_source['cp_removed'], npc_source['final_hp'], npc_source['final_cp']), (0, 1, 100))

    def test_short_cast_schedule_and_independent_cutoff(self):
        for continuous in [False, True]:
            for animation, launch, impact, cutoff in [(0, 0, 1, 0), ('1/2', 0, 1, 0),
                                                      (1, 0, 4, 0), (50, 0, 4, 0), (100, 0, 4, 0),
                                                      (1200, 5 if continuous else 8, 9 if continuous else 12, 10)]:
                vector = next(v for v in self.formulas['cast_staging'] if v['inputs']['hit_time_ms'] == animation
                              and v['inputs']['is_static'] and v['inputs']['is_continuous'] == continuous)
                out = vector['rounded_result']
                self.assertEqual((out['launch_tick'], out['impact_tick'], out['interrupt_cutoff_tick']),
                                 (100 + launch, 100 + impact, 100 + cutoff))
            for animation, launch, impact in [(500, 0 if continuous else 1, 4 if continuous else 5),
                                              (550, 0 if continuous else 2, 4 if continuous else 6)]:
                vector = next(v for v in self.formulas['cast_staging'] if v['inputs']['hit_time_ms'] == animation
                              and not v['inputs']['is_static'] and v['inputs']['cast_speed'] == 333
                              and v['inputs']['is_continuous'] == continuous)
                out = vector['rounded_result']
                self.assertEqual((out['launch_tick'], out['impact_tick'], out['interrupt_cutoff_tick']),
                                 (100 + launch, 100 + impact, 103))
        for v in self.formulas['cast_staging']:
            self.assertGreaterEqual(v['rounded_result']['launch_tick'], v['inputs']['start_tick'])
            self.assertGreaterEqual(v['rounded_result']['impact_tick'], v['inputs']['start_tick'] + 1)

    def test_karma_binary64_counterexample(self):
        v = next(v for v in self.formulas['calculateKarmaLost'] if v['inputs']['exp'] == 6208133913)
        self.assertEqual(v['intermediate']['rational_truncation'], 100000000)
        self.assertEqual(v['rounded_result'], 99999999)
        # Independently evaluate the exact caller sequence with host binary64.
        for v in self.formulas['calculateKarmaLost']:
            inp = v['inputs']
            expected = min(2**31 - 1, int(abs(inp['exp']) / float(F(inp['karma_multiplier'])) / 30))
            self.assertEqual(v['rounded_result'], expected)

    def test_pinned_cast_speed_and_effect_success_defaults(self):
        speed = next(v for v in self.formulas['FuncMAtkSpeed'] if v['inputs']['wit_stat'] == 57)
        self.assertEqual(F(speed['intermediate']['raw_speed']), F('2024.64'))
        self.assertEqual(speed['rounded_result'], 1999)
        for v in self.formulas['FuncMAtkSpeed']:
            self.assertEqual(v['inputs']['max_m_atk_speed'], 1999)
        for v in self.formulas['calcEffectSuccess']:
            self.assertEqual((v['inputs']['min_chance'], v['inputs']['max_chance']), (10, 90))

    def test_regen_multiple_pulses_remainders_and_full_clear(self):
        for name in ['calcHpRegen', 'calcMpRegen', 'calcCpRegen']:
            vectors = self.formulas[name]
            for v in vectors:
                trace = v['pulse_trace']
                self.assertEqual(len(trace), 3)
                self.assertEqual([p['tick'] for p in trace], [30, 60, 90])
                for count, p in enumerate(trace, 1):
                    total_q = count * v['q_result']
                    self.assertEqual((p['cumulative_whole'], p['remainder_q']), divmod(total_q, 1000000))
            minimum = next(v for v in vectors if v['inputs']['posture'] == 'standing'
                           and v['intermediate']['init_after_clamp'] == '1')
            self.assertEqual([p['cumulative_whole'] for p in minimum['pulse_trace']], [1, 2, 3])
            self.assertEqual([p['remainder_q'] for p in minimum['pulse_trace']], [100000, 200000, 300000])
        trace = accumulation_trace(1100000, maximum=3, current=1, remainder_q=900000)
        self.assertEqual(trace[0]['current'], 3)
        self.assertTrue(all(p['remainder_q'] == 0 for p in trace))

    def test_periodic_remainders_and_per_dynamic_reads(self):
        hot = next(v for v in self.formulas['TickHp'] if v['inputs']['power'] == '100'
                   and v['inputs']['current_hp'] == 500 and v['inputs']['ticks'] == 5)
        self.assertEqual([p['final_hp'] for p in hot['pulse_trace']], [832, 1000, 1000])
        self.assertEqual([p['restoration_remainder_q'] for p in hot['pulse_trace']], [999992, 0, 0])
        per = next(v for v in self.formulas['TickHp'] if v['inputs']['mode'] == 'PER'
                   and v['inputs']['current_hp'] == 1000 and v['inputs']['power'] == '-1/20')
        self.assertEqual([p['final_hp'] for p in per['pulse_trace']], [834, 696, 581])
        self.assertGreater(abs(F(per['pulse_trace'][0]['raw_amount'])), abs(F(per['pulse_trace'][1]['raw_amount'])))
        for v in self.formulas['TickHp']:
            ticks = v['inputs']['ticks']
            self.assertEqual([p['ideal_offset_ms'] for p in v['pulse_trace']], [ticks * 666 * n for n in [1, 2, 3]])


if __name__ == '__main__':
    unittest.main()
