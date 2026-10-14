# Phase 1 — Stat Formulas

This document gives the base stats, the derived stats, and the experience curves. All combat
systems in Phases 2–4 call the functions in section 3 of this document.

---

## 1. Purpose and scope

**Delivers**

- The six base stats (STR, DEX, CON, INT, WIT, MEN) and the values that each stat controls. Also the
  multiplier curve that changes a stat value into a bonus.
- Derived stats: P.Atk, M.Atk, P.Def, M.Def, max HP/MP/CP, HP/MP/CP regeneration, accuracy, evasion,
  physical and magic critical rate, attack speed, cast speed, run speed, weight limit, hit chance.
- Leveling: the XP-to-level table, XP loss on death, and party XP sharing (per-member bonus and
  level-gap cutoff). Also the XP/damage penalties when you fight monsters far above or below your level.
- The Nightfall formulas as Rust signatures, a `Stats` struct, the data-file schema, and the proto changes.

**Excludes**

- These items are not in this phase:
  - Per-class stat values for classes other than the nine starting classes (Phase 2).
  - Skill/buff modifiers, and the damage pipeline that uses these stats (Phase 3).
  - Item stats (Phase 4).
  - SP/skill cost curves (Phase 3).
  - Attribute (fire/water/...) math (Phase 3).

---

## 2. Reference: how Lineage 2 does it

The numbers in this section come from the L2J server emulator (Interlude/C6 branch and the High Five
branch). The L2J team reverse-engineered the data files from retail. If the two branches are different,
this section quotes both.

### 2.1 The six base stats

| Stat | Governs (multiplier applied to) | Also affects |
|------|---------------------------------|--------------|
| **STR** | P.Atk | (HF) no other value; in L2, CON sets the weight limit, not STR |
| **DEX** | P.Atk speed, physical critical rate, run speed, accuracy and evasion (through √DEX) | shield block rate (Phase 3) |
| **CON** | Max HP, Max CP, HP regen, CP regen, weight limit | stun/bleed resistance in retail ("shock resistance"), breath |
| **INT** | M.Atk (squared) | debuff land rate in later chronicles |
| **WIT** | Casting speed, magic critical rate | magic resist / hold resistance |
| **MEN** | M.Def, Max MP, MP regen | mental debuff resistance, magic cancel resistance |

Each bonus is a multiplier with no dimension. It is approximately 1.0 at a "neutral" value. L2J
Interlude supplies the bonuses as 100-entry arrays in `stats/statBonus.xml`. The XML comments give the
formulas that make the arrays:

```
STR:  1.036 ^ (STR − 34.845)
INT:  1.020 ^ (INT − 31.375)
CON:  1.030 ^ (CON − 27.632)
MEN:  1.010 ^ (MEN + 0.060)
DEX:  1.009 ^ (DEX − 19.360)
WIT:  1.050 ^ (WIT − 20.000)
```

The maximum value is `MAX_STAT_VALUE = 100` (index clamp). The table below gives the values at each
5 points, rounded to two decimals. The table is the same as the XML, with only rounding differences.
For example, the XML has STR 40 → 1.20, 60 → 2.43, 99 → 9.67; WIT 50 → 4.32; CON 43 → 1.57:

| stat | 5 | 10 | 15 | 20 | 25 | 30 | 35 | 40 | 45 | 50 | 55 | 60 | 65 | 70 | 75 | 80 | 85 | 90 | 95 |
|------|---|----|----|----|----|----|----|----|----|----|----|----|----|----|----|----|----|----|----|
| STR | 0.35 | 0.42 | 0.50 | 0.59 | 0.71 | 0.84 | 1.01 | 1.20 | 1.43 | 1.71 | 2.04 | 2.43 | 2.91 | 3.47 | 4.14 | 4.94 | 5.89 | 7.03 | 8.39 |
| INT | 0.59 | 0.65 | 0.72 | 0.80 | 0.88 | 0.97 | 1.07 | 1.19 | 1.31 | 1.45 | 1.60 | 1.76 | 1.95 | 2.15 | 2.37 | 2.62 | 2.89 | 3.19 | 3.53 |
| CON | 0.51 | 0.59 | 0.69 | 0.80 | 0.93 | 1.07 | 1.24 | 1.44 | 1.67 | 1.94 | 2.25 | 2.60 | 3.02 | 3.50 | 4.06 | 4.70 | 5.45 | 6.32 | 7.33 |
| MEN | 1.05 | 1.11 | 1.16 | 1.22 | 1.28 | 1.35 | 1.42 | 1.49 | 1.57 | 1.65 | 1.73 | 1.82 | 1.91 | 2.01 | 2.11 | 2.22 | 2.33 | 2.45 | 2.58 |
| DEX | 0.88 | 0.92 | 0.96 | 1.01 | 1.05 | 1.10 | 1.15 | 1.20 | 1.26 | 1.32 | 1.38 | 1.44 | 1.51 | 1.57 | 1.65 | 1.72 | 1.80 | 1.88 | 1.97 |
| WIT | 0.48 | 0.61 | 0.78 | 1.00 | 1.28 | 1.63 | 2.08 | 2.65 | 3.39 | 4.32 | 5.52 | 7.04 | 8.99 | 11.47 | 14.64 | 18.68 | 23.84 | 30.43 | 38.83 |

These observations are important for tuning:

- WIT has the steepest curve, by a large quantity (×5 between 20 and 55). This is the reason that L2
  mages feel "fast" at approximately 40 WIT.
- MEN is almost linear and is never below 1.0.
- DEX has the flattest curve (a +10 DEX buff gives only approximately +9% attack speed).

The High Five branch rebased the stats. Each class got approximately +45 on its values, for example
Human Fighter STR 88 and not 40. The branch also replaced the arrays with closed forms, for example
`STR: 1.009^(STR−49)`, `INT: 1.01^(INT−49.4)`, `DEX: 1.00456^(DEX−19.27)`, `WIT: 1.01383^(WIT−64.57)`,
`CON: 1.01169^(CON−34.80)`, `MEN: 1.00369^(MEN+30.45)`. These curves have the same shape, but flatter
slopes and moved neutral points. We use the Interlude curves (steeper and easier to read) with the
Interlude base values.

