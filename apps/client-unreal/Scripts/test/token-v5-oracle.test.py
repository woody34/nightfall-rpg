#!/usr/bin/env python3
"""Source-derived timing/chance bounds and scenario budget consistency."""
from fractions import Fraction as F
import importlib.util
from pathlib import Path
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('token_v5', SCRIPTS / 'token-v5-oracle.py')
v5 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v5)


class TokenV5Tests(unittest.TestCase):
    def test_pinned_hit_roll_boundary_and_timing(self):
        # r7 actual player accuracy53/73, sentinel evasion125.
        for accuracy in (53, 73):
            chance = v5.oracle.calc_auto_hit_chance(accuracy, 125)
            self.assertEqual(chance, 200)
            self.assertEqual(sum(roll <= chance for roll in range(1000)), 201)
        self.assertEqual(v5.oracle.calc_auto_timing_ticks(417), (6, 12))
        self.assertEqual(v5.oracle.calc_auto_timing_ticks(253), (10, 20))
        self.assertLess(F(799, 1000) ** 64, F(1, 1000000))

    def test_sentinel_damage_exceeds_full_hp_even_at_minimum_spread(self):
        # Four actual r7 class states:20 base/warrior,40 base/gladiator.
        # Formulas.java:665-738: front, no shots, no traits, NPC -> player.
        for defence, max_hp in ((F('87.2'), 516), (F('103.2'), 1649)):
            damage = v5.oracle.floor(v5.oracle.calc_auto_damage(10000, defence, spread=F(87, 100)))
            self.assertGreater(damage, max_hp)

    def test_both_death_waits_and_total_budget_are_updated(self):
        budget = v5.death_budget(417, 253, F('3.95'))
        self.assertEqual(budget, 120)
        for level, previous in ((20, 290), (40, 310)):
            text = (SCRIPTS.parent / f'Scenarios/2-token-milestone{level}.nfs').read_text()
            self.assertEqual(text.count(f'nf.WaitFor own_dead {budget}\n'), 2)
            self.assertIn(f'nf.Within {previous + 2 * (budget - 10)}\n', text)

    def test_missing_native_evidence_fails_closed(self):
        for rows in ([], [{'kind': 'manifest'}, {'kind': 'complete'}]):
            with self.assertRaises(ValueError):
                v5.verify(rows)


if __name__ == '__main__':
    unittest.main()
