#!/usr/bin/env python3
"""Independent skill formula oracle for Nightfall Phase 3.

Python 3.11+, standard library only, no floating-point arithmetic in golden outputs.
Uses exact Fraction arithmetic and rational root intervals certified by isqrt.

Covers all 13 required skill formulas:
1. calcSkillPhysDam (+ skill crit)
2. calcMagicDam (+ magic crit, spiritshot absent)
3. calcMagicSuccess
4. calcEffectSuccess (with traits, mAtkMod, DEBUFF_VULN default)
5. effect duration (abnormal time with stat/resist mods as in source)
6. calcAtkBreak
7. calcAttributeBonus (attack from skill only, defence 0, weapon none)
8. calcHpRegen / calcMpRegen / calcCpRegen
9. calculateKarmaGain / calculateKarmaLost
10. calculateSkillResurrectRestorePercent EXACT source quirk
11. calcCastTime (magic vs physical vs static, 500 ms floor rule)
12. doRevive vitals (HP to 65% if lower; MP/CP unchanged)
13. M.Atk / M.Def / MAtkSpd derivations (FuncMAtkMod / FuncMDefMod / FuncMAtkSpeed)

Run `python3 packages/data/scripts/skill_oracle.py [--check]`.
"""

import argparse
from fractions import Fraction as F
import hashlib
import json
from math import isqrt
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[1]
Q = 10**6

L2J_GAME_REV = "abfde0490ac52a2106e884b7242c0276c44cdf76"
L2J_DP_REV = "3ca488dd2bd0bfaca43e378886a3c2e37968153a"


def read_toml(rel: str) -> dict:
    return tomllib.loads((ROOT / rel).read_text(encoding="utf-8"))


def floor(x: F) -> int:
    return x.numerator // x.denominator


def ceil(x: F) -> int:
    return -floor(-x)


def rounded(x: F) -> int:
    """Java Math.round rule: floor(x + 1/2)."""
    return floor(x + F(1, 2))


def q_scale(x: F) -> int:
    return floor(x * Q)


def root_bounds(n: F) -> tuple[F, F]:
    """Bounds sqrt(n) within a fine rational interval [lo, hi]."""
    n = F(n)
    if n == 0:
        return F(0), F(0)
    scale = 10**12
    a, b = n.numerator, n.denominator
    ab = a * b
    lo = isqrt(ab * scale**2)
    hi = lo if lo * lo == ab * scale**2 else lo + 1
    return F(lo, b * scale), F(hi, b * scale)


def binary_nearest(x: F, bits: int) -> F:
    """Positive, normal IEEE value rounded ties-even, represented as a Fraction.

    Used to model Java float/double cast semantics without host floats.
    """
    assert x > 0
    exponent = x.numerator.bit_length() - x.denominator.bit_length()
    power = lambda n: F(2**n) if n >= 0 else F(1, 2**(-n))
    if x < power(exponent):
        exponent -= 1
    unit = power(exponent - bits + 1)
    a = x / unit
    k = floor(a)
    r = a - k
    k += r > F(1, 2) or (r == F(1, 2) and k % 2 == 1)
    return k * unit


def format_fraction(x: F) -> str:
    """Format fraction as exact string: integer or n/d."""
    if x.denominator == 1:
        return str(x.numerator)
    return f"{x.numerator}/{x.denominator}"


# ---------------------------------------------------------------------------
# Formula 1: calcSkillPhysDam (+ skill crit)
# ---------------------------------------------------------------------------
def make_calc_skill_phys_dam_vectors() -> list[dict]:
    source_lines = "Formulas.java:746-809, PhysicalAttack.java:86-96"
    vectors = []

    powers = [0, 1, 10, 25, 50, 100, 250, 500, 1000, 1204, 2000, 5000]
    p_atks = [1, 10, 50, 100, 250, 500, 1000, 2500]
    p_defs = [1, 10, 30, 50, 100, 250, 500, 1000, 50000]
    proximities = ["front", "side", "behind"]

    case_configs = [
        # (power, p_atk, p_def, proximity, soulshot, crit, desc)
        (0, 10, 10, "front", False, False, "Zero power baseline"),
        (1, 1, 1, "front", False, False, "Minimal baseline: 1/1/1"),
        (1, 1, 50000, "front", False, False, "Damage clamped to 1 due to high defence"),
        (10, 20, 30, "front", False, False, "Low tier physical skill"),
        (50, 50, 50, "front", False, False, "Symmetric power/atk/def"),
        (100, 100, 100, "front", False, False, "Base 100 stats"),
        (100, 100, 100, "side", False, False, "Side proximity bonus 1.1x"),
        (100, 100, 100, "behind", False, False, "Behind proximity bonus 1.2x"),
        (100, 100, 100, "front", True, False, "Soulshot charged (2x pAtk)"),
        (100, 100, 100, "front", False, True, "Skill crit (2x damage)"),
        (100, 100, 100, "side", True, True, "Side + Soulshot + Crit"),
        (100, 100, 100, "behind", True, True, "Behind + Soulshot + Crit"),
        (250, 150, 80, "front", False, False, "Mid-level combat strike"),
        (250, 150, 80, "behind", False, True, "Mid-level crit backstab"),
        (500, 300, 150, "front", False, False, "High-tier skill front"),
        (500, 300, 150, "behind", True, True, "High-tier backstab crit with soulshot"),
        (1204, 500, 250, "front", False, False, "Mortal Blow / Power Strike high rank"),
        (1204, 500, 250, "front", False, True, "High rank skill crit"),
        (2000, 1000, 500, "front", False, False, "Epic tier strike front"),
        (2000, 1000, 500, "behind", True, True, "Epic tier full multiplier"),
        (5000, 2500, 1000, "front", False, False, "Maximum parameter scale"),
        (5000, 2500, 1000, "behind", True, True, "Maximum parameter scale crit behind"),
    ]

    for power, p_atk, p_def, prox, ss, crit, desc in case_configs:
        vectors.append((power, p_atk, p_def, prox, ss, crit, desc))

    # Add systematic boundary grid
    for p in [10, 100, 500]:
        for d in [20, 100, 500]:
            for crit in [False, True]:
                if len(vectors) < 50:
                    vectors.append((p, 100, d, "front", False, crit, f"Grid p={p}, d={d}, crit={crit}"))

    # Add near-integer boundary tests
    near_integers = [
        # (power, p_atk, p_def, prox, ss, crit, desc)
        (76, 1, 77, "front", False, False, "Near 77: 77*(76+1)/77 = 77 exactly"),
        (76, 2, 77, "front", False, False, "Near 77+: 77*78/77 = 78 exactly"),
        (75, 1, 77, "front", False, False, "Near 77-: 77*76/77 = 76 exactly"),
        (10, 1, 100, "front", False, False, "Fractional damage: 77*11/100 = 8.47"),
        (10, 1, 100, "front", False, True, "Fractional crit: 8.47 * 2 = 16.94"),
        (1, 1, 77, "front", False, False, "Fractional damage: 77*2/77 = 2 exactly"),
        (1, 0, 77, "front", False, False, "Fractional damage: 77*1/77 = 1 exactly"),
    ]
    for row in near_integers:
        if len(vectors) < 50:
            vectors.append(row)

    out = []
    for power, p_atk, p_def, prox, ss, crit, desc in vectors:
        pow_f = F(power)
        atk_f = F(p_atk)
        def_f = F(p_def)
        ss_boost = F(2) if ss else F(1)

        base_mod = (F(77) * (pow_f + atk_f * ss_boost)) / def_f
        if prox == "behind":
            prox_mul = F(6, 5)
        elif prox == "side":
            prox_mul = F(11, 10)
        else:
            prox_mul = F(1)

        dmg = base_mod * prox_mul
        dmg = max(F(1), dmg)
        if crit:
            dmg *= F(2)

        rounded_res = floor(dmg)

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "power": power,
                "p_atk": p_atk,
                "defence": p_def,
                "proximity": prox,
                "soulshot": ss,
                "crit": crit,
            },
            "intermediate": {
                "base_mod": format_fraction(base_mod),
                "proximity_bonus": format_fraction(prox_mul),
            },
            "exact_rational": format_fraction(dmg),
            "q_result": q_scale(dmg),
            "rounded_result": rounded_res,
            "rounding_rule": "(int) truncation at PhysicalAttack.java:98",
        })

    assert len(out) >= 40, f"Expected >= 40 vectors, got {len(out)}"
    return out


