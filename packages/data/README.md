# packages/data

TOML and CSV game data, loaded and validated at server start by `apps/api/src/infrastructure/`
(`zone_data`, `npc_data`, `rules_data`, `class_data`). Any error aborts startup and **all** problems are listed together.
Numbers are integers or exact decimal strings (`"8.8"`, up to 6 places); float literals are
rejected. The zone's `config_hash` is a `DataHash` (`data_hash.rs`) over the zone file and every
NPC template, so any data change changes the hash recorded in snapshots. The stat rules carry
their own canonical `config_hash` (`rules_data`); folding it into snapshots is Story E2.2.
The class catalog carries its own canonical `config_hash` (`class_data::load_classes`), computed
as a SHA-256 digest over the canonical JSON encoding of `ClassRegistry` and source provenance,
capturing all metadata and growth curves.

| Path | Holds |
|------|-------|
| `zones/<id>.toml` | bounds, fixed non-combat `[[npcs]]` (`attackable = false`), `[safe_point]`, `[[spawn_slots]]` |
| `npcs/<id>.toml` | one attackable monster template; file stem must equal `id` |
| `tables/*.toml`, `classes/*.toml` | **generated** Phase 1 HF stat rules (bonuses, formulas, XP, death loss, starter weapon, starting classes); see [SOURCES.md](SOURCES.md), never hand-edit |
| `races/<id>.toml` | **generated** 5 classic playable race definitions (`human`, `elf`, `dark_elf`, `orc`, `dwarf`) with racial fighter baseline movement, decimal collision strings, environmental traits, and passive skill descriptors |
| `professions/<key>.toml` | **generated** 89 classic class definitions across tiers 0..=3 (tier distribution 9-18-31-31; IDs 0..=57 and 88..=118; reserved Kamael IDs 123..=136 absent), fixed 170 base stats, collision in integer Q world units, and transfer requirements |
| `skill_catalog.toml` | **generated** 378 referenced skill IDs with source maximum levels and explicit deferred runtime status |
| `growth/<key>.csv` | **generated** 89 per-class resource growth curves for levels 1..=85 (7,565 rows total, 22,695 exact HP/MP/CP decimal scalars) |

## Units

| Quantity | Unit |
|----------|------|
| Positions in zone files (`pos`, `home`, `bounds`, `safe_point`) | whole tiles |
| `attack_range`, `collision_radius`, `aggro_range`, `clan_help_range`, `leash_radius` | milli-tiles (1000 = 1 tile) |
| `move_speed`, NPC `speed` | milli-tiles per tick (tick = 100 ms) |
| `p_atk`, `p_def` | final values, decimal string or integer (stored as Q = 1e-6) |
| `attack_speed` | P.Atk.Spd (interval derived by the combat rules) |
| `max_hp`, `max_mp`, `xp_reward` | whole numbers |
| `corpse_decay_ticks` | ticks |
| `respawn_delay_secs`, `respawn_random_secs` | seconds; actual respawn = delay + seeded `0..=random` |
| Race movement (`walk`, `run`, `swim`) | reference world units per second (racial fighter baseline) |
| Class movement (`walk`, `run`, `swim`) | reference world units per second; actual class values (Human Mystic and Orc Mystic differ from fighter baseline) |
| Race collision (`radius_male`, `radius_female`, `height_male`, `height_female`) | exact decimal strings in reference world units (fighter baseline) |
| Class collision (`radius_male`, `radius_female`, `height_male`, `height_female`) | integer world units at Q = 1,000,000 (`Scaled`); actual class values (Human Mystic and Orc Mystic differ from fighter baseline) |

Spawn slot: `id` (unique), `template`, `home`, `count` (1..=256), optional respawn overrides
(omitted = the template's). Validation: template resolves, home inside bounds, delay ≥ 1,
random ≥ 0, leash ≥ aggro range, clan help range needs a clan.

## Class and Race Catalog (Phase 2)

- **Provenance & Verification**: Generated from pinned L2J High Five datapack `3ca488dd2bd0bfaca43e378886a3c2e37968153a` (CP formulas from game `abfde0490ac52a2106e884b7242c0276c44cdf76`). Verification via `python3 packages/data/scripts/gen_classes.py --datapack <path> --check` audits generated content, directory inventory, and embedded Rust definitions (`apps/api/src/infrastructure/class_data_embedded.rs`). Full details in [SOURCES.md](SOURCES.md).
- **Scope & Validation**: 89 classic classes (tiers 0..=3), 5 races, and 89 per-class growth CSVs (7,565 rows, 22,695 exact decimals; 240 non-quadratic curves vs 27 quadratic base curves). Deserialization re-executes `ClassRegistry::validate()`, enforcing 170 stat sum, positive collision dimensions, strict nested structs, non-decreasing growth, and lineage consistency.
- **Learning & Proficiency Boundaries**:
  - 39 populated direct trees (9 base, 18 first, 12 second MVP `[2, 5, 8, 9, 12, 16, 17, 27, 33, 46, 52, 55]`) provide 6,927 exact learning rows (SP costs, default bools, NPC learning, 48 item requirements with canonical `l2.item.<id>` future hooks). 50 other direct trees are deferred (ancestor trees inherited).
  - All 89 classes carry 3,521 equipment mastery and Expertise 239 proficiency rows.
  - 378 targeted skill definitions in `skill_catalog.toml` with runtime status strictly `Deferred` (runtime engine absent).
  - Automatic `autoGet` metadata grants: persistent metadata grant at create/transfer/level (max level per key at zero SP charge). Manual learning, SP expenditures, and skill effects are Phase 3 excluded.
  - Sourced `required_level` gates govern learning availability, not active skill effects.
- **Attributes, Positions & Names**: Racial fighter baselines in `races/` (decimal collision strings) vs class actuals in `professions/` (integer Q collision). Fixture start positions `[[0, 0]]` preserve Phase 1; reference starting zones name unimplemented retail villages. Retail `l2_ref` names are exact; the 89 original Nightfall `display_name` labels are mutable draft design metadata.
- **Progression & Subclass Eligibility**: Authoritative level cap is 85. Transfer gate is capped at transfer 2 (tier 3 unreachable in gameplay). Subclass eligibility (`domain/subclass.rs`) is a pure eligibility model: level 40 start, level 80 cap, max 3 slots, 12 certifications, enforcing tier ≥ 2 main, level ≥ 75, quest/noble, slot limits, permitted tier 2 classes (excluding 51 and 57), and Elf/Dark Elf mutual exclusion.
- **Integration Status**: Whole catalog reconstructed compact JSON equivalent is ~2.05 MB raw (~156 KB zlib default; size estimate, not full zone checkpoint). Independent source audits matched every growth value, learning row, proficiency row, and targeted skill definition. Admission findings were corrected and covered by validation tests; see [SOURCES.md](SOURCES.md).
