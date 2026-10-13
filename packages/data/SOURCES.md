# Data sources

Provenance for the generated stat tables (Phase 1 plan §2 fidelity gate, Story E1.1).
`tables/*.toml` and `classes/*.toml` are generated; `zones/` and `npcs/` are hand-written.

## Phase 3 peace geometry (Nightfall configuration)

`zones/test_zone_phase3.toml` is a hand-written, schema-1 additive input. Its
`[provenance].kind = "Nightfall configuration"` distinguishes architect policy
from pinned L2J values. Architecture decisions §6 R7 / §7 D-R11 set a configurable
inclusive square with half-width **10,000 milli-tiles**, centred on the legacy
safe point **(126,126)** (`zones/test_zone.toml:33`). Its inclusive edges are
**(116,116)..(136,136)** whole tiles and contain the Class Master **(126,128)**
(`zones/test_zone.toml:28`); legacy bounds are **(0,0)..(256,256)** (`:9–10`).

Schema: top-level `schema_version = 1`, `id = "test_zone"`; `[peace_zone]`
contains `shape = "square"`, `centre = [126,126]` in whole tiles,
`half_width_millitiles = 10000`, `inclusive = true`; `[provenance]` contains
`kind`, `decision`, and `safe_point` citations. Width is configurable, not a
source-derived constant. `check_all.py` parses both files, requires centre to
equal safe_point, validates integer units, bounds containment and inclusive
Class Master containment, and prints SHA-256 of the new file's complete bytes.

The engine must include this separate input in Phase3Rules hashing and embed it
in its Phase 3 input inventory. Do not include it in the legacy zone config hash
or load it as a legacy zone file. `zones/test_zone.toml` bytes and historical
oracle manifests stay unchanged. No generator owns this geometry file.

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

## Phase 2 class and race catalog provenance

Provenance and reproduction for the Phase 2 race, profession, and growth catalogs. The catalog expands beyond Phase 1's 9 starting classes (`classes/*.toml`) to encompass all 89 classic professions across tiers 0..=3, 5 classic playable races, and 89 per-class growth tables, while preserving Phase 1 tables, starter classes, and fidelity oracles unchanged.

### Pinned High Five source

| Repository | Revision | Role |
|---|---|---|
| <https://bitbucket.org/l2jserver/l2j-server-datapack> (`develop`) | `3ca488dd2bd0bfaca43e378886a3c2e37968153a` (2026-09-13) | Class definitions, static attributes, skill trees, and per-level resource growth |
| <https://bitbucket.org/l2jserver/l2j-server-game> (`develop`) | `abfde0490ac52a2106e884b7242c0276c44cdf76` (2026-09-13) | CP CON-multiplier (`FuncMaxCpMul`) and integer floor (`PcStat.getMaxCp`) |

Datapack files read (prefix `src/main/resources/data/stats/chars/`):
- `classList.xml`: class IDs, parent links, and retail class names.
- `src/main/resources/data/skillTrees/classSkillTree.xml`: level/SP gates, automatic grants, NPC learning flags, item requirements, Expertise and equipment mastery references.
- `src/main/resources/data/stats/skills/*.xml`: targeted skill ID/name/level-range metadata for validating learning references, without copying skill effects.
- `baseStats/*.xml`: 89 classic class XML templates containing `<staticData>` (base stats, movement speeds, collision boxes, environmental traits) and `<lvlUpgainData>` (85 per-level HP, MP, and CP rows).
- Base fighter XML templates (`HumanFighter.xml`, `ElvenFighter.xml`, `DarkFighter.xml`, `OrcFighter.xml`, `DwarvenFighter.xml`) providing baseline race movement, collision, and traits.

### Generated catalog directories

| Directory | Count / Schema | Contents |
|---|---|---|
| `packages/data/races/*.toml` | 5 TOML files | 5 classic playable races (`human`, `elf`, `dark_elf`, `orc`, `dwarf`). Contains racial fighter-baseline movement speeds, decimal collision strings, environmental traits (`breath`, `safe_fall`), and racial passive skill descriptor keys. |
| `packages/data/professions/*.toml` | 89 TOML files | 89 classic classes across tiers 0..=3 (tier distribution 9-18-31-31; IDs 0..=57 and 88..=118). Contains base stats summing to 170, class-specific movement, collision in integer world units at Q = 1,000,000, transfer token requirements, and subclass configuration. Reserved Kamael IDs 123..=136 are absent. |
| `packages/data/skill_catalog.toml` | 378 targeted skill definitions | Source skill IDs, maximum levels, reference names and definition paths; runtime status is explicitly deferred. |
| `packages/data/growth/*.csv` | 89 CSV files | Per-class resource growth for levels 1..=85. Each file contains header `level,hp,mp,cp` followed by 85 rows: 7,565 rows total, 22,695 exact HP/MP/CP plain decimal scalars (up to 6 decimal places, exact at Q = 1e-6). |

### Regenerating and verifying

```bash
git clone -b develop https://bitbucket.org/l2jserver/l2j-server-datapack.git /tmp/phase2-l2j-dp
git -C /tmp/phase2-l2j-dp checkout 3ca488dd2bd0bfaca43e378886a3c2e37968153a
python3 packages/data/scripts/gen_classes.py --datapack /tmp/phase2-l2j-dp --check
```

The generation script `packages/data/scripts/gen_classes.py`:
- Validates the exact pinned revision `3ca488dd2bd0bfaca43e378886a3c2e37968153a` and refuses modified, untracked, or ignored files in the character, skill-definition, and learning-tree source paths.
- Extracts XML source strings verbatim, formatting plain decimals without floating-point conversion.
- Generates all 5 race files, 89 profession files, 89 growth CSV files and the targeted skill reference catalog.
- Refuses unknown skill IDs, source levels outside their definition range, duplicate learning/proficiency rows and unexpected files in generated directories during `--check`.
- In `--check` mode, verifies that committed data files in `packages/data/` (`races/`, `professions/`, `growth/`) AND the compiled Rust inventory file at `apps/api/src/infrastructure/class_data_embedded.rs` match generated contents byte-for-byte.

### Data extraction and lineage fidelity

- **XML source exact strings**: Base stats, movement speeds, collision boxes, and `lvlUpgainData` values are transcribed directly as exact plain decimal strings (up to 6 fractional decimal digits, exact at Q = 1,000,000). Float literals are forbidden.
- **Retail parent errata**: Retail `classList.xml` contains two defective parent links that break class lineage trees, corrected via `PARENT_ERRATA`:
  - Class 34 (`Bladedancer` / `Edge Cantor`): retail lists `parentClassId="33"` (Shillien Knight). Corrected to `parentClassId="32"` (Palus Knight).
  - Class 104 (`Elemental Master` / `Tide Sovereign`): retail lists `parentClassId="26"` (Elven Wizard). Corrected to `parentClassId="28"` (Elemental Summoner).
