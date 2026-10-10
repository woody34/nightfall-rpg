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