### 2.2 Starting base stats per race/class (Interlude `playerTemplates.xml`)

| Class (id) | STR | DEX | CON | INT | WIT | MEN | base P.Atk | base M.Atk | base P.Def | base M.Def | atk spd | cast spd | base crit | run |
|------------|-----|-----|-----|-----|-----|-----|-----------|-----------|-----------|-----------|---------|----------|-----------|-----|
| Human Fighter (0) | 40 | 30 | 43 | 21 | 11 | 25 | 4 | 6 | 80 | 41 | 300 | 333 | 44 | 115 |
| Human Mystic (10) | 22 | 21 | 27 | 41 | 20 | 39 | 3 | 6 | 54 | 41 | 300 | 333 | 40 | 120 |
| Elf Fighter (18) | 36 | 35 | 36 | 23 | 14 | 26 | 4 | 6 | 80 | 41 | 300 | 333 | 46 | 125 |
| Elf Mystic (25) | 21 | 24 | 25 | 37 | 23 | 40 | 3 | 6 | 54 | 41 | 300 | 333 | 41 | 122 |
| Dark Elf Fighter (31) | 41 | 34 | 32 | 25 | 12 | 26 | 4 | 6 | 80 | 41 | 300 | 333 | 45 | 122 |
| Dark Elf Mystic (38) | 23 | 23 | 24 | 44 | 19 | 37 | 3 | 6 | 54 | 41 | 300 | 333 | 41 | 122 |
| Orc Fighter (44) | 40 | 26 | 47 | 18 | 12 | 27 | 4 | 6 | 80 | 41 | 300 | 333 | 42 | 117 |
| Orc Mystic (49) | 27 | 24 | 31 | 31 | 15 | 42 | 4 | 6 | 54 | 41 | 300 | 333 | 41 | 121 |
| Dwarf Fighter (53) | 39 | 29 | 45 | 20 | 10 | 27 | 4 | 6 | 80 | 41 | 300 | 333 | 43 | 115 |

In L2, the base stats are **fixed per class** (each Human Fighter has 40 STR for all its life). Only
dyes (henna), buffs, and later-chronicle items change them. The `grpc.rs` stub now returns the Human
Fighter row.

### 2.3 Level modifier

Almost all derived stats change with level through this formula:

```
levelMod(level) = (level + 89) / 100          // L2J L2Character.getLevelMod()
```

Level 1 → 0.90, level 20 → 1.09, level 40 → 1.29, level 60 → 1.49, level 76 → 1.65, level 80 → 1.69.

### 2.4 Derived stats (L2J `Func*` classes and `CharStat`)

| Derived | Formula | Source |
|---------|---------|--------|
| P.Atk | `(basePAtk + weaponPAtk) × STRmod × levelMod` | `FuncPAtkMod` |
| M.Atk | `(baseMAtk + weaponMAtk) × INTmod² × levelMod²` | `FuncMAtkMod` ("Level Modifier^2 * INT Modifier^2") |
| P.Def | `(basePDef − base of each occupied armour slot + armour P.Def) × levelMod` | `FuncPDefMod`; base per slot HF Human Fighter: chest 31, legs 18, head 12, feet 7, gloves 8, underwear 3, cloak 1 |
| M.Def | `(baseMDef − base of each occupied jewel slot + jewel M.Def) × MENmod × levelMod` | `FuncMDefMod`; base: earrings 9+9, rings 5+5, necklace 13 |
| Max HP | `classHpTable(level) × CONmod` | `FuncMaxHpMul` |
| Max CP | `classCpTable(level) × CONmod` | `FuncMaxCpMul` |
| Max MP | `classMpTable(level) × MENmod` | `FuncMaxMpMul` |
| HP regen /tick | `classHpRegen(level) × levelMod × CONmod × posture` | `Formulas.calcHpRegen`; posture: sitting ×1.5, standing ×1.1, running ×0.7 |
| MP regen | `classMpRegen(level) × levelMod × MENmod × posture` | `calcMpRegen` |
| CP regen | `classCpRegen(level) × levelMod × CONmod × posture` | `calcCpRegen` |
| Accuracy | `√DEX × 5 + level (+1 at 78+, +2 at 81+, +2 at 88+ …) + weapon accuracy` | `FuncAtkAccuracy` (the comment is "[Square(DEX)] * 5 + lvl") |
| Evasion | `√DEX × 5 + level (+ (level−69) above 69 for players)` | `FuncAtkEvasion` |
| P. critical rate | `baseCrit × DEXmod` (Interlude baseCrit 40–46) or `baseCrit × DEXmod × 10` (HF baseCrit 4) — both give ≈ 40–50 per mille | `FuncAtkCritical`; cap `MaxPCritRate = 500` |
| M. critical rate | `baseMCrit(5) × WITmod × 10` per mille | `FuncMAtkCritical`; cap `MaxMCritRate = 200` |
| P. attack speed | `basePAtkSpd(300 fighter / 240 HF mystic) × DEXmod` | `FuncPAtkSpeed`; cap 1500 |
| Casting speed | `baseMAtkSpd(333) × WITmod` | `FuncMAtkSpeed`; cap 1999 |
| Run speed | `baseRunSpd × DEXmod` | `FuncMoveSpeed`; cap `MaxRunSpeed = 300` (HF) |
| Weight limit | `floor(CONmod × 69000)`, clamped: CON < 1 → 31000, CON > 59 → 176000 | Interlude `getMaxLoad()` |

Crit and m-crit are **per mille**: `calcCrit` rolls `rate > Rnd.get(1000)`. A Human Fighter with
DEX 30 has rate 44 × 1.10 = 48 → 4.8 % crit. Physical crits cause ×2 damage. Magic crits cause ×3
damage against monsters and ×2 against players (HF).

**Hit chance** (`Formulas.calcHitMiss`):

```
chance‰ = clamp((80 + 2 × (attackerAccuracy − targetEvasion)) × 10 × conditionBonus, 200, 980)
miss    = chance < Rnd(1000)
```