- **Fixed 170 stat sum**: For all 89 classic classes, the six base stats (`STR + DEX + CON + INT + WIT + MEN`) sum to exactly 170.
- **Lineage consistency**: For all non-root classes (tiers 1..=3), all combat-sourced static attributes match the lineage root class: `race`, `archetype`, `base_class_id`, and all six `base_stats` are identical to the root starter class.
- **Source-explicit growth necessity (240 vs 27 curves)**:
  - The 9 starter XML templates have 27 curves (9 classes × 3 resources HP/MP/CP), all of which are exact quadratics (`base + per_level*(L-1) + accel*(L-1)^2`).
  - Across all 89 classes (267 curves total), 240 curves are non-quadratic due to transfer discontinuities and growth shifts around the level 20 and 40 transfer thresholds. Only the 27 base curves are quadratic.
  - Storing explicit 85-row source tables (7,565 rows, 22,695 decimal scalars) is strictly necessary to maintain source fidelity across the entire catalog; single quadratic formulas cannot represent advancing professions.

### Movement and collision semantics

- **Racial baseline vs class-specific attributes**:
  - `RaceDef` (and `RaceInfo`): Movement speeds (`walk`, `run`, `swim`) and collision dimensions (`radius_male`, `radius_female`, `height_male`, `height_female`) record the fighter baseline for that race (read from `HumanFighter.xml`, `ElvenFighter.xml`, etc.).
  - `ClassDef`: Records the actual class movement and collision from each class's XML. Human Mystic (class ID 10) and Orc Mystic (class ID 49) differ from their racial fighter baselines (e.g. Human Mystic walk/run 78/120 vs Human Fighter 80/115; collision dimensions differ accordingly).
- **Unit representations**:
  - In race files (`races/*.toml`): Collision dimensions are stored as exact decimal strings in reference world units (e.g. `radius_male = "9.0"`, `height_male = "23"`).
  - In profession files (`professions/*.toml`): Collision dimensions are stored as integers at fixed-point Q = 1,000,000 world units (`Scaled`, e.g. `radius_male = 9000000`, `height_male = 23000000`).
  - Movement speeds are whole integers in reference world units per second.

### Start points, reference zones, and naming conventions

- **Start points and fixture zones**: `start_points` in `races/*.toml` are set to fixture test coordinates `[[0, 0]]` within `test_zone` (with respawn safe point `[126, 126]`).
- **Reference starting zones**: `reference_starting_zone` (`talking_island_village`, `elven_village`, `dark_elf_village`, `orc_village`, `dwarven_village`) is purely informational metadata pointing to classic starter villages; these are NOT implemented villages.
- **Naming identity**:
  - `l2_ref`: Retains exact, unmodified retail names (e.g. "Human Fighter", "Warrior", "Gladiator", "Duelist").
  - `display_name`: Curated original draft mutable metadata for Nightfall (e.g. "Human Armsbearer", "Steel Initiate", "Twinblade Champion", "Blade Paragon").
  - The 89 original Nightfall labels and fixture starting positions are design metadata, separate from the cited retail values.

### Deferred hooks and future phase boundaries

- **Class skill tree and proficiencies**: `SkillLearnDef` (`skill_tree`) contains the complete source learning rows for all 9 base classes, 18 first classes, and the 12 second-class MVP professions (`[2, 5, 8, 9, 12, 16, 17, 27, 33, 46, 52, 55]`, 6,927 entries total). The 50 other direct trees are marked `deferred`, while ancestor entries remain queryable. `ProficiencyDef` records 3,521 source equipment mastery and Expertise unlock rows across all 89 classes. Source-exact learning level gates (`required_level` / `getLevel`) govern learning availability, not active class skill effects.
- **Automatic metadata grants**: Sourced `autoGet` entries grant persisted metadata on character creation, transfer, or level-up (max level per key at zero SP charge). Manual learning flows, SP expenditures, and skill effects are Phase 3 excluded.
- **Item requirement hooks**: 48 learning entries require items; keys use canonical `l2.item.<id>` with a positive unsigned 32-bit ID, pointing to future inventory item definitions.
- **Known skill reference catalog**: `packages/data/skill_catalog.toml` defines 378 targeted skill entries with known IDs, maximum levels, and source XML paths; runtime status is strictly `Deferred` (runtime engine absent).
- **Racial passive keys**: `passive_skill_keys` in `races/*.toml` (e.g. `racial.adaptable`, `racial.forest_step`, `racial.shadow_precision`, `racial.iron_constitution`, `racial.pack_mule`) are descriptor strings gated behind the future Phase 3 skill registry. Server runtime effects are defined by progression and combat rules.
- **Reserved Kamael IDs**: Classic catalog IDs include 0..=57 and 88..=118; Kamael IDs 123..=136 are absent (Nightfall has no Kamael race).

### Level progression, transfer gates, and subclass eligibility

- **Level cap**: Authoritative source level cap and playable level cap remain 85, preserving Phase 1.
- **Transfer gate**: Catalog files specify transfer token requirements across tiers 1..=3 (`class_transfer_token_1`, `class_transfer_token_2`, `class_transfer_token_3`). However, the application transfer gate is capped at transfer 2. Third-tier professions (tier 3, min level 76, IDs 88..=118) exist in catalog metadata and validation but are currently unreachable in gameplay under the transfer gate.
- **Subclass eligibility model** (`apps/api/src/domain/subclass.rs`):
  - Pure eligibility verification model without mutating character state or learning skills; village-master dialogue and interaction flows are deferred.
  - Subclasses start at level 40 (`SUBCLASS_START_LEVEL = 40`).
  - Subclass level cap is authoritative main level cap minus 5 (`subclass_level_cap(85) = 80`).
  - Up to 3 additional subclass slots per character (`MAX_SUBCLASSES = 3`).
  - Certification limits: 4 certifications per subclass slot (`CERTIFICATIONS_PER_SUBCLASS = 4`), up to 12 total across all three slots (`MAX_CERTIFICATIONS = 12`).
  - Subclass eligibility criteria:
    - Main class must have completed the second transfer (tier ≥ 2).
    - Main level must be at least 75.
    - Subclass quest completed (`quest_completed = true`) or noble status (`noble = true`).
    - Additional subclass slots not exhausted (< 3).
    - Candidate profession must be tier 2 and permitted (`subclass_allowed = true`; Overlord ID 51 and Warsmith ID 57 are forbidden).
    - Racial restriction: Elf and Dark Elf lineages cannot cross.
    - Equivalence restriction: Candidate cannot match or share an equivalence set with any held profession (main or existing subclasses) via `subclass_equivalents`.

### Registry validation, canonical hashing, and Phase 1 preservation

