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

- **Provenance & Reproduction**: Catalog files (`races/`, `professions/`, `growth/`) are generated from pinned L2J High Five datapack commit `3ca488dd2bd0bfaca43e378886a3c2e37968153a` using `python3 packages/data/scripts/gen_classes.py --datapack <path> [--check]`. The `--check` flag validates that committed data files and compiled Rust inventory (`apps/api/src/infrastructure/class_data_embedded.rs`) match regenerated content; growth decimals retain source literals.
- **Deserialization and Validation**: `ClassRegistry` serde deserialization (`try_from = "RegistryParts"`) revalidates the entire catalog via `ClassRegistry::validate()`, verifying 89 classic classes, 170 stat sum, lineage consistency, non-decreasing growth, and subclass constraints. Replay snapshots embed `ClassRegistry` so replay remains self-contained without filesystem access.
- **Movement and Collision Baseline vs Class Actuals**: `RaceDef` (and `RaceInfo`) movement and collision represent the racial fighter baseline. `ClassDef` movement and collision provide actual values per profession; Human Mystic (ID 10) and Orc Mystic (ID 49) differ from their racial fighter baseline. Race collision is stored as decimal strings; class collision is stored in integer Q world units.
- **Start Points and Reference Zones**: `start_points` in `races/*.toml` are fixture test coordinates `[[0, 0]]` within `test_zone` (safe point `[126, 126]`). `reference_starting_zone` entries (`talking_island_village`, `elven_village`, `dark_elf_village`, `orc_village`, `dwarven_village`) are purely informational retail references and NOT implemented villages.
- **Naming Conventions**: `l2_ref` is retail exact. Nightfall `display_name` entries are curated original draft mutable metadata; no claim of source provenance is made for Nightfall names or fixture positions.
- **Deferred Hooks**: `SkillLearnDef` carries 6,927 sourced learning entries across all 9 base classes, 18 first classes, and the 12 second-class MVP professions. The other 50 direct trees are explicitly deferred; ancestors remain inherited. `ProficiencyDef` carries 3,521 equipment mastery and Expertise unlock references across all 89 classes. Skill effects and manual learning are deferred to Phase 3; item/mastery effects are deferred to Phase 4. `passive_skill_keys` are descriptor strings in `races/*.toml` gated behind the future Phase 3 skill registry; runtime effects are defined by the server progression and combat rules.
- **Transfer Gates and Subclass Model**:
  - Authoritative source and playable level cap remains 85, preserving Phase 1.
  - Class transfer token requirements exist for tiers 1..=3, but the application transfer gate is capped at transfer 2, leaving third-tier profession metadata currently unreachable in gameplay.
  - Subclass eligibility (`apps/api/src/domain/subclass.rs`) is a pure eligibility model: starts at level 40 (`SUBCLASS_START_LEVEL = 40`), level cap is main cap minus 5 (`subclass_level_cap`), up to 3 additional slots (`MAX_SUBCLASSES = 3`), 4 certifications per subclass slot (`CERTIFICATIONS_PER_SUBCLASS = 4`) up to 12 total (`MAX_CERTIFICATIONS = 12`). Pure eligibility checks enforce tier ≥ 2 main, level ≥ 75, quest/noble, slot limits, permitted tier 2 classes (excluding Overlord 51 and Warsmith 57), Elf/Dark Elf mutual exclusion, and no duplicate/equivalent classes. Village-master dialogue and interaction flows are deferred.
