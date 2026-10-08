# packages/data

TOML game data, loaded and validated at server start by `apps/api/src/infrastructure/`
(`zone_data`, `npc_data`). Any error aborts startup and **all** problems are listed together.
Numbers are integers or exact decimal strings (`"8.8"`, up to 6 places); float literals are
rejected. The zone's `config_hash` is a `DataHash` (`data_hash.rs`) over the zone file and every
NPC template, so any data change changes the hash recorded in snapshots.

| Path | Holds |
|------|-------|
| `zones/<id>.toml` | bounds, fixed non-combat `[[npcs]]` (`attackable = false`), `[safe_point]`, `[[spawn_slots]]` |
| `npcs/<id>.toml` | one attackable monster template; file stem must equal `id` |

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

Spawn slot: `id` (unique), `template`, `home`, `count` (1..=256), optional respawn overrides
(omitted = the template's). Validation: template resolves, home inside bounds, delay ≥ 1,
random ≥ 0, leash ≥ aggro range, clan help range needs a clan.
