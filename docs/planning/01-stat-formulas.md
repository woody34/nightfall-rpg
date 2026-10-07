# Phase 1 — Stat Formulas

Base stats, derived stats, and experience curves. Everything combat-related in Phases 2–4 calls the
functions defined in section 3 of this document.

---

## 1. Purpose and scope

**Delivers**

- The six base stats (STR, DEX, CON, INT, WIT, MEN), what each governs, and the multiplier curve
  that converts a stat value into a bonus.
- Derived stats: P.Atk, M.Atk, P.Def, M.Def, max HP/MP/CP, HP/MP/CP regeneration, accuracy, evasion,
  physical and magic critical rate, attack speed, cast speed, run speed, weight limit, hit chance.
- Leveling: the XP-to-level table, XP loss on death, party XP sharing (per-member bonus and level-gap
  cutoff), and XP/damage penalties for fighting monsters far above or below your level.
- Nightfall's chosen formulas as Rust signatures, a `Stats` struct, data-file schema, and proto changes.

**Excludes**

- Per-class stat values beyond the nine starting classes (Phase 2), skill/buff modifiers and the
  damage pipeline that consumes these stats (Phase 3), item stats (Phase 4), SP/skill cost curves
  (Phase 3). Attribute (fire/water/...) math is Phase 3.

---

## 2. Reference: how Lineage 2 does it

Numbers below come from the L2J server emulator (Interlude/C6 branch and the High Five branch),
whose data files were reverse-engineered from retail. Where the two differ, both are quoted.

### 2.1 The six base stats

| Stat | Governs (multiplier applied to) | Also affects |
|------|---------------------------------|--------------|
| **STR** | P.Atk | (HF) nothing else; weight limit is CON in L2, not STR |
| **DEX** | P.Atk speed, physical critical rate, run speed, accuracy and evasion (via √DEX) | shield block rate (Phase 3) |
| **CON** | Max HP, Max CP, HP regen, CP regen, weight limit | stun/bleed resistance in retail ("shock resistance"), breath |
| **INT** | M.Atk (squared) | debuff land rate in later chronicles |
| **WIT** | Casting speed, magic critical rate | magic resist / hold resistance |
| **MEN** | M.Def, Max MP, MP regen | mental debuff resistance, magic cancel resistance |

Every bonus is a dimensionless multiplier around 1.0 at a "neutral" value. L2J Interlude ships them
as 100-entry arrays in `stats/statBonus.xml`; the XML comments give the generating formulas:

```
STR:  1.036 ^ (STR − 34.845)
INT:  1.020 ^ (INT − 31.375)
CON:  1.030 ^ (CON − 27.632)
MEN:  1.010 ^ (MEN + 0.060)
DEX:  1.009 ^ (DEX − 19.360)
WIT:  1.050 ^ (WIT − 20.000)
```

Values are capped at `MAX_STAT_VALUE = 100` (index clamp). Rounded to two decimals, the table every 5
points (this reproduces the XML to within rounding; e.g. the XML has STR 40 → 1.20, 60 → 2.43,
99 → 9.67; WIT 50 → 4.32; CON 43 → 1.57):

| stat | 5 | 10 | 15 | 20 | 25 | 30 | 35 | 40 | 45 | 50 | 55 | 60 | 65 | 70 | 75 | 80 | 85 | 90 | 95 |
|------|---|----|----|----|----|----|----|----|----|----|----|----|----|----|----|----|----|----|----|
| STR | 0.35 | 0.42 | 0.50 | 0.59 | 0.71 | 0.84 | 1.01 | 1.20 | 1.43 | 1.71 | 2.04 | 2.43 | 2.91 | 3.47 | 4.14 | 4.94 | 5.89 | 7.03 | 8.39 |
| INT | 0.59 | 0.65 | 0.72 | 0.80 | 0.88 | 0.97 | 1.07 | 1.19 | 1.31 | 1.45 | 1.60 | 1.76 | 1.95 | 2.15 | 2.37 | 2.62 | 2.89 | 3.19 | 3.53 |
| CON | 0.51 | 0.59 | 0.69 | 0.80 | 0.93 | 1.07 | 1.24 | 1.44 | 1.67 | 1.94 | 2.25 | 2.60 | 3.02 | 3.50 | 4.06 | 4.70 | 5.45 | 6.32 | 7.33 |
| MEN | 1.05 | 1.11 | 1.16 | 1.22 | 1.28 | 1.35 | 1.42 | 1.49 | 1.57 | 1.65 | 1.73 | 1.82 | 1.91 | 2.01 | 2.11 | 2.22 | 2.33 | 2.45 | 2.58 |
| DEX | 0.88 | 0.92 | 0.96 | 1.01 | 1.05 | 1.10 | 1.15 | 1.20 | 1.26 | 1.32 | 1.38 | 1.44 | 1.51 | 1.57 | 1.65 | 1.72 | 1.80 | 1.88 | 1.97 |
| WIT | 0.48 | 0.61 | 0.78 | 1.00 | 1.28 | 1.63 | 2.08 | 2.65 | 3.39 | 4.32 | 5.52 | 7.04 | 8.99 | 11.47 | 14.64 | 18.68 | 23.84 | 30.43 | 38.83 |

