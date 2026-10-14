# Phase 6: World and Content

This phase is about zones, monsters and AI, raids, quests, instances, travel, and day/night. It depends on Phases 0-5
(networking and persistence, stat formulas, classes, combat, items, drops).

---

## 1. Purpose and scope

**Delivers**

- A seamless 2D world that is made of chunked tilemaps. It has server-side collision, line of sight,
  spatial partitioning and area-of-interest (AOI) streaming.
- Zone volumes (town/peace, battle, siege, water, swamp, no-landing, arena, boss, no-restart).
  Each zone volume sets or clears flags on the characters in it.
- NPC templates, spawn tables (fixed, random-in-territory, day/night, routes) and respawn timers
  with jitter.
- A server-authoritative monster AI (intention state machine, hate list, social
  aggro, leash/return-home, flee/heal behaviors, minions).
- Raid bosses and grand ("epic") bosses, with a persistent status and respawn windows, minion
  control, loot-right rules and level gates of the raid-curse type.
- A quest engine: quest states and variables, quest items, kill/talk/item/zone hooks, class
  transfer and repeatable/daily quests. It has a data-driven definition format and an embedded
  scripting escape hatch.
- Instanced dungeons: templates, timers, party-size limits, re-entry cooldowns.
- Travel: gatekeepers, escape scrolls, residence teleports, mounts and scheduled transports.
- A game clock with a day/night cycle. The cycle controls spawns and racial/skill bonuses.

**Excludes**

- Drop tables and spoil (Phase 5).
- Combat formulas and the aggro *damage* calculation (Phase 3).
- Siege-specific NPCs and castle ownership (Phase 7).
- Client camera/UI (Phase 8).
- GM spawn tools and live events (Phase 9).

---

## 2. Reference: how Lineage 2 does it

### 2.1 World structure, regions and geodata

The L2 world is one continent with no loading screens. The server (L2J) divides it in two ways:

