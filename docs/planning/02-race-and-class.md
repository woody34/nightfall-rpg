# Phase 2: Race and class

Status: reference design. The [executable plan](../plans/phase-2-race-and-class.md) and [verification outcome](../plans/phase-2-race-and-class-outcome.md) define and record the verified contracts, token bridge policy, and closed verification receipts. The implemented Phase 1 cap 85 replaces the reference cap 60. Playable transfers stop at tier 2 (levels 20/40), and tier 3 has metadata only. Production creation starts at (0,0). The Class Master is at (126,128), radius 3. Local Phase 2, with E4 token provisioning, is proven complete when this final reviewed docs commit merges. The evidence follows:

- Root main merged and pushed `cf9225db5cd49432e96de97ffb31e01ad0f3cf94` (tracked tree `3dcf6b9c8c832ebc2715bef9ec812784a097bd74` matches candidate `e568fd2`).
- The root Game build (28.43 s, exit 0) passed.
- The root Editor build (19.03 s, exit 0) passed.
- The separate root smoke suites (Nightfall.Class 8/8, TemplateTarget 1/1, total 9/9, zero failures/skips, both exit 0) passed.
- These checks passed with seven root-local dependency links and a clean tracked checkout. The only untracked item was the unchanged user trace `d8a708831677fc5ffa0d72df9d66142bcefc4144f36d6cc043f33b4899824a32`.

The external self-hosted Linux runner is still not registered. The existing two-week reliability programme is still a separate operational follow-up. It is not a local acceptance gate, and it is not a new 14-day wait. This phase depends on Phase 1, and it gives input to Phases 3 and 4.

## 1. Purpose and scope

This phase gives all the items that are necessary to make a character with a race and a class. It also gives the items to increase its level and move it through the class tree:

- Race definitions: base stat arrays, movement speed, collision, racial traits, starting zone.
- Class definitions: the class tree (base class, first/second/third transfer), growth tables for each class (HP/MP/CP by level), skill trees that only the class can use, equipment proficiency unlocks.
- Class transfer rules: level gates, cost/quest hooks, the changes that a transfer makes.
- Subclass rules: data model and eligibility rules (the implementation is for a later phase, see section 3).
- A data-driven definition format (TOML), a Rust data model in `apps/api/src/character/`, and proto additions.

These items are not in the scope of this phase:

- The stat formulas (Phase 1).
- The skills that a class learns. Phase 3 defines the skill data. This phase only defines the *tree* that tells who learns what, at which level and SP cost.
- Item grades (Phase 4).
- Quest scripts for transfers (Phase 6).
- Dual class and Awakening (Goddess of Destruction era; not in the reference scope).

## 2. Reference: how Lineage 2 does it

All numbers below are from the Interlude to High Five era, if not stated differently. The primary data source is the L2J High Five datapack (`data/stats/chars/`). This datapack is an accurate dump of the client/server class templates. We compared it with the l2db.net class pages.

### 2.1 Races

There are five classic races, plus Kamael (added in Chaotic Throne: The Kamael, 2007). All races except Dwarf and Kamael have two start paths: Fighter and Mystic. Dwarves are fighter-only. Kamael are fighter-only and split by gender (male and female Soldiers have different trees).

#### Base stat arrays per starting class

Source: L2J `data/stats/chars/baseStats/*.xml` (`baseSTR`...`baseMEN`). These values agree with the l2db.net C1/C4/Interlude class pages. These are level-1 values. In classic L2, they never change with level. Only equipment, buffs, tattoos (henna) and later dyes change them.

| Starting class (classId) | STR | DEX | CON | INT | WIT | MEN | Sum |
|---|---|---|---|---|---|---|---|
| Human Fighter (0) | 40 | 30 | 43 | 21 | 11 | 25 | 170 |
| Human Mystic (10) | 22 | 21 | 27 | 41 | 20 | 39 | 170 |
| Elven Fighter (18) | 36 | 35 | 36 | 23 | 14 | 26 | 170 |
| Elven Mystic (25) | 21 | 24 | 25 | 37 | 23 | 40 | 170 |
| Dark Fighter (31) | 41 | 34 | 32 | 25 | 12 | 26 | 170 |
| Dark Mystic (38) | 23 | 23 | 24 | 44 | 19 | 37 | 170 |
| Orc Fighter (44) | 40 | 26 | 47 | 18 | 12 | 27 | 170 |
| Orc Mystic (49) | 27 | 24 | 31 | 31 | 15 | 42 | 170 |
| Dwarven Fighter (53) | 39 | 29 | 45 | 20 | 10 | 27 | 170 |
| Male Kamael Soldier (123) | 41 | 33 | 31 | 29 | 11 | 25 | 170 |
| Female Kamael Soldier (124) | 39 | 35 | 30 | 28 | 11 | 27 | 170 |

The sum of each array is 170. That is the design budget: a race/class is a distribution of 170 points across six stats. Nightfall must keep the fixed-budget rule, because it makes balance arguments easier to control.

#### Other per-template static data (L2J `staticData`)