# ---------------------------------------------------------------------------
# Formula 2: calcMagicDam (+ magic crit, spiritshot absent)
# ---------------------------------------------------------------------------
def make_calc_magic_dam_vectors() -> list[dict]:
    source_lines = "Formulas.java:815-898, MagicalAttack.java:73"
    vectors = []

    # Using perfect squares for m_atk to yield exact rationals, plus bounded checks
    configs = [
        # (power, m_atk, m_def, is_pvp, mcrit, failure, second_success, lvl_diff, attr_bonus, desc)
        (10, 1, 10, False, False, False, False, 0, F(1), "Minimal magic strike: mAtk=1"),
        (20, 4, 15, False, False, False, False, 0, F(1), "Low magic strike: mAtk=4 (sqrt=2)"),
        (35, 9, 20, False, False, False, False, 0, F(1), "Low-mid strike: mAtk=9 (sqrt=3)"),
        (50, 16, 25, False, False, False, False, 0, F(1), "Rank 1 Wind Strike tier: mAtk=16 (sqrt=4)"),
        (50, 25, 30, False, False, False, False, 0, F(1), "PvE normal hit: mAtk=25 (sqrt=5)"),
        (50, 25, 30, False, True, False, False, 0, F(1), "PvE magic crit (3.0x): mAtk=25"),
        (50, 25, 30, True, False, False, False, 0, F(1), "PvP normal hit: mAtk=25"),
        (50, 25, 30, True, True, False, False, 0, F(1), "PvP magic crit (2.5x): mAtk=25"),
        (75, 36, 40, False, False, False, False, 0, F(1), "Mid tier: mAtk=36 (sqrt=6)"),
        (75, 49, 45, False, False, False, False, 0, F(1), "Mid tier: mAtk=49 (sqrt=7)"),
        (100, 64, 50, False, False, False, False, 0, F(1), "Mid-high: mAtk=64 (sqrt=8)"),
        (100, 81, 55, False, False, False, False, 0, F(1), "Mid-high: mAtk=81 (sqrt=9)"),
        (100, 100, 60, False, False, False, False, 0, F(1), "High tier: mAtk=100 (sqrt=10)"),
        (100, 100, 60, False, True, False, False, 0, F(1), "High tier PvE crit: 3x"),
        (100, 100, 60, True, True, False, False, 0, F(1), "High tier PvP crit: 2.5x"),
        (120, 144, 70, False, False, False, False, 0, F(1), "Advanced tier: mAtk=144 (sqrt=12)"),
        (150, 225, 80, False, False, False, False, 0, F(1), "High power: mAtk=225 (sqrt=15)"),
        (180, 400, 100, False, False, False, False, 0, F(1), "High rank: mAtk=400 (sqrt=20)"),
        (200, 625, 120, False, False, False, False, 0, F(1), "Master tier: mAtk=625 (sqrt=25)"),
        (250, 900, 150, False, False, False, False, 0, F(1), "Endgame strike: mAtk=900 (sqrt=30)"),
        (300, 1600, 200, False, False, False, False, 0, F(1), "Epic strike: mAtk=1600 (sqrt=40)"),
        (400, 2500, 250, False, False, False, False, 0, F(1), "Max test strike: mAtk=2500 (sqrt=50)"),
        # Failure calculations
        (50, 100, 50, False, False, True, True, 0, F(1), "Magic failure: 2nd success, diff=0 <= 9 -> half damage"),
        (50, 100, 50, False, False, True, True, 5, F(1), "Magic failure: 2nd success, diff=5 <= 9 -> half damage"),
        (50, 100, 50, False, False, True, True, 9, F(1), "Magic failure: 2nd success, diff=9 <= 9 -> half damage"),
        (50, 100, 50, False, False, True, True, 10, F(1), "Magic failure: 2nd success, diff=10 > 9 -> resisted (1 dmg)"),
        (50, 100, 50, False, False, True, True, 15, F(1), "Magic failure: 2nd success, diff=15 > 9 -> resisted (1 dmg)"),
        (50, 100, 50, False, False, True, False, 0, F(1), "Magic failure: 2nd fail -> resisted (1 dmg)"),
        (50, 100, 50, False, False, True, False, 5, F(1), "Magic failure: 2nd fail, diff=5 -> resisted (1 dmg)"),
        # With attribute bonus
        (50, 100, 50, False, False, False, False, 0, F(11, 10), "With attribute bonus 1.1x"),
        (50, 100, 50, False, False, False, False, 0, F(12, 10), "With attribute bonus 1.2x"),
        (50, 100, 50, False, True, False, False, 0, F(11, 10), "PvE crit + attribute bonus 1.1x"),
        (50, 100, 50, True, True, False, False, 0, F(11, 10), "PvP crit + attribute bonus 1.1x"),
        # High M.Def boundary
        (10, 1, 1000, False, False, False, False, 0, F(1), "Extreme mDef: 91*1/1000 * 10 = 0.91 -> int 0"),
        (20, 4, 1000, False, False, False, False, 0, F(1), "High mDef: 91*2/1000 * 20 = 3.64 -> int 3"),
        # Non-perfect squares with root_bounds certificate
        (30, 2, 20, False, False, False, False, 0, F(1), "Non-perfect square mAtk=2"),
        (30, 3, 20, False, False, False, False, 0, F(1), "Non-perfect square mAtk=3"),
        (30, 5, 20, False, False, False, False, 0, F(1), "Non-perfect square mAtk=5"),
        (30, 7, 20, False, False, False, False, 0, F(1), "Non-perfect square mAtk=7"),
        (30, 8, 20, False, False, False, False, 0, F(1), "Non-perfect square mAtk=8"),
        (50, 10, 30, False, False, False, False, 0, F(1), "Non-perfect square mAtk=10"),
        (50, 15, 30, False, False, False, False, 0, F(1), "Non-perfect square mAtk=15"),
        (50, 20, 30, False, False, False, False, 0, F(1), "Non-perfect square mAtk=20"),
        (50, 50, 40, False, False, False, False, 0, F(1), "Non-perfect square mAtk=50"),
        (50, 75, 40, False, False, False, False, 0, F(1), "Non-perfect square mAtk=75"),
        (100, 150, 60, False, False, False, False, 0, F(1), "Non-perfect square mAtk=150"),
        (100, 200, 60, False, False, False, False, 0, F(1), "Non-perfect square mAtk=200"),
        (100, 300, 80, False, False, False, False, 0, F(1), "Non-perfect square mAtk=300"),
        (100, 500, 100, False, False, False, False, 0, F(1), "Non-perfect square mAtk=500"),
    ]

    out = []
    for power, m_atk, m_def, pvp, mcrit, fail, sec_succ, diff, attr, desc in configs:
        pow_f = F(power)
        mdef_f = F(m_def)
        lo_sq, hi_sq = root_bounds(F(m_atk))

        def calc_dmg(sq: F) -> F:
            dmg = ((F(91) * sq) / mdef_f) * pow_f
            if fail:
                if sec_succ and diff <= 9:
                    dmg = dmg / F(2)
                else:
                    dmg = F(1)
            elif mcrit:
                dmg *= F(5, 2) if pvp else F(3)
            dmg *= attr
            return dmg

        lo_d = calc_dmg(lo_sq)
        hi_d = calc_dmg(hi_sq)
        assert floor(lo_d) == floor(hi_d), f"Ambiguous sqrt rounding for {desc}"

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "power": power,
                "m_atk": m_atk,
                "m_def": m_def,
                "is_pvp": pvp,
                "mcrit": mcrit,
                "magic_failure": fail,
                "second_success": sec_succ,
                "lvl_diff": diff,
                "attribute_bonus": format_fraction(attr),
            },
            "exact_rational": format_fraction(lo_d),
            "q_result": q_scale(lo_d),
            "rounded_result": floor(lo_d),
            "rounding_rule": "(int) truncation at MagicalAttack.java:88",
        })

    assert len(out) >= 40, f"Expected >= 40 vectors, got {len(out)}"
    return out


# ---------------------------------------------------------------------------
# Formula 3: calcMagicSuccess
# ---------------------------------------------------------------------------
def make_calc_magic_success_vectors() -> list[dict]:
    source_lines = "Formulas.java:1286-1314"
    vectors = []

    # lvl_diff from -20 to +25, plus res_modifier variations
    diffs = list(range(-15, 26))  # 41 diff values
    for d in diffs:
        vectors.append((d, F(1), F(1), f"Level difference d={d}"))

    # Additional boundary and modifier variations
    extras = [
        (-25, F(1), F(1), "Extremely under-leveled target d=-25"),
        (-20, F(1), F(1), "Large negative d=-20"),
        (0, F(8, 10), F(1), "Same level, target resModifier=0.8"),
        (0, F(12, 10), F(1), "Same level, target resModifier=1.2"),
        (0, F(15, 10), F(1), "Same level, target resModifier=1.5"),
        (3, F(9, 10), F(1), "d=3, resModifier=0.9"),
        (5, F(11, 10), F(1), "d=5, resModifier=1.1"),
        (10, F(1), F(8, 10), "d=10, NPC penalty targetModifier=0.8"),
        (15, F(1), F(5, 10), "d=15, targetModifier=0.5"),
    ]
    vectors.extend(extras)

    out = []
    for diff, res_mod, tgt_mod, desc in vectors:
        if diff >= 0:
            lvl_mod_exact = F(13, 10) ** diff
        else:
            lvl_mod_exact = F(10, 13) ** (-diff)

        raw = lvl_mod_exact * tgt_mod * res_mod
        flt = binary_nearest(raw, 24)
        subtrahend = rounded(flt)
        rate = 100 - subtrahend

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "lvl_difference": diff,
                "res_modifier": format_fraction(res_mod),
                "target_modifier": format_fraction(tgt_mod),
            },
            "intermediate": {
                "lvl_modifier_exact": format_fraction(lvl_mod_exact),
                "raw_product": format_fraction(raw),
                "binary32_float_val": format_fraction(flt),
                "subtrahend_rounded": subtrahend,
            },
            "exact_rational": format_fraction(raw),
            "rounded_result": rate,
            "rounding_rule": "Math.round((float) ...) at Formulas.java:1301; rate = 100 - subtrahend",
        })

    assert len(out) >= 40, f"Expected >= 40 vectors, got {len(out)}"
    return out