Thus, if accuracy and evasion are equal, the chance to hit is 80 %. Each point of difference
changes the chance by ±2 %. The floor is 20 % and the ceiling is 98 %. `hitConditionBonus.xml` gives
the bonuses that follow. Attack from behind +10 %, from the side +5 %, from high ground +3 %, from low ground −3 %,
at night −10 %, in rain −3 %.

**Attack interval** (`L2Character.calculateTimeBetweenAttacks` / `Formulas.calcPAtkSpd`):

```
swing_ms   = 470000 / pAtkSpd          (HF; older branches 500000; minimum rate 2 → 2700 ms)
bow_ms     = 1500 × 345 / pAtkSpd      crossbow 1200 × 345 / pAtkSpd
bow_reuse  = weaponReuse × 345 / pAtkSpd
cast_ms    = skillTime × 333 / mAtkSpd (physical skills: skillTime × 300 / pAtkSpd)
```

300 atk speed → 1567 ms per swing; 500 → 940 ms; 1500 (cap) → 313 ms.

**Weight penalty** (`refreshOverloaded`): the load fraction is `w = currentLoad / maxLoad`. The penalty
levels are:

- `w < 50 %`: no penalty.
- 50–66.6 %: level 1.
- 66.6–80 %: level 2.
- 80–100 %: level 3.
- ≥ 100 %: level 4 (in retail, the character cannot move or attack).

Each penalty level is a level of skill 4270. This skill decreases the speed and the attack speed.

### 2.5 HP / MP / CP per level

High Five supplies an explicit per-level table for each class (`lvlUpgainData`). A fit of
`value(L) = base + a·(L−1) + b·(L−1)²` gives all of the 99 entries with zero error. Thus, the
retail tables are quadratics:

| Class | HP base | HP a | HP b | MP base | MP a | MP b | CP / HP |
|-------|---------|------|------|---------|------|------|---------|
| Human Fighter | 80 | 11.765 | 0.065 | 30 | 5.430 | 0.030 | 0.40 |
| Human Mystic | 101 | 15.385 | 0.085 | 40 | 7.240 | 0.040 | 0.50 |
| Elf Fighter | 89 | 12.670 | 0.070 | 30 | 5.430 | 0.030 | 0.40 |
| Elf Mystic | 104 | 15.385 | 0.085 | 40 | 7.240 | 0.040 | 0.50 |
| Dark Elf Fighter | 94 | 13.575 | 0.075 | 30 | 5.430 | 0.030 | 0.40 |
| Dark Elf Mystic | 106 | 15.385 | 0.085 | 40 | 7.240 | 0.040 | 0.50 |
| Orc Fighter | 80 | 12.670 | 0.070 | 30 | 5.430 | 0.030 | 0.50 |
| Orc Mystic | 95 | 15.385 | 0.085 | 40 | 7.240 | 0.040 | 0.50 |
| Dwarf Fighter | 80 | 12.670 | 0.070 | 30 | 5.430 | 0.030 | 0.70 |

Sample values (Human Fighter, before CON): L1 80 HP / 30 MP; L20 327 / 144; L40 637.7 / 287.4;
L60 1000.4 / 454.8; L76 1328 / 606. Mystics have *more* raw HP than fighters. CON causes the difference.

Regeneration per tick (the same for all nine starting classes, HF table):

- HP: 2.0 at L1, +0.05/level to 2.95 at L20, then `1.4 + 0.1 × L` (3.4 at 20, 5.4 at 40, 7.4 at 60,
  9.0 at 76, 9.4 at 80).
- MP: 0.9 through L10, then `0.9 + 0.03 × (L−10)` with a maximum of 3.0 (1.2 at 20, 1.8 at 40, 2.4 at 60).
- CP: 2.0 through L10, then `0.5 + 0.1 × L` (2.5 at 20, 4.5 at 40, 6.5 at 60), with a maximum of 8.5 at 76+.

The Interlude branch uses a different method. It stores `baseHpMax, levelHpAdd, levelHpMod` for each
class (Human Fighter 80.0 / 11.83 / 0.37) and calculates the curve in code. The HF table is the
better source, and we fit to it.

### 2.6 Experience table

This table gives the cumulative XP that is necessary to **reach** each level (`experience.xml`,
`tolevel`). Interlude (max 80) and High Five (max 85 for mains) are the same up to level 20, and
different after level 20. (The HF values were re-tuned to be much steeper in the 40s and 70s.) The
table shows both. Nightfall uses the Interlude column.