| Template | Walk | Run | Breath | Safe fall | Lvl-1 HP | Lvl-1 MP | Lvl-1 CP |
|---|---|---|---|---|---|---|---|
| Human Fighter | 80 | 115 | 100 | 250 | 80 | 30 | 32 |
| Human Mystic | 78 | 120 | 100 | 200 | 101 | 40 | 50.5 |
| Elven Fighter | 90 | 125 | 150 | 350 | 89 | 30 | 35.6 |
| Elven Mystic | 85 | 122 | 150 | 300 | 104 | 40 | 52 |
| Dark Fighter | 85 | 122 | 150 | 350 | 94 | 30 | 37.6 |
| Dark Mystic | 85 | 122 | 150 | 300 | 106 | 40 | 53 |
| Orc Fighter | 70 | 117 | 90 | 200 | 80 | 30 | 40 |
| Orc Mystic | 70 | 121 | 90 | 250 | 95 | 40 | 47.5 |
| Dwarven Fighter | 80 | 115 | 80 | 180 | 80 | 30 | 56 |
| Kamael Soldier (M/F) | 87 | 122 | 100 | 500 | 95 / 97 | 30 / 40 | 47.5 / 48.5 |

All templates have the same values for these fields:

- `basePAtk` 4 (3 for Human Mystic), `baseMAtk` 6, `baseCritRate` 4, `basePAtkSpd` 300, `baseAtkRange` 20, `baseRndDam` 10.
- Base P.Def for each slot (chest 31 / legs 18 / head 12 / feet 7 / gloves 8 / underwear 3 / cloak 1 for fighters; chest 15 / legs 8 for mystics).
- Base M.Def (ears 9 each, rings 5 each, necklace 13).

The slot P.Def is the "naked" defence. When the character equips armor in the slot, the armor replaces it (L2J `FuncPDefMod` subtracts the slot base when the slot is filled).

#### Racial traits

In the classic era, L2 has almost no explicit racial passive skills. The racial identity comes from the stat array, through the Phase 1 derived formulas. Some race-specific skill trees and zones also add to it:

| Race | Where the trait actually comes from | Effect in play |
|---|---|---|
| Human | Balanced array; widest class tree (five fighter branches, including two tanks, and five mystic branches). | Can do all roles; Human mystics have the best MEN/WIT balance for healers. |
| Elf | DEX 35 (highest, the same as female Kamael), run speed 125 (highest), breath 150, safe fall 350. Accuracy and evasion are `sqrt(DEX)*6 + level`; P.Atk speed is `base * DEX_bonus`; crit rate is `baseCrit * DEX_bonus * 10`. WIT 23 on mystics gives the fastest casting (`mAtkSpd = base * WIT_bonus`). Elf is also the only race that gets an HP/MP regen bonus in the Mother Tree zone (L2J `ZoneId.MOTHER_TREE`). | Fastest attack/cast speed, highest evasion, low damage (STR 36, INT 37). |
| Dark Elf | STR 41 (highest fighter STR), DEX 34, INT 44 (highest INT), but CON 32 and MEN 37. STR controls P.Atk (`base * STR_bonus * levelMod`) and physical-skill crit chance (`STR_bonus * skillCritChance`); DEX controls crit rate. | Highest burst crit and nuke damage; lowest HP/M.Def ("glass cannon"). |
| Orc | CON 47 (highest; max HP = `base * CON_bonus`, HP regen scales with CON), MEN 42 on mystics (M.Def, debuff resistance). DEX 26 (lowest). Slow walk (70). Orc mystics are the only buffer line that uses fighter-style melee (Shaman -> Overlord/Warcryer). | Most HP and regen, slowest, worst evasion/crit. |
| Dwarf | CON 45, DEX 29, WIT 10. Weight limit is `floor(CON_bonus * 69000)` (L2J `PcStat.getMaxLoad`), thus Dwarves can carry the most. Skills that only Dwarves have: Spoil (id 254), Sweeper (42), Create Item / Crystallize (172 / 248), Dwarven Craft. Only Dwarves have access to crafting recipes at Create Item levels. | The economy race: spoil loot, craft, carry. Fighter only, two branches. |
| Kamael | Light armor only (no heavy/robe mastery). Soul system (absorb souls on hit, use them for skills). Racial transformations (wings) at 3rd class. Female Soldier DEX 35; male STR 41. Safe fall 500. No mystic path. Cannot subclass to/from other races. | Hybrid fighter/caster with a unique resource; not in the Nightfall MVP (see section 3). |

#### Starting zones

L2J `data/stats/chars/pcCreationPoints.xml` uses the class id as the key for spawn point groups. World coordinates (L2 units; ~16 units for each metre):

| Race / path | Village | Typical spawn (x, y, z) |
|---|---|---|
| Human Fighter | Talking Island Village | (-71338, 258271, -3104) |
| Human Mystic | Talking Island, Einhasad temple area | (-90875, 248162, -3570) |
| Elf (both) | Elven Village (Shadow of the Mother Tree) | (46045, 41251, -3440) |
| Dark Elf (both) | Dark Elf Village | (28295, 11063, -4224) |
| Orc (both) | Orc Village (Immortal Plateau) | (-56733, -113459, -690) |
| Dwarf | Dwarven Village | (108644, -173947, -400) |
| Kamael (both) | Isle of Souls, Kamael Village | (-125607, 38452, 1152) |

### 2.2 The class tree

Levels: base class at 1, first transfer at 20, second at 40, third at 76. The class ids are the retail ids that the client and L2J `classList.xml` use. 3rd-class ids start at 88, because ids 58-87 were reserved. Ids 119-122 are internal NPC-class placeholders.

