# Data sources

Provenance for the generated stat tables (Phase 1 plan §2 fidelity gate, Story E1.1).
`tables/*.toml` and `classes/*.toml` are generated; `zones/` and `npcs/` are hand-written.

## Pinned High Five source

| Repository | Revision | Role |
|---|---|---|
| <https://bitbucket.org/l2jserver/l2j-server-game> (`develop`; GitHub mirror `monkey1sai/l2j-server-game`) | `abfde0490ac52a2106e884b7242c0276c44cdf76` (2026-09-13) | Formulas, functions, config defaults |
| <https://bitbucket.org/l2jserver/l2j-server-datapack> (`develop`) | `3ca488dd2bd0bfaca43e378886a3c2e37968153a` (2026-09-13) | Tables and templates |

Both READMEs: "develop branch is the current High Five development" (client: High Five part 5).

Files read (game prefix `src/main/java/com/l2jserver/gameserver/`, datapack prefix
`src/main/resources/data/`):

- Game: `model/stats/Formulas.java` (`calcPhysDam`, `calcHitMiss`, `calcCrit`),
  `model/stats/BaseStats.java`, `model/stats/functions/formulas/Func{PAtkMod,PDefMod,MaxHpMul,MaxMpMul,AtkAccuracy,AtkEvasion,AtkCritical,PAtkSpeed}.java`,
  `model/stats/functions/StatFunction.java` (`SET` order 0), `model/actor/L2Character.java`
  (`getLevelMod`, `calculateTimeBetweenAttacks`, `doAttack`, `getRandomDamageMultiplier`,
  `doRevive`), `model/actor/stat/{CharStat,PcStat,PlayableStat}.java`,
  `model/actor/L2Attackable.java`, `model/AggroInfo.java`,
  `model/actor/instance/L2PcInstance.java` (`calculateDeathExpPenalty`),
  `data/xml/impl/PlayerTemplateData.java`, `src/main/resources/config/character.properties`.
- Datapack: `stats/statBonus.xml`, `stats/expData.json`, `stats/chars/playerXpPercentLost.xml`,
  `stats/chars/baseStats/{Human,Elven,Dark,Orc,Dwarven}{Fighter,Mystic}.xml` (no Dwarven mystic),
  `stats/initialEquipment.xml`, `stats/items/02300-02399.xml` (item 2369).

## Regenerating

```bash
git clone -b develop https://bitbucket.org/l2jserver/l2j-server-game.git /tmp/l2j-game
git -C /tmp/l2j-game checkout abfde0490ac52a2106e884b7242c0276c44cdf76
git clone -b develop https://bitbucket.org/l2jserver/l2j-server-datapack.git /tmp/l2j-dp
git -C /tmp/l2j-dp checkout 3ca488dd2bd0bfaca43e378886a3c2e37968153a
python3 packages/data/scripts/gen_tables.py --game /tmp/l2j-game --datapack /tmp/l2j-dp --check
```

The script refuses other revisions, uses `decimal`/`fractions` only, parses every constant
from a literal source line that must exist verbatim, and proves each class HP/MP quadratic
reproduces all 85 source rows. `--check` fails on any drift from the committed files.

## Fidelity ledger (resolved)

| Item | Resolved value | Source |
|---|---|---|
| Normal-attack damage coefficient | **76**; crit ×2 | `Formulas.calcPhysDam`: `damage = (76 * damage * proximityBonus) / defence;` |
| Attack interval | **500000 / pAtkSpd** ms, impact at half | `L2Character.calculateTimeBetweenAttacks`, `doAttack` (`timeAtk / 2`) |
| Accuracy | **`sqrt(DEX) * 6 + L`**, `+(L−69)` above 69, `+(L−76)` above 77 | `FuncAtkAccuracy` |
| Evasion (player) | **`sqrt(DEX) * 6 + L`**, `+(L−69)` from 70, ×1.2 on that addition from 78; cap 250 | `FuncAtkEvasion`, `MaxEvasion` |
| Stat bonuses | **`statBonus.xml` 2-decimal tables**, indices 0–99; comments `1.036^(STR−34.845)`, `1.020^(INT−31.375)`, `1.030^(CON−27.632)`, `1.010^(MEN+0.060)`, `1.009^(DEX−19.360)`, `1.050^(WIT−20.000)` | `BaseStats` static loader |
| Class HP/MP | Per-level `lvlUpgainData` rows 1–85; exact quadratics (e.g. Human Fighter `80 + 11.765n + 0.065n²`, `30 + 5.43n + 0.03n²`) | `baseStats/*.xml` |
| XP table | `expData.json` levels 1–85, **sentinel L86 = 16,890,558,728**; cap `X[86] − 1` | `ExperienceData`, `PlayableStat.addExp` |
| Death loss | 10.0 % at L1, −0.125/level to 4.0 % at L49, 4.0 % to L75, 2.5/2.0/1.5 % at L76/77/78, 1.0 % L79–85 | `playerXpPercentLost.xml` |
| Starter weapon | Squire's Sword: P.Atk 6, crit 8, speed 379, random 10, range 40, accuracy 0 | item 2369 |
| Caps / respawn | crit 500 ‰, speed 1500, hit 200–980 ‰, HP 0.65 / MP 0, protection 600 s, max level 85 | `character.properties` |