- **Startup loader and deserialization validation**: `infrastructure::class_data::load_classes` loads all catalog files at startup. `ClassRegistry` serde deserialization (`try_from = "RegistryParts"`) re-executes `validate()`, guaranteeing that deserializing saved snapshots revalidates all catalog invariants (IDs, stat totals, tier gates, parent links, non-decreasing growth, subclass configurations, collision positivity, strict nested fields, and known skill ID/level bounds).
- **Canonical config hash**: `ResolvedClasses.config_hash` is a SHA-256 digest over the canonical JSON encoding of `(&registry, &provenance)`. It tracks all resolved metadata, source provenance, and growth tables, while remaining insensitive to TOML whitespace/comments and equivalent accepted decimal representations. CSV headers and contiguous four-column rows remain strict; comments and whitespace outside plain decimal cells are rejected.
- **Phase 1 preservation**: Phase 1 stat tables (`tables/*.toml`), starter classes (`classes/*.toml`), formula tests (`formula_fidelity`), and independent verification fixtures (`oracle.py`, `oracle.json`, `rounding-audit.json`) remain completely intact and unchanged. The 89 original curated draft Nightfall display names are separate mutable design metadata, while retail names remain in `l2_ref`. Fixture start positions remain `[0, 0]`.

### Independent reviews, verification boundaries, and catalog size

- **Independent read-only review v1**: Review of `53be185` verified all 89 classes, 5 races, 7,565 rows and 22,695 exact resource values. It identified three admission bugs:
  1. *Class collision dimensions*: `validate_class` checked movement but omitted positive collision dimensions.
  2. *Nested unknown fields*: nested `BaseStats`, `RaceDef`, and `GrowthRow` lacked `deny_unknown_fields`.
  3. *Generator `--check`*: verified expected files without comparing directory inventories (orphan file miss).
  Commit `e02ee69` resolved all three (12 catalog tests, all-targets clippy `-D warnings`, fmt, tests, and gen check passing).
- **Independent field-tuple audit v2**: Audited `e02ee69`, independently extracting and verifying all 6,927 learning rows (SP costs, default bools, NPC learning, 48 item requirements), 3,521 proficiency rows, 378 known source definitions (runtime status `Deferred`), 89 classes, and 7,565 growth rows (22,695 scalars).
- **Review findings and disposition**: The second audit found two admission gaps: a pinned Git HEAD permitted modified source inputs, and nonempty item references could contain invalid retail IDs. Follow-up validation now rejects dirty source input paths and accepts only canonical positive `l2.item.<u32>` IDs for learning requirements. Tests cover source revision/working-tree admission and malformed learning references in both startup loading and snapshot deserialization. The follow-up passed 13 Rust catalog tests, 3 Python provenance tests, all-targets clippy with warnings denied, formatting, and source regeneration checks. The independent audit verified the shipped tuples before these fixes; the fixes have separate regression coverage.
- **Catalog size and snapshot integration**: Independent Python compact JSON reconstruction across all resolved catalog fields yields 2,048,521 raw bytes (~2.05 MB; zlib fast 207,161 bytes, default 156,512 bytes). This is an independent approximate-equivalent measurement of the catalog encoding, rather than a full zone checkpoint, and motivates compressed checkpoint storage.


### CP maximum multiplier