| Race | Base (lvl 1) | 1st class (lvl 20) | 2nd class (lvl 40) | 3rd class (lvl 76) |
|---|---|---|---|---|
| Human | Human Fighter (0) | Warrior (1) | Gladiator (2) | Duelist (88) |
| | | | Warlord (3) | Dreadnought (89) |
| | | Human Knight (4) | Paladin (5) | Phoenix Knight (90) |
| | | | Dark Avenger (6) | Hell Knight (91) |
| | | Rogue (7) | Treasure Hunter (8) | Adventurer (93) |
| | | | Hawkeye (9) | Sagittarius (92) |
| | Human Mystic (10) | Human Wizard (11) | Sorcerer (12) | Archmage (94) |
| | | | Necromancer (13) | Soultaker (95) |
| | | | Warlock (14) | Arcana Lord (96) |
| | | Cleric (15) | Bishop (16) | Cardinal (97) |
| | | | Prophet (17) | Hierophant (98) |
| Elf | Elven Fighter (18) | Elven Knight (19) | Temple Knight (20) | Eva's Templar (99) |
| | | | Sword Singer (21) | Sword Muse (100) |
| | | Elven Scout (22) | Plains Walker (23) | Wind Rider (101) |
| | | | Silver Ranger (24) | Moonlight Sentinel (102) |
| | Elven Mystic (25) | Elven Wizard (26) | Spellsinger (27) | Mystic Muse (103) |
| | | | Elemental Summoner (28) | Elemental Master (104) |
| | | Elven Oracle (29) | Elven Elder (30) | Eva's Saint (105) |
| Dark Elf | Dark Fighter (31) | Palus Knight (32) | Shillien Knight (33) | Shillien Templar (106) |
| | | | Bladedancer (34) | Spectral Dancer (107) |
| | | Assassin (35) | Abyss Walker (36) | Ghost Hunter (108) |
| | | | Phantom Ranger (37) | Ghost Sentinel (109) |
| | Dark Mystic (38) | Dark Wizard (39) | Spellhowler (40) | Storm Screamer (110) |
| | | | Phantom Summoner (41) | Spectral Master (111) |
| | | Shillien Oracle (42) | Shillien Elder (43) | Shillien Saint (112) |
| Orc | Orc Fighter (44) | Orc Raider (45) | Destroyer (46) | Titan (113) |
| | | Monk (47) | Tyrant (48) | Grand Khavatari (114) |
| | Orc Mystic (49) | Orc Shaman (50) | Overlord (51) | Dominator (115) |
| | | | Warcryer (52) | Doom Cryer (116) |
| Dwarf | Dwarven Fighter (53) | Scavenger (54) | Bounty Hunter (55) | Fortune Seeker (117) |
| | | Artisan (56) | Warsmith (57) | Maestro (118) |
| Kamael (M) | Male Soldier (123) | Trooper (125) | Berserker (127) | Doombringer (131) |
| | | | Male Soul Breaker (128) | Male Soul Hound (132) |
| Kamael (F) | Female Soldier (124) | Warder (126) | Female Soul Breaker (129) | Female Soul Hound (133) |
| | | | Arbalester (130) | Trickster (134) |
| Kamael (both, 3rd sub only) | | | Inspector (135) | Judicator (136) |

Totals: 9 (or 11 with Kamael) base classes, 18 (22) first classes, 31 (36) second classes, 31 (36) third classes. The L2J data has these errors:

- In `classList.xml`, the parent of Elemental Master is Elven Wizard (26), not Elemental Summoner (28). This is a data bug. Do not copy it.
- The parent of Bladedancer is Shillien Knight (33), not Palus Knight (32). This is also a data bug (Bladedancer is a 2nd class from Palus Knight).

Inspector is available only as the third subclass of a Kamael, after two other Kamael subclasses get to 75.

### 2.3 What a class transfer changes

1. **Growth template.** Each class id has its own `baseStats/<Class>.xml` with a `lvlUpgainData` table of HP/MP/CP for each level. The STR..MEN array does *not* change on transfer (Gladiator and Duelist both have the 40/30/43/21/11/25 of Human Fighter). The difference is the HP/MP/CP curve. Example (the Gladiator file and the Duelist file are the same, thus the 3rd transfer does not change growth):

   | Level | HP | MP | CP |
   |---|---|---|---|
   | 1 | 80 | 30 | 72 |
   | 20 | 327 | 144 | 294.3 |
   | 40 | 1044 | 359.1 | 939.6 |
   | 76 | 3061.8 | 1155.6 | 2755.62 |
   | 85 | 3643.2 | 1385.1 | 3278.88 |

   Human Fighter level 1 CP is 32, and Gladiator level 1 CP is 72. The L2J file for the 2nd class gives the full 1..85 curve again, and the server swaps curves on transfer (max HP/CP increase immediately).

2. **Skill tree.** `skillTrees/classSkillTree.xml` has one `<skillTree classId=N>` for each class. A class inherits the tree of each ancestor (L2J walks `parentClassId`). Each entry has these fields: `skillId`, `skillLvl`, `getLevel` (character level gate), `levelUpSp` (SP cost), optional `autoGet` (free on level-up), `learnedByNpc`, and optional item requirements (spellbooks). Human Fighter examples: Power Strike 1-3 at level 5 for 50 SP each, Weapon Mastery 1 at level 5 for 160 SP, Power Strike 4-6 at level 10 for 370 SP each. The transfer unlocks the tree of the new class immediately.

