#!/usr/bin/env python3
"""Independent skill formula oracle for Nightfall Phase 3.

Python 3.11+, standard library only, no floating-point arithmetic in golden outputs.
Uses exact Fraction arithmetic and rational root intervals certified by isqrt.

Covers skill formula groups, including periodic adaptation and affect-limit semantics:
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
14. TickHp non-lethal clamp (currentHp - 1) & EffectTickRatio amount multiplier
15. cast staging times (continuous -300 ms, launch -400 ms, impact +400 ms, interrupt cutoff)
16. Heal (Heal.java:84-102 branches, weapon grade / no-shot multiplier, static bypass, overheal clamp)
17. FuncAtkEvasion ((int) intermediate cast before stat modifiers)
18. stun-removal roll condition (PcStatus.java:83-96, CharStatus.java:142-146 rule vector)

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


def ieee_normal(x: F, bits: int) -> F:
    """Integer-only IEEE nearest/ties-even adapter for bounded normal values.

    bits=24 models binary32; bits=53 models binary64. Sign and zero are exact.
    No struct, numpy, or host float arithmetic; all significand rounding uses
    integer quotient/remainder. Subnormals/overflow are outside these fixtures.
    """
    return F(0) if x == 0 else (binary_nearest(x, bits) if x > 0 else -binary_nearest(-x, bits))


def ticks_multiplier(ticks: int, ratio: int = 666) -> F:
    # AbstractEffect.java:149: int product converted to binary32, divided by
    # binary32 1000f, rounded in binary32, then widened exactly to binary64.
    return ieee_normal(ieee_normal(F(ticks * ratio), 24) / F(1000), 24)


def accumulation_trace(rate_q: int, maximum: int = 10**9, current: int = 0,
                       remainder_q: int = 0, pulses: int = 3) -> list[dict]:
    """D-R12: Q restoration credit, carry between pulses, clear at full clamp."""
    trace = []
    cumulative = 0
    for pulse in range(1, pulses + 1):
        whole, remainder_q = divmod(remainder_q + rate_q, Q)
        granted = min(whole, max(0, maximum - current))
        current += granted
        cumulative += granted
        if current >= maximum:
            remainder_q = 0
        trace.append({"pulse": pulse, "tick": pulse * 30, "whole_granted": granted,
                      "cumulative_whole": cumulative, "current": current, "remainder_q": remainder_q})
    return trace


def format_fraction(x: F) -> str:
    """Format fraction as exact string: integer or n/d."""
    if x.denominator == 1:
        return str(x.numerator)
    return f"{x.numerator}/{x.denominator}"


def calc_skill_phys_damage(power, p_atk, defence, proximity="front", soulshot=False,
                           crit=False, spread=F(1)) -> F:
    """Formulas.java:746-809; PhysicalAttack.java:86-104, no equipped shield."""
    if proximity not in ("front", "side", "behind") or F(defence) <= 0 or F(spread) <= 0:
        raise ValueError("unsupported physical damage inputs")
    proximity_bonus = {"front": F(1), "side": F(11, 10), "behind": F(6, 5)}[proximity]
    damage = F(77) * (F(power) + F(p_atk) * (2 if soulshot else 1)) / F(defence)
    return max(F(1), damage * proximity_bonus * F(spread)) * (2 if crit else 1)


def calc_magic_damage_bounds(power, m_atk, m_def, is_pvp, mcrit, magic_failure,
                             second_success, lvl_diff, attribute_bonus=F(1),
                             spread=F(1)) -> tuple[F, F]:
    """Formulas.java:845-898, MagicalAttack.java:88; certify whole damage."""
    if F(m_atk) < 0 or F(m_def) <= 0 or F(spread) <= 0:
        raise ValueError("unsupported magical damage inputs")
    def at_root(root):
        damage = F(91) * root * F(power) / F(m_def)
        if magic_failure:
            damage = damage / 2 if second_success and lvl_diff <= 9 else F(1)
        elif mcrit:
            damage *= F(5, 2) if is_pvp else 3
        return damage * F(attribute_bonus) * F(spread)
    lo, hi = root_bounds(F(m_atk))
    return at_root(lo), at_root(hi)


def calc_magic_success(lvl_difference, res_modifier=F(1), target_modifier=F(1)) -> int:
    """Formulas.java:1286-1301: float cast followed by Java Math.round."""
    raw = F(13, 10) ** lvl_difference * F(res_modifier) * F(target_modifier)
    if raw < 0:
        raise ValueError("unsupported negative magic resistance")
    return 100 - min(2**31 - 1, rounded(ieee_normal(raw, 24)))


def calc_effect_success_bounds(magic_level, target_level, activate_rate, lvl_bonus_rate,
                               target_base_stat, basic_property, is_magic, m_atk, m_def,
                               trait_mod, element_mod, buff_debuff_mod,
                               min_chance, max_chance) -> tuple[F, F]:
    """Formulas.java:1158-1224: continuous chance, clamp before roll comparison."""
    if activate_rate == -1 or basic_property == "NONE":
        return F(100), F(100)
    if F(trait_mod) <= 0:
        return F(0), F(0)
    level = target_level + 3 if magic_level <= -1 else magic_level
    coefficient = F((level - target_level + 3) * lvl_bonus_rate + activate_rate + 30 - target_base_stat)
    coefficient *= F(element_mod) * F(trait_mod) * F(buff_debuff_mod)
    roots = root_bounds(F(m_atk)) if is_magic else (F(1), F(1))
    if is_magic:
        coefficient *= F(11) / F(m_def)
    rates = sorted(coefficient * root for root in roots)
    return tuple(max(F(min_chance), min(F(max_chance), rate)) for rate in rates)


def calc_effect_duration(abnormal_time, is_debuff, magic_level, attacker_level,
                         target_level, lvl_bonus_rate, stat_bonus, trait_mod,
                         element_mod) -> F:
    """Formulas.java:1760-1784: return clamped seconds before Math.ceil."""
    time = F(abnormal_time)
    if not is_debuff:
        return time
    level = magic_level if magic_level > 0 else attacker_level
    raw = time * F(trait_mod) * (1 + F(lvl_bonus_rate, 100))
    raw *= (1 + F(level - target_level, 100)) * F(element_mod) / F(stat_bonus)
    return max(time / 2, min(time, raw))


def fold_stat(initial, modifiers, quantum=Q) -> F:
    """Calculator.java:110-127; Func{Set,Add,Sub,Mul,Div}.java:36-40.
    Nightfall D-R12 quantizes each runtime Func to Q with Java Math.round.
    None retains the exact expression for source/golden comparisons.
    """
    value = F(initial)
    for operation, operand in modifiers:
        operand = F(operand)
        if operation == "Set": value = operand
        elif operation == "Add": value += operand
        elif operation == "Sub": value -= operand
        elif operation in ("Mul", "Product"): value *= operand
        elif operation == "Div": value /= operand
        else: raise ValueError("unsupported independent modifier " + operation)
        if quantum is not None:
            value = F(rounded(value * quantum), quantum)
    return value


def calc_regen_initial(base, level_mod=F(1), stat_bonus=F(1), minimum=F(1)) -> F:
    """Formulas.java:263-351,357-442,442-450: clamp before stat Funcs."""
    return max(F(minimum), F(base) * F(level_mod) * F(stat_bonus))


def calc_regen_credit(initial, config_multiplier=F(1), posture_multiplier=F(1)) -> F:
    """Formulas.java:334-351,417-434,448-453: config/posture after Funcs."""
    return F(initial) * F(config_multiplier) * F(posture_multiplier)


def calc_stat_base(formula, value, level=1, stat_value=0, max_m_atk_speed=None) -> F:
    """Source FuncMAtkMod.java:42-47, FuncMDefMod.java:58-61,
    FuncMAtkSpeed.java:42-45 and FuncPAtkMod.java:43-44, at arbitrary input levels.
    Literal bonuses are independent oracle inputs, never producer Product values.
    """
    bonuses = read_toml("tables/stat_bonus.toml")
    level_mod = F(level + 89, 100)  # L2Character.java:5007-5008
    if formula == "FuncMAtkMod":
        return F(value) * F(bonuses["int"]["bonus"][stat_value])**2 * level_mod**2
    if formula == "FuncMDefMod":
        return F(value) * F(bonuses["men"]["bonus"][stat_value]) * level_mod
    if formula == "FuncMAtkSpeed":
        return min(F(max_m_atk_speed), F(value) * F(bonuses["wit"]["bonus"][stat_value]))
    if formula == "FuncPAtk":
        return F(value) * F(bonuses["str"]["bonus"][stat_value]) * level_mod
    if formula == "FuncPDef":  # FuncPDefMod.java:70
        return F(value) * level_mod
    raise ValueError("unsupported independent stat formula " + formula)


def calc_evasion_base(level, dex, is_player=True) -> int:
    """FuncAtkEvasion.java:37,61: intermediate (int) before later modifiers."""
    addition = F(level)
    if is_player and level >= 70:
        addition += F(level - 69) * (F(6, 5) if level >= 78 else 1)
    elif not is_player and level > 69:
        addition += level - 69 + 2
    lo, hi = root_bounds(F(dex))
    a, b = floor(lo * 6 + addition), floor(hi * 6 + addition)
    if a != b:
        raise ValueError("ambiguous independent evasion rounding")
    return a


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

        dmg = calc_skill_phys_damage(power, p_atk, p_def, prox, ss, crit)

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

        lo_d, hi_d = calc_magic_damage_bounds(power, m_atk, m_def, pvp, mcrit,
                                                fail, sec_succ, diff, attr)
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
        rate = calc_magic_success(diff, res_mod, tgt_mod)

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
        (0, 1, -1, 0, 20, "MEN", False, 0, 1, F(1), F(1), F(101, 100), 10, 90, "activateRate == -1 unconditional success"),
        (0, 1, 50, 1, 20, "NONE", False, 0, 1, F(1), F(1), F(101, 100), 10, 90, "basicProperty == NONE unconditional success"),
        (-1, 20, 50, 2, 25, "MEN", False, 0, 1, F(1), F(1), F(101, 100), 10, 90, "magicLevel <= -1 defaults to targetLvl + 3"),
        (0, 20, 50, 0, 25, "MEN", False, 0, 1, F(0), F(1), F(101, 100), 10, 90, "traitMod == 0 (trait immunity yields 0%)"),
        (0, 50, 10, 1, 99, "CON", False, 0, 1, F(1), F(1), F(101, 100), 10, 90, "Extreme high stat MEN=99 hitting minChance clamp"),
        (50, 1, 90, 5, 10, "INT", False, 0, 1, F(1), F(1), F(101, 100), 10, 90, "Over-leveled high rate hitting maxChance clamp"),
        (20, 20, 40, 2, 25, "MEN", False, 0, 1, F(1), F(1), F(1), 10, 90, "Physical debuff default DEBUFF_VULN 1.00"),
        (20, 20, 40, 2, 25, "MEN", False, 0, 1, F(1), F(1), F(1), 10, 90, "Physical debuff DEBUFF_VULN 1.00"),
        (20, 20, 40, 2, 25, "MEN", False, 0, 1, F(1), F(1), F(12, 10), 10, 90, "Physical debuff DEBUFF_VULN 1.20"),
        (20, 20, 40, 2, 25, "MEN", False, 0, 1, F(8, 10), F(1), F(101, 100), 10, 90, "Physical debuff trait resistance 0.8"),
        (20, 20, 40, 2, 25, "MEN", False, 0, 1, F(1), F(11, 10), F(101, 100), 10, 90, "Physical debuff element mod 1.1"),
        (20, 20, 40, 2, 25, "MEN", False, 0, 1, F(1), F(1), F(101, 100), 10, 90, "Pinned default clamp range [10, 90]"),
        # Magic debuffs: mAtkMod = (sqrt(mAtk) / mDef) * 11
        (20, 20, 40, 2, 25, "MEN", True, 100, 50, F(1), F(1), F(101, 100), 10, 90, "Magic debuff: mAtk=100 (sqrt=10), mDef=50"),
        (20, 20, 40, 2, 25, "MEN", True, 25, 50, F(1), F(1), F(101, 100), 10, 90, "Magic debuff: mAtk=25 (sqrt=5), mDef=50"),
        (20, 20, 40, 2, 25, "MEN", True, 400, 100, F(1), F(1), F(101, 100), 10, 90, "Magic debuff: mAtk=400 (sqrt=20), mDef=100"),
        (20, 20, 40, 2, 25, "MEN", True, 900, 150, F(1), F(1), F(101, 100), 10, 90, "Magic debuff: mAtk=900 (sqrt=30), mDef=150"),
    ]

    # Add level difference sweep
    for diff in [-10, -5, -2, 0, 2, 5, 10]:
        for act in [20, 50, 80]:
            if len(configs) < 50:
                configs.append((20 + diff, 20, act, 2, 25, "MEN", False, 0, 1, F(1), F(1), F(101, 100), 10, 90, f"Level diff {diff}, actRate {act}"))

    # Add stat variations
    for stat in [10, 20, 30, 40, 50, 60, 70, 80]:
        if len(configs) < 50:
            configs.append((25, 25, 50, 2, stat, "MEN", False, 0, 1, F(1), F(1), F(101, 100), 10, 90, f"Target stat MEN={stat}"))

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
            final_rate, _ = calc_effect_success_bounds(
                mlvl, tlvl, act, lbr, tstat, prop, is_mag, matk, mdef,
                trt, elem, buff, min_c, max_c)
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
        clamped = calc_effect_duration(t, is_deb, mlvl, alvl, tlvl, lbr, sb, trt, elem)
        sec = ceil(clamped)
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

            clamped_init = calc_regen_initial(base_val, lvl_mod, stat_b)
            rate = calc_regen_credit(clamped_init, posture_multiplier=post_mul)

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
                "pulse_trace": accumulation_trace(q_scale(rate)),
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
        (2, 6208133913),  # DATA review: rational/Java binary64 counterexample
    ])

    lost_out = []
    for lvl, exp in lost_configs:
        mul = multipliers[lvl - 1]
        # (int) ((Math.abs(exp) / karmaLooseMul) / 30)
        exact = (F(abs(exp)) / mul) / F(30)
        # Java: decimal literal -> double; long exp -> double; each division
        # rounds separately, then narrowing double->int truncates/saturates.
        java_mul = ieee_normal(mul, 53)
        java_first = ieee_normal(ieee_normal(F(abs(exp)), 53) / java_mul, 53)
        java_final = ieee_normal(java_first / F(30), 53)
        rounded_res = min(2**31 - 1, floor(java_final))

        lost_out.append({
            "description": f"Karma lost level {lvl}, exp {exp}",
            "source_lines": lost_source_lines,
            "inputs": {
                "player_level": lvl,
                "exp": exp,
                "karma_multiplier": format_fraction(mul),
            },
            "exact_rational": format_fraction(exact),
            "intermediate": {"binary64_multiplier": format_fraction(java_mul),
                             "binary64_first_division": format_fraction(java_first),
                             "binary64_final_division": format_fraction(java_final),
                             "rational_truncation": floor(exact)},
            "rounded_result": rounded_res,
            "rounding_rule": "binary64 conversion and two separately rounded divisions, then (int) truncation/saturation at Formulas.java:1820; RateKarmaLost=1",
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
                exact = calc_stat_base("FuncMAtkMod", 6, lvl, int_stat)
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
                exact = calc_stat_base("FuncMDefMod", 41, lvl, men_stat)
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

    # FuncMAtkSpeed: 333 * WIT.bonus, capped at pinned MaxMAtkSpeed (1999, character.properties:268)
    for wit_stat in range(1, 99, 2):
        if len(matkspd_vectors) < 45:
            wit_b = bonuses["wit"][wit_stat]
            exact = F(333) * wit_b
            clamped = calc_stat_base("FuncMAtkSpeed", 333, stat_value=wit_stat, max_m_atk_speed=1999)
            matkspd_vectors.append({
                "description": f"FuncMAtkSpeed base=333, WIT={wit_stat}",
                "source_lines": matkspd_source,
                "inputs": {"init_val": 333, "wit_stat": wit_stat, "max_m_atk_speed": 1999},
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


# ---------------------------------------------------------------------------
# Formula 14: TickHp non-lethal clamp & EffectTickRatio amount multiplier
# ---------------------------------------------------------------------------
def make_tick_hp_vectors() -> list[dict]:
    source_lines = "TickHp.java:62-76, AbstractEffect.java:148-149, character.properties:145"
    out = []

    configs = [
        # (power, ticks, mode, cur_hp, max_hp, desc)
        # Non-lethal clamp DIFF DoT cases (TickHp.java:75-76)
        (-100, 1, "DIFF", 50, 100, "Lethal DoT attempt clamped to curHp - 1 (50-1=49)"),
        (-100, 5, "DIFF", 50, 100, "High power DoT (333 dmg) clamped to curHp - 1 (49)"),
        (-500, 5, "DIFF", 200, 1000, "Massive DoT (1665 dmg) clamped to curHp - 1 (199)"),
        (-50, 1, "DIFF", 1, 100, "DoT at 1 HP clamped to 0 (cannot kill)"),
        (-50, 3, "DIFF", 1, 100, "DoT at 1 HP with ticks=3 clamped to 0"),
        (-50, 5, "DIFF", 1, 100, "DoT at 1 HP with ticks=5 clamped to 0"),
        (-50, 1, "DIFF", 2, 100, "DoT at 2 HP clamped to 1 HP remaining"),
        (-10, 1, "DIFF", 5, 20, "DoT at 5 HP with 6.66 raw dmg clamped to 4"),
        (-20, 1, "DIFF", 10, 50, "DoT at 10 HP with 13.32 raw dmg clamped to 9"),
        (-100, 1, "DIFF", 67, 100, "DoT with raw dmg 66.6 at 67 HP (67-1=66 < 66.6 -> clamped to 66)"),
        # Sub-lethal DIFF DoT cases (not clamped)
        (-10, 1, "DIFF", 100, 200, "Sub-lethal DoT: 10 * 0.666 = 6.66 dmg (hp 100 -> 93.34)"),
        (-25, 1, "DIFF", 500, 1000, "Sub-lethal DoT: 25 * 0.666 = 16.65 dmg"),
        (-50, 1, "DIFF", 500, 1000, "Sub-lethal DoT: 50 * 0.666 = 33.3 dmg"),
        (-25, 3, "DIFF", 500, 1000, "Sub-lethal DoT with ticks=3: 25 * 1.998 = 49.95 dmg"),
        (-50, 3, "DIFF", 500, 1000, "Sub-lethal DoT with ticks=3: 50 * 1.998 = 99.9 dmg"),
        (-25, 5, "DIFF", 500, 1000, "Sub-lethal DoT with ticks=5: 25 * 3.33 = 83.25 dmg"),
        (-50, 5, "DIFF", 500, 1000, "Sub-lethal DoT with ticks=5: 50 * 3.33 = 166.5 dmg"),
        # PER DoT cases (dynamic hp * power * ticksMultiplier)
        (F("-0.05"), 5, "PER", 1000, 1000, "PER DoT 5% at 1000 HP: 1000 * 0.05 * 3.33 = 166.5 dmg"),
        (F("-0.02"), 5, "PER", 1000, 1000, "PER DoT 2% at 1000 HP: 1000 * 0.02 * 3.33 = 66.6 dmg"),
        (F("-0.10"), 1, "PER", 1000, 1000, "PER DoT 10% at 1000 HP, ticks=1: 1000 * 0.1 * 0.666 = 66.6 dmg"),
        (F("-0.50"), 5, "PER", 10, 100, "PER DoT high power at 10 HP: raw 16.65 clamped to 9 (curHp-1)"),
        (F("-0.05"), 5, "PER", 1, 100, "PER DoT at 1 HP clamped to 0"),
        # HoT (positive power healing)
        (25, 1, "DIFF", 50, 100, "HoT 25 power, ticks=1: 16.65 heal"),
        (50, 3, "DIFF", 50, 200, "HoT 50 power, ticks=3: 99.9 heal"),
        (100, 5, "DIFF", 500, 1000, "HoT 100 power, ticks=5: 333 heal"),
        (100, 5, "DIFF", 900, 1000, "HoT 100 power clamped by maxHp (333 -> 100 heal)"),
        (100, 5, "DIFF", 1000, 1000, "HoT at maxHp: 0 heal"),
    ]

    for hp_val in [20, 40, 80, 150, 300, 600, 1200]:
        for p_val in [-15, -30, -60, -120]:
            if len(configs) < 55:
                configs.append((p_val, 1, "DIFF", hp_val, 2000, f"Grid DoT p={p_val}, hp={hp_val}"))

    for p, ticks, mode, cur_hp, max_hp, desc in configs:
        out.append(tick_hp_vector(p, ticks, mode, cur_hp, max_hp, desc))

    # CP absorption follows the HP-1 clamp, not a cap on total HP+CP.
    for power, hp, cp, player_source in [(-100, 50, 100, True), (-100, 50, 10, True),
                                        (-100, 50, 49, True), (-50, 1, 100, True),
                                        (-10, 100, 5, True), (-10, 100, 6, True),
                                        (-100, 50, 100, False), (-100, 2, 1, True)]:
        out.append(tick_hp_vector(power, 1, "DIFF", hp, 200, "CP absorption after non-lethal clamp",
                                  cp, player_source))

    assert len(out) >= 40
    return out


def tick_hp_vector(power, ticks: int, mode: str, cur_hp: int, max_hp: int,
                   desc: str, current_cp: int = 0, player_source: bool = False) -> dict:
    mult = ticks_multiplier(ticks)
    # _power is parsed as Java double. PER multiplies hp*power first, then the
    # widened binary32 multiplier (TickHp.java:62-71). DIFF freezes this amount.
    java_power = ieee_normal(F(power), 53)
    product = java_power if mode == "DIFF" else ieee_normal(F(cur_hp) * java_power, 53)
    raw = ieee_normal(product * mult, 53)
    cap = max(0, cur_hp - 1)
    nonlethal = raw < 0 and abs(raw) >= cap
    effective = min(abs(raw), F(cap)) if raw < 0 else min(raw, F(max(0, max_hp - cur_hp)))
    amount = floor(effective)
    cp_removed = min(current_cp, amount) if raw < 0 and player_source else 0
    hp_removed = amount - cp_removed if raw < 0 else 0
    final_hp = cur_hp - hp_removed if raw < 0 else cur_hp + amount
    remainder = 0 if raw < 0 or final_hp >= max_hp else q_scale(effective) % Q
    # Preserve Java fractional evidence separately from Nightfall's whole
    # damage/Q restoration adaptation; neither is mislabeled as the other.
    source_cp = min(F(current_cp), effective) if raw < 0 and player_source else F(0)
    source_hp_removed = ieee_normal(effective - source_cp, 53) if raw < 0 else F(0)
    source_hp = ieee_normal(F(cur_hp) - source_hp_removed, 53) if raw < 0 else (
        F(cur_hp) if cur_hp > max_hp else min(ieee_normal(F(cur_hp) + raw, 53), F(max_hp)))
    source_cp_after = ieee_normal(F(current_cp) - source_cp, 53)
    # A three-pulse trace proves frozen DIFF / dynamic PER and Q restoration.
    trace = []
    hp, cp, rem = cur_hp, current_cp, 0
    for pulse in range(1, 4):
        prod = java_power if mode == "DIFF" else ieee_normal(F(hp) * java_power, 53)
        pulse_raw = ieee_normal(prod * mult, 53)
        if pulse_raw < 0:
            dmg = floor(min(abs(pulse_raw), F(max(0, hp - 1))))
            cp_dmg = min(cp, dmg) if player_source else 0
            cp -= cp_dmg
            hp -= dmg - cp_dmg
            granted = 0
        else:
            credit_q = q_scale(min(pulse_raw, F(max(0, max_hp - hp))))
            grant, rem = divmod(rem + credit_q, Q)
            granted = min(grant, max(0, max_hp - hp))
            hp += granted
            if hp >= max_hp:
                rem = 0
            dmg, cp_dmg = 0, 0
        trace.append({"pulse": pulse, "ideal_offset_ms": pulse * ticks * 666,
                      "tick_offset": ceil(F(pulse * ticks * 666, 100)),
                      "hp_before": cur_hp if pulse == 1 else trace[-1]["final_hp"],
                      "raw_amount": format_fraction(pulse_raw), "whole_damage": dmg,
                      "cp_removed": cp_dmg, "whole_restored": granted,
                      "final_hp": hp, "final_cp": cp, "restoration_remainder_q": rem})
    return {
        "description": desc,
        "source_lines": "AbstractEffect.java:148-149; TickHp.java:62-89; PcStatus.java:185-194; architecture-decisions D-R12",
        "inputs": {"power": format_fraction(F(power)), "ticks": ticks, "mode": mode,
                   "current_hp": cur_hp, "max_hp": max_hp, "current_cp": current_cp,
                   "player_source": player_source, "target_is_player": True, "effect_tick_ratio": 666},
        "intermediate": {"ticks_multiplier": format_fraction(mult), "interval_ms": ticks * 666,
                         "binary64_power": format_fraction(java_power), "raw_amount": format_fraction(raw),
                         "non_lethal_cap": str(cap) if raw < 0 else None,
                         "clamped_by_non_lethal": nonlethal},
        "exact_rational": format_fraction(effective),
        "source_fractional_result": {"amount": format_fraction(effective),
                                     "cp_removed": format_fraction(source_cp),
                                     "hp_removed": format_fraction(source_hp_removed),
                                     "final_hp": format_fraction(source_hp),
                                     "final_cp": format_fraction(source_cp_after)},
        "rounded_result": {"amount": amount, "cp_removed": cp_removed, "hp_removed": hp_removed,
                           "final_hp": final_hp, "final_cp": current_cp - cp_removed,
                           "restoration_remainder_q": remainder},
        "pulse_trace": trace,
        "rounding_rule": "binary32 multiplier widened exactly; binary64 power/products; HP-1 cap before whole-damage conversion and CP-first subtraction; restoration accumulates Q, clears at full clamp (D-R12)",
    }


# ---------------------------------------------------------------------------
# Formula 15: Cast staging times
# ---------------------------------------------------------------------------
def make_cast_staging_vectors() -> list[dict]:
    source_lines = "L2Character.java:1508, 1625-1643, 4577-4581"
    out = []

    configs = [
        # (hit_time_ms, cast_speed, is_continuous, is_static, start_tick, desc)
        # Standard non-continuous
        (1080, 333, False, False, 100, "Standard 1080 ms cast at base speed: cutoff=8, launch=680 ms, impact=1080 ms"),
        (1500, 333, False, False, 100, "1500 ms cast: cutoff=13, launch=1100 ms, impact=1500 ms"),
        (2000, 333, False, False, 100, "2000 ms cast: cutoff=18, launch=1600 ms, impact=2000 ms"),
        (3000, 333, False, False, 100, "3000 ms cast: cutoff=28, launch=2600 ms, impact=3000 ms"),
        (4000, 333, False, False, 100, "4000 ms cast: cutoff=38, launch=3600 ms, impact=4000 ms"),
        # Continuous (-300 ms animation adjustment per L2Character.java:1627)
        (1080, 333, True, False, 100, "Continuous 1080 ms: cutoff=8 (uses original 1080 ms), launch=380 ms, impact=780 ms"),
        (1500, 333, True, False, 100, "Continuous 1500 ms: cutoff=13, launch=800 ms, impact=1200 ms"),
        (2000, 333, True, False, 100, "Continuous 2000 ms: cutoff=18, launch=1300 ms, impact=1700 ms"),
        (3000, 333, True, False, 100, "Continuous 3000 ms: cutoff=28, launch=2300 ms, impact=2700 ms"),
        (4000, 333, True, False, 100, "Continuous 4000 ms: cutoff=38, launch=3300 ms, impact=3700 ms"),
        # Short casts (design review F3/F4 citations)
        (500, 333, False, False, 100, "500 ms cast: cutoff=3, launch=100 ms, impact=500 ms"),
        (550, 333, False, False, 100, "550 ms cast: cutoff=3 (floor(5.5)-2), launch=150 ms, impact=550 ms"),
        (500, 333, True, False, 100, "Continuous 500 ms: cutoff=3, immediate launch, impact=400 ms"),
        (550, 333, True, False, 100, "Continuous 550 ms: cutoff=3, immediate launch, impact=400 ms"),
        # Long casts
        (6000, 333, False, False, 100, "Long 6000 ms cast: cutoff=58, launch=5600 ms, impact=6000 ms"),
        (8000, 333, False, False, 100, "Long 8000 ms cast: cutoff=78, launch=7600 ms, impact=8000 ms"),
        (6000, 333, True, False, 100, "Continuous 6000 ms: cutoff=58, launch=5300 ms, impact=5700 ms"),
        # Cast speed scaling
        (2000, 500, False, False, 100, "Speed 500: anim=1332 ms, cutoff=11, launch=932 ms, impact=1332 ms"),
        (2000, 666, False, False, 100, "Speed 666: anim=1000 ms, cutoff=8, launch=600 ms, impact=1000 ms"),
        (2000, 1000, False, False, 100, "Speed 1000: anim=666 ms, cutoff=4, launch=266 ms, impact=666 ms"),
        (2000, 1500, False, False, 100, "Speed 1500: anim=444 ms -> 500 ms floor, cutoff=3, launch=100 ms"),
        # Static casts
        (1000, 333, False, True, 100, "Static 1000 ms cast: anim unscaled=1000 ms, cutoff=8, launch=600 ms"),
        (1000, 333, True, True, 100, "Static continuous 1000 ms: cutoff=8, launch=300 ms, impact=700 ms"),
    ]

    # Task skillTime == trunc(original animation); short positive animations
    # with nonzero skillTime still schedule the 400 ms impact stage.
    for animation in [F(0), F(1, 2), F(1), F(50), F(100), F(1200)]:
        for continuous in [False, True]:
            configs.append((animation, 333, continuous, True, 100,
                            f"Short/boundary animation {format_fraction(animation)} ms, continuous={continuous}"))

    for ht in [1200, 1800, 2400, 3500]:
        for spd in [333, 450, 600]:
            for cont in [False, True]:
                if len(configs) < 55:
                    configs.append((ht, spd, cont, False, 200, f"Grid ht={ht}, spd={spd}, cont={cont}"))

    for ht, spd, cont, stat, st_tick, desc in configs:
        raw_anim = F(ht) if stat else (F(ht) / F(spd)) * F(333)
        animation = raw_anim
        if not stat and animation < 500 and ht > 500:
            animation = F(500)
        anim_ms = int(animation)

        # Interrupt cutoff based on original anim_ms per L2Character.java:1508
        cutoff_offset_ticks = (anim_ms // 100) - 2
        cutoff_tick = st_tick + max(0, cutoff_offset_ticks)

        # Continuous adjustment per L2Character.java:1627
        effective_anim_ms = int(animation - 300) if cont else anim_ms
        launch_offset_ms = max(0, effective_anim_ms - 400)
        launch_offset_ticks = ceil(F(launch_offset_ms, 100))
        launch_tick = st_tick + launch_offset_ticks

        impact_offset_ms = launch_offset_ms + (400 if anim_ms != 0 else 0)
        impact_offset_ticks = max(1, ceil(F(impact_offset_ms, 100)))
        impact_tick = st_tick + impact_offset_ticks

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "hit_time_ms": int(ht) if F(ht).denominator == 1 else format_fraction(F(ht)),
                "cast_speed": spd,
                "is_continuous": cont,
                "is_static": stat,
                "start_tick": st_tick,
            },
            "intermediate": {
                "anim_ms": anim_ms,
                "effective_anim_ms": effective_anim_ms,
                "launch_offset_ms": launch_offset_ms,
                "impact_offset_ms": impact_offset_ms,
            },
            "exact_rational": format_fraction(raw_anim),
            "rounded_result": {
                "interrupt_cutoff_offset_ticks": cutoff_offset_ticks,
                "interrupt_cutoff_tick": cutoff_tick,
                "interrupt_window_empty": cutoff_offset_ticks <= 0,
                "launch_offset_ticks": launch_offset_ticks,
                "launch_tick": launch_tick,
                "impact_offset_ticks": impact_offset_ticks,
                "impact_tick": impact_tick,
            },
            "rounding_rule": "Cutoff = floor(trunc(original animation)/100)-2 (nonpositive is empty); launch=max(0,trunc(animation-continuous*300)-400); impact=launch+(trunc(original animation)==0?0:400); ceil each offset once, complete at least start+1",
        })

    assert len(out) >= 40
    return out


# ---------------------------------------------------------------------------
# Formula 16: Heal.java branches (84-102)
# ---------------------------------------------------------------------------
def make_heal_vectors() -> list[dict]:
    source_lines = "Heal.java:84-102"
    out = []

    configs = [
        # (power, m_atk, grade, is_stat, is_mag, bss, heal_effect, is_crit, cur_hp, max_hp, desc)
        # Phase 3 standard: nongrade weapon, no shots (mAtkMul = 1 + 1 = 2)
        (100, 200, "NONE", False, True, False, F(1), False, 500, 1000, "Phase 3 standard: nongrade weapon, mAtkMul=2, power=100, mAtk=200"),
        (446, 200, "NONE", False, True, False, F(1), False, 500, 1000, "Battle Heal lvl 1: power=446, mAtk=200, mAtkMul=2"),
        (892, 500, "NONE", False, True, False, F(1), False, 500, 2000, "Major Heal: power=892, mAtk=500, mAtkMul=2"),
        # Unarmed / null weapon (mAtkMul = 1 + 1 = 2)
        (100, 50, "UNARMED", False, True, False, F(1), False, 500, 1000, "Unarmed heal: mAtkMul=2"),
        # S80 weapon (mAtkMul = 2 + 1 = 3)
        (446, 500, "S80", False, True, False, F(1), False, 500, 2000, "S80 weapon without shots: mAtkMul=3"),
        # S84 weapon (mAtkMul = 4 + 1 = 5)
        (446, 500, "S84", False, True, False, F(1), False, 500, 2000, "S84 weapon without shots: mAtkMul=5"),
        # Static heal (bypasses weapon, mAtk, heal_effect, crit)
        (500, 500, "NONE", True, True, False, F(1), False, 200, 1000, "Static heal: bypasses mAtk contribution and crit"),
        (500, 500, "NONE", True, True, False, F(12, 10), True, 200, 1000, "Static heal: ignores recipient heal_effect and crit"),
        # Magic crit (amount * 3 per CT2.3 Gracia Final)
        (100, 200, "NONE", False, True, False, F(1), True, 100, 2000, "Heal crit: amount * 3"),
        (446, 500, "NONE", False, True, False, F(1), True, 100, 4000, "Battle Heal crit: amount * 3"),
        # Recipient HEAL_EFFECT modifiers
        (100, 200, "NONE", False, True, False, F(12, 10), False, 500, 1000, "Recipient HEAL_EFFECT = 1.2 (+20%)"),
        (100, 200, "NONE", False, True, False, F(8, 10), False, 500, 1000, "Recipient HEAL_EFFECT = 0.8 (-20%)"),
        # Overheal clamping
        (500, 200, "NONE", False, True, False, F(1), False, 950, 1000, "Overheal clamp: 500 heal against 50 HP gap -> clamped to 50"),
        (500, 200, "NONE", False, True, False, F(1), False, 1000, 1000, "Target already at max HP -> clamped to 0"),
        (1000, 500, "NONE", False, True, False, F(1), True, 900, 1000, "Crit overheal clamp: 3000+ heal against 100 HP gap -> clamped to 100"),
    ]

    for p in [50, 150, 300, 600]:
        for ma in [50, 100, 250, 400]:
            for gr in ["NONE", "S80"]:
                if len(configs) < 55:
                    configs.append((p, ma, gr, False, True, False, F(1), False, 200, 2000, f"Grid p={p}, ma={ma}, gr={gr}"))

    for p, ma, gr, stat, mag, bss, he, crit, cur_hp, max_hp, desc in configs:
        if gr == "S84":
            gr_mult = 4
        elif gr == "S80":
            gr_mult = 2
        else:
            gr_mult = 1
        m_atk_mul = gr_mult * 4 if bss else gr_mult + 1

        base_amt = F(p)
        if not stat:
            lo_root, _ = root_bounds(F(m_atk_mul * ma))
            amt = (base_amt + lo_root) * he
            if mag and crit:
                amt *= 3
        else:
            amt = base_amt

        recoverable = max(F(0), F(max_hp) - F(cur_hp))
        clamped_amt = max(F(0), min(amt, recoverable))
        final_hp = F(cur_hp) + clamped_amt

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "power": p,
                "m_atk": ma,
                "weapon_grade": gr,
                "is_static": stat,
                "is_magic": mag,
                "bss": bss,
                "heal_effect": format_fraction(he),
                "is_crit": crit,
                "current_hp": cur_hp,
                "max_recoverable_hp": max_hp,
            },
            "intermediate": {
                "m_atk_multiplier": m_atk_mul,
                "pre_clamp_amount": format_fraction(amt),
                "recoverable_gap": format_fraction(recoverable),
            },
            "exact_rational": format_fraction(clamped_amt),
            "rounded_result": {
                "heal_amount": int(floor(clamped_amt)),
                "final_hp": int(floor(final_hp)),
            },
            "rounding_rule": "Heal.java:102 max(0, min(amount, maxRecoverableHp - currentHp)); (int) floor amount",
        })

    assert len(out) >= 40
    return out


# ---------------------------------------------------------------------------
# Formula 17: FuncAtkEvasion (int) intermediate cast
# ---------------------------------------------------------------------------
def make_func_atk_evasion_vectors() -> list[dict]:
    source_lines = "FuncAtkEvasion.java:37,61, CharStat.java:171-174"
    out = []

    configs = [
        # (level, dex, is_player, mod_op, mod_val, desc)
        # Level 1 DEX 20 player cases (F9 finding: raw = sqrt(20)*6 + 1 = 27.8328 -> (int) cast = 27 vs without cast round = 28)
        (1, 20, True, "none", 0, "Level 1 DEX 20 player base: (int) cast gives 27, without cast gives 28!"),
        (1, 20, True, "add", 4, "Level 1 DEX 20 player +4 mod: cast gives 31 (27+4), without cast gives 32 (round(31.83))!"),
        (1, 20, True, "add", 10, "Level 1 DEX 20 player +10 mod: cast gives 37, without cast gives 38!"),
        (1, 20, True, "mul", F(11, 10), "Level 1 DEX 20 player *1.1 buff: cast gives 30 (round(29.7)), without cast gives 31 (round(30.62))!"),
        (1, 20, True, "mul", F(9, 10), "Level 1 DEX 20 player *0.9 debuff: cast gives 24 (round(24.3)), without cast gives 25 (round(25.05))!"),
        # Other DEX values at level 1
        (1, 25, True, "none", 0, "Level 1 DEX 25 player: sqrt(25)*6+1 = 31.0 exact"),
        (1, 30, True, "none", 0, "Level 1 DEX 30 player: sqrt(30)*6+1 = 33.86 -> cast 33 vs round 34"),
        (1, 30, True, "add", 4, "Level 1 DEX 30 player +4 mod: cast 37 vs round 38"),
        (1, 35, True, "none", 0, "Level 1 DEX 35 player: sqrt(35)*6+1 = 36.49 -> cast 36 vs round 36 (agreement)"),
        (1, 40, True, "none", 0, "Level 1 DEX 40 player: sqrt(40)*6+1 = 38.94 -> cast 38 vs round 39"),
        # Levels 20, 40, 60, 69
        (20, 20, True, "none", 0, "Level 20 DEX 20 player: 46.83 -> cast 46 vs round 47"),
        (40, 20, True, "none", 0, "Level 40 DEX 20 player: 66.83 -> cast 66 vs round 67"),
        (60, 20, True, "none", 0, "Level 60 DEX 20 player: 86.83 -> cast 86 vs round 87"),
        (69, 20, True, "none", 0, "Level 69 DEX 20 player: 95.83 -> cast 95 vs round 96"),
        # Level >= 70 player diff branch (+(lvl - 69))
        (70, 20, True, "none", 0, "Level 70 DEX 20 player: diff=1 -> 97.83 -> cast 97 vs round 98"),
        (75, 20, True, "none", 0, "Level 75 DEX 20 player: diff=6 -> 107.83 -> cast 107 vs round 108"),
        (77, 20, True, "none", 0, "Level 77 DEX 20 player: diff=8 -> 111.83 -> cast 111 vs round 112"),
        # Level >= 78 player diff * 1.2 branch
        (78, 20, True, "none", 0, "Level 78 DEX 20 player: diff=9*1.2=10.8 -> 115.63 -> cast 115 vs round 116"),
        (80, 20, True, "none", 0, "Level 80 DEX 20 player: diff=11*1.2=13.2 -> 120.03 -> cast 120 vs round 120"),
        (85, 20, True, "none", 0, "Level 85 DEX 20 player: diff=16*1.2=19.2 -> 131.03 -> cast 131 vs round 131"),
        (85, 30, True, "add", 5, "Level 85 DEX 30 player +5: diff=19.2 -> cast vs round"),
        # Additional player levels to ensure rich coverage
        (45, 25, True, "none", 0, "Level 45 DEX 25 player: 76.0 exact"),
        (55, 30, True, "add", 3, "Level 55 DEX 30 player +3 mod"),
        (65, 35, True, "mul", F(11, 10), "Level 65 DEX 35 player *1.1 mod"),
        # Monster / NPC cases (level > 69 addition: level - 69 + 2)
        (1, 20, False, "none", 0, "NPC level 1 DEX 20: cast 27 vs round 28"),
        (20, 20, False, "none", 0, "NPC level 20 DEX 20: cast 46 vs round 47"),
        (70, 20, False, "none", 0, "NPC level 70 DEX 20: diff = (70-69)+2=3 -> cast 99 vs round 100"),
        (75, 20, False, "none", 0, "NPC level 75 DEX 20: diff = (75-69)+2=8 -> cast 109 vs round 110"),
        (80, 25, False, "none", 0, "NPC level 80 DEX 25: diff = (80-69)+2=13"),
        (85, 35, False, "none", 0, "NPC level 85 DEX 35: diff = (85-69)+2=18"),
    ]

    for lv in [10, 30, 50, 72, 82]:
        for dx in [22, 28, 33]:
            if len(configs) < 55:
                configs.append((lv, dx, True, "none", 0, f"Grid lvl={lv}, dex={dx}"))

    for lv, dx, is_p, m_op, m_val, desc in configs:
        lo_root, _ = root_bounds(F(dx))
        raw = lo_root * 6 + F(lv)
        if is_p:
            if lv >= 70:
                diff = F(lv - 69) * (F(12, 10) if lv >= 78 else F(1))
                raw += diff
        else:
            if lv > 69:
                raw += F(lv - 69 + 2)

        intermediate_cast = calc_evasion_base(lv, dx, is_p)

        if m_op == "add":
            val_with = fold_stat(intermediate_cast, [("Add", m_val)], quantum=None)
            val_without = raw + F(m_val)
        elif m_op == "mul":
            val_with = fold_stat(intermediate_cast, [("Mul", m_val)], quantum=None)
            val_without = raw * F(m_val)
        else:
            val_with = F(intermediate_cast)
            val_without = raw

        final_with = rounded(val_with)
        final_without = rounded(val_without)
        discrepancy = (final_with != final_without)

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "level": lv,
                "dex": dx,
                "is_player": is_p,
                "modifier": {
                    "op": m_op,
                    "val": format_fraction(F(m_val)),
                },
            },
            "intermediate": {
                "raw_evasion": format_fraction(raw),
                "intermediate_cast": intermediate_cast,
                "has_intermediate_discrepancy": discrepancy,
            },
            "exact_rational": format_fraction(val_with),
            "rounded_result": {
                "evasion_with_intermediate_cast": final_with,
                "evasion_without_intermediate_cast": final_without,
            },
            "rounding_rule": "FuncAtkEvasion.java:61 (int) cast at order 1; CharStat.java:172 Math.round final evasion",
        })

    assert len(out) >= 40
    return out


# ---------------------------------------------------------------------------
# Formula 18: Stun removal roll condition
# ---------------------------------------------------------------------------
def make_stun_removal_vectors() -> list[dict]:
    source_lines = "PcStatus.java:83-96, CharStatus.java:142-146"
    out = []

    configs = [
        # (cause, target_kind, is_stunned, roll_draw, desc)
        # Direct attack / physical / magic skill on stunned target with roll == 0 -> stun REMOVED
        ("DIRECT_ATTACK", "PLAYER", True, 0, "Direct attack on stunned player, roll=0 -> stun REMOVED"),
        ("PHYSICAL_SKILL", "PLAYER", True, 0, "Physical skill on stunned player, roll=0 -> stun REMOVED"),
        ("MAGICAL_SKILL", "PLAYER", True, 0, "Magical skill on stunned player, roll=0 -> stun REMOVED"),
        ("DIRECT_ATTACK", "NPC", True, 0, "Direct attack on stunned NPC, roll=0 -> stun REMOVED"),
        ("PHYSICAL_SKILL", "NPC", True, 0, "Physical skill on stunned NPC, roll=0 -> stun REMOVED"),
        # Direct attack on stunned target with rolls 1..9 -> stun RETAINED, roll consumed
        ("DIRECT_ATTACK", "PLAYER", True, 1, "Direct attack on stunned player, roll=1 -> stun retained"),
        ("DIRECT_ATTACK", "PLAYER", True, 2, "Direct attack on stunned player, roll=2 -> stun retained"),
        ("DIRECT_ATTACK", "PLAYER", True, 5, "Direct attack on stunned player, roll=5 -> stun retained"),
        ("DIRECT_ATTACK", "PLAYER", True, 9, "Direct attack on stunned player, roll=9 -> stun retained"),
        ("MAGICAL_SKILL", "NPC", True, 4, "Magical skill on stunned NPC, roll=4 -> stun retained"),
        # DOT damage on stunned target -> stun RETAINED, roll is NOT consumed (!isDOT guard)
        ("DOT", "PLAYER", True, 0, "DoT on stunned player: !isDOT guard suppresses roll, stun retained"),
        ("DOT", "PLAYER", True, 1, "DoT on stunned player: roll not consumed, stun retained"),
        ("DOT", "NPC", True, 0, "DoT on stunned NPC: !isDOT guard suppresses roll, stun retained"),
        ("DOT", "NPC", True, 9, "DoT on stunned NPC: roll not consumed, stun retained"),
        # HP_CONSUMPTION on stunned target -> stun RETAINED, roll NOT consumed (!isHPConsumption guard)
        ("HP_CONSUMPTION", "PLAYER", True, 0, "HP consumption on stunned player: !isHPConsumption suppresses roll, stun retained"),
        ("HP_CONSUMPTION", "NPC", True, 0, "HP consumption on stunned NPC: !isHPConsumption suppresses roll, stun retained"),
        # Target NOT stunned -> roll NOT consumed
        ("DIRECT_ATTACK", "PLAYER", False, 0, "Direct attack on non-stunned player: roll not consumed"),
        ("PHYSICAL_SKILL", "PLAYER", False, 0, "Physical skill on non-stunned player: roll not consumed"),
        ("MAGICAL_SKILL", "NPC", False, 0, "Magical skill on non-stunned NPC: roll not consumed"),
        ("DOT", "PLAYER", False, 0, "DoT on non-stunned player: roll not consumed"),
    ]

    for roll in range(10):
        configs.append(("PHYSICAL_SKILL", "PLAYER", True, roll, f"Physical skill on stunned player roll={roll}"))
        configs.append(("MAGICAL_SKILL", "PLAYER", True, roll, f"Magical skill on stunned player roll={roll}"))
        if roll < 5:
            configs.append(("DOT", "PLAYER", True, roll, f"DoT on stunned player roll={roll} (never removed)"))
            configs.append(("HP_CONSUMPTION", "PLAYER", True, roll, f"HP consume on stunned player roll={roll} (never removed)"))

    for cause, t_kind, stunned, roll, desc in configs:
        is_dot = (cause == "DOT")
        is_hp_con = (cause == "HP_CONSUMPTION")
        eligible = (not is_dot and not is_hp_con and stunned)
        roll_consumed = eligible
        stun_removed = (eligible and roll == 0)
        final_stunned = False if stun_removed else stunned

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "damage_cause": cause,
                "target_kind": t_kind,
                "target_is_stunned": stunned,
                "roll_draw": roll,
            },
            "intermediate": {
                "is_dot": is_dot,
                "is_hp_consumption": is_hp_con,
                "is_roll_eligible": eligible,
                "roll_consumed": roll_consumed,
            },
            "exact_rational": "1" if stun_removed else "0",
            "rounded_result": {
                "roll_consumed": roll_consumed,
                "stun_removed": stun_removed,
                "final_is_stunned": final_stunned,
            },
            "rounding_rule": "PcStatus.java:93-96 and CharStatus.java:142-146: !isDOT && !isHPConsumption && isStunned -> Rnd.get(10)==0 (10% chance)",
        })

    assert len(out) >= 40
    return out


# ---------------------------------------------------------------------------
# Formula 19: Skill.getAffectLimit and AffectScope limit evaluation
# ---------------------------------------------------------------------------
def make_get_affect_limit_vectors() -> list[dict]:
    source_lines = (
        "Skill.java:637-641, AffectScope.java:73-75, 143-145, 182-184, 233-235, 287-289, 357-359, 386, 401, 417"
    )
    out = []

    configs = [
        # (offset, width, roll_draw, desc)
        # Width 0 cases: no draw and NO limit (unlimited)
        (0, 0, None, "Uninitialized default 0-0: width 0 -> no draw, returns 0, AffectScope treats as UNLIMITED"),
        (10, 0, None, "Fixed limit candidate 10-0 (skill 981): width 0 -> no draw, returns 0, UNLIMITED"),
        (8, 0, None, "Fixed limit candidate 8-0 (skill 986): width 0 -> no draw, returns 0, UNLIMITED"),
        (9, 0, None, "Fixed limit candidate 9-0 (skill 1547): width 0 -> no draw, returns 0, UNLIMITED"),
        (5, 0, None, "Offset 5 width 0: width 0 -> no draw, returns 0, UNLIMITED"),
        (1, 0, None, "Offset 1 width 0: width 0 -> no draw, returns 0, UNLIMITED"),
        (50, 0, None, "Offset 50 width 0: width 0 -> no draw, returns 0, UNLIMITED"),
        (100, 0, None, "Offset 100 width 0: width 0 -> no draw, returns 0, UNLIMITED"),
    ]

    # Width > 0 cases from retail datapack:
    # 5-12 (skills 9, 24, 1174, 1181, 1295, 1296)
    for roll in [0, 1, 3, 5, 8, 11]:
        configs.append((5, 12, roll, f"Retail 5-12 with roll={roll} -> limit {5 + roll}"))
    # 10-10 (skills 18, 97, 103, 1034, 1049, 1096, 1099, 1101, 1208, 1209, 1272, 1399, 404)
    for roll in [0, 1, 4, 7, 9]:
        configs.append((10, 10, roll, f"Retail 10-10 with roll={roll} -> limit {10 + roll}"))
    # 9-10 (skills 286, 302, 1072, 1167)
    for roll in [0, 1, 3, 6, 9]:
        configs.append((9, 10, roll, f"Retail 9-10 with roll={roll} -> limit {9 + roll}"))
    # 6-12 (skills 36, 1171)
    for roll in [0, 1, 4, 7, 11]:
        configs.append((6, 12, roll, f"Retail 6-12 with roll={roll} -> limit {6 + roll}"))
    # 50-50 (skills 1003, 1005, 1254)
    for roll in [0, 1, 10, 25, 49]:
        configs.append((50, 50, roll, f"Retail 50-50 with roll={roll} -> limit {50 + roll}"))
    # Other retail XML patterns:
    # 1-10
    for roll in [0, 2, 5, 9]:
        configs.append((1, 10, roll, f"Retail 1-10 with roll={roll} -> limit {1 + roll}"))
    # 3-6
    for roll in [0, 1, 3, 5]:
        configs.append((3, 6, roll, f"Retail 3-6 with roll={roll} -> limit {3 + roll}"))
    # 5-5
    for roll in [0, 1, 2, 4]:
        configs.append((5, 5, roll, f"Retail 5-5 with roll={roll} -> limit {5 + roll}"))
    # 5-15
    for roll in [0, 3, 7, 14]:
        configs.append((5, 15, roll, f"Retail 5-15 with roll={roll} -> limit {5 + roll}"))
    # 15-25
    for roll in [0, 5, 12, 24]:
        configs.append((15, 25, roll, f"Retail 15-25 with roll={roll} -> limit {15 + roll}"))
    # 50-100
    for roll in [0, 10, 49, 99]:
        configs.append((50, 100, roll, f"Retail 50-100 with roll={roll} -> limit {50 + roll}"))

    for offset, width, roll, desc in configs:
        width_is_zero = (width == 0)
        draw_performed = (width > 0)
        raw_limit = 0 if width_is_zero else (offset + roll)
        is_unlimited = (raw_limit <= 0)
        stream_limit = 2147483647 if is_unlimited else raw_limit

        out.append({
            "description": desc,
            "source_lines": source_lines,
            "inputs": {
                "offset": offset,
                "width": width,
                "roll_draw": roll if draw_performed else None,
            },
            "intermediate": {
                "width_is_zero": width_is_zero,
                "draw_performed": draw_performed,
                "raw_limit": raw_limit,
                "is_unlimited": is_unlimited,
            },
            "exact_rational": "0" if is_unlimited else str(raw_limit),
            "rounded_result": {
                "limit_return": raw_limit,
                "is_unlimited": is_unlimited,
                "stream_limit": stream_limit,
            },
            "rounding_rule": "Skill.java:638 if (affectLimit[1] == 0) return 0; else offset + Rnd.get(width); AffectScope.java limit 0 is UNLIMITED",
        })

    assert len(out) >= 40, f"make_get_affect_limit_vectors has only {len(out)} vectors (< 40)"
    return out


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
        "TickHp": make_tick_hp_vectors(),
        "cast_staging": make_cast_staging_vectors(),
        "Heal": make_heal_vectors(),
        "FuncAtkEvasion": make_func_atk_evasion_vectors(),
        "stun_removal_condition": make_stun_removal_vectors(),
        "getAffectLimit": make_get_affect_limit_vectors(),
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
            "TickHp": "Binary32 amount multiplier at AbstractEffect.java:149; DIFF/PER binary64 products at TickHp.java:62-71; HP-1 cap at75-76 before whole damage/CP absorption (PcStatus.java:185-194); Q restoration remainder per D-R12. See pulse traces.",
            "cast_staging": "Cast timing staging at L2Character.java:1508, 1625-1643, 4577-4581: base animation time, continuous-skill -300 ms, launch=max(0,trunc(adjusted animation)-400), impact=launch+(trunc(original animation)==0?0:400), interrupt cutoff deadline floor(trunc(animation_ms)/100) - 2 ticks",
            "Heal": "Heal.java:84-102: weapon grade multiplier (S84=4, S80=2, other=1), no-shot dynamic bonus (+1), sqrt(mAtkMul * mAtk) added unless static, recipient HEAL_EFFECT, x3 magic crit, and [0, maxRecoverableHp - currentHp] overheal clamp",
            "FuncAtkEvasion": "FuncAtkEvasion.java:37,61: order 1 built-in func truncates (int) value before later stat modifiers; CharStat.java:171-174 rounds the final completed stat",
            "stun_removal_condition": "PcStatus.java:83-96 and CharStatus.java:142-146: stun removal roll condition (!isDOT && !isHPConsumption && isStunned) triggering Rnd.get(10) == 0 (10% chance) on eligible direct damage",
            "getAffectLimit": "Skill.java:637-641: width 0 (affectLimit[1] == 0) returns 0 with no draw; width > 0 returns offset + Rnd.get(width); AffectScope.java:73,143,182,233,287,357,386,401,417 treats limit <= 0 as UNLIMITED (Integer.MAX_VALUE)",
        },
        "double_arithmetic_discrepancies": {
            "calcMagicSuccess_float_cast": "Formulas.java:1301 explicitly casts the double result to (float) before Math.round. In IEEE 754 binary32, precision is 24 bits. The oracle models this with binary_nearest(x, 24) to guarantee exact equivalence with the Java JVM.",
            "calcMagicDam_sqrt": "Formulas.java:845 uses Math.sqrt(mAtk). The oracle uses rational interval bounds [lo, hi] certified by integer square root (isqrt) to prove that the whole-number damage is invariant to floating-point vs exact square root.",
            "calcAtkBreak_sqrt": "Formulas.java:1010 uses Math.sqrt(13 * dmg). The oracle similarly certifies integer interval agreement for all vectors.",
            "calculateKarmaLost_division": "Formulas.java:1820 rounds the multiplier conversion and both divisions in binary64 before int truncation. Exact rational truncation is NOT universally equivalent: level2, XP6208133913, multiplier2.069377971 gives rational100000000 vs Java99999999. All supplied vectors model the binary64 sites with integer-only nearest/ties-even significand rounding; RateKarmaLost=1.",
            "resurrect_quirk_boundary": "Formulas.java:1377 uses strictly greater comparison (restorePercent - baseRestorePercent > 20.0). Floating point round-off near boundary could flip this condition; rational arithmetic guarantees exactness at the 20.0 boundary.",
            "FuncAtkEvasion_intermediate_cast": "FuncAtkEvasion.java:61 truncates intermediate evasion to (int) before modifiers. Without this intermediate truncation, rounding after modifiers produces different integer values (e.g. lvl 1 DEX 20 player yields 27 with cast vs 28 without; adding +4 modifier yields 31 vs 32; buff 1.1x yields 30 vs 31).",
            "Heal_nongrade_shot_multiplier": "In Phase 3 with no spiritshots, nongrade weapon or unarmed yields mAtkMul = 1 + 1 = 2 per Heal.java:89. Static healing bypasses mAtk, heal_effect and crit.",
            "TickHp_binary32_multiplier": "AbstractEffect.java:149 converts integer ticks*666 to binary32, divides by 1000f in binary32, then widens to double. ticks5 multiplier=1745879/524288, not333/100. Integer-only binary_nearest(...,24) emulates nearest/ties-even without struct/numpy/host floats; subsequent power parsing and products use binary_nearest(...,53). HoT100 yields332 whole plus999992 Q remainder; DoT truncates before whole HP subtraction (D-R12).",
            "TickHp_nonlethal_clamp": "TickHp.java:75-76 caps negative DoT power at currentHp - 1, preventing death from DoT. Clamped power is non-negative and zero if currentHp <= 1.",
            "getAffectLimit_unlimited_zero": "Skill.java:638 returns 0 when width==0. AffectScope treats 0 as unlimited targets (affectLimit > 0 guard and .limit(affectLimit > 0 ? affectLimit : Integer.MAX_VALUE)). Fixed-limit skills require width > 0 in retail datapack.",
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


def generate_class_phase3() -> tuple[dict, str]:
    """Independent rational vectors driven by the new immutable template inputs.

    Separate artifact: the existing formula vectors remain byte-for-byte identical.
    No production/engine or generator formula functions are used for expectations.
    """
    keys = ("human_fighter", "human_mystic", "elven_mystic", "dark_fighter",
            "orc_mystic", "dwarven_fighter")
    levels = (1, 20, 40, 85)
    bonuses = {s: list(map(F, read_toml("tables/stat_bonus.toml")[s]["bonus"]))
               for s in ("int", "wit", "men", "con")}
    cap = read_toml("tables/formulas.toml")["constants"]["MaxMAtkSpeed"]["value"]
    config = read_toml("tables/regen.toml")
    formulas = {k: [] for k in ("FuncMAtkMod", "FuncMDefMod", "FuncMAtkSpeed",
                                "calcHpRegen", "calcMpRegen", "calcCpRegen")}
    paths = [ROOT / f"classes-phase3/{key}.toml" for key in keys]

    for key in keys:
        profile = read_toml(f"classes-phase3/{key}.toml")
        stats, magic = profile["base_stats"], profile["magical"]
        for level in levels:
            level_mod = F(level + 89, 100)  # L2Character.java:5007-5008
            common = {"class_id": profile["id"], "class_key": key, "level": level,
                      "template_file": f"classes-phase3/{key}.toml",
                      "is_player": True, "equipment": "unarmed_empty_jewelry",
                      "modifiers": [], "rolls": []}
            for name, base_key, stat, exponent, use_level, lines in (
                ("FuncMAtkMod", "base_m_atk", "int", 2, True, "42-46"),
                ("FuncMDefMod", "base_m_def", "men", 1, True, "44-66"),
                ("FuncMAtkSpeed", "base_m_atk_speed", "wit", 1, False, "42-43"),
            ):
                base = magic[base_key]
                bonus = bonuses[stat][stats[stat]]
                exact = F(base) * bonus**exponent * (level_mod**exponent if use_level else 1)
                result = min(F(cap), exact) if name == "FuncMAtkSpeed" else exact
                formulas[name].append({
                    "description": f"{profile['l2_ref']} level {level}: {name} from class template",
                    "source_lines": f"model/stats/functions/formulas/{name}.java:{lines}",
                    "template_source": profile["source"],
                    "inputs": {**common, "base": base, "stat_name": stat.upper(),
                               "stat_value": stats[stat], **({"max_m_atk_speed": cap} if name == "FuncMAtkSpeed" else {})},
                    "intermediate": {"stat_bonus": format_fraction(bonus),
                                     "level_mod": format_fraction(level_mod),
                                     "before_cap": format_fraction(exact),
                                     "empty_jewelry_m_def_slots": magic["m_def_slots"] if name == "FuncMDefMod" else {}},
                    "exact_rational": format_fraction(result), "q_result": q_scale(result),
                    "rounded_result": floor(result),
                    "rounding_rule": "MAtk/MDef retain Q (floor only for display); MAtkSpd truncates after cap (CharStat.java:237-262,284-293)",
                })
            row = profile["regen"]["levels"][level - 1]
            assert row["level"] == level
            for name, resource, stat, lines in (("calcHpRegen", "hp", "con", "263-349"),
                                                ("calcMpRegen", "mp", "men", "357-434"),
                                                ("calcCpRegen", "cp", "con", "442-453")):
                literal = row[f"{resource}_regen"]
                bonus = bonuses[stat][stats[stat]]
                init = F(literal) * level_mod * bonus
                clamped = max(F(1), init)
                for posture, factor in (("standing", F(11, 10)), ("running", F(7, 10)),
                                         ("walking", F(1))):
                    config_mul = F(config["character_multipliers"][f"{resource}_regen_multiplier"])
                    exact = clamped * config_mul * factor
                    formulas[name].append({
                        "description": f"{profile['l2_ref']} level {level}: {resource.upper()} {posture}",
                        "source_lines": f"model/stats/Formulas.java:{lines}",
                        "template_source": profile["regen"]["source"],
                        "inputs": {**common, "base_regen": literal, "stat_name": stat.upper(),
                                   "stat_value": stats[stat], "posture": posture,
                                   "character_multiplier": format_fraction(config_mul),
                                   "regen_stat_modifiers": [], "zone_bonus": 0},
                        "intermediate": {"level_mod": format_fraction(level_mod),
                                         "stat_bonus": format_fraction(bonus),
                                         "init_before_clamp": format_fraction(init),
                                         "init_after_clamp": format_fraction(clamped),
                                         "posture_multiplier": format_fraction(factor)},
                        "exact_rational": format_fraction(exact), "q_result": q_scale(exact),
                        "rounded_result": floor(exact), "pulse_trace": accumulation_trace(q_scale(exact)),
                        "rounding_rule": "Clamp init before stat funcs; exact literals to Q once after formula; retain Q remainder each 30-tick pulse (D-R12)",
                    })

    npc = read_toml("npcs/keltir.toml")["phase3"]
    for name, resource, lines in (("calcHpRegen", "hp", "263-349"),
                                 ("calcMpRegen", "mp", "357-434")):
        base = F(npc["template"][f"{resource}_regen"])
        rate = max(F(1), base)
        formulas[name].append({
            "description": f"Young Keltir 20545: scalar {resource.upper()} regen",
            "source_lines": f"model/stats/Formulas.java:{lines}", "template_source": npc["source"],
            "inputs": {"class_key": "keltir", "source_npc_id": 20545, "level": 1,
                       "is_player": False, "base_regen": format_fraction(base),
                       "xml_literal": npc["xml"][f"{resource}_regen"], "rolls": [],
                       "regen_stat_modifiers": [], "raid": False, "champion": False},
            "intermediate": {"level_mod": "1", "stat_bonus": "1", "posture_multiplier": "1",
                             "init_before_clamp": format_fraction(base), "init_after_clamp": format_fraction(rate)},
            "exact_rational": format_fraction(rate), "q_result": q_scale(rate),
            "rounded_result": floor(rate), "pulse_trace": accumulation_trace(q_scale(rate)),
            "rounding_rule": "Inherited binary32 scalar regen getter; clamp >=1 before stat funcs; no player multipliers; no NPC CP",
        })
    paths += [ROOT / rel for rel in ("npcs/keltir.toml", "tables/stat_bonus.toml", "tables/regen.toml",
                                    "tables/formulas.toml", "scripts/skill_oracle.py")]
    golden = {
        "schema": 1, "source_revisions": {"l2j_game": L2J_GAME_REV, "l2j_datapack": L2J_DP_REV},
        "policy": "Exact decimal/Fraction formula evidence with Q restoration adaptation; not a Java binary64 equivalence claim.",
        "coverage": {"class_keys": list(keys), "levels": list(levels),
                     "regen_postures": ["standing", "running", "walking"], "npc": "Young Keltir 20545"},
        "inputs_sha256": {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(paths)},
        "formulas": formulas,
        "vector_set_sha256": hashlib.sha256(json.dumps(formulas, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
    }
    return golden, json.dumps(golden, indent=2) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Verify fixtures/skill-oracle.json is up-to-date")
    parser.add_argument("--class-phase3", action="store_true", help="Generate/check the separate class-template oracle fixture")
    args = parser.parse_args()

    golden, text = generate_class_phase3() if args.class_phase3 else generate()
    out_path = ROOT / ("fixtures/skill-oracle-class-phase3.json" if args.class_phase3 else "fixtures/skill-oracle.json")

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