## Errata against the planning docs

- **E-1 Stat curves.** `01-stat-formulas.md` §2.1 calls `1.009^(STR−49)` … `1.00369^(MEN+30.45)`
  and Human Fighter STR 88 "High Five". They come from `andridgitalbox/l2j-mobius`
  (`d2a8c5d5`, an Ertheia-era tree: LUC/CHA, Ertheia classes). HF uses the tables whose
  comments the doc labels "Interlude". Plan vector 1 is re-based on HF (STR 40 = 1.20).
- **E-2 XP table.** The doc's "High Five" column (L41 8,718,976; L85 5,008,025,097) is the
  Mobius/Ertheia table. HF `expData.json` matches the doc's "Interlude" column only to L10 and
  L76–78 (e.g. L11 71,202 vs 71,201; L80 3,075,966,164; L85 13,180,481,103).
- **E-3 Accuracy/evasion.** HF uses `* 6` and the level additions above, not stats §2.4's
  `* 5` / "+1 at 78+, +2 at 81+". Combat §2.3's `* 6` is right; its additions are imprecise.
- **E-4 CON 43** is 1.58 in the HF table (the doc prints 1.57).
- **E-5 Weapon stats replace, not add.** Item stats are `<set>` funcs (order 0), applied before
  the STR/DEX/level multipliers (order 1), so P.Atk = `weapon × STR × LM`, not
  `(fist + weapon) × …`; crit base and attack speed are likewise replaced (8, 379).
- **E-6 Spawn protection** is `PlayerSpawnProtection = 600` **seconds** (6000 ticks), not
  600 ticks.
- **E-7 Rounding (reviewed E1.4).** The earlier claim that every difference was ≤1 unit
  was false for hit chance and attack milliseconds. Nightfall now reproduces source
  `Math.round` at accuracy/evasion, attack speed and death loss using integer arithmetic,
  and both integer millisecond casts in attack scheduling. HP/MP retain exact decimal
  rows under D3. See the measured ledger below; crit's `(int)` **before** `+0.5` does
  not round a fractional crit rate up.
- **E-8 The "77".** HF's normal attack uses 76; 77 appears only in the crit additive term
  (`CRITICAL_DAMAGE_ADD * 77 / defence`, zero without buffs) and in skill/blow damage.
- **E-9 Interval.** Stats §2.4 prints 470000 as HF; HF is 500000. The 470000 value was not
  verified against an Interlude source here.
- **E-10 Misc, unused in Phase 1.** `MaxRunSpeed = 250` (doc: 300); `baseMDef` sums the right
  ring twice (`PlayerTemplateData`); fists use random radius `5 + (int) sqrt(level)` (template
  `baseRndDam` unused for players); an attack dealing 0 hate to a player-attacked mob adds 1.
- **E-11 Table index 0.** CON[0] 0.45 and WIT[0] 0.39 differ from their comment expressions
  (0.44, 0.38); the table is authoritative and index 0 is unreachable (base stats ≥ 1).

## Missing rows

- Kamael starters (class 123/124) are omitted: Nightfall has no Kamael race.
- XP row 87 exists in the source and is not used (only the L86 sentinel is).
- Nothing in the E1.1 ledger was left unsourced.

## E1.4 independent fidelity review

The test-only [oracle](scripts/oracle.py) uses Python `Fraction`, integer square-root
bounds and integer IEEE ties-even emulation for the resource-storage audit. It imports
neither `gen_tables.py` nor Rust, reads literal `bonus`, HP/MP `levels`, weapon stats,
XP and percentage rows, and never reads `_q` values or fitted quadratic coefficients.
Its source expressions are transcribed independently and guarded against literal lines
in `formulas.toml`. Static bonuses use the authoritative XML values, including E-11;
they are not reinterpreted as exponential curves. Production has no new dependency.

```bash
python3 packages/data/scripts/oracle.py
python3 packages/data/scripts/oracle.py --check
cargo test --test formula_fidelity
```

The checked-in [golden](fixtures/oracle.json) stores independent per-axis results; Rust
assembles expected sheets by lookup only. Input SHA-256 checks prevent stale fixtures
when any table, class, NPC target or oracle script changes. The accompanying
[rounding audit](fixtures/rounding-audit.json) has exact maximum differences and witnesses.
It compares the pre-E1.4 contract with source semantics; it is not a tolerance file.

Coverage:

- All **600** stat bonus rows (six tables, indices 0–99), all **85** LM and accuracy/evasion
  addition rows, all **86** XP thresholds including the sentinel, every death-loss row.
- All **765** original class/level profiles with sword and fists; each class/level also
  uses the full Cartesian DEX/STR/CON grid **{1,9,25,30,43,64,99}³**, both armed and bare:
  **524,790** grid sheets. Another **75,735** armed sheets exhaust all 99 legal values on
  each attribute axis together. Total **602,055** deterministic sheet comparisons.