3. **Equipment proficiency.** Expertise is an auto skill (id 239). The level controls it, not the class. The grades are: D at 20, C at 40, B at 52, A at 61, S at 76, S80 at 80, S84 at 84. If a character uses a grade above its expertise, a penalty applies (Phase 4). The class *does* control the weapon/armor *masteries* (passive bonuses). It also controls the weapons that skills require (`<using kind="SWORD,BLUNT"/>` conditions on skills). Also, Kamael cannot wear heavy/robe armor at all.

4. **Identity.** Class name, title eligibility, Olympiad bracket, and (at 3rd class) the visual "class mark". Subclass and certification eligibility (2.4) also depend on the class level.

#### Class transfer gating (quests)

- **1st transfer (20).** In the classic game, there are three quests for each branch. A character can start them at 18-19, and completes them at 20. Each quest gives a "Mark", and the character exchanges the Marks with a class master. For example, Human Fighter -> Warrior: "Trial of the Warrior" (Master Auron, Mark of the Warrior, completed with Grand Master Bitz). Knight: "Trial of the Human Knight" (Grand Master Ramos). Rogue: "Trial of the Human Rogue" (Master Terry).
- **2nd transfer (40).** Three quests. Each quest gives a Mark (for example Mark of Challenger, Mark of Duelist, Mark of Trust for Gladiator). The class master requires all the Marks. A character can start the quests at 35-39.
- **3rd transfer (76).** A "Saga of the <class>" quest chain for each 3rd class (Interlude+). Some parts of the chain are instanced. The chain gives the 3rd class and a Hero-style skill.
- **Classic / private-server shortcut.** Later official Classic servers and almost all private servers let a player buy the transfer for adena or a token. L2J has `SubclassWithoutQuests`-style toggles, and sells Marks through multisell. For subclass eligibility, the L2J village master examines only `isNoble() || Q234 Fate's Whisper || Q235 Mimir's Elixir`.

### 2.4 Subclass system

L2J defaults (`character.properties`): `MaxSubclass = 3`, `BaseSubclassLevel = 40`, `MaxSubclassLevel = 80` (75 in Interlude; 80 from Gracia Final/High Five), `SubclassWithoutQuests = False`, `SubclassEverywhere = False`.

These rules are in L2J `PlayerClass.getAvailableSubclasses` and `L2VillageMasterInstance`:

1. The main class must have completed the 2nd transfer and be at level 75. It must also have completed Fate's Whisper (Q234) or Mimir's Elixir (Q235), or be Noble.
2. A new subclass starts at level 40 as a *2nd-class* profession (the player selects for example "Gladiator", not "Warrior"). It has its own XP/SP, skills, and level. The player switches at a village master, and the switch swaps the full skill set and stats. The switch removes buffs, but not the buffs with the flag `isStayOnSubclassChange`.
3. These classes are never allowed as a subclass: Overlord, Warsmith.
4. An Elf main cannot take a Dark Elf class, and a Dark Elf main cannot take an Elf class. Non-Kamael cannot take Kamael classes. Kamael can take *only* Kamael classes (and the Soul Breaker of the same gender). Inspector requires two other Kamael subclasses at 75.
5. A character cannot take the "equivalent" class of its own class. The five equivalence sets are {Dark Avenger, Paladin, Temple Knight, Shillien Knight}, {Treasure Hunter, Abyss Walker, Plains Walker}, {Hawkeye, Silver Ranger, Phantom Ranger}, {Warlock, Elemental Summoner, Phantom Summoner}, {Sorcerer, Spellsinger, Spellhowler}. A character also cannot take its own base class, or a class that it already has as another subclass (L2J examines this with `equalsOrChildOf`).
6. Certification ("stacking") skills (`skillTrees/subClassSkillTree.xml`): when a subclass gets to 65 / 70 / 75 / 80, the character gets a certificate item. The character spends certificates on main-class passives: Emergent Ability (Attack / Defense / Casting, at 65), Master Ability (at 70), transformation and class-group skills (at 75/80). The maximum is 4 certified skills for each subclass, and 12 in total with three subclasses. At the village master, the character can cancel (refund) certificates for adena.
7. Dual class (Goddess of Destruction, 2013+) replaces one subclass with a second "main" class. This class levels to the cap and can Awaken. This is not in the reference scope. The data model below keeps space for it (`is_dual`).

## 3. Design decisions for Nightfall

### 3.1 MVP scope