| L | Interlude | High Five | L | Interlude | High Five |
|---|-----------|-----------|---|-----------|-----------|
| 1 | 0 | 0 | 41 | 17,137,002 | 8,718,976 |
| 2 | 68 | 68 | 42 | 18,995,573 | 12,842,357 |
| 3 | 363 | 363 | 43 | 21,007,103 | 14,751,932 |
| 4 | 1,168 | 1,168 | 44 | 23,180,442 | 17,009,030 |
| 5 | 2,884 | 2,884 | 45 | 25,524,751 | 19,686,117 |
| 6 | 6,038 | 6,038 | 46 | 28,049,509 | 22,875,008 |
| 7 | 11,287 | 11,287 | 47 | 30,764,519 | 26,695,470 |
| 8 | 19,423 | 19,423 | 48 | 33,679,907 | 31,312,332 |
| 9 | 31,378 | 31,378 | 49 | 36,806,133 | 36,982,854 |
| 10 | 48,229 | 48,229 | 50 | 40,153,995 | 44,659,561 |
| 11 | 71,201 | 71,202 | 51 | 45,524,865 | 48,128,727 |
| 12 | 101,676 | 101,677 | 52 | 51,262,204 | 52,277,875 |
| 13 | 141,192 | 141,193 | 53 | 57,383,682 | 57,248,635 |
| 14 | 191,452 | 191,454 | 54 | 63,907,585 | 63,216,221 |
| 15 | 254,327 | 254,330 | 55 | 70,852,742 | 70,399,827 |
| 16 | 331,864 | 331,867 | 56 | 80,700,339 | 79,078,300 |
| 17 | 426,284 | 426,288 | 57 | 91,162,131 | 89,616,178 |
| 18 | 539,995 | 540,000 | 58 | 102,265,326 | 102,514,871 |
| 19 | 675,590 | 675,596 | 59 | 114,038,008 | 118,552,044 |
| 20 | 835,854 | 835,862 | 60 | 126,509,030 | 140,517,709 |
| 21 | 1,023,775 | 920,357 | 61 | 146,307,211 | 153,064,754 |
| 22 | 1,242,536 | 1,015,431 | 62 | 167,243,291 | 168,231,664 |
| 23 | 1,495,531 | 1,123,336 | 63 | 189,363,788 | 186,587,702 |
| 24 | 1,786,365 | 1,246,808 | 64 | 212,716,741 | 208,840,245 |
| 25 | 2,118,860 | 1,389,235 | 65 | 237,351,413 | 235,877,658 |
| 26 | 2,497,059 | 1,554,904 | 66 | 271,973,532 | 268,833,561 |
| 27 | 2,925,229 | 1,749,413 | 67 | 308,441,375 | 309,192,920 |
| 28 | 3,407,873 | 1,980,499 | 68 | 346,825,235 | 358,998,712 |
| 29 | 3,949,727 | 2,260,321 | 69 | 387,197,529 | 421,408,669 |
| 30 | 4,555,766 | 2,634,751 | 70 | 429,632,402 | 493,177,635 |
| 31 | 5,231,213 | 2,844,287 | 71 | 474,205,751 | 555,112,374 |
| 32 | 5,981,539 | 3,093,068 | 72 | 532,692,055 | 630,494,192 |
| 33 | 6,812,472 | 3,389,496 | 73 | 606,319,094 | 722,326,994 |
| 34 | 7,729,999 | 3,744,042 | 74 | 696,376,867 | 834,354,722 |
| 35 | 8,740,372 | 4,169,902 | 75 | 804,219,972 | 971,291,524 |
| 36 | 9,850,111 | 4,683,988 | 76 | 931,275,828 | 1,139,165,674 |
| 37 | 11,066,012 | 5,308,556 | 77 | 1,151,275,834 | 1,345,884,863 |
| 38 | 12,395,149 | 6,074,376 | 78 | 1,511,275,834 | 1,602,331,019 |
| 39 | 13,844,879 | 7,029,248 | 79 | 2,099,275,834 | 1,902,355,477 |
| 40 | 15,422,851 | 8,342,182 | 80 | 4,200,000,000 | 2,288,742,870 |

HF continues: 81 → 2,703,488,268; 82 → 3,174,205,601; 83 → 3,708,727,539; 84 → 4,316,300,702;
85 → 5,008,025,097 (then a ×2 increase for each level to 99, for the Awakening content). The Interlude
file lists 81–87 as a geometric 1.5×/1.4×/1.3×… extension. Only the subclass/GM cap uses it. Interlude has a "wall": 76→77 costs 220 M, 77→78 360 M, 78→79 588 M, 79→80 2.1 B. The designers
wanted level 80 to be almost impossible to reach.

### 2.7 XP loss on death

Interlude (`PlayerInstance.deathPenalty`) uses a flat percentage of the **span of the current level**
(`expForLevel(L+1) − expForLevel(L)`). It does not use a percentage of the total XP:

| Level | % of current level lost |
|-------|-------------------------|
| 1–19 | 10 % |
| 20–39 | 7 % |
| 40–74 | 4 % |
| 75–80 | 2 % |

Modifiers:

- ÷4 if the character dies during a clan war, siege, or festival.
- × `RateKarmaExpLost` (default 1) if the victim has karma.
- Zero in peace zones and events.

High Five replaced the brackets with a per-level table (`playerXpPercentLost.xml`). The loss is 10.0 %
at L1, and decreases by 0.125 for each level to 4.0 % at L49. It is a flat 4.0 % through L75, then
2.5 (76), 2.0 (77), 1.5 (78), 1.0 % at 79+.

Classic (2015 relaunch) went in the opposite direction. It used a flat 10 % at all levels. Level loss
was possible from level 10 and up, and a PvE death could drop items. The 2016 Classic 1.5 notes list 4 %.
Later chronicles (GoD+) remove level loss fully, and add the Shilen's Breath debuff in its place.

The L2J rule for level loss: XP can go below the level floor, and then the character de-levels
(`AltGameDelevel`), down to level 1.

### 2.8 Party XP

`L2Party.distributeXpAndSp`:

1. Find the **valid members**. These are the members in the level cutoff, relative to the
   highest-level member (`topLvl`). The methods are:
   - `level` (gap ≤ `PartyXpCutoffLevel` = 20).
   - `percentage` (the level² of the member must be ≥ 3 % of the sum of squares).
   - `auto`.
   - The retail-accurate `highfive` gaps. These gaps give `0–9 → 100 %`, `10–14 → 30 %`, `15+ → 0 %` of the calculated share of the member.
2. Multiply the XP/SP of the mob by the **party bonus** for the number of valid members:

| Members | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |
|---------|---|---|---|---|---|---|---|---|---|
| Interlude | 1.00 | 1.30 | 1.39 | 1.50 | 1.54 | 1.58 | 1.63 | 1.67 | 1.71 |
| High Five | 1.00 | 1.10 | 1.20 | 1.30 | 1.40 | 1.50 | 2.00 | 2.10 | 2.20 |
| Community (Classic) | 1.0 | 1.3 | 1.4 | 1.5 | 1.6 | 1.7 | 2.0 | — | — |

3. Divide by **level squared**: the share of a member = `level² / Σ level²` of the valid members. If a
   level 40 and a level 20 are in a party, the level 40 gets 80 % of the pool. Dead members get
   nothing. If a servitor has an XP penalty, that penalty also applies to the share of its owner.

### 2.9 Level difference vs monsters

In L2, the XP reward of a monster is a fixed value on the NPC template. In Interlude/HF code, there
is **no XP scaling by level difference**. The limit is indirect, through damage and hit chance.

HF (`NPC.properties`, Gracia Epilogue rule) applies penalties if a monster of level ≥ 78 is above you:

