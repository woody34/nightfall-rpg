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
            "TickHp",
            "cast_staging",
            "Heal",
            "FuncAtkEvasion",
            "stun_removal_condition",
        ]
        self.assertEqual(len(self.formulas), 23)
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

    def test_tick_hp_non_lethal_clamp(self):
        vecs = self.formulas["TickHp"]
        # Lethal DoT against 200 HP target with power -500: raw 1665 clamped to 199 dmg, surviving with 1 HP
        v_lethal = next(v for v in vecs if v["inputs"]["current_hp"] == 200 and v["inputs"]["power"] == "-500")
        self.assertTrue(v_lethal["intermediate"]["clamped_by_non_lethal"])
        self.assertEqual(v_lethal["rounded_result"]["final_hp"], 1)
        self.assertEqual(v_lethal["rounded_result"]["amount"], 199)

        # DoT against 1 HP target: clamped to 0 dmg, surviving with 1 HP
        v_1hp = next(v for v in vecs if v["inputs"]["current_hp"] == 1 and v["inputs"]["power"] == "-50")
        self.assertTrue(v_1hp["intermediate"]["clamped_by_non_lethal"])
        self.assertEqual(v_1hp["rounded_result"]["final_hp"], 1)
        self.assertEqual(v_1hp["rounded_result"]["amount"], 0)

        # EffectTickRatio multiplier check: ticks=5 * 666 / 1000 = 3.33 -> power=100 HoT gives 333
        v_hot = next(v for v in vecs if v["inputs"]["power"] == "100" and v["inputs"]["ticks"] == 5 and v["inputs"]["current_hp"] == 500)
        self.assertEqual(v_hot["rounded_result"]["amount"], 333)
        self.assertEqual(v_hot["intermediate"]["interval_ms"], 3330)

    def test_cast_staging_times(self):
        vecs = self.formulas["cast_staging"]
        # Standard 1080 ms cast at base speed: cutoff = floor(10.8) - 2 = 8 ticks
        v_std = next(v for v in vecs if v["inputs"]["hit_time_ms"] == 1080 and not v["inputs"]["is_continuous"] and not v["inputs"]["is_static"] and v["inputs"]["cast_speed"] == 333)
        self.assertEqual(v_std["rounded_result"]["interrupt_cutoff_offset_ticks"], 8)
        self.assertEqual(v_std["rounded_result"]["interrupt_cutoff_tick"], 108)
        self.assertEqual(v_std["intermediate"]["launch_offset_ms"], 680)
        self.assertEqual(v_std["intermediate"]["impact_offset_ms"], 1080)

        # Continuous 1080 ms cast: anim adjusted by -300 ms (effective 780 ms), launch=380 ms, impact=780 ms
        # but cutoff uses original anim_ms (8 ticks)
        v_cont = next(v for v in vecs if v["inputs"]["hit_time_ms"] == 1080 and v["inputs"]["is_continuous"] and not v["inputs"]["is_static"] and v["inputs"]["cast_speed"] == 333)
        self.assertEqual(v_cont["intermediate"]["effective_anim_ms"], 780)
        self.assertEqual(v_cont["intermediate"]["launch_offset_ms"], 380)
        self.assertEqual(v_cont["intermediate"]["impact_offset_ms"], 780)
        self.assertEqual(v_cont["rounded_result"]["interrupt_cutoff_offset_ticks"], 8)

        # 500 ms cast: cutoff = 5 - 2 = 3 ticks, launch = 100 ms, impact = 500 ms
        v_short = next(v for v in vecs if v["inputs"]["hit_time_ms"] == 500 and not v["inputs"]["is_continuous"] and not v["inputs"]["is_static"] and v["inputs"]["cast_speed"] == 333)
        self.assertEqual(v_short["rounded_result"]["interrupt_cutoff_offset_ticks"], 3)
        self.assertEqual(v_short["intermediate"]["launch_offset_ms"], 100)
        self.assertEqual(v_short["intermediate"]["impact_offset_ms"], 500)

    def test_heal_branches(self):
        vecs = self.formulas["Heal"]
        # Nongrade weapon without shots: mAtkMul = 1 + 1 = 2
        v_ng = next(v for v in vecs if v["inputs"]["weapon_grade"] == "NONE" and not v["inputs"]["is_static"] and v["inputs"]["power"] == 100 and v["inputs"]["m_atk"] == 200 and not v["inputs"]["bss"])
        self.assertEqual(v_ng["intermediate"]["m_atk_multiplier"], 2)

        # S80 weapon without shots: mAtkMul = 2 + 1 = 3
        v_s80 = next(v for v in vecs if v["inputs"]["weapon_grade"] == "S80" and not v["inputs"]["is_static"] and not v["inputs"]["bss"])
        self.assertEqual(v_s80["intermediate"]["m_atk_multiplier"], 3)

        # S84 weapon without shots: mAtkMul = 4 + 1 = 5
        v_s84 = next(v for v in vecs if v["inputs"]["weapon_grade"] == "S84" and not v["inputs"]["is_static"] and not v["inputs"]["bss"])
        self.assertEqual(v_s84["intermediate"]["m_atk_multiplier"], 5)

        # Static heal: bypasses mAtk and crit, heal amount equals power (clamped only by recoverable)
        v_static = next(v for v in vecs if v["inputs"]["is_static"] and v["inputs"]["power"] == 500 and v["inputs"]["current_hp"] == 200 and v["inputs"]["max_recoverable_hp"] == 1000)
        self.assertEqual(v_static["exact_rational"], "500")

        # Overheal clamping: power 500 against 50 HP deficit -> clamped to 50
        v_over = next(v for v in vecs if v["inputs"]["current_hp"] == 950 and v["inputs"]["max_recoverable_hp"] == 1000 and v["inputs"]["power"] == 500)
        self.assertEqual(v_over["exact_rational"], "50")
        self.assertEqual(v_over["rounded_result"]["final_hp"], 1000)

    def test_func_atk_evasion_intermediate_cast(self):
        vecs = self.formulas["FuncAtkEvasion"]
        # Level 1 DEX 20 player: intermediate cast yields 27 vs without cast 28
        v_base = next(v for v in vecs if v["inputs"]["level"] == 1 and v["inputs"]["dex"] == 20 and v["inputs"]["is_player"] and v["inputs"]["modifier"]["op"] == "none")
        self.assertEqual(v_base["intermediate"]["intermediate_cast"], 27)
        self.assertEqual(v_base["rounded_result"]["evasion_with_intermediate_cast"], 27)
        self.assertEqual(v_base["rounded_result"]["evasion_without_intermediate_cast"], 28)
        self.assertTrue(v_base["intermediate"]["has_intermediate_discrepancy"])

        # Level 1 DEX 20 player with +4 mod: cast gives 31 (27+4) vs without cast 32 (round(31.83))
        v_plus4 = next(v for v in vecs if v["inputs"]["level"] == 1 and v["inputs"]["dex"] == 20 and v["inputs"]["is_player"] and v["inputs"]["modifier"]["op"] == "add" and v["inputs"]["modifier"]["val"] == "4")
        self.assertEqual(v_plus4["rounded_result"]["evasion_with_intermediate_cast"], 31)
        self.assertEqual(v_plus4["rounded_result"]["evasion_without_intermediate_cast"], 32)
        self.assertTrue(v_plus4["intermediate"]["has_intermediate_discrepancy"])

        # Level 1 DEX 20 player with 1.1x buff: cast gives 30 (round(29.7)) vs without cast 31 (round(30.62))
        v_buff = next(v for v in vecs if v["inputs"]["level"] == 1 and v["inputs"]["dex"] == 20 and v["inputs"]["is_player"] and v["inputs"]["modifier"]["op"] == "mul" and v["inputs"]["modifier"]["val"] == "11/10")
        self.assertEqual(v_buff["rounded_result"]["evasion_with_intermediate_cast"], 30)
        self.assertEqual(v_buff["rounded_result"]["evasion_without_intermediate_cast"], 31)
        self.assertTrue(v_buff["intermediate"]["has_intermediate_discrepancy"])

    def test_stun_removal_roll_condition(self):
        vecs = self.formulas["stun_removal_condition"]
        # Direct attack on stunned player, roll=0 -> stun REMOVED, roll consumed
        v_rem = next(v for v in vecs if v["inputs"]["damage_cause"] == "DIRECT_ATTACK" and v["inputs"]["target_is_stunned"] and v["inputs"]["roll_draw"] == 0)
        self.assertTrue(v_rem["rounded_result"]["stun_removed"])
        self.assertTrue(v_rem["rounded_result"]["roll_consumed"])

        # Direct attack on stunned player, roll=1 -> stun retained, roll consumed
        v_ret = next(v for v in vecs if v["inputs"]["damage_cause"] == "DIRECT_ATTACK" and v["inputs"]["target_is_stunned"] and v["inputs"]["roll_draw"] == 1)
        self.assertFalse(v_ret["rounded_result"]["stun_removed"])
        self.assertTrue(v_ret["rounded_result"]["roll_consumed"])

        # DoT on stunned player, roll=0 -> stun retained, roll NOT consumed (!isDOT guard)
        v_dot = next(v for v in vecs if v["inputs"]["damage_cause"] == "DOT" and v["inputs"]["target_is_stunned"] and v["inputs"]["roll_draw"] == 0)
        self.assertFalse(v_dot["rounded_result"]["stun_removed"])
        self.assertFalse(v_dot["rounded_result"]["roll_consumed"])

        # HP consumption on stunned player, roll=0 -> stun retained, roll NOT consumed (!isHPConsumption guard)
        v_hp_con = next(v for v in vecs if v["inputs"]["damage_cause"] == "HP_CONSUMPTION" and v["inputs"]["target_is_stunned"] and v["inputs"]["roll_draw"] == 0)
        self.assertFalse(v_hp_con["rounded_result"]["stun_removed"])
        self.assertFalse(v_hp_con["rounded_result"]["roll_consumed"])

        # Direct attack on non-stunned player -> roll NOT consumed
        v_non_stunned = next(v for v in vecs if v["inputs"]["damage_cause"] == "DIRECT_ATTACK" and not v["inputs"]["target_is_stunned"] and v["inputs"]["roll_draw"] == 0)
        self.assertFalse(v_non_stunned["rounded_result"]["stun_removed"])
        self.assertFalse(v_non_stunned["rounded_result"]["roll_consumed"])


if __name__ == "__main__":
    unittest.main()