| Decision | Choice | Alternatives considered |
|---|---|---|
| Races | 5: Human, Elf, Dark Elf, Orc, Dwarf. Kamael is not included. | 3 races (Human/Elf/Dark Elf) cost less for art. But the racial stat identity is data-only, thus all five races cost the same on the server. Kamael requires trees split by gender, light-only armor rules and the Soul resource, and these touch combat. Do Kamael after the MVP. |
| Base classes | 9 (all L2 base classes). | We considered a fighter-only start. But mystics are necessary to validate the magic formulas in Phase 3. |
| 1st transfer | All 18 first classes at level 20, data-only. | Fewer classes would make the tree asymmetric across races. |
| 2nd transfer | Data defined for all 31. But in MVP, only the skill trees for a vertical slice (Gladiator, Paladin, Treasure Hunter, Hawkeye, Sorcerer, Bishop, Prophet, Spellsinger, Shillien Knight, Destroyer, Warcryer, Bounty Hunter) have data. | All 31 skill trees are a content task for Phase 3/6. The class *definitions* are cheap; the *skills* are not. |
| 3rd transfer | Defined in data, not reachable in MVP (level cap 60 for MVP; see Phase 1). | |
| Subclass | The data model and eligibility rules are implemented. The UI and village master flow are for a later phase. | This prevents a schema migration later. |
| Transfer gating | Level gate + a `ClassTransferToken` item that a player can buy from a class master NPC (Classic-style). Quest hooks are a `requirements` list in data, thus Phase 6 can add quests without a schema change. | Full quest chains (L2 retail) are a Phase 6 content item. |
| Stat budget | Fixed 170-point budget. The loader makes sure of it. | |
| Growth curves | HP/MP/CP tables for each class, copied from L2J as the initial tuning set. Each class has its table, with inheritance: if a class has no table, it uses the table of its parent. | We considered formula-based growth (`hp = a + b*lvl + c*lvl^2`). Tables are easier to tune for each class, and they match L2 exactly. |
| Class ids | Use the retail ids again (0..57, 88..118), thus L2 reference data and community knowledge map 1:1. Kamael ids 123-136 are reserved. | Dense Nightfall-only ids would be cleaner, but they break all cross-references. |
| Name uniqueness | Class names are Nightfall's own (Nightfall is L2-inspired, not L2). The TOML has `display_name` plus `l2_ref` for traceability. | |

### 3.2 Racial traits in Nightfall

Keep the L2 approach: traits are mostly the stat array plus movement/collision. A small list of explicit racial passives makes the differences easy for players to see:

| Race | Explicit passive (given at level 1) | Notes |
|---|---|---|
| Human | `Adaptable`: +5% XP/SP gain | Replaces the L2 "widest tree" advantage with a bonus that players can see. |
| Elf | `Forest Step`: +3 run speed, +3% evasion; `Mother Tree Attunement`: +50% HP/MP regen in Elven forest zones | Copies the L2 numbers (run 125 vs 115) and the Mother Tree zone. |
| Dark Elf | `Shadow Precision`: +5% critical damage | L2 DE has no explicit passive; we added it so that players can see the difference. |
| Orc | `Iron Constitution`: +10% HP regen, +5 Shock (stun) resistance | |
| Dwarf | `Pack Mule`: +20% weight limit; `Keen Eye`: Spoil/Sweep skill access; `Artisan Hands`: crafting skill access | The base formula of the weight limit stays CON-driven, as in Phase 1. |

### 3.3 Transfer mechanics

- A transfer is possible when `level >= class.min_level`, `current_class == class.parent`, and all `requirements` are met (MVP: the character has a `ClassTransferToken` of tier 1/2). The transfer consumes the token.
- On transfer, the server does these steps:
  1. Set `class_id`.
  2. Calculate max HP/MP/CP again from the new growth table.
  3. Give the `auto_get` skills from the new tree, up to the current level.
  4. Keep the current HP/MP percentages.
  5. Broadcast `ClassChanged`.
- No downgrades. Transfers are permanent (as in L2).

### 3.4 Subclass (post-MVP, data reserved)

Implement L2 rules 1-5 from 2.4 as a pure function `subclass::eligible(main: &CharacterSheet, candidate: ClassId) -> Result<(), SubclassDenied>`. Subclass start level 40, cap = `main_level_cap - 5`. The character has the certification skills as a `certifications: Vec<Certification>`. The limits (4 for each sub, 12 total) are constants in data.

## 4. Data model

### 4.1 TOML race definition (`data/races/elf.toml`)

```toml
id = "elf"                 # stable string id, also proto enum name
proto = "RACE_ELF"         # Race enum value in game.proto
display_name = "Elf"
l2_ref = "Elf"
starting_zone = "elven_village"
start_points = [[46045, 41251], [46117, 41247], [46182, 41198]]  # map units (x, y)
mystic_path = true

[movement]
walk = 90
run = 125
swim = 50

[collision]
radius_m = 8.0
height_m = 24.0
radius_f = 7.0
height_f = 23.0

[passives]                 # skill ids granted at level 1
grant = ["racial.forest_step", "racial.mother_tree_attunement"]

[traits]
breath = 150
safe_fall = 350
```

### 4.2 TOML class definition (`data/classes/elven_fighter.toml`)

```toml
id = 18                    # retail id, stable
key = "elven_fighter"
display_name = "Elven Fighter"
l2_ref = "Elven Fighter"
race = "elf"
tier = 0                   # 0 base, 1 first, 2 second, 3 third
parent = -1                # class id or -1
min_level = 1
archetype = "fighter"      # fighter | mystic (drives naked P.Def slots etc.)

[base_stats]               # must sum to 170
str = 36
dex = 35
con = 36
int = 23
wit = 14
men = 26

[combat_base]
p_atk = 4
m_atk = 6
crit_rate = 4
p_atk_spd = 300
atk_range = 20
rnd_dam = 10

[naked_def]                # slot -> base P.Def when slot is empty
chest = 31
legs = 18
head = 12
feet = 7
gloves = 8
underwear = 3
cloak = 1

[naked_mdef]
rear = 9
lear = 9
rfinger = 5
lfinger = 5
neck = 13

# HP/MP/CP growth. Either an explicit table or "inherit".
[growth]
table = "elven_fighter"    # refers to data/growth/elven_fighter.csv (level,hp,mp,cp)

[transfer]                 # requirements to *become* this class
requires = []              # base class: none
```

