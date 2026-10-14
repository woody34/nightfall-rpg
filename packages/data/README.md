# packages/data

This package has the TOML and CSV game data. At server start, `apps/api/src/infrastructure/`
(`zone_data`, `npc_data`, `rules_data`, `class_data`) loads and validates the data. Any error stops the startup, and the server lists **all** problems together.
Numbers are integers or exact decimal strings (`"8.8"`, up to 6 places). The loader rejects float literals.
The `config_hash` of the zone is a `DataHash` (`data_hash.rs`) of the zone file and of each
NPC template. Thus, any data change changes the hash that the snapshots record. The stat rules have
their own canonical `config_hash` (`rules_data`). Story E2.2 adds this hash to the snapshots.
The class catalog has its own canonical `config_hash` (`class_data::load_classes`). This hash is
a SHA-256 digest of the canonical JSON encoding of `ClassRegistry` and of the source provenance.
It includes all the metadata and the growth curves.

| Path | Holds |
|------|-------|
| `zones/<id>.toml` | bounds, fixed non-combat `[[npcs]]` (`attackable = false`), `[safe_point]`, `[[spawn_slots]]` |
| `zones/test_zone_phase3.toml` | additive Phase 3 peace geometry, written by hand. It is separate from the legacy zone config hash. |
| `npcs/<id>.toml` | one attackable monster template. The file stem must be equal to `id`. |
| `tables/*.toml`, `classes/*.toml` | **generated** Phase 1 HF stat rules (bonuses, formulas, XP, death loss, starter weapon, starting classes). Refer to [SOURCES.md](SOURCES.md). Do not change these files by hand. |
| `races/<id>.toml` | **generated** definitions of the 5 classic playable races (`human`, `elf`, `dark_elf`, `orc`, `dwarf`). They have the racial fighter baseline movement, decimal collision strings, environmental traits and passive skill descriptors. |
| `professions/<key>.toml` | **generated** definitions of 89 classic classes in tiers 0..=3 (tier distribution 9-18-31-31; IDs 0..=57 and 88..=118; the reserved Kamael IDs 123..=136 are not present). They have fixed 170 base stats, collision in integer Q world units, and transfer requirements. |
| `skill_catalog.toml` | **generated** 378 referenced skill IDs, with the source maximum levels and the calculated runtime status (180 executable / 198 deferred) |
| `growth/<key>.csv` | **generated** 89 resource growth curves, one for each class, for levels 1..=85 (7,565 rows total, 22,695 exact HP/MP/CP decimal scalars) |

## Units

| Quantity | Unit |
|----------|------|
| Positions in zone files (`pos`, `home`, `bounds`, `safe_point`) | whole tiles |
| `attack_range`, `collision_radius`, `aggro_range`, `clan_help_range`, `leash_radius` | milli-tiles (1000 = 1 tile) |
| `move_speed`, NPC `speed` | milli-tiles for each tick (tick = 100 ms) |
| `p_atk`, `p_def` | final values, decimal string or integer (stored as Q = 1e-6) |
| `attack_speed` | P.Atk.Spd (the combat rules calculate the interval) |
| `max_hp`, `max_mp`, `xp_reward` | whole numbers |
| `corpse_decay_ticks` | ticks |
| `respawn_delay_secs`, `respawn_random_secs` | seconds. The actual respawn = delay + seeded `0..=random` |
| Race movement (`walk`, `run`, `swim`) | reference world units for each second (racial fighter baseline) |
| Class movement (`walk`, `run`, `swim`) | reference world units for each second. These are the actual class values (Human Mystic and Orc Mystic are different from the fighter baseline). |
| Race collision (`radius_male`, `radius_female`, `height_male`, `height_female`) | exact decimal strings in reference world units (fighter baseline) |
| Class collision (`radius_male`, `radius_female`, `height_male`, `height_female`) | integer world units at Q = 1,000,000 (`Scaled`). These are the actual class values (Human Mystic and Orc Mystic are different from the fighter baseline). |

A spawn slot has these fields: `id` (unique), `template`, `home`, `count` (1..=256), and optional respawn overrides
(if omitted, the values of the template apply). The validation makes sure of these conditions:

- The template resolves, and the home is in the bounds.
- Delay ≥ 1,
random ≥ 0, and leash ≥ aggro range.
- A clan help range has a clan.

## Class and Race Catalog (Phase 2)