Observations that matter for tuning: WIT is by far the steepest curve (×5 between 20 and 55), which is
why L2 mages feel "fast" at ~40 WIT; MEN is nearly linear and never below 1.0; DEX is the flattest
(a +10 DEX buff is only ~+9% attack speed).

The High Five branch rebased the stats (every class got ~+45 to its values, e.g. Human Fighter STR 88
instead of 40) and replaced the arrays with closed forms, e.g. `STR: 1.009^(STR−49)`,
`INT: 1.01^(INT−49.4)`, `DEX: 1.00456^(DEX−19.27)`, `WIT: 1.01383^(WIT−64.57)`,
`CON: 1.01169^(CON−34.80)`, `MEN: 1.00369^(MEN+30.45)`. Same shape, flatter slopes, shifted
neutral points. We use the Interlude curves (steeper, more legible) with Interlude base values.

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

Base stats in L2 are **fixed per class** (every Human Fighter has 40 STR for life); only dyes
(henna), buffs, and later-chronicle items change them. The current `grpc.rs` stub already returns the
Human Fighter row.

### 2.3 Level modifier

Almost every derived stat scales with level through

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
| Accuracy | `√DEX × 5 + level (+1 at 78+, +2 at 81+, +2 at 88+ …) + weapon accuracy` | `FuncAtkAccuracy` (comment says "[Square(DEX)] * 5 + lvl") |
| Evasion | `√DEX × 5 + level (+ (level−69) above 69 for players)` | `FuncAtkEvasion` |
| P. critical rate | `baseCrit × DEXmod` (Interlude baseCrit 40–46) or `baseCrit × DEXmod × 10` (HF baseCrit 4) — both land at ≈ 40–50 per mille | `FuncAtkCritical`; cap `MaxPCritRate = 500` |
| M. critical rate | `baseMCrit(5) × WITmod × 10` per mille | `FuncMAtkCritical`; cap `MaxMCritRate = 200` |
| P. attack speed | `basePAtkSpd(300 fighter / 240 HF mystic) × DEXmod` | `FuncPAtkSpeed`; cap 1500 |
| Casting speed | `baseMAtkSpd(333) × WITmod` | `FuncMAtkSpeed`; cap 1999 |
| Run speed | `baseRunSpd × DEXmod` | `FuncMoveSpeed`; cap `MaxRunSpeed = 300` (HF) |
| Weight limit | `floor(CONmod × 69000)`, clamped: CON < 1 → 31000, CON > 59 → 176000 | Interlude `getMaxLoad()` |

Crit and m-crit are **per mille**: `calcCrit` rolls `rate > Rnd.get(1000)`. A Human Fighter with
DEX 30 has rate 44 × 1.10 = 48 → 4.8 % crit. Physical crits deal ×2 damage; magic crits ×3 vs
monsters, ×2 vs players (HF).

**Hit chance** (`Formulas.calcHitMiss`):

```
chance‰ = clamp((80 + 2 × (attackerAccuracy − targetEvasion)) × 10 × conditionBonus, 200, 980)
miss    = chance < Rnd(1000)
```

so equal accuracy/evasion gives 80 % to hit, each point of difference is ±2 %, floor 20 %, ceiling
98 %. `hitConditionBonus.xml`: attacking from behind +10 %, from the side +5 %, from high ground +3 %,
from low ground −3 %, at night −10 %, in rain −3 %.