# ---------------------------------------------------------------------------
# Formula 4: calcEffectSuccess (with traits, mAtkMod, DEBUFF_VULN default)
# ---------------------------------------------------------------------------
def make_calc_effect_success_vectors() -> list[dict]:
    source_lines = "Formulas.java:1158-1232"
    vectors = []

    # Systematics covering all branches and boundaries
    configs = [
        # (magic_lvl, target_lvl, act_rate, lvl_bonus, target_stat, prop, is_magic, m_atk, m_def, trait_mod, elem_mod, buff_mod, min_c, max_c, desc)
        (0, 1, -1, 0, 20, "MEN", False, 0, 1, F(1), F(1), F(101, 100), 1, 99, "activateRate == -1 unconditional success"),
        (0, 1, 50, 1, 20, "NONE", False, 0, 1, F(1), F(1), F(101, 100), 1, 99, "basicProperty == NONE unconditional success"),
        (-1, 20, 50, 2, 25, "MEN", False, 0, 1, F(1), F(1), F(101, 100), 1, 99, "magicLevel <= -1 defaults to targetLvl + 3"),
        (0, 20, 50, 0, 25, "MEN", False, 0, 1, F(0), F(1), F(101, 100), 1, 99, "traitMod == 0 (trait immunity yields 0%)"),
        (0, 50, 10, 1, 99, "CON", False, 0, 1, F(1), F(1), F(101, 100), 1, 99, "Extreme high stat MEN=99 hitting minChance clamp"),
        (50, 1, 90, 5, 10, "INT", False, 0, 1, F(1), F(1), F(101, 100), 1, 99, "Over-leveled high rate hitting maxChance clamp"),
        (20, 20, 40, 2, 25, "MEN", False, 0, 1, F(1), F(1), F(101, 100), 1, 99, "Physical debuff default DEBUFF_VULN 1.01"),
        (20, 20, 40, 2, 25, "MEN", False, 0, 1, F(1), F(1), F(1), 1, 99, "Physical debuff DEBUFF_VULN 1.00"),
        (20, 20, 40, 2, 25, "MEN", False, 0, 1, F(1), F(1), F(12, 10), 1, 99, "Physical debuff DEBUFF_VULN 1.20"),
        (20, 20, 40, 2, 25, "MEN", False, 0, 1, F(8, 10), F(1), F(101, 100), 1, 99, "Physical debuff trait resistance 0.8"),
        (20, 20, 40, 2, 25, "MEN", False, 0, 1, F(1), F(11, 10), F(101, 100), 1, 99, "Physical debuff element mod 1.1"),
        (20, 20, 40, 2, 25, "MEN", False, 0, 1, F(1), F(1), F(101, 100), 10, 90, "Custom clamp range [10, 90]"),
        # Magic debuffs: mAtkMod = (sqrt(mAtk) / mDef) * 11
        (20, 20, 40, 2, 25, "MEN", True, 100, 50, F(1), F(1), F(101, 100), 1, 99, "Magic debuff: mAtk=100 (sqrt=10), mDef=50"),
        (20, 20, 40, 2, 25, "MEN", True, 25, 50, F(1), F(1), F(101, 100), 1, 99, "Magic debuff: mAtk=25 (sqrt=5), mDef=50"),
        (20, 20, 40, 2, 25, "MEN", True, 400, 100, F(1), F(1), F(101, 100), 1, 99, "Magic debuff: mAtk=400 (sqrt=20), mDef=100"),
        (20, 20, 40, 2, 25, "MEN", True, 900, 150, F(1), F(1), F(101, 100), 1, 99, "Magic debuff: mAtk=900 (sqrt=30), mDef=150"),
    ]

    # Add level difference sweep
    for diff in [-10, -5, -2, 0, 2, 5, 10]:
        for act in [20, 50, 80]:
            if len(configs) < 50:
                configs.append((20 + diff, 20, act, 2, 25, "MEN", False, 0, 1, F(1), F(1), F(101, 100), 1, 99, f"Level diff {diff}, actRate {act}"))

    # Add stat variations
    for stat in [10, 20, 30, 40, 50, 60, 70, 80]:
        if len(configs) < 50:
            configs.append((25, 25, 50, 2, stat, "MEN", False, 0, 1, F(1), F(1), F(101, 100), 1, 99, f"Target stat MEN={stat}"))

    out = []
    for mlvl, tlvl, act, lbr, tstat, prop, is_mag, matk, mdef, trt, elem, buff, min_c, max_c, desc in configs:
        if act == -1 or prop == "NONE":
            final_rate = F(100)
            rounded_res = 100
            base_mod = F(0)
            matk_mod = F(1)
        else:
            eff_mlvl = (tlvl + 3) if mlvl <= -1 else mlvl
            base_mod = F((eff_mlvl - tlvl + 3) * lbr + act + 30 - tstat)
            if is_mag:
                lo_sq, _ = root_bounds(F(matk))
                matk_mod = (lo_sq / F(mdef)) * F(11)
            else:
                matk_mod = F(1)

            rate = base_mod * elem * trt * matk_mod * buff
            if trt > 0:
                final_rate = max(F(min_c), min(F(max_c), rate))
            else:
                final_rate = F(0)
            rounded_res = floor(final_rate)

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "magic_level": mlvl,
                "target_level": tlvl,
                "activate_rate": act,
                "lvl_bonus_rate": lbr,
                "target_base_stat": tstat,
                "basic_property": prop,
                "is_magic": is_mag,
                "m_atk": matk,
                "m_def": mdef,
                "trait_mod": format_fraction(trt),
                "element_mod": format_fraction(elem),
                "buff_debuff_mod": format_fraction(buff),
                "min_chance": min_c,
                "max_chance": max_c,
            },
            "intermediate": {
                "base_mod": format_fraction(base_mod),
                "m_atk_mod": format_fraction(matk_mod),
            },
            "exact_rational": format_fraction(final_rate),
            "q_result": q_scale(final_rate),
            "rounded_result": rounded_res,
            "rounding_rule": "Continuous rate compared against roll; integer floor for discrete display",
        })

    assert len(out) >= 40, f"Expected >= 40 vectors, got {len(out)}"
    return out


# ---------------------------------------------------------------------------
# Formula 5: effect duration (calcEffectAbnormalTime)
# ---------------------------------------------------------------------------
def make_calc_effect_duration_vectors() -> list[dict]:
    source_lines = "Formulas.java:1760-1784, Formulas.java:1144"
    vectors = []

    # Buffs (exact time retained)
    for t in [10, 15, 20, 30, 60, 120, 300, 1200]:
        vectors.append((t, False, 0, 1, 1, 0, F(1), F(1), F(1), f"Buff duration {t}s (unmodified)"))

    # Debuffs with various modifiers
    debuff_cases = [
        # (time, mlvl, atk_lvl, tgt_lvl, lbr, stat_bonus, trait_mod, elem_mod, desc)
        (10, 20, 20, 20, 0, F(1), F(1), F(1), "Debuff baseline: 10s all 1.0"),
        (15, 20, 20, 20, 0, F(1), F(1), F(1), "Debuff baseline: 15s all 1.0"),
        (30, 20, 20, 20, 0, F(1), F(1), F(1), "Debuff baseline: 30s all 1.0"),
        (60, 20, 20, 20, 0, F(1), F(1), F(1), "Debuff baseline: 60s all 1.0"),
        # Saving stat reductions
        (30, 20, 20, 20, 0, F(12, 10), F(1), F(1), "Stat bonus 1.2 -> 30/1.2 = 25s"),
        (30, 20, 20, 20, 0, F(15, 10), F(1), F(1), "Stat bonus 1.5 -> 30/1.5 = 20s"),
        (30, 20, 20, 20, 0, F(2), F(1), F(1), "Stat bonus 2.0 -> 30/2.0 = 15s (floor boundary time*0.5)"),
        (30, 20, 20, 20, 0, F(25, 10), F(1), F(1), "Stat bonus 2.5 -> clamped to time*0.5 = 15s"),
        (30, 20, 20, 20, 0, F(3), F(1), F(1), "Stat bonus 3.0 -> clamped to time*0.5 = 15s"),
        # Trait resistance reductions
        (30, 20, 20, 20, 0, F(1), F(8, 10), F(1), "Trait mod 0.8 -> 24s"),
        (30, 20, 20, 20, 0, F(1), F(5, 10), F(1), "Trait mod 0.5 -> 15s (floor boundary)"),
        (30, 20, 20, 20, 0, F(1), F(4, 10), F(1), "Trait mod 0.4 -> clamped to 15s"),
        # Level difference variations
        (30, 25, 20, 20, 0, F(1), F(1), F(1), "Attacker magicLvl 25 vs target 20 (+5% duration)"),
        (30, 30, 20, 20, 0, F(1), F(1), F(1), "Attacker magicLvl 30 vs target 20 (+10% duration)"),
        (30, 15, 20, 20, 0, F(1), F(1), F(1), "Attacker magicLvl 15 vs target 20 (-5% duration)"),
        (30, 10, 20, 20, 0, F(1), F(1), F(1), "Attacker magicLvl 10 vs target 20 (-10% duration)"),
        # Level bonus rate
        (30, 20, 20, 20, 10, F(1), F(1), F(1), "lvlBonusRate 10% -> 1.1x duration"),
        (30, 20, 20, 20, 20, F(1), F(1), F(1), "lvlBonusRate 20% -> 1.2x duration"),
        (30, 20, 20, 20, 50, F(1), F(1), F(1), "lvlBonusRate 50% -> clamped to time (30s)"),
        # Element modifier
        (30, 20, 20, 20, 0, F(1), F(1), F(11, 10), "Element mod 1.1 -> clamped to 30s"),
        (30, 20, 20, 20, 0, F(12, 10), F(1), F(11, 10), "Element 1.1 + stat 1.2 -> 30*1.1/1.2 = 27.5s -> ceil 28s"),
        # Near-integer ceiling sites
        (10, 20, 20, 20, 0, F(101, 100), F(1), F(1), "10 / 1.01 = 9.9009s -> ceil to 10s"),
        (10, 20, 20, 20, 0, F(11, 10), F(1), F(1), "10 / 1.1 = 9.0909s -> ceil to 10s"),
        (10, 20, 20, 20, 0, F(125, 100), F(1), F(1), "10 / 1.25 = 8.0s -> ceil to 8s"),
        (10, 20, 20, 20, 0, F(13, 10), F(1), F(1), "10 / 1.3 = 7.6923s -> ceil to 8s"),
        (10, 20, 20, 20, 0, F(14, 10), F(1), F(1), "10 / 1.4 = 7.1428s -> ceil to 8s"),
        (10, 20, 20, 20, 0, F(15, 10), F(1), F(1), "10 / 1.5 = 6.6666s -> ceil to 7s"),
        (10, 20, 20, 20, 0, F(16, 10), F(1), F(1), "10 / 1.6 = 6.25s -> ceil to 7s"),
        (10, 20, 20, 20, 0, F(17, 10), F(1), F(1), "10 / 1.7 = 5.8823s -> ceil to 6s"),
        (10, 20, 20, 20, 0, F(18, 10), F(1), F(1), "10 / 1.8 = 5.5555s -> ceil to 6s"),
        (10, 20, 20, 20, 0, F(19, 10), F(1), F(1), "10 / 1.9 = 5.2631s -> ceil to 6s"),
        (10, 20, 20, 20, 0, F(2), F(1), F(1), "10 / 2.0 = 5.0s -> ceil to 5s"),
        (15, 20, 20, 20, 0, F(11, 10), F(1), F(1), "15 / 1.1 = 13.636s -> ceil to 14s"),
        (15, 20, 20, 20, 0, F(12, 10), F(1), F(1), "15 / 1.2 = 12.5s -> ceil to 13s"),
        (15, 20, 20, 20, 0, F(13, 10), F(1), F(1), "15 / 1.3 = 11.538s -> ceil to 12s"),
        (15, 20, 20, 20, 0, F(14, 10), F(1), F(1), "15 / 1.4 = 10.714s -> ceil to 11s"),
        (15, 20, 20, 20, 0, F(15, 10), F(1), F(1), "15 / 1.5 = 10.0s -> ceil to 10s"),
        (15, 20, 20, 20, 0, F(16, 10), F(1), F(1), "15 / 1.6 = 9.375s -> ceil to 10s"),
        (15, 20, 20, 20, 0, F(17, 10), F(1), F(1), "15 / 1.7 = 8.823s -> ceil to 9s"),
        (15, 20, 20, 20, 0, F(18, 10), F(1), F(1), "15 / 1.8 = 8.333s -> ceil to 9s"),
        (15, 20, 20, 20, 0, F(19, 10), F(1), F(1), "15 / 1.9 = 7.894s -> ceil to 8s"),
        (15, 20, 20, 20, 0, F(2), F(1), F(1), "15 / 2.0 = 7.5s -> ceil to 8s"),
    ]
    for row in debuff_cases:
        t, mlvl, alvl, tlvl, lbr, sb, trt, elem, desc = row
        vectors.append((t, True, mlvl, alvl, tlvl, lbr, sb, trt, elem, desc))

    out = []
    for t, is_deb, mlvl, alvl, tlvl, lbr, sb, trt, elem, desc in vectors:
        t_f = F(t)
        if is_deb:
            eff_alvl = mlvl if mlvl > 0 else alvl
            sk_bonus = F(1) + F(lbr, 100)
            lvl_mod = F(1) + F(eff_alvl - tlvl, 100)
            lvl_bonus_mod = sk_bonus * lvl_mod
            raw = (t_f * trt * lvl_bonus_mod * elem) / sb
            min_t = t_f * F(1, 2)
            max_t = t_f
            clamped = max(min_t, min(max_t, raw))
            sec = ceil(clamped)
        else:
            clamped = t_f
            sec = t
        ticks = sec * 10

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "abnormal_time": t,
                "is_debuff": is_deb,
                "magic_level": mlvl,
                "attacker_level": alvl,
                "target_level": tlvl,
                "lvl_bonus_rate": lbr,
                "stat_bonus": format_fraction(sb),
                "trait_mod": format_fraction(trt),
                "element_mod": format_fraction(elem),
            },
            "intermediate": {
                "clamped_seconds_rational": format_fraction(clamped),
                "ceil_seconds": sec,
            },
            "exact_rational": format_fraction(clamped),
            "rounded_result": sec,
            "ticks_result": ticks,
            "rounding_rule": "Math.ceil whole seconds at Formulas.java:1781, multiplied by 10 for ticks",
        })

    assert len(out) >= 40, f"Expected >= 40 vectors, got {len(out)}"
    return out