```toml
# data/classes/temple_knight.toml
id = 20
key = "temple_knight"
display_name = "Temple Knight"
race = "elf"
tier = 2
parent = 19
min_level = 40
archetype = "fighter"
[growth]
table = "temple_knight"
[transfer]
requires = [{ item = "class_transfer_token_2", count = 1 }]
subclass_equivalents = ["paladin", "dark_avenger", "shillien_knight"]
subclass_allowed = true    # false for Overlord, Warsmith
```

Growth CSV example (`data/growth/gladiator.csv`): `level,hp,mp,cp` with rows 1..85. The values come from L2J as the initial set.

### 4.3 Rust data model

```rust
// apps/api/src/character/race.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaceId { Human, Elf, DarkElf, Orc, Dwarf, Kamael }

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RaceDef {
    pub id: RaceId,
    pub display_name: String,
    pub starting_zone: String,
    pub start_points: Vec<(i32, i32)>,
    pub mystic_path: bool,
    pub movement: Movement,          // walk, run, swim
    pub collision: Collision,
    pub passives: Passives,          // grant: Vec<SkillKey>
    pub traits: RaceTraits,          // breath, safe_fall
}

// apps/api/src/character/class.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize)]
#[serde(transparent)]
pub struct ClassId(pub u16);         // retail ids: 0..=57, 88..=118, 123..=136

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Archetype { Fighter, Mystic }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub struct BaseStats { pub str: u8, pub dex: u8, pub con: u8, pub int: u8, pub wit: u8, pub men: u8 }

impl BaseStats {
    pub const BUDGET: u16 = 170;
    pub fn sum(&self) -> u16 { [self.str, self.dex, self.con, self.int, self.wit, self.men].iter().map(|&v| v as u16).sum() }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ClassDef {
    pub id: ClassId,
    pub key: String,
    pub display_name: String,
    pub race: RaceId,
    pub tier: u8,                     // 0..=3
    pub parent: Option<ClassId>,
    pub min_level: u8,
    pub archetype: Archetype,
    pub base_stats: BaseStats,
    pub combat_base: CombatBase,
    pub naked_def: SlotDef,
    pub naked_mdef: SlotMDef,
    pub growth: GrowthRef,            // table name, resolved at load
    pub transfer: TransferRules,      // requires: Vec<Requirement>, subclass_allowed, subclass_equivalents
}

// apps/api/src/character/template.rs
pub struct GrowthTable { pub rows: Vec<GrowthRow> }          // index = level - 1
pub struct GrowthRow { pub hp: f32, pub mp: f32, pub cp: f32 }

pub struct ClassRegistry {
    classes: HashMap<ClassId, ClassDef>,
    by_key: HashMap<String, ClassId>,
    children: HashMap<ClassId, Vec<ClassId>>,
    growth: HashMap<String, Arc<GrowthTable>>,
}

impl ClassRegistry {
    pub fn load(dir: &Path) -> anyhow::Result<Self>;        // parses TOML + CSV, then validate()
    pub fn get(&self, id: ClassId) -> Option<&ClassDef>;
    pub fn ancestors(&self, id: ClassId) -> impl Iterator<Item = ClassId>;   // for skill-tree inheritance
    pub fn transfer_options(&self, from: ClassId, level: u8) -> Vec<ClassId>;
    pub fn growth(&self, id: ClassId) -> &GrowthTable;      // walks parents if class has no table
    fn validate(&self) -> anyhow::Result<()>;              // budget == 170, tree acyclic, tier == parent.tier + 1,
                                                           // min_level in {1,20,40,76}, race matches parent race
}
```

Character-side state (the server stores it; see Phase 0 for storage):

```rust
pub struct CharacterSheet {
    pub id: CharacterId,
    pub name: String,
    pub race: RaceId,
    pub sex: Sex,
    pub base_class: ClassId,          // the "main" class lineage root is derivable
    pub active_class_slot: u8,        // 0 = main, 1..=3 = subclass slots
    pub classes: Vec<ClassProgress>,  // index = slot
    pub certifications: Vec<Certification>,
}
pub struct ClassProgress { pub class_id: ClassId, pub level: u8, pub exp: u64, pub sp: u64, pub skills: Vec<LearnedSkill> }
```

### 4.4 Proto additions (`packages/proto/nightfall/v1/game.proto`)