**Attack interval** (`L2Character.calculateTimeBetweenAttacks` / `Formulas.calcPAtkSpd`):

```
swing_ms   = 470000 / pAtkSpd          (HF; older branches 500000; minimum rate 2 → 2700 ms)
bow_ms     = 1500 × 345 / pAtkSpd      crossbow 1200 × 345 / pAtkSpd
bow_reuse  = weaponReuse × 345 / pAtkSpd
cast_ms    = skillTime × 333 / mAtkSpd (physical skills: skillTime × 300 / pAtkSpd)
```

300 atk speed → 1567 ms per swing; 500 → 940 ms; 1500 (cap) → 313 ms.

**Weight penalty** (`refreshOverloaded`): load fraction `w = currentLoad / maxLoad`; `w < 50 %` no
penalty; 50–66.6 % level 1; 66.6–80 % level 2; 80–100 % level 3; ≥ 100 % level 4 (cannot move/attack
in retail). Penalty levels map to skill 4270 which reduces speed and attack speed.

### 2.5 HP / MP / CP per level

High Five ships an explicit per-level table per class (`lvlUpgainData`). Fitting
`value(L) = base + a·(L−1) + b·(L−1)²` reproduces every one of the 99 entries with zero error, so the
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

Spot values (Human Fighter, before CON): L1 80 HP / 30 MP; L20 327 / 144; L40 637.7 / 287.4;
L60 1000.4 / 454.8; L76 1328 / 606. Mystics have *more* raw HP than fighters; the gap comes from CON.

Regeneration per tick (same for all nine starters, HF table): HP 2.0 at L1, +0.05/level to 2.95 at
L20, then `1.4 + 0.1 × L` (3.4 at 20, 5.4 at 40, 7.4 at 60, 9.0 at 76, 9.4 at 80). MP 0.9 through
L10 then `0.9 + 0.03 × (L−10)` capped at 3.0 (1.2 at 20, 1.8 at 40, 2.4 at 60). CP 2.0 through L10,
then `0.5 + 0.1 × L` (2.5 at 20, 4.5 at 40, 6.5 at 60) capped 8.5 at 76+. The Interlude branch instead
stores `baseHpMax, levelHpAdd, levelHpMod` per class (Human Fighter 80.0 / 11.83 / 0.37) and derives
the curve in code; the HF table is the cleaner source and is what we fit.

### 2.6 Experience table

Cumulative XP required to **reach** each level (`experience.xml`, `tolevel`). Interlude (max 80) and
High Five (max 85 for mains) agree to level 20 and diverge after (HF was re-tuned to be much steeper
in the 40s and 70s). Both are reproduced; Nightfall uses the Interlude column.

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
85 → 5,008,025,097 (then a ×2 jump per level to 99 for the Awakening content). The Interlude file lists
81–87 as a geometric 1.5×/1.4×/1.3×… extension used only for the subclass/GM cap. Note the Interlude
"wall": 76→77 costs 220 M, 77→78 360 M, 78→79 588 M, 79→80 2.1 B — level 80 was intended to be
near-unreachable.

### 2.7 XP loss on death

Interlude (`PlayerInstance.deathPenalty`): a flat percentage of the **span of the current level**
(`expForLevel(L+1) − expForLevel(L)`), not of total XP:

| Level | % of current level lost |
|-------|-------------------------|
| 1–19 | 10 % |
| 20–39 | 7 % |
| 40–74 | 4 % |
| 75–80 | 2 % |

Modifiers: ÷4 if killed during clan war, siege, or festival; × `RateKarmaExpLost` (default 1) when the
victim has karma; zero in peace zones and events. High Five replaced the brackets with a per-level
table (`playerXpPercentLost.xml`): 10.0 % at L1 decreasing by 0.125 per level to 4.0 % at L49, flat
4.0 % through L75, then 2.5 (76), 2.0 (77), 1.5 (78), 1.0 % at 79+. Classic (2015 relaunch) went the
other way: a flat 10 % at all levels, level loss possible from level 10 up, with items droppable on
PvE death; the 2016 Classic 1.5 notes list 4 %. Later chronicles (GoD+) remove level loss entirely and
add the Shilen's Breath debuff instead. The losing-a-level rule in L2J: XP can drop below the level
floor and the character de-levels (`AltGameDelevel`), down to level 1.