# ---------------------------------------------------------------------------
# Formula 6: calcAtkBreak
# ---------------------------------------------------------------------------
def make_calc_atk_break_vectors() -> list[dict]:
    source_lines = "Formulas.java:992-1025"
    vectors = []

    # dmg and men_bonus combinations
    # Perfect squares for 13*dmg:
    # 13*13 = 169 (sqrt 13)
    # 13*52 = 676 (sqrt 26)
    # 13*117 = 1521 (sqrt 39)
    # 13*208 = 2704 (sqrt 52)
    # 13*325 = 4225 (sqrt 65)
    # 13*468 = 6084 (sqrt 78)

    configs = [
        # (dmg, men_bonus, is_casting, desc)
        (100, F(1), False, "Not casting: breakCast = false"),
        (0, F(1), True, "Zero damage: 15 - 0 = 15%"),
        (0, F(13, 10), True, "Zero damage, MEN 1.3: 15 - 30 = -15 -> clamped to 1%"),
        (1, F(1), True, "Minimal damage: 1"),
        (2, F(1), True, "Damage 2"),
        (5, F(1), True, "Damage 5"),
        (10, F(1), True, "Damage 10"),
        (13, F(1), True, "Perfect square: 13*13 = 169 (sqrt 13) -> rate 28%"),
        (13, F(11, 10), True, "Perfect sq 13*13, MEN 1.1 (-10) -> rate 18%"),
        (13, F(12, 10), True, "Perfect sq 13*13, MEN 1.2 (-20) -> rate 8%"),
        (13, F(13, 10), True, "Perfect sq 13*13, MEN 1.3 (-30) -> rate -2 -> clamped to 1%"),
        (20, F(1), True, "Damage 20"),
        (35, F(1), True, "Damage 35"),
        (50, F(1), True, "Damage 50"),
        (52, F(1), True, "Perfect square: 13*52 = 676 (sqrt 26) -> rate 41%"),
        (52, F(11, 10), True, "Perfect sq 13*52, MEN 1.1 -> rate 31%"),
        (52, F(12, 10), True, "Perfect sq 13*52, MEN 1.2 -> rate 21%"),
        (52, F(13, 10), True, "Perfect sq 13*52, MEN 1.3 -> rate 11%"),
        (52, F(14, 10), True, "Perfect sq 13*52, MEN 1.4 -> rate 1%"),
        (52, F(15, 10), True, "Perfect sq 13*52, MEN 1.5 -> rate -9 -> clamped to 1%"),
        (80, F(1), True, "Damage 80"),
        (100, F(1), True, "Damage 100"),
        (117, F(1), True, "Perfect square: 13*117 = 1521 (sqrt 39) -> rate 54%"),
        (117, F(12, 10), True, "Perfect sq 13*117, MEN 1.2 -> rate 34%"),
        (150, F(1), True, "Damage 150"),
        (200, F(1), True, "Damage 200"),
        (208, F(1), True, "Perfect square: 13*208 = 2704 (sqrt 52) -> rate 67%"),
        (208, F(13, 10), True, "Perfect sq 13*208, MEN 1.3 -> rate 37%"),
        (300, F(1), True, "Damage 300"),
        (325, F(1), True, "Perfect square: 13*325 = 4225 (sqrt 65) -> rate 80%"),
        (400, F(1), True, "Damage 400"),
        (468, F(1), True, "Perfect square: 13*468 = 6084 (sqrt 78) -> rate 93%"),
        (500, F(1), True, "Damage 500: rate near 95%"),
        (550, F(1), True, "Damage 550: rate 99.5% -> clamped to 99%"),
        (600, F(1), True, "Damage 600: rate > 99% -> clamped to 99%"),
        (1000, F(1), True, "High damage 1000 -> clamped to 99%"),
        (5000, F(1), True, "Extreme damage 5000 -> clamped to 99%"),
    ]

    # Add systematic MEN variations
    for men in [10, 15, 20, 25, 30, 40, 50, 75]:
        if len(configs) < 50:
            men_f = F(1) + F(men, 100)
            configs.append((100, men_f, True, f"Damage 100, MEN deduction +{men}%"))

    out = []
    for dmg, men_b, is_cast, desc in configs:
        if not is_cast:
            rate = F(0)
            rounded_res = 0
            sqrt_val = F(0)
        else:
            lo_sq, hi_sq = root_bounds(F(13) * F(dmg))
            men_deduction = (men_b * 100) - 100
            lo_rate = F(15) + lo_sq - men_deduction
            hi_rate = F(15) + hi_sq - men_deduction
            lo_clamped = max(F(1), min(F(99), lo_rate))
            hi_clamped = max(F(1), min(F(99), hi_rate))
            assert floor(lo_clamped) == floor(hi_clamped), f"Ambiguous sqrt in {desc}"
            rate = lo_clamped
            rounded_res = floor(rate)
            sqrt_val = lo_sq

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "damage": dmg,
                "men_bonus": format_fraction(men_b),
                "is_casting": is_cast,
            },
            "intermediate": {
                "sqrt_13_dmg": format_fraction(sqrt_val),
            },
            "exact_rational": format_fraction(rate),
            "q_result": q_scale(rate),
            "rounded_result": rounded_res,
            "rounding_rule": "Rnd.get(100) < rate fractional comparison at Formulas.java:1022; rate clamped [1, 99]",
        })

    assert len(out) >= 40, f"Expected >= 40 vectors, got {len(out)}"
    return out