The pinned game source at `abfde0490ac52a2106e884b7242c0276c44cdf76` uses the same
CON multiplier for CP as HP. `FuncMaxCpMul.calc` multiplies the template's CP row
by `BaseStats.CON.calcBonus(effector)`; `PcStat.getMaxCp` casts the resulting
positive value to an integer. Nightfall therefore floors the exact CP × CON product,
using the existing Q arithmetic and no binary floating point. Sources:
[FuncMaxCpMul.java](https://raw.githubusercontent.com/monkey1sai/l2j-server-game/abfde0490ac52a2106e884b7242c0276c44cdf76/src/main/java/com/l2jserver/gameserver/model/stats/functions/formulas/FuncMaxCpMul.java)
and [PcStat.java](https://raw.githubusercontent.com/monkey1sai/l2j-server-game/abfde0490ac52a2106e884b7242c0276c44cdf76/src/main/java/com/l2jserver/gameserver/model/actor/stat/PcStat.java).
The Phase 2 catalog stores pre-multiplier CP, including fractional source rows; the
server applies CON when it builds the active class sheet.

### Source learning and proficiency coverage

The populated second-class IDs are **2, 5, 8, 9, 12, 16, 17, 27, 33, 46, 52, 55**,
as specified by the Phase 2 MVP slice. Together with all bases and first transfers,
these provide 39 direct learning trees and 6,927 source entries. Each entry retains
`skillId`, `skillLvl`, `getLevel`, `levelUpSp` (zero when absent), `autoGet`,
`learnedByNpc`, and optional item ID/count requirements. Stable keys are
`l2.skill.<id>` and `l2.item.<id>`; source names remain `l2_ref`.

All 89 classes carry their source equipment mastery and Expertise rows as proficiency
metadata (3,521 entries), inherited through the corrected class graph. Expertise 239
unlocks at 20/40/52/61/76/80/84 for levels 1..7. Equipment mastery names are explicitly
selected; Cubic Mastery, Skill Mastery and Focus Skill Mastery are outside equipment
proficiency scope. The 378 distinct referenced skills are cross-checked against the
pinned skill-definition XML IDs and maximum levels and serialized with the registry.
Every known skill's runtime status remains `deferred`: metadata grants do not claim
that the Phase 3 effects engine exists. Automatic grants may persist progress at zero
SP cost; manual learning and effect execution remain future work.

## Phase 3 skill data, pvp, regen, and keltir sp reward

Provenance and reproduction for the Phase 3 skill definitions, capability executability classification, PvP/karma tables, resource regeneration rules, and starter NPC SP reward.

### Pinned High Five source

| Repository | Revision | Role |
|---|---|---|
| <https://bitbucket.org/l2jserver/l2j-server-datapack> (`develop`) | `3ca488dd2bd0bfaca43e378886a3c2e37968153a` (2026-09-13) | Skill definitions, NPC templates, karma increase multipliers |
| <https://bitbucket.org/l2jserver/l2j-server-game> (`develop`) | `abfde0490ac52a2106e884b7242c0276c44cdf76` (2026-09-13) | Formulas for skill damage, success rates, traits, interruption, resurrection, regeneration, karma gain/lost |

Files read:
- Datapack (`src/main/resources/data/`):
  - `stats/skills/*.xml`: All targeted skill definitions (378 catalog skills + 5076 + 9 transitive referenced skills + 286 sample).
  - `stats/npcs/20500-20599.xml`: NPC 20545 (`Young Keltir`) SP reward (`sp="2"`).
  - `stats/chars/pcKarmaIncrease.xml`: 85 per-level karma loss multipliers.
- Game (`src/main/java/com/l2jserver/gameserver/`):
  - `model/stats/Formulas.java`:
    - `calcSkillPhysDam`: Physical skill damage and critical hits (`lines 746-809`).
    - `calcMagicDam`: Magical skill damage and magic critical hits (`lines 815-898`).
    - `calcMagicSuccess`: Success rate for magic attacks (`lines 1286-1314`).
    - `calcEffectSuccess`: Success rate for debuffs and abnormal effects with traits and mAtkMod (`lines 1158-1232`).
    - `calcAtkBreak`: Damage interruption rate for physical/magic casting (`lines 992-1025`).
    - `calcAttributeBonus`: Elemental attribute power vs defence multiplier (`lines 1440-1560`).
    - `calcHpRegen`, `calcMpRegen`, `calcCpRegen`: Resource regeneration rates and multipliers (`lines 263-460`).
    - `HP_REGENERATE_PERIOD = 3000`: 3000 ms (30 ticks at 100 ms/tick), `line 109`.
    - `calculateSkillResurrectRestorePercent`: Pinned resurrection XP restoration formula (`lines 1372-1386`).
    - `calculateKarmaGain`, `calculateKarmaLost`: Karma gain upon player kill and karma loss upon XP gain (`lines 1814-1848`).
  - `model/actor/L2Character.java`:
    - `doRevive()`: Base revival vitals (`lines 2098-2124`).
  - `model/actor/instance/L2PcInstance.java`:
    - `doRevive(double revivePower)`: Player revival calls doRevive() then restoreExp (`lines 7984-7986`); XP refund Math.round at `4700-4702`.

### Generated skill definitions and catalogue

- `packages/data/skills/*.toml`: 389 one-file-per-skill definitions generated by `packages/data/scripts/gen_skills.py`.
  - 378 catalog skills referenced by the 39 populated class learning trees.
  - Skill 5076 (`Death Penalty`, passive).
  - 9 transitive referenced skills discovered through effect parameters (`997, 5603, 5604, 5676, 5677, 5678, 6090, 6687, 6920`).
  - Skill 286 (`Provoke`, sample-only reference, marked `sample_only = true`).
  - Classification: **208 executable**, **181 deferred** overall (**199 executable**, **179 deferred** among the 378 catalogue skills).
  - Executability evaluated per Tier B capability rules (zone-design-v3 §1.1). `TargetMe` is explicitly deferred with `effect:TargetMe`.
- `packages/data/skill_catalog.toml`: Regenerated with computed `runtime_status` (`"executable"` or `"deferred"`) matching each skill's blocker analysis.

### Tables and NPC updates

- `packages/data/npcs/keltir.toml`: Added `sp_reward = 2` sourced from pinned Young Keltir NPC 20545 (`D/src/main/resources/data/stats/npcs/20500-20599.xml`). `xp_reward = 28` remains tunable.
- `packages/data/tables/pvp.toml`:
  - Flag durations: `pvp_flag_ticks = 1200` (120 s normal), `pvp_flag_low_ticks = 600` (60 s when attacking purple).
  - Karma gain constants from `Formulas.calculateKarmaGain` (lines 1830-1848).
  - Karma lost divisor `30` and all 85 rows of `pcKarmaIncrease.xml` as exact decimal strings (levels 1..=85).
  - Drop rules on chaotic death are explicitly omitted (not part of Phase 3).
- `packages/data/tables/regen.toml`:
  - Period: `hp_regenerate_period_ms = 3000` (30 ticks).
  - Movement multipliers: `sitting = "1.5"`, `standing = "1.1"`, `moving = "0.7"`.
  - Stat dependencies: CON for HP/CP, MEN for MP.
  - Base regen curve source: class templates (`packages/data/classes/*.toml`).


### DATA review 1 corrections and verification

- All 389 definitions parse with `tomllib`, including deferred definitions. Serialization uses recursive arrays of tables for conditions and stat children and checks structural round-trip equality. All source sets and every referenced rank table are retained; unresolved, short, or malformed tables are errors.
- Stat conditions are source apply conditions (`DocumentBase.java:246–247`), including bare `<using>`, `<player>` and boolean nodes. Unsupported leaves defer all affected skills; 94/148/171/225/290/291/293/294/1297 are deferred and 217 retains its SWORD/BLUNT condition. Full XML conditions and their unsupported attribute strings are retained at each site.
- Parameter names/values, operations and scope filters are admitted explicitly. Supported stat operations are mul/add/sub; div/set/share defer. Scope filters ALL/default and NOT_FRIEND are admitted, with other filters including FRIEND deferred until implemented (`AffectObjectStaticImpl.java:32–38,58–63,84–99`).
- Pinned defaults: missing magicLvl=0, ranges=-1, abnormalLvl=0 (`Skill.java:212–214,273`); abnormal clamp 10/90 (`character.properties:277–278`, `Skill.java:276–277`); cast-speed cap 1999 (`character.properties:268`, `CharStat.java:252`). Locked reuse is imported (`Skill.java:204`, `L2Character.java:1514–1524`). C5 resolves the earlier architecture/v3 default contradictions.
- The independent oracle contains 24 formula sets / 1,203 vectors. Corrected continuous 500/550 ms staging launches immediately and impacts 400 ms later (`L2Character.java:1625–1643,4577–4581`). Zero/sub-ms and <=100-ms cases use the original task skillTime truncation, with minimum one Nightfall completion tick.
- TickHp uses binary32 division before widening (`AbstractEffect.java:148–149`), then binary64 power/products (`TickHp.java:62–71`). Source fractional results are separate from D-R12 whole-damage/Q-restoration adaptations. Fixtures include CP absorption after HP-1 clamping (`TickHp.java:75–76`, `PcStatus.java:185–194`), dynamic PER reads, multi-pulse Q carry and full-clamp clearing; regen minimum standing traces yield cumulative 1/2/3 and Q remainder 100000/200000/300000.
- Rational karma truncation is not universally Java-equivalent. Level 2, XP 6208133913, multiplier 2.069377971 yields rational 100000000 vs binary64 99,999,999. The oracle models literal/long conversion and each division's IEEE rounding separately (`Formulas.java:1814–1820`), with the pinned RateKarmaLost=1 input (`config/rates.properties:56`).
- Additional source observations: skill 60 specifies `nextActionAttack="fake_death"` (`stats/skills/00000-00099.xml:1929`); the game reads a boolean (`Skill.java:279`, `StatsSet.java:112–121`), so the normalized value is false and the raw string is retained. `AbstractEffect.java:70–72` retains attachCond but comments out applyCond assignment, although `DocumentBase.java:270` parses it. DATA still preserves effect conditions and defers unsupported leaves per the binding capability rule; this observation does not silently change the execution contract.

## Additive Phase 3 class magical and regeneration templates

`classes-phase3/<profession-key>.toml` is a separate schema-1 V5 input for all 89
classic classes. It does not replace `classes/`, `professions/`, `growth/`, or any
historical stat-rule input. The existing `tables/regen.toml` `base_regen_source`
pointer predates these profiles: **use `classes-phase3/` for V5 regen bases**.
Keep this registry and the NPC `[phase3]` section in the Phase3Rules hash, outside
the legacy class/stat hash. The engine owner must consume the new fields through
its Phase 3 loader; the existing strict NPC loader does not yet admit them
(`apps/api/src/infrastructure/npc_data.rs:59-84`).

Generation reads only the pinned local datapack
`3ca488dd2bd0bfaca43e378886a3c2e37968153a` and game
`abfde0490ac52a2106e884b7242c0276c44cdf76`. Both revision and clean source inputs
(including ignored/untracked inputs) are checked before writing. Reproduce:

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

`--check` compares exact contents, detects extra files in `classes-phase3/`, and
verifies the generated NPC suffix without rewriting it. Generation only appends
or replaces its delimited `[phase3]` suffix in `npcs/keltir.toml`; all bytes before
that suffix are preserved. Existing generated data and historical oracle vectors
are preserved. The separate new fixture is `fixtures/skill-oracle-class-phase3.json`.

Class profile fields:

| Table/key | Meaning and pinned source |
|---|---|
| `id`, `key`, `parent`, `base_class_id`, `tier`, `min_level` | Same catalog identity and corrected graph as `gen_classes.py`. Own XML is selected by `classId`; no ancestor curve is substituted. The existing classList errata (34→32 and 104→28) remain those documented in the Phase 2 provenance above. |
| `base_stats` | Exact six static XML attributes for that class; no effects or racial hooks applied. `HumanFighter.xml:5-10` is an example. |
| `magical.base_m_atk` | Class template's XML `staticData/baseMAtk`, not an equipped weapon override (`HumanFighter.xml:24`; game `L2CharTemplate.java:108,332-333`, `CharStat.java:237-246`). |
| `magical.base_m_def` | Loaded template total: rear + lear + **rfinger twice** + neck, preserving `PlayerTemplateData.java:106-108`. Example XML slots are `HumanFighter.xml:25-31`. |
| `magical.m_def_slots_xml`, `magical.m_def_slots` | Literal slots plus the actual `getBaseDefBySlot` mapping. `L2PcTemplate.java:71-75` also uses RFINGER for LFINGER. `FuncMDefMod.java:44-66` subtracts occupied-slot bases before MEN/level multipliers. |
| `magical.base_m_atk_speed` | Cast-speed base; missing in all 89 XML staticData sections, so sourced from `L2CharTemplate.java:112` default **333**. This is distinct from the physical `basePAtkSpd`. Getter uses this base at `CharStat.java:258` and caps/truncates at `:259-262`; pinned cap **1999** is in `character.properties:268`. |
| `magical.base_m_critical` | Missing XML `baseMCritRate` defaults to **0**, `L2CharTemplate.java:119`. The actual magic-crit stat getter instead starts at **1**, `CharStat.java:270-275`; do not substitute this template field for that getter input. |
| `combat_getter_bases.accuracy`, `.evasion`, `.magic_critical` | Stat-getter initial values **0**, **0**, **1** (`CharStat.java:108-109,171-174,270-275`). Accuracy/evasion are not staticData bases; their built-in Funcs provide derived additions. |
| `regen.kind`, `regen.levels` | `kind="per_level"`, exactly 85 rows, each with integer `level` and original decimal strings `hp_regen`, `mp_regen`, `cp_regen`. Each value has its class XML path:line comment. There are 7,565 rows / 22,695 scalars. |

**Player regen is per level, not a class scalar.** `PlayerTemplateData.java:111-123`
loads every `lvlUpgainData/level/*Regen` node into `L2PcTemplate`'s three double
arrays (`L2PcTemplate.java:41-43,59-61,96-103`). Its getters index those arrays by
the current level (`:135-152`), and Formulas selects these getters
(`Formulas.java:264,358,444`). Every class has its own complete rows, just like the
HP/MP/CP growth inputs in `gen_classes.py`; in this pinned revision all 89 regen
curves happen to agree. They still vary with level:

| Level | HP base | MP base | CP base | Example `HumanFighter.xml` lines |
|---|---|---|---|---|
| 1 | `2.0` | `0.9` | `2.0` | 63-65 |
| 20 | `3.4` | `1.2` | `2.5` | 215-217 |
| 40 | `5.4` | `1.8` | `4.5` | 375-377 |
| 85 | `9.9` | `3.0` | `8.5` | 735-737 |

These are **pre-formula** values. Player HP/CP multiply by level modifier and CON,
MP by level modifier and MEN; `Math.max(1,init)` is applied before regen-stat
functions, followed by config and posture multipliers
(`Formulas.java:334-349,419-434,442-453`). Level modifier is `(level+89)/100`
(`L2Character.java:5007-5008`). Standing is 11/10, running 7/10, walking 1;
sitting 3/2 is source behavior but outside Phase 3. Pinned character config
multipliers are 1.00 (`character.properties:47-49`). No stat, posture, clamp, or
bonus is pre-applied to the emitted class rows.

### Young Keltir 20545 additive NPC inputs

The new `npcs/keltir.toml` `[phase3]` section cites
`stats/npcs/20500-20599.xml:3648-3661`. `[phase3.xml]` preserves **M.Atk 5.78704,
M.Def 29.5916164000214, HP regen 2, MP regen 0.9, accuracy 4.75** as source
decimal strings (`:3659-3661`); source attributes are at `:3658`.
`NpcData.java:164-185` loads those values into StatsSet. The inherited template
getters differ from the raw XML:

- `[phase3.template]` has **M.Atk 5 / M.Def 29**, because `L2CharTemplate.java:108,110`
  calls `getInt` on parsed Numbers and `StatsSet.java:241-247` uses `intValue()`.
  Cast-speed base is **333** from the same `L2CharTemplate.java:112` default;
  `hitTime=370` (`npc XML:3669`) is not a cast-speed base.
- HP/MP regen are **scalar inherited getters**, not level arrays.
  `L2NpcTemplate.java:100-101` calls the superclass constructor;
  `L2CharTemplate.java:105-106,311-319` stores/returns binary32 values.
  The new template table preserves the exact getter rationals: HP `2`,
  MP `7549747/8388608` (binary32 conversion of decimal 0.9).
  Retain that rational until the formula clamp; do not silently truncate it to Q.
- Formulas uses scalar NPC regen without player CON/MEN/level/posture bonuses
  (`Formulas.java:264,358,343-349,428-434`). MP's initial value below 1 is clamped
  to 1. CP regeneration is player-only (`:442-453`), so the section explicitly
  records `cp_regen_kind="not_applicable"` and supplies no invented CP scalar.
- XML accuracy is an unused TODO in `NpcData.java:174`; evasion is likewise a
  TODO at `:185`. The real getter inputs are zero (`CharStat.java:108-109,171-174`).
  The section preserves the XML accuracy separately and does not apply it twice.
  Existing `sp_reward=2` (`npc XML:3657`) and tunable physical/HP/MP/XP values stay
  unchanged.

### Template-driven oracle and validation

`skill_oracle.py --class-phase3` emits **6 formula groups / 290 vectors** in the
new fixture. All six requested starters (Human Fighter/Mystic, Elven Mystic,
Dark Elf Fighter, Orc Mystic, Dwarven Fighter) occur at **1/20/40/85** for
FuncMAtkMod, FuncMDefMod, FuncMAtkSpeed and all three regen functions, with
standing/running/walking regen and NPC HP/MP cases. The six class names map to
the existing profession keys; Dark Elf Fighter is `dark_fighter`.

Expectations use independent Fraction arithmetic over emitted template values
and literal stat-bonus tables. M.Atk uses base×INT²×levelMod²
(`FuncMAtkMod.java:42-46`); M.Def uses the empty-jewelry base×MEN×levelMod
(`FuncMDefMod.java:44-66`); speed uses base×WIT then getter cap/truncation
(`FuncMAtkSpeed.java:42-43`, `CharStat.java:258-262`). Cases explicitly specify
unarmed/empty jewelry, no modifiers, no injected RNG, config multipliers and
no estate/zone bonus. The fixture records exact rationals, intermediates,
rounding sites, Q values, three-pulse remainder traces, input SHA-256s and a
canonical vector-set SHA-256. Q conversion floors only at the final formula
boundary, as in the existing oracle; whole-resource transfer retains the
remainder under D-R12. This is exact-decimal formula evidence, not a claim of
universal Java binary64 equivalence. NPC binary32 getter conversion is modeled
with the integer-only IEEE adapter and independently checked with `struct`.

`test_gen_class_phase3.py` independently audits every class, graph mapping,
static magic base/slot and every regen literal against pinned XML. It also tests
missing levels/regen, source revision/dirty-input rejection, asymmetric rings
(to catch accidental correction of the source's duplicated RFINGER), orphan
and stale-file detection, NPC prefix preservation, getter conversions, requested
oracle coverage, exact expectations and fractional carry.

## Phase 3 equipped starter weapon and remaining caps

`gen_class_phase3.py` also produces `tables-phase3/starter_weapon.toml` and
`tables-phase3/constants.toml`. These are separate V5 inputs: include their complete
contents in the Phase3Rules hash, never the legacy StatRules hash. The separate
folder deliberately avoids `oracle.py`'s historical `tables/*.toml` scan;
`tables/starter_weapon.toml`, all prior fixtures and their input hashes stay exact.
The generator admits the pinned weapon XML as a clean source input, validates TOML
before writing, and `--check` detects drift and orphan files in both Phase 3 folders.
Use the same `gen_class_phase3.py --datapack ... --game ... [--check]` command above.

The pinned datapack `src/main/resources/data/stats/items/02300-02399.xml:844-867`
identifies Squire's Sword **2369**. Its type **SWORD** (`:847`) supports the equipped
`using kind` condition; bodypart **rhand** (`:848`), random damage **10** (`:849`)
and attack range **40 L2 units** (`:850`) are preserved. `[[functions]]` carries
all four source SET Funcs in XML order: **pAtk 6** (`:862`), **mAtk 5** (`:863`),
**critRate 8** (`:864`), **pAtkSpd 379** (`:865`). Each entry contains operation,
resolved priority, XML ordinal, exact decimal string, Q value and file:line provenance.
There are no extra weapon stat Funcs in that `for` block (`:861-866`).

Weapon SET does **not add to the class M.Atk base**. The pinned game
`engines/DocumentBase.java:230-247` supplies missing order as -1;
`model/stats/functions/FuncTemplate.java:48-54` resolves it to the enum default;
`enums/StatFunction.java:30` declares SET priority **0**. Then
`model/stats/functions/FuncSet.java:36-40` returns the weapon value, replacing the
getter's template initial value (`model/actor/stat/CharStat.java:237-246`).
`model/stats/Calculator.java:110-127,183-188` folds in ascending priority, with
insertion order for equal priorities. `FuncMAtkMod.java:37-46` runs at priority
**1** and multiplies that replaced value by INT bonus squared and level modifier
squared. `model/actor/L2Character.java:5007-5008` defines the level modifier.
The new profile's `[ordering]` records the full pinned repository/revision/file/line
provenance for these semantics.

The separate `[constants]` entries preserve **MaxMCritRate 200**, **MaxRunSpeed
250**, **RunSpeedBoost 0**, from the pinned game
`src/main/resources/config/character.properties:260,252,31` respectively. Each
entry carries integer value, original literal, file, line and symbol; the top-level
source carries the pinned repository and revision. Existing `tables/formulas.toml`
and its legacy hash remain unchanged.

`starter_weapon_oracle.py` generates
`fixtures/skill-oracle-starter-weapon-phase3.json`: **24 FuncMAtkMod vectors**,
covering the same six reference classes at levels **1/20/40/85** as the unarmed
class-profile fixture. Each vector records template initial value, order-0 weapon
replacement, priority-1 result, unarmed result and an incorrect-addition comparator.
The six class XML `staticData/baseMAtk` values are **6**, with each profile/vector
citing its source file; the equipped base is **5** (`stats/items/02300-02399.xml:863`).
Expectations use independent Fraction arithmetic: `5 * INT_bonus^2 * levelMod^2`,
then floor once to Q; whole-number floor is display evidence only. This is exact
rational evidence and the existing Q adaptation, not a Java binary64 equivalence
claim. Input SHA-256s include the new weapon table, six profiles, stat-bonus table
and oracle script; a separate canonical vector-set SHA-256 covers the expectations.

```sh
python3 -B packages/data/scripts/starter_weapon_oracle.py
python3 -B packages/data/scripts/starter_weapon_oracle.py --check
```

`check_all.py` now runs this independent oracle check as well as the existing
checks. `test_starter_weapon_phase3.py` audits all weapon functions against XML,
SET defaults/semantics against Java, cap literals against exact config lines,
missing/unsupported/invalid weapon functions, full requested coverage,
replacement-versus-addition arithmetic and historical table/manifest preservation.

### Additive Phase 3 NPC penalties and hit conditions

`gen_class_phase3.py` also owns `tables-phase3/npc_level_penalty.toml` and
`tables-phase3/hit_condition_bonus.toml` (via `gen_npc_phase3.py`). Hash their
complete bytes separately into Phase3Rules; historical tables, class profiles,
fixtures and oracle script remain byte-exact. Run the existing generator command
with `--game`, `--datapack` and `--check`; `check_all.py` checks both tables and
`scripts/npc_phase3/skill_oracle.py --check`. This separate oracle path preserves
`fixtures/skill-oracle.json`'s existing hash of `scripts/skill_oracle.py`.

Pinned game `src/main/resources/config/npc.properties:54,61-63,70,74` contains
minimum NPC levels **78/78**, normal damage **0.7,0.6,0.6,0.55**, critical damage
**0.75,0.65,0.6,0.58**, skill damage **0.8,0.7,0.65,0.62**, and magic failure
multipliers **2.5,3.0,3.25,3.5**. All source entries, including unreachable index
zero entries, are preserved as exact decimal strings and Q integers with line
provenance. The source has an indexing quirk: damage gates on target minus acting
player level >= **2**, then indexes **gap-1**; chance gates on gap >= **3**, then
indexes **gap-2**. Both saturate to the final entry. Thus the effective rows are:

| Target minus acting-player level | Auto normal | Auto critical | Magic damage | Physical skill normal | Physical skill critical | Magic failure factor |
|---|---|---|---|---|---|---|
| 2 | 0.6 | 0.65 | 0.7 | 0.42 | 0.455 | 1 |
| 3 | 0.6 | 0.6 | 0.65 | 0.39 | 0.39 | 3.0 |
| 4 | 0.55 | 0.58 | 0.62 | 0.341 | 0.3596 | 3.25 |
| 5 and above | 0.55 | 0.58 | 0.62 | 0.341 | 0.3596 | 3.5 |

These rows follow pinned game `model/stats/Formulas.java:713-731` (auto),
`:776-800` (physical skill, skill factor times normal/critical factor),
`:887-898` (magic damage) and `:1286-1314` (magic success). Eligibility also
requires an attackable non-raid, non-raid-minion target, target level at least the
configured damage minimum and a non-null acting player. Physical skills use
`instanceof L2Attackable`; other paths use `isAttackable`. Chance **also reads
MinNPCLevelForDmgPenalty**, leaving MinNPCLevelForMagicPenalty unused here
(`:1291`). Failed gates mean unity. Chance scales the **failure subtrahend**:
`100 - Math.round((float)(1.3^lvlDifference * targetModifier * resModifier))`,
with success strictly `Rnd.get(100) < rate` (`:1288-1301,1313`). Skill magic level
controls the exponential difference, but the acting-player level controls the
NPC penalty eligibility/index. No RNG is consumed by the table lookup itself.

Pinned datapack `src/main/resources/data/stats/hitConditionBonus.xml:3-9`
contains back **10**, dark **-10**, front **0**, high **3**, low **-3**, rain **-3**,
side **5** (percent additions). Game
`data/xml/impl/HitConditionBonusData.java:75-81,92-118` loads all seven rows;
starts at **100**, adds high iff source Z difference **>50**, low iff **<-50**,
dark at night, then exactly one of back/front/side in that precedence. Returns
`max(mod/100,0)`. Rain is loaded but unused (`:104-105`). Facing/height/night are
source inputs, not permission to invent Phase 3 position/time mechanics; the
engine must decide supported input capabilities explicitly.

The separate `fixtures/skill-oracle-npc-phase3.json` contains **42 vectors each**
for the three damage penalty stages, **21** magic chance penalty vectors, and
**60** hit-condition vectors. Coverage includes skipped index zero, saturation,
all eligibility guards, threshold boundaries, normal/critical physical skills,
strict chance-roll boundary, height boundaries **-51/-50/0/50/51**, day/night,
all facing branches, and rain as a no-op. Counts and coverage are generated by
`scripts/npc_phase3/skill_oracle.py`; expectations use independent Fraction
arithmetic, including integer-only binary32 rounding for the isolated chance
factor. Penalty-stage damage vectors expose exact incoming **1001** times the
multiplier and whole truncation evidence; they do not claim equivalence for
unmodeled earlier Java binary64 stages. Input byte hashes and a separate
canonical vector-set hash are recorded; no previous fixture is rewritten.
`test_npc_phase3.py` independently audits config/XML provenance, source branches,
effective mappings, every vector, hashes, and all pre-task generated bytes.

## Phase 5 economy and crafting data

Phase 5 introduces economy, crafting, merchant, multisell, drop, corpse, herb, party,
ground, rate, storage, mail, flood, auction and manor tables under `packages/data/tables-phase5/`,
generated by `packages/data/scripts/gen_economy.py` from pinned L2J sources.

### Pinned sources

| Repository / Asset | Revision / Checksum | Role |
|---|---|---|
| <https://bitbucket.org/l2jserver/l2j-server-game> (`develop`) | `abfde0490ac52a2106e884b7242c0276c44cdf76` | Rates, formulas, ground constants, party constants, flood protectors, storage/mail constants |
| <https://bitbucket.org/l2jserver/l2j-server-datapack> (`develop`) | `3ca488dd2bd0bfaca43e378886a3c2e37968153a` | NPC drop/corpse lists, herbs, merchant buylists, multisell lists, recipes, item auctions, seeds.xml |
| `packages/data/jvm-pow-tables.txt` | sha256 `4386eed8ca269e85885dabaf9bae02c5327cf425013221dfbf7df962136201f6` | OpenJDK 21 binary64 `pow` oracle table (79 `pow56` + 241 `pow13` entries, 320 total) |
| this repository, `docs/plans/phase-5-zone-design.md` @ `0d525c7` | git object | Frozen design §14.2 adaptation ledger (X1–X14, P1–P21, P7b, T1) |
| this repository, `packages/data` @ `95754e0f837295ff55130ba7aee322692c476896` (`codex/phase4-data`; identical at `codex/phase4-integration` 96f1e82) | git objects | Phase 4 `tables-phase4/capabilities.toml` and `scripts/gen_skills.py` (story 1.4 storability and skill executability) |

### Generation and validation

```bash
python3 packages/data/scripts/gen_economy.py --datapack ~/.cache/nightfall/l2j/l2j-dp --game ~/.cache/nightfall/l2j/l2j-game
python3 packages/data/scripts/gen_economy.py --datapack ~/.cache/nightfall/l2j/l2j-dp --game ~/.cache/nightfall/l2j/l2j-game --check
python3 packages/data/scripts/check_all.py --datapack ~/.cache/nightfall/l2j/l2j-dp --game ~/.cache/nightfall/l2j/l2j-game
```

Every constant is parsed out of a pinned source line (properties key, Java literal line, XML or SQL), never
typed in (E47); a line that no longer matches exactly once fails generation. Each table starts with a
`sources` array naming every file it read with its repository and revision (E50). The only non-pinned
inputs are named Nightfall settings: the global multiplier `G = "1"` (P2), `pk_drop_protection = "None"`
(E24), the auction fixture anchor and round count (B4/E48), and the ratified Gludio manor fixture (E48).
Decimal literals stay exact strings; binary64/binary32 values are computed with exact rational
round-to-nearest-even (`fl`, `fl32`) and written as bit patterns.

### Generated tables (`packages/data/tables-phase5/`)

- `sources.toml`: Provenance ledger recording repository URLs, pinned revisions, the design and Phase 4 data revisions, and the pow table SHA-256 and entry counts (parsed from the file).
- `drops.toml`: Drop and corpse lists (`death` / `corpse`, one list per scope with `kind = "group"` / `"item"` nodes mixed in document order, as `NpcData.parseDropList` builds them) across 59 NPC files with `<death>` lists (14,589 `<group>` blocks, 3,032 single-item; 744 ungrouped death items) and 35 corpse files covering 2,260 NPCs (7,335 ungrouped corpse items). Tracks 234 `corpseTime` overrides and 36 SP-only templates across all 10,461 NPCs.
- `rates.toml`: Rate configuration chain with kinds parsed from `RatesConfiguration.java` (`double` / `float` / `int`), global rate multiplier `G = "1"`, precomputed effective rates `rate' = fl32(fl(rate * G))`, the precise-drop strategy flags (`PreciseDropCalculation`, `PreciseDropMultipleGroupRolls`, `PreciseDropMultipleRollsAggregateDrops`), and deep-blue level-gap penalty windows.
- `ground.toml`: Ground item lifecycle and protection constants (15 s loot protection, 600 s normal decay, 60 s herb decay, every `auto_destroy_time` template, 5,000 ms scanner), PK drop rules (min PK 6, KarmaRateDrop 40, item 50, equip 40, weapon 10, PlayerRateDrop 0), 30 NonDroppable items (29 unique ids), and 11 PetItems.
- `party.toml`: Party constants (size 9, ranges 1600/1400, vote timeout 15 s, `ExponentXp = 0`, `ExponentSp = 0`, `BONUS_EXP_SP` source literals and bits, the even-distribute flags, and the highfive cutoff method, percent, level and gaps).
- `herbs.toml`: 68 immediate-effect items (59 handled with 48 distinct skill ids and levels, 9 unhandled destroying with no effect; 3 of the 9 are Armor templates).
- `adaptations.toml`: The design §14.2 adaptation ledger (37 labels, withdrawn flags), the 22-row killer-modifier `L` table recomputed from `IKillerChanceModifierStrategy`, `Util.map` and `npc.properties` (19 distinct bit patterns), and the pow table reference.
- `merchants.toml`: 274 merchant scripts covering 20,236 entries with source array, buylist ids and NPC ids (583 buylists, id = `npcId + "" + index`; ClassMaster lists are built for two NPCs and the CastleMerchant list for its 14 ManorManager subclasses), reference price, tax literal, delay literal, and count (2,145 limited entries; 5,803 distinct `(template, tax)` pairs). Unit prices for every castle percent {0} ∪ the Seven Signs ceilings {5, 15, 25} are `(long) fl(P * fl(1 + fl(fl(tax/100) + fl(castle/100))))` in exact rational emulation; `naive_truncation_differences_castle_0 = 362`. Castle tax percents come from `castle.sql`. Zero-price templates (8187) are derived from the item data.
- `multisell.toml`: 196 files / 195 Java list ids (`Integer.parseInt` of the file name). `001.xml` and `1.xml` both map to id 1 and differ (6 vs 29 entries), so list 1 is `1.xml` and the loss of `001.xml` is recorded in `deviations` (E49; pinned keeps whichever file `File.listFiles()` returns last). 14 `applyTaxes` lists, 31 `maintainEnchantment` lists, 21 `maintainIngredient` ingredients.
- `recipes.toml`: 1,000 recipes (830 Dwarven, 170 Common) with 582 rare variants, names, statUse and altStatChange lists; the full Create Item (172, 10 ranks) and Create Common Item (1320, 9 ranks) rank tables (rank, max craft level, magic level; skill ids from `CommonSkill.java`); recipe limits 50/50, `CraftMasterwork`, `AlternativeCrafting`.
- `storage_mail.toml`: Warehouse capacity 100/120 (fee 30), freight 200 (fee 1000), mail constants (`RequestSendPost`, `Message`), and flood protector intervals in 100 ms ticks.
- `auction.toml`: Item auction instances 32320, 32321, 32322 (82 items each, initial bid 100,000, length 300 minutes), `ItemAuctionExpiredAfter` / `ItemAuctionTimeExtendsOnBid`, the START/FINISH time spaces, and the B4/E48 fixture schedule: what pinned `ItemAuctionInstance` creates from a first start at the fixture anchor (2026-10-12T00:00Z, UTC DB time), two rounds per instance, auction ids in pinned creation order, items drawn at `AuctionCreate`.
- `manor.toml`: 258 crops from `seeds.xml` (129 alternative), the Sow/Harvest skills read from the seed and Harvester templates, the `setSeeded` strong-type multipliers, and the E48-ratified Gludio-only fixture seed production and crop procurement lists (32 crops).
- `catalogue.toml` / `catalogue_counts.json` (story 1.4): every template referenced by a Phase 5 table with its roles, Phase 4 catalogue membership, D5 storability (the Phase 4 `gen_items.local_blockers` contexts, cross-checked against all 8,423 Phase 4 rows), and its Phase 5 executability (herb pickup effect under the Phase 4 skill rules, cross-checked against all 2,705 Phase 4 skill rows; Sow/Harvest/RegisterRecipe as Phase 5 engine paths; storage only otherwise). E51: storability is independent of executability, so common, shadow and time-limited templates are storable (shadow/time-limited tagged `lifetime_deferred`) and `for_npc` is not a blocker (flagged `pending_d97` until Phase 4 D97, data rev 9, is integrated; the manifest is regenerated then and the non-storable set must stay 0). E52: multisell entries with Fame (−300) or Clan Reputation (−200) are kept, their lists admitted, and executing them refuses ItemCapabilityDeferred. It names the non-executable herbs and the templates the Phase 4 EtcItem closure must add at integration. No row of any table is dropped for a capability.

### Economy oracle (`scripts/economy_oracle.py`, stories 1.5–1.7)

The oracle implements the 22 families of design §14.1 independently of the Rust engine and of
`gen_economy.py`'s arithmetic. It has its own exact `fl`/`fl32` over Fractions, and it uses CPython
`float` only as the binary64 reference. It reads the E1 tables and the pinned clones directly (and
checks their revisions). Vectors go to `fixtures/economy-oracle-{drops,party,vendor}.json`, and
`--check` (wired into `check_all.py`) regenerates them in memory and asserts every Done-When number.
The pinned `(long)(P * 0.05)` manor fee equals `P div 20` for every P below 6,755,399,441,055,759.
That is the first P where they differ. Every reachable P (at most the 99,900,000,000 adena cap) is
far below it, so the design's "every P < 2^53" claim is too strong, but the fee is not affected.