### 2.8 Party XP

`L2Party.distributeXpAndSp`:

1. Determine **valid members**: those within the level cutoff relative to the highest-level member
   (`topLvl`). Methods: `level` (gap ≤ `PartyXpCutoffLevel` = 20), `percentage` (member's level² must
   be ≥ 3 % of the sum of squares), `auto`, or the retail-accurate `highfive` gaps
   `0–9 → 100 %`, `10–14 → 30 %`, `15+ → 0 %` of the member's computed share.
2. Multiply the mob's XP/SP by the **party bonus** for the number of valid members:

| Members | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |
|---------|---|---|---|---|---|---|---|---|---|
| Interlude | 1.00 | 1.30 | 1.39 | 1.50 | 1.54 | 1.58 | 1.63 | 1.67 | 1.71 |
| High Five | 1.00 | 1.10 | 1.20 | 1.30 | 1.40 | 1.50 | 2.00 | 2.10 | 2.20 |
| Community (Classic) | 1.0 | 1.3 | 1.4 | 1.5 | 1.6 | 1.7 | 2.0 | — | — |

3. Split by **level squared**: member share = `level² / Σ level²` of valid members. A level 40 next to
   a level 20 takes 80 % of the pool. Dead members get nothing; a servitor with an XP penalty scales
   its owner's share.

### 2.9 Level difference vs monsters