- **Provenance & Verification**: The generator makes the catalog from the pinned L2J High Five datapack `3ca488dd2bd0bfaca43e378886a3c2e37968153a` (the CP formulas come from game `abfde0490ac52a2106e884b7242c0276c44cdf76`). For verification, `python3 packages/data/scripts/gen_classes.py --datapack <path> --check` examines the generated content, the directory inventory and the embedded Rust definitions (`apps/api/src/infrastructure/class_data_embedded.rs`). For all the details, refer to [SOURCES.md](SOURCES.md).
- **Scope & Validation**: 89 classic classes (tiers 0..=3), 5 races, and 89 growth CSVs, one for each class. The CSVs have 7,565 rows and 22,695 exact decimals (240 non-quadratic curves and 27 quadratic base curves). Deserialization runs `ClassRegistry::validate()` again. This validation makes sure of a 170 stat sum, positive collision dimensions, strict nested structs, growth that does not decrease, and lineage consistency.
- **Learning & Proficiency Boundaries**:
  - 39 populated direct trees (9 base, 18 first, 12 second MVP `[2, 5, 8, 9, 12, 16, 17, 27, 33, 46, 52, 55]`) give 6,927 exact learning rows. These rows have SP costs, default bools, NPC learning, and 48 item requirements with canonical `l2.item.<id>` hooks for the future. 50 other direct trees are deferred (the ancestor trees are inherited).
  - All 89 classes have 3,521 equipment mastery rows and 239 Expertise proficiency rows.
  - `skill_catalog.toml` has 378 targeted skill definitions. The Phase 3 generation now calculates 180 executable / 198 deferred entries.
  - Automatic `autoGet` metadata grants: a persistent metadata grant at create, transfer and level (the maximum level for each key, at zero SP charge). Manual learning, SP expenditures and skill effects are not in Phase 3.
  - The sourced `required_level` gates control when learning is available. They do not control active skill effects.
- **Attributes, Positions & Names**: The racial fighter baselines are in `races/` (decimal collision strings). The actual class values are in `professions/` (integer Q collision). The fixture start positions `[[0, 0]]` keep the Phase 1 behavior. The reference starting zones name retail villages that are not implemented. The retail `l2_ref` names are exact. The 89 original Nightfall `display_name` labels are draft design metadata that can change.
- **Progression & Subclass Eligibility**: The authoritative level cap is 85. The transfer gate stops at transfer 2 (players cannot get tier 3 in gameplay). The subclass eligibility (`domain/subclass.rs`) is only an eligibility model: start at level 40, cap at level 80, a maximum of 3 slots, and 12 certifications. It makes sure of a main class at tier ≥ 2, level ≥ 75, quest/noble, and the slot limits. It also makes sure of the permitted tier 2 classes (not 51 and 57) and the Elf/Dark Elf mutual exclusion.
- **Integration Status**: The compact JSON equivalent of all the reconstructed catalog is ~2.05 MB raw (~156 KB with zlib default). This is a size estimate, not a full zone checkpoint. Independent source audits found that each growth value, learning row, proficiency row and targeted skill definition agreed. The admission findings were corrected, and validation tests cover them. Refer to [SOURCES.md](SOURCES.md).

## Generator ownership and complete verification

Each generated output has one owner. Do not change generated files by hand.

| Generator | Owned outputs |
|-----------|---------------|
| `scripts/gen_tables.py` | `tables/stat_bonus.toml`, `tables/formulas.toml`, `tables/experience.toml`, `tables/penalties.toml`, `tables/starter_weapon.toml`, `classes/*.toml` |
| `scripts/gen_classes.py` | `races/*.toml`, `professions/*.toml`, `growth/*.csv`. It also makes the existing Rust embedded inventory at `apps/api/src/infrastructure/class_data_embedded.rs`. |
| `scripts/gen_class_phase3.py` | `classes-phase3/*.toml` and only the `[phase3]` suffix of `npcs/keltir.toml` |
| `scripts/gen_skills.py` | `skills/*.toml` and **`skill_catalog.toml`** (the only owner, which includes the runtime status) |
| `scripts/skill_oracle.py` | `fixtures/skill-oracle.json`. With `--class-phase3`, also `fixtures/skill-oracle-class-phase3.json` |
| `scripts/oracle.py` | `fixtures/oracle.json`, `fixtures/rounding-audit.json` |

`gen_classes.py` collects the source learning metadata for the profession generation. But
it does not emit or validate the catalogue contents. Its embedded inventory keeps
its catalogue reference. `gen_skills.py` reads the existing catalogue identities.
It validates the catalogue and makes it again with the calculated capability status.
`tables/pvp.toml` and `tables/regen.toml` are source-backed tables that the team maintains.
They are not outputs of `gen_tables.py`.

To run the `--check` of each generator, which includes the two skill-oracle modes and the original
stat oracle, use this command from the repository root:

```sh
python3 -B packages/data/scripts/check_all.py --datapack /path/to/l2j-dp --game /path/to/l2j-game
```

The runner also parses and validates the hand-written `zones/test_zone_phase3.toml`.
It reports the separate byte SHA-256 of this file. Its `[peace_zone]` has a whole-tile `centre`,
`half_width_millitiles`, `shape = "square"` and `inclusive = true`. The `[provenance]` section
identifies the Nightfall configuration. Include this input only in the Phase3Rules hash.
For the policy provenance, refer to [SOURCES.md](SOURCES.md#phase-3-peace-geometry-nightfall-configuration).

The runner continues through all the checks. It exits with a nonzero code if the geometry is not valid, if an output is stale
or if a generator fails. The oracle comparisons include their input manifests. When inputs change, make new
manifests, and make sure that the historical vectors do not change.
