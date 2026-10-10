# Phase 2: Race and class

Status: reference design. The [executable plan](../plans/phase-2-race-and-class.md) and [verification outcome](../plans/phase-2-race-and-class-outcome.md) record the current implementation and open acceptance gates. The reference's suggested Nightfall cap of 60 is superseded by the implemented Phase 1 cap of 85; playable transfers stop at tier 2 (levels 20/40), with tier 3 metadata only. Depends on Phase 1; feeds Phases 3 and 4.

## 1. Purpose and scope

This phase delivers everything needed to create a character with a race and a class, level it, and move it through the class tree:

- Race definitions: base stat arrays, movement speed, collision, racial traits, starting zone.
- Class definitions: the class tree (base class, first/second/third transfer), per-class growth tables (HP/MP/CP by level), class-restricted skill trees, equipment proficiency unlocks.
- Class transfer rules: level gates, cost/quest hooks, what changes on transfer.
- Subclass rules: data model and eligibility rules (implementation deferred, see section 3).
- Data-driven definition format (TOML), Rust data model under `apps/api/src/character/`, and proto additions.

Explicitly excluded: the stat formulas themselves (Phase 1), the skills a class learns (Phase 3 defines skill data; this phase only defines the *tree* that says who learns what at which level and SP cost), item grades (Phase 4), quest scripting for transfers (Phase 6), dual class and Awakening (Goddess of Destruction era; out of reference scope).

## 2. Reference: how Lineage 2 does it

All numbers below are Interlude-to-High Five era unless stated otherwise. The primary data source is the L2J High Five datapack (`data/stats/chars/`), which is a faithful dump of the client/server class templates, cross-checked against l2db.net class pages.

### 2.1 Races

Five classic races plus Kamael (added in Chaotic Throne: The Kamael, 2007). Every race except Dwarf and Kamael has two starting paths: Fighter and Mystic. Dwarves are fighter-only; Kamael are fighter-only and gender-split (male and female Soldiers have different trees).

#### Base stat arrays per starting class

Source: L2J `data/stats/chars/baseStats/*.xml` (`baseSTR`...`baseMEN`), matching l2db.net C1/C4/Interlude class pages. These are level-1 values and never change with level in classic L2; only equipment, buffs, tattoos (henna) and later dyes modify them.

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

Every array sums to 170. That is the design budget: a race/class is a redistribution of 170 points across six stats. Nightfall should keep the fixed-budget rule because it makes balance arguments tractable.

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

All templates share: `basePAtk` 4 (3 for Human Mystic), `baseMAtk` 6, `baseCritRate` 4, `basePAtkSpd` 300, `baseAtkRange` 20, `baseRndDam` 10, per-slot base P.Def (chest 31 / legs 18 / head 12 / feet 7 / gloves 8 / underwear 3 / cloak 1 for fighters; chest 15 / legs 8 for mystics) and base M.Def (ears 9 each, rings 5 each, necklace 13). The slot P.Def is the "naked" defence that gets replaced when armor is equipped (L2J `FuncPDefMod` subtracts the slot base when the slot is filled).

#### Racial traits

L2 has almost no explicit racial passive skills in the classic era. Racial identity is produced by the stat array feeding the Phase 1 derived formulas, plus a few race-specific skill trees and zones:

| Race | Where the trait actually comes from | Effect in play |
|---|---|---|
| Human | Balanced array; widest class tree (five fighter branches incl. two tanks, five mystic branches). | Jack of all trades; Human mystics have the best MEN/WIT balance for healers. |
| Elf | DEX 35 (highest with female Kamael), run speed 125 (highest), breath 150, safe fall 350. Accuracy and evasion are `sqrt(DEX)*6 + level`; P.Atk speed is `base * DEX_bonus`; crit rate is `baseCrit * DEX_bonus * 10`. WIT 23 on mystics gives fastest casting (`mAtkSpd = base * WIT_bonus`). Also the only race that gets HP/MP regen bonus inside the Mother Tree zone (L2J `ZoneId.MOTHER_TREE`). | Fastest attack/cast speed, highest evasion, low damage (STR 36, INT 37). |
| Dark Elf | STR 41 (highest fighter STR), DEX 34, INT 44 (highest INT), but CON 32 and MEN 37. STR drives P.Atk (`base * STR_bonus * levelMod`) and physical-skill crit chance (`STR_bonus * skillCritChance`); DEX drives crit rate. | Highest burst crit and nuke damage; lowest HP/M.Def ("glass cannon"). |
| Orc | CON 47 (highest; max HP = `base * CON_bonus`, HP regen scales with CON), MEN 42 on mystics (M.Def, debuff resistance). DEX 26 (lowest). Slow walk (70). Orc mystics are the only buffer line that uses fighter-style melee (Shaman -> Overlord/Warcryer). | Most HP and regen, slowest, worst evasion/crit. |
| Dwarf | CON 45, DEX 29, WIT 10. Weight limit is `floor(CON_bonus * 69000)` (L2J `PcStat.getMaxLoad`), so Dwarves carry the most. Exclusive skills: Spoil (id 254), Sweeper (42), Create Item / Crystallize (172 / 248), Dwarven Craft; also exclusive access to crafting recipes at Create Item levels. | The economy race: spoil loot, craft, carry. Fighter only, two branches. |
| Kamael | Light armor only (no heavy/robe mastery), Soul system (absorb souls on hit, consume for skills), racial transformations (wings) at 3rd class. Female Soldier DEX 35; male STR 41. Safe fall 500. No mystic path. Cannot subclass to/from other races. | Hybrid fighter/caster with unique resource; excluded from Nightfall MVP (see section 3). |

#### Starting zones

L2J `data/stats/chars/pcCreationPoints.xml` keys spawn point groups by class id. World coordinates (L2 units; ~16 units per metre):

| Race / path | Village | Representative spawn (x, y, z) |
|---|---|---|
| Human Fighter | Talking Island Village | (-71338, 258271, -3104) |
| Human Mystic | Talking Island, Einhasad temple area | (-90875, 248162, -3570) |
| Elf (both) | Elven Village (Shadow of the Mother Tree) | (46045, 41251, -3440) |
| Dark Elf (both) | Dark Elf Village | (28295, 11063, -4224) |
| Orc (both) | Orc Village (Immortal Plateau) | (-56733, -113459, -690) |
| Dwarf | Dwarven Village | (108644, -173947, -400) |
| Kamael (both) | Isle of Souls, Kamael Village | (-125607, 38452, 1152) |

### 2.2 The class tree

Levels: base class at 1, first transfer at 20, second at 40, third at 76. Class ids are the retail ids used by the client and by L2J `classList.xml`; 3rd-class ids start at 88 because ids 58-87 were reserved. Ids 119-122 are internal NPC-class placeholders.

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

Totals: 9 (or 11 with Kamael) base classes, 18 (22) first classes, 31 (36) second classes, 31 (36) third classes. Note the oddities L2J encodes: Elemental Master's parent is Elven Wizard (26) not Elemental Summoner (28) in `classList.xml`, which is a data bug to avoid copying; Bladedancer's parent is Shillien Knight (33) not Palus Knight (32), also a data bug (Bladedancer is a 2nd class from Palus Knight). Inspector is only reachable as a Kamael's third subclass after two other Kamael subclasses reach 75.

### 2.3 What a class transfer changes