# ---------------------------------------------------------------------------
# Formula 7: calcAttributeBonus (attack from skill only, defence 0, weapon none)
# ---------------------------------------------------------------------------
def make_calc_attribute_bonus_vectors() -> list[dict]:
    source_lines = "Formulas.java:1440-1560"
    vectors = []

    # Early returns
    early_cases = [
        # (atk, def, wpn_elem, sk_elem, pvp, desc)
        (20, 0, -2, 3, False, "Weapon None (-2) with WIND skill -> returns 1.0"),
        (20, 0, None, 3, False, "Weapon None (null) with WIND skill -> returns 1.0"),
        (20, 0, 0, -1, False, "Skill element NONE (-1) -> returns 1.0"),
        (20, 0, 0, 1, False, "Weapon Fire (0) vs Skill Water (1) mismatch -> returns 1.0"),
        (0, 0, 0, 0, False, "Attack == Defence (0 == 0) -> returns 1.0"),
        (50, 50, 0, 0, False, "Attack == Defence (50 == 50) -> returns 1.0"),
        (20, 50, 0, 0, False, "Attack < Defence (20 < 50) -> returns 1.0"),
    ]
    for row in early_cases:
        vectors.append(row)

    # Core mathematical formula across brackets (attack element matches skill element, or weapon active)
    # Brackets:
    # atk >= 450: def>=450, def>=350, def<350
    # atk >= 300: def>=300, def>=150, def<150
    # atk >= 150: def>=150, def>=0, def<0
    # atk >= -99: def>=0, def<0
    # atk < -99: def>=450, def>=350, def<350
    bracket_cases = [
        # atk >= 450
        (450, 450, 0, 0, False, "Atk 450, Def 450 (equal early return)"),
        (460, 450, 0, 0, False, "Atk 460, Def 450: brackets (0.06909, 0.078)"),
        (450, 360, 0, 0, False, "Atk 450, Def 360: brackets (0.0887, 0.1007)"),
        (450, 200, 0, 0, False, "Atk 450, Def 200: brackets (0.129, 0.1473)"),
        (500, 50, 0, 0, False, "Atk 500, Def 50: diff 450 >= 300 (clamp [-50, 100])"),
        # atk >= 300
        (350, 300, 0, 0, False, "Atk 350, Def 300: brackets (0.0887, 0.1007)"),
        (350, 200, 0, 0, False, "Atk 350, Def 200: brackets (0.129, 0.1473)"),
        (350, 50, 0, 0, False, "Atk 350, Def 50: brackets (0.25, 0.2894), diff 300"),
        (300, 100, 0, 0, False, "Atk 300, Def 100: brackets (0.25, 0.2894), diff 200"),
        # atk >= 150
        (200, 150, 0, 0, False, "Atk 200, Def 150: brackets (0.129, 0.1473)"),
        (200, 50, 0, 0, False, "Atk 200, Def 50: brackets (0.25, 0.2894), diff 150"),
        (150, 20, 0, 0, False, "Atk 150, Def 20: brackets (0.25, 0.2894), diff 130"),
        (150, -20, 0, 0, False, "Atk 150, Def -20: brackets (0.4, 0.55), diff 170"),
        # atk >= -99
        (100, 0, 0, 0, False, "Atk 100, Def 0: brackets (0.25, 0.2894), diff 100"),
        (50, 0, 0, 0, False, "Atk 50, Def 0: brackets (0.25, 0.2894), diff 50"),
        (20, 0, 0, 0, False, "Atk 20, Def 0: skill 20 power with matching element"),
        (10, 0, 0, 0, False, "Atk 10, Def 0: skill 10 power"),
        (50, -50, 0, 0, False, "Atk 50, Def -50: brackets (0.4, 0.55), diff 100"),
        (0, -20, 0, 0, False, "Atk 0, Def -20: brackets (0.4, 0.55), diff 20"),
        (-50, -80, 0, 0, False, "Atk -50, Def -80: brackets (0.4, 0.55), diff 30"),
        # Difference clamp boundaries
        (450, 100, 0, 0, False, "Diff 350 >= 300: clamp [-50, 100]"),
        (300, 140, 0, 0, False, "Diff 160 >= 150: clamp [-50, 70]"),
        (200, 100, 0, 0, False, "Diff 100 >= -150: clamp [-50, 40]"),
        (20, 10, 0, 0, False, "Diff 10 >= -150: clamp [-50, 40]"),
        # PvP floor enforcement
        (50, 0, 0, 0, True, "PvP floor test (positive diff)"),
        (10, 0, 0, 0, True, "PvP floor test (small positive diff)"),
    ]
    vectors.extend(bracket_cases)

    # Grid expansion to reach >= 50
    for a in [30, 60, 120, 180, 240, 320, 400]:
        for d in [0, 50, 100]:
            if a > d and len(vectors) < 50:
                vectors.append((a, d, 0, 0, False, f"Grid Atk {a}, Def {d}"))

    out = []
    for atk, dfn, wpn_elem, sk_elem, pvp, desc in vectors:
        # Check early returns
        if wpn_elem is None or wpn_elem == -2:
            res = F(1)
        elif sk_elem == -1:
            res = F(1)
        elif wpn_elem != sk_elem:
            res = F(1)
        elif atk <= dfn:
            res = F(1)
        else:
            # Piecewise calculation
            if atk >= 450:
                if dfn >= 450:
                    a_mod, d_mod = F("0.06909"), F("0.078")
                elif dfn >= 350:
                    a_mod, d_mod = F("0.0887"), F("0.1007")
                else:
                    a_mod, d_mod = F("0.129"), F("0.1473")
            elif atk >= 300:
                if dfn >= 300:
                    a_mod, d_mod = F("0.0887"), F("0.1007")
                elif dfn >= 150:
                    a_mod, d_mod = F("0.129"), F("0.1473")
                else:
                    a_mod, d_mod = F("0.25"), F("0.2894")
            elif atk >= 150:
                if dfn >= 150:
                    a_mod, d_mod = F("0.129"), F("0.1473")
                elif dfn >= 0:
                    a_mod, d_mod = F("0.25"), F("0.2894")
                else:
                    a_mod, d_mod = F("0.4"), F("0.55")
            elif atk >= -99:
                if dfn >= 0:
                    a_mod, d_mod = F("0.25"), F("0.2894")
                else:
                    a_mod, d_mod = F("0.4"), F("0.55")
            else:
                if dfn >= 450:
                    a_mod, d_mod = F("0.06909"), F("0.078")
                elif dfn >= 350:
                    a_mod, d_mod = F("0.0887"), F("0.1007")
                else:
                    a_mod, d_mod = F("0.129"), F("0.1473")

            diff = atk - dfn
            if diff >= 300:
                min_v, max_v = F(-50), F(100)
            elif diff >= 150:
                min_v, max_v = F(-50), F(70)
            elif diff >= -150:
                min_v, max_v = F(-50), F(40)
            elif diff >= -300:
                min_v, max_v = F(-60), F(40)
            else:
                min_v, max_v = F(-80), F(40)

            a_term = (F(atk + 100) ** 2 / F(144)) * a_mod
            d_term = (F(dfn + 100) ** 2 / F(169)) * d_mod
            diff_mod = a_term - d_term
            clamped_mod = max(min_v, min(max_v, diff_mod))
            res = (clamped_mod / F(100)) + F(1)
            if pvp and res < F(1):
                res = F(1)

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "attack_attribute": atk,
                "defence_attribute": dfn,
                "weapon_element": wpn_elem,
                "skill_element": sk_elem,
                "is_pvp": pvp,
            },
            "exact_rational": format_fraction(res),
            "q_result": q_scale(res),
            "rounded_result": floor(res),
            "rounding_rule": "Exact piecewise calculation at Formulas.java:1440-1560; double precision retained",
        })

    assert len(out) >= 40, f"Expected >= 40 vectors, got {len(out)}"
    return out


# ---------------------------------------------------------------------------
# Formula 8: calcHpRegen / calcMpRegen / calcCpRegen
# ---------------------------------------------------------------------------
def make_calc_regen_vectors(bonuses: dict) -> dict[str, list[dict]]:
    source_lines = "Formulas.java:263-460"
    hp_vectors = []
    mp_vectors = []
    cp_vectors = []

    levels = [1, 5, 10, 20, 30, 40, 50, 60, 70, 75, 80, 85]
    postures = ["standing", "moving", "sitting"]

    # Base regen levels from Human Fighter
    base_hp = F("2.0")
    base_mp = F("0.9")
    base_cp = F("2.0")

    # HP regen cases
    for lvl in levels:
        for post in postures:
            for con in [20, 43, 80]:
                if len(hp_vectors) < 45:
                    hp_vectors.append((lvl, con, post, base_hp, True, f"Player HP regen lvl {lvl}, CON {con}, {post}"))
    # Add clamp test (low init < 1 clamped to 1)
    hp_vectors.append((1, 1, "moving", F("0.1"), True, "Player HP init < 1 clamped to 1 (moving)"))
    hp_vectors.append((1, 1, "standing", F("0.1"), True, "Player HP init < 1 clamped to 1 (standing)"))
    # Add NPC HP cases (no posture, no level mod, no stat bonus)
    hp_vectors.append((1, 0, "standing", F("2.0"), False, "Young Keltir NPC base HP regen 2.0"))
    hp_vectors.append((1, 0, "standing", F("3.5"), False, "NPC base HP regen 3.5"))
    hp_vectors.append((1, 0, "standing", F("0.5"), False, "NPC base HP regen 0.5 clamped to 1"))

    # MP regen cases
    for lvl in levels:
        for post in postures:
            for men in [20, 43, 80]:
                if len(mp_vectors) < 45:
                    mp_vectors.append((lvl, men, post, base_mp, True, f"Player MP regen lvl {lvl}, MEN {men}, {post}"))
    mp_vectors.append((1, 1, "moving", F("0.1"), True, "Player MP init < 1 clamped to 1 (moving)"))
    mp_vectors.append((1, 1, "standing", F("0.1"), True, "Player MP init < 1 clamped to 1 (standing)"))
    # NPC MP cases (0.9 clamps to 1.0!)
    mp_vectors.append((1, 0, "standing", F("0.9"), False, "Young Keltir NPC base MP regen 0.9 clamped to 1.0"))
    mp_vectors.append((1, 0, "standing", F("1.5"), False, "NPC base MP regen 1.5"))
    mp_vectors.append((1, 0, "standing", F("0.4"), False, "NPC base MP regen 0.4 clamped to 1.0"))

    # CP regen cases (players only)
    for lvl in levels:
        for post in postures:
            for con in [20, 43, 80]:
                if len(cp_vectors) < 45:
                    cp_vectors.append((lvl, con, post, base_cp, True, f"Player CP regen lvl {lvl}, CON {con}, {post}"))
    cp_vectors.append((1, 1, "moving", F("0.1"), True, "Player CP init < 1 clamped to 1 (moving)"))
    cp_vectors.append((1, 1, "standing", F("0.1"), True, "Player CP init < 1 clamped to 1 (standing)"))

    def build_regen_output(items, stat_name: str) -> list[dict]:
        res = []
        for lvl, stat_val, post, base_val, is_ply, desc in items:
            stat_b = bonuses[stat_name][stat_val] if is_ply else F(1)
            lvl_mod = F(lvl + 89, 100) if is_ply else F(1)

            if is_ply:
                init = base_val * lvl_mod * stat_b
                post_mul = F(15, 10) if post == "sitting" else (F(11, 10) if post == "standing" else F(7, 10))
            else:
                init = base_val
                post_mul = F(1)

            clamped_init = max(F(1), init)
            rate = clamped_init * post_mul

            # Multi-step accumulation: over 30 ticks (1 pulse), 60 ticks (2 pulses), 90 ticks (3 pulses)
            # In Nightfall, 1 pulse = 30 ticks = rate added to fractional accumulator
            res.append({
                "description": desc,
                "source_lines": source_lines,
                "inputs": {
                    "level": lvl,
                    "stat_value": stat_val,
                    "stat_name": stat_name.upper(),
                    "posture": post,
                    "base_regen": format_fraction(base_val),
                    "is_player": is_ply,
                },
                "intermediate": {
                    "level_mod": format_fraction(lvl_mod),
                    "stat_bonus": format_fraction(stat_b),
                    "init_before_clamp": format_fraction(init),
                    "init_after_clamp": format_fraction(clamped_init),
                    "posture_multiplier": format_fraction(post_mul),
                },
                "exact_rational": format_fraction(rate),
                "q_result": q_scale(rate),
                "rounded_result": floor(rate),
                "pulse_30_ticks_whole": floor(rate),
                "rounding_rule": "Math.max(1, init) floor clamp; Q fractions accumulated; whole integer emitted per 30-tick pulse",
            })
        return res

    return {
        "calcHpRegen": build_regen_output(hp_vectors, "con"),
        "calcMpRegen": build_regen_output(mp_vectors, "men"),
        "calcCpRegen": build_regen_output(cp_vectors, "con"),
    }


