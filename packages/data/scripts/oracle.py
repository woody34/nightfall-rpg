#!/usr/bin/env python3
"""Independent E1.4 oracle. Python 3.11+, standard library, no floating point.

Reads literal TOML source rows, NEVER *_q, fitted curve coefficients, gen_tables.py,
Rust output or Rust arithmetic helpers. Expressions below independently transcribe
SOURCES.md / pinned Java statements retained in formulas.toml. HP/MP use the original
85 rows, not the implementation's quadratic. Square roots are bounded by rational
intervals and rounding is accepted only when both endpoints agree.

Run `python3 packages/data/scripts/oracle.py [--check]`. The factored JSON golden is
consumed by Rust exhaustive and property tests; the audit JSON measures the OLD E1.3
contract against source rounding. Factorization avoids repeating identical values in
9 * 85 * 99^3 possible player sheets. No production dependency on Python.
"""
from fractions import Fraction as F
from math import isqrt
from pathlib import Path
import argparse
import hashlib
import json
import tomllib

ROOT = Path(__file__).resolve().parents[1]
Q = 10**6
STATS = ('str', 'dex', 'con', 'int', 'wit', 'men')
GRID = [1, 9, 25, 30, 43, 64, 99]


def read(rel):
    return tomllib.loads((ROOT / rel).read_text())


def floor(x):
    return x.numerator // x.denominator


def ceil(x):
    return -floor(-x)


def rounded(x):
    """Java Math.round's mathematical rule: floor(x + 1/2)."""
    return floor(x + F(1, 2))


def q(x):
    return floor(x * Q)


def root_bounds(n):
    # Much finer than production's root_Q; certify rather than assume its rounding.
    scale = 10**12
    lo = isqrt(n * scale**2)
    return F(lo, scale), F(lo if lo * lo == n * scale**2 else lo + 1, scale)


def rounded_root(dex, addition):
    lo, hi = root_bounds(dex)
    a, b = rounded(6 * lo + addition), rounded(6 * hi + addition)
    assert a == b, (dex, addition, 'ambiguous root rounding')
    return a