- HP/MP quadratics checked against all **1,530 original resource rows**. DEX source
  rounding checked at every index 0–99 and level 1–85; bounded square roots certify the
  rounded result independently of production's Q approximation.
- **32,130** normal/critical damage vectors (nine profiles × 85 levels × 21 spreads × two
  crit states) against the literal Keltir defence, with hate; **510** hate vectors across
  all levels including zero and maximum `u32` damage; hate cap boundaries.
- **700** speed/timing vectors including synthetic speed-cap inputs, **600** crit/cap
  vectors, every roll 0–999 for fractional hit boundaries and all supported DEX crit rates.
  Threshold ±1, each death/delevel boundary, XP saturation and sentinel are checked.
- Two **1,024-case** property tests vary profiles, levels, all legal DEX/STR/CON values,
  equipment, opponent levels/DEX, rolls, XP and rewards against those independent results.
  Ertheia P.Atk, Interlude base-44 crit and the docs' CON=1.57 resource fixture remain
  explicitly labelled legacy/fixture inputs, separate from production HF profiles.

The rounding sites were re-read in pinned game revision `abfde0490ac52a2106e884b7242c0276c44cdf76`:
[CharStat.java](https://raw.githubusercontent.com/monkey1sai/l2j-server-game/abfde0490ac52a2106e884b7242c0276c44cdf76/src/main/java/com/l2jserver/gameserver/model/actor/stat/CharStat.java)
`getAccuracy` and `getEvasionRate` use `(int) Math.round(calcStat(...))`;
`getPAtkSpd` returns `Math.round(calcStat(...))`; `getCriticalHit` casts to int before
adding `.5`. `PcStat.getPAtkSpd` caps the rounded result; `PcStat.getMaxHp/getMaxMp`
cast the resource product to int. The interval and death-loss statements are retained
verbatim in `formulas.toml` and `penalties.toml` respectively.

| Quantity/site | Maximum old-contract difference | Disposition / witness |
|---|---|---|
| Bonus lookups, LM, P.Atk/P.Def | 0 against exact literal expressions | All products for these source inputs are exactly representable at Q; unchanged. |
| Accuracy `Math.round` | 0.497188 stat units | Fixed: DEX 55, L1 was 45.497188, now 45. |
| Evasion `Math.round` | 0.499884 stat units | Fixed: DEX 56, L82 was 142.499884, now 142. |
| Hit chance after those getters | **19‰** (19 roll outcomes out of 1000) | Fixed: same-level DEX 26 attacker / DEX 21 target was 861‰, now 880‰. Source inclusive `roll <= chance` retained. The fractional rounding-error bound over all DEX pairs and all five evasion fractional additions is also 19‰, including differing levels. |
| Crit `(int)` then `+0.5` | **0‰** | Unchanged floor; the integer cast makes the subsequent half ineffective. Rational binary64 emulation also finds no rate difference for sword/fists across all DEX indices. |
| Attack speed `Math.round` | 0.49 speed units | Fixed: sword DEX 78 was 640.51, now 641. |
| Attack interval integer cast, including speed rounding | **46261/19329 ms** (≈2.39335 ms) | Fixed: sword DEX 22, raw 25000000/19329 ms versus source 1291 ms. |
| Half-interval integer division, including preceding operations | **32795/19329 ms** (≈1.69667 ms) | Fixed: same witness, source impact 645 ms. |
| Impact/cycle tick deadlines | **1 tick** each | Fixed: fist DEX 39 impact was +8, now +7; fist DEX 17 cycle was +18, now +17. Tick ceiling is Nightfall's explicit 100 ms scheduling boundary. |
| Death loss `Math.round` | **1 XP**, potentially **1 level** | Fixed: L5 loss 299→300; XP 3183 now becomes 2883/L4 instead of 2884/L5. L1 loss 6→7 cannot delevel below 1. |
| HP from Java binary32 rows (then binary64 product and int cast) | **1 HP** on extended CON axis; **0** for all original profiles | Retain D3 exact-decimal floor: Dark Fighter L40, CON 79 gives 3363 versus Java 3362. No level/hit outcome changes. |
| MP from Java binary32 rows (then binary64 product and int cast) | **1 MP** on extended MEN axis; **0** for all original profiles | Retain D3 exact-decimal floor: Dark Fighter L36, MEN 22 gives 321 versus Java 320. No level/hit outcome changes. |
| Hate, XP thresholds/cap | **0** against rational source expressions | Unchanged; no rounding-site discrepancy. |

The grid maxima are measured over all table indices (not just the seven-value Cartesian
grid). Resource audit extends MEN as well as CON to all indices. The retained one-resource
unit differences are explicitly permitted by the E1.4 gate; this is rational formula
fidelity, not emulation of every Java binary64 intermediate in the combat pipeline.

The decimal-parser consolidation also resolves two inconsistent admission rules: NPC
strings ending in `.` now fail the shared plain-decimal grammar, and extra trailing zeros
are accepted when the number is exactly representable at Q (e.g. `0.1234560`). Both loaders
now accept the entire signed Q range including `-9223372036854.775808`; out-of-range
neighbours still fail. NPC TOML float literals remain rejected. The shared module contains
the union of both parser suites plus exact signed endpoint tests.