- Damage ×0.7 / 0.6 / 0.6 / 0.55 for 2 / 3 / 4 / 5 levels below the mob (crits 0.75 / 0.65 / 0.6 / 0.58;
  skills 0.8 / 0.7 / 0.65 / 0.62).
- Magic land chance is divided by 2.5 / 3.0 / 3.25 / 3.5 at 3–6 levels below.

Magic success also includes `lvlModifier = 1.3^(targetLevel − casterLevel)` in `calcMagicSuccess`.
Thus, each level that the target has above you decreases the land rate by 30 %. Private servers and
Classic-era community guides describe explicit XP tables. For example: "|Δ| < 3 → 100 %, 3–10 → 70 %,
11–14 → 10 %, 15+ → 0 %" or "0–5 → 100 %, 6–9 → 30 %, 10+ → 0 %". These are fan rules, not retail rules.

---

## 3. Design decisions for Nightfall

### 3.1 Keep the six stats and the Interlude multiplier curves

The reasons:

- The curves are small closed forms.
- The curves give the same values as the retail tables.
- The curves give useful tuning controls (a +4 STR dye gives a visible +15 % P.Atk).

Stat values are `u8`, clamped to `1..=99`. The Nightfall curves are exactly the Interlude formulas:

```rust
pub fn stat_bonus(stat: Stat, value: u8) -> f32 {
    let v = value.clamp(1, 99) as f32;
    match stat {
        Stat::Str => 1.036f32.powf(v - 34.845),
        Stat::Int => 1.020f32.powf(v - 31.375),
        Stat::Con => 1.030f32.powf(v - 27.632),
        Stat::Men => 1.010f32.powf(v + 0.060),
        Stat::Dex => 1.009f32.powf(v - 19.360),
        Stat::Wit => 1.050f32.powf(v - 20.000),
    }
}
```

At boot, calculate the values into a `[[f32; 100]; 6]` table. Thus, the combat code does lookups, not `powf`.

### 3.2 Level modifier, caps, and starting values

- `level_mod(L) = (L + 89) / 100`, level cap **80**, XP table = Interlude column (data file).
- Starting base stats, base P/M.Atk, base P/M.Def, speeds, base crit = the Interlude table in §2.2.
  These values are stored in `classes/*.toml`. In Nightfall also, base stats do not change with
  level. Dyes and buffs add to them.
- Hard caps (from HF `Character.properties`, and we keep them): run speed 300, P.Atk speed 1500,
  cast speed 1999, P.crit 500 ‰, M.crit 200 ‰, evasion 250. Hit chance floor 20 %, ceiling 98 %.

### 3.3 Derived stat formulas (normative)

Let `S(x) = stat_bonus(x, value)`, `LM = level_mod(level)`, `L = level`. The equipment values are
added into `gear` (Phase 4), and the buff values into `mods` (Phase 3). Both are `(add, mul)` pairs.
At the end, the pairs apply as `(base_result + add) * mul`.

```
p_atk       = (class.base_p_atk + gear.p_atk) * S(STR) * LM
m_atk       = (class.base_m_atk + gear.m_atk) * S(INT)^2 * LM^2
p_def       = (class.base_p_def_unarmoured + gear.p_def) * LM     // unarmoured base shrinks per worn slot, as L2
m_def       = (class.base_m_def_unjewelled + gear.m_def) * S(MEN) * LM
max_hp      = (class.hp.base + class.hp.per_level*(L-1) + class.hp.accel*(L-1)^2) * S(CON)
max_mp      = (class.mp.base + class.mp.per_level*(L-1) + class.mp.accel*(L-1)^2) * S(MEN)
max_cp      = max_hp_before_con * class.cp_ratio * S(CON)
hp_regen    = hp_regen_table(L) * LM * S(CON) * posture      // posture: sit 1.5, stand 1.1, move 0.7
mp_regen    = mp_regen_table(L) * LM * S(MEN) * posture
cp_regen    = cp_regen_table(L) * LM * S(CON) * posture
accuracy    = sqrt(DEX) * 5 + L + gear.accuracy
evasion     = sqrt(DEX) * 5 + L + max(0, L - 69) + gear.evasion
crit_rate   = min(500, class.base_crit * S(DEX) + gear.crit)                      // per mille
m_crit_rate = min(200, 50 * S(WIT))                                                // per mille
p_atk_spd   = min(1500, class.base_p_atk_spd * S(DEX) * gear.atk_spd_mul)
m_atk_spd   = min(1999, class.base_m_atk_spd * S(WIT))
run_speed   = min(300, class.run_speed * S(DEX))
weight_max  = clamp(CON, 1, 59) -> floor(S(CON) * 69000), floor to 31000 / cap 176000
swing_ms    = 470000 / p_atk_spd        bow: 1500 * 345 / p_atk_spd
cast_ms     = skill.cast_ms * 333 / m_atk_spd
hit_chance‰ = clamp((80 + 2*(acc_attacker - eva_target)) * 10 * condition, 200, 980)
```

The regen tables are the piecewise-linear HF curves from §2.5, supplied as data. The posture
multipliers apply for each 3-second regen tick. This is the same as the regen task period of L2.

**Worked example (Human Fighter, no gear, Interlude stats 40/30/43/21/11/25):**

| L | LM | HP | MP | CP | P.Atk | M.Atk | P.Def | M.Def | Acc | Crit‰ | Atk spd | Cast spd | Run | Weight |
|---|----|----|----|----|-------|-------|-------|-------|-----|-------|---------|----------|-----|--------|
| 1 | 0.90 | 126 | 38 | 50 | 4.3 | 3.2 | 72 | 47 | 28 | 48 | 330 | 215 | 127 | 108,675 |
| 20 | 1.09 | 515 | 185 | 206 | 5.2 | 4.7 | 87 | 57 | 47 | 48 | 330 | 215 | 127 | 108,675 |
| 40 | 1.29 | 1004 | 369 | 402 | 6.2 | 6.6 | 103 | 68 | 67 | 48 | 330 | 215 | 127 | 108,675 |
| 76 | 1.65 | 2092 | 778 | 837 | 7.9 | 10.8 | 132 | 87 | 103 | 48 | 330 | 215 | 127 | 108,675 |