# ---------------------------------------------------------------------------
# Formula 9: calculateKarmaGain and calculateKarmaLost
# ---------------------------------------------------------------------------
def make_calculate_karma_vectors(multipliers: list[F]) -> tuple[list[dict], list[dict]]:
    gain_source_lines = "Formulas.java:1830-1848"
    lost_source_lines = "Formulas.java:1814-1821"

    # Karma Gain vectors
    # Boundaries: 0, 1, 98, 99, 179, 180, and intermediate points
    pk_counts = [
        0, 1, 2, 5, 10, 15, 20, 30, 40, 50, 60, 70, 80, 90, 95, 96, 97, 98,
        99, 100, 105, 110, 120, 130, 140, 150, 160, 170, 175, 176, 177, 178, 179,
        180, 181, 182, 190, 200, 250, 300, 500, 1000, 2000, 5000, 10000
    ]

    gain_out = []
    for pk in pk_counts:
        if pk < 99:
            # (pk * 0.5 + 1) * 720 = pk * 360 + 720
            exact = pk * 360 + 720
            branch = "pkCount < 99: ((pk * 0.5 + 1) * 60) * 12"
        elif pk < 180:
            # (pk * 0.125 + 37.75) * 720 = pk * 90 + 27180
            exact = pk * 90 + 27180
            branch = "99 <= pkCount < 180: ((pk * 0.125 + 37.75) * 60) * 12"
        else:
            exact = 43200
            branch = "pkCount >= 180: 43200 flat"

        gain_out.append({
            "description": f"PK count {pk} ({branch})",
            "source_lines": gain_source_lines,
            "inputs": {
                "pk_count": pk,
                "is_summon": False,
            },
            "exact_rational": str(exact),
            "rounded_result": exact,
            "rounding_rule": "Exact integer arithmetic from Formulas.java:1830-1848",
        })

    assert len(gain_out) >= 40, f"Expected >= 40 gain vectors, got {len(gain_out)}"

    # Karma Lost vectors
    # Levels 1..85 with exp variations
    lost_configs = []
    sample_levels = [1, 2, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 65, 70, 75, 76, 77, 78, 80, 85]
    for lvl in sample_levels:
        for exp in [100, 1000, 50000]:
            lost_configs.append((lvl, exp))
    # Extreme cases
    lost_configs.extend([
        (1, 1),
        (1, 10),
        (85, 1000000),
        (85, 10000000),
        (85, 2**31 - 1),
    ])

    lost_out = []
    for lvl, exp in lost_configs:
        mul = multipliers[lvl - 1]
        # (int) ((Math.abs(exp) / karmaLooseMul) / 30)
        exact = (F(abs(exp)) / mul) / F(30)
        rounded_res = floor(exact)

        lost_out.append({
            "description": f"Karma lost level {lvl}, exp {exp}",
            "source_lines": lost_source_lines,
            "inputs": {
                "player_level": lvl,
                "exp": exp,
                "karma_multiplier": format_fraction(mul),
            },
            "exact_rational": format_fraction(exact),
            "rounded_result": rounded_res,
            "rounding_rule": "(int) truncation at Formulas.java:1820",
        })

    assert len(lost_out) >= 40, f"Expected >= 40 lost vectors, got {len(lost_out)}"
    return gain_out, lost_out


# ---------------------------------------------------------------------------
# Formula 10: calculateSkillResurrectRestorePercent EXACT source quirk
# ---------------------------------------------------------------------------
def make_calculate_skill_resurrect_vectors() -> list[dict]:
    source_lines = "Formulas.java:1372-1386, L2PcInstance.java:4700"
    vectors = []

    # Bypasses: base == 0 or base == 100
    bypasses = [
        (0, F(1), "Base 0% bypass -> returns 0"),
        (0, F(15, 10), "Base 0% bypass with high WIT -> returns 0"),
        (0, F(2), "Base 0% bypass with 2.0 WIT -> returns 0"),
        (100, F(1), "Base 100% bypass -> returns 100"),
        (100, F(15, 10), "Base 100% bypass with high WIT -> returns 100"),
        (100, F(2), "Base 100% bypass with 2.0 WIT -> returns 100"),
    ]
    for b, w, d in bypasses:
        vectors.append((b, w, d))

    # Quirk tests:
    # r = base * wit
    # if (r - base) > 20.0: r += 20.0
    # r = max(r, base)
    # r = min(r, 90.0)
    quirk_cases = [
        # Exactly on the 20 boundary (r - base == 20: NO addition!)
        (20, F(2), "Base 20, WIT 2.0: r=40, diff=20 <= 20 -> NO +20 quirk! Returns 40%"),
        (10, F(3), "Base 10, WIT 3.0: r=30, diff=20 <= 20 -> NO +20 quirk! Returns 30%"),
        (40, F(15, 10), "Base 40, WIT 1.5: r=60, diff=20 <= 20 -> NO +20 quirk! Returns 60%"),
        # Just below the 20 boundary (diff < 20)
        (20, F(199, 100), "Base 20, WIT 1.99: diff=19.8 <= 20 -> NO quirk, returns 39.8%"),
        (30, F(15, 10), "Base 30, WIT 1.5: r=45, diff=15 <= 20 -> NO quirk, returns 45%"),
        (50, F(12, 10), "Base 50, WIT 1.2: r=60, diff=10 <= 20 -> NO quirk, returns 60%"),
        (60, F(11, 10), "Base 60, WIT 1.1: r=66, diff=6 <= 20 -> NO quirk, returns 66%"),
        (70, F(11, 10), "Base 70, WIT 1.1: r=77, diff=7 <= 20 -> NO quirk, returns 77%"),
        # Just above the 20 boundary (diff > 20: QUIRK TRIGGERS!)
        (20, F(201, 100), "Base 20, WIT 2.01: diff=20.2 > 20 -> QUIRK +20! r=40.2+20 = 60.2%"),
        (20, F(21, 10), "Base 20, WIT 2.1: diff=22 > 20 -> QUIRK +20! r=42+20 = 62%"),
        (30, F(17, 10), "Base 30, WIT 1.7: diff=21 > 20 -> QUIRK +20! r=51+20 = 71%"),
        (30, F(2), "Base 30, WIT 2.0: diff=30 > 20 -> QUIRK +20! r=60+20 = 80%"),
        (40, F(16, 10), "Base 40, WIT 1.6: diff=24 > 20 -> QUIRK +20! r=64+20 = 84%"),
        (40, F(17, 10), "Base 40, WIT 1.7: diff=28 > 20 -> QUIRK +20! r=68+20 = 88%"),
        # Hitting the 90% cap after quirk
        (40, F(18, 10), "Base 40, WIT 1.8: diff=32 > 20 -> r=72+20=92 -> CLAMPED to 90%"),
        (40, F(2), "Base 40, WIT 2.0: diff=40 > 20 -> r=80+20=100 -> CLAMPED to 90%"),
        (50, F(15, 10), "Base 50, WIT 1.5: diff=25 > 20 -> r=75+20=95 -> CLAMPED to 90%"),
        (50, F(2), "Base 50, WIT 2.0: diff=50 > 20 -> r=100+20=120 -> CLAMPED to 90%"),
        (60, F(15, 10), "Base 60, WIT 1.5: diff=30 > 20 -> r=90+20=110 -> CLAMPED to 90%"),
        (70, F(15, 10), "Base 70, WIT 1.5: diff=35 > 20 -> r=105+20=125 -> CLAMPED to 90%"),
    ]
    vectors.extend(quirk_cases)

    # Standard skill 1016 ranks (rank 1 = 0%? rank 2 = 20%, rank 3 = 30%, etc.)
    for base in [10, 20, 30, 40, 50, 60, 70, 80]:
        for wit in [F(1), F(11, 10), F(125, 100), F(14, 10)]:
            if len(vectors) < 50:
                vectors.append((base, wit, f"Skill 1016 base {base}%, WIT bonus {wit}"))

    out = []
    for base, wit, desc in vectors:
        base_f = F(base)
        if base_f == 0 or base_f == 100:
            res = base_f
            quirk_fired = False
        else:
            r = base_f * wit
            diff = r - base_f
            if diff > F(20):
                r += F(20)
                quirk_fired = True
            else:
                quirk_fired = False
            r = max(r, base_f)
            r = min(r, F(90))
            res = r

        # XP refund test with 10000 recoverable XP loss
        loss = 10000
        refund = rounded(F(loss) * res / F(100))

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "base_restore_percent": base,
                "wit_bonus": format_fraction(wit),
                "recoverable_loss_example": loss,
            },
            "intermediate": {
                "quirk_triggered": quirk_fired,
            },
            "exact_rational": format_fraction(res),
            "rounded_result": floor(res),
            "xp_refund_example": refund,
            "rounding_rule": "Exact source quirk (>20 branch quirk, max(r, base), min(r, 90.0)) at Formulas.java:1372; Math.round for XP refund",
        })

    assert len(out) >= 40, f"Expected >= 40 vectors, got {len(out)}"
    return out