```proto
enum Race {
  RACE_UNSPECIFIED = 0;
  RACE_HUMAN = 1;
  RACE_ELF = 2;
  RACE_DARK_ELF = 3;
  RACE_ORC = 4;
  RACE_DWARF = 5;
  RACE_KAMAEL = 6;   // reserved, not selectable in MVP
}

enum Archetype { ARCHETYPE_UNSPECIFIED = 0; ARCHETYPE_FIGHTER = 1; ARCHETYPE_MYSTIC = 2; }

// Static class definition sent once (client caches by data_version).
message ClassInfo {
  uint32 class_id = 1;          // retail-compatible id
  string key = 2;
  string display_name = 3;
  Race race = 4;
  uint32 tier = 5;              // 0..3
  uint32 parent_class_id = 6;   // 0xFFFF when none
  uint32 min_level = 7;
  Archetype archetype = 8;
  BaseStats base_stats = 9;
  bool subclass_allowed = 10;
  repeated uint32 subclass_equivalents = 11;
}

message RaceInfo {
  Race race = 1;
  string display_name = 2;
  bool mystic_path = 3;
  uint32 walk_speed = 4;
  uint32 run_speed = 5;
  repeated uint32 base_class_ids = 6;
  repeated string passive_skill_keys = 7;
}

message ClassProgress {
  uint32 slot = 1;              // 0 main, 1..3 sub
  uint32 class_id = 2;
  uint32 level = 3;
  uint64 exp = 4;
  uint64 sp = 5;
}

// Extend the existing Character message (keep field numbers 1-6).
message Character {
  string id = 1;
  string name = 2;
  Race race = 3;
  uint32 level = 4;             // level of active class
  BaseStats stats = 5;
  Position position = 6;
  uint32 class_id = 7;          // active class
  uint32 base_class_id = 8;
  uint32 active_class_slot = 9;
  repeated ClassProgress classes = 10;
  Sex sex = 11;
}
enum Sex { SEX_UNSPECIFIED = 0; SEX_MALE = 1; SEX_FEMALE = 2; }

message ListClassesRequest {}
message ListClassesResponse { string data_version = 1; repeated RaceInfo races = 2; repeated ClassInfo classes = 3; }

message CreateCharacterRequest { string account_id = 1; string name = 2; Race race = 3; Sex sex = 4; uint32 base_class_id = 5; uint32 hair_style = 6; uint32 hair_color = 7; uint32 face = 8; }
message CreateCharacterResponse { Character character = 1; }

message ChangeClassRequest { string character_id = 1; uint32 target_class_id = 2; }
message ChangeClassResponse { Character character = 1; repeated uint32 granted_skill_ids = 2; }

message TransferOptionsRequest { string character_id = 1; }
message TransferOptionsResponse {
  message Option { uint32 class_id = 1; bool eligible = 2; repeated string unmet = 3; }
  repeated Option options = 1;
}

service GameService {
  rpc Ping(PingRequest) returns (PingResponse);
  rpc GetCharacter(GetCharacterRequest) returns (Character);
  rpc ListClasses(ListClassesRequest) returns (ListClassesResponse);
  rpc CreateCharacter(CreateCharacterRequest) returns (CreateCharacterResponse);
  rpc TransferOptions(TransferOptionsRequest) returns (TransferOptionsResponse);
  rpc ChangeClass(ChangeClassRequest) returns (ChangeClassResponse);
}
```

The subclass RPCs (`AddSubclass`, `SwitchClassSlot`, `CancelSubclass`) have specifications in the same style, but they are not implemented in MVP.

## 5. Interfaces

### 5.1 gRPC (request/response, outside the realtime stream)

| RPC | Server behaviour | Errors |
|---|---|---|
| `ListClasses` | Returns static data + `data_version` (hash of the TOML set). The client caches the data. | none |
| `CreateCharacter` | Validates the name (3-16 chars, unique, profanity list). Validates that the race is selectable, that `base_class_id` belongs to the race and tier 0, and that a mystic path exists for the race. Builds `CharacterSheet` from `ClassDef` + `RaceDef`. Gives the level-1 `auto_get` skills and racial passives. Puts the character at a random `start_point`. | `ALREADY_EXISTS` (name), `INVALID_ARGUMENT`, `RESOURCE_EXHAUSTED` (slots for each account, 7 as in L2) |
| `TransferOptions` | `registry.transfer_options(current, level)` plus the requirement evaluation. | `NOT_FOUND` |
| `ChangeClass` | Validates eligibility again on the server, consumes the token, applies 3.3, and stores the result. Emits `ClassChanged` on the realtime stream to the players near the character (visual change). | `FAILED_PRECONDITION` with the unmet list |

### 5.2 Server events (realtime stream, defined fully in Phase 3)

- `ClassChanged { character_id, class_id }` to all players who can see the character.
- `StatsRecomputed { max_hp, max_mp, max_cp, derived... }` to the owner after a transfer or level-up.
- `SkillsGranted { skill_ids }` to the owner.

### 5.3 What the client needs

The client uses the static `ListClassesResponse` to render the creation screen (race -> base class), the class-tree UI (ancestors/children), and the transfer dialogs (`TransferOptions`). The client does not need formulas, because the server sends the final derived stats.

## 6. Rust implementation notes

Module layout in `apps/api/src/`:

```
character/
  mod.rs          // pub use; CharacterService facade used by grpc.rs
  race.rs         // RaceId, RaceDef, RaceRegistry
  class.rs        // ClassId, ClassDef, Archetype, BaseStats, TransferRules
  template.rs     // GrowthTable, ClassRegistry (load + validate + queries)
  sheet.rs        // CharacterSheet, ClassProgress, derived stat recompute glue (calls stats:: from Phase 1)
  progression.rs  // level-up, exp/sp add, transfer(), grant_auto_skills()
  subclass.rs     // eligibility rules (pure functions), certification limits
  creation.rs     // CreateCharacter validation and sheet construction
  loader.rs       // TOML/CSV parsing (serde + toml + csv crates), data_version hashing
data/
  races/*.toml
  classes/*.toml
  growth/*.csv
```