1. **Growth template.** Each class id has its own `baseStats/<Class>.xml` with a `lvlUpgainData` table of HP/MP/CP per level. The STR..MEN array does *not* change on transfer (Gladiator and Duelist both carry Human Fighter's 40/30/43/21/11/25). What differs is the HP/MP/CP curve. Example (Gladiator = Duelist file, both identical, so the 3rd transfer does not change growth):

   | Level | HP | MP | CP |
   |---|---|---|---|
   | 1 | 80 | 30 | 72 |
   | 20 | 327 | 144 | 294.3 |
   | 40 | 1044 | 359.1 | 939.6 |
   | 76 | 3061.8 | 1155.6 | 2755.62 |
   | 85 | 3643.2 | 1385.1 | 3278.88 |

   Human Fighter level 1 CP is 32 vs Gladiator's 72: L2J's file for the 2nd class simply restates the full 1..85 curve and the server swaps curves on transfer (max HP/CP jump immediately).

2. **Skill tree.** `skillTrees/classSkillTree.xml` has one `<skillTree classId=N>` per class. A class inherits every ancestor's tree (L2J walks `parentClassId`). Each entry: `skillId`, `skillLvl`, `getLevel` (character level gate), `levelUpSp` (SP cost), optional `autoGet` (free on level-up), `learnedByNpc`, optional item requirements (spellbooks). Human Fighter examples: Power Strike 1-3 at level 5 for 50 SP each, Weapon Mastery 1 at level 5 for 160 SP, Power Strike 4-6 at level 10 for 370 SP each. Transfer unlocks the new class's tree immediately.

3. **Equipment proficiency.** Expertise is a level-gated auto skill (id 239), not class-gated: D at 20, C at 40, B at 52, A at 61, S at 76, S80 at 80, S84 at 84. Using a grade above expertise applies a penalty (Phase 4). Class *does* gate weapon/armor *masteries* (passive bonuses) and which weapons skills require (`<using kind="SWORD,BLUNT"/>` conditions on skills). Kamael additionally cannot wear heavy/robe at all.

4. **Identity.** Class name, title eligibility, Olympiad bracket, and (at 3rd class) the "class mark" visual. Subclass and certification eligibility (2.4) also depend on the class level.

#### Class transfer gating (quests)

- **1st transfer (20).** Three quests per branch in the classic game, startable at 18-19 and completed at 20, each granting a "Mark" exchanged with a class master. E.g. Human Fighter -> Warrior: "Trial of the Warrior" (Master Auron, Mark of the Warrior, finalized by Grand Master Bitz). Knight: "Trial of the Human Knight" (Grand Master Ramos). Rogue: "Trial of the Human Rogue" (Master Terry).
- **2nd transfer (40).** Three quests, each giving a Mark (e.g. Mark of Challenger, Mark of Duelist, Mark of Trust for Gladiator), all required by the class master. Startable at 35-39.
- **3rd transfer (76).** A "Saga of the <class>" quest chain per 3rd class (Interlude+), partly instanced, awarding the 3rd class and a Hero-style skill.
- **Classic / private-server shortcut.** Later official Classic servers and nearly all private servers let you buy the transfer for adena or a token; L2J exposes `SubclassWithoutQuests`-style toggles and sells Marks via multisell. The L2J village master checks for subclass eligibility only `isNoble() || Q234 Fate's Whisper || Q235 Mimir's Elixir`.

### 2.4 Subclass system

L2J defaults (`character.properties`): `MaxSubclass = 3`, `BaseSubclassLevel = 40`, `MaxSubclassLevel = 80` (75 in Interlude; 80 from Gracia Final/High Five), `SubclassWithoutQuests = False`, `SubclassEverywhere = False`.

Rules encoded in L2J `PlayerClass.getAvailableSubclasses` and `L2VillageMasterInstance`:

1. The main class must have completed the 2nd transfer and reached level 75, and have completed Fate's Whisper (Q234) or Mimir's Elixir (Q235) or be Noble.
2. A new subclass starts at level 40 as a *2nd-class* profession (you pick e.g. "Gladiator", not "Warrior"), with its own XP/SP, skills, and level. Switching is done at a village master and swaps the whole skill set and stats; buffs are removed on switch unless flagged `isStayOnSubclassChange`.
3. Never allowed as subclass: Overlord, Warsmith.
4. An Elf main cannot take any Dark Elf class and vice versa. Non-Kamael cannot take Kamael classes; Kamael can *only* take Kamael classes (and gender-matched Soul Breaker). Inspector requires two other Kamael subclasses at 75.
5. You cannot take the "equivalent" class of your own: the five equivalence sets are {Dark Avenger, Paladin, Temple Knight, Shillien Knight}, {Treasure Hunter, Abyss Walker, Plains Walker}, {Hawkeye, Silver Ranger, Phantom Ranger}, {Warlock, Elemental Summoner, Phantom Summoner}, {Sorcerer, Spellsinger, Spellhowler}. You also cannot take your own base class, or a class already held as another subclass (checked via `equalsOrChildOf`).
6. Certification ("stacking") skills (`skillTrees/subClassSkillTree.xml`): raising a subclass to 65 / 70 / 75 / 80 grants a certificate item; certificates are spent on main-class passives: Emergent Ability (Attack / Defense / Casting, at 65), Master Ability (at 70), transformation and class-group skills (at 75/80). Max 4 certified skills per subclass, 12 total with three subclasses. Certificates can be cancelled (refunded) for adena at the village master.
7. Dual class (Goddess of Destruction, 2013+) replaces one subclass with a second "main" that levels to cap and can Awaken. Out of reference scope; the data model below leaves room for it (`is_dual`).

## 3. Design decisions for Nightfall

### 3.1 MVP scope

| Decision | Choice | Alternatives considered |
|---|---|---|
| Races | 5: Human, Elf, Dark Elf, Orc, Dwarf. Kamael excluded. | 3 races (Human/Elf/Dark Elf) is cheaper for art but racial stat identity is data-only, so all five cost the same server-side. Kamael requires gender-split trees, light-only armor rules and the Soul resource, which touch combat; defer to post-MVP. |
| Base classes | 9 (all L2 base classes). | Fighter-only start was considered; mystics are needed to validate the magic formulas in Phase 3. |
| 1st transfer | All 18 first classes at level 20, data-only. | Fewer would make the tree asymmetric across races. |
| 2nd transfer | Data defined for all 31, but only skill trees for a vertical slice (Gladiator, Paladin, Treasure Hunter, Hawkeye, Sorcerer, Bishop, Prophet, Spellsinger, Shillien Knight, Destroyer, Warcryer, Bounty Hunter) are populated in MVP. | Shipping all 31 skill trees is a content task for Phase 3/6. The class *definitions* are cheap; the *skills* are not. |
| 3rd transfer | Defined in data, not reachable in MVP (level cap 60 for MVP; see Phase 1). | |
| Subclass | Data model and eligibility rules implemented; UI and village master flow deferred. | Avoids a schema migration later. |
| Transfer gating | Level gate + a `ClassTransferToken` item purchasable from a class master NPC (Classic-style). Quest hooks are a `requirements` list in data so Phase 6 can add quests without schema change. | Full quest chains (L2 retail) are a Phase 6 content item. |
| Stat budget | Fixed 170-point budget enforced by the loader. | |
| Growth curves | Per-class HP/MP/CP tables copied from L2J as the initial tuning set, stored per class with inheritance: a class without its own table uses its parent's. | Formula-based growth (`hp = a + b*lvl + c*lvl^2`) was considered; tables are easier to tune per class and match L2 exactly. |
| Class ids | Reuse retail ids (0..57, 88..118) so L2 reference data and community knowledge map 1:1. Kamael ids 123-136 reserved. | Dense Nightfall-only ids would be cleaner but break every cross-reference. |
| Name uniqueness | Class names are Nightfall's own (we are L2-inspired, not L2); the TOML carries `display_name` plus `l2_ref` for traceability. | |

### 3.2 Racial traits in Nightfall

Keep L2's approach: traits are mostly the stat array plus movement/collision, with a small list of explicit racial passives so the differences are legible to players:

| Race | Explicit passive (granted at level 1) | Notes |
|---|---|---|
| Human | `Adaptable`: +5% XP/SP gain | Replaces L2's "widest tree" advantage with something visible. |
| Elf | `Forest Step`: +3 run speed, +3% evasion; `Mother Tree Attunement`: +50% HP/MP regen in Elven forest zones | Mirrors L2 numbers (run 125 vs 115) and the Mother Tree zone. |
| Dark Elf | `Shadow Precision`: +5% critical damage | L2 DE has no explicit passive; added for legibility. |
| Orc | `Iron Constitution`: +10% HP regen, +5 Shock (stun) resistance | |
| Dwarf | `Pack Mule`: +20% weight limit; `Keen Eye`: Spoil/Sweep skill access; `Artisan Hands`: crafting skill access | Weight limit base formula stays CON-driven as in Phase 1. |

### 3.3 Transfer mechanics

- A transfer is allowed when `level >= class.min_level`, `current_class == class.parent`, and all `requirements` are met (MVP: possess `ClassTransferToken` of tier 1/2). Transfer consumes the token.
- On transfer: set `class_id`, recompute max HP/MP/CP from the new growth table, grant `auto_get` skills from the new tree up to current level, keep current HP/MP percentages, broadcast `ClassChanged`.
- No downgrades. Transfers are irreversible (as in L2).

### 3.4 Subclass (post-MVP, data reserved)

Implement L2 rules 1-5 from 2.4 as a pure function `subclass::eligible(main: &CharacterSheet, candidate: ClassId) -> Result<(), SubclassDenied>`. Subclass start level 40, cap = `main_level_cap - 5`. Certification skills are modelled as a `certifications: Vec<Certification>` on the character; limits (4 per sub, 12 total) are constants in data.

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

Growth CSV example (`data/growth/gladiator.csv`): `level,hp,mp,cp` with rows 1..85, values from L2J as the initial set.

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

Character-side state (persisted; see Phase 0 for storage):

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

Subclass RPCs (`AddSubclass`, `SwitchClassSlot`, `CancelSubclass`) are specified in the same style but not implemented in MVP.

## 5. Interfaces

### 5.1 gRPC (request/response, outside the realtime stream)

| RPC | Server behaviour | Errors |
|---|---|---|
| `ListClasses` | Returns static data + `data_version` (hash of the TOML set). Client caches. | none |
| `CreateCharacter` | Validates name (3-16 chars, unique, profanity list), race selectable, `base_class_id` belongs to race and tier 0, mystic path exists for race. Builds `CharacterSheet` from `ClassDef` + `RaceDef`, grants level-1 `auto_get` skills and racial passives, places at a random `start_point`. | `ALREADY_EXISTS` (name), `INVALID_ARGUMENT`, `RESOURCE_EXHAUSTED` (slots per account, 7 as in L2) |
| `TransferOptions` | `registry.transfer_options(current, level)` plus requirement evaluation. | `NOT_FOUND` |
| `ChangeClass` | Re-validates eligibility server-side, consumes token, applies 3.3, persists, emits `ClassChanged` on the realtime stream to nearby players (visual change). | `FAILED_PRECONDITION` with unmet list |

### 5.2 Server events (realtime stream, defined fully in Phase 3)

- `ClassChanged { character_id, class_id }` to everyone who can see the character.
- `StatsRecomputed { max_hp, max_mp, max_cp, derived... }` to the owner after transfer or level-up.
- `SkillsGranted { skill_ids }` to the owner.

### 5.3 What the client needs

Static `ListClassesResponse` to render the creation screen (race -> base class), the class-tree UI (ancestors/children), and transfer dialogs (`TransferOptions`). Nothing about formulas; the server sends final derived stats.

## 6. Rust implementation notes

Module layout under `apps/api/src/`:

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

Crates: `serde`, `toml`, `csv`, `thiserror` (typed errors mapped to `tonic::Status` in `grpc.rs`), `blake3` for `data_version`. All registries are immutable after startup and shared as `Arc<StaticData>` through tonic's service struct; hot reload is a GM command (Phase 9) that swaps the `Arc` behind an `ArcSwap`.

Concurrency: class transfer mutates a `CharacterSheet`. Sheets are owned by the zone actor that currently simulates the character (Phase 0 architecture); the gRPC handler sends a `Command::ChangeClass` to that actor and awaits a oneshot reply, so there is no lock on the sheet. Persistence is write-behind from the actor.

Validation at load (fail fast, before binding ports): stat budget 170; parent exists and is `tier - 1`; `min_level` matches tier (1/20/40/76); race consistent along the chain; `subclass_equivalents` symmetric; growth table has rows 1..cap; every `start_point` is inside the named zone (Phase 6 geodata); every `passives.grant` resolves to a skill key (Phase 3 registry) — this makes the Phase 3 skill registry a load-time dependency of the character module, which is intended.

Build: `apps/api/build.rs` already compiles `game.proto` via `protox`; adding messages requires no build change. Regenerate the TypeScript client types from the same proto (Phase 0 pipeline).

## 7. Client implications

- Character creation screen: race cards (5), then base class (fighter/mystic where available), sex, appearance. Preview uses the `BaseStats` from `ClassInfo`; show the 170 budget as six bars.
- Class tree view: render from `ClassInfo.parent_class_id`; highlight current path; grey-out ineligible.
- Class master NPC dialog: calls `TransferOptions`, shows unmet requirements as text, confirms with `ChangeClass`.
- Phaser: per-race sprite sheets (5 races x 2 sexes); class does not change the body in MVP (L2 only changes appearance via armor and 3rd-class marks).
- Movement speed from `RaceInfo` is for *prediction* only; the server's `StatsRecomputed` is authoritative.

## 8. Open questions

1. Level cap for MVP (Phase 1 says 60): do we gate the 2nd transfer at 40 as in L2 or pull it to 35 so testers see it sooner?
2. Should Human get an explicit passive at all, or stay "no passive, widest tree"? The +5% XP is a placeholder.
3. Growth tables: copy L2J values for all 31 second classes now (cheap) or only the vertical slice?
4. Class names: do we keep L2 names as `display_name` during development and rename before public release, or name from day one? Legal review of names pending.
5. Whether `ClassId` should stay retail-compatible once Kamael/custom classes are added (ids 58-87 and 119-122 are free; 137+ open).
6. Transfer token economics (adena price per tier) belong to Phase 5; needs a placeholder value.
7. Subclass start level 40 with a level-cap-60 MVP would make subclasses meaningless; defer until cap >= 75 or redefine as "start at cap - 35".

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