# ---------------------------------------------------------------------------
# Formula 11: calcCastTime (magic vs physical vs static, 500 ms floor rule)
# ---------------------------------------------------------------------------
def make_calc_cast_time_vectors() -> list[dict]:
    source_lines = "Formulas.java:1027-1046"
    vectors = []

    configs = [
        # (hit_time, speed, is_magic, is_static, shots, desc)
        # Static skills (unscaled)
        (100, 333, True, True, False, "Static skill 100 ms (unscaled)"),
        (500, 333, True, True, False, "Static skill 500 ms (unscaled)"),
        (1000, 333, True, True, False, "Static skill 1000 ms (unscaled)"),
        (4000, 1500, True, True, False, "Static skill 4000 ms with high speed (unscaled)"),
        # hit_time <= 500 (500 ms floor does NOT apply!)
        (100, 333, True, False, False, "Base 100 ms <= 500: floor does not clamp (100 ms)"),
        (200, 333, True, False, False, "Base 200 ms <= 500: (200 ms)"),
        (300, 333, True, False, False, "Base 300 ms <= 500: (300 ms)"),
        (400, 333, True, False, False, "Base 400 ms <= 500: (400 ms)"),
        (500, 333, True, False, False, "Base 500 ms <= 500: (500 ms)"),
        (400, 666, True, False, False, "Base 400 ms with speed 666 -> 200 ms (floor doesn't clamp)"),
        (500, 1000, True, False, False, "Base 500 ms with speed 1000 -> 166.5 ms (floor doesn't clamp)"),
        # hit_time > 500 with fast speed (500 ms floor DOES apply!)
        (501, 1000, True, False, False, "hitTime 501 > 500 with speed 1000 -> clamped to 500.0 ms!"),
        (600, 1000, True, False, False, "hitTime 600 > 500 with speed 1000 -> clamped to 500.0 ms!"),
        (1000, 1000, True, False, False, "hitTime 1000 > 500 with speed 1000 -> 333 ms -> clamped to 500.0 ms!"),
        (1200, 1200, True, False, False, "hitTime 1200 > 500 with speed 1200 -> 333 ms -> clamped to 500.0 ms!"),
        (1500, 1500, True, False, False, "hitTime 1500 > 500 with speed 1500 -> 333 ms -> clamped to 500.0 ms!"),
        (2000, 1500, True, False, False, "hitTime 2000 > 500 with speed 1500 -> 444 ms -> clamped to 500.0 ms!"),
        # hit_time > 500 with slow speed (time >= 500, floor does not clamp)
        (600, 333, True, False, False, "hitTime 600, speed 333 -> 600 ms (unclamped)"),
        (1000, 333, True, False, False, "hitTime 1000, speed 333 -> 1000 ms (unclamped)"),
        (1500, 333, True, False, False, "hitTime 1500, speed 333 -> 1500 ms"),
        (4000, 333, True, False, False, "hitTime 4000, speed 333 -> 4000 ms"),
        (1000, 200, True, False, False, "Slow speed 200: (1000/200)*333 = 1665 ms"),
        (2000, 200, True, False, False, "Slow speed 200: (2000/200)*333 = 3330 ms"),
        # Physical skill speed scaling (pAtkSpd)
        (1000, 300, False, False, False, "Physical skill hitTime 1000, pAtkSpd 300 -> 1110 ms"),
        (1000, 500, False, False, False, "Physical skill hitTime 1000, pAtkSpd 500 -> 666 ms"),
        (1000, 1000, False, False, False, "Physical skill hitTime 1000, pAtkSpd 1000 -> clamped to 500 ms!"),
        # Magic skill with spiritshots (0.6x multiplier)
        (1000, 333, True, False, True, "Magic skill with spiritshot: 1000 * 0.6 = 600 ms"),
        (1500, 333, True, False, True, "Magic skill with spiritshot: 1500 * 0.6 = 900 ms"),
        (2000, 333, True, False, True, "Magic skill with spiritshot: 2000 * 0.6 = 1200 ms"),
        (1000, 500, True, False, True, "Magic skill with spiritshot: 666 * 0.6 = 399.6 -> clamped to 500 ms!"),
        # Tick conversion edge cases (100 ms per tick)
        (100, 333, True, False, False, "100 ms -> 1 tick"),
        (101, 333, True, False, False, "101 ms -> ceil to 2 ticks"),
        (199, 333, True, False, False, "199 ms -> ceil to 2 ticks"),
        (200, 333, True, False, False, "200 ms -> 2 ticks"),
        (499, 333, True, False, False, "499 ms -> ceil to 5 ticks"),
        (500, 333, True, False, False, "500 ms -> 5 ticks"),
        (501, 333, True, False, False, "501 ms -> ceil to 6 ticks"),
    ]

    # Expand to >= 50 vectors
    for ht in [800, 1200, 2500, 3000]:
        for spd in [250, 400, 600, 800]:
            if len(configs) < 50:
                configs.append((ht, spd, True, False, False, f"Grid ht={ht}, spd={spd}"))

    out = []
    for ht, spd, is_mag, is_stat, shots, desc in configs:
        time = F(ht)
        clamped_by_floor = False
        if not is_stat:
            time = (time / F(spd)) * F(333)
            if is_mag and shots:
                time *= F(6, 10)
            if time < F(500) and ht > 500:
                time = F(500)
                clamped_by_floor = True

        ms = floor(time)
        ticks = max(1, ceil(F(ms, 100)))

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "hit_time": ht,
                "speed": spd,
                "is_magic": is_mag,
                "is_static": is_stat,
                "charged_shots": shots,
            },
            "intermediate": {
                "clamped_by_500ms_floor": clamped_by_floor,
            },
            "exact_rational": format_fraction(time),
            "rounded_result": ms,
            "ticks_result": ticks,
            "rounding_rule": "(int) truncation of ms at L2Character.java:1508; ceil(ms / 100) for Nightfall ticks",
        })

    assert len(out) >= 40, f"Expected >= 40 vectors, got {len(out)}"
    return out


# ---------------------------------------------------------------------------
# Formula 12: doRevive vitals (HP to 65% if lower; MP/CP unchanged)
# ---------------------------------------------------------------------------
def make_do_revive_vectors() -> list[dict]:
    source_lines = "L2Character.java:2098-2124, character.properties:40-42"
    vectors = []

    # Various max_hp values, non-divisible by 100, edge cases
    configs = [
        # (max_hp, cur_hp, max_mp, cur_mp, max_cp, cur_cp, desc)
        (1, 0, 10, 5, 20, 10, "maxHp=1 at death (0 HP) -> floor(1 * 0.65) = 0"),
        (2, 0, 10, 5, 20, 10, "maxHp=2 at death -> floor(2 * 0.65) = 1"),
        (3, 0, 10, 5, 20, 10, "maxHp=3 at death -> floor(3 * 0.65) = 1"),
        (10, 0, 20, 10, 30, 15, "maxHp=10 at death -> floor(10 * 0.65) = 6"),
        (99, 0, 50, 25, 80, 40, "maxHp=99 at death -> floor(99 * 0.65) = 64"),
        (100, 0, 50, 25, 80, 40, "maxHp=100 at death -> floor(100 * 0.65) = 65"),
        (101, 0, 50, 25, 80, 40, "maxHp=101 at death -> floor(101 * 0.65) = 65"),
        (103, 0, 50, 25, 80, 40, "maxHp=103 at death -> floor(103 * 0.65) = 66"),
        (155, 0, 100, 50, 150, 75, "maxHp=155 at death -> floor(155 * 0.65) = 100"),
        (200, 0, 100, 50, 150, 75, "maxHp=200 at death -> floor(200 * 0.65) = 130"),
        (333, 0, 150, 75, 200, 100, "maxHp=333 at death -> floor(333 * 0.65) = 216"),
        (500, 0, 250, 125, 300, 150, "maxHp=500 at death -> floor(500 * 0.65) = 325"),
        (1000, 0, 500, 250, 600, 300, "maxHp=1000 at death -> floor(1000 * 0.65) = 650"),
        (1234, 0, 600, 300, 700, 350, "maxHp=1234 at death -> floor(1234 * 0.65) = 802"),
        (5000, 0, 2000, 1000, 3000, 1500, "maxHp=5000 at death -> floor(5000 * 0.65) = 3250"),
        (10000, 0, 4000, 2000, 5000, 2500, "maxHp=10000 at death -> floor(10000 * 0.65) = 6500"),
        (50000, 0, 10000, 5000, 20000, 10000, "maxHp=50000 at death -> floor(50000 * 0.65) = 32500"),
        # Current HP already >= 65% (unchanged)
        (100, 65, 50, 25, 80, 40, "curHp == 65% (65/100) -> remains 65"),
        (100, 66, 50, 25, 80, 40, "curHp > 65% (66/100) -> remains 66"),
        (100, 90, 50, 25, 80, 40, "curHp > 65% (90/100) -> remains 90"),
        (1000, 650, 500, 250, 600, 300, "curHp == 65% (650/1000) -> remains 650"),
        (1000, 700, 500, 250, 600, 300, "curHp > 65% (700/1000) -> remains 700"),
        (1000, 1000, 500, 250, 600, 300, "curHp full (1000/1000) -> remains 1000"),
        # Current HP < 65% but > 0 (e.g. non-death revive hook test)
        (100, 10, 50, 25, 80, 40, "curHp < 65% (10/100) -> raised to 65"),
        (100, 50, 50, 25, 80, 40, "curHp < 65% (50/100) -> raised to 65"),
        (100, 64, 50, 25, 80, 40, "curHp just below (64/100) -> raised to 65"),
        (1000, 649, 500, 250, 600, 300, "curHp just below (649/1000) -> raised to 650"),
    ]

    # Additional systematic HP levels
    for hp in [50, 75, 77, 88, 120, 125, 250, 300, 400, 450, 600, 800, 999, 1500, 2000, 2500, 3500, 7500, 15000, 20000, 25000, 30000, 40000]:
        if len(configs) < 50:
            configs.append((hp, 0, 100, 50, 100, 50, f"Standard revive maxHp={hp}"))

    out = []
    for max_hp, cur_hp, max_mp, cur_mp, max_cp, cur_cp, desc in configs:
        exact_restore_hp = F(max_hp) * F(65, 100)
        floor_restore_hp = floor(exact_restore_hp)
        new_hp = max(cur_hp, floor_restore_hp)
        new_mp = cur_mp
        new_cp = cur_cp

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "max_hp": max_hp,
                "current_hp": cur_hp,
                "max_mp": max_mp,
                "current_mp": cur_mp,
                "max_cp": max_cp,
                "current_cp": cur_cp,
            },
            "intermediate": {
                "exact_restore_hp_rational": format_fraction(exact_restore_hp),
                "floor_restore_hp": floor_restore_hp,
            },
            "exact_rational": format_fraction(exact_restore_hp),
            "rounded_result": {
                "hp": new_hp,
                "mp": new_mp,
                "cp": new_cp,
            },
            "rounding_rule": "HP raised to floor(max_hp * 0.65) if lower; MP and CP unchanged (RespawnRestoreMP/CP=0.00)",
        })

    assert len(out) >= 40, f"Expected >= 40 vectors, got {len(out)}"
    return out


