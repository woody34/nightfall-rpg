# Data sources

This file records the provenance of the generated stat tables (Phase 1 plan §2 fidelity gate, Story E1.1).
The generator makes `tables/*.toml` and `classes/*.toml`. A person writes `zones/` and `npcs/` by hand.

## Phase 3 peace geometry (Nightfall configuration)

`zones/test_zone_phase3.toml` is a hand-written, schema-1 additive input. Its
`[provenance].kind = "Nightfall configuration"` shows that the values are architect policy
and not pinned L2J values. Architecture decisions §6 R7 / §7 D-R11 set a configurable
inclusive square. The half-width of the square is **10,000 milli-tiles**, and its centre is the legacy
safe point **(126,126)** (`zones/test_zone.toml:33`). Its inclusive edges are
**(116,116)..(136,136)** whole tiles, and the square includes the Class Master **(126,128)**
(`zones/test_zone.toml:28`). The legacy bounds are **(0,0)..(256,256)** (`:9–10`).

Schema: the top level has `schema_version = 1` and `id = "test_zone"`. `[peace_zone]`
has `shape = "square"`, `centre = [126,126]` in whole tiles,
`half_width_millitiles = 10000` and `inclusive = true`. `[provenance]` has the
`kind`, `decision` and `safe_point` citations. The width is configurable. It is not a
constant that comes from the source. `check_all.py` parses the two files and makes sure that the centre is
equal to safe_point. It validates the integer units, the containment in the bounds and the inclusive
containment of the Class Master. Then it prints the SHA-256 of all the bytes of the new file.

The engine must include this separate input in the Phase3Rules hashing. The engine must also embed it
in its Phase 3 input inventory. Do not include it in the legacy zone config hash.
Do not load it as a legacy zone file. The bytes of `zones/test_zone.toml` and the historical
oracle manifests do not change. No generator is responsible for this geometry file.

## Pinned High Five source

| Repository | Revision | Role |
|---|---|---|
| <https://bitbucket.org/l2jserver/l2j-server-game> (`develop`; GitHub mirror `monkey1sai/l2j-server-game`) | `abfde0490ac52a2106e884b7242c0276c44cdf76` (2026-09-13) | Formulas, functions, config defaults |
| <https://bitbucket.org/l2jserver/l2j-server-datapack> (`develop`) | `3ca488dd2bd0bfaca43e378886a3c2e37968153a` (2026-09-13) | Tables and templates |

The two READMEs say: "develop branch is the current High Five development" (client: High Five part 5).