For comparison, the Human Mystic at L1: M.Atk 7.1 (INT 41 → 1.19² × 0.81), cast speed 333 (WIT 20 → 1.00),
MP 59, HP 99. The "fists" P.Atk is approximately 4. This is the reason that L2 starting weapons
(P.Atk 8–15) are important: the weapon P.Atk is the largest part of `p_atk` until level 40 gear.

### 3.4 Experience rules

- **Gain**: `xp = npc.xp * level_gap_mult(player, npc) * rates.xp`. Nightfall **does** add an
  explicit XP level-gap multiplier. (L2 gives only indirect penalties, and players cannot see them.) The multipliers are:

| `npc.level − player.level` | multiplier |
|----------------------------|------------|
| ≥ +6 | 1.00 (plus the damage/land-rate penalties below) |
| −5 … +5 | 1.00 |
| −6 … −8 | 0.70 |
| −9 … −10 | 0.40 |
| −11 … −14 | 0.10 |
| ≤ −15 | 0.00 |

- **Over-level penalties** (from HF): these penalties apply to all monsters, not only L78+. If `npc.level −
  player.level ≥ 2`, these penalties apply:
  - Outgoing damage ×0.7/0.6/0.6/0.55 at Δ 2/3/4/5+ (crits 0.75/0.65/0.6/0.58, skills 0.8/0.7/0.65/0.62).
  - Magic land chance `× 1 / 1.3^Δ` for Δ ≥ 1 (the `calcMagicSuccess` term).
- **Death**: the Interlude brackets — 10 % (L1–19), 7 % (20–39), 4 % (40–74), 2 % (75–80) of the XP
  span of the current level.
  - ÷4 in clan war and siege zones. 0 in peace zones and instanced events.
  - De-leveling is possible down to the level 1 XP floor. But if a character reached level 10, it
    cannot go below level 10. (We use this Classic rule to protect new players.)
  - For repeat deaths, death uses the Shilen's Breath-style debuff system (Phase 3), not XP.
- **Party**: find the valid members by the HF gaps (0–9 → 100 %, 10–14 → 30 %, 15+ → 0 %), measured
  from the highest-level member.
  - The bonus for the valid-member count = the Interlude table (1.00, 1.30, 1.39, 1.50, 1.54,
    1.58, 1.63, 1.67, 1.71).
  - The share comes from `level² / Σlevel²`.
  - Members must be in the 3 × 3 interest block of the killer (Phase 0 cell grid, ≈ 1500 units). This
    is almost the same as the 1500-unit party-reward range of L2.
- All constants are in `tables/experience.toml` and `tables/penalties.toml`. Thus, you can tune them
  without a rebuild.

### 3.5 Alternatives considered

- *Linear stat bonuses* ("+1 STR = +1 % P.Atk") — this is simpler. But dyes/buffs become less
  interesting, and the result is different from what L2 players expect from a build. Rejected.
- *HF rebased stats* (STR 88 and other values) — flatter curves and larger numbers. These give no
  advantage for a new game.
- *XP scaling by formula and not by table* — the Interlude table is irregular intentionally (it has
  plateaus at the 20/40/76 class-change levels). Keep it as data.
- *Percent-of-total XP death penalty* — this gives much larger penalties at high levels than L2
  intends. Keep percent-of-level-span.

---

## 4. Data model

### 4.1 Rust types (`apps/api/src/stats/`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Deserialize)]
pub enum Stat { Str, Dex, Con, Int, Wit, Men }

#[derive(Clone, Copy, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct BaseStats { pub str: u8, pub dex: u8, pub con: u8, pub int: u8, pub wit: u8, pub men: u8 }

#[derive(Clone, Copy, Debug, serde::Deserialize)]
pub struct Curve { pub base: f32, pub per_level: f32, pub accel: f32 }   // base + a(L-1) + b(L-1)^2

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ClassTemplate {
    pub id: String, pub race: Race, pub base: BaseStats,
    pub hp: Curve, pub mp: Curve, pub cp_ratio: f32,
    pub base_p_atk: f32, pub base_m_atk: f32, pub base_p_def: f32, pub base_m_def: f32,
    pub base_p_atk_spd: f32, pub base_m_atk_spd: f32, pub base_crit: f32,
    pub run_speed: f32, pub walk_speed: f32,
    pub p_def_slots: SlotDefs,   // chest/legs/head/feet/gloves/under/cloak unarmoured bases
    pub m_def_slots: JewelDefs,  // rear/lear/rfinger/lfinger/neck bases
}

/// Flat additive / multiplicative modifiers gathered from gear and effects (Phases 3–4).
#[derive(Clone, Copy, Debug, Default)]
pub struct Modifiers { pub add: DerivedTable, pub mul: DerivedTable }

/// Fully derived snapshot; recomputed when level, base stats, gear, or effects change (never per tick).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub level: u8,
    pub base: BaseStats,                 // after dyes/buffs, clamped 1..=99
    pub p_atk: f32, pub m_atk: f32, pub p_def: f32, pub m_def: f32,
    pub max_hp: u32, pub max_mp: u32, pub max_cp: u32,
    pub hp_regen: f32, pub mp_regen: f32, pub cp_regen: f32,   // per 3 s regen tick, standing
    pub accuracy: f32, pub evasion: f32,
    pub crit_rate: u16, pub m_crit_rate: u16,                 // per mille, capped
    pub p_atk_spd: u16, pub m_atk_spd: u16,
    pub run_speed: f32, pub walk_speed: f32,
    pub weight_max: u32,
}

pub fn stat_bonus(stat: Stat, value: u8) -> f32;
pub fn level_mod(level: u8) -> f32;
pub fn derive(class: &ClassTemplate, level: u8, base: BaseStats, mods: &Modifiers) -> Stats;
pub fn swing_interval_ms(p_atk_spd: u16, weapon: WeaponKind) -> u32;
pub fn cast_time_ms(skill_cast_ms: u32, m_atk_spd: u16) -> u32;
pub fn hit_chance_permille(acc: f32, eva: f32, condition: f32) -> u16;   // clamp(…,200,980)
pub fn weight_penalty_level(current: u32, max: u32) -> u8;              // 0..=4