# ---------------------------------------------------------------------------
# Formula 13: M.Atk / M.Def / MAtkSpd derivations
# ---------------------------------------------------------------------------
def make_m_stat_derivation_vectors(bonuses: dict) -> dict[str, list[dict]]:
    matk_source = "FuncMAtkMod.java:42-47"
    mdef_source = "FuncMDefMod.java:58-61"
    matkspd_source = "FuncMAtkSpeed.java:42-45, CharStat.java:252"

    matk_vectors = []
    mdef_vectors = []
    matkspd_vectors = []

    levels = [1, 5, 10, 20, 30, 40, 50, 60, 70, 75, 80, 85]

    # FuncMAtkMod: initVal * (INT.bonus)^2 * (levelMod)^2
    # Starter weapon mAtk = 6
    for lvl in levels:
        for int_stat in [10, 25, 43, 64]:
            if len(matk_vectors) < 45:
                int_b = bonuses["int"][int_stat]
                lvl_mod = F(lvl + 89, 100)
                exact = F(6) * (int_b**2) * (lvl_mod**2)
                matk_vectors.append({
                    "description": f"FuncMAtkMod initVal=6, level={lvl}, INT={int_stat}",
                    "source_lines": matk_source,
                    "inputs": {"init_val": 6, "level": lvl, "int_stat": int_stat},
                    "intermediate": {
                        "level_mod": format_fraction(lvl_mod),
                        "int_bonus": format_fraction(int_b),
                    },
                    "exact_rational": format_fraction(exact),
                    "q_result": q_scale(exact),
                    "rounded_result": floor(exact),
                    "rounding_rule": "Exact Q quantity at CharStat.java:207; floor for whole display",
                })

    # FuncMDefMod: value * MEN.bonus * levelMod
    # Unarmored base MDef = 41
    for lvl in levels:
        for men_stat in [10, 25, 43, 64]:
            if len(mdef_vectors) < 45:
                men_b = bonuses["men"][men_stat]
                lvl_mod = F(lvl + 89, 100)
                exact = F(41) * men_b * lvl_mod
                mdef_vectors.append({
                    "description": f"FuncMDefMod value=41, level={lvl}, MEN={men_stat}",
                    "source_lines": mdef_source,
                    "inputs": {"value": 41, "level": lvl, "men_stat": men_stat},
                    "intermediate": {
                        "level_mod": format_fraction(lvl_mod),
                        "men_bonus": format_fraction(men_b),
                    },
                    "exact_rational": format_fraction(exact),
                    "q_result": q_scale(exact),
                    "rounded_result": floor(exact),
                    "rounding_rule": "Exact Q quantity at CharStat.java:284; floor for whole display",
                })

    # FuncMAtkSpeed: 333 * WIT.bonus, capped at maxMAtkSpeed (2000)
    for wit_stat in range(1, 99, 2):
        if len(matkspd_vectors) < 45:
            wit_b = bonuses["wit"][wit_stat]
            exact = F(333) * wit_b
            clamped = min(F(2000), exact)
            matkspd_vectors.append({
                "description": f"FuncMAtkSpeed base=333, WIT={wit_stat}",
                "source_lines": matkspd_source,
                "inputs": {"init_val": 333, "wit_stat": wit_stat, "max_m_atk_speed": 2000},
                "intermediate": {
                    "wit_bonus": format_fraction(wit_b),
                    "raw_speed": format_fraction(exact),
                },
                "exact_rational": format_fraction(clamped),
                "q_result": q_scale(clamped),
                "rounded_result": floor(clamped),
                "rounding_rule": "(int) truncation after cap at CharStat.java:252",
            })

    return {
        "FuncMAtkMod": matk_vectors,
        "FuncMDefMod": mdef_vectors,
        "FuncMAtkSpeed": matkspd_vectors,
    }


def generate() -> tuple[dict, str]:
    stat_bonus_data = read_toml("tables/stat_bonus.toml")
    pvp_data = read_toml("tables/pvp.toml")
    regen_data = read_toml("tables/regen.toml")

    bonuses = {s: list(map(F, stat_bonus_data[s]["bonus"])) for s in ("str", "dex", "con", "int", "wit", "men")}
    karma_multipliers = list(map(F, pvp_data["karma_lost"]["multipliers"]))

    regen_dict = make_calc_regen_vectors(bonuses)
    karma_gain, karma_lost = make_calculate_karma_vectors(karma_multipliers)
    m_stat_dict = make_m_stat_derivation_vectors(bonuses)

    formulas = {
        "calcSkillPhysDam": make_calc_skill_phys_dam_vectors(),
        "calcMagicDam": make_calc_magic_dam_vectors(),
        "calcMagicSuccess": make_calc_magic_success_vectors(),
        "calcEffectSuccess": make_calc_effect_success_vectors(),
        "effect_duration": make_calc_effect_duration_vectors(),
        "calcAtkBreak": make_calc_atk_break_vectors(),
        "calcAttributeBonus": make_calc_attribute_bonus_vectors(),
        "calcHpRegen": regen_dict["calcHpRegen"],
        "calcMpRegen": regen_dict["calcMpRegen"],
        "calcCpRegen": regen_dict["calcCpRegen"],
        "calculateKarmaGain": karma_gain,
        "calculateKarmaLost": karma_lost,
        "calculateSkillResurrectRestorePercent": make_calculate_skill_resurrect_vectors(),
        "calcCastTime": make_calc_cast_time_vectors(),
        "doRevive": make_do_revive_vectors(),
        "FuncMAtkMod": m_stat_dict["FuncMAtkMod"],
        "FuncMDefMod": m_stat_dict["FuncMDefMod"],
        "FuncMAtkSpeed": m_stat_dict["FuncMAtkSpeed"],
    }

    # Verify counts
    for name, vecs in formulas.items():
        assert len(vecs) >= 40, f"Formula {name} has only {len(vecs)} vectors (< 40)"

    golden = {
        "schema": 1,
        "source_revisions": {
            "l2j_game": L2J_GAME_REV,
            "l2j_datapack": L2J_DP_REV,
        },
        "rounding_ledger": {
            "calcSkillPhysDam": "(int) truncation at PhysicalAttack.java:98",
            "calcMagicDam": "(int) truncation at MagicalAttack.java:88",
            "calcMagicSuccess": "Math.round((float) ...) with binary32 conversion at Formulas.java:1301",
            "calcEffectSuccess": "Double/Q comparison without whole-percent rounding at Formulas.java:1224; int cast for discrete rate",
            "effect_duration": "Math.ceil whole seconds at Formulas.java:1781, multiplied by 10 for ticks",
            "calcAtkBreak": "Fractional comparison Rnd.get(100) < rate at Formulas.java:1022; rate clamped [1, 99]",
            "calcAttributeBonus": "Piecewise rational calculation at Formulas.java:1440-1560; double precision retained",
            "calcHpRegen": "Math.max(1, init) floor clamp; Q fractions accumulated; whole integer emitted per 30-tick pulse",
            "calcMpRegen": "Math.max(1, init) floor clamp; Q fractions accumulated; whole integer emitted per 30-tick pulse",
            "calcCpRegen": "Math.max(1, init) floor clamp; Q fractions accumulated; whole integer emitted per 30-tick pulse",
            "calculateKarmaGain": "Exact integer arithmetic: pkCount*360+720 (pk<99), pkCount*90+27180 (99<=pk<180), 43200 (pk>=180) at Formulas.java:1830-1848",
            "calculateKarmaLost": "(int) truncation at Formulas.java:1820",
            "calculateSkillResurrectRestorePercent": "Exact source quirk (>20 branch quirk, max(r, base), min(r, 90.0)) at Formulas.java:1372; Math.round for XP refund at L2PcInstance.java:4700",
            "calcCastTime": "(int) truncation of ms at L2Character.java:1508; ceil(ms / 100) for Nightfall ticks; 500 ms floor rule if hit_time > 500 at Formulas.java:1027-1046",
            "doRevive": "HP raised to floor(max_hp * 0.65) if lower; MP and CP unchanged at L2Character.java:2098-2124 and character.properties:40-42",
            "FuncMAtkMod": "Exact Q initVal * (INT.bonus)^2 * (levelMod)^2 at FuncMAtkMod.java:42-47",
            "FuncMDefMod": "Exact Q value * MEN.bonus * levelMod at FuncMDefMod.java:58-61",
            "FuncMAtkSpeed": "(int) truncation after cap at CharStat.java:252; 333 * WIT.bonus at FuncMAtkSpeed.java:42-45",
        },
        "double_arithmetic_discrepancies": {
            "calcMagicSuccess_float_cast": "Formulas.java:1301 explicitly casts the double result to (float) before Math.round. In IEEE 754 binary32, precision is 24 bits. The oracle models this with binary_nearest(x, 24) to guarantee exact equivalence with the Java JVM.",
            "calcMagicDam_sqrt": "Formulas.java:845 uses Math.sqrt(mAtk). The oracle uses rational interval bounds [lo, hi] certified by integer square root (isqrt) to prove that the whole-number damage is invariant to floating-point vs exact square root.",
            "calcAtkBreak_sqrt": "Formulas.java:1010 uses Math.sqrt(13 * dmg). The oracle similarly certifies integer interval agreement for all vectors.",
            "calculateKarmaLost_division": "Formulas.java:1820 divides integer exp by double multiplier then 30. All 85 multipliers in pcKarmaIncrease.xml have finite decimal expansions with up to 9 fractional digits. Rational arithmetic exp / F(mul) / 30 truncated matches Java double division for all game exp values.",
            "resurrect_quirk_boundary": "Formulas.java:1377 uses strictly greater comparison (restorePercent - baseRestorePercent > 20.0). Floating point round-off near boundary could flip this condition; rational arithmetic guarantees exactness at the 20.0 boundary.",
        },
        "formulas": formulas,
    }

    # Input file hashes
    input_paths = sorted([
        ROOT / "tables/stat_bonus.toml",
        ROOT / "tables/pvp.toml",
        ROOT / "tables/regen.toml",
        ROOT / "tables/formulas.toml",
        ROOT / "npcs/keltir.toml",
        ROOT / "scripts/skill_oracle.py",
    ])
    golden["inputs_sha256"] = {
        str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in input_paths
    }

    text = json.dumps(golden, indent=2) + "\n"
    return golden, text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Verify fixtures/skill-oracle.json is up-to-date")
    args = parser.parse_args()

    golden, text = generate()
    out_path = ROOT / "fixtures/skill-oracle.json"

    if args.check:
        if not out_path.exists():
            raise SystemExit(f"Missing fixture file: {out_path}")
        existing = out_path.read_text(encoding="utf-8")
        if existing != text:
            raise SystemExit(f"Stale fixture output: {out_path}")
        print("skill_oracle: fixture is clean and up to date.")
    else:
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(text, encoding="utf-8")
        total_vectors = sum(len(v) for v in golden["formulas"].values())
        print(f"skill_oracle: generated {len(golden['formulas'])} formula sets with {total_vectors} total vectors into {out_path}")


if __name__ == "__main__":
    main()
