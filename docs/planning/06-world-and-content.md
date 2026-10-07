# Phase 6: World and Content

Zones, monsters and AI, raids, quests, instances, travel, day/night. Depends on Phases 0-5
(networking and persistence, stat formulas, classes, combat, items, drops).

---

## 1. Purpose and scope

**Delivers**

- A seamless 2D world made of chunked tilemaps, with server-side collision, line of sight,
  spatial partitioning and area-of-interest (AOI) streaming.
- Zone volumes (town/peace, battle, siege, water, swamp, no-landing, arena, boss, no-restart)
  and the flags each one toggles on characters inside it.
- NPC templates, spawn tables (fixed, random-in-territory, day/night, routes), respawn timers
  with jitter, and a server-authoritative monster AI (intention state machine, hate list, social
  aggro, leash/return-home, flee/heal behaviours, minions).
- Raid bosses and grand ("epic") bosses with persisted status and respawn windows, minion
  management, loot-right rules and raid-curse style level gating.
- A quest engine: quest states and variables, quest items, kill/talk/item/zone hooks, class
  transfer and repeatable/daily quests, with a data-driven definition format plus an embedded
  scripting escape hatch.
- Instanced dungeons: templates, timers, party-size limits, re-entry cooldowns.
- Travel: gatekeepers, escape scrolls, residence teleports, mounts and scheduled transports.
- A game clock with a day/night cycle driving spawns and racial/skill bonuses.

**Excludes**

- Drop tables and spoil (Phase 5), combat formulas and aggro *damage* math (Phase 3),
  siege-specific NPCs and castle ownership (Phase 7), client camera/UI (Phase 8),
  GM spawn tools and live events (Phase 9).

---

## 2. Reference: how Lineage 2 does it

### 2.1 World structure, regions and geodata

L2's world is a single continent with no loading screens. The server (L2J) partitions it twice:

**Map tiles (geodata).** The world is a grid of 32768 x 32768 unit tiles indexed `XX_YY`
(for example `20_18` contains the origin). L2J's geodata description: the world is divided into
32x32 regions, the populated world is 11 regions wide and 16 high, each region is 256x256
*blocks*, each block is 8x8 *cells*, and a cell is 16 world units square ([L2J geodata
homepage](https://l2j-geodata.sourceforge.net/index.php?p=4)). 256 * 8 * 16 = 32768 units,
matching the tile size. Blocks are stored in three forms: *flat* (one height for the whole
block), *complex* (64 cells, each a 16-bit value: height in the upper bits and 4 NSWE
passability bits in the low nibble) and *multilayer* (complex plus a layer count per cell for
bridges and floors). Geodata gives the server three things: `getHeight(x, y, z)` for Z
snapping, `canMoveToTarget(from, to)` by walking cells and checking NSWE bits, and
`canSeeTarget(a, b)` by stepping a 3D line and checking that terrain never rises above it.
Without geodata L2J servers let mobs attack through walls, which is the single most noticed
difference between "geodata on" and "geodata off" private servers.

**World regions (`L2World`).** Independently of geodata, L2J bucketizes every object into a
coarse region grid for AOI. The constants (quoted from `L2World.java` in L2J High Five; verify
against [bitbucket.org/l2jserver](https://bitbucket.org/l2jserver/l2j_server) since the file could
not be fetched during research):

| Constant | Value | Meaning |
|---|---|---|
| `SHIFT_BY` | 12 | region edge = 2^12 = 4096 units |
| `TILE_SIZE` | 32768 | geodata tile edge |
| `TILE_X_MIN..MAX`, `TILE_Y_MIN..MAX` | 11..26, 10..26 | populated tiles |
| `TILE_ZERO_COORD_X/Y` | 20, 18 | tile that holds (0,0) |
| `MAP_MIN_X / MAX_X` | -294912 / 229375 | derived: `(TILE_X_MIN - 20) * 32768`, `(TILE_X_MAX - 20 + 1) * 32768 - 1` |
| `MAP_MIN_Y / MAX_Y` | -262144 / 294911 | same derivation on Y |
| `OFFSET_X / OFFSET_Y` | 72 / 64 | `abs(MAP_MIN >> SHIFT_BY)` |
| `REGIONS_X / REGIONS_Y` | 127 / 135 | `(MAP_MAX >> SHIFT_BY) + OFFSET` |

`getRegion(x, y) = regions[(x >> 12) + 72][(y >> 12) + 64]`. Each region knows its 8
neighbours; "visible objects" for a player are gathered from the 3x3 block of regions and then
filtered by distance. Regions with no players for about a minute are *deactivated*: their NPC
AI tasks stop and random walking pauses, which is how one process runs ~60k NPCs.

### 2.2 Zone types

Zones are polygon or cuboid volumes loaded from XML (`<zone type="PeaceZone" shape="NPoly"
minZ=... maxZ=...>`) ([L2J zone XML sample](https://bitbucket.org/l2jserver/l2j-server-game/issues/180/custom-zones)).
Each zone sets one or more `ZoneId` bits on characters inside it. The core set and what each
toggles:

| Zone | Flags set | Effect |
|---|---|---|
| Town | PEACE, TOWN | No attacks, PvP flag cleared, shop prices taxed by the owning castle (`taxById`), respawn point on death |
| Peace (generic) | PEACE | No attacks; used for quest huts, Olympiad lobby |
| Battle / PvP | PVP | Free PvP, no PK karma, flag auto-on |
| Arena | PVP + revive-in-place | Coliseum, Monster Track |
| Siege | SIEGE, PVP during siege | Non-registered players teleported out at siege start; death penalty reduced; HQ flags allowed |
| Castle / Fort / Clan hall | CASTLE / FORT / CLAN_HALL | Residence functions, door control, `NO_SUMMON_FRIEND` |
| Water | WATER | Swim animation, breath meter, no fighting underwater in older chronicles |
| Swamp | SWAMP | Movement speed reduced (zone parameter, typically -50%) |
| Damage / Effect | (none) | Periodic HP loss or skill application (flame tower zones, lava) |
| No-landing | NO_LANDING | Mounted wyvern is force-dismounted after a countdown |
| Jail | JAIL, NO_SUMMON | GM punishment |
| Mother Tree | MOTHER_TREE | HP/MP regen bonus for Elves in the Elven village |
| Boss / No-restart | NO_RESTART, NO_BOOKMARK | Logout ejects to town; raid zones with entry items |
| Olympiad stadium | PVP + scripted | Match rules, spectators |
| Condition zones | NO_ITEM_DROP, NO_STORE, NO_SUMMON_FRIEND | Marketplaces, event areas |

### 2.3 Layout: towns, hunting zones, dungeons, catacombs

The world is organised around castle towns, each with a radius of hunting grounds whose level
rises with distance. Five starter villages (Talking Island, Elven, Dark Elven, Orc, Dwarven)
cover levels 1-20; Gludin/Gludio 20-35; Dion 30-45; Giran 40-55; Oren/Hunters Village 50-65;
Aden 60-75; Goddard/Rune/Schuttgart 70-80 ([C4 notes](https://lineage2wiki.org/c4/patch-notes/),
[Interlude XP zones](https://l2db.net/guides/en/best-xp-zones-interlude)). Open dungeons
(Cruma Tower 40+, Tower of Insolence 50-75, Dragon Valley, Antharas' Lair) are multi-floor
interiors with no instancing; floor selection is by walking or by consumable items
(Dimensional Stone, 10,000 adena, chooses ToI floor 1/5/10 in High Five).

The Seven Signs catacombs and necropolises (C3) are the canonical "dungeon per level band",
accessible only to players who registered with Dawn or Dusk that period, with the winning side
owning them during the seal period. The C3 notes list ([source](https://lineage2wiki.org/c3/patch-notes/)):

| Dungeon | Level band | Entrance town |
|---|---|---|
| Necropolis of Sacrifice | 20-30 | Gludio |
| Heretics Catacomb / Pilgrim's Necropolis | 30-40 | Dion |
| Catacomb of the Branded / Worshipers Necropolis | 40-50 | Giran / Innadril |
| Catacomb of the Apostate / Patriots Necropolis | 50-60 | Oren / Gludio |
| Catacomb of the Witch / Ascetics' and Martyrs' Necropolis | 60-70 | Aden / Oren, Giran |
| Dark Omens, Forbidden Path / Saints', Disciples' Necropolis | 70-80 | Oren, Aden / Innadril, Aden |

Seven Signs runs on a two-week cycle: one week of competition (seal stones collected, A/B/C
stones from 70+/40-69/20-39 monsters) and one week of seal validation
([ludo.guide](https://www.ludo.guide/guide/lineage-ii/side-quests-activities/seven-signs-questline)).

### 2.4 NPC templates and spawns

An L2J `L2NpcTemplate` is loaded from `npcs/*.xml` and carries: `id`, `displayId`, `name`,
`title`, `type` (`L2Monster`, `L2RaidBoss`, `L2GrandBoss`, `L2Guard`, `L2Npc`, `L2Merchant`,
`L2Teleporter`, `L2Warehouse`, `L2Chest`, `L2FeedableBeast`, `L2ControlTower`, `L2SiegeFlag`
and about twenty more), `level`, `race`, `sex`, `baseStats` (STR..MEN, HP/MP/CP, regen, pAtk,
mDef, atkSpd, speeds, attack range), `collision` radius/height, `exp`, `sp`, `rhand/lhand`
items, `skills`, `dropLists`, `corpseTime` (seconds the body stays), and an `ai` block:
`aggroRange` (typically 300-500 for aggressive mobs, 0 for passive), `clanHelpRange`
(social aggro radius), `clan` ids (NPCs sharing a clan string help each other), `dodge`,
`isChaos` (attacks other monsters), `isAggressive`, `canMove`, `soulShot/spiritShot` counts and
chances, `minSkillChance/maxSkillChance`, `targetable`, `talkable`, `undying`, `attackable`
([L2C4 NPC API](https://mintlify.wiki/fermanzolido/L2C4/api/npc)).

A spawn (`L2Spawn`) is `(npcId, x, y, z, heading, count, respawnDelay, respawnRandom,
periodOfDay)`; the datapack `spawnlist` table has exactly those columns plus a location id.
On death the spawn schedules `respawnDelay + rand(0, respawnRandom)` seconds; `periodOfDay`
0/1/2 means always / day only / night only (`DayNightSpawnManager`). Territory spawns
(`<territory>` polygons in `spawns/*.xml`) pick a random point inside a polygon each respawn.
Walking routes (`WalkingManager`, `Routes.xml`) list waypoints with `delay` and optional
`chat` text; an NPC on a route alternates MOVE_TO and short waits and resumes after combat.

### 2.5 The monster AI state machine

`L2AttackableAI` is an intention machine. Intentions: `IDLE`, `ACTIVE`, `REST`, `ATTACK`,
`CAST`, `MOVE_TO`, `FOLLOW`, `PICK_UP`, `INTERACT`. Transitions are driven by events
(`onEvtAttacked`, `onEvtAggression`, `onEvtArrived`, `onEvtArrivedBlocked`, `onEvtDead`,
`onEvtSeeSpell`, `onEvtForgetObject`) and by a *think* task that runs every second for every
NPC in an active region ([L2J AI commit with constants](https://git.vmatviienko.pp.ua/L2j/l2j-server-game/commit/6759959a98d72a3de119f54284197fe6d7195741)):

- `RANDOM_WALK_RATE = 30`: an `ACTIVE` mob that can move has a 1-in-30 chance per think to
  wander a short distance around its spawn.
- `MAX_ATTACK_TIMEOUT = 1200` ticks (120 s; was 300 ticks = 30 s): if the mob has not been
  able to hit its target for this long it forgets the hate list and returns home.
- `MAX_DRIFT_RANGE = 300`: minions stay within this radius of their leader.

**thinkActive.** If the hate list is empty, scan the known list for anyone passing
`autoAttackCondition`: inside `aggroRange`, not dead, not GM-invisible, not in a peace zone;
guards attack monsters and players with karma; `isAggressive` mobs attack any player
(dwarf *Silent Move* and the Spoil/level checks reduce range); NPCs with `isChaos` attack other
monsters. Each candidate gets `addDamageHate(target, 0, 1)` so the first hate entry exists.
If hate exists, intention becomes `ATTACK`. Otherwise: minions move back toward the leader
when beyond `MAX_DRIFT_RANGE`, raid bosses and `isReturningToSpawnPoint` NPCs walk home and
regenerate to full, and random walking happens.

**thinkAttack.** Target = `getMostHated()`. If the target is dead, out of range (> aggro range
and > 2000 units) or unreachable, drop it (`stopHating`) and re-evaluate. Social aggro: when
first engaging, every NPC within `clanHelpRange` with the same `clan` id that is not already
fighting gets the attacker added to its hate list ("faction call"). Skill selection scans
template skills by category: heal when own HP < 50% (or an ally in range), buff when buff
missing, debuff/attack skills with `minSkillChance..maxSkillChance` roll, then fall back to
auto-attack; ranged/caster mobs with `dodge` or low HP flee to range before casting. If the
target is beyond attack range, intention `MOVE_TO`/follow with geodata pathing. Timeout
bookkeeping resets on every successful hit.

**Hate list.** `AggroInfo { attacker, hate, damage }` per attacker. `addDamageHate(attacker,
damage, aggro)` adds `aggro` (default = damage, scaled by Phase 3 aggro formulas; taunts add
flat hate; healers of the target get hate equal to heal amount). `getMostHated()` takes the max
hate among reachable attackers; `reduceHate` handles aggression-reversal skills; raid scripts
sometimes randomise (Antharas tracks the top 3 attackers and adds `damage + rand(3000)`;
[L2C4 AI scripts](https://mintlify.wiki/fermanzolido/L2C4/scripting/ai-scripts)).

**Mob types.** *Normal* monsters; *minions* (spawned by a leader's `MinionList`, re-spawned
on a timer while the leader lives, deleted when it dies); *champions* (L2J option: a normal mob
with a random chance to spawn with x7-8 HP/XP/drops and a title); *raid bosses* (type
`L2RaidBoss`, status persisted, minions, raid curse); *grand/epic bosses* (`L2GrandBoss`,
scripted multi-phase AI, dedicated boss zone, spawn window persisted in `grandboss_data`).

### 2.6 Raid and epic bosses

Normal raid bosses (levels 20-85) are persisted in `raidboss_spawnlist(boss_id, loc_x, loc_y,
loc_z, heading, amount, respawn_delay, respawn_random, respawn_time, currentHP, currentMP)` with
status `ALIVE | DEAD | UNDEFINED`; on death `respawn_time = now + rand(min, max) * multiplier`
([L2C4 raid docs](https://mintlify.wiki/fermanzolido/L2C4/systems/raid-bosses)). Typical datapack
values are 12 h + rand(0, 24 h); Interlude community guides describe fixed timers of 30-360
minutes for low raids ([l2db.net](https://l2db.net/guides/en/raidboss-farming-guide-for-lineage-2-interlude-ct0-3)).
HP/MP are saved so a boss that was half-killed before a restart comes back at that HP.

Epic bosses spawn inside a *window*: a fixed minimum after death plus a randomised tail
([guildorder](https://guildorder.com/games/lineage_2/wiki/epic-boss-windows-and-scouting)).
Two reference sets (L2J `GrandBoss.properties` defaults quoted from memory, verify; L2C4 docs
fetched):

| Boss | Level | Zone / entry | L2J H5 default (min + random) | L2C4 docs window |
|---|---|---|---|---|
| Queen Ant | 40 | Ant Nest (Gludio) | 36 h + 17 h | 36-48 h |
| Core | 50 | Cruma Tower top | 60 h + 24 h | 40-60 h |
| Orfen | 50 | Sea of Spores | 48 h + 20 h | 48-72 h |
| Zaken | 60 | Devil's Isle, Zaken's ship | 60 h + 20 h | - |
| Baium | 75 | Tower of Insolence 13F, wake with Blooded Fabric | 168 h + 48 h | 120-192 h |
| Antharas | 79 | Antharas' Lair, Portal Stone | 264 h + 72 h | 192-216 h |
| Valakas | 85 | Forge of the Gods, Floating Stone | 264 h + 72 h | 264-312 h |
| Frintezza | 85 | Last Imperial Tomb (Interlude, command channel only) | 48 h + 8 h | - |

Boss-specific mechanics worth copying: Queen Ant's Nurse Ants heal her and Royal Guards
re-spawn; Baium sleeps (`ASLEEP`) until an item wakes him, then fights with five Archangels and
despawns if nobody is in the zone for 30 min; Zaken teleports between ship rooms and spawns
clones, and is weaker at night ([ludo.guide](https://www.ludo.guide/guide/lineage-ii/zaken));
Antharas/Valakas have `DORMANT -> WAITING -> FIGHTING -> DEAD` statuses, a 200-player zone cap
and a 30-minute reconnect window ([C4 notes](https://lineage2wiki.org/c4/patch-notes/)).
Minions use `MinionList` with a 5-minute respawn while the leader lives.

**Raid curse.** Attacking, healing or buffing a raid (or its minions) while more than 8 levels
above it applies *Raid Curse*: petrify for physical attackers, silence for casters
([L2J raid curse commit](https://git.vmatviienko.pp.ua/L2j/l2j-server-game/commit/0cd797a74646dbeaa6b67c0bfee1a825b560c968),
[l2db.net](https://l2db.net/guides/en/raidboss-farming-guide-for-lineage-2-interlude-ct0-3)).

**Loot rules.** Raid drops belong to the party or command channel of the top damage dealer
(aggro list `damage` sum) and follow its loot mode. Interlude set command-channel size
thresholds for looting rights: 18+ members for raid bosses, 36+ for boss monsters, 225+ in
Antharas' Nest ([Interlude notes](https://lineage2wiki.org/interlude/patch-notes/)); C5 priority
numbers were 255/99/54 for Antharas/Valakas/Baium ([C5 notes](https://lineage2wiki.org/c5/patch-notes/)).
Raid kills also feed a *raid points* ranking that converts to clan reputation monthly.

### 2.7 Quest system

L2J models a quest as a Java/Jython script extending `Quest` and a per-player `QuestState`
([quest doc commit](https://git.vmatviienko.pp.ua/L2j/l2j-server-datapack/commit/1cbd1dc0d12d91fb7fa1d07c18ac4e113787249f),
[L2C4 scripting](https://mintlify.wiki/fermanzolido/L2C4/scripting/overview)):

- `State`: `CREATED (0) -> STARTED (1) -> COMPLETED (2)`.
- Variables: a `Map<String,String>` persisted row-per-variable in
  `character_quests(charId, name, var, value)`. The conventional `cond` integer is the step
  counter shown in the journal; non-linear quests also keep a `__compltdStateFlags` bitmask so
  the client can show which optional steps are done.
- Registration in the constructor: `addStartNpc`, `addTalkId`, `addKillId`, `addAttackId`,
  `addSpawnId`, `addSkillSeeId`, `addAggroRangeEnterId`, `addItemTalkId`, `addEnterZoneId`,
  `registerQuestItems(...)` (items deleted on `exitQuest`).
- Callbacks: `onTalk(npc, player)` returns an HTML page; `onFirstTalk` replaces the default
  dialog; `onAdvEvent(event, npc, player)` is called for every `<a action="bypass -h Quest
  <name> <event>">` link **and** for every quest timer (`startQuestTimer(name, ms, npc,
  player, repeating)`); `onKill(npc, killer, isSummon)`; `onItemUse`; `onEnterZone`.
- Helpers: `giveItems`, `takeItems`, `rewardItems` (rate-adjusted), `hasQuestItems`,
  `getQuestItemsCount`, `addExpAndSp`, `giveAdena`, `playSound("ItemSound.quest_middle")`,
  `getRandomPartyMember(player, cond)` to pick a party member who is on the right step when a
  kill happens, and `exitQuest(repeatable)` which deletes all variables if repeatable or keeps
  `COMPLETED` otherwise.
- Quest items live in a separate inventory tab, cannot be traded or dropped, and are capped
  by a quest-item slot limit independent of the main inventory.

Quest categories: *class transfer* (first class at 20 via one quest per path, second class at
40 via three "Test of..." quests producing Marks, third class at 76 via the class "Saga"
chain), *one-time* story quests, *repeatable* collection quests (`exitQuest(true)`), and from
Gracia onward *daily* quests reset at 06:30 server time by a daily task manager
([ludo.guide reset content](https://www.ludo.guide/guide/lineage-ii/side-quests-activities/daily-quests-repeatable-hunts/daily-weekly-reset-content)).
Class and Noblesse quests gate systems (Phase 7 Olympiad depends on Noblesse).

### 2.8 Instanced content

True instances arrived with Kamael (CT1); Interlude had only zone-locked raids and the
Dimensional Rift (six level bands, nine chambers, 8-10 minute auto-teleport, one hour limit;
[C4 notes](https://lineage2wiki.org/c4/patch-notes/)). An L2J instance template is XML:
`<instance id name maxworlds>` with `<activityTime val="minutes">` (hard duration),
`<allowSummon>`, `<emptyDestroyTime ms>` (destroy when empty), `<ejectTime ms>` (dead players
kicked), `<showTimer>`, `<spawnPoint>` (exit location), `<doorlist>`, `<spawnlist>` groups.
Re-entry is persisted in `character_instance_time(charId, instanceId, time)`. Kamaloka: party
of 2-6, all within 5 levels of the instance level, 30 minutes, one successful run per day,
reset 06:30 ([Rim Kamaloka](https://lineage2.com/news/Rim-kamaloka-event)); Pailaka: solo,
daily; Zaken's ship and Frintezza instances: weekly (Wednesday 06:30 reset).

### 2.9 Teleportation and travel

- **Gatekeepers** in every town and some camps; price scales with distance (hundreds of adena
  between neighbouring towns, tens of thousands for cross-continent at level cap; Rune to
  Primeval Isle 150,000). Teleports were free below level 41 in Interlude-era servers
  ([l2db.net](https://l2db.net/guides/en/l2-interlude-new-player-tips-getting-started-in-the-world-of-lineage-2)).
  L2J stores `teleports.xml`: `id, name, x, y, z, price, fornoble`.
- **Scroll of Escape**: consumable, ~20 s cast, returns to the nearest town; clan-hall and
  castle variants return to the residence; Blessed SoE is instant.
- **Residence teleports**: clan hall and castle gatekeepers offer discounted/exclusive
  destinations to members; Noblesse get extra destinations.
- **Mounts**: a Hatchling pet becomes a Strider at pet level 55 through a quest; mounted
  speed is high but attacking/skills are blocked (later chronicles added Strider Siege
  Assault). Wyverns are castle-lord only, exchanged at the Wyvern Manager for a strider plus
  10 B-grade crystals, and auto-dismount in `NO_LANDING` zones.
- **Boats**: scheduled ferries (Talking Island-Gludin, Giran-Talking Island, Gludin-Rune,
  Rune-Primeval Isle, Innadril tour) with ticket NPCs and a fixed timetable; **airships**
  (Gracia) need a clan level 5 licence.

### 2.10 Day/night cycle and weather

One game day is 4 real hours; L2J's `GameTimeController` uses 10 ticks/s, 6 game days per real
day, and treats game hours 00:00-06:00 as night, so night is 1 real hour in 4
([4gameforum](https://eu.4gameforum.com/threads/651942/)). Effects: night-only spawns
(`periodOfDay`), e.g. vampires near Rune and undead in Devil's Pass ([C4 notes](https://lineage2wiki.org/c4/patch-notes/));
the Dark Elf racial passive *Shadow Sense* (the "night bonus" often remembered as a Shilen
blessing) adds accuracy, evasion and speed only while `isNight()`; Zaken's strength and some
raid scripts key off the clock. Weather is cosmetic only (client fog/rain, High Five's blood
rain), never mechanical.

---

## 3. Design decisions for Nightfall

### 3.1 Tile-based 2D world

- **Authoring format**: Tiled `.tmj` (JSON) maps, one map per *zone file* (a town plus its
  hunting ring, or a dungeon floor). Tile size **32 px**. World coordinates are floats in
  pixels; `Position{x, y}` in the proto already matches. Layers: `ground`, `ground2`,
  `decor`, `overhead` (rendered above entities), `collision` (tile layer, non-zero = blocked),
  `los` (optional; blocks sight but not movement, e.g. tall grass), and object layers
  `zones` (polygons with `type` and properties), `spawns` (points/polygons referencing spawn
  table ids), `portals` (zone-file transitions), `npcs`.
- **Chunking**: the build step (Phase 0 data pipeline) slices every map into **32x32-tile
  chunks** (1024 px). Each chunk is one file the client fetches lazily; the server loads the
  collision/los bitsets for all chunks at boot (1024 bits per layer per chunk, trivial).
- **Seamless but file-partitioned**: zone files abut on a shared world grid; portals exist
  only for interiors (dungeon floors, catacombs) which have their own coordinate space
  (`map_id`). This gives us L2's continent feel outdoors and cheap interiors.
- **Spatial hash (server)**: `HashMap<(map_id, cx, cy), Cell>` keyed by chunk index
  (`cx = x as i32 >> 10`). Every entity is in exactly one cell; moving across a boundary
  re-buckets it. AOI = the 3x3 neighbourhood (radius 1.5 chunks = 1536 px, roughly 48 tiles),
  tunable per map. This is L2's region grid at a different scale.
- **Region activation**: a cell with no players in its 3x3 neighbourhood for 60 s is
  *dormant*: AI does not tick, respawn timers still fire (so bosses respawn), routes pause.
- **Collision and LOS**: tile bitset lookups; LOS is a Bresenham walk over the `los | collision`
  bitset. Pathfinding is A* on tiles within the local 3x3 chunk block (fallback: straight line
  with sliding). Long NPC routes are precomputed waypoint lists.

### 3.2 Monster AI in Rust

**Decision: intention state machine with a hate list, plus utility-scored skill selection;
behaviour trees only for scripted bosses.** L2's AI *is* an FSM and every designer reference
describes it that way; a general BT library adds indirection without adding behaviour we need.
Boss scripts (phases, add waves, cinematic timers) get a small BT/sequence runner.

```rust
pub enum Intention { Idle, Active, Attack(EntityId), Cast{skill: SkillId, target: EntityId},
                     MoveTo(Vec2), Follow(EntityId), ReturnHome }

pub struct HateEntry { hate: f32, damage: u64, last_seen: Tick }
pub struct AiState {
    intention: Intention,
    hate: SmallVec<[(EntityId, HateEntry); 8]>,
    home: Vec2,
    leash_px: f32,              // template.leash (default 1200)
    last_hit_tick: Tick,        // for ATTACK_TIMEOUT (default 120 s)
    next_think: Tick,
    route: Option<RouteCursor>,
    leader: Option<EntityId>,   // minions
}
```

Tick scheduling: the world loop runs at **10 Hz**. AI thinks are bucketed into 10 slots by
`entity_id % 10`, so each NPC thinks once per second (L2J's cadence) and each frame handles a
tenth of the active NPCs. NPCs in `Attack` with a target within attack range think every 500 ms
(two buckets). Dormant cells skip the think pass entirely.

Behaviour table (mirrors 2.5 with our numbers as defaults in the template):

| Rule | Default |
|---|---|
| Aggro scan radius | `aggro_range` (0 = passive; 300-500 px typical) |
| Social call radius | `social_range` (0 = none), same `social_group` id |
| Random walk | 1/30 chance per think, 64-256 px around `home` |
| Leash | drop target beyond `leash_px` from home or beyond `aggro_range * 4`; `ReturnHome` walks back and heals 10%/s |
| Attack timeout | 120 s without landing a hit: clear hate, `ReturnHome` |
| Target switch | most hate; re-evaluated every think; healer/taunt hate from Phase 3 |
| Flee | `flee_hp_pct` (casters/archers 30%) run to `preferred_range` before casting |
| Heal | if `has_heal && hp < 50%` or ally in social group < 30% |
| Minions | spawn with leader, follow within 300 px, respawn 5 min while leader alive |
| Champion | 1% roll on normal spawns (config), x5 HP, x4 XP/SP, extra drop roll |

### 3.3 Spawn table format

Spawns are data, not code. One TOML file per zone file (or embedded in the Tiled `spawns`
object layer; the build step emits the same struct either way):

```toml
[[spawn]]
id = "gludio.ol_mahum_01"
template = 20061            # npc template id
map = "gludio_fields"
point = [1532.0, 880.0]     # or: polygon = [[x,y],[x,y],...] for territory spawns
count = 6
respawn_s = 60
respawn_jitter_s = 30       # delay = respawn_s + rand(0..=jitter)
period = "always"           # "always" | "day" | "night"
heading = 0
route = "gludio.patrol_a"   # optional walking route id
[spawn.ai]                  # optional per-spawn overrides of template.ai
aggro_range = 400
social_group = "ol_mahum"

[[raid]]
id = "raid.queen_ant"
template = 29001
map = "ant_nest_b3"
point = [4096.0, 2048.0]
respawn_min_h = 36
respawn_max_h = 48
minions = [{ template = 29002, count = 4, respawn_s = 300 }, { template = 29003, count = 2 }]
zone = "boss.queen_ant"     # boss zone: NO_RESTART, player cap, eject on dormancy
```

### 3.4 Quest scripting approach

Options considered:

1. **Pure data-driven state machine** (TOML/JSON steps, triggers, conditions, actions).
   Covers kill-N, collect-N, talk-chain, deliver, class transfer. Cheap, hot-reloadable,
   diffable, tool-friendly. Cannot express bespoke logic (timers that spawn escorts, dialog
   branching on party composition, raid-phase quests).
2. **Embedded Rhai**: pure Rust, sandboxed by construction, no C toolchain, `#[derive]`
   bindings, hot reload. Slower than LuaJIT, but quests run on talk/kill events, not per tick.
3. **Embedded Lua via `mlua`**: faster and more familiar, but needs a C Lua build, `unsafe` FFI,
   and sandboxing work; any coroutine-based scripting we might want is also available in Rhai
   via our own timer API.

**Recommendation: data-driven definitions as the primary format, with optional Rhai hook
scripts per quest for the ~10-15% of quests that need custom logic.** The engine exposes the
same event set L2J does (`on_talk`, `on_first_talk`, `on_event` for dialog links and timers,
`on_kill`, `on_item_use`, `on_enter_zone`) and the same helper verbs (`give_items`,
`take_items`, `set_cond`, `exit_quest`, `start_timer`, `spawn`). A quest definition:

```toml
[quest]
id = 101
name = "Sword of Solidarity"
min_level = 9
repeatable = false
quest_items = [1000, 1001]
start_npc = 30008
script = "quests/q101.rhai"      # optional

[[step]]                           # cond = 1
talk = { npc = 30008, dialog = "q101/start.html" }
on_accept = { set_cond = 1, give = [{ item = 1000, count = 1 }] }

[[step]]                           # cond = 2
kill = { templates = [20361, 20362], item = 1001, count = 10, chance = 0.6, party_share = true }
on_complete = { set_cond = 3 }

[[step]]                           # cond = 3
talk = { npc = 30008, dialog = "q101/finish.html" }
on_accept = { reward = [{ item = 738, count = 1 }], exp = 25747, sp = 2171, exit = true }
```

Daily quests carry `reset = "daily@06:30"`; class-transfer quests set `grants_class = <id>`.
State persists as `(character_id, quest_id, state, cond, vars jsonb)`.

### 3.5 Raids, instances, travel, clock

- Raid status (`Alive | Dead{respawn_at} | Dormant | Fighting`) is persisted on every change;
  a `RaidScheduler` task wakes at the next `respawn_at`. HP/MP are checkpointed every 60 s
  during fights so a crash does not reset a boss.
- Instances are `map_id` plus `instance_id`; a template has `duration_min`, `party_min/max`,
  `level_band`, `empty_destroy_s`, `eject_dead_s`, `reenter` (`daily@06:30 | weekly@wed06:30 |
  cooldown_s`). Entities in an instance are bucketed in the same spatial hash with the
  `instance_id` folded into the key.
- Travel: gatekeeper lists are data (`destination, price, min_level_free = 40`); scroll of
  escape is a Phase 3 skill with a 20 s cast and the `nearest_town` target rule; mounts set a
  movement multiplier and a `can_attack = false` flag; boats are an entity on a route that
  carries passengers (positions relative to the boat while `aboard`).
- Clock: `GameClock { epoch, day_len_s = 14_400 }`, `hour() = (elapsed % 14_400) / 600`,
  `is_night() = hour() < 6`. Day/night transitions emit a world event that the spawn manager
  and the skill system (Shadow Sense equivalent) subscribe to. Weather is a client-only
  cosmetic broadcast.

---

## 4. Data model

### 4.1 Persistent entities (Postgres)

| Table | Key columns |
|---|---|
| `npc_templates` | id, name, title, kind (`monster\|raid\|grand\|guard\|merchant\|gatekeeper\|...`), level, stats jsonb, exp, sp, aggro_range, social_range, social_group, leash_px, ai jsonb, corpse_s, skills int[] |
| `spawns` | id, map_id, template_id, point/polygon, count, respawn_s, jitter_s, period, route_id, ai_override jsonb |
| `raid_state` | spawn_id, status, respawn_at, hp, mp, last_killed_by_party |
| `zones` | id, map_id, kind, polygon, params jsonb (speed_mult, damage_per_tick, tax_castle_id, player_cap) |
| `routes` | id, waypoints jsonb |
| `quests` | id, definition jsonb, script_path |
| `character_quests` | character_id, quest_id, state, cond, vars jsonb, completed_at |
| `instance_templates` / `character_instance_reentry` | see 3.5 |
| `teleport_lists` | npc_template_id, destination map/x/y, price, noble_only |

Templates, spawns, zones, routes and quests are *authored* in files and imported; Postgres is
the runtime read model so GM tools (Phase 9) can hot-patch.

### 4.2 Proto sketches (`packages/proto/nightfall/v1/world.proto`)

```proto
syntax = "proto3";
package nightfall.v1;
import "nightfall/v1/game.proto";   // Position

enum EntityKind { ENTITY_KIND_UNSPECIFIED = 0; ENTITY_KIND_PLAYER = 1; ENTITY_KIND_MONSTER = 2;
                  ENTITY_KIND_NPC = 3; ENTITY_KIND_PET = 4; ENTITY_KIND_RAID = 5;
                  ENTITY_KIND_GRAND_BOSS = 6; ENTITY_KIND_DOOR = 7; ENTITY_KIND_BOAT = 8; }

message EntitySpawn {
  uint64 entity_id = 1;
  EntityKind kind = 2;
  uint32 template_id = 3;          // npc template or 0 for players
  string name = 4;
  string title = 5;
  Position position = 6;
  float heading = 7;               // radians
  uint32 level = 8;
  uint32 hp_pct = 9;               // 0..100, exact HP only for self/party (Phase 3)
  uint32 mp_pct = 10;
  repeated uint32 visible_equipment = 11;  // item template ids for rendering
  bool attackable = 12;
  bool aggressive = 13;            // red name
  uint32 clan_id = 14;             // Phase 7
  bool champion = 15;
  uint32 instance_id = 16;
}

message EntityMove {
  uint64 entity_id = 1;
  Position from = 2;
  Position to = 3;                 // destination; client interpolates
  float speed = 4;                 // px/s
  uint64 server_tick = 5;
  bool teleport = 6;               // snap, no interpolation
}

message EntityDespawn { uint64 entity_id = 1; bool died = 2; }

message WorldSnapshot {              // sent on login / chunk block change
  string map_id = 1;
  uint32 instance_id = 2;
  repeated EntitySpawn entities = 3;
  uint32 game_hour = 4;            // 0..23
  bool night = 5;
  repeated uint32 active_zone_flags = 6;  // ZoneFlag bits for the player's current position
  uint64 server_tick = 7;
}

message ZoneFlags {                  // sent when the player's flag set changes
  uint32 flags = 1;                // bitset: PEACE=1, PVP=2, SIEGE=4, WATER=8, SWAMP=16, NO_LANDING=32,
                                   // ARENA=64, NO_RESTART=128, TOWN=256, CASTLE=512, CLAN_HALL=1024
  uint32 tax_castle_id = 2;
}

message NpcDialog {
  uint64 npc_id = 1;
  string html = 2;                 // sanitized subset: <p>, <a action="bypass ...">, <br>
  repeated DialogOption options = 3;  // structured alternative to html links
}
message DialogOption { string label = 1; string bypass = 2; }  // "quest 101 accept", "teleport 7", "shop 3"
message NpcDialogRequest { uint64 npc_id = 1; string bypass = 2; }

enum QuestStatus { QUEST_STATUS_UNSPECIFIED = 0; QUEST_STATUS_CREATED = 1;
                   QUEST_STATUS_STARTED = 2; QUEST_STATUS_COMPLETED = 3; }
message QuestState {
  uint32 quest_id = 1;
  QuestStatus status = 2;
  uint32 cond = 3;
  uint32 completed_steps_mask = 4; // non-linear quests
  map<string, string> vars = 5;
  int64 reset_at_unix = 6;         // daily/weekly
}
message QuestJournal { repeated QuestState quests = 1; }

message RaidStatus { uint32 spawn_id = 1; string name = 2; bool alive = 3;
                     int64 window_open_unix = 4; int64 window_close_unix = 5; }

message GameTime { uint32 hour = 1; uint32 minute = 2; bool night = 3; int64 server_time_ms = 4; }
```

---

## 5. Interfaces

**gRPC (tonic).** Extend `GameService` or add `WorldService`:

```proto
service WorldService {
  rpc EnterWorld(EnterWorldRequest) returns (WorldSnapshot);
  // Real-time ClientEvent/ServerEvent traffic is NOT an RPC: it rides the Phase 0 WebSocket
  // envelope (00-foundations.md §3.2). Only request/response calls live in this service.
  rpc GetChunkManifest(ChunkManifestRequest) returns (ChunkManifest); // chunk ids + content hashes
  rpc InteractNpc(NpcDialogRequest) returns (NpcDialog);
  rpc GetQuestJournal(GetQuestJournalRequest) returns (QuestJournal);
  rpc Teleport(TeleportRequest) returns (TeleportResponse);      // gatekeeper choice, price check
  rpc GetRaidBoard(GetRaidBoardRequest) returns (RaidBoard);     // optional: alive/dead list
}
message ClientEvent { oneof ev { MoveIntent move = 1; NpcDialogRequest talk = 2;
                                 TargetIntent target = 3; UseItemIntent use_item = 4; } }
message ServerEvent { oneof ev { EntitySpawn spawn = 1; EntityMove move = 2; EntityDespawn despawn = 3;
                                 ZoneFlags zone = 4; QuestState quest = 5; NpcDialog dialog = 6;
                                 GameTime time = 7; WorldSnapshot snapshot = 8; } }
```

**Chunk assets** are served over the axum HTTP side (`GET /maps/{map}/chunks/{cx}_{cy}.json`,
immutable, content-hashed) so the game stream carries no map data.

**Server-internal events** (broadcast channel): `EntityEntered(cell)`, `EntityLeft(cell)`,
`Damage{attacker, victim, amount}` (feeds hate), `Died`, `ZoneChanged`, `DayNightChanged`,
`RaidStatusChanged`, `QuestEvent{Kill|Talk|ItemUse|EnterZone}`.

**Client needs**: snapshot on enter, spawn/move/despawn deltas for its AOI, zone flags for HUD
(peace icon, PvP, siege timer), dialog pages, quest journal, game time for the lighting
overlay, raid board (optional).

---

## 6. Rust implementation notes

Module layout under `apps/api/src/`:

```
world/
  mod.rs          // WorldServer: owns maps, spatial hash, tick loop (tokio interval 100 ms)
  map.rs          // Tiled .tmj loader -> Map { chunks, collision: Vec<BitVec>, los, zones, portals }
  spatial.rs      // SpatialHash<(MapId, InstanceId, i32, i32), Cell>; Cell { players, npcs, items }
  zone.rs         // ZoneKind enum, polygon tests (point-in-poly cached per tile at build time)
  los.rs          // bresenham over bitsets; path.rs: A* within 3x3 chunk window
  clock.rs        // GameClock, DayNight events
npc/
  template.rs     // NpcTemplate (serde), AiParams
  spawn.rs        // SpawnTable, respawn BinaryHeap<(Instant, SpawnId)>
  ai.rs           // Intention, think(), hate list; ai_boss.rs: scripted sequences
  minion.rs, route.rs, champion.rs
raid/
  mod.rs          // RaidScheduler, status persistence (sqlx)
quest/
  def.rs          // serde TOML definition -> QuestDef
  engine.rs       // event dispatch: on_kill/on_talk/...; QuestState persistence
  script.rs       // rhai::Engine with registered API (give_items, set_cond, start_timer...)
instance/
  mod.rs          // InstanceManager: create/destroy, reentry table
travel/
  gatekeeper.rs, escape.rs, mount.rs, transport.rs
```

Concurrency: the world tick runs on a dedicated `tokio::task` (or a std thread with
`crossbeam` channels) and owns all mutable world state; gRPC handlers never touch entities
directly but send `ClientEvent`s through an `mpsc` into the tick and receive `ServerEvent`s via
a per-session `mpsc` fed from the AOI broadcaster. This keeps the hot path lock-free. Rayon can
parallelise the AI think pass per cell block once profiling says so; the spatial hash makes
cells independent as long as cross-cell writes (damage, hate) are queued as events applied at
the end of the frame.

Crates: `tiled` or hand-rolled serde structs for `.tmj`; `bitvec` for collision; `pathfinding`
for A*; `rhai` (features `sync`, `no_closure` off); `sqlx` for persistence; `rand` with a
per-tick `SmallRng` seeded from the frame; `smallvec` for hate lists; `tokio::time` for the
respawn heap wakeups; `tracing` spans per subsystem.

Respawn heap: a `BinaryHeap<Reverse<(Instant, SpawnId)>>` polled each tick (O(1) peek) instead
of one timer task per NPC. Dormancy check: per cell `last_player_seen: Tick`.

---

## 7. Client implications

- **Tilemap rendering**: Phaser `Tilemap` per loaded chunk; keep a 5x5 chunk ring around the
  camera loaded, destroy outside 7x7. Chunk JSON is fetched by content hash and cached in
  IndexedDB.
- **Entities**: create sprites on `EntitySpawn`, tween position on `EntityMove`
  (duration = distance / speed, server tick used for reconciliation), destroy on
  `EntityDespawn` with a death animation when `died`.
- **Zone HUD**: peace icon, PvP border, siege clock and water/swamp tint from `ZoneFlags`.
- **Dialog**: render `NpcDialog.options` as buttons; the `html` field is a fallback for
  legacy-style pages (sanitized server-side).
- **Quest journal** from `QuestState`; show `cond`-indexed step text from the client-side
  quest text bundle (Phase 8 localization).
- **Day/night**: a full-screen tint layer driven by `GameTime` (dusk ramps over game hours
  18-20, dawn 5-7); night-only spawns need no client logic.
- **Travel UI**: gatekeeper list from `NpcDialog.options` with prices; boat boarding shows a
  countdown from the transport schedule.
- The client sends **intents only** (move target tile, talk, bypass string); it never asserts
  positions or quest progress.

---

## 8. Open questions

1. **Transport**: resolved. Phase 0 settled on a WebSocket at `/ws` on the axum listener carrying
   protobuf `ClientMessage`/`ServerMessage` envelopes for real-time traffic, with gRPC (tonic-web)
   for request/response only. The world events in §5 are envelope payloads, not a bidi RPC.
2. **Interior maps**: one coordinate space with offset interiors vs. separate `map_id`
   spaces (chosen here). Revisit if we want seamless cave entrances.
3. **Chunk size** (32x32 tiles) and AOI radius (1.5 chunks) are guesses; tune after measuring
   entity density in a town.
4. **Rhai vs. data-only**: if the first 30 quests never need a script, drop Rhai to keep the
   binary and attack surface smaller.
5. **Champion mobs**: a L2J-ism, not retail. Keep as a config flag off by default?
6. **Epic boss windows**: the two reference sets disagree; pick ours during tuning
   (proposal: QA 24+12 h, Core/Orfen 36+12 h, Zaken 48+12 h, Baium 96+24 h, dragons 144+48 h).
7. **Raid loot rights**: command-channel size thresholds (Phase 7) or simple top-damage-party.
8. **Seven Signs**: catacomb access gating by faction is a large social/economic system; out
   of scope for v1, but the dungeon level bands are useful as a content map.
9. L2J constants quoted from memory (`L2World`, `GameTimeController`, grand boss intervals) must
   be verified against the repository before being cited in tuning docs.

---

## 9. Sources

- L2J geodata structure (regions, blocks, cells): https://l2j-geodata.sourceforge.net/index.php?p=4
- L2J server source (world regions, AI, spawns, quests): https://bitbucket.org/l2jserver/l2j_server and https://github.com/L2J/L2J_Server
- L2J AI constants commit (RANDOM_WALK_RATE, MAX_ATTACK_TIMEOUT, MAX_DRIFT_RANGE): https://git.vmatviienko.pp.ua/L2j/l2j-server-game/commit/6759959a98d72a3de119f54284197fe6d7195741
- L2J raid curse commit: https://git.vmatviienko.pp.ua/L2j/l2j-server-game/commit/0cd797a74646dbeaa6b67c0bfee1a825b560c968
- L2J datapack quest documentation commit: https://git.vmatviienko.pp.ua/L2j/l2j-server-datapack/commit/1cbd1dc0d12d91fb7fa1d07c18ac4e113787249f
- L2J zone XML sample (custom zones issue): https://bitbucket.org/l2jserver/l2j-server-game/issues/180/custom-zones
- L2J Mobius project: https://l2jmobius.org ; Interlude fork used as a path reference: https://github.com/Hl4p3x/L2JServer_C6_Interlude
- L2C4 (Mobius Chronicle 4) documentation: raid bosses https://mintlify.wiki/fermanzolido/L2C4/systems/raid-bosses ; AI scripts https://mintlify.wiki/fermanzolido/L2C4/scripting/ai-scripts ; NPC API https://mintlify.wiki/fermanzolido/L2C4/api/npc ; scripting overview https://mintlify.wiki/fermanzolido/L2C4/scripting/overview
- Lineage 2 patch notes (lineage2wiki.org): C1 https://lineage2wiki.org/c1/patch-notes/ ; C3 https://lineage2wiki.org/c3/patch-notes/ ; C4 https://lineage2wiki.org/c4/patch-notes/ ; C5 https://lineage2wiki.org/c5/patch-notes/ ; Interlude https://lineage2wiki.org/interlude/patch-notes/ ; High Five https://lineage2wiki.org/hi-five/patch-notes/
- Epic boss windows: https://guildorder.com/games/lineage_2/wiki/epic-boss-windows-and-scouting
- Raid boss farming (Interlude): https://l2db.net/guides/en/raidboss-farming-guide-for-lineage-2-interlude-ct0-3
- Interlude XP zones by level: https://l2db.net/guides/en/best-xp-zones-interlude ; new player tips (free teleport): https://l2db.net/guides/en/l2-interlude-new-player-tips-getting-started-in-the-world-of-lineage-2
- Day/night timing: https://eu.4gameforum.com/threads/651942/
- Zaken mechanics: https://www.ludo.guide/guide/lineage-ii/zaken
- Seven Signs cycle and seal stones: https://www.ludo.guide/guide/lineage-ii/side-quests-activities/seven-signs-questline
- Instances and reset times: https://www.ludo.guide/guide/lineage-ii/side-quests-activities/instance-dungeons ; https://www.ludo.guide/guide/lineage-ii/side-quests-activities/daily-quests-repeatable-hunts/daily-weekly-reset-content ; Kamaloka rules https://lineage2.com/news/Rim-kamaloka-event