In L2 a monster's XP reward is a fixed value on the NPC template; there is **no XP scaling by level
difference** in Interlude/HF code — the gating is indirect, through damage and hit chance. HF
(`NPC.properties`, Gracia Epilogue rule) applies penalties when a monster of level ≥ 78 is above you:
damage ×0.7 / 0.6 / 0.6 / 0.55 for 2 / 3 / 4 / 5 levels below the mob (crits 0.75 / 0.65 / 0.6 / 0.58;
skills 0.8 / 0.7 / 0.65 / 0.62), and magic land chance is divided by 2.5 / 3.0 / 3.25 / 3.5 at 3–6
levels below. Magic success also includes `lvlModifier = 1.3^(targetLevel − casterLevel)` in
`calcMagicSuccess`, so every level the target has over you cuts land rate by 30 %. Private servers and
Classic-era community guides describe explicit XP tables (e.g. "|Δ| < 3 → 100 %, 3–10 → 70 %,
11–14 → 10 %, 15+ → 0 %" or "0–5 → 100 %, 6–9 → 30 %, 10+ → 0 %"); treat those as fan rules, not
retail.

---

## 3. Design decisions for Nightfall

### 3.1 Keep the six stats and the Interlude multiplier curves

Reasons: the curves are tiny closed forms, reproduce the retail tables, and give meaningful knobs
(a +4 STR dye is visibly +15 % P.Atk). Stat values are `u8` clamped to `1..=99`. Nightfall's curves
are the Interlude formulas verbatim:

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

Precompute into a `[[f32; 100]; 6]` table at boot so combat code does lookups, not `powf`.

### 3.2 Level modifier, caps, and starting values

- `level_mod(L) = (L + 89) / 100`, level cap **80**, XP table = Interlude column (data file).
- Starting base stats, base P/M.Atk, base P/M.Def, speeds, base crit = the Interlude table in §2.2,
  stored in `classes/*.toml`. Base stats never change with level in Nightfall either; dyes/buffs add.
- Hard caps (from HF `Character.properties`, kept): run speed 300, P.Atk speed 1500, cast speed 1999,
  P.crit 500 ‰, M.crit 200 ‰, evasion 250. Hit chance floor 20 %, ceiling 98 %.

### 3.3 Derived stat formulas (normative)

Let `S(x) = stat_bonus(x, value)`, `LM = level_mod(level)`, `L = level`. Equipment contributions are
summed into `gear` (Phase 4) and buffs into `mods` (Phase 3) as `(add, mul)` pairs applied as
`(base_result + add) * mul` at the end.

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

Regen tables are the piecewise-linear HF curves from §2.5, shipped as data. Posture multipliers are
applied per 3-second regen tick, matching L2's regen task period.

**Worked example (Human Fighter, no gear, Interlude stats 40/30/43/21/11/25):**

| L | LM | HP | MP | CP | P.Atk | M.Atk | P.Def | M.Def | Acc | Crit‰ | Atk spd | Cast spd | Run | Weight |
|---|----|----|----|----|-------|-------|-------|-------|-----|-------|---------|----------|-----|--------|
| 1 | 0.90 | 126 | 38 | 50 | 4.3 | 3.2 | 72 | 47 | 28 | 48 | 330 | 215 | 127 | 108,675 |
| 20 | 1.09 | 515 | 185 | 206 | 5.2 | 4.7 | 87 | 57 | 47 | 48 | 330 | 215 | 127 | 108,675 |
| 40 | 1.29 | 1004 | 369 | 402 | 6.2 | 6.6 | 103 | 68 | 67 | 48 | 330 | 215 | 127 | 108,675 |
| 76 | 1.65 | 2092 | 778 | 837 | 7.9 | 10.8 | 132 | 87 | 103 | 48 | 330 | 215 | 127 | 108,675 |

Human Mystic at L1 for contrast: M.Atk 7.1 (INT 41 → 1.19² × 0.81), cast speed 333 (WIT 20 → 1.00),
MP 59, HP 99. The "fists" P.Atk of ~4 is why L2 starting weapons (P.Atk 8–15) matter: weapon P.Atk
dominates `p_atk` until level 40 gear.

### 3.4 Experience rules

- **Gain**: `xp = npc.xp * level_gap_mult(player, npc) * rates.xp` where Nightfall **does** add an
  explicit XP level-gap multiplier (L2 only penalises indirectly, which is opaque to players):

| `npc.level − player.level` | multiplier |
|----------------------------|------------|
| ≥ +6 | 1.00 (plus the damage/land-rate penalties below) |
| −5 … +5 | 1.00 |
| −6 … −8 | 0.70 |
| −9 … −10 | 0.40 |
| −11 … −14 | 0.10 |
| ≤ −15 | 0.00 |

- **Over-level penalties** (from HF, applied to all monsters, not just L78+): when `npc.level −
  player.level ≥ 2`, outgoing damage ×0.7/0.6/0.6/0.55 at Δ 2/3/4/5+ (crits 0.75/0.65/0.6/0.58,
  skills 0.8/0.7/0.65/0.62); magic land chance `× 1 / 1.3^Δ` for Δ ≥ 1 (the `calcMagicSuccess` term).
- **Death**: Interlude brackets — 10 % (L1–19), 7 % (20–39), 4 % (40–74), 2 % (75–80) of the current
  level's XP span; ÷4 in clan war and siege zones; 0 in peace zones and instanced events; de-leveling
  allowed down to level 1 XP floor but never below level 10 for characters that reached 10 (a Classic
  rule we adopt to protect newcomers). Death also spends the Shilen's Breath-style debuff system
  (Phase 3), not XP, for repeat deaths.
- **Party**: valid members by HF gaps (0–9 → 100 %, 10–14 → 30 %, 15+ → 0 %) measured from the
  highest-level member; bonus by valid-member count = Interlude table (1.00, 1.30, 1.39, 1.50, 1.54,
  1.58, 1.63, 1.67, 1.71); share by `level² / Σlevel²`. Members must be within the killer's 3 × 3
  interest block (Phase 0 cell grid, ≈ 1500 units, close to L2's 1500-unit party-reward range).
- All constants live in `tables/experience.toml` and `tables/penalties.toml` so they are tunable
  without a rebuild.

### 3.5 Alternatives considered

- *Linear stat bonuses* ("+1 STR = +1 % P.Atk") — simpler but makes dyes/buffs uninteresting and
  breaks parity with L2 build intuition. Rejected.
- *HF rebased stats* (STR 88 etc.) — flatter curves, bigger numbers, no benefit for a new game.
- *XP scaling by formula instead of table* — the Interlude table is irregular on purpose (plateaus at
  20/40/76 class-change levels); keep it as data.
- *Percent-of-total XP death penalty* — punishes high levels far more than L2 intends; keep
  percent-of-level-span.

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

`derive` is pure; tests pin the §3.3 worked-example rows to two decimals so refactors cannot drift.

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

Derived stats are **sent, not computed on the client**: the client has no gear/effect data model yet
and must never disagree with the server's numbers.

---

## 5. Interfaces

- `GameService.GetCharacter` now fills `class_id`, `xp`, `xp_to_next`, `derived`, `vitals`.
- New server-side events on the world bus: `StatsRecomputed{entity, Stats}` (emitted by the stats
  system when level/base/gear/effects change; replication turns it into a `DerivedStats` delta for
  the owner and an `EntityDelta{max_hp}` for observers), `XpChanged{char, xp, delta, reason}`,
  `LevelChanged{char, from, to}`.
- `WorldDelta.EntityDelta` gains optional `hp`, `max_hp`, `cp`, `max_cp` for all visible entities,
  and `mp`, `max_mp`, `xp`, `level` for the owner only.
- Admin: `POST /admin/stats/recompute` forces `derive` for all online characters after a data
  hot-reload changes a curve.

---

## 6. Rust implementation notes

- Module `stats/` (see Phase 0 layout): `bonus.rs` (curves + cached `[[f32;100];6]`),
  `derive.rs` (`derive`, caps), `xp.rs` (`XpTable`, death/party/gap functions), `regen.rs`
  (3-second regen task per active cell), `tests.rs`.
- `Stats` is a plain `Copy` struct stored on the entity; recompute is event-driven, never per tick.
  Recompute cost is ~30 multiplies — fine to do on every equip change for 5k entities.
- All randomness in combat uses one `rand::rngs::SmallRng` per world thread seeded from config so
  replays are deterministic; `hit_chance_permille` and crit rolls take `&mut impl Rng`.
- XP is `u64`; the Interlude table tops out at 4.2 × 10⁹ which already overflows `i32` (L2J uses
  `long`). Postgres column is `BIGINT`.
- Floating point: use `f32` for stats, round to integer only at the API boundary (`as u32` after
  `.round()`), and compare in tests with `approx::assert_relative_eq!(…, epsilon = 0.01)`.
- Data validation at load: every `Curve` strictly increasing over 1..=80, `cp_ratio ∈ [0.3, 1.0]`,
  XP table strictly increasing with `to_level[1] == 0`, all penalty tables sorted by key.

---

## 7. Client implications

- Character sheet reads `DerivedStats` + `BaseStats` straight from the server; it shows base stats in
  white and the dye/buff delta in green/red (L2 convention) using `base_stats` vs `stats`.
- Movement uses `derived.run_speed` as units/second; the client's dead reckoning in Phase 0 relies on
  this being the exact value the server integrates with.
- Attack and cast bars are driven by server-sent `swing_ms`/`cast_ms` in the action start message
  (Phase 3); the client does not recompute them from attack speed.
- HP/CP bars for other entities come from `EntityDelta`; MP and XP are owner-only. The XP bar is
  `(xp − table[level]) / xp_to_next_span`; the client receives both cumulative `xp` and
  `xp_to_next` so it needs no table.
- Weight: show the four penalty thresholds at 50 / 66.6 / 80 / 100 % of `weight_max`.

---

## 8. Open questions

1. **Crit base**: Interlude per-class `baseCritRate` (40–46) vs HF `4 × 10`. We use the Interlude
   per-class value; confirm against feel once weapons add crit (Phase 4).
2. **Mystic P.Atk speed**: Interlude lists 300 for mystics, HF 240. Using 300 (Interlude parity); HF
   240 could become a class-tree knob in Phase 2.
3. **Regen tick period**: 3 s per L2; a 1 s tick with a third of the value is smoother for the HP
   bar. Decide in Phase 3 together with potion ticks.
4. **Whether to expose "shock/mental resistance" from CON/MEN** as in retail — depends on the status
   effect model (Phase 3).
5. **De-level floor**: we picked level 10 as a floor for anyone who reached it; L2J allows de-level to
   1. Revisit with the death-debuff design.
6. **XP table beyond 80**: none planned; if a cap raise is needed later, extend with the Interlude
   1.5× extension entries already in the file (81 → 6.3 B …).
7. Interlude's parametric HP formula (`baseHpMax, levelHpAdd, levelHpMod`) was not reproduced; we
   fit the HF per-level table instead. If an Interlude-exact curve is wanted, derive it from the C6
   `FuncMaxHpAdd` source.

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
- Lineage II Classic 1.5 patch notes (4 % XP loss on death) via <https://www.lineage2.com/news/lineage-ii-classic-launch-patch-notes>.