pub struct XpTable(Vec<u64>);        // index = level, value = cumulative XP to reach it
impl XpTable {
    pub fn for_level(&self, level: u8) -> u64;
    pub fn level_for(&self, xp: u64) -> u8;
    pub fn span(&self, level: u8) -> u64 { self.for_level(level + 1) - self.for_level(level) }
}
pub fn death_xp_loss(table: &XpTable, level: u8, ctx: DeathContext) -> u64;
pub fn level_gap_xp_mult(player_level: u8, npc_level: u8) -> f32;
pub fn party_bonus(valid_members: usize) -> f32;
pub fn party_shares(members: &[(CharId, u8 /*level*/)], top_level: u8) -> Vec<(CharId, f32)>;
```

`derive` is pure. Tests lock the §3.3 worked-example rows to two decimals. Thus, a refactor cannot change the results.

### 4.2 Data files

```toml
# packages/data/tables/experience.toml
max_level = 80
to_level = [0, 0, 68, 363, 1168, 2884, 6038, 11287, 19423, 31378, 48229, # … Interlude column …
            4200000000]

# packages/data/tables/penalties.toml
death_pct_by_level = [[1, 10.0], [20, 7.0], [40, 4.0], [75, 2.0]]   # [bracket start, percent of level span]
death_war_divisor = 4.0
party_bonus = [1.0, 1.30, 1.39, 1.50, 1.54, 1.58, 1.63, 1.67, 1.71]
party_gap = [[0, 1.0], [10, 0.3], [15, 0.0]]
xp_gap_below = [[0, 1.0], [6, 0.7], [9, 0.4], [11, 0.1], [15, 0.0]]
dmg_vs_higher = [1.0, 1.0, 0.7, 0.6, 0.6, 0.55]                      # index = level gap 0..5+
crit_vs_higher = [1.0, 1.0, 0.75, 0.65, 0.6, 0.58]
skill_vs_higher = [1.0, 1.0, 0.8, 0.7, 0.65, 0.62]
magic_land_base = 1.3

# packages/data/tables/regen.toml — HF piecewise curves, see §2.5
```

### 4.3 Proto changes (`packages/proto/nightfall/v1/game.proto`)

```proto
message BaseStats {              // unchanged field numbers; document semantics
  uint32 str = 1; uint32 dex = 2; uint32 con = 3; uint32 int = 4; uint32 wit = 5; uint32 men = 6;
}

message DerivedStats {
  uint32 p_atk = 1;  uint32 m_atk = 2;  uint32 p_def = 3;  uint32 m_def = 4;
  uint32 max_hp = 5; uint32 max_mp = 6; uint32 max_cp = 7;
  uint32 accuracy = 8; uint32 evasion = 9;
  uint32 crit_rate = 10; uint32 m_crit_rate = 11;     // per mille
  uint32 p_atk_spd = 12; uint32 m_atk_spd = 13;
  float  run_speed = 14; float walk_speed = 15;
  uint32 weight_max = 16; uint32 weight_current = 17;
}

message Vitals { uint32 hp = 1; uint32 mp = 2; uint32 cp = 3; }