def binary_nearest(x, bits):
    """Positive, normal IEEE value rounded ties-even, represented as a Fraction.

    Used ONLY to quantify Java Float.parseFloat resource-row error; no host floats.
    All resource rows/bonuses are normal, finite, positive in binary32 and binary64.
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


def generate():
    bonuses = {s: list(map(F, read('tables/stat_bonus.toml')[s]['bonus'])) for s in STATS}
    formulas = read('tables/formulas.toml')
    # Guard the literal expressions that define this oracle. The oracle does not use
    # the loader's parsed constants, addition arrays, or scaled representations.
    literals = formulas['formulas']
    guards = {
        'level_mod': 'return ((getLevel() + 89) / 100d);',
        'p_atk': 'return initVal * BaseStats.STR.calcBonus(effector) * effector.getLevelMod();',
        'p_def': 'return value * effector.getLevelMod();',
        'max_hp': 'return initVal * BaseStats.CON.calcBonus(effector);',
        'max_mp': 'return initVal * BaseStats.MEN.calcBonus(effector);',
        'accuracy': 'double value = initVal + (Math.sqrt(effector.getDEX()) * 6) + level;',
        'evasion': 'diff *= 1.2;',
        'hit_chance': 'int chance = (80 + (2 * (attacker.getAccuracy() - target.getEvasionRate(attacker)))) * 10;',
        'crit_rate': 'return initVal * BaseStats.DEX.calcBonus(effector) * 10;',
        'phys_damage': 'damage = (76 * damage * proximityBonus) / defence;',
        'attack_speed': 'return initVal * BaseStats.DEX.calcBonus(effector);',
        'attack_interval': 'return (int) (500000 / getPAtkSpd());',
        'damage_hate': 'addDamageHate(attacker, damage, (damage * 100) / (getLevel() + 7));',
    }
    for key, literal in guards.items():
        assert literal in literals[key]['literal'], key
    weapon = read('tables/starter_weapon.toml')
    npc = read('npcs/keltir.toml')
    xp = [row['xp'] for row in read('tables/experience.toml')['to_level']]
    losses = [F(row['percent']) / 100 for row in read('tables/penalties.toml')['death_xp_loss']]
    assert len(xp) == 86 and len(losses) == 85
    for level, loss in enumerate(losses, 1):
        expected = max(F(4), F(10) - F(level - 1, 8)) if level <= 75 else {
            76: F('2.5'), 77: F(2), 78: F('1.5')}.get(level, F(1))
        assert loss == expected / 100
    level_of = lambda x: max(l for l in range(1, 86) if xp[l - 1] <= x)
    audit = {}

    def observe(name, delta, witness):
        delta = abs(F(delta))
        if name not in audit or delta > F(audit[name]['maximum']):
            audit[name] = {'maximum': str(delta), 'witness': witness}

    levels, dex_rows, old_dex = [], [], []
    for level in range(1, 86):
        acc_add = max(0, level - 69) + (level - 76 if level > 77 else 0)
        eva_add = max(0, level - 69) * (F(6, 5) if level >= 78 else 1)
        loss_exact = (xp[level] - xp[level - 1]) * losses[level - 1]
        loss = rounded(loss_exact)
        levels.append([q(F(level + 89, 100)), q(F(acc_add)), q(F(eva_add)), loss])
        observe('death_xp', loss - floor(loss_exact), [level, floor(loss_exact), loss])
        rows, old = [], []
        for dex in range(100):
            acc = rounded_root(dex, level + acc_add + F(weapon['accuracy']))
            eva = min(250, rounded_root(dex, level + eva_add))
            rows.append([acc * Q, eva * Q])
            root_q = isqrt(dex * Q**2)
            oa, oe = 6 * root_q + (level + acc_add) * Q, 6 * root_q + q(level + eva_add)
            old.append((oa, oe))
            observe('accuracy_q_to_rounded', F(oa, Q) - acc, [level, dex, oa, acc])
            observe('evasion_q_to_rounded', F(oe, Q) - eva, [level, dex, oe, eva])
        dex_rows.append(rows)
        old_dex.append(old)

    # All level pairs enter hit chance only via integer translations and the five
    # possible fractional evasion additions. Scan every DEX pair at every level,
    # then shift attacker level by -100..100 implicitly to put the difference in
    # the unclamped interval. Rounding error is invariant under those translations.
    for li, old in enumerate(old_dex):
        for ad in range(1, 100):
            for ed in range(1, 100):
                oa, oe = old[ad][0], old[ed][1]
                na, ne = dex_rows[li][ad][0], dex_rows[li][ed][1]
                delta = (20 * (oa - oe)) // Q - (20 * (na - ne)) // Q
                observe('hit_permille_unclamped_bound', delta, [li + 1, ad, ed])
                before = max(200, min(980, 800 + (20 * (oa - oe)) // Q))
                after = max(200, min(980, 800 + (20 * (na - ne)) // Q))
                observe('hit_permille_same_level', before - after, [li + 1, ad, ed, before, after])

    # Crit casts before +0.5, so the addition cannot round a fractional rate up.
    for base in (4, 8):
        for dex, bonus in enumerate(bonuses['dex']):
            rate = base * bonus * 10
            observe('crit_cast_then_half', floor(rate) - floor(F(floor(rate)) + F(1, 2)), [base, dex])
            java = binary_nearest(binary_nearest(base * binary_nearest(bonus, 53), 53) * 10, 53)
            observe('crit_binary_storage', floor(java) - floor(rate), [base, dex])

    speed_cases = []
    for base in (1, 300, 379, 1499, 1500, 2000, 10000):
        for dex in range(100):
            speed = min(1500, rounded(base * bonuses['dex'][dex]))
            ms = floor(F(500000, speed))
            speed_cases.append([base, dex, speed * Q, ms, max(1, ceil(F(ms // 2, 100))), max(1, ceil(F(ms, 100)))])
    crit_cases = [[base, dex, min(500, floor(base * bonuses['dex'][dex] * 10))]
                  for base in (0, 4, 8, 44, 100, 1000) for dex in range(100)]
    speed_rows = []
    for dex in range(100):
        row = []
        for base, crit in [(weapon['p_atk_spd'], weapon['crit_rate']), (300, 4)]:
            raw = base * bonuses['dex'][dex]
            speed = min(1500, rounded(raw))
            ms = floor(F(500000, speed))
            impact_ms = ms // 2
            impact, cycle = max(1, ceil(F(impact_ms, 100))), max(1, ceil(F(ms, 100)))
            rate = min(500, floor(crit * bonuses['dex'][dex] * 10))
            row.extend([speed * Q, rate, ms, impact, cycle])
            old_interval = F(500000) / raw
            observe('attack_speed', raw - speed, [base, dex, str(raw), speed])
            observe('interval_ms', old_interval - ms, [base, dex, str(old_interval), ms])
            observe('impact_ms', old_interval / 2 - impact_ms, [base, dex, str(old_interval / 2), impact_ms])
            observe('cycle_ticks', ceil(old_interval / 100) - cycle, [base, dex])
            observe('impact_ticks', ceil(old_interval / 200) - impact, [base, dex])
        speed_rows.append(row)

    classes, damage = [], []
    for path in sorted((ROOT / 'classes').glob('*.toml')):
        c = read(str(path.relative_to(ROOT)))
        base = c['base_stats']
        rows = []
        for level in range(1, 86):
            hp, mp = F(c['hp']['levels'][level - 1]), F(c['mp']['levels'][level - 1])
            lm = F(level + 89, 100)
            row = {'hp_curve': q(hp), 'mp_curve': q(mp),
                   'hp': [floor(hp * b) for b in bonuses['con']],
                   'mp': floor(mp * bonuses['men'][base['men']]),
                   'p_def': q(sum(c['p_def_slots'].values()) * lm),
                   'p_atk': [q(F(weapon['p_atk']) * b * lm) for b in bonuses['str']],
                   'fist_p_atk': [q(F(c['combat']['p_atk']) * b * lm) for b in bonuses['str']]}
            rows.append(row)
            # Java resources: source per-level float, promoted to double, times
            # double bonus, then integer cast. Quantify without introducing floats.
            for name, value, stat in [('hp', hp, 'con'), ('mp', mp, 'men')]:
                for index, b in enumerate(bonuses[stat]):
                    java = floor(binary_nearest(binary_nearest(value, 24) * binary_nearest(b, 53), 53))
                    observe(name + '_binary_storage', java - floor(value * b), [c['id'], level, index, floor(value * b), java])
                    if index == base[stat]:
                        observe(name + '_binary_storage_profiles', java - floor(value * b), [c['id'], level, index, floor(value * b), java])
            atk = F(weapon['p_atk']) * bonuses['str'][base['str']] * lm
            for crit in (0, 1):
                for spread in range(-10, 11):
                    dmg = max(1, floor(76 * atk * (2 if crit else 1) * F(100 + spread, 100) / F(npc['p_def'])))
                    hate = floor(F(dmg * 100, npc['level'] + 7))
                    damage.append([len(classes), level, crit, spread, dmg, hate])
        classes.append({'id': c['id'], 'base': [base[s] for s in STATS], 'rows': rows})

    xp_cases = []
    death_cases = []
    for li, threshold in enumerate(xp):
        for x in sorted({max(0, threshold - 1), threshold, threshold + 1}):
            xp_cases.append([x, level_of(x)])
        if li == 85:
            continue
        loss = levels[li][3]
        for x in sorted({threshold, threshold + loss - 1, threshold + loss, xp[li + 1] - 1}):
            after = max(0, x - loss)
            death_cases.append([li + 1, x, after, level_of(after)])
    hit_cases = []
    for a in ('0', '0.499999', '0.5', '0.500001', '33.863350', '40', '100', '250'):
        for e in ('0', '0.499999', '0.5', '0.500001', '31', '35', '100', '250'):
            chance = max(200, min(980, 800 + 20 * (rounded(F(a)) - rounded(F(e)))))
            hit_cases.append([q(F(a)), q(F(e)), chance])
    hate_cases = [[d, l, floor(F(d * 100, l + 7))] for d in (0, 1, 7, 152, 100000, 2**32 - 1) for l in range(1, 86)]
    golden = {'schema': 1, 'grid': GRID, 'bonus': [[q(v) for v in bonuses[s]] for s in STATS],
              'levels': levels, 'dex': dex_rows, 'speed': speed_rows, 'classes': classes,
              'xp': xp, 'xp_cases': xp_cases, 'death_cases': death_cases,
              'hit_cases': hit_cases, 'damage': damage, 'hate': hate_cases,
              'speed_cases': speed_cases, 'crit_cases': crit_cases,
              'hit_by_difference': [max(200, min(980, 800 + 20 * d)) for d in range(-250, 251)]}
    # These are evaluated separately from HF rows; never relabel legacy inputs HF.
    golden['inputs_sha256'] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted([*ROOT.glob('tables/*.toml'), *ROOT.glob('classes/*.toml'), ROOT / 'npcs/keltir.toml', ROOT / 'scripts/oracle.py'])}
    golden['legacy'] = {'ertheia_atk_q': q(F(12) * F('1.418259') * F('0.9')),
                        'interlude_crit': floor(44 * F('1.10')),
                        'fixture_hp': floor(F('637.7') * F('1.57')),
                        'fixture_mp': floor(F('287.4') * F('1.28'))}
    return golden, audit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    golden, audit = generate()
    outputs = {'oracle.json': json.dumps(golden, separators=(',', ':')) + '\n',
               'rounding-audit.json': json.dumps(audit, indent=2) + '\n'}
    for name, text in outputs.items():
        path = ROOT / 'fixtures' / name
        if args.check:
            if not path.exists() or path.read_text() != text:
                raise SystemExit(f'stale oracle output: {path}')
        else:
            path.parent.mkdir(exist_ok=True)
            path.write_text(text)
    print('oracle: 9 classes x 85 levels; 600 bonuses; 100-value stat axes; rounding audit verified')


if __name__ == '__main__':
    main()
