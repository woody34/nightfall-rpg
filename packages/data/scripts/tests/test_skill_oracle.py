#!/usr/bin/env python3
"""Unit tests for the independent skill oracle and formula calculations."""

from fractions import Fraction as F
import unittest

from packages.data.scripts.skill_oracle import (
    binary_nearest,
    ceil,
    floor,
    generate,
    root_bounds,
    rounded,
)


class OracleMathTests(unittest.TestCase):
    def test_floor_ceil_rounded(self):
        self.assertEqual(floor(F("3.7")), 3)
        self.assertEqual(ceil(F("3.2")), 4)
        self.assertEqual(ceil(F("3.0")), 3)
        self.assertEqual(rounded(F("3.4")), 3)
        self.assertEqual(rounded(F("3.5")), 4)
        self.assertEqual(rounded(F("3.6")), 4)

    def test_root_bounds_exact_and_approx(self):
        # Perfect squares
        lo, hi = root_bounds(F(4))
        self.assertEqual(lo, F(2))
        self.assertEqual(hi, F(2))

        lo, hi = root_bounds(F(100))
        self.assertEqual(lo, F(10))
        self.assertEqual(hi, F(10))

        # Non-perfect square
        lo, hi = root_bounds(F(2))
        self.assertTrue(lo <= hi)
        self.assertTrue(lo * lo <= F(2) <= hi * hi)
        self.assertTrue((hi - lo) < F(1, 10**10))

    def test_binary_nearest_float(self):
        # 1.3 power representation in binary32 (24 bits)
        exact = F(13, 10)
        bn = binary_nearest(exact, 24)
        self.assertEqual(bn, F(10905190, 8388608))
        self.assertTrue(abs(bn - exact) < F(1, 10**7))


class FormulaVerificationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.golden, cls.json_text = generate()
        cls.formulas = cls.golden["formulas"]

    def test_all_formulas_have_at_least_40_vectors(self):
        expected_formulas = [
            "calcSkillPhysDam",
            "calcMagicDam",
            "calcMagicSuccess",
            "calcEffectSuccess",
            "effect_duration",
            "calcAtkBreak",
            "calcAttributeBonus",
            "calcHpRegen",
            "calcMpRegen",
            "calcCpRegen",
            "calculateKarmaGain",
            "calculateKarmaLost",
            "calculateSkillResurrectRestorePercent",
            "calcCastTime",
            "doRevive",
            "FuncMAtkMod",
            "FuncMDefMod",
            "FuncMAtkSpeed",
        ]
        self.assertEqual(len(self.formulas), 18)
        for name in expected_formulas:
            self.assertIn(name, self.formulas)
            vecs = self.formulas[name]
            self.assertGreaterEqual(
                len(vecs), 40,
                f"Formula {name} has only {len(vecs)} vectors (< 40)"
            )

    def test_resurrection_quirk_behavior(self):
        vecs = self.formulas["calculateSkillResurrectRestorePercent"]
        # Find base 20, wit 2.0 (diff == 20: NO quirk)
        v_no_quirk = next(v for v in vecs if v["inputs"]["base_restore_percent"] == 20 and v["inputs"]["wit_bonus"] == "2")
        self.assertEqual(v_no_quirk["exact_rational"], "40")
        self.assertFalse(v_no_quirk["intermediate"]["quirk_triggered"])

        # Find base 30, wit 2.0 (diff == 30 > 20: QUIRK TRIGGERS +20)
        v_quirk = next(v for v in vecs if v["inputs"]["base_restore_percent"] == 30 and v["inputs"]["wit_bonus"] == "2")
        self.assertEqual(v_quirk["exact_rational"], "80")
        self.assertTrue(v_quirk["intermediate"]["quirk_triggered"])

        # Find base 50, wit 2.0 (diff == 50 > 20: 100+20=120 -> capped at 90)
        v_capped = next(v for v in vecs if v["inputs"]["base_restore_percent"] == 50 and v["inputs"]["wit_bonus"] == "2")
        self.assertEqual(v_capped["exact_rational"], "90")

    def test_do_revive_65_percent_hp_and_mp_cp_unchanged(self):
        vecs = self.formulas["doRevive"]
        # Dead player maxHp 1000, curHp 0 -> HP 650, MP and CP preserved
        v = next(v for v in vecs if v["inputs"]["max_hp"] == 1000 and v["inputs"]["current_hp"] == 0)
        self.assertEqual(v["rounded_result"]["hp"], 650)
        self.assertEqual(v["rounded_result"]["mp"], v["inputs"]["current_mp"])
        self.assertEqual(v["rounded_result"]["cp"], v["inputs"]["current_cp"])

    def test_cast_time_500ms_floor(self):
        vecs = self.formulas["calcCastTime"]
        # hit_time 1000, speed 1000 (calculated 333 ms < 500 ms, hit_time > 500 -> clamped to 500 ms)
        v = next(v for v in vecs if v["inputs"]["hit_time"] == 1000 and v["inputs"]["speed"] == 1000 and not v["inputs"]["is_static"])
        self.assertEqual(v["rounded_result"], 500)
        self.assertEqual(v["ticks_result"], 5)
        self.assertTrue(v["intermediate"]["clamped_by_500ms_floor"])

        # hit_time 300 <= 500 -> floor does NOT clamp
        v2 = next(v for v in vecs if v["inputs"]["hit_time"] == 300 and not v["inputs"]["is_static"])
        self.assertEqual(v2["rounded_result"], 300)
        self.assertEqual(v2["ticks_result"], 3)
        self.assertFalse(v2["intermediate"]["clamped_by_500ms_floor"])

    def test_karma_gain_integer_boundaries(self):
        vecs = self.formulas["calculateKarmaGain"]
        v0 = next(v for v in vecs if v["inputs"]["pk_count"] == 0)
        self.assertEqual(v0["rounded_result"], 720)

        v98 = next(v for v in vecs if v["inputs"]["pk_count"] == 98)
        self.assertEqual(v98["rounded_result"], 98 * 360 + 720)

        v99 = next(v for v in vecs if v["inputs"]["pk_count"] == 99)
        self.assertEqual(v99["rounded_result"], 99 * 90 + 27180)

        v179 = next(v for v in vecs if v["inputs"]["pk_count"] == 179)
        self.assertEqual(v179["rounded_result"], 179 * 90 + 27180)

        v180 = next(v for v in vecs if v["inputs"]["pk_count"] == 180)
        self.assertEqual(v180["rounded_result"], 43200)


if __name__ == "__main__":
    unittest.main()