We read these files (the game prefix is `src/main/java/com/l2jserver/gameserver/`, and the datapack prefix is
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
  `stats/chars/baseStats/{Human,Elven,Dark,Orc,Dwarven}{Fighter,Mystic}.xml` (there is no Dwarven mystic),
  `stats/initialEquipment.xml`, `stats/items/02300-02399.xml` (item 2369).

## Regenerating

```bash
git clone -b develop https://bitbucket.org/l2jserver/l2j-server-game.git /tmp/l2j-game
git -C /tmp/l2j-game checkout abfde0490ac52a2106e884b7242c0276c44cdf76
git clone -b develop https://bitbucket.org/l2jserver/l2j-server-datapack.git /tmp/l2j-dp
git -C /tmp/l2j-dp checkout 3ca488dd2bd0bfaca43e378886a3c2e37968153a
python3 packages/data/scripts/gen_tables.py --game /tmp/l2j-game --datapack /tmp/l2j-dp --check
```

The script refuses other revisions. It uses only `decimal`/`fractions`. It parses each constant
from a literal source line, and that line must exist exactly as written. It also proves that the HP/MP quadratic of each class
makes all 85 source rows again. If the committed files are different from the output, `--check` fails.

## Fidelity ledger (resolved)

| Item | Resolved value | Source |
|---|---|---|
| Normal-attack damage coefficient | **76**; crit ×2 | `Formulas.calcPhysDam`: `damage = (76 * damage * proximityBonus) / defence;` |
| Attack interval | **500000 / pAtkSpd** ms, the impact occurs at half | `L2Character.calculateTimeBetweenAttacks`, `doAttack` (`timeAtk / 2`) |
| Accuracy | **`sqrt(DEX) * 6 + L`**, `+(L−69)` above 69, `+(L−76)` above 77 | `FuncAtkAccuracy` |
| Evasion (player) | **`sqrt(DEX) * 6 + L`**, `+(L−69)` from 70, ×1.2 on that addition from 78; cap 250 | `FuncAtkEvasion`, `MaxEvasion` |
| Stat bonuses | **`statBonus.xml` 2-decimal tables**, indices 0–99; comments `1.036^(STR−34.845)`, `1.020^(INT−31.375)`, `1.030^(CON−27.632)`, `1.010^(MEN+0.060)`, `1.009^(DEX−19.360)`, `1.050^(WIT−20.000)` | `BaseStats` static loader |
| Class HP/MP | `lvlUpgainData` rows for each level, 1–85; exact quadratics (for example, Human Fighter `80 + 11.765n + 0.065n²`, `30 + 5.43n + 0.03n²`) | `baseStats/*.xml` |
| XP table | `expData.json` levels 1–85, **sentinel L86 = 16,890,558,728**; cap `X[86] − 1` | `ExperienceData`, `PlayableStat.addExp` |
| Death loss | 10.0 % at L1, −0.125 for each level to 4.0 % at L49, 4.0 % to L75, 2.5/2.0/1.5 % at L76/77/78, 1.0 % L79–85 | `playerXpPercentLost.xml` |
| Starter weapon | Squire's Sword: P.Atk 6, crit 8, speed 379, random 10, range 40, accuracy 0 | item 2369 |
| Caps / respawn | crit 500 ‰, speed 1500, hit 200–980 ‰, HP 0.65 / MP 0, protection 600 s, max level 85 | `character.properties` |

## Errata against the planning docs

- **E-1 Stat curves.** `01-stat-formulas.md` §2.1 gives the name "High Five" to `1.009^(STR−49)` … `1.00369^(MEN+30.45)`
  and to Human Fighter STR 88. These values come from `andridgitalbox/l2j-mobius`
  (`d2a8c5d5`, an Ertheia-era tree: LUC/CHA, Ertheia classes). HF uses the tables whose
  comments the doc gives the label "Interlude". We changed plan vector 1 to use HF as its base (STR 40 = 1.20).
- **E-2 XP table.** The "High Five" column of the doc (L41 8,718,976; L85 5,008,025,097) is the
  Mobius/Ertheia table. HF `expData.json` agrees with the "Interlude" column of the doc only to L10 and at
  L76–78 (for example, L11 71,202 vs 71,201; L80 3,075,966,164; L85 13,180,481,103).
- **E-3 Accuracy/evasion.** HF uses `* 6` and the level additions above. It does not use the `* 5` / "+1 at 78+, +2 at 81+"
  of stats §2.4. The `* 6` of combat §2.3 is correct, but its additions are not precise.
- **E-4 CON 43** is 1.58 in the HF table (the doc prints 1.57).
- **E-5 Weapon stats replace, not add.** Item stats are `<set>` funcs (order 0). They apply before
  the STR/DEX/level multipliers (order 1). Thus P.Atk = `weapon × STR × LM`, not
  `(fist + weapon) × …`. The weapon also replaces the crit base and the attack speed (8, 379).
- **E-6 Spawn protection** is `PlayerSpawnProtection = 600` **seconds** (6000 ticks), not
  600 ticks.
- **E-7 Rounding (reviewed E1.4).** The earlier statement that each difference was ≤1 unit
  was false for hit chance and attack milliseconds. Now, Nightfall makes the same source
  `Math.round` results at accuracy/evasion, attack speed and death loss with integer arithmetic.
  It also makes the two integer millisecond casts in the attack schedule. HP/MP keep the exact decimal
  rows under D3. Refer to the measured ledger below. Because the `(int)` of crit occurs **before** `+0.5`, it does
  not round a fractional crit rate up.
- **E-8 The "77".** The HF normal attack uses 76. The value 77 occurs only in the crit additive term
  (`CRITICAL_DAMAGE_ADD * 77 / defence`, which is zero without buffs) and in skill/blow damage.
- **E-9 Interval.** Stats §2.4 prints 470000 as the HF value, but the HF value is 500000. We did not
  compare the 470000 value with an Interlude source here.
- **E-10 Misc, not used in Phase 1.** `MaxRunSpeed = 250` (the doc gives 300). `baseMDef` adds the right
  ring two times (`PlayerTemplateData`). Fists use the random radius `5 + (int) sqrt(level)` (players do not use the template
  `baseRndDam`). If an attack causes 0 hate to a mob that a player attacks, the attack adds 1.
- **E-11 Table index 0.** CON[0] 0.45 and WIT[0] 0.39 are different from their comment expressions
  (0.44, 0.38). The table is the authority, and the code cannot get to index 0 (base stats ≥ 1).

## Missing rows

- The Kamael starters (class 123/124) are not included, because Nightfall has no Kamael race.
- XP row 87 exists in the source, but Nightfall does not use it (Nightfall uses only the L86 sentinel).
- All items in the E1.1 ledger have a source.
## E1.4 independent fidelity review

The [oracle](scripts/oracle.py) is only for tests. It uses Python `Fraction`, integer square-root
bounds and an integer emulation of IEEE ties-even for the resource-storage audit. It does not import
`gen_tables.py` or Rust. It reads the literal `bonus` rows, the HP/MP `levels` rows, the weapon stats,
and the XP and percentage rows. It never reads the `_q` values or the fitted quadratic coefficients.
We wrote its source expressions again independently, and the oracle compares them with the literal lines
in `formulas.toml`. The static bonuses use the XML values, which are the authority (this includes E-11).
The oracle does not change them into exponential curves. Production has no new dependency.

```bash
python3 packages/data/scripts/oracle.py
python3 packages/data/scripts/oracle.py --check
cargo test --test formula_fidelity
```

The checked-in [golden](fixtures/oracle.json) file keeps the independent results for each axis. Rust
makes the expected sheets only by lookup. If a table, a class, an NPC target or the oracle script changes, the input SHA-256 checks
prevent out-of-date fixtures. The related
[rounding audit](fixtures/rounding-audit.json) has the exact maximum differences and the witnesses.
It compares the contract from before E1.4 with the source semantics. It is not a tolerance file.

Coverage:

- All **600** stat bonus rows (six tables, indices 0–99). All **85** LM and accuracy/evasion
  addition rows. All **86** XP thresholds with the sentinel. Each death-loss row.
- All **765** original class/level profiles, with the sword and with fists. Each class/level also
  uses the full Cartesian DEX/STR/CON grid **{1,9,25,30,43,64,99}³**, armed and not armed. This gives
  **524,790** grid sheets. A further **75,735** armed sheets use all 99 legal values on
  each attribute axis together. The total is **602,055** deterministic sheet comparisons.
- The HP/MP quadratics are examined against all **1,530 original resource rows**. The DEX source
  rounding is examined at each index 0–99 and level 1–85. Bounded square roots make sure that the
  rounded result is correct independently of the Q approximation of production.
- **32,130** normal/critical damage vectors (nine profiles × 85 levels × 21 spreads × two
  crit states) against the literal Keltir defence, with hate. **510** hate vectors for
  all levels, which include zero and the maximum `u32` damage. The hate cap boundaries.
- **700** speed/timing vectors, which include synthetic inputs at the speed cap. **600** crit/cap
  vectors. Each roll 0–999 for the fractional hit boundaries, and all the supported DEX crit rates.
  The tests examine threshold ±1, each death/delevel boundary, the XP saturation and the sentinel.
- Two **1,024-case** property tests change the profiles, levels, all legal DEX/STR/CON values,
  equipment, opponent levels/DEX, rolls, XP and rewards. The tests compare the results with those independent results.
  Ertheia P.Atk, the Interlude base-44 crit and the CON=1.57 resource fixture of the docs stay
  as inputs with the explicit label legacy/fixture. They are separate from the production HF profiles.

We read the rounding sites again in the pinned game revision `abfde0490ac52a2106e884b7242c0276c44cdf76`:
[CharStat.java](https://raw.githubusercontent.com/monkey1sai/l2j-server-game/abfde0490ac52a2106e884b7242c0276c44cdf76/src/main/java/com/l2jserver/gameserver/model/actor/stat/CharStat.java)
`getAccuracy` and `getEvasionRate` use `(int) Math.round(calcStat(...))`.
`getPAtkSpd` returns `Math.round(calcStat(...))`. `getCriticalHit` casts to int before
it adds `.5`. `PcStat.getPAtkSpd` caps the rounded result. `PcStat.getMaxHp/getMaxMp`
cast the resource product to int. `formulas.toml` keeps the interval statement exactly as written,
and `penalties.toml` keeps the death-loss statement exactly as written.

| Quantity/site | Maximum old-contract difference | Disposition / witness |
|---|---|---|
| Bonus lookups, LM, P.Atk/P.Def | 0 against exact literal expressions | Q can show all products for these source inputs exactly. No change. |
| Accuracy `Math.round` | 0.497188 stat units | Fixed: DEX 55, L1 was 45.497188, now 45. |
| Evasion `Math.round` | 0.499884 stat units | Fixed: DEX 56, L82 was 142.499884, now 142. |
| Hit chance after those getters | **19‰** (19 roll outcomes out of 1000) | Fixed: a same-level DEX 26 attacker against a DEX 21 target was 861‰, now 880‰. The source inclusive `roll <= chance` stays. The fractional rounding-error bound for all DEX pairs and all five evasion fractional additions is also 19‰, also with different levels. |
| Crit `(int)` then `+0.5` | **0‰** | The floor does not change. Because of the integer cast, the half that follows has no effect. A rational binary64 emulation also finds no rate difference for sword/fists at all DEX indices. |
| Attack speed `Math.round` | 0.49 speed units | Fixed: sword DEX 78 was 640.51, now 641. |
| Attack interval integer cast, including speed rounding | **46261/19329 ms** (≈2.39335 ms) | Fixed: sword DEX 22, raw 25000000/19329 ms against source 1291 ms. |
| Half-interval integer division, including preceding operations | **32795/19329 ms** (≈1.69667 ms) | Fixed: the same witness, source impact 645 ms. |
| Impact/cycle tick deadlines | **1 tick** each | Fixed: the fist DEX 39 impact was +8, now +7. The fist DEX 17 cycle was +18, now +17. The tick ceiling is the explicit 100 ms schedule boundary of Nightfall. |
| Death loss `Math.round` | **1 XP**, and possibly **1 level** | Fixed: L5 loss 299→300. XP 3183 now becomes 2883/L4 and not 2884/L5. The L1 loss 6→7 cannot delevel below 1. |
| HP from Java binary32 rows (then binary64 product and int cast) | **1 HP** on the extended CON axis; **0** for all original profiles | Keep the D3 exact-decimal floor: Dark Fighter L40, CON 79 gives 3363, and Java gives 3362. No level/hit outcome changes. |
| MP from Java binary32 rows (then binary64 product and int cast) | **1 MP** on the extended MEN axis; **0** for all original profiles | Keep the D3 exact-decimal floor: Dark Fighter L36, MEN 22 gives 321, and Java gives 320. No level/hit outcome changes. |
| Hate, XP thresholds/cap | **0** against rational source expressions | No change. There is no difference at a rounding site. |

We measured the grid maxima over all table indices (not only the Cartesian grid of seven values).
The resource audit extends MEN and CON to all indices. The E1.4 gate explicitly permits the differences of one resource
unit that stay. This is fidelity to the rational formulas. It is not an emulation of each Java binary64 intermediate value in the combat pipeline.

The consolidation of the decimal parser also solves two admission rules that did not agree. NPC
strings that end in `.` now fail the shared plain-decimal grammar. The parser accepts more trailing zeros
when Q can show the number exactly (for example, `0.1234560`). The two loaders
now accept the full signed Q range, which includes `-9223372036854.775808`. Neighbour values out of the range
still fail. The loaders still reject NPC TOML float literals. The shared module has
the union of the two parser suites, and also exact tests for the signed endpoints.

## Phase 2 class and race catalog provenance

This section gives the provenance and the reproduction steps for the Phase 2 race, profession and growth catalogs. The catalog extends past the 9 starting classes of Phase 1 (`classes/*.toml`). It includes all 89 classic professions in tiers 0..=3, 5 classic playable races, and 89 growth tables (one for each class). The Phase 1 tables, starter classes and fidelity oracles do not change.

### Pinned High Five source

| Repository | Revision | Role |
|---|---|---|
| <https://bitbucket.org/l2jserver/l2j-server-datapack> (`develop`) | `3ca488dd2bd0bfaca43e378886a3c2e37968153a` (2026-09-13) | Class definitions, static attributes, skill trees, and the resource growth for each level |
| <https://bitbucket.org/l2jserver/l2j-server-game> (`develop`) | `abfde0490ac52a2106e884b7242c0276c44cdf76` (2026-09-13) | CP CON-multiplier (`FuncMaxCpMul`) and integer floor (`PcStat.getMaxCp`) |

We read these datapack files (the prefix is `src/main/resources/data/stats/chars/`):
- `classList.xml`: the class IDs, the parent links, and the retail class names.
- `src/main/resources/data/skillTrees/classSkillTree.xml`: the level/SP gates, the automatic grants, the NPC learning flags, the item requirements, and the Expertise and equipment mastery references.
- `src/main/resources/data/stats/skills/*.xml`: the skill ID/name/level-range metadata of the targeted skills. We use it to validate the learning references. We do not copy the skill effects.
- `baseStats/*.xml`: 89 classic class XML templates. Each template has `<staticData>` (base stats, movement speeds, collision boxes, environmental traits) and `<lvlUpgainData>` (85 rows of HP, MP and CP, one for each level).
- The base fighter XML templates (`HumanFighter.xml`, `ElvenFighter.xml`, `DarkFighter.xml`, `OrcFighter.xml`, `DwarvenFighter.xml`). They give the baseline race movement, collision and traits.

### Generated catalog directories

| Directory | Count / Schema | Contents |
|---|---|---|
| `packages/data/races/*.toml` | 5 TOML files | 5 classic playable races (`human`, `elf`, `dark_elf`, `orc`, `dwarf`). The files have the movement speeds of the racial fighter baseline, decimal collision strings, environmental traits (`breath`, `safe_fall`), and the descriptor keys of the racial passive skills. |
| `packages/data/professions/*.toml` | 89 TOML files | 89 classic classes in tiers 0..=3 (tier distribution 9-18-31-31; IDs 0..=57 and 88..=118). The files have base stats with a sum of 170, the movement of each class, collision in integer world units at Q = 1,000,000, transfer token requirements, and subclass configuration. The reserved Kamael IDs 123..=136 are not there. |
| `packages/data/skill_catalog.toml` | 378 targeted skill definitions | Source skill IDs, maximum levels, reference names and definition paths. The runtime status is explicitly deferred. |
| `packages/data/growth/*.csv` | 89 CSV files | The resource growth of each class for levels 1..=85. Each file has the header `level,hp,mp,cp` and then 85 rows: 7,565 rows in total, 22,695 exact HP/MP/CP plain decimal scalars (up to 6 decimal places, exact at Q = 1e-6). |

### Regenerating and verifying

```bash
git clone -b develop https://bitbucket.org/l2jserver/l2j-server-datapack.git /tmp/phase2-l2j-dp
git -C /tmp/phase2-l2j-dp checkout 3ca488dd2bd0bfaca43e378886a3c2e37968153a
python3 packages/data/scripts/gen_classes.py --datapack /tmp/phase2-l2j-dp --check
```

The generation script `packages/data/scripts/gen_classes.py` does these steps:
- It validates the exact pinned revision `3ca488dd2bd0bfaca43e378886a3c2e37968153a`. It refuses changed, untracked or ignored files in the source paths for the character, the skill definitions and the learning tree.
- It extracts the XML source strings exactly as written. It writes plain decimals without a floating-point conversion.
- It makes all 5 race files, 89 profession files, 89 growth CSV files and the targeted skill reference catalog.
- During `--check`, it refuses unknown skill IDs, source levels out of their definition range, duplicate learning/proficiency rows and unexpected files in the generated directories.
- In `--check` mode, it compares the generated contents byte-for-byte with two sets of files. These are the committed data files in `packages/data/` (`races/`, `professions/`, `growth/`) AND the compiled Rust inventory file at `apps/api/src/infrastructure/class_data_embedded.rs`.

### Data extraction and lineage fidelity

- **XML source exact strings**: The base stats, movement speeds, collision boxes and `lvlUpgainData` values are copied directly as exact plain decimal strings. These strings have up to 6 fractional decimal digits and are exact at Q = 1,000,000. Float literals are not permitted.
- **Retail parent errata**: The retail `classList.xml` has two incorrect parent links that break the class lineage trees. `PARENT_ERRATA` corrects them:
  - Class 34 (`Bladedancer` / `Edge Cantor`): retail gives `parentClassId="33"` (Shillien Knight). The correction is `parentClassId="32"` (Palus Knight).
  - Class 104 (`Elemental Master` / `Tide Sovereign`): retail gives `parentClassId="26"` (Elven Wizard). The correction is `parentClassId="28"` (Elemental Summoner).
- **Fixed 170 stat sum**: For all 89 classic classes, the sum of the six base stats (`STR + DEX + CON + INT + WIT + MEN`) is exactly 170.
- **Lineage consistency**: All non-root classes (tiers 1..=3) have the same static attributes from combat sources as their lineage root class. Thus `race`, `archetype`, `base_class_id`, and all six `base_stats` are the same as in the root starter class.
- **Source-explicit growth necessity (240 vs 27 curves)**:
  - The 9 starter XML templates have 27 curves (9 classes × 3 resources HP/MP/CP). All of these curves are exact quadratics (`base + per_level*(L-1) + accel*(L-1)^2`).
  - For all 89 classes (267 curves in total), 240 curves are not quadratic. This is because of the transfer discontinuities and the growth shifts near the level 20 and 40 transfer thresholds. Only the 27 base curves are quadratic.
  - Thus it is necessary to store explicit source tables of 85 rows (7,565 rows, 22,695 decimal scalars) to keep source fidelity for the full catalog. One quadratic formula cannot show the professions that advance.

### Movement and collision semantics

- **Racial baseline vs class-specific attributes**:
  - `RaceDef` (and `RaceInfo`): The movement speeds (`walk`, `run`, `swim`) and the collision dimensions (`radius_male`, `radius_female`, `height_male`, `height_female`) record the fighter baseline for that race. The values come from `HumanFighter.xml`, `ElvenFighter.xml` and the other fighter files.
  - `ClassDef`: Records the actual movement and collision of the class from the XML of each class. Human Mystic (class ID 10) and Orc Mystic (class ID 49) are different from their racial fighter baselines. For example, Human Mystic walk/run is 78/120, and Human Fighter is 80/115. The collision dimensions are also different.
- **Unit representations**:
  - In race files (`races/*.toml`): The collision dimensions are exact decimal strings in reference world units (for example, `radius_male = "9.0"`, `height_male = "23"`).
  - In profession files (`professions/*.toml`): The collision dimensions are integers at fixed-point Q = 1,000,000 world units (`Scaled`, for example `radius_male = 9000000`, `height_male = 23000000`).
  - The movement speeds are whole integers in reference world units per second.

### Start points, reference zones, and naming conventions

- **Start points and fixture zones**: `start_points` in `races/*.toml` have the value of the fixture test coordinates `[[0, 0]]` in `test_zone` (with the respawn safe point `[126, 126]`).
- **Reference starting zones**: `reference_starting_zone` (`talking_island_village`, `elven_village`, `dark_elf_village`, `orc_village`, `dwarven_village`) is only information metadata. It refers to the classic starter villages. These villages are NOT implemented.
- **Naming identity**:
  - `l2_ref`: Keeps the exact retail names without changes (for example, "Human Fighter", "Warrior", "Gladiator", "Duelist").
  - `display_name`: The original draft metadata for Nightfall, which we selected and which can change (for example, "Human Armsbearer", "Steel Initiate", "Twinblade Champion", "Blade Paragon").
  - The 89 original Nightfall labels and the fixture starting positions are design metadata. They are separate from the cited retail values.

### Deferred hooks and future phase boundaries

- **Class skill tree and proficiencies**: `SkillLearnDef` (`skill_tree`) has all the source learning rows for 9 base classes, 18 first classes and 12 second-class MVP professions. The MVP professions are `[2, 5, 8, 9, 12, 16, 17, 27, 33, 46, 52, 55]`, and there are 6,927 entries in total. The 50 other direct trees have the mark `deferred`, but it is possible to query the ancestor entries. `ProficiencyDef` records 3,521 source rows of equipment mastery and Expertise unlocks for all 89 classes. The source-exact learning level gates (`required_level` / `getLevel`) control when a skill is available to learn. They do not control the active class skill effects.
- **Automatic metadata grants**: The sourced `autoGet` entries give metadata that is kept at character creation, transfer or level-up. The grant is the maximum level for each key, at a charge of zero SP. Phase 3 does not include manual learning flows, SP expenditures and skill effects.
- **Item requirement hooks**: 48 learning entries must have items. The keys use the canonical `l2.item.<id>` with a positive unsigned 32-bit ID. They refer to future inventory item definitions.
- **Known skill reference catalog**: `packages/data/skill_catalog.toml` defines 378 targeted skill entries with known IDs, maximum levels and source XML paths. The runtime status is always `Deferred` (there is no runtime engine).
- **Racial passive keys**: `passive_skill_keys` in `races/*.toml` (for example, `racial.adaptable`, `racial.forest_step`, `racial.shadow_precision`, `racial.iron_constitution`, `racial.pack_mule`) are descriptor strings. They are not used until the future Phase 3 skill registry exists. The progression and combat rules define the server runtime effects.
- **Reserved Kamael IDs**: The classic catalog IDs include 0..=57 and 88..=118. The Kamael IDs 123..=136 are not there (Nightfall has no Kamael race).

### Level progression, transfer gates, and subclass eligibility

- **Level cap**: The source level cap (which is the authority) and the playable level cap stay at 85, as in Phase 1.
- **Transfer gate**: The catalog files specify transfer token requirements for tiers 1..=3 (`class_transfer_token_1`, `class_transfer_token_2`, `class_transfer_token_3`). But the application transfer gate has a cap at transfer 2. The third-tier professions (tier 3, min level 76, IDs 88..=118) exist in the catalog metadata and validation. But because of the transfer gate, a player cannot get to them in the game now.
- **Subclass eligibility model** (`apps/api/src/domain/subclass.rs`):
  - This is only a model that examines eligibility. It does not change the character state, and it does not learn skills. The village-master dialogue and interaction flows are deferred.
  - Subclasses start at level 40 (`SUBCLASS_START_LEVEL = 40`).
  - The subclass level cap is the main level cap (which is the authority) minus 5 (`subclass_level_cap(85) = 80`).
  - Each character can have a maximum of 3 more subclass slots (`MAX_SUBCLASSES = 3`).
  - Certification limits: 4 certifications for each subclass slot (`CERTIFICATIONS_PER_SUBCLASS = 4`), and a maximum of 12 in total for all three slots (`MAX_CERTIFICATIONS = 12`).
  - Subclass eligibility criteria:
    - The main class must have done the second transfer (tier ≥ 2).
    - The main level must be a minimum of 75.
    - The subclass quest is complete (`quest_completed = true`), or the character has noble status (`noble = true`).
    - Not all of the more subclass slots are used (< 3).
    - The candidate profession must be tier 2 and permitted (`subclass_allowed = true`; Overlord ID 51 and Warsmith ID 57 are not permitted).
    - Racial restriction: Elf and Dark Elf lineages cannot cross.
    - Equivalence restriction: The candidate cannot be the same as a held profession (the main class or the subclasses that exist). Through `subclass_equivalents`, it also cannot be in the same equivalence set.

### Registry validation, canonical hashing, and Phase 1 preservation

- **Startup loader and deserialization validation**: `infrastructure::class_data::load_classes` loads all catalog files at startup. The `ClassRegistry` serde deserialization (`try_from = "RegistryParts"`) runs `validate()` again. Thus, when the code deserializes saved snapshots, it validates all catalog invariants again. The invariants are the IDs, stat totals, tier gates and parent links. They also include growth that does not decrease, subclass configurations, positive collision values, strict nested fields, and the ID/level bounds of known skills.
- **Canonical config hash**: `ResolvedClasses.config_hash` is a SHA-256 digest of the canonical JSON encoding of `(&registry, &provenance)`. It includes all resolved metadata, the source provenance and the growth tables. TOML whitespace/comments and equivalent accepted decimal forms do not change it. The CSV headers and the contiguous rows of four columns stay strict. The parser rejects comments and whitespace out of the plain decimal cells.
- **Phase 1 preservation**: The Phase 1 stat tables (`tables/*.toml`), starter classes (`classes/*.toml`), formula tests (`formula_fidelity`) and independent verification fixtures (`oracle.py`, `oracle.json`, `rounding-audit.json`) stay complete. They do not change. The 89 original draft Nightfall display names, which we selected, are separate design metadata that can change. The retail names stay in `l2_ref`. The fixture start positions stay `[0, 0]`.

### Independent reviews, verification boundaries, and catalog size

- **Independent read-only review v1**: The review of `53be185` examined all 89 classes, 5 races, 7,565 rows and 22,695 exact resource values. It found three admission bugs:
  1. *Class collision dimensions*: `validate_class` examined the movement, but it did not examine that the collision dimensions are positive.
  2. *Nested unknown fields*: the nested `BaseStats`, `RaceDef` and `GrowthRow` did not have `deny_unknown_fields`.
  3. *Generator `--check`*: it examined the expected files, but it did not compare the directory inventories (it did not find orphan files).
  Commit `e02ee69` solved all three (12 catalog tests, all-targets clippy `-D warnings`, fmt, tests and the gen check passed).
- **Independent field-tuple audit v2**: This audit examined `e02ee69`. It independently extracted and examined these items:
  - All 6,927 learning rows (SP costs, default bools, NPC learning, 48 item requirements).
  - 3,521 proficiency rows.
  - 378 known source definitions (runtime status `Deferred`).
  - 89 classes.
  - 7,565 growth rows (22,695 scalars).
- **Review findings and disposition**: The second audit found two admission gaps. A pinned Git HEAD permitted changed source inputs, and item references that were not empty could have incorrect retail IDs. The validation that followed now rejects source input paths that are not clean. For learning requirements, it accepts only canonical positive `l2.item.<u32>` IDs. Tests cover the admission of the source revision/working tree and incorrect learning references, at startup load and at snapshot deserialization. The follow-up passed 13 Rust catalog tests, 3 Python provenance tests, all-targets clippy with warnings denied, formatting and the source regeneration checks. The independent audit examined the released tuples before these fixes. The fixes have separate regression coverage.
- **Catalog size and snapshot integration**: We made an independent Python compact JSON reconstruction of all resolved catalog fields. It gives 2,048,521 raw bytes (~2.05 MB; zlib fast 207,161 bytes, default 156,512 bytes). This is an independent measurement that is approximately equivalent to the catalog encoding. It is not a full zone checkpoint. This measurement is the reason for compressed checkpoint storage.


### CP maximum multiplier

The pinned game source at `abfde0490ac52a2106e884b7242c0276c44cdf76` uses the same
CON multiplier for CP as for HP. `FuncMaxCpMul.calc` multiplies the CP row of the template
by `BaseStats.CON.calcBonus(effector)`. `PcStat.getMaxCp` casts the positive value that results
to an integer. Thus Nightfall calculates the floor of the exact CP × CON product.
It uses the Q arithmetic that exists and no binary floating point. Sources:
[FuncMaxCpMul.java](https://raw.githubusercontent.com/monkey1sai/l2j-server-game/abfde0490ac52a2106e884b7242c0276c44cdf76/src/main/java/com/l2jserver/gameserver/model/stats/functions/formulas/FuncMaxCpMul.java)
and [PcStat.java](https://raw.githubusercontent.com/monkey1sai/l2j-server-game/abfde0490ac52a2106e884b7242c0276c44cdf76/src/main/java/com/l2jserver/gameserver/model/actor/stat/PcStat.java).
The Phase 2 catalog stores the CP before the multiplier, with the fractional source rows. The
server applies CON when it builds the active class sheet.

### Source learning and proficiency coverage

The second-class IDs that have data are **2, 5, 8, 9, 12, 16, 17, 27, 33, 46, 52, 55**,
as the Phase 2 MVP slice specifies. Together with all bases and first transfers,
these give 39 direct learning trees and 6,927 source entries. Each entry keeps
`skillId`, `skillLvl`, `getLevel`, `levelUpSp` (zero when it is not there), `autoGet`,
`learnedByNpc`, and the optional item ID/count requirements. The stable keys are
`l2.skill.<id>` and `l2.item.<id>`. The source names stay in `l2_ref`.

All 89 classes have their source equipment mastery and Expertise rows as proficiency
metadata (3,521 entries). They inherit these rows through the corrected class graph. Expertise 239
unlocks at 20/40/52/61/76/80/84 for levels 1..7. We selected the equipment mastery names explicitly.
Cubic Mastery, Skill Mastery and Focus Skill Mastery are not in the scope of equipment
proficiency. The 378 different referenced skills are compared with the
IDs and maximum levels of the pinned skill-definition XML, and are serialized with the registry.
The runtime status of each known skill stays `deferred`. The metadata grants do not say
that the Phase 3 effects engine exists. Automatic grants can keep progress at a cost of zero
SP. Manual learning and the execution of effects are future work.

## Phase 3 skill data, pvp, regen, and keltir sp reward

This section gives the provenance and the reproduction steps for these Phase 3 items:

- The skill definitions.
- The classification of capability executability.
- The PvP/karma tables.
- The resource regeneration rules.
- The SP reward of the starter NPC.

### Pinned High Five source

| Repository | Revision | Role |
|---|---|---|
| <https://bitbucket.org/l2jserver/l2j-server-datapack> (`develop`) | `3ca488dd2bd0bfaca43e378886a3c2e37968153a` (2026-09-13) | Skill definitions, NPC templates, karma increase multipliers |
| <https://bitbucket.org/l2jserver/l2j-server-game> (`develop`) | `abfde0490ac52a2106e884b7242c0276c44cdf76` (2026-09-13) | Formulas for skill damage, success rates, traits, interruption, resurrection, regeneration, karma gain/lost |

We read these files:
- Datapack (`src/main/resources/data/`):
  - `stats/skills/*.xml`: All targeted skill definitions (378 catalog skills + 5076 + 9 transitive referenced skills + 286 sample).
  - `stats/npcs/20500-20599.xml`: The SP reward of NPC 20545 (`Young Keltir`) (`sp="2"`).
  - `stats/chars/pcKarmaIncrease.xml`: 85 karma loss multipliers, one for each level.
- Game (`src/main/java/com/l2jserver/gameserver/`):
  - `model/stats/Formulas.java`:
    - `calcSkillPhysDam`: Physical skill damage and critical hits (`lines 746-809`).
    - `calcMagicDam`: Magical skill damage and magic critical hits (`lines 815-898`).
    - `calcMagicSuccess`: The success rate for magic attacks (`lines 1286-1314`).
    - `calcEffectSuccess`: The success rate for debuffs and abnormal effects, with traits and mAtkMod (`lines 1158-1232`).
    - `calcAtkBreak`: The damage interruption rate for physical/magic casting (`lines 992-1025`).
    - `calcAttributeBonus`: The multiplier of elemental attribute power against defence (`lines 1440-1560`).
    - `calcHpRegen`, `calcMpRegen`, `calcCpRegen`: The resource regeneration rates and multipliers (`lines 263-460`).
    - `HP_REGENERATE_PERIOD = 3000`: 3000 ms (30 ticks at 100 ms/tick), `line 109`.
    - `calculateSkillResurrectRestorePercent`: The pinned resurrection XP restoration formula (`lines 1372-1386`).
    - `calculateKarmaGain`, `calculateKarmaLost`: The karma gain when a player kills a player, and the karma loss when XP increases (`lines 1814-1848`).
  - `model/actor/L2Character.java`:
    - `doRevive()`: The base revival vitals (`lines 2098-2124`).
  - `model/actor/instance/L2PcInstance.java`:
    - `doRevive(double revivePower)`: The player revival calls doRevive() and then restoreExp (`lines 7984-7986`). The XP refund Math.round is at `4700-4702`.

### Generated skill definitions and catalogue

- `packages/data/skills/*.toml`: 389 skill definitions, one file for each skill. `packages/data/scripts/gen_skills.py` makes them.
  - 378 catalog skills that the 39 class learning trees with data refer to.
  - Skill 5076 (`Death Penalty`, passive).
  - 9 transitive referenced skills that we found through effect parameters (`997, 5603, 5604, 5676, 5677, 5678, 6090, 6687, 6920`).
  - Skill 286 (`Provoke`, a reference only for the sample, with the mark `sample_only = true`).
  - Classification: **208 executable** and **181 deferred** in total (**199 executable** and **179 deferred** in the 378 catalogue skills).
  - The executability of each skill comes from the Tier B capability rules (zone-design-v3 §1.1). `TargetMe` is explicitly deferred with `effect:TargetMe`.
- `packages/data/skill_catalog.toml`: Regenerated with the calculated `runtime_status` (`"executable"` or `"deferred"`). This value agrees with the blocker analysis of each skill.

### Tables and NPC updates

- `packages/data/npcs/keltir.toml`: We added `sp_reward = 2`. Its source is the pinned Young Keltir NPC 20545 (`D/src/main/resources/data/stats/npcs/20500-20599.xml`). `xp_reward = 28` stays tunable.
- `packages/data/tables/pvp.toml`:
  - Flag durations: `pvp_flag_ticks = 1200` (120 s normal), `pvp_flag_low_ticks = 600` (60 s when the player attacks a purple player).
  - The karma gain constants from `Formulas.calculateKarmaGain` (lines 1830-1848).
  - The karma lost divisor `30` and all 85 rows of `pcKarmaIncrease.xml` as exact decimal strings (levels 1..=85).
  - The item drop rules at a chaotic death are explicitly not included (they are not part of Phase 3).
- `packages/data/tables/regen.toml`:
  - Period: `hp_regenerate_period_ms = 3000` (30 ticks).
  - Movement multipliers: `sitting = "1.5"`, `standing = "1.1"`, `moving = "0.7"`.
  - Stat dependencies: CON for HP/CP, MEN for MP.
  - The source of the base regen curve: the class templates (`packages/data/classes/*.toml`).


### DATA review 1 corrections and verification

- All 389 definitions parse with `tomllib`, and this includes the deferred definitions. The serialization uses recursive arrays of tables for the conditions and the stat children. It also examines that the structure is equal after a round trip. All source sets and each referenced rank table stay. Unresolved, short or incorrect tables are errors.
- The stat conditions are the source apply conditions (`DocumentBase.java:246–247`), with bare `<using>`, `<player>` and boolean nodes. If a leaf is not supported, all the skills that it affects are deferred. Skills 94/148/171/225/290/291/293/294/1297 are deferred, and skill 217 keeps its SWORD/BLUNT condition. The full XML conditions and their unsupported attribute strings stay at each site.
- The parameter names/values, the operations and the scope filters are admitted explicitly. The supported stat operations are mul/add/sub. The operations div/set/share cause a deferral. The scope filters ALL/default and NOT_FRIEND are admitted. The other filters (FRIEND also) are deferred until they are implemented (`AffectObjectStaticImpl.java:32–38,58–63,84–99`).
- Pinned defaults: when they are not there, magicLvl=0, ranges=-1 and abnormalLvl=0 (`Skill.java:212–214,273`). The abnormal clamp is 10/90 (`character.properties:277–278`, `Skill.java:276–277`). The cast-speed cap is 1999 (`character.properties:268`, `CharStat.java:252`). The locked reuse is imported (`Skill.java:204`, `L2Character.java:1514–1524`). C5 solves the earlier default contradictions between the architecture and v3.
- The independent oracle has 24 formula sets / 1,203 vectors. The corrected continuous 500/550 ms staging starts immediately, and the impact occurs 400 ms later (`L2Character.java:1625–1643,4577–4581`). The zero/sub-ms cases and the <=100-ms cases use the original truncation of the task skillTime. The minimum is one Nightfall completion tick.
- TickHp uses binary32 division before it widens the value (`AbstractEffect.java:148–149`). Then it uses binary64 power/products (`TickHp.java:62–71`). The source fractional results are separate from the D-R12 adaptations for whole damage and Q restoration. The fixtures include CP absorption after the HP-1 clamp (`TickHp.java:75–76`, `PcStatus.java:185–194`) and dynamic PER reads. They also include Q carry over many pulses and the clear operation after a full clamp. The minimum regen traces when standing give the cumulative values 1/2/3 and the Q remainders 100000/200000/300000.
- Rational karma truncation is not always equivalent to Java. Level 2, XP 6208133913 and the multiplier 2.069377971 give the rational value 100000000, but binary64 gives 99,999,999. The oracle models the literal/long conversion and the IEEE rounding of each division separately (`Formulas.java:1814–1820`), with the pinned input RateKarmaLost=1 (`config/rates.properties:56`).
- More source observations: skill 60 specifies `nextActionAttack="fake_death"` (`stats/skills/00000-00099.xml:1929`). The game reads a boolean (`Skill.java:279`, `StatsSet.java:112–121`). Thus the normalized value is false, and the raw string stays. `AbstractEffect.java:70–72` keeps attachCond but puts the applyCond assignment in a comment, but `DocumentBase.java:270` parses it. DATA still keeps the effect conditions and defers the unsupported leaves, as the binding capability rule specifies. This observation does not change the execution contract without a record.

## Additive Phase 3 class magical and regeneration templates

`classes-phase3/<profession-key>.toml` is a separate schema-1 V5 input for all 89
classic classes. It does not replace `classes/`, `professions/`, `growth/` or a
historical stat-rule input. The `base_regen_source` pointer in `tables/regen.toml`
is older than these profiles: **use `classes-phase3/` for V5 regen bases**.
Keep this registry and the NPC `[phase3]` section in the Phase3Rules hash, out of
the legacy class/stat hash. The engine owner must read the new fields through
its Phase 3 loader. The strict NPC loader that exists now does not admit them yet
(`apps/api/src/infrastructure/npc_data.rs:59-84`).

The generation reads only the pinned local datapack
`3ca488dd2bd0bfaca43e378886a3c2e37968153a` and game
`abfde0490ac52a2106e884b7242c0276c44cdf76`. Before it writes, it examines the revision and makes sure that the source inputs are clean
(this includes ignored/untracked inputs). To make the output again, run:

```bash
python3 packages/data/scripts/gen_class_phase3.py \
  --datapack /home/matt-woodruff/.cache/nightfall/l2j/l2j-dp \
  --game /home/matt-woodruff/.cache/nightfall/l2j/l2j-game
python3 packages/data/scripts/gen_class_phase3.py \
  --datapack /home/matt-woodruff/.cache/nightfall/l2j/l2j-dp \
  --game /home/matt-woodruff/.cache/nightfall/l2j/l2j-game --check
python3 packages/data/scripts/skill_oracle.py --class-phase3
python3 packages/data/scripts/skill_oracle.py --class-phase3 --check
python3 -m unittest discover -s packages/data/scripts/tests
```

`--check` compares the exact contents and finds extra files in `classes-phase3/`. It also
examines the generated NPC suffix and does not write it again. The generation only adds
or replaces its delimited `[phase3]` suffix in `npcs/keltir.toml`. All bytes before
that suffix stay the same. The generated data that exists and the historical oracle vectors
stay the same. The separate new fixture is `fixtures/skill-oracle-class-phase3.json`.

Class profile fields:

| Table/key | Meaning and pinned source |
|---|---|
| `id`, `key`, `parent`, `base_class_id`, `tier`, `min_level` | The same catalog identity and corrected graph as `gen_classes.py`. The generator selects the XML of the class by `classId`. It does not use an ancestor curve in its place. The classList errata (34→32 and 104→28) stay as the Phase 2 provenance above records them. |
| `base_stats` | The exact six static XML attributes for that class. No effects or racial hooks are applied. `HumanFighter.xml:5-10` is an example. |
| `magical.base_m_atk` | The XML `staticData/baseMAtk` of the class template, not an override from an equipped weapon (`HumanFighter.xml:24`; game `L2CharTemplate.java:108,332-333`, `CharStat.java:237-246`). |
| `magical.base_m_def` | The loaded template total: rear + lear + **rfinger two times** + neck, as in `PlayerTemplateData.java:106-108`. The example XML slots are `HumanFighter.xml:25-31`. |
| `magical.m_def_slots_xml`, `magical.m_def_slots` | The literal slots and the actual `getBaseDefBySlot` mapping. `L2PcTemplate.java:71-75` also uses RFINGER for LFINGER. `FuncMDefMod.java:44-66` subtracts the bases of occupied slots before the MEN/level multipliers. |
| `magical.base_m_atk_speed` | The cast-speed base. It is not in all 89 XML staticData sections, thus its source is the `L2CharTemplate.java:112` default **333**. This is different from the physical `basePAtkSpd`. The getter uses this base at `CharStat.java:258`, and it caps/truncates at `:259-262`. The pinned cap **1999** is in `character.properties:268`. |
| `magical.base_m_critical` | When the XML `baseMCritRate` is not there, it is **0** by default, `L2CharTemplate.java:119`. But the actual magic-crit stat getter starts at **1**, `CharStat.java:270-275`. Do not use this template field as that getter input. |
| `combat_getter_bases.accuracy`, `.evasion`, `.magic_critical` | The initial values of the stat getters are **0**, **0**, **1** (`CharStat.java:108-109,171-174,270-275`). Accuracy/evasion are not staticData bases. Their built-in Funcs give the derived additions. |
| `regen.kind`, `regen.levels` | `kind="per_level"`, exactly 85 rows. Each row has an integer `level` and the original decimal strings `hp_regen`, `mp_regen`, `cp_regen`. Each value has a comment with its class XML path:line. There are 7,565 rows / 22,695 scalars. |

**Player regen is per level, not a class scalar.** `PlayerTemplateData.java:111-123`
loads each `lvlUpgainData/level/*Regen` node into the three double arrays of `L2PcTemplate`
(`L2PcTemplate.java:41-43,59-61,96-103`). Its getters index those arrays by
the current level (`:135-152`), and Formulas selects these getters
(`Formulas.java:264,358,444`). Each class has its own complete rows, as for the
HP/MP/CP growth inputs in `gen_classes.py`. In this pinned revision, all 89 regen
curves agree, but this is by chance. They still change with the level:

| Level | HP base | MP base | CP base | Example `HumanFighter.xml` lines |
|---|---|---|---|---|
| 1 | `2.0` | `0.9` | `2.0` | 63-65 |
| 20 | `3.4` | `1.2` | `2.5` | 215-217 |
| 40 | `5.4` | `1.8` | `4.5` | 375-377 |
| 85 | `9.9` | `3.0` | `8.5` | 735-737 |

These values are **pre-formula** values. Player HP/CP is multiplied by the level modifier and CON.
MP is multiplied by the level modifier and MEN. `Math.max(1,init)` is applied before the regen-stat
functions, and then the config and posture multipliers are applied
(`Formulas.java:334-349,419-434,442-453`). The level modifier is `(level+89)/100`
(`L2Character.java:5007-5008`). Standing is 11/10, running is 7/10 and walking is 1.
Sitting 3/2 is source behavior, but it is not in Phase 3. The pinned character config
multipliers are 1.00 (`character.properties:47-49`). No stat, posture, clamp or
bonus is applied before to the emitted class rows.

### Young Keltir 20545 additive NPC inputs

The new `[phase3]` section in `npcs/keltir.toml` cites
`stats/npcs/20500-20599.xml:3648-3661`. `[phase3.xml]` keeps **M.Atk 5.78704,
M.Def 29.5916164000214, HP regen 2, MP regen 0.9, accuracy 4.75** as source
decimal strings (`:3659-3661`). The source attributes are at `:3658`.
`NpcData.java:164-185` loads those values into StatsSet. The inherited template
getters are different from the raw XML:

- `[phase3.template]` has **M.Atk 5 / M.Def 29**, because `L2CharTemplate.java:108,110`
  calls `getInt` on parsed Numbers and `StatsSet.java:241-247` uses `intValue()`.
  The cast-speed base is **333** from the same `L2CharTemplate.java:112` default.
  `hitTime=370` (`npc XML:3669`) is not a cast-speed base.
- HP/MP regen are **scalar inherited getters**, not level arrays.
  `L2NpcTemplate.java:100-101` calls the superclass constructor.
  `L2CharTemplate.java:105-106,311-319` stores/returns binary32 values.
  The new template table keeps the exact getter rationals: HP `2`,
  MP `7549747/8388608` (the binary32 conversion of decimal 0.9).
  Keep that rational until the formula clamp. Do not truncate it to Q without a record.
- Formulas uses the scalar NPC regen without the player CON/MEN/level/posture bonuses
  (`Formulas.java:264,358,343-349,428-434`). If the initial value of MP is less than 1, it is clamped
  to 1. CP regeneration is only for players (`:442-453`). Thus the section explicitly
  records `cp_regen_kind="not_applicable"` and does not supply an invented CP scalar.
- The XML accuracy is a TODO in `NpcData.java:174`, and the code does not use it. Evasion is also a
  TODO at `:185`. The real getter inputs are zero (`CharStat.java:108-109,171-174`).
  The section keeps the XML accuracy separately and does not apply it two times.
  The `sp_reward=2` that exists (`npc XML:3657`) and the tunable physical/HP/MP/XP values do
  not change.

### Template-driven oracle and validation

`skill_oracle.py --class-phase3` emits **6 formula groups / 290 vectors** in the
new fixture. The fixture has all six requested starters (Human Fighter/Mystic, Elven Mystic,
Dark Elf Fighter, Orc Mystic, Dwarven Fighter). They occur at **1/20/40/85** for
FuncMAtkMod, FuncMDefMod, FuncMAtkSpeed and all three regen functions. They include
regen when standing/running/walking and the NPC HP/MP cases. The six class names agree with
the profession keys that exist. Dark Elf Fighter is `dark_fighter`.

The expected values use independent Fraction arithmetic on the emitted template values
and the literal stat-bonus tables. M.Atk uses base×INT²×levelMod²
(`FuncMAtkMod.java:42-46`). M.Def uses the empty-jewelry base×MEN×levelMod
(`FuncMDefMod.java:44-66`). Speed uses base×WIT and then the getter cap/truncation
(`FuncMAtkSpeed.java:42-43`, `CharStat.java:258-262`). The cases explicitly specify
no weapon/empty jewelry, no modifiers, no injected RNG, the config multipliers and
no estate/zone bonus. The fixture records the exact rationals, the intermediate values, the
rounding sites and the Q values. It also records the remainder traces of three pulses, the input SHA-256s and a
canonical vector-set SHA-256. The Q conversion calculates the floor only at the final formula
boundary, as in the oracle that exists. The transfer of whole resources keeps the
remainder under D-R12. This is evidence for the exact-decimal formulas. It does not say that the result is
equivalent to Java binary64 in all conditions. The NPC binary32 getter conversion is modeled
with the integer-only IEEE adapter, and `struct` examines it independently.

`test_gen_class_phase3.py` independently audits each class, the graph mapping, the
static magic base/slot and each regen literal against the pinned XML. It also tests
these conditions:

- Levels/regen that are not there.
- The rejection of a source revision or of dirty inputs.
- Asymmetric rings (to find an accidental correction of the duplicated RFINGER of the source).
- The detection of orphan and out-of-date files.
- The preservation of the NPC prefix.
- The getter conversions.
- The requested oracle coverage.
- The exact expected values and the fractional carry.

## Phase 3 equipped starter weapon and remaining caps

`gen_class_phase3.py` also makes `tables-phase3/starter_weapon.toml` and
`tables-phase3/constants.toml`. These are separate V5 inputs. Include their complete
contents in the Phase3Rules hash, and never in the legacy StatRules hash. The separate
folder is intentionally out of the historical `tables/*.toml` scan of `oracle.py`.
`tables/starter_weapon.toml`, all the earlier fixtures and their input hashes stay exactly the same.
The generator admits the pinned weapon XML as a clean source input, and validates the TOML
before it writes. `--check` finds differences and orphan files in the two Phase 3 folders.
Use the same `gen_class_phase3.py --datapack ... --game ... [--check]` command above.

The pinned datapack `src/main/resources/data/stats/items/02300-02399.xml:844-867`
identifies Squire's Sword **2369**. Its type **SWORD** (`:847`) supports the equipped
`using kind` condition. The bodypart **rhand** (`:848`), the random damage **10** (`:849`)
and the attack range **40 L2 units** (`:850`) stay. `[[functions]]` has
all four source SET Funcs in XML order: **pAtk 6** (`:862`), **mAtk 5** (`:863`),
**critRate 8** (`:864`), **pAtkSpd 379** (`:865`). Each entry has the operation, the
resolved priority, the XML ordinal, the exact decimal string, the Q value and the file:line provenance.
There are no more weapon stat Funcs in that `for` block (`:861-866`).

The weapon SET does **not add to the class M.Atk base**. In the pinned game,
`engines/DocumentBase.java:230-247` gives -1 when the order is not there.
`model/stats/functions/FuncTemplate.java:48-54` changes it to the enum default.
`enums/StatFunction.java:30` declares the SET priority **0**. Then
`model/stats/functions/FuncSet.java:36-40` returns the weapon value. This value replaces the
template initial value of the getter (`model/actor/stat/CharStat.java:237-246`).
`model/stats/Calculator.java:110-127,183-188` applies the functions in ascending priority. For equal priorities, it uses the
insertion order. `FuncMAtkMod.java:37-46` runs at priority
**1**. It multiplies that replaced value by the square of the INT bonus and by the square of the level
modifier. `model/actor/L2Character.java:5007-5008` defines the level modifier.
The `[ordering]` of the new profile records the full pinned repository/revision/file/line
provenance for these semantics.

The separate `[constants]` entries keep **MaxMCritRate 200**, **MaxRunSpeed
250** and **RunSpeedBoost 0** from the pinned game
`src/main/resources/config/character.properties:260,252,31` respectively. Each
entry has the integer value, the original literal, the file, the line and the symbol. The top-level
source has the pinned repository and revision. `tables/formulas.toml`
and its legacy hash do not change.

`starter_weapon_oracle.py` makes
`fixtures/skill-oracle-starter-weapon-phase3.json`: **24 FuncMAtkMod vectors**.
They cover the same six reference classes at levels **1/20/40/85** as the class-profile fixture
without a weapon. Each vector records the template initial value, the order-0 weapon
replacement and the priority-1 result. It also records the result without a weapon and a comparator for an incorrect addition.
The six class XML `staticData/baseMAtk` values are **6**, and each profile/vector
cites its source file. The equipped base is **5** (`stats/items/02300-02399.xml:863`).
The expected values use independent Fraction arithmetic: `5 * INT_bonus^2 * levelMod^2`,
and then one floor operation to Q. The whole-number floor is only evidence for the display. This is exact
rational evidence and the Q adaptation that exists. It does not say that the result is equivalent to Java binary64.
The input SHA-256s include the new weapon table, the six profiles, the stat-bonus table
and the oracle script. A separate canonical vector-set SHA-256 covers the expected values.

```sh
python3 -B packages/data/scripts/starter_weapon_oracle.py
python3 -B packages/data/scripts/starter_weapon_oracle.py --check
```

`check_all.py` now runs this independent oracle check together with the checks that
exist. `test_starter_weapon_phase3.py` audits these items:

- All weapon functions against the XML.
- The SET defaults/semantics against Java.
- The cap literals against the exact config lines.
- The weapon functions that are not there, not supported or not valid.
- The full requested coverage.
- The arithmetic of replacement against addition.
- The preservation of the historical tables/manifests.

### Additive Phase 3 NPC penalties and hit conditions

`gen_class_phase3.py` (through `gen_npc_phase3.py`) is also responsible for `tables-phase3/npc_level_penalty.toml` and
`tables-phase3/hit_condition_bonus.toml`. Hash all their
bytes separately into Phase3Rules. The historical tables, class profiles,
fixtures and oracle script stay byte-exact. Run the generator command that exists
with `--game`, `--datapack` and `--check`. `check_all.py` examines the two tables and
`scripts/npc_phase3/skill_oracle.py --check`. This separate oracle path keeps
the hash of `scripts/skill_oracle.py` that `fixtures/skill-oracle.json` has now.

The pinned game `src/main/resources/config/npc.properties:54,61-63,70,74` has these values:

- The minimum NPC levels **78/78**.
- The normal damage **0.7,0.6,0.6,0.55**.
- The critical damage **0.75,0.65,0.6,0.58**.
- The skill damage **0.8,0.7,0.65,0.62**.
- The magic failure multipliers **2.5,3.0,3.25,3.5**.

All source entries stay as exact decimal strings and Q integers with line
provenance. This includes the index zero entries that the code cannot get to. The source has an unusual index rule. The gap is the target level minus the acting
player level. Damage starts when the gap is >= **2**, and then it uses the index **gap-1**. Chance starts when the gap is >= **3**, and then it uses the
index **gap-2**. The two saturate at the final entry. Thus the effective rows are:

| Target minus acting-player level | Auto normal | Auto critical | Magic damage | Physical skill normal | Physical skill critical | Magic failure factor |
|---|---|---|---|---|---|---|
| 2 | 0.6 | 0.65 | 0.7 | 0.42 | 0.455 | 1 |
| 3 | 0.6 | 0.6 | 0.65 | 0.39 | 0.39 | 3.0 |
| 4 | 0.55 | 0.58 | 0.62 | 0.341 | 0.3596 | 3.25 |
| 5 and above | 0.55 | 0.58 | 0.62 | 0.341 | 0.3596 | 3.5 |

These rows come from the pinned game `model/stats/Formulas.java:713-731` (auto),
`:776-800` (physical skill, the skill factor times the normal/critical factor),
`:887-898` (magic damage) and `:1286-1314` (magic success). For eligibility, these conditions are also necessary:

- An attackable target that is not a raid and not a raid minion.
- A target level that is a minimum of the configured damage minimum.
- An acting player that is not null.

Physical skills use
`instanceof L2Attackable`. The other paths use `isAttackable`. Chance **also reads
MinNPCLevelForDmgPenalty**, thus MinNPCLevelForMagicPenalty is not used here
(`:1291`). If a gate fails, the factor is unity. Chance scales the **failure subtrahend**:
`100 - Math.round((float)(1.3^lvlDifference * targetModifier * resModifier))`,
and the success is strictly `Rnd.get(100) < rate` (`:1288-1301,1313`). The skill magic level
controls the exponential difference, but the acting-player level controls the
eligibility/index of the NPC penalty. The table lookup itself does not use RNG.

The pinned datapack `src/main/resources/data/stats/hitConditionBonus.xml:3-9`
has back **10**, dark **-10**, front **0**, high **3**, low **-3**, rain **-3**,
side **5** (percent additions). The game
`data/xml/impl/HitConditionBonusData.java:75-81,92-118` loads all seven rows.
It starts at **100**. It adds high only if the source Z difference is **>50**, and low only if it is **<-50**.
It adds dark at night. Then it adds exactly one of back/front/side, in that order of precedence. It returns
`max(mod/100,0)`. Rain is loaded, but the code does not use it (`:104-105`). Facing/height/night are
source inputs. They do not give permission to invent Phase 3 position/time mechanics. The
engine must explicitly decide which input capabilities it supports.

The separate `fixtures/skill-oracle-npc-phase3.json` has **42 vectors each**
for the three damage penalty stages, **21** magic chance penalty vectors, and
**60** hit-condition vectors. The coverage includes the skipped index zero, the saturation,
all eligibility guards and the threshold boundaries. It also includes normal/critical physical skills and
the strict boundary of the chance roll. It also includes the height boundaries **-51/-50/0/50/51**, day/night,
all facing branches, and rain with no effect. `scripts/npc_phase3/skill_oracle.py` makes the counts and the coverage.
The expected values use independent Fraction
arithmetic, with integer-only binary32 rounding for the isolated chance
factor. The damage vectors of the penalty stages show the exact incoming **1001** times the
multiplier and the evidence of whole truncation. They do not say that they are equivalent for the
earlier Java binary64 stages that are not modeled. The input byte hashes and a separate
canonical vector-set hash are recorded. No earlier fixture is written again.
`test_npc_phase3.py` independently audits the config/XML provenance, the source branches, the
effective mappings, each vector, the hashes, and all the bytes that were generated before the task.

## Phase 4 item rules and independent item oracle (V6 only)

### Pinned source and schema

The pinned source authority does not change: **datapack
`3ca488dd2bd0bfaca43e378886a3c2e37968153a`**, **game
`abfde0490ac52a2106e884b7242c0276c44cdf76`**. `scripts/gen_items.py`
checks the two HEADs. It rejects dirty tracked, untracked and ignored source inputs.

The [schema](tables-phase4/SCHEMA.md) describes each field/type/unit, the source
ordinal, the capability gate and the hash input. Decimal values are exact lexical strings.
The TOML files contain no floats. An Expat live-node parser removes the XML comments.
The source lines are parser positions. They are not the result of a text search, thus a comment cannot match by accident.
Duplicate source `<set>` properties use the last-write semantics of the pinned StatsSet.
They also stay in `raw` (example: item20994,
`stats/items/20900-20999.xml:1445-1461`). Unknown semantic dependencies become named
blockers. The generator never discards them.

### Source inventory and capability census

`tables-phase4/source_inventory.json` records the byte SHA-256 of each source file.
It also records each source item, its file:line and its semantic node hash. The recounted source inventory
is **3,893 Weapon / 3,825 Armor / 11,481 EtcItem**. The generator imports all Weapon/Armor templates.
The EtcItems are closed over these groups:

- Shots, arrows, bolts, enchant scrolls and support items.
- Crystals, stones/gemstones and currency.
- Soul stages.
- HP/MP/CP potions and elixirs.
- Escape scrolls and resurrection scrolls.
- All starter rows, and the inputs that learning and skills consume.

`tables-phase4/capability_counts.json` is the committed census of imported, admitted and deferred items.
It also has the Phase3 before/Phase4 after counts, the totals for each tree and the blockers of each changed skill.
The census does not state a target executable total. The generator loads missing custom skills
recursively only for V6. V5 keeps its earlier source inventory and generated bytes.

### D54 consumable admission closure

D54 adds an admission closure over the items that a skill consumes at the referenced rank.
Each positive cost target must be imported, storable and admitted. Deferral
propagates until it gets to a fixed point. When a target is missing, the closure fails closed.
Recipe ingredients and recipe products have the same storage/admission requirement.
`capabilities.toml` contains the named consumable blockers, and
`capability_counts.json` counts them. `rules_manifest.toml` binds their changed semantics.

Pinned counterexample:

- `data/stats/items/21000-21099.xml:718-735` attaches skill2864-1 to item21040.
- `data/stats/skills/02800-02899.xml:1560-1574` consumes14701×1 (`:1563-1564`).
- `data/stats/items/14700-14799.xml:20-36` marks14701 NPC-only (`:32`).
- Item21040 stays storable, but it is deferred with `consumable:skill:2864:1:item:14701`.

This is an admission rule. It does not use an engine special case. (For the D34/D47 promotion, refer to "Phase 4 capability promotions" below.)

The same rule also defers these items:

- Item21039 (`stats/items/21000-21099.xml:699-717`). Skill2863 consumes14700 at `stats/skills/02800-02899.xml:1539-1559`. Item14700 is NPC-only at `stats/items/14700-14799.xml:3-19`.
- Item20973 (item skill21197 at `stats/items/20900-20999.xml:1110-1125`). It consumes20976 at
  `stats/skills/21100-21199.xml:2317-2332`. The cost is time-limited at
  `stats/items/20900-20999.xml:1158-1168`.

The admission changes are exactly Armor1819→1818 and EtcItem372→370. These admitted counts do not change: Weapon907, skills1491, options23725,
augmentation27 and recipes170. The imported totals, the source inventory and the skill definitions do not change.

### Generated files

The generator writes these files:

- `weapons.toml`, `armors.toml`, `etc_items.toml`, `capabilities.toml`,
  `weapon_types.toml`, `grades.toml`, `capacities.toml`;
- `enchant_groups.toml`, `enchant_scrolls.toml`, `enchant_options.toml`,
  `func_enchant.toml`, `enchant_hp.toml`, `armor_sets.toml`;
- `augmentation_options.toml`, `augmentation_chances.toml`,
  `augmentation_skills.toml`, `sa_variants.toml`, `soul_crystals.toml`,
  `starter_kits.toml`, `angle_table.toml`, `rules_manifest.toml`.

### Loader defaults, functions, grades and slots

The loader defaults cite `game/model/items/L2Item.java:157-218`,
`L2Weapon.java:82-123`, `L2Armor.java:47-80` and `L2EtcItem.java:49-73`.
The functions keep the XML order and the exact priorities from
`game/enums/StatFunction.java:25-32`: ADD/SUB30, MUL/DIV20, SET/ENCHANT0,
ENCHANTHP40. The weapon masks/traits come from the enum
(`model/items/type/WeaponType.java:27-63`). They are not an invented universal table of weapon statistics.
The grade IDs, the crystal IDs and the coefficients come from
`model/items/type/CrystalType.java:26-33`. The S-plus collapse applies to enchant/shot
compatibility. It does not apply to expertise. The slot aliases/masks come from `datatables/ItemTable.java:74-109`
and `model/items/L2Item.java:70-103`.

### Enchant data

- The enchant rate ranges/groups are `datapack/enchantItemGroups.xml:4-92`.
- The scrolls and the support/allowed-item lists are live `enchantItemData.xml` nodes. The old crystal scrolls in comments are not included.
- The maximum validation is **current>max**, not current>=max
  (`model/items/enchant/AbstractEnchantItem.java:109-130`).
- The scroll type/blessed/safe flags come from EtcItemType (`EnchantScroll.java:49-56`). The weapon support type is
  `EnchantSupportItem.java:35-38`.
- The blacklist is `game/src/main/resources/config/character.properties:364`.
- FuncEnchant is `model/stats/functions/FuncEnchant.java:43-155`. Only the actual ENCHANT functions use it. This includes the sDef shield branch that has no effect.
- The HP bonuses are `datapack/stats/enchantHPBonus.xml:3-94`. They attach implicitly to graded armour through
  `game/data/xml/impl/EnchantItemHPBonusData.java:83-105`. The lookup uses S-plus,
  the cap of the last entry and the onepiece3/2 truncation (`:116-126`).
- The crystal refund on failure is `RequestEnchantItem.java:259-271`. The full crystal count for intentional crystallization is different
  (`model/items/L2Item.java:405-420`).

### Armour sets, starter kits and capacities

The generator keeps all live sets from all21 `datapack/stats/armorsets/*.xml` files.
This includes the alternatives, the direct base-stat deltas and the full/shield/enchant6 skills.
The set-shield activation keeps the mutation history. The generator does not derive it as full_set&&shield
(`model/itemcontainer/PcInventory.java:1198-1208,1232-1285`, D20).

The starter rows are exactly `stats/initialEquipment.xml:4-109`. This includes the counts, the equipped flags and the
tutorial/teleport extras that are not supported.

- Capacities80/100/100 and WeightLimit1 cite `character.properties:27,317-320`.
- Load69000, the penalty base1, the strict integer thresholds and the overload history for an unchanged rank cite
  `model/actor/instance/L2PcInstance.java:1668-1693,9427-9438`.
- Blacksmith(124,128), radius3000milli-tiles, tax0 and the glow tiers are explicit Nightfall design policy.
  They are not source retail claims. The manifest hashes all of them.

### Augmentation

Augmentation keeps all function/active/passive/attack/magic skill providers of `stats/options/*.xml`
and all rows of `stats/augmentation/augmentation_skillmap.xml`.

- The retail weapon/accessory table order does not change. The draw order is option12→category→option34.
- The selectors use the source **descending tail**, not an ascending CDF
  (`game/datatables/AugmentationData.java:447-559,689-737`).
- The category integer weights
  5500/3500/700/300 and5500/3500/900/100 implement the accepted A8 adaptation.
- The non-retail91/3640 offsets stay an explicitly inactive provenance policy
  (`:563-686`). The generator never mixes them into retail.
- The stone levels/tiers/IDs and the gem costs cite
  `network/clientpackets/AbstractRefinePacket.java:43-188,380-414`. The cancel fees change with the
  crystal count (`RequestRefineCancel.java:64-109`).
- Trigger option dependencies and option dependencies that are not implemented defer the full stone/weapon-type selector.
  A reroll or a probability rebalance is not permitted.
- The exact modulo-residue partitions include the final rows with a sum that is too low. The source still selects the final option for them.

### Special abilities, recipes and soul crystals

The SA lists are selected by the actual base Weapon + the explicit soul-crystal ingredient → the
Weapon product. The generator keeps all matching pinned blacksmith/Mammon lists, the ingredients/products,
and the maintainEnchantment and applyTaxes flags.

- The recipe IDs use
  `list_id*2^32 + zero_based_item_ordinal`. The NPC-list metadata is not an item ordinal.
- Each parent recipe has an admission gate over each ingredient and each product.
- The Stormbringer green-stage5 recipe is `multisell/1005.xml:31-41`. The Damascus red-stage10 recipe
  is `:2503-2513`.
- `levelUpCrystalData.xml:4-59` includes all transitions up to and including
  17→18, and all NPC detail data. The absorb policy is explicitly DeferredPhase6, with no RNG.

### V6 skill capability analysis

`gen_skills.py` gives an optional V6 capability analysis. Through `gen_items.py`, it writes only
`skills-phase4/`. It adds System4270/6209/6213/3599,
mastery/expertise/crystallize and all item/set/enchant4/option provider dependencies.
Then it adds the transitive references.

The V6 contract includes these items:

- The equipment kinds/slot.
- The load/inventory/posture/escape guards.
- FAN, FatalBlow/Lethal, Hp/Escape and the equipment stats.

The historical default analysis and the `skills/` bytes do not change. These items stay deferred: charges,
EnergyAttack, the scopes/filters/handlers/parameters that are not supported, and the skill enchant
routes. Admission is a **data evaluator contract**. Live runtime
support and independent review are separate release gates. A TOML with zero blockers does not prove them.

### Angle table

`angle_table.toml` certifies each integer angle0..360 for coordinate components up to the
bound256000milli-tiles.

- A Machin alternating rational pi and cosine Taylor intervals
  bound cos and cos².
- A continued-fraction proof with the minimal denominator excludes each
  rational in the squared interval with denominator<=4·bound⁴. This set contains all
  reachable values of dot²/(norm_f²·norm_v²).
- The special rational squared boundaries are exact.
  Coordinates that are not supported and angles that are not certified fail closed.
- The independent oracle checks with the separate identity pi=8atan(1/3)+4atan(1/7), and with an independent
  test of the nearest-rational denominator.
- The runtime uses no trig, no float and no sector approximation.
- The bounds and the intervals are hash inputs (D21).

### Independent item oracle

`scripts/item_oracle.py` reads the pinned XML and Java source files. It does not use the item generator.
It records the hashes of the files, and it writes `fixtures/item-oracle.json`.
The [actual-input functions](tables-phase4/ORACLE.md) use Fraction and injected residues.
Branch/boundary families add to the minimum24 named gear matrix.

- D17 has bow12499/12500/12501milli for distance89/90/90, the two stages101.99→101→90 and
  reuse1500·333/400=1248. The weapon-reuse modifier does not change this value.
- D24 asserts unarmed
  5+isqrt(level), bound2r+1 and the endpoints at1/3/4/8/9/15/16
  (`model/actor/L2Character.java:5034-5044`).
- D25 asserts per-recipient49.9→49→41,
  perfect1→0 secondary, exact85/(23/20) and the advancement on an eligible miss
  (`:1282-1283,1335-1338`).
- The chance vectors document the **quantized success set** and the
  u32 modulo bias. They do not document an identity of the continuous RNG distribution.
- The shot/shield callers keep their different formulas and early-return draws
  (`model/stats/Formulas.java:502-555,667-897,1027-1105,1621-1640`). This includes blow1.458,
  the shield denominator addition and the perfect residues91..99.
- The Heal branches cite
  `datapack/handlers/effecthandlers/instant/Heal.java:68-104`.

### Checks

Run `gen_items.py --check`, `item_oracle.py --check`, the Python unittest suite and
`scripts/check_all.py --datapack PATH --game PATH`. The two new checks are registered.

- Before the generator writes, it validates the equality of the parsed TOML. It also checks for stale and
  unexpected files.
- The data hashes normalize the semantic input, and they do not include provenance.
  The byte source hashes stay separate.
- The typed Phase4Rules hash of the runtime (design§1.2)
  includes the full Phase3Rules hash. An independent implementation must make and check it.
- Live received assertions, the byte preservation of the F13 carrier and the full size of the compressed V6
  snapshot stay integration gates. This data foundation claims none of them.

### Zone review follow-up

- The pinned selection handler admits the core EnchantScrolls entries
  (`datapack/handlers/itemhandlers/EnchantScrolls.java:32-49`).
  EnchantItem does the durable target operation.
- Static item-selector skills
  with targetNONE and **no effects in any section** are explicit V6 no-ops. An example is
  support2515 (`stats/skills/02500-02599.xml`). This exception admits no targetless skill that causes damage or an effect.
- Each item/skill that is not admitted now has a named
  blocker. This includes `sample:out_of_scope` of sample286.

These changes came from the first review of the generated tables by the
zone worker. They must have the same runtime validation.

### Final source audit

- The audit imports the Java-derived Common/hero/PvP identity flags directly
  from `model/items/L2Item.java:252-255`. It does not guess them from the names.
- Crystallizable uses grade!=NONE and a base crystal_count>0 (`:352-353`).
- `augmentation_chances.blacklist` is the complete configured target list
  (`character.properties:410`). `capacities.policy.allow_augment_pvp_items` keeps
  `:414`.
- The chance strings of the inactive skill/glow keep their pinned configuration.
  The alternate non-retail branch does not run (`:371-389`).

### SA foundation audit

change_weaponId is **not** an equip/stat/enchant/SA hook.
Its only behavior consumers are `datapack/handlers/effecthandlers/instant/ConvertItem.java:69-73`
and `game/model/conditions/ConditionChangeWeapon.java:49`.

- Keep the property.
- Keep ConvertItem and its condition deferred in the skill capability analysis.
- Hash `capacities.policy.weapon_conversion=deferred_ConvertItem_skill_mechanic`.

This admits the Stormbringer/Damascus base recipes and the Focus variant
recipes that are otherwise complete. Thus, the generator does not incorrectly defer their ordinary item operations.

### Architect decisions D33, D32, D27/D31

Architect D33 resolves Mana Potion728:

- The pinned `game/src/main/resources/config/customs.properties:234` is
  `EnableManaPotionSupport = False`.
- `datapack/handlers/itemhandlers/ManaPotion.java:30-32` refuses the use when the value is false.
- The NPC-only flag stays, and configuration:mana_potions_disabled is explicit.
- capacities.policy.mana_potion_support hashes the default.
- Player use is excluded in Phase4. This decision replaces the earlier inclusion in A14. The MP elixirs stay in scope.

D32 keeps the deferred trigger/effect option selectors, with no reroll and no rebalance.

D27/D31 make all V6 rules/skill definitions hash references to the tables that are embedded in the build.
They are not serialized into snapshots or records. The semantic hashes of each file
stay stable inputs. Runtime rules that do not match must cause admission and replay to reject.

### Data review revision 2 (same pinned DP/game revisions)

| Finding | Pinned evidence and corrected contract |
|---|---|
| 1 | `game/model/items/L2EtcItem.java:51-55`: the effective SHOT overrides the literal etcitem_type for four default actions. Ordinary Soulshot D1463 `data/stats/items/01400-01499.xml:627-638`, Spiritshot No Grade2509 `02500-02599.xml:210-220`, Blessed Spiritshot3947 `03900-03999.xml:861-874`, summon6645/6646/6647 `06600-06699.xml:900-937`. The literal properties and `raw` do not change. |
| 2 | `game/model/stats/Formulas.java:815-847`: the precedence of shield, PvP defence and shot. `:848-873`: failure with a second success for players only, a level difference≤9, and critical player-PvP5/2 against PvE3. `:876-896`: spread→PvP→attribute→resolved NPC penalty. The perfect return comes before the other inputs. |
| 3 | `datapack/handlers/effecthandlers/instant/Heal.java:68-104`: the exact dynamic/static branches and the clamp of recoverable HP. The Q restoration/remainder is the intentional Phase3 contract (`docs/plans/phase-3-zone-design.md:188`, line 128 before the STE100 rewrite). Phase4 §5.2 keeps it. The restoration is certified at Q before the transfer of whole units. |
| 4 | `data/stats/skills/08400-08499.xml:1906-1916`: skill8498 +2STR. `game/model/stats/BaseStats.java:80-85`: effective STR. `game/model/stats/functions/formulas/FuncPAtkMod.java:42-43`: derived bonus. `FuncArmorSet.java:46-67`: priority1 set attributes. The complete set interaction uses `data/stats/armorsets/a_grade.xml:37-52` (set41, STR+1/CON−1 and the two full-set skills). The attribute functions keep the priority, the install sequence and the XML order before the dependent stats. Stats and conditions that are not supported fail closed. |
| 5 | Phase4 D21/§5.3: the independent pi identity `pi/4=2atan(1/3)+atan(1/7)`, rational Taylor enclosures, nine exact squared boundaries and a separation with bounded denominators. The generated angles are only validation inputs. Mutations of the exact values, certificates, enclosures, separation and coordinate bounds cause a rejection. |
| 6 | Player pole15 `data/stats/items/00000-00099.xml:322-343` with skill3599 `stats/skills/03500-03599.xml:1695-1705`. NPC8205 `08200-08299.xml:76-94` causes a rejection. Scrolls/groups: `data/enchantItemData.xml:4-136`, `enchantItemGroups.xml:4-99`, `game/model/items/enchant/EnchantScroll.java:49-56,102-157`. Payment/reduction: `datapack/handlers/itemhandlers/SoulShots.java:61-98`, `SpiritShot.java:60-92`, `BlessedSpiritShot.java:63-95`. Consumable costs/lane: `ItemSkillsTemplate.java:81-149`, `game/model/actor/L2Character.java:1560-1566`. The §12.1 coverage gate must have the lifecycle/overlap, payment/shared-bundle and consume-once/reuse/lane vectors. It also must have a contiguous coverage of each scroll, group and current level. The integration owner keeps the live §12.2/12.4 gates. |
| 7 | Elixir2287–2289 `data/stats/skills/02200-02299.xml:2027-2082`, greater2860–2862 `02800-02899.xml:1473-1540`. `game/model/conditions/ConditionPlayerLevelRange.java:41-43`: an inclusive comparison of the effector level. There is one expanded range for each rank. The tests are below, at the two endpoints and above. D34 was promoted after the Z execution was published (4f9ac8c/c5229d4/f6d882b): all36 elixirs are admitted. capability_counts.required_condition_closure records the IDs/counts (36/36/0) and the publishing SHAs. |
| 8 | `datapack/handlers/itemhandlers/ManaPotion.java:30-32`: the guard for disabled support. `game/src/main/resources/config/customs.properties:234` stays False. |

In the table above, the paths that start with `game/` are under `src/main/java/com/l2jserver/gameserver/` of the game.
The paths that start with `datapack/handlers/` are under `src/main/java/com/l2jserver/datapack/handlers/` of the DP.
The paths that start with `data/` are under `src/main/resources/data/` of the DP. Adjacent XML basenames keep
the `data/stats/items/` or `data/stats/skills/` directory of that row.

### D35 class slot projection

`gen_items.py` reads the live `basePDef/baseMDef` nodes (15-31) of the nine starting-class
`data/stats/chars/baseStats/*.xml` files. Each row has its pinned provenance.
The generator writes `tables-phase4/class_slots.toml`.

- `L2PcTemplate.java:64-75` maps physical UNDER to the XML underwear. It intentionally
  maps LFINGER to RFINGER.
- `PlayerTemplateData.java:106-108` uses RFINGER two times in the total.
- Independent oracle vectors cover these cases: empty, each occupied slot and all slots. They also cover the
  fullarmor/onepiece legs subtraction against `FuncPDefMod.java:44-70` and
  `FuncMDefMod.java:44-66`.
- The manifest hashes the effective maps and the literal maps. The
  inventory ledger hashes each new source file. No historical class input changes.
- Two explicitly synthetic oracle vectors with unequal fingers (RFINGER=7, LFINGER=999)
  show the getter duplication, although all nine pinned XML pairs are equal.
  They keep the complete overridden XML as inputs. The coverage must have them.

### D75 class unarmed combat bases

`gen_items.py` reads each
`data/stats/chars/baseStats/*.xml` file and `data/stats/chars/classList.xml`. It writes
`tables-phase4/class_combat.toml` for the nine starting classes (`baseAtkType`,
`baseAtkRange`, `basePAtk`, `basePAtkSpd`, `baseCritRate`; lines 11-14,33 of each
base XML). It fails closed if a descendant does not have the same values as its base
(census of 89 classes).

- The unarmed reach is 20. The exception is the two orc lineages (25,
  `OrcFighter.xml:33`, `OrcMystic.xml:33`).
- Game citations:
  `CharStat.java:424-435` (no-weapon range), `:438-447` (no-weapon angle 120),
  `:145,399,410`, `L2Character.java:5034-5044` (radius 5+isqrt(level)) and
  `:5560-5572`, `L2CharTemplate.java:107-118` defaults, `PlayerTemplateData.java:98-103`,
  `L2PcInstance.java:1018-1019,7668-7674`.
- The generator uses a regex to verify the literals 120 and
  5 in the pinned Java.
- `item_oracle.py` adds 90 independent F1 vectors. There is one
  `class-combat-<classId>-unarmed` vector for each of the 89 lineage classes, selected from the
  class graph. The vector `class-combat-44-armed-2369` shows that the weapon value 40 overrides
  the orc value 25. `item_coverage.py` must have each vector.

### Phase 4 capability promotions

These promotions are for V6 only. The V5/historical allow-lists do not change.
The documented `gen_items.py` command makes the tables again. Never edit them by hand.

- D34 PvP: `gen_skills.PHASE4_STATS` adds `pvpPhysDmg`, `pvpPhysSkillsDmg` and
  `pvpMagicalDmg` (`data/stats/skills/03500-03599.xml:679-695`, skill3552 muls
  691-693; Z callers 4f9ac8c, FatalBlow bfc00a7). Admitted deltas: Weapon +629,
  Armor +16, skills +47, options +39, recipes +755.
- D34 levelRange: the V6 `is_cond_allowed` admits a `player` leaf when its only attribute
  is `levelRange` (`#table` or `min;max`; `parse_cond_node` validates the ranks;
  `ConditionPlayerLevelRange.java:41-43`; Z 4f9ac8c/c5229d4/f6d882b). Deltas:
  EtcItem +46 and skills +21. The EtcItem delta is the 36 necessary elixirs 8622-8639/14682-14699, and also 20354-20363
  "Secret medicine of Will/Life". The only blocker of these items was the same leaf.
- D47: `PHASE4_STATS` adds `enlargeAbnormalSlot` (skill1405
  `data/stats/skills/01400-01499.xml:264-277`, stat at 274; Z 402e554). Skills +1
  (1405). Books 8618-8621 (`data/stats/items/08600-08699.xml:170-200`; learn costs
  `data/skillTrees/classSkillTree.xml:172-183`) were already admitted learn inputs.
  They keep `use:no_handler`, and the item counts do not change.
- D50: `PHASE4_EFFECT_PARAMS["FatalBlow"]` adds `blowChance: int`
  (`datapack/handlers/effecthandlers/instant/FatalBlow.java:43-46`: double power,
  int blowChance/criticalChance). The params at Lethal.java:42-43 and the `blowRate` stat
  were already represented (Z runtime bfc00a7). The `params` of each skill already had the parameter value.
  The change gives it a type and validates it. Skills +2
  (16 Mortal Blow, 263 Deadly Blow). The item/option/recipe counts do not change, because no item
  had only these blockers. 409 keeps `section:selfEffects`, and 30 keeps
  `effect:Backstab`.

Each delta is equal to the read-only preview of revision 7. This includes the item and skill identities
(691 items, 71 skills; no demotions).

### Data review revision 4

`scripts/item_coverage.py` independently lists the
§12.1 requirements from the design boundaries and the pinned source domains.
`make_vectors`, the generated fixtures and their counts are never requirement inputs.

- Each case binds its family and its necessary input dimensions. These include:
  - The exact starter kit XML order/count/equipped flags, and each scroll/group/rate interval.
  - Each CON/grade/HP row, each retail selector and each cancellation cut point.
  - Each SA ingredient/product row and each soul transition.
- Each necessary case and each separate mandatory family has deletion
  mutations. The necessary input dimensions have deletion/type/value mutations.
- The added enchant cases for the bow/crossbow bodypart and the shield distinction cite
  `FuncEnchant.java:43-155` (bow/crossbow checks at 105-107, 123-125, 143-145).
- All eleven source starter kits cite `data/stats/initialEquipment.xml:4-109`.
  The nine supported races get effective-stat vectors. These vectors cite their class XML
  and the existing slot/stat formulas.
- The two Kamael kits are only source evidence, and they stay deferred.
- `ClassId.java:185-186` contains hex IDs with mixed case. The parser now reads them,
  and it does not drop the female starter row.
- The angle vectors are part of `make_vectors`. Thus, each coverage caller sees the same mandatory positional evidence.

### Data review revision 5

`scripts/item_behavior_oracle.py` adds the missing
§12.1 state-transition evidence. `item_coverage.py` lists its source/input domains separately.

- The bow definitions bind the actual XML `mp_consume`,
  `reduced_mp_consume` and random-damage radius (`L2Weapon.java:90-106`).
- The arrow search collapses S80/S84 to S (`Inventory.java:679-694`, `L2Item.java:387-391`).
  The pinned source trusts an occupied LHAND (`L2PcInstance.java:5334-5360`).
- A refill with the wrong grade causes a rejection. The added row for an occupied slot with the wrong grade intentionally
  records that source quirk. It does not give permission to bypass the V6 equipment validation.
- The arrow availability and the reuse come before the reduction RNG. When the MP is not sufficient, the
  attempt draw stays (`L2Character.java:724-757`).
- Each accepted hit or miss consumes one
  arrow. An empty stack clears LHAND (`:1012-1021`, `L2PcInstance.java:5280-5323`).
- V6 moves the projectile payment to the accepted start (§2.1). The vectors identify this
  adaptation of the schedule.
- The isolated hit cases use caller inputs with no shield, no critical and a neutral spread.
  The hit equality stays (`Formulas.java:1054-1063`).

Scroll737 binds skill2014 (`data/stats/skills/02000-02099.xml:189-211`), a 15000ms
static completion and itemConsume737×1.

- Its Resurrection effect does not give a power.
  `Resurrection.java:37-41` sets the default to zero, and this bypasses the WIT scaling
  (`Formulas.java:1367-1382`).
- The scroll is consumed one time before the offer. It is
  never refunded on a decline or an expiry (§8).
- These items come from `docs/plans/phase-3-zone-design.md:192` (line 132 before the STE100 rewrite):
  the 600-tick offer lifetime, the retention of the first valid offer,
  the offer-ID/death-incarnation fence and the duplicate rejection. They are not a claimed Java timeout.
- The acceptance uses `character.properties:41` HP65%. The source MP/CP configuration is
  zero, and the MP/CP do not change (`L2Character.java:2098-2132`). The Q conversion is the P3
  resource representation.
- Tutorial5588 and scrolls10650/12753 keep the exact counts
  from `initialEquipment.xml:4-109`. They reject their wider tutorial/teleport
  capabilities, and they do not change the inventory, allocator, reuse, paperdoll or RNG
  (D12, §9.1).
- The two Kamael classes that are not supported stay only in the source census.

The line-by-line audit also must have these items:

- The safe+3/fullarmor+4 source rates
  (`enchantItemGroups.xml:4-59`).
- Exhaustive comparisons of the quantized chance, and the weight
  limit modifiers.
- The expertise/clamp of each paperdoll slot, and the bow reuse that changes with the speed.
- The dual canonical second resolution, the pole count bounds, and the blow minimum/caller
  rounding.
- The retail-only draw contracts.
- All option function/active/passive and
  attack/critical/magic trigger forms (`Options.java:61-74,136-178`).
- The itemConsume3031×1 of the learned skill1401.
- The order of regeneration, status and HP potion.

The source amount435 of Potion1540 comes from skill2038. `Hp.java:60-89` gives
its dead/status guard and the clamp of recoverable HP. Regeneration uses the class XML,
the CON bonus, the posture and the overload modifiers (`Formulas.java:263-351`,
skill4270 at `04200-04299.xml:1623-1639`) before the item restoration under §2.1.

- Each new family/boundary and all necessary input dimensions have deletion/value
  mutations. The existing revision-4 tests also visit each new necessary case.
- Explicit Escape timing vectors calculate the normal/blessed source animation, the launch,
  the completion and the interrupt cutoff at two casting speeds. They keep the resources and the
  incarnation at destination(126,126) (`L2Character.java:1625-1643,4577-4581`, §8).
- Added HP-potion interaction cases keep the regeneration, but they skip the
  restoration of the item for invulnerability and for an HP-blocked status (`Hp.java:60-89`).

### Data review revision 6

Escape and the consumable sequences use the same P3 launch/impact
staging (`docs/plans/phase-3-zone-design.md:174`, line 126 before the STE100 rewrite; `L2Character.java:1643,4577`).

- The 200ms animation of the blessed skill2036 (`02000-02099.xml:440-447`) launches immediately
  and impacts after 400ms: start10 completes14.
- The normal Escape stays210. Potions with zero time keep their minimum of one tick.
- All 23 positional/boundary blow rows write
  the unconditional blow draw. This includes the zero rates and the certain rates (`Formulas.java:1640`).
- Each row binds the site, bound1000 and the injected residue. It cites the §5 per-mille adaptation
  at `docs/plans/phase-4-zone-design.md:269`: `roll1000*Q < rate_q*10`. Under A8, this replaces the
  integer100 resolution of the source.
- The independent coverage requirements reject
  traces that are absent, empty or wrong, and missing source/adaptation citations.
- The tests mutate
  each draw field, and they independently assert the outcomes and the completion deadlines.