**Map tiles (geodata).** The world is a grid of 32768 x 32768 unit tiles with the index `XX_YY`
(for example, `20_18` has the origin). The L2J geodata description gives this structure. The world has
32x32 regions. The populated world is 11 regions wide and 16 regions high. Each region is 256x256
*blocks*, each block is 8x8 *cells*, and a cell is 16 world units square ([L2J geodata
homepage](https://l2j-geodata.sourceforge.net/index.php?p=4)). 256 * 8 * 16 = 32768 units.
This value agrees with the tile size. Blocks have three storage forms:

- *Flat*: one height for all the block.
- *Complex*: 64 cells, each a 16-bit value. The height is in the upper bits and 4 NSWE
  passability bits are in the low nibble.
- *Multilayer*: complex, plus a layer count for each cell, for bridges and floors.

Geodata gives the server three functions. `getHeight(x, y, z)` gives Z
snapping. `canMoveToTarget(from, to)` walks cells and examines the NSWE bits.
`canSeeTarget(a, b)` steps along a 3D line and makes sure that the terrain never goes above it.
Without geodata, L2J servers let mobs attack through walls. Players see this difference between "geodata on" and "geodata off" private servers more than any other difference.

**World regions (`L2World`).** Independently of geodata, L2J puts each object into a bucket of a
coarse region grid for AOI. The constants come from `L2World.java` in L2J High Five. Examine them
against [bitbucket.org/l2jserver](https://bitbucket.org/l2jserver/l2j_server), because the research could
not get the file:

| Constant | Value | Meaning |
|---|---|---|
| `SHIFT_BY` | 12 | region edge = 2^12 = 4096 units |
| `TILE_SIZE` | 32768 | geodata tile edge |
| `TILE_X_MIN..MAX`, `TILE_Y_MIN..MAX` | 11..26, 10..26 | populated tiles |
| `TILE_ZERO_COORD_X/Y` | 20, 18 | the tile that has (0,0) |
| `MAP_MIN_X / MAX_X` | -294912 / 229375 | derived: `(TILE_X_MIN - 20) * 32768`, `(TILE_X_MAX - 20 + 1) * 32768 - 1` |
| `MAP_MIN_Y / MAX_Y` | -262144 / 294911 | the same derivation on Y |
| `OFFSET_X / OFFSET_Y` | 72 / 64 | `abs(MAP_MIN >> SHIFT_BY)` |
| `REGIONS_X / REGIONS_Y` | 127 / 135 | `(MAP_MAX >> SHIFT_BY) + OFFSET` |

`getRegion(x, y) = regions[(x >> 12) + 72][(y >> 12) + 64]`. Each region knows its 8
neighbors. The server collects the "visible objects" for a player from the 3x3 block of regions, and then
filters them by distance. If a region has no players for approximately one minute, the server *deactivates* it. Its NPC
AI tasks stop and the random walk pauses. This is how one process runs ~60k NPCs.

### 2.2 Zone types

Zones are polygon or cuboid volumes that the server loads from XML (`<zone type="PeaceZone" shape="NPoly"
minZ=... maxZ=...>`) ([L2J zone XML sample](https://bitbucket.org/l2jserver/l2j-server-game/issues/180/custom-zones)).
Each zone sets one or more `ZoneId` bits on the characters in it. This table gives the core set and the effect of each zone:

| Zone | Flags set | Effect |
|---|---|---|
| Town | PEACE, TOWN | No attacks, the PvP flag is cleared, the owner castle taxes the shop prices (`taxById`), respawn point on death |
| Peace (generic) | PEACE | No attacks. Used for quest huts and the Olympiad lobby |
| Battle / PvP | PVP | Free PvP, no PK karma, the flag is set automatically |
| Arena | PVP + revive-in-place | Coliseum, Monster Track |
| Siege | SIEGE, PVP during siege | At siege start, the server teleports non-registered players out. The death penalty is less. HQ flags are permitted |
| Castle / Fort / Clan hall | CASTLE / FORT / CLAN_HALL | Residence functions, door control, `NO_SUMMON_FRIEND` |
| Water | WATER | Swim animation, breath meter. In older chronicles, no combat under water |
| Swamp | SWAMP | Movement speed decreases (zone parameter, in most conditions -50%) |
| Damage / Effect | (none) | Periodic HP loss or skill application (flame tower zones, lava) |
| No-landing | NO_LANDING | After a countdown, a mounted wyvern must dismount |
| Jail | JAIL, NO_SUMMON | GM punishment |
| Mother Tree | MOTHER_TREE | HP/MP regen bonus for Elves in the Elven village |
| Boss / No-restart | NO_RESTART, NO_BOOKMARK | Logout moves the player to town. Raid zones with entry items |
| Olympiad stadium | PVP + scripted | Match rules, spectators |
| Condition zones | NO_ITEM_DROP, NO_STORE, NO_SUMMON_FRIEND | Marketplaces, event areas |

### 2.3 Layout: towns, hunting zones, dungeons, catacombs

The world has castle towns as its structure. Each town has hunting grounds around it, and their level
increases with the distance. Five starter villages (Talking Island, Elven, Dark Elven, Orc, Dwarven)
are for levels 1-20. Gludin/Gludio is for 20-35, Dion for 30-45 and Giran for 40-55. Oren/Hunters Village is for 50-65,
Aden for 60-75, and Goddard/Rune/Schuttgart for 70-80 ([C4 notes](https://lineage2wiki.org/c4/patch-notes/),
[Interlude XP zones](https://l2db.net/guides/en/best-xp-zones-interlude)). Open dungeons
(Cruma Tower 40+, Tower of Insolence 50-75, Dragon Valley, Antharas' Lair) are multi-floor
interiors with no instancing. Players go to a floor on foot or with consumable items.
In High Five, the Dimensional Stone (10,000 adena) selects ToI floor 1/5/10.

The Seven Signs catacombs and necropolises (C3) are the canonical "dungeon for each level band".
Only players who registered with Dawn or Dusk in that period can go in. The winner side
controls them during the seal period. The C3 notes give this list ([source](https://lineage2wiki.org/c3/patch-notes/)):

| Dungeon | Level band | Entrance town |
|---|---|---|
| Necropolis of Sacrifice | 20-30 | Gludio |
| Heretics Catacomb / Pilgrim's Necropolis | 30-40 | Dion |
| Catacomb of the Branded / Worshipers Necropolis | 40-50 | Giran / Innadril |
| Catacomb of the Apostate / Patriots Necropolis | 50-60 | Oren / Gludio |
| Catacomb of the Witch / Ascetics' and Martyrs' Necropolis | 60-70 | Aden / Oren, Giran |
| Dark Omens, Forbidden Path / Saints', Disciples' Necropolis | 70-80 | Oren, Aden / Innadril, Aden |

Seven Signs has a two-week cycle. The first week is the competition (players collect seal stones: A/B/C
stones come from 70+/40-69/20-39 monsters). The second week is the seal validation
([ludo.guide](https://www.ludo.guide/guide/lineage-ii/side-quests-activities/seven-signs-questline)).

### 2.4 NPC templates and spawns

L2J loads an `L2NpcTemplate` from `npcs/*.xml`. The template has these fields: `id`, `displayId`, `name`,
`title`, `type` (`L2Monster`, `L2RaidBoss`, `L2GrandBoss`, `L2Guard`, `L2Npc`, `L2Merchant`,
`L2Teleporter`, `L2Warehouse`, `L2Chest`, `L2FeedableBeast`, `L2ControlTower`, `L2SiegeFlag`
and approximately twenty more), `level`, `race`, `sex`, `baseStats` (STR..MEN, HP/MP/CP, regen, pAtk,
mDef, atkSpd, speeds, attack range), `collision` radius/height, `exp`, `sp`, `rhand/lhand`
items, `skills`, `dropLists`, `corpseTime` (the seconds that the body stays), and an `ai` block.
That block has these fields:
`aggroRange` (300-500 for most aggressive mobs, 0 for passive), `clanHelpRange`
(social aggro radius), `clan` ids (NPCs with the same clan string help each other), `dodge`,
`isChaos` (attacks other monsters), `isAggressive`, `canMove`, `soulShot/spiritShot` counts and
chances, `minSkillChance/maxSkillChance`, `targetable`, `talkable`, `undying`, `attackable`
([L2C4 NPC API](https://mintlify.wiki/fermanzolido/L2C4/api/npc)).

A spawn (`L2Spawn`) is `(npcId, x, y, z, heading, count, respawnDelay, respawnRandom,
periodOfDay)`; the datapack `spawnlist` table has exactly those columns and a location id.
On death, the spawn schedules a respawn after `respawnDelay + rand(0, respawnRandom)` seconds. The `periodOfDay` values
0/1/2 mean always / only day / only night (`DayNightSpawnManager`). For territory spawns
(`<territory>` polygons in `spawns/*.xml`), each respawn selects a random point in a polygon.
Walking routes (`WalkingManager`, `Routes.xml`) list waypoints with a `delay` and optional
`chat` text. An NPC on a route does MOVE_TO and short waits one after the other. It continues the route after combat.

### 2.5 The monster AI state machine

`L2AttackableAI` is an intention machine. Intentions: `IDLE`, `ACTIVE`, `REST`, `ATTACK`,
`CAST`, `MOVE_TO`, `FOLLOW`, `PICK_UP`, `INTERACT`. Events cause the transitions
(`onEvtAttacked`, `onEvtAggression`, `onEvtArrived`, `onEvtArrivedBlocked`, `onEvtDead`,
`onEvtSeeSpell`, `onEvtForgetObject`). A *think* task also causes transitions. This task runs each second for each
NPC in an active region ([L2J AI commit with constants](https://git.vmatviienko.pp.ua/L2j/l2j-server-game/commit/6759959a98d72a3de119f54284197fe6d7195741)):

- `RANDOM_WALK_RATE = 30`: if an `ACTIVE` mob can move, it has a 1-in-30 chance for each think to
  walk a short distance around its spawn.
- `MAX_ATTACK_TIMEOUT = 1200` ticks (120 s; before, 300 ticks = 30 s). If the mob cannot
  hit its target for this time, it forgets the hate list and goes back home.
- `MAX_DRIFT_RANGE = 300`: minions stay in this radius of their leader.

**thinkActive.** If the hate list is empty, the mob scans the known list for any entity that passes
`autoAttackCondition`. The conditions are: in `aggroRange`, not dead, not GM-invisible, not in a peace zone.
Guards attack monsters and players with karma. `isAggressive` mobs attack all players
(the dwarf *Silent Move* and the Spoil/level checks decrease the range). NPCs with `isChaos` attack other
monsters. Each candidate gets `addDamageHate(target, 0, 1)`, so that the first hate entry exists.
If hate exists, the intention becomes `ATTACK`. If not, these actions occur:

- If minions are farther than `MAX_DRIFT_RANGE`, they move back to the leader.
- Raid bosses and `isReturningToSpawnPoint` NPCs walk home and regenerate to full.
- The random walk occurs.

**thinkAttack.** Target = `getMostHated()`. The target can be dead, out of range (> aggro range
and > 2000 units) or not reachable. In that condition, the mob removes it (`stopHating`) and does the evaluation again. Social aggro: at the
first engagement, the mob calls the NPCs in `clanHelpRange` with the same `clan` id. If such an NPC is not already in a
fight, it adds the attacker to its hate list ("faction call"). The skill selection scans the
template skills by category in this sequence:

1. Heal when its own HP < 50% (or an ally in range).
2. Buff when a buff is missing.
3. Debuff/attack skills with a `minSkillChance..maxSkillChance` roll.
4. If none applies, auto-attack.

Ranged/caster mobs with `dodge` or low HP flee to range before they cast. If the
target is farther than the attack range, the intention is `MOVE_TO`/follow, with geodata pathing. The timeout
record resets after each hit that is a success.

**Hate list.** There is one `AggroInfo { attacker, hate, damage }` for each attacker. `addDamageHate(attacker,
damage, aggro)` adds `aggro`. The default is damage, scaled by the Phase 3 aggro formulas. Taunts add
flat hate. Healers of the target get hate equal to the heal quantity. `getMostHated()` takes the maximum
hate of the reachable attackers. `reduceHate` is for aggression-reversal skills. Some raid scripts
use random values: Antharas monitors the top 3 attackers and adds `damage + rand(3000)`
([L2C4 AI scripts](https://mintlify.wiki/fermanzolido/L2C4/scripting/ai-scripts)).

**Mob types.**

- *Normal* monsters.
- *Minions*: a leader's `MinionList` spawns them. They spawn again on a timer while the leader is alive, and the server removes them when the leader dies.
- *Champions*: an L2J option. A normal mob has a random chance to spawn with x7-8 HP/XP/drops and a title.
- *Raid bosses*: type `L2RaidBoss`, persistent status, minions, raid curse.
- *Grand/epic bosses*: `L2GrandBoss`, scripted multi-phase AI, a dedicated boss zone, and a persistent spawn window in `grandboss_data`.

### 2.6 Raid and epic bosses

The server keeps normal raid bosses (levels 20-85) in `raidboss_spawnlist(boss_id, loc_x, loc_y,
loc_z, heading, amount, respawn_delay, respawn_random, respawn_time, currentHP, currentMP)` with
status `ALIVE | DEAD | UNDEFINED`. On death, `respawn_time = now + rand(min, max) * multiplier`
([L2C4 raid docs](https://mintlify.wiki/fermanzolido/L2C4/systems/raid-bosses)). In most datapacks, the
values are 12 h + rand(0, 24 h). Interlude community guides give fixed timers of 30-360
minutes for low raids ([l2db.net](https://l2db.net/guides/en/raidboss-farming-guide-for-lineage-2-interlude-ct0-3)).
The server records HP/MP. Thus, if players killed half of a boss before a restart, the boss comes back at that HP.

Epic bosses spawn in a *window*. The window is a fixed minimum after death plus a random tail
([guildorder](https://guildorder.com/games/lineage_2/wiki/epic-boss-windows-and-scouting)).
There are two reference sets. The L2J `GrandBoss.properties` defaults come from memory and must be examined. The L2C4 docs were
downloaded:

| Boss | Level | Zone / entry | L2J H5 default (min + random) | L2C4 docs window |
|---|---|---|---|---|
| Queen Ant | 40 | Ant Nest (Gludio) | 36 h + 17 h | 36-48 h |
| Core | 50 | Cruma Tower top | 60 h + 24 h | 40-60 h |
| Orfen | 50 | Sea of Spores | 48 h + 20 h | 48-72 h |
| Zaken | 60 | Devil's Isle, Zaken's ship | 60 h + 20 h | - |
| Baium | 75 | Tower of Insolence 13F, wake with Blooded Fabric | 168 h + 48 h | 120-192 h |
| Antharas | 79 | Antharas' Lair, Portal Stone | 264 h + 72 h | 192-216 h |
| Valakas | 85 | Forge of the Gods, Floating Stone | 264 h + 72 h | 264-312 h |
| Frintezza | 85 | Last Imperial Tomb (Interlude, only command channel) | 48 h + 8 h | - |

These boss-specific mechanics are good to copy:

- Queen Ant: her Nurse Ants heal her, and the Royal Guards spawn again.
- Baium: he sleeps (`ASLEEP`) until an item wakes him. Then he fights with five Archangels. He
  despawns if nobody is in the zone for 30 min.
- Zaken: he teleports between ship rooms and spawns clones. He is weaker at night ([ludo.guide](https://www.ludo.guide/guide/lineage-ii/zaken)).
- Antharas/Valakas: they have the statuses `DORMANT -> WAITING -> FIGHTING -> DEAD`, a 200-player zone cap
  and a 30-minute reconnect window ([C4 notes](https://lineage2wiki.org/c4/patch-notes/)).

Minions use `MinionList`, with a 5-minute respawn while the leader is alive.

**Raid curse.** A player can attack, heal or buff a raid (or its minions) while the player is more than 8 levels
above it. In that condition, the player gets the *Raid Curse*. Physical attackers become petrified, and casters become silent
([L2J raid curse commit](https://git.vmatviienko.pp.ua/L2j/l2j-server-game/commit/0cd797a74646dbeaa6b67c0bfee1a825b560c968),
[l2db.net](https://l2db.net/guides/en/raidboss-farming-guide-for-lineage-2-interlude-ct0-3)).

**Loot rules.** Raid drops go to the party or the command channel of the top damage dealer
(the sum of `damage` in the aggro list). The drops follow the loot mode of that group. Interlude set command-channel size
thresholds for loot rights: 18+ members for raid bosses, 36+ for boss monsters, 225+ in
Antharas' Nest ([Interlude notes](https://lineage2wiki.org/interlude/patch-notes/)). The C5 priority
numbers were 255/99/54 for Antharas/Valakas/Baium ([C5 notes](https://lineage2wiki.org/c5/patch-notes/)).
Raid kills also add to a *raid points* ranking. Each month, the ranking becomes clan reputation.

### 2.7 Quest system

L2J makes a quest as a Java/Jython script that extends `Quest`, and a `QuestState` for each player
([quest doc commit](https://git.vmatviienko.pp.ua/L2j/l2j-server-datapack/commit/1cbd1dc0d12d91fb7fa1d07c18ac4e113787249f),
[L2C4 scripting](https://mintlify.wiki/fermanzolido/L2C4/scripting/overview)):

- `State`: `CREATED (0) -> STARTED (1) -> COMPLETED (2)`.
- Variables: a `Map<String,String>`, kept as one row for each variable in
  `character_quests(charId, name, var, value)`. The conventional `cond` integer is the step
  counter that the journal shows. Non-linear quests also keep a `__compltdStateFlags` bitmask. Thus,
  the client can show which optional steps are done.
- Registration in the constructor: `addStartNpc`, `addTalkId`, `addKillId`, `addAttackId`,
  `addSpawnId`, `addSkillSeeId`, `addAggroRangeEnterId`, `addItemTalkId`, `addEnterZoneId`,
  `registerQuestItems(...)` (the server removes these items on `exitQuest`).
- Callbacks: `onTalk(npc, player)` returns an HTML page. `onFirstTalk` replaces the default
  dialog. The server calls `onAdvEvent(event, npc, player)` for each `<a action="bypass -h Quest
  <name> <event>">` link **and** for every quest timer (`startQuestTimer(name, ms, npc,
  player, repeating)`); `onKill(npc, killer, isSummon)`; `onItemUse`; `onEnterZone`.
- Helpers: `giveItems`, `takeItems`, `rewardItems` (rate-adjusted), `hasQuestItems`,
  `getQuestItemsCount`, `addExpAndSp`, `giveAdena`, `playSound("ItemSound.quest_middle")`.
  When a kill occurs, `getRandomPartyMember(player, cond)` selects a party member who is on the correct step.
  `exitQuest(repeatable)` removes all variables if the quest is repeatable. If not, it keeps
  `COMPLETED`.
- Quest items are in a separate inventory tab. Players cannot trade or drop them. A quest-item slot limit, independent of the main inventory,
  is the cap for them.

Quest categories:

- *Class transfer*:
  - The first class at 20, through one quest for each path.
  - The second class at
  40, through three "Test of..." quests that give Marks.
  - The third class at 76, through the class "Saga"
  chain.
- *One-time* story quests.
- *Repeatable* collection quests (`exitQuest(true)`).
- From Gracia, *daily* quests. A daily task manager resets them at 06:30 server time
  ([ludo.guide reset content](https://www.ludo.guide/guide/lineage-ii/side-quests-activities/daily-quests-repeatable-hunts/daily-weekly-reset-content)).

Class and Noblesse quests are gates for systems (the Phase 7 Olympiad depends on Noblesse).

### 2.8 Instanced content

True instances came with Kamael (CT1). Interlude had only zone-locked raids and the
Dimensional Rift (six level bands, nine chambers, 8-10 minute auto-teleport, one hour limit;
[C4 notes](https://lineage2wiki.org/c4/patch-notes/)). An L2J instance template is XML. It has
`<instance id name maxworlds>` with `<activityTime val="minutes">` (hard duration),
`<allowSummon>`, `<emptyDestroyTime ms>` (destroy when empty), `<ejectTime ms>` (dead players
are removed), `<showTimer>`, `<spawnPoint>` (exit location), `<doorlist>`, and `<spawnlist>` groups.
The server records re-entry in `character_instance_time(charId, instanceId, time)`. These are the rules of some instances:

- Kamaloka: a party
  of 2-6, all in 5 levels of the instance level. 30 minutes, one successful run each day,
  reset at 06:30 ([Rim Kamaloka](https://lineage2.com/news/Rim-kamaloka-event)).
- Pailaka: solo, daily.
- Zaken's ship and Frintezza instances: weekly (reset on Wednesday at 06:30).

### 2.9 Teleportation and travel

- **Gatekeepers** are in all towns and in some camps. The price increases with the distance. It is hundreds of adena
  between neighbor towns, and tens of thousands across the continent at level cap (Rune to
  Primeval Isle 150,000). In servers of the Interlude period, teleports were free below level 41
  ([l2db.net](https://l2db.net/guides/en/l2-interlude-new-player-tips-getting-started-in-the-world-of-lineage-2)).
  L2J stores `teleports.xml`: `id, name, x, y, z, price, fornoble`.
- **Scroll of Escape**: a consumable with a ~20 s cast. It returns the player to the nearest town. The clan-hall and
  castle variants return the player to the residence. Blessed SoE is instant.
- **Residence teleports**: clan hall and castle gatekeepers give discounted/exclusive
  destinations to members. Noblesse get more destinations.
- **Mounts**: a Hatchling pet becomes a Strider at pet level 55 through a quest. The mounted
  speed is high, but attacks and skills are blocked (later chronicles added Strider Siege
  Assault). Only castle lords can have Wyverns. The Wyvern Manager exchanges a strider and
  10 B-grade crystals for a wyvern. Wyverns dismount automatically in `NO_LANDING` zones.
- **Boats**: scheduled ferries (Talking Island-Gludin, Giran-Talking Island, Gludin-Rune,
  Rune-Primeval Isle, Innadril tour) with ticket NPCs and a fixed timetable. **Airships**
  (Gracia) need a clan level 5 licence.

### 2.10 Day/night cycle and weather

One game day is 4 real hours. The L2J `GameTimeController` uses 10 ticks/s and 6 game days for each real
day. It sets game hours 00:00-06:00 as night. Thus, night is 1 real hour in 4
([4gameforum](https://eu.4gameforum.com/threads/651942/)). These are the effects:

- Spawns that occur only at night (`periodOfDay`). For example, vampires near Rune and undead in Devil's Pass ([C4 notes](https://lineage2wiki.org/c4/patch-notes/)).
- The Dark Elf racial passive *Shadow Sense* adds accuracy, evasion and speed only while `isNight()`.
  Players frequently remember this "night bonus" as a Shilen
  blessing.
- The strength of Zaken and some raid scripts use the clock.

Weather is only cosmetic (client fog/rain, the blood
rain of High Five). It is never mechanical.

---

## 3. Design decisions for Nightfall

### 3.1 Tile-based 2D world

- **Authoring format**: Tiled `.tmj` (JSON) maps, one map for each *zone file* (a town and its
  hunting ring, or a dungeon floor). The tile size is **32 px**. World coordinates are floats in
  pixels. `Position{x, y}` in the proto already agrees. Layers:
  - `ground`, `ground2`,
  `decor`, `overhead` (rendered above entities).
  - `collision` (tile layer, non-zero = blocked).
  - `los` (optional; blocks sight but not movement, for example tall grass).
  - The object layers
  `zones` (polygons with `type` and properties), `spawns` (points/polygons that refer to spawn
  table ids), `portals` (zone-file transitions), `npcs`.
- **Chunking**: the build step (Phase 0 data pipeline) cuts each map into **32x32-tile
  chunks** (1024 px). Each chunk is one file that the client gets when it needs it. At boot, the server loads the
  collision/los bitsets for all chunks (1024 bits for each layer for each chunk, a very small quantity).
- **Seamless but file-partitioned**: zone files touch each other on a shared world grid. Portals exist
  only for interiors (dungeon floors, catacombs), which have their own coordinate space
  (`map_id`). This gives us the L2 continent feel outdoors, and interiors at a low cost.
- **Spatial hash (server)**: `HashMap<(map_id, cx, cy), Cell>` with the chunk index as the key
  (`cx = x as i32 >> 10`). Each entity is in exactly one cell. When an entity moves across a boundary,
  it goes into a new bucket. AOI = the 3x3 neighborhood (radius 1.5 chunks = 1536 px, approximately 48 tiles).
  You can tune it for each map. This is the L2 region grid at a different scale.
- **Region activation**: if a cell has no players in its 3x3 neighborhood for 60 s, it is
  *dormant*. The AI does not tick, the respawn timers continue to fire (thus bosses respawn), and routes pause.
- **Collision and LOS**: these are tile bitset lookups. LOS is a Bresenham walk over the `los | collision`
  bitset. Pathfinding is A* on tiles in the local 3x3 chunk block (fallback: straight line
  with sliding). Long NPC routes are waypoint lists that are calculated before.

### 3.2 Monster AI in Rust

**Decision: an intention state machine with a hate list, and skill selection with utility scores.
Behavior trees are only for scripted bosses.** The L2 AI *is* an FSM, and each designer reference
gives this description. A general BT library adds indirection, but it does not add behavior that we need.
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

Tick schedule: the world loop runs at **10 Hz**. AI thinks go into 10 buckets by
`entity_id % 10`. Thus, each NPC thinks one time each second (the L2J cadence), and each frame does a
tenth of the active NPCs. NPCs in `Attack` with a target in attack range think each 500 ms
(two buckets). Dormant cells do not do the think pass.

Behavior table (the same as 2.5, with our numbers as defaults in the template):

| Rule | Default |
|---|---|
| Aggro scan radius | `aggro_range` (0 = passive; 300-500 px in most conditions) |
| Social call radius | `social_range` (0 = none), the same `social_group` id |
| Random walk | 1/30 chance for each think, 64-256 px around `home` |
| Leash | remove the target when it is farther than `leash_px` from home or farther than `aggro_range * 4`. `ReturnHome` walks back and heals 10%/s |
| Attack timeout | 120 s with no hit: clear the hate, `ReturnHome` |
| Target switch | most hate. Evaluated again at each think. Healer/taunt hate from Phase 3 |
| Flee | `flee_hp_pct` (casters/archers 30%): run to `preferred_range` before the cast |
| Heal | if `has_heal && hp < 50%` or an ally in the social group < 30% |
| Minions | spawn with the leader, follow in 300 px, respawn 5 min while the leader is alive |
| Champion | 1% roll on normal spawns (config), x5 HP, x4 XP/SP, one more drop roll |

### 3.3 Spawn table format

Spawns are data, not code. There is one TOML file for each zone file (or the data is embedded in the Tiled `spawns`
object layer). In the two conditions, the build step emits the same struct:

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

We examined these options:

1. **Pure data-driven state machine** (TOML/JSON steps, triggers, conditions, actions).
   It covers kill-N, collect-N, talk-chain, deliver and class transfer. It has a low cost, hot reload,
   easy diffs and is easy for tools. It cannot show bespoke logic (timers that spawn escorts, dialog
   branches that use the party composition, raid-phase quests).
2. **Embedded Rhai**: only Rust, sandboxed by construction, no C toolchain, `#[derive]`
   bindings, hot reload. It is slower than LuaJIT, but quests run on talk/kill events, not on each tick.
3. **Embedded Lua through `mlua`**: faster and more familiar. But it needs a C Lua build, `unsafe` FFI,
   and sandbox work. If we want coroutine-based scripts, Rhai also gives them
   through our own timer API.

**Recommendation: data-driven definitions as the primary format, with optional Rhai hook
scripts for each quest, for the ~10-15% of quests that need custom logic.** 

The engine gives the
same event set as L2J (`on_talk`, `on_first_talk`, `on_event` for dialog links and timers,
`on_kill`, `on_item_use`, `on_enter_zone`). It also gives the same helper verbs (`give_items`,
`take_items`, `set_cond`, `exit_quest`, `start_timer`, `spawn`). This is a quest definition:

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

Daily quests have `reset = "daily@06:30"`. Class-transfer quests set `grants_class = <id>`.
The state is kept as `(character_id, quest_id, state, cond, vars jsonb)`.

### 3.5 Raids, instances, travel, clock

- The server records the raid status (`Alive | Dead{respawn_at} | Dormant | Fighting`) at each change.
  A `RaidScheduler` task starts again at the next `respawn_at`. During fights, the server makes a checkpoint of HP/MP each 60 s.
  Thus, a crash does not reset a boss.
- Instances are `map_id` plus `instance_id`. A template has `duration_min`, `party_min/max`,
  `level_band`, `empty_destroy_s`, `eject_dead_s`, `reenter` (`daily@06:30 | weekly@wed06:30 |
  cooldown_s`). The server puts the entities in an instance into buckets of the same spatial hash, with the
  `instance_id` added to the key.
- Travel:
  - Gatekeeper lists are data (`destination, price, min_level_free = 40`).
  - The scroll of escape is a Phase 3 skill with a 20 s cast and the `nearest_town` target rule.
  - Mounts set a movement multiplier and a `can_attack = false` flag.
  - Boats are an entity on a route that carries passengers (positions are relative to the boat while `aboard`).
- Clock: `GameClock { epoch, day_len_s = 14_400 }`, `hour() = (elapsed % 14_400) / 600`,
  `is_night() = hour() < 6`. Day/night transitions emit a world event. The spawn manager
  and the skill system (the Shadow Sense equivalent) subscribe to this event. Weather is a cosmetic
  broadcast, only for the client.

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
| `instance_templates` / `character_instance_reentry` | refer to 3.5 |
| `teleport_lists` | npc_template_id, destination map/x/y, price, noble_only |

The team writes templates, spawns, zones, routes and quests in files, and then imports them. Postgres is
the runtime read model. Thus, GM tools (Phase 9) can hot-patch the data.

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

**Chunk assets**: the axum HTTP side sends them (`GET /maps/{map}/chunks/{cx}_{cy}.json`,
immutable, content-hashed). Thus, the game stream does not carry map data.

**Server-internal events** (broadcast channel): `EntityEntered(cell)`, `EntityLeft(cell)`,
`Damage{attacker, victim, amount}` (gives data to the hate list), `Died`, `ZoneChanged`, `DayNightChanged`,
`RaidStatusChanged`, `QuestEvent{Kill|Talk|ItemUse|EnterZone}`.

**Client needs**:

- A snapshot on entry.
- Spawn/move/despawn deltas for its AOI.
- Zone flags for the HUD
(peace icon, PvP, siege timer).
- Dialog pages and the quest journal.
- The game time for the lighting
overlay.
- The raid board (optional).

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
`crossbeam` channels). It is the owner of all the mutable world state. gRPC handlers never touch entities
directly. They send `ClientEvent`s through an `mpsc` into the tick, and they receive `ServerEvent`s through
an `mpsc` for each session, which the AOI broadcaster fills. This keeps the hot path lock-free. When profiling shows the need, Rayon can
parallelize the AI think pass for each cell block. The spatial hash makes
cells independent, with one condition. The cross-cell writes (damage, hate) must go into a queue as events, and the tick applies them at
the end of the frame.

Crates:

- `tiled` or hand-written serde structs for `.tmj`.
- `bitvec` for collision.
- `pathfinding`
for A*.
- `rhai` (features `sync`, `no_closure` off).
- `sqlx` for persistence.
- `rand` with a
`SmallRng` for each tick, seeded from the frame.
- `smallvec` for hate lists.
- `tokio::time` for the
respawn heap wakeups.
- `tracing` spans for each subsystem.

Respawn heap: a `BinaryHeap<Reverse<(Instant, SpawnId)>>` that the server polls at each tick (O(1) peek). This replaces
one timer task for each NPC. Dormancy check: `last_player_seen: Tick` for each cell.

---

## 7. Client implications

- **Tilemap rendering**: one Phaser `Tilemap` for each loaded chunk. Keep a 5x5 chunk ring around the
  camera loaded, and destroy the chunks outside 7x7. The client gets the chunk JSON by content hash and keeps it in an
  IndexedDB cache.
- **Entities**: make sprites on `EntitySpawn`, and tween the position on `EntityMove`
  (duration = distance / speed; the server tick is for reconciliation). Destroy them on
  `EntityDespawn`, with a death animation when `died`.
- **Zone HUD**: peace icon, PvP border, siege clock and water/swamp tint from `ZoneFlags`.
- **Dialog**: render `NpcDialog.options` as buttons. The `html` field is a fallback for
  pages of the legacy style (the server sanitizes them).
- **Quest journal**: from `QuestState`. Show the step text for each `cond` index from the client-side
  quest text bundle (Phase 8 localization).
- **Day/night**: `GameTime` controls a full-screen tint layer (dusk increases over game hours
  18-20, dawn 5-7). Spawns that occur only at night need no client logic.
- **Travel UI**: the gatekeeper list comes from `NpcDialog.options`, with prices. Boat boarding shows a
  countdown from the transport schedule.
- The client sends **only intents** (move target tile, talk, bypass string). It never sets
  positions or quest progress.

---

## 8. Open questions

1. **Transport**: resolved. Phase 0 selected a WebSocket at `/ws` on the axum listener. It carries
   protobuf `ClientMessage`/`ServerMessage` envelopes for real-time traffic. gRPC (tonic-web) is
   only for request/response. The world events in §5 are envelope payloads, not a bidi RPC.
2. **Interior maps**: one coordinate space with offset interiors, or separate `map_id`
   spaces (the selection here). Examine this again if we want seamless cave entrances.
3. **Chunk size** (32x32 tiles) and AOI radius (1.5 chunks) are estimates. Tune them after a measurement of the
   entity density in a town.
4. **Rhai or only data**: if the first 30 quests never need a script, remove Rhai. This keeps the
   binary and the attack surface smaller.
5. **Champion mobs**: these come from L2J, not from retail. Do we keep them as a config flag that is off by default?
6. **Epic boss windows**: the two reference sets do not agree. Select our values during tuning
   (proposal: QA 24+12 h, Core/Orfen 36+12 h, Zaken 48+12 h, Baium 96+24 h, dragons 144+48 h).
7. **Raid loot rights**: command-channel size thresholds (Phase 7), or only the party with the top damage.
8. **Seven Signs**: the catacomb access gate by faction is a large social/economic system. It is not
   in the scope of v1, but the dungeon level bands are useful as a content map.
9. The L2J constants that come from memory (`L2World`, `GameTimeController`, grand boss intervals) must
   be examined against the repository before the tuning docs use them.

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