message Character {
  string id = 1; string name = 2; Race race = 3; uint32 level = 4;
  BaseStats stats = 5;           // effective (after dyes/buffs)
  Position position = 6;
  string class_id = 7;           // data id, e.g. "human_fighter"
  uint64 xp = 8;                 // cumulative
  uint64 xp_to_next = 9;         // table[level+1] - xp, 0 at cap
  DerivedStats derived = 10;
  Vitals vitals = 11;
  BaseStats base_stats = 12;     // class template values, for the character sheet
}
```

The server **sends the derived stats. The client does not calculate them**. The client has no
gear/effect data model now, and its numbers must always agree with the numbers of the server.

---

## 5. Interfaces

- `GameService.GetCharacter` now fills `class_id`, `xp`, `xp_to_next`, `derived`, `vitals`.
- New server-side events on the world bus:
  - `StatsRecomputed{entity, Stats}`. The stats system emits this event if level/base/gear/effects
    change. Replication changes it into a `DerivedStats` delta for the owner, and into an
    `EntityDelta{max_hp}` for observers.
  - `XpChanged{char, xp, delta, reason}`.
  - `LevelChanged{char, from, to}`.
- `WorldDelta.EntityDelta` gets optional `hp`, `max_hp`, `cp`, `max_cp` for all visible entities,
  and `mp`, `max_mp`, `xp`, `level` only for the owner.
- Admin: if a data hot-reload changes a curve, `POST /admin/stats/recompute` runs `derive` for all
  online characters.

---

## 6. Rust implementation notes

- Module `stats/` (see the Phase 0 layout): `bonus.rs` (curves + cached `[[f32;100];6]`),
  `derive.rs` (`derive`, caps), `xp.rs` (`XpTable`, death/party/gap functions), `regen.rs`
  (3-second regen task for each active cell), `tests.rs`.
- `Stats` is a plain `Copy` struct, stored on the entity. An event starts each recalculation. The
  recalculation never occurs for each tick.
- A recalculation costs approximately 30 multiplications. Thus, it is satisfactory to do it at each
  equip change for 5k entities.
- All randomness in combat uses one `rand::rngs::SmallRng` for each world thread. The seed comes from
  config, so replays are deterministic. `hit_chance_permille` and crit rolls take `&mut impl Rng`.
- XP is `u64`. The maximum of the Interlude table is 4.2 × 10⁹, which is already too large for `i32`
  (L2J uses `long`). The Postgres column is `BIGINT`.
- Floating point: use `f32` for stats. Round to integer only at the API boundary (`as u32` after
  `.round()`). In tests, compare with `approx::assert_relative_eq!(…, epsilon = 0.01)`.
- Data validation at load:
  - Each `Curve` is strictly increasing over 1..=80.
  - `cp_ratio ∈ [0.3, 1.0]`.
  - The XP table is strictly increasing, with `to_level[1] == 0`.
  - All penalty tables are sorted by key.

---

## 7. Client implications

- The character sheet reads `DerivedStats` + `BaseStats` directly from the server. It shows the base
  stats in white, and the dye/buff delta in green/red (L2 convention). It uses `base_stats` vs `stats`
  for this.
- Movement uses `derived.run_speed` as units/second. The Phase 0 dead reckoning of the client must
  have the exact value that the server uses for its integration.
- The server-sent `swing_ms`/`cast_ms` in the action start message (Phase 3) control the attack and
  cast bars. The client does not calculate them again from attack speed.
- The HP/CP bars for other entities come from `EntityDelta`. Only the owner gets MP and XP. The XP bar
  is `(xp − table[level]) / xp_to_next_span`. The client gets both the cumulative `xp` and
  `xp_to_next`, so it does not have to have a table.
- Weight: show the four penalty thresholds at 50 / 66.6 / 80 / 100 % of `weight_max`.

---

## 8. Open questions

1. **Crit base**: Interlude per-class `baseCritRate` (40–46) vs HF `4 × 10`. We use the Interlude
   per-class value. When weapons add crit (Phase 4), make sure that the value feels correct.
2. **Mystic P.Atk speed**: Interlude lists 300 for mystics, HF 240. We use 300 (Interlude parity). In
   Phase 2, HF 240 can become a class-tree tuning control.
3. **Regen tick period**: 3 s, as in L2. A 1 s tick with a third of the value gives a smoother HP
   bar. Make the decision in Phase 3, together with the potion ticks.
4. **Do we show "shock/mental resistance" from CON/MEN** as in retail? This decision is related to the
   status effect model (Phase 3).
5. **De-level floor**: we selected level 10 as a floor for all characters that reached it. L2J lets a
   character de-level to 1. Examine this again with the death-debuff design.
6. **XP table beyond 80**: we have no plan for this. If a cap increase becomes necessary, extend the
   table with the Interlude 1.5× extension entries. These entries are already in the file (81 → 6.3 B …).
7. We did not use the parametric HP formula of Interlude (`baseHpMax, levelHpAdd, levelHpMod`). We
   fit the HF per-level table in its place. If an Interlude-exact curve is necessary, calculate it
   from the C6 `FuncMaxHpAdd` source.

---

## 9. Sources

- L2J Interlude (C6) fork, <https://github.com/Hl4p3x/L2JServer_C6_Interlude>:
  `dist/game/data/stats/statBonus.xml` (six 100-entry tables with generating formulas in comments),
  `dist/game/data/stats/playerTemplates.xml` (starting base stats, base P/M.Atk/Def, speeds, crit,
  HP/MP/CP parameters), `dist/game/data/stats/experience.xml` (XP table, maxLevel 80),
  `model/skills/BaseStat.java` (`MAX_STAT_VALUE = 100`, array lookup),
  `model/actor/instance/PlayerInstance.java` (`getMaxLoad`: "Weight Limit = (CON Modifier*69000)",
  clamps 31000/176000; `deathPenalty`: 10/7/4/2 % brackets, ÷4 at war/siege),
  `model/skills/Formulas.java` (`calcHitMiss`, `calcPhysDam` 70·P.Atk/P.Def, `calcMagicDam`
  91·√M.Atk/M.Def), `model/Party.java` (BONUS_EXP_SP 1.00…1.71, level² shares).
- L2J High Five era source mirrored at <https://github.com/andridgitalbox/l2j-mobius>:
  `model/stats/BaseStats.java` (HF closed-form curves), `model/stats/functions/formulas/FuncPAtkMod,
  FuncMAtkMod, FuncPDefMod, FuncMDefMod, FuncMaxHpMul, FuncMaxMpMul, FuncMaxCpMul, FuncAtkAccuracy,
  FuncAtkEvasion, FuncAtkCritical, FuncMAtkCritical, FuncPAtkSpeed, FuncMAtkSpeed, FuncMoveSpeed`,
  `model/actor/L2Character.java` (`getLevelMod` = (level+89)/100; `calculateTimeBetweenAttacks`),
  `model/stats/Formulas.java` (`calcPAtkSpd` 470000/rate, `calcAtkSpd` 333/300, `calcHpRegen`
  posture multipliers, `calcMagicSuccess` 1.3^Δ, `calcCrit` per-mille roll),
  `model/actor/instance/L2PcInstance.java` (`calculateDeathExpPenalty`, `refreshOverloaded`
  thresholds 500/666/800/1000 ‰), `model/L2Party.java` (BONUS_EXP_SP HF, `highfive` cutoff),
  `dist/game/data/stats/chars/baseStats/{HumanFighter,HumanMystic,ElvenFighter,ElvenMystic,
  DarkFighter,DarkMystic,OrcFighter,OrcMystic,DwarvenFighter}.xml` (per-level HP/MP/CP/regen tables,
  base P.Def per slot), `dist/game/data/stats/chars/playerXpPercentLost.xml`,
  `dist/game/data/stats/hitConditionBonus.xml`, `dist/game/data/stats/experience.xml` (HF table to 99),
  `dist/game/config/Character.properties` (stat caps, party cutoff defaults),
  `dist/game/config/NPC.properties` (level-difference damage and magic penalties).
- L2 Classic death penalty (flat 10 %, level loss from level 10, item drop table):
  <https://www.l2scroll.com/2015/10/classic-death-penalty.html>.
- Shilen's Breath / no-level-loss death penalty (later chronicles):
  <https://wiki.l2ertheia.eu/doku.php?id=general:death-penalty>.
- Death penalty overview and XP-protection items: <https://www.ludo.guide/guide/lineage-ii/main-walkthrough/death-penalties-resurrection>.
- Community party-bonus and level-gap tables (Classic): Steam discussion
  <https://steamcommunity.com/app/373700/discussions/0/485624149158226564>; private-server rule set
  <https://wiki.ymirheim.org/en/EXP>.
- Stat descriptions (DEX accuracy/evasion/atk speed/crit): <https://lineage2.fandom.com/wiki/DEX>;
  general stat roles: <https://www.mmoexp.com/News/lineage-2m-stats-guide-how-to-build-the-perfect-character.html>.
- Lineage II Classic 1.5 patch notes (4 % XP loss on death) at <https://www.lineage2.com/news/lineage-ii-classic-launch-patch-notes>.