Crates: `serde`, `toml`, `csv`, `thiserror` (typed errors mapped to `tonic::Status` in `grpc.rs`), `blake3` for `data_version`. All registries do not change after startup. The tonic service struct shares them as `Arc<StaticData>`. Hot reload is a GM command (Phase 9) that swaps the `Arc` behind an `ArcSwap`.

Concurrency: a class transfer changes a `CharacterSheet`. The zone actor that now simulates the character controls the sheet (Phase 0 architecture). The gRPC handler sends a `Command::ChangeClass` to that actor, and waits for a oneshot reply. Thus there is no lock on the sheet. The actor stores the sheet with write-behind persistence.

Validation at load (fail fast, before the server binds ports):

- Stat budget 170.
- The parent exists and is `tier - 1`.
- `min_level` matches the tier (1/20/40/76).
- The race is the same along the chain.
- `subclass_equivalents` is symmetric.
- The growth table has rows 1..cap.
- Each `start_point` is in the named zone (Phase 6 geodata).
- Each `passives.grant` resolves to a skill key (Phase 3 registry).

Thus the Phase 3 skill registry is a load-time dependency of the character module. This is intentional.

Build: `apps/api/build.rs` already compiles `game.proto` through `protox`. New messages do not need a build change. Generate the TypeScript client types again from the same proto (Phase 0 pipeline).

## 7. Client implications

- Character creation screen: race cards (5), then base class (fighter/mystic where available), sex, appearance. The preview uses the `BaseStats` from `ClassInfo`. Show the 170 budget as six bars.
- Class tree view: render from `ClassInfo.parent_class_id`. Highlight the current path. Show the ineligible classes in grey.
- Class master NPC dialog: calls `TransferOptions`, shows the unmet requirements as text, and sends `ChangeClass` when the player accepts.
- Phaser: sprite sheets for each race (5 races x 2 sexes). In MVP, the class does not change the body (L2 changes the appearance only through armor and 3rd-class marks).
- Movement speed from `RaceInfo` is for *prediction* only. The `StatsRecomputed` from the server is authoritative.

## 8. Open questions

1. Level cap for MVP (Phase 1 says 60): do we put the 2nd transfer gate at 40 as in L2? Or do we move it to 35, so that testers see it earlier?
2. Do we give Human an explicit passive, or does Human stay "no passive, widest tree"? The +5% XP is a placeholder.
3. Growth tables: copy the L2J values for all 31 second classes now (cheap), or only the vertical slice?
4. Class names: do we keep L2 names as `display_name` during development and change them before public release? Or do we use our own names from the first day? The legal review of names is not complete.
5. Do we keep `ClassId` retail-compatible after we add Kamael/custom classes (ids 58-87 and 119-122 are free; 137+ open)?
6. Transfer token economics (adena price for each tier) are part of Phase 5. A placeholder value is necessary.
7. With a level-cap-60 MVP, a subclass start level of 40 makes subclasses of no value. Postpone subclasses until the cap is >= 75, or define the start again as "start at cap - 35".

## 9. Sources

- L2J Server (High Five) game source, `Formulas.java`, `PcStat.java` (`getMaxLoad`), `CharStat.java`, `PlayerClass.java`, `ClassId.java`, `L2VillageMasterInstance.java`, `config/character.properties`: https://bitbucket.org/l2jserver/l2j-server-game/src/develop/
- L2J Server datapack (High Five): `data/stats/chars/classList.xml`, `data/stats/chars/baseStats/*.xml`, `data/stats/chars/pcCreationPoints.xml`, `data/skillTrees/classSkillTree.xml`, `data/skillTrees/subClassSkillTree.xml`, `data/stats/statBonus.xml`: https://bitbucket.org/l2jserver/l2j-server-datapack/src/develop/
- L2J C6 Interlude fork (for Interlude-era differences): https://github.com/Hl4p3x/L2JServer_C6_Interlude
- l2db.net class pages (base stats cross-check, Interlude/C4): https://www.l2db.net/interlude/classes/human-fighter, https://www.l2db.net/interlude/classes/elven-mystic, https://www.l2db.net/interlude/classes/dark-fighter, https://www.l2db.net/c4/classes/orc-mystic, https://l2db.net/c4/classes/bounty-hunter
- l2db.net races codex (racial descriptions): https://l2db.duckdns.org/en/interlude/races
- ludo.guide, First Class Transfer (quest names, Marks, NPCs): https://www.ludo.guide/guide/lineage-ii/main-walkthrough/first-class-transfer
- ludo.guide, subclass quests and dual class overview: https://www.ludo.guide/guide/lineage-ii/subclass-dual-class-system/subclass-quests
- Lineage 2 Wiki (fandom), Subclass and Dualclass pages (subclass restrictions, certification): https://lineage2.fandom.com/wiki/Dualclass
- lineage2wiki.org Hellbound patch notes (subclass certification skills at 65/70/75): https://lineage2wiki.org/hellbound/patch-notes/
- Lineage II Classic Chronicle 1.5 patch notes (Kamael absent in Classic, fighter-only Kamael later): https://www.lineage2.com/news/lineage-ii-classic-launch-patch-notes
- Lineage 2 Wiki (fandom), Kamael Soldier (Female): https://lineage2.fandom.com/wiki/Kamael_Soldier_(Female)
- StrategyWiki Lineage II classes overview (36 classes, transfer levels): https://strategywiki.org/wiki/Lineage_II/Classes
