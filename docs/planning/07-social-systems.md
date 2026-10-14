# Phase 7: Social Systems

This phase is about the party, friends and chat, mentoring, clans, alliances and clan wars, castle sieges, and the Olympiad.
It depends on Phases 0-6 (accounts and persistence, combat and PvP flags, items, drops, world
zones and NPC AI).

---

## 1. Purpose and scope

**Delivers**

- Party: up to 9 members, leader, loot modes, XP/SP share with a size bonus and level-gap
  rules, party matching room, command channel for raids.
- Friends list, block list, chat channels with range/flood/ban rules.
- Mentor/mentee bond with rewards.
- Clans: levels 0-11 with costs, member caps, sub-units (Academy, Royal Guards, Order of
  Knights), reputation ledger, clan skills, crests, leader transfer, penalties, clan halls.
- Alliances (3 clans) and clan wars (declaration, mutual war, kill count, reputation).
- Castle sieges: registration, two-week schedule, HQ flags, control towers, gates, siege
  weapons, artifact engraving, ownership benefits. A short description of fortresses.
- Grand Olympiad: eligibility, points, match types, weekly/monthly cycle, hero selection and
  rewards. Noblesse is the gate.

**Excludes**

- PvP damage/karma formulas (Phase 3).
- The internal parts of the clan warehouse item storage (Phase 4).
- Manor/tax economics, other than the rates (Phase 5).
- Siege NPC AI (Phase 6).
- UI layout (Phase 8).
- GM moderation tools (Phase 9).

---

## 2. Reference: how Lineage 2 does it

### 2.1 Party

- **Size and leadership.** Up to 9 members. The player who sends the invitation is the leader. The leader can give the leadership
  to a different member (`/changepartyleader`). If the leader leaves, the leadership goes to the next member. The leader
  invites and dismisses members, sets the loot mode (only when the party is formed) and opens a command
  channel.
- **Loot distribution modes** (L2J `L2Party.ITEM_*`): *Finders Keepers* (`ITEM_LOOTER`),
  *Random* (`ITEM_RANDOM`), *Random including spoil* (`ITEM_RANDOM_SPOIL`), *By Turn*
  (`ITEM_ORDER`), *By Turn including spoil* (`ITEM_ORDER_SPOIL`). Adena is always divided
  equally between the members in range. Herbs are always finders-keepers. Spoil goes to the spoiler,
  but not when an "including spoil" mode is active.
- **XP/SP sharing.** On a kill, the members in party range (L2J `ALT_PARTY_RANGE = 1600`
  units) that are alive are candidates. Each valid member gets
  `reward * (member_level / sum(valid_member_levels)) * party_bonus`. The L2J `getValidMembers`
  has four cut-off modes:
  - *level* (in `PARTY_XP_CUTOFF_LEVEL = 20` levels of the highest member).
  - *percentage* (member `level^2 / sum(level^2) >= 3%`).
  - *auto* (gap by party size).
  - *highfive* (the retail High Five table).

  These are the retail bonus tables by member count
  ([High Five notes](https://lineage2wiki.org/hi-five/patch-notes/), [C3 notes](https://lineage2wiki.org/c3/patch-notes/)):

| Members | C3-Interlude bonus | High Five bonus |
|---|---|---|
| 1 | +0% | +0% |
| 2 | +30% | +10% |
| 3 | +39% | +20% |
| 4 | +50% | +30% |
| 5 | +54% | +40% |
| 6 | +58% | +50% |
| 7 | +63% | +100% |
| 8 | +67% | +110% |
| 9 | +72% | +120% |

  The High Five level-gap rule: with a gap of 0-9, all members get full XP. With 10-14, the lowest member gets 30%. With 15+, the lowest member
  gets 0. Essence/Classic servers use 0-5 / 6-9 / 10+ at 100% / 30% / 0%
  ([Steam thread](https://steamcommunity.com/app/373700/discussions/0/485624149158226564)).
  The mob-vs-party level penalty uses the *highest* party level (`partyLvl`). Thus, if a level 75
  player brings a level 20 player, the level 75 player gets normal XP and the level 20 player gets nothing.
- **Party matching.** A leader opens a *party room* (name, level range, loot mode, max 12
  members in the room, C4). The rooms are listed for each region. Players put a flag on themselves on a waiting list, and other players can
  search for them by class ([C4 notes](https://lineage2wiki.org/c4/patch-notes/). High Five added
  the search by name/class).
- **Command channel.** A party leader can open one with the *Clan Imperium* skill (clan
  level 5 passive, 0 reputation). From Interlude, a leader can also open one with a *Strategy Guide* item, which the action consumes
  ([Interlude notes](https://lineage2wiki.org/interlude/patch-notes/),
  [pmfun clan skills](https://lineage.pmfun.com/list/skillclan)). The channel leader can see the
  member parties and dismiss parties. When the channel is sufficiently large (18+ raid, 36+ boss, 225+ Antharas in Interlude),
  the channel leader gets the raid loot priority. Chat prefix `` ` ``.

### 2.2 Friends, block list, chat

- **Friends**: `/friendinvite`, mutual acceptance, online status, friend chat (`L2FRIEND`).
  **Block**: `/block name` stops whispers, trades and invitations from that player. High Five shows
  a notice when a player whispers to a player who blocks them.
- **Chat channels** (L2J `Say2` ids and client prefixes):
  - General `ALL` (0, no prefix, ~1250-unit radius).
  - Shout `SHOUT` (1, `!`, all the region). The L2J "region" mode sends to the
    region of the player and to the neighbor regions. The "global" mode is a server option.
  - Whisper `TELL` (2, `"name`).
  - Party `PARTY` (3, `#`).
  - Clan `CLAN` (4, `@`).
  - Trade `TRADE` (8, `+`, the same range as shout).
  - Alliance `ALLIANCE` (9, `$`).
  - Party room `PARTYMATCH_ROOM` (14).
  - Command channel leader `PARTYROOM_COMMANDER` (15, yellow).
  - Command channel all `PARTYROOM_ALL` (16, `` ` ``).
  - Hero `HERO_VOICE` (17, `%`, all the server, 10 s cooldown
    [legacy-lineage2](https://legacy-lineage2.com/Knowledge/hero.html)).
  - The system types `GM`, `PETITION_*`, `ANNOUNCEMENT`, `BOAT`, `BATTLEFIELD`, `MPCC_ROOM`,
    `NPC_ALL/SHOUT`.
- **Flood and bans.** The L2J `FloodProtector` limits `GlobalChat`/`Shout` to approximately one
  message each 2 s. On the client side, a player cannot send the same text again immediately. GMs apply
  `CHAT_BAN` punishments (timed or permanent) that reject all `Say2` messages. Retail Classic added a
  level gate (general/shout chat from level 20) to decrease the effect of spam bots. Interlude did not have a level gate.

### 2.3 Mentor and mentee (Goddess of Destruction+)

Mentors are level 85+ (awakened). Mentees are 1-84. The pair gets *Mentor's Guidance* buffs
while the two players are online (P./M. Atk/Def +5%, XP/SP bonuses in event variants). The mentee gets
*Mentee's Mark* currency when they go up in level. The mentor gets Mentor Points, and a *Mentor
Certificate* when the mentee graduates at 85/86. The two players spend marks at a Mentoring Manager
NPC for enchant scrolls, crystals and consumables. If a player stops the bond early, the player loses the rewards.
The player also gets a cooldown before a new pair (community figures: 2 days for the mentor, 5 for the mentee).
Later, retail removed Diplomas/Certificates and replaced them with a graduation box
([ludo.guide](https://www.ludo.guide/guide/lineage-ii/community-clan-benefits/mentoring-system),
[NCWest dev tracker](https://devtrackers.gg/lineageii/p/4d1c7012-mentor-mentee)).

### 2.4 Clan system

**Creation**: character level 10+, 16-character alphanumeric name, and not in the 10 days after the player
dissolved a clan ([l2scroll Classic](https://www.l2scroll.com/2015/10/clan-in-classic.html)).

**Levels.** The leader buys levels 1-5 with SP + adena/items. Levels 6+ need reputation and
member counts. These are the Interlude values ([legacy-lineage2](https://legacy-lineage2.com/Knowledge/clans.html),
[C5 notes](https://lineage2wiki.org/c5/patch-notes/), High Five changes
[lineage2h5](http://lineage2h5.blogspot.com/p/clan-adjustments.html)):

| Level | Cost (Interlude) | Reputation | Min members | Main-unit cap | Unlocks |
|---|---|---|---|---|---|
| 0 | create | - | 1 | 10 | clan chat |
| 1 | 30,000 SP + 650,000 adena | - | 1 | 15 | warehouse (deposit) |
| 2 | 150,000 SP + 2,500,000 adena | - | 1 | 20 | clan hall bids, notice board |
| 3 | 500,000 SP + Blood Mark (quest) | - | 1 | 30 | crest, titles, wars, siege registration (C1: level 4) |
| 4 | 1,400,000 SP + Alliance Manifesto (quest) | - | 1 | 40 | siegeable clan halls |
| 5 | 3,500,000 SP + Seal of Aspiration (quest) | - | 1 | 40 | alliance, Academy, clan skills, Clan Imperium |
| 6 | - | 10,000 (H5: 5,000) | 30 | 40 + 2 Royal Guards x 20 + Academy 20 = 100 | Royal Guards |
| 7 | - | 20,000 (H5: 10,000) | 80 (H5: 50) | +2 Knights x 10 = 120 | Order of Knights |
| 8 | - | 40,000-50,000 (H5: 20,000) | 120 (H5: 80) | +2 Knights x 10 = 140 | more skills |
| 9 | 150 Blood Oaths | 40,000 | 120-140 | 170 (guards 25, knights 15) | Gracia |
| 10 | 5 Blood Pledges | 40,000 | 140 | 200 (guards 30, knights 20) | Gracia |
| 11 | territory ownership | 75,000 | 170 | 220 (knights 25) | Gracia Final |

Classic servers use different costs (L1 1,000 SP + 150k; L2 15,000 SP + 300k; L3 100,000 SP +
100 Proof of Blood; L4 1M SP + 5,000; L5 5M SP + 10,000) ([l2scroll](https://www.l2scroll.com/2015/10/clan-in-classic.html)).

**Units** (sub-pledges).

- *Academy*: level 5+, 20 members. It is for characters below level 40 without a
  second class. When a member graduates at the second class transfer, the clan gets reputation. The graduate gets a
  circlet and no penalty to join again.
- *Royal Guards*: level 6+, two units, 20 members each. The leader appoints a
  member to lead each unit.
- *Order of Knights*: level 7+, two units; level 8+, four units. Each unit is
  attached to a Royal Guard, with 10 members each.

Unit captains get a title and rank. Privileges are a
bitmask for each rank (invite, dismiss, warehouse withdraw, crest, hall functions, siege
registration, war declaration).

**Reputation.** Only clans of level 3+ (C5: 5+) get reputation. The L2J `Clan.properties` defaults
agree with retail:

- Take a castle +1500, defend a castle +750, lose a castle -3000. Take a fortress +200.
- A member becomes Hero +1000. Academy graduate +190..650 (more if they graduate earlier).
- Festival of Darkness win +200.
- A mutual-war or siege kill gives +1 to the clan of the killer and -1 to the clan of the
  victim. If the clan of the victim is at 0, it does not lose reputation.
- Blood Alliance item +500, Blood Oath +200, Knight's
  Epaulette +20.
- Classic also gives +2..25 for each member level-up, and -500 for a war surrender
  ([wiki.l2db.club](https://wiki.l2db.club/classic/Clans%20-%20Clan%20Reputation.html),
  [C5 notes](https://lineage2wiki.org/c5/patch-notes/)).

Clans spend reputation on level-ups (the table above), clan
skills and items that have a reputation price. If the reputation is positive, the clan name shows in blue. If it is negative, it shows
in red.

**Clan skills** (passives for each member of the necessary rank; reputation costs from
[pmfun](https://lineage.pmfun.com/list/skillclan)):

| Clan level | Skill (level 1) | Reputation |
|---|---|---|
| 5 | Clan Imperium | 0 |
| 5 | Clan Lifeblood (HP), Clan Magic Protection (M.Def), Clan Vitality (CP) | 1,500 each |
| 6 | Clan Shield Boost, Clan Spirituality | 2,100 |
| 6 | Clan Morale | 2,600 |
| 6 | Clan Aegis, Clan Might | 3,000 |
| 7 | Clan Cyclonic/Magmatic Resistance, Fortitude, Freedom, Vigilance, Withstand-Attack | 5,100 |
| 7 | Clan Guidance | 5,600 |
| 7 | Clan Luck | 6,900 |
| 8 | Clan March | 11,400 |
| 8 | Clan Clarity, Empowerment, Essence | 11,700 |
| 8 | Clan Agility | 12,000 |
| 11 | Lifeblood/Magic Protection/Vitality level 3 | 13,200 |

In C5-Interlude, when a clan learns a skill, it also uses raid-boss items (Blood Crystals and other items).

**Clan halls.** There are auction halls in towns. The C3 grades are:

- A in Aden: 50M base / 1.5M deposit / 1M weekly rent.
- B in Gludio/Gludin: 20M / 1M / 500k.
- C in Dion: 8M / 500k / 200k.

These are the auction rules:

- A clan of level 2+ bids from the clan warehouse. Each clan can make one bid.
- If a clan cancels a bid, it gets 90% back.
- The auction runs for one week (two weeks in older versions).
- The rent is taken each week. If the clan does not pay, it must leave the hall
  ([C3 notes](https://lineage2wiki.org/c3/patch-notes/),
  [NCWest dev tracker](https://devtrackers.gg/lineageii/p/ec06cab7-clan-hall-auctions)).

The clan buys functions with adena: HP/MP regen, XP restore on death, teleport list, buffs by tier,
item creation. Siegeable halls (Bandit Stronghold, Devastated Castle, Fortress of Resistance,
Fortress of the Dead, Rainbow Springs, Wild Beast Reserve) need clan level 4. A clan cannot
have one of these halls and a castle at the same time.

**Crests and titles.** The clan crest is a 16x12 px 256-color bitmap. The alliance crest is 8x12. The castle
insignia is 64x64 (C4). The leader sets the member titles.

**Leader transfer, leaving, dismissal, dissolution.**

- Transfer: the leader requests it at an NPC. It
  applies at the next maintenance (weekly in retail).
- Leave: the member cannot join a different clan for 24 h
  (C5 decreased this from 5 days).
- Dismissal: the member waits 24 h, and the clan cannot recruit for
  24 h.
- Dissolution: a 7-day countdown (the leader can cancel it). The leader loses XP as for one death. The clan skills are
  removed, the warehouse is lost, and there is a 10-day cooldown before a new clan.

The L2J config names are
`ALT_CLAN_JOIN_DAYS = 1`, `ALT_CLAN_CREATE_DAYS = 10`, `ALT_CLAN_DISSOLVE_DAYS = 7`,
`ALT_ACCEPT_CLAN_DAYS_WHEN_DISMISSED = 1` ([legacy-lineage2](https://legacy-lineage2.com/Knowledge/clans.html),
[l2scroll](https://www.l2scroll.com/2015/10/clan-in-classic.html)).

### 2.5 Alliances and clan wars

**Alliance**: the leader of a clan of level 5+ makes an alliance at an NPC. From C5/Interlude, the maximum is **3 clans**, which includes the
founder. C1 increased the cap to 5, and some sources give larger
early limits. No version after C5 permits 12. These are the rules:

- A clan can be in one alliance at a time.
- Clans that are at war with each other cannot make an alliance.
- Allies cannot declare war on each other.
- If a clan is expelled or withdraws, it cannot join a different alliance for 24 h.
  The alliance also cannot add a clan for
  24 h.
- Dissolution is immediate, but it is not possible during a siege.
- The alliance has an ally crest and `$` chat
  ([C5 notes](https://lineage2wiki.org/c5/patch-notes/),
  [l2scroll](https://www.l2scroll.com/2015/10/clan-in-classic.html),
  [C1 notes](https://lineage2wiki.org/c1/patch-notes/)).

L2J: `ALT_MAX_NUM_OF_CLANS_IN_ALLY = 3`,
`ALT_ALLY_JOIN_DAYS_WHEN_LEAVED/DISMISSED = 1`, `ALT_CREATE_ALLY_DAYS_WHEN_DISSOLVED = 1`.

**Clan war** (C4 replaced alliance wars): the clan that declares the war must be level 3+ with 15+ members.
A clan can declare war on up to 30 clans, and it can receive an unlimited number of declarations. No consent is necessary.

- In a *one-sided* war, the members of the declarer can attack the target without PK karma. But they lose
  full XP on death.
- The war becomes *mutual* when the target accepts, or when the target kills 5+ members of the declarer.
- In a mutual war, the two sides fight without karma, and a death costs
  1/4 XP. Each kill gives +1 reputation to the clan of the killer and -1 to the clan of the victim.
  If the clan of the victim is at or below 0, it does not lose reputation. War icons show above names.
- The war stops by surrender (the
  clan that surrenders loses reputation; Classic: -500 / 5,000 CRP cease-fire), by truce, or later
  by inactivity ([C4 notes](https://lineage2wiki.org/c4/patch-notes/),
  [l2scroll](https://www.l2scroll.com/2015/10/clan-war-and-siege.html),
  [Ertheia guide](https://forum.lineage2ertheia.com/guides/clan-wars-system-rules-guide/)).

### 2.6 Castle sieges

**Castles**: Gludio, Dion, Giran, Oren, Aden (C1); Innadril (C2); Goddard (C3); Rune (C4);
Schuttgart (C5). There are nine in Interlude/High Five.

**Schedule.** Each castle has a siege each **2 weeks**. The owner selects the hour (retail: Sunday
at 16:00 or 20:00; the first siege after a change of owner uses the default). Registration opens
when the date is set and **closes 24 h before**. The siege lasts **2 hours**
([l2-servera Gludio](https://l2-servera.com/en/kategorii/wiki/castles/gludio-castle/),
[C1 notes](https://lineage2wiki.org/c1/patch-notes/)). L2J `Siege.properties`:
`SiegeLength = 120`, `SiegeCycle = 2` weeks, `AttackerMaxClans = 500`, `DefenderMaxClans = 500`,
`MaxFlags = 1`, `SiegeClanMinLevel = 4`, `AttackerRespawn = 0` ms
([L2C4 siege manager](https://mintlify.wiki/fermanzolido/L2C4/api/managers/siege),
[L2C4 castle siege](https://mintlify.wiki/fermanzolido/L2C4/systems/castle-siege)).

**Registration.** Clan level 4+ (3+ on Classic). These are the rules:

- Attackers register themselves.
- Defenders send a request, and the owner clan approves it.
- The owner is automatically a defender.
- A clan that owns a castle cannot attack a different castle.
- The alliances of the owner are defenders that are approved automatically.

When the siege starts, the server teleports non-registered players to the nearest town. A death in the siege zone costs less XP
(1/4 in C1 "75% less").

**Battlefield objects.**
- *Headquarters* (siege flag NPC): the leader of the attacker clan casts *Build Headquarters*
  outside the walls. This uses crystals/gemstones (Classic: 500 D-crystals). There is one flag for each clan.
  Attackers respawn at their flag. *Build Advanced Headquarters* (Noblesse) doubles the HP for 300
  C-gemstones ([C4 notes](https://lineage2wiki.org/c4/patch-notes/)).
- *Life Control Towers*: 3 for each castle, 10,000 HP each. While one or more towers stand, defenders respawn in
  the castle each 30 s, and mercenaries/guards spawn again. When all towers are destroyed, the respawn
  delay becomes 8 minutes and the guards stop
  ([l2-servera](https://l2-servera.com/en/kategorii/wiki/castles/gludio-castle/),
  [C1 notes](https://lineage2wiki.org/c1/patch-notes/)).
- *Flame Control Towers*: 2 for each castle. When the lord pays for them (3,000,000 adena for each zone),
  they activate damage and slow zones at the gates.
- *Gates and doors*: 9 fortified doors (Gludio), closed at siege start. The lord can buy door
  HP upgrades. The defender side cannot destroy its own gates (Interlude). Gates get full
  damage only from siege weapons.
- *Siege weapons*: the Dwarven *Summon Siege Golem* (Warsmith/Artisan; 300 C-gemstones to summon,
  60/minute upkeep in C1), *Wild Hog Cannon* and *Swoop Cannon*. The Noblesse *Strider Siege
  Assault*. Golems go away when the summoner teleports.
- *Mercenaries*: the lord buys tickets (sword, spear, bow, healer, mage; later Nephilim/Seven
  Signs variants) with castle funds. The lord puts them in position before the siege. The count has a cap for each castle.

**Capture.** The leader of the attacker clan casts *Seal of Ruler* (skill 246) on the *Holy
Artifact* of the castle. It has a 180 s fixed cast, range 140, and a 5 s reuse. Any hit interrupts it, and then it must start again
([Seal of Ruler](https://l2-servera.com/en/kategorii/wiki/skill-desc/seal-of-ruler/)). A success
is a *mid-victory*:

- The clan of the caster becomes the owner.
- The server gives the attacker flag again to the previous owner and to each attacker that is not
  allied with the new owner.
- The doors/towers of the castle are not reset.
- The fight continues until the 2-hour clock expires.

The clan that owns the castle at the
end wins. If an NPC owned the castle and nobody engraved, the NPC stays the owner
([L2C4](https://mintlify.wiki/fermanzolido/L2C4/systems/castle-siege),
[strategywiki](https://strategywiki.org/wiki/Lineage_II/Castle_Sieges)).

**Ownership benefits.**

- A tax rate on all NPC shops in the territory of the castle. The lord sets 0-15%.
  Seven Signs sets a cap of 5% if Dusk holds the Seal of Strife, and increases it to 25% for Dawn. Aden also
  taxes the other castles.
- Manor seeds/crops. The castle warehouse and treasury.
- The castle gatekeeper
  and *Clan Gate* summons. Wyvern mounts.
- The *Lord's Crown* and member circlets, which give
  residence skills.
- +1500 clan reputation on capture, and +750 for each successful defense (and a
  *Blood Alliance* item in Gracia).
- Castle siege wins give data to the Olympiad/Hero monuments
  ([l2-servera](https://l2-servera.com/en/kategorii/wiki/castles/gludio-castle/),
  [C3 notes](https://lineage2wiki.org/c3/patch-notes/),
  [Interlude notes](https://lineage2wiki.org/interlude/patch-notes/)).

**Fortresses (Kamael+)**: there are 21 small forts.

- Any clan of level 4+ can register for 250,000 adena. The
  siege starts approximately one hour later (a maximum of one siege for each fort each 4 hours).
- The battle is 60 minutes.
- The attackers destroy/disable the three *power units* in the control room to open the room of the commander.
  Then the clan leader raises the flag at the flagpole.
- The owner signs an *independent* or a
  *castle contract* (it pays a part of the income to the castle for guards/upgrades).
- Fort skills come through
  *Knight's Epaulettes*.

### 2.7 Grand Olympiad and Heroes

- **Eligibility** (C4-High Five):
  - *Noblesse* status.
  - The player competes on the **main class** after the
  third class transfer (thus, level 76+ in practice).
  - The player is not in a subclass at registration.
  - The inventory is
  below 80% (legacy: fewer than 64 of 80 slots).
  - No other event registration.

  Classic 2.x
  servers decreased the entry to level 55+ with the second class, because Noblesse/third class did not
  exist there. Essence is different again ([legacy-lineage2](https://legacy-lineage2.com/Knowledge/hero.html),
  [C4 notes](https://lineage2wiki.org/c4/patch-notes/)).
- **Noblesse**: a player gets it at level 75 on a subclass, after the "Path to a
  Noblesse" chain. This chain has Possessor of a Precious Soul 1-4, Caradine's Letter and the Barakiel raid.
  It gives the Noblesse Tiara, *Blessing of Noblesse*, Noblesse teleports and Olympiad access
  ([ludo.guide](https://www.ludo.guide/guide/lineage-ii/side-quests-activities/noblesse-questline)).
- **Points.** Interlude: 18 points at the start of each monthly period, +3 each week
  (L2J `ALT_OLY_START_POINTS = 18`, `ALT_OLY_WEEKLY_POINTS = 3`).
  - The loser gives a part
  of their points to the winner. Interlude L2J: loser points / 3, with a cap of 10. High Five
  retail: one fifth.
  - A draw costs points to the two sides.
  - A player at 0 points cannot register.
  - At the
  period end, the points above 50 change 1:1000 into Noblesse Gate Passes (High Five: Olympiad
  tokens).

  High Five changed this. A player gets 10 points when they become Noblesse, and +10 each week up to 50. The rank
  rewards are 100/75/55/40/30 by percentile, and there is a 200-point Hero bonus
  ([lineage2h5](https://lineage2h5.blogspot.com/p/blog-page.html),
  [High Five notes](https://lineage2wiki.org/hi-five/patch-notes/)).
- **Match types.** *Class-based* 1v1 (the same class id; buffs are permitted in C5), *non-class* 1v1,
  and (Gracia+) *3v3 team*. The minimum number of registrants to start a round: Interlude 5 class / 9
  non-class; High Five 11 / 11 / 6 teams. In Mobius C4, registration needs 3 points (classed) or 5
  (non-classed) ([L2C4 olympiad](https://mintlify.wiki/fermanzolido/L2C4/api/managers/olympiad)).
  The weekly caps in High Five are 70 total, 60 non-class, 30 class, 10 team. Interlude had no caps.
- **Schedule.**
  - The competition is daily, 18:00-00:00 server time (L2J `ALT_OLY_START_TIME = 18`,
  `ALT_OLY_CPERIOD = 6 h`).
  - The points are given each week (`ALT_OLY_WPERIOD = 7 d`).
  - The period lasts
  one month (`ALT_OLY_PERIOD = MONTH`). After it, there is a 24 h validation period
  (`ALT_OLY_VPERIOD = 24 h`). In this period, the server calculates the heroes and players claim the rewards.
- **Match rules.**
  - The players teleport to a stadium (22 in Interlude, 160 in High Five).
  - There is a 60 s (High Five:
  25 s) preparation. HP/MP/CP are restored, and buffs are removed (not in class matches from C5).
  - No recall items. Only shots/echo crystals/energy stones.
  - The time limit is 6 minutes (C4: 3). The first player at 0
  HP loses. If not, the damage dealt decides.
  - Hero and clan skills are disabled.
- **Hero.** At validation, for each class, one Noblesse becomes Hero for the month. This is the player with the most points who fought at
  least **9 matches** (C4: 5; High Five: 15) and **won at least 1**.
  If there is a tie, the match count decides, and then the win rate ([legacy-lineage2](https://legacy-lineage2.com/Knowledge/hero.html)).
  Heroes get:
  - Hero weapons (Infinity series, cannot be traded, one month, strong SA).
  - Five hero skills (Heroic Miracle, Heroic Berserker, Heroic Valor, Heroic Grandeur, Heroic
  Dread).
  - The hero aura, `%` hero chat, +1000 clan reputation, and the Monument of Heroes
  activation when they are selected again.

---

## 3. Design decisions for Nightfall

### 3.1 Scope and numbers

- Party: 9 members, five loot modes, and an XP share by level ratio. It uses the **High Five bonus table**
  (+10%..+120%) and the **gap rule 0-9 / 10-14 (30%) / 15+ (0%)**. The range is 1,536 px (one AOI ring).
  Command channel: up to 8 parties. A party leader can open it if the clan of the leader is level 5+ *or* if the leader
  uses a Strategy Guide item.
- Chat: General (radius 40 tiles), Shout and Trade (all the map file), Party, Clan and Alliance.
  Also Command channel, Whisper, Hero (all the server, 10 s cooldown) and System. Flood: a token bucket of 1
  msg/2 s with a burst of 3 for each channel class. The limit is 500 characters. Chat bans have an expiry.
- Mentoring:
  - The mentor is at level cap - 5 or higher. The mentee is below level 40 (our cap is lower than
  the L2 cap).
  - A maximum of 3 mentees.
  - A shared buff while the two players are online, marks on mentee level-ups, and a graduation
  certificate.
  - 2-day/5-day cooldowns before a new pair.
- Clans:
  - Levels 0-8 at launch (9-11 reserved).
  - The Interlude cost table, with "clan quest" tokens in place of the items.
  - Member caps 10/15/20/30/40/40/100/120/140 (units as in L2).
  - A reputation ledger with the L2J quantities as start values.
  - The pmfun skill list as the initial clan
  skill tree.
  - The crest is a 16x12 indexed PNG. The player uploads it, and the server validates it.
- Alliance: 3 clans, level 5 founder, 24 h cooldowns.
- War:
  - Level 3 + 15 members, max 30 declarations.
  - The war becomes mutual on acceptance or after 5 kills. +1/-1 for each kill.
  - 1/4 XP loss. A surrender costs 500 reputation.
  - The war expires automatically after 21 days without a kill.
- Sieges:
  - Start with 4 castles (one for each region tier).
  - A 2-week cycle, Sunday 20:00 server time
  (the owner can select 16:00). Registration closes at T-24 h. The battle is 120 min.
  - 1 HQ for each clan, 3 life
  towers (10k HP, 30 s -> 8 min respawn), 2 flame towers.
  - Only siege weapons can cause damage to gates.
  - Engrave: a 180 s channel at a range of 4 tiles. Damage interrupts it. A mid-victory changes the
  sides.
  - Tax 0-15%.
- Olympiad:
  - We keep the Noblesse gate as a quest chain. The entry is at level 70+ (our cap is 80 in Phase 1),
  main class.
  - 18 start points, +3/week. The loser pays 1/3, with a cap of 10.
  - Class and non-class 1v1.
  - A daily window of 18:00-00:00. A monthly period and a 24 h validation.
  - A hero needs 9 matches + 1 win.

### 3.2 Social data model: in-memory vs. persisted

| System | Storage | Rationale |
|---|---|---|
| Party, party room, command channel | **In memory** in the world process, `HashMap<PartyId, Party>`. A reconnect grace period of 120 s keeps the slot | The lifetime is one session. L2J also keeps `L2Party` in memory. Only the `loot_mode` history is logged, for disputes |
| Friends, block list | Postgres | Across sessions |
| Mentorship | Postgres | A contract of many days, with rewards |
| Clan, members, units, ranks, skills, reputation ledger, crest, halls, wars, alliance | Postgres (source of truth) + a warm cache in the world process | Must continue after restarts. Reputation is money |
| Siege schedule and registrations | Postgres. The live battle state is in memory, with periodic checkpoints | The scheduler must continue after restarts. The server makes a checkpoint of the towers/gates HP each 30 s |
| Olympiad points, matches, heroes | Postgres | Monthly economy |
| Chat | Not persisted (but optional: a whisper mailbox for offline friends) | Volume. Moderation gets a ring buffer for each channel (Phase 9) |

### 3.3 Chat as a separate streaming service

At first, chat is its own tonic service (`ChatService`) in the same binary. It is isolated behind a
trait, so that it can move to its own process. It controls the channel subscriptions and the flood control, and it
never touches the world state. For the range-limited channels (General, Shout/Trade for each map), it asks the world
service for the `(map_id, cell)` of the sender through an internal channel. Then it publishes to the applicable
topic. The internal structure is a `tokio::sync::broadcast` for each topic (`party:{id}`, `clan:{id}`,
`ally:{id}`, `cc:{id}`, `map:{id}`, `hero`, `system`), and a fan-in task for each session. This task
filters by the block list and the local radius. When we shard (Phase 0 decides), replace the broadcast layer with NATS/Redis pub/sub.

### 3.4 Siege scheduler

There is a `SiegeScheduler` task for each castle, with a persistent state machine:

```
Idle --(owner sets hour or default)--> Registration --(T-24h)--> Locked
Locked --(T-0)--> Battle(120m, checkpoints) --(T+120m)--> Resolution --> Idle(next = T+14d)
Battle --(engrave success)--> Battle (mid_victory: swap owner/attackers)
```

Each transition writes `castle_sieges(castle_id, siege_at, state, owner_clan_id,
mid_victories jsonb)`; at boot, the server puts each castle back into its stored state, with the correct
remaining time. The events during the battle (`TowerDestroyed`, `GateDestroyed`, `EngraveStarted`,
`EngraveInterrupted`, `Engraved`) are world events. The scheduler consumes them and broadcasts them as
`SiegeStatus`.

---

## 4. Data model

### 4.1 Postgres tables

```sql
create table clans (
  id bigserial primary key, name text unique not null, level smallint not null default 0,
  leader_character_id bigint not null, reputation int not null default 0,
  crest bytea, ally_id bigint references alliances(id), castle_id int, hall_id int,
  notice text, created_at timestamptz not null, dissolve_at timestamptz,
  blocked_recruit_until timestamptz, blocked_ally_join_until timestamptz
);
create table clan_members (
  character_id bigint primary key, clan_id bigint not null references clans(id),
  unit smallint not null default 0,          -- 0 main, -1 academy, 100/200 royal guards, 1001..1004 knights
  rank smallint not null default 5,          -- 1..9, privilege bitmask looked up from clan_ranks
  title text, joined_at timestamptz not null, apprentice_of bigint
);
create table clan_ranks (clan_id bigint, rank smallint, privileges int, primary key (clan_id, rank));
create table clan_units (clan_id bigint, unit smallint, name text, captain_character_id bigint, primary key (clan_id, unit));
create table clan_skills (clan_id bigint, skill_id int, level smallint, unit smallint default 0, primary key (clan_id, skill_id, unit));
create table clan_reputation_ledger (
  id bigserial primary key, clan_id bigint not null, delta int not null, reason text not null,
  ref_id bigint, at timestamptz not null default now()
);
create table clan_penalties (character_id bigint primary key, kind smallint not null, until timestamptz not null);
create table alliances (id bigserial primary key, name text unique not null, leader_clan_id bigint not null, crest bytea, created_at timestamptz not null);
create table clan_wars (
  attacker_clan_id bigint, target_clan_id bigint, state smallint not null,  -- 1 declared, 2 mutual, 3 ended
  attacker_kills int default 0, target_kills int default 0, declared_at timestamptz, mutual_at timestamptz,
  last_kill_at timestamptz, ended_at timestamptz, ended_by smallint, primary key (attacker_clan_id, target_clan_id)
);
create table clan_halls (id int primary key, name text, grade smallint, map_id text, owner_clan_id bigint, rent bigint, paid_until timestamptz, functions jsonb);
create table clan_hall_auctions (hall_id int, ends_at timestamptz, bids jsonb, primary key (hall_id, ends_at));
create table castles (id int primary key, name text, map_id text, owner_clan_id bigint, tax_pct smallint default 0, treasury bigint default 0, siege_hour smallint default 20, next_siege_at timestamptz, door_upgrades jsonb);
create table castle_sieges (id bigserial primary key, castle_id int, siege_at timestamptz, state smallint, owner_at_start bigint, result jsonb);
create table siege_registrations (siege_id bigint, clan_id bigint, side smallint, approved bool, primary key (siege_id, clan_id));
create table friends (character_id bigint, friend_id bigint, since timestamptz, primary key (character_id, friend_id));
create table blocks (character_id bigint, blocked_id bigint, primary key (character_id, blocked_id));
create table mentorships (mentor_id bigint, mentee_id bigint primary key, started_at timestamptz, graduated_at timestamptz, marks_awarded int default 0);
create table mentor_penalties (character_id bigint primary key, until timestamptz);
create table chat_bans (character_id bigint primary key, until timestamptz, reason text, by_gm bigint);
create table olympiad_participants (character_id bigint, period_id int, class_id int, points int, matches int, wins int, losses int, draws int, primary key (character_id, period_id));
create table olympiad_matches (id bigserial primary key, period_id int, kind smallint, a bigint, b bigint, winner bigint, points_moved int, at timestamptz);
create table heroes (character_id bigint, period_id int, class_id int, claimed bool, primary key (character_id, period_id));
create table noblesse (character_id bigint primary key, since timestamptz);
```

### 4.2 Proto sketches (`packages/proto/nightfall/v1/social.proto`)

```proto
syntax = "proto3";
package nightfall.v1;
import "nightfall/v1/game.proto";

enum LootMode { LOOT_MODE_UNSPECIFIED = 0; LOOT_MODE_FINDERS_KEEPERS = 1; LOOT_MODE_RANDOM = 2;
                LOOT_MODE_RANDOM_SPOIL = 3; LOOT_MODE_BY_TURN = 4; LOOT_MODE_BY_TURN_SPOIL = 5; }

message PartyMember { string character_id = 1; string name = 2; uint32 class_id = 3; uint32 level = 4;
                      uint32 hp_pct = 5; uint32 mp_pct = 6; uint32 cp_pct = 7; bool online = 8;
                      Position position = 9; string map_id = 10; }
message PartyState { uint64 party_id = 1; string leader_id = 2; LootMode loot_mode = 3;
                     repeated PartyMember members = 4; uint64 command_channel_id = 5; }
message CommandChannelState { uint64 id = 1; string leader_id = 2; repeated uint64 party_ids = 3; uint32 member_count = 4; }
message PartyInvite { string from_id = 1; string from_name = 2; LootMode loot_mode = 3; int64 expires_unix = 4; }
message PartyRoom { uint64 id = 1; string title = 2; uint32 min_level = 3; uint32 max_level = 4;
                    uint32 max_members = 5; LootMode loot_mode = 6; string leader_name = 7; string map_id = 8; }

message ClanMember { string character_id = 1; string name = 2; uint32 class_id = 3; uint32 level = 4;
                     int32 unit = 5; uint32 rank = 6; string title = 7; bool online = 8; }
message ClanUnit { int32 unit = 1; string name = 2; string captain_id = 3; uint32 capacity = 4; }
message ClanSkill { uint32 skill_id = 1; uint32 level = 2; }
message ClanInfo {
  uint64 clan_id = 1; string name = 2; uint32 level = 3; string leader_name = 4;
  int32 reputation = 5; uint64 ally_id = 6; string ally_name = 7; uint32 castle_id = 8; uint32 hall_id = 9;
  bytes crest_png = 10; bytes ally_crest_png = 11; string notice = 12;
  repeated ClanMember members = 13; repeated ClanUnit units = 14; repeated ClanSkill skills = 15;
  repeated ClanWarEntry wars = 16; uint32 member_cap = 17;
}
message ClanWarEntry { uint64 other_clan_id = 1; string other_name = 2; bool mutual = 3; bool we_declared = 4;
                       uint32 our_kills = 5; uint32 their_kills = 6; }
message ClanRequest { oneof op { string invite_character = 1; string dismiss_character = 2; uint32 level_up = 3;
                                 uint32 learn_skill = 4; uint64 declare_war = 5; uint64 accept_war = 6;
                                 uint64 surrender = 7; bytes set_crest = 8; string set_notice = 9;
                                 string transfer_leader = 10; } }

enum ChatChannel { CHAT_CHANNEL_UNSPECIFIED = 0; CHAT_CHANNEL_GENERAL = 1; CHAT_CHANNEL_SHOUT = 2;
                   CHAT_CHANNEL_TRADE = 3; CHAT_CHANNEL_WHISPER = 4; CHAT_CHANNEL_PARTY = 5;
                   CHAT_CHANNEL_CLAN = 6; CHAT_CHANNEL_ALLIANCE = 7; CHAT_CHANNEL_COMMAND = 8;
                   CHAT_CHANNEL_HERO = 9; CHAT_CHANNEL_SYSTEM = 10; CHAT_CHANNEL_PARTY_ROOM = 11; }
message ChatMessage { ChatChannel channel = 1; string sender_id = 2; string sender_name = 3;
                      string target_name = 4;  // whisper
                      string text = 5; int64 sent_unix_ms = 6; uint32 sender_clan_id = 7; bool sender_hero = 8; }
message ChatSend { ChatChannel channel = 1; string target_name = 2; string text = 3; }
message ChatError { enum Reason { REASON_UNSPECIFIED = 0; FLOOD = 1; BANNED = 2; BLOCKED = 3; NO_CHANNEL = 4; TOO_LONG = 5; }
                    Reason reason = 1; int64 retry_after_ms = 2; }

enum SiegePhase { SIEGE_PHASE_UNSPECIFIED = 0; SIEGE_PHASE_IDLE = 1; SIEGE_PHASE_REGISTRATION = 2;
                  SIEGE_PHASE_LOCKED = 3; SIEGE_PHASE_BATTLE = 4; SIEGE_PHASE_RESOLUTION = 5; }
message SiegeStructure { uint64 entity_id = 1; string kind = 2;  // "life_tower" | "flame_tower" | "gate" | "hq" | "artifact"
                         uint32 hp_pct = 3; bool destroyed = 4; uint64 clan_id = 5; }
message SiegeStatus {
  uint32 castle_id = 1; string castle_name = 2; SiegePhase phase = 3; int64 phase_ends_unix = 4;
  uint64 owner_clan_id = 5; string owner_clan_name = 6;
  repeated uint64 attacker_clan_ids = 7; repeated uint64 defender_clan_ids = 8;
  repeated SiegeStructure structures = 9; uint32 life_towers_alive = 10; uint32 defender_respawn_s = 11;
  string engraving_clan = 12; int64 engrave_ends_unix = 13; uint32 tax_pct = 14;
}
message SiegeRegister { uint32 castle_id = 1; bool as_defender = 2; bool withdraw = 3; }

message OlympiadStanding { uint32 period_id = 1; int32 points = 2; uint32 matches = 3; uint32 wins = 4; uint32 losses = 5;
                           bool noble = 6; uint32 weekly_matches_left = 7; }
```

---

## 5. Interfaces

```proto
service SocialService {
  // Party invite/accept/leave/kick/loot/leader intents and PartyState pushes are oneof variants of
  // the Phase 0 WebSocket envelope (00-foundations.md §3.2), not a bidi RPC (gRPC-Web cannot client-stream).
  rpc ListPartyRooms(ListPartyRoomsRequest) returns (ListPartyRoomsResponse);
  rpc GetClan(GetClanRequest) returns (ClanInfo);
  rpc ClanOp(ClanRequest) returns (ClanOpResponse);
  rpc Friends(FriendsRequest) returns (FriendsResponse);     // list/add/remove/block/unblock
  rpc Mentoring(MentoringRequest) returns (MentoringResponse);
  rpc SiegeInfo(SiegeInfoRequest) returns (SiegeStatus);
  rpc SiegeRegister(SiegeRegister) returns (SiegeStatus);
  rpc OlympiadRegister(OlympiadRegisterRequest) returns (OlympiadStanding);
}
service ChatService {
  // ChatSend / ChatMessage / ChatError are also envelope variants on the same WebSocket. The chat
  // module stays a separate server-side broadcaster; only the wire shares the socket.
  rpc History(ChatHistoryRequest) returns (ChatHistoryResponse);  // request/response only
}
```

Server events that this phase consumes from other phases:

- `Died{victim, killer}` (war kill count, siege
respawn).
- `ExpGained{character, amount, source}` (the party split occurs in the Phase 5 reward path,
which calls `party::distribute`).
- `LevelUp` (clan reputation, mentor marks, academy graduation).
- `ZoneEntered(SIEGE)` (teleport-out at siege start).
- `ClassTransfer` (academy graduation).
- `DamageTaken{target: Artifact engrave caster}` (interrupt).

Emitted events: `PartyChanged`,
`ClanChanged`, `WarChanged`, `SiegePhaseChanged`, `HeroCrowned`, `ReputationChanged`.

Client needs:

- `PartyState` pushes on each change, and 1 Hz HP/MP/CP for members.
- `ClanInfo` when the window
opens, and deltas.
- The `ChatMessage` stream.
- `SiegeStatus` when the player goes into a siege zone and when a structure
changes.
- `OlympiadStanding` on request.
- Invitation popups with an expiry.

---

## 6. Rust implementation notes

```
social/
  party/   mod.rs (Party, PartyRegistry: DashMap<PartyId, Party>), loot.rs (mode rotation, random), xp.rs (share + bonus table), room.rs, command_channel.rs
  chat/    service.rs (tonic ChatService), topics.rs (broadcast per topic), flood.rs (token bucket), filter.rs
  friends.rs, block.rs, mentor.rs
  clan/    model.rs, repo.rs (sqlx), level.rs (cost table), reputation.rs (ledger + reasons), skills.rs, crest.rs (png decode + 16x12 validate), units.rs, penalties.rs, hall.rs
  alliance.rs, war.rs (state machine, kill counter, expiry task)
  siege/   scheduler.rs (per-castle task, persisted phase), battle.rs (structures, respawn, engrave channel), registration.rs
  olympiad/ period.rs (monthly/weekly/validation timers), matchmaking.rs (queue per kind, pairing), game.rs (stadium instance via Phase 6 instances), hero.rs
```

- **Concurrency**: the party and command-channel state is in the owned state of the world tick, as
  entities are (no locks). The reward path reads `PartyRegistry` synchronously. The server loads the clan data
  into an `Arc<RwLock<ClanCache>>`. It fills this cache at boot, and `ClanChanged`
  events invalidate it. All writes go through `clan::repo` transactions. The reputation ledger is
  append-only (`delta`, `reason`), and the cached total comes from it.
- **XP share**: `fn split(reward: u64, members: &[(Level, Alive, InRange)]) -> Vec<(CharId, u64)>`,
  implemented as pure functions with unit tests against the High Five table.
- **War expiry and siege scheduling** use `tokio::time::sleep_until` tasks. At boot, the server makes these tasks again from
  Postgres. Do not think that in-process timers continue after restarts.
- **Engrave**: a Phase 3 channelled skill with `interrupt_on_damage = true`, `fixed_cast_ms =
  180_000`; completion emits `Engraved{castle, clan}`. The siege battle does this as a
  mid-victory (owner swap, new side flags, announcement).
- **Olympiad matches** run in Phase 6 instances (`stadium_{n}`), with a `MatchRules`
  component. This component removes buffs, sets caps on items, and stops the match at HP 0 or at the timer. The results go to
  `olympiad_matches` and to the standings in one transaction.
- Crates: `sqlx`, `dashmap` (only the party rooms list), `tokio`, `image` (crest PNG decode, strict
  16x12 check). Also `governor` or a hand-written token bucket for chat, and `chrono-tz` for the server-time
  schedules.

---

## 7. Client implications

- Party HUD: member bars from `PartyState`, leader crown, loot-mode icon, and map pins from
  `PartyMember.position` when on the same map. An invitation modal with a countdown.
- Party matching window: a room list with filters. A command channel panel with one row for each party.
- Chat window: tabs per channel class, prefix parsing (`!`, `+`, `#`, `@`, `$`, `` ` ``, `%`,
  `"name`), a color for each channel, a local flood hint from `ChatError.retry_after_ms`, and block
  control.
- Clan window: roster by unit, ranks/privileges editor (leader), reputation ledger, skills with
  costs, crest upload (16x12 PNG, validated first on the client side), war list with kill counters,
  alliance tab.
- Siege UI: the siege clock and a phase banner when the player goes into the zone. Structure HP bars above towers
  and gates (`SiegeStructure`). An engrave progress bar. A registration dialog at the castle NPC.
- Olympiad UI: standing, registration buttons that the points control, match countdown. A spectator
  mode later.
- Crests show as small textures above names (`EntitySpawn.clan_id` lookup, cached PNG).

---

## 8. Open questions

1. Alliance size: L2 selected 3 after C5. Early chronicles permitted 5 (C1 notes), and some
   community pages give larger caps that we could not examine. We release with 3.
2. Command channel size cap: we could not find a hard limit in the L2J code. 8 parties
   is our estimate.
3. Chat level gate: L2 Classic prevents general/shout chat below level 20, against spam. Do we
   want it at launch, or only when bots come?
4. The mentoring level bands depend on the Phase 1 level cap. The L2 "awakened mentor" concept has
   no equivalent here.
5. The clan quest items for levels 3-5 (Blood Mark, Alliance Manifesto, Seal of Aspiration) need
   Phase 6 quests. Until then, level-ups cost only SP + adena.
6. Which Olympiad transfer gives a better ladder with a small population: the Interlude loser-pays-1/3 or the High Five 1/5? Do a simulation before launch.
7. The castle count and which maps have sieges (this needs the Phase 6 world layout). Fortresses are
   deferred until the castles operate correctly.
8. The death XP loss in war/siege (1/4) must be connected to the Phase 3 death penalty.
9. The L2J config constants that come from memory (`Clan.properties` reputation values, `Olympiad`
   timings, `Siege.properties`) must be examined against the repository.

---

## 9. Sources

- Lineage II Legacy knowledge base: clans https://legacy-lineage2.com/Knowledge/clans.html ; heroes and Olympiad https://legacy-lineage2.com/Knowledge/hero.html
- Lineage 2 patch notes (lineage2wiki.org): C1 https://lineage2wiki.org/c1/patch-notes/ ; C3 https://lineage2wiki.org/c3/patch-notes/ ; C4 https://lineage2wiki.org/c4/patch-notes/ ; C5 https://lineage2wiki.org/c5/patch-notes/ ; Interlude https://lineage2wiki.org/interlude/patch-notes/ ; High Five https://lineage2wiki.org/hi-five/patch-notes/
- High Five clan adjustments: http://lineage2h5.blogspot.com/p/clan-adjustments.html ; High Five Olympiad enhancements: https://lineage2h5.blogspot.com/p/blog-page.html
- Clan skills with reputation costs: https://lineage.pmfun.com/list/skillclan
- Classic clan and alliance rules: https://www.l2scroll.com/2015/10/clan-in-classic.html ; clan war and siege: https://www.l2scroll.com/2015/10/clan-war-and-siege.html
- Classic clan reputation: https://wiki.l2db.club/classic/Clans%20-%20Clan%20Reputation.html
- Clan wars system guide (Ertheia): https://forum.lineage2ertheia.com/guides/clan-wars-system-rules-guide/ ; Olympiad guide: https://forum.lineage2ertheia.com/guides/olympiad-guide-and-hero-rules/
- Gludio castle siege and tax: https://l2-servera.com/en/kategorii/wiki/castles/gludio-castle/ ; Seal of Ruler skill: https://l2-servera.com/en/kategorii/wiki/skill-desc/seal-of-ruler/
- StrategyWiki castle sieges: https://strategywiki.org/wiki/Lineage_II/Castle_Sieges ; Classic siege guide: https://mmoauctions.com/news/lineage-2-classic-siege-guide-how-to-conquer-the-castle
- L2C4 (Mobius Chronicle 4) docs: castle siege https://mintlify.wiki/fermanzolido/L2C4/systems/castle-siege ; siege manager https://mintlify.wiki/fermanzolido/L2C4/api/managers/siege ; Olympiad manager https://mintlify.wiki/fermanzolido/L2C4/api/managers/olympiad
- L2J server source (party, clan, siege, olympiad classes and config defaults): https://bitbucket.org/l2jserver/l2j_server ; https://github.com/L2J/L2J_Server ; L2J Mobius https://l2jmobius.org
- Party XP bonus and level gap (community): https://steamcommunity.com/app/373700/discussions/0/485624149158226564
- Clan hall auctions (NCWest): https://devtrackers.gg/lineageii/p/ec06cab7-clan-hall-auctions ; clan halls overview https://guildorder.com/games/lineage_2/guides/clan-halls
- Mentoring: https://www.ludo.guide/guide/lineage-ii/community-clan-benefits/mentoring-system ; https://devtrackers.gg/lineageii/p/4d1c7012-mentor-mentee
- Hero system: https://www.ludo.guide/guide/lineage-ii/tips-secrets/pvp-tactics-preparation/hero-system-grand-olympiad ; Noblesse questline: https://www.ludo.guide/guide/lineage-ii/side-quests-activities/noblesse-questline
- Guild Order wiki (structure overviews, version caveats): https://guildorder.com/games/lineage_2/wiki/clan-levelling-and-reputation ; https://guildorder.com/games/lineage_2/wiki/clan-and-alliance-structure ; https://guildorder.com/games/lineage_2/wiki/clan-warfare-and-declarations
- Essence castle sieges (for comparison): https://l2wiki.com/essence/articles/144.html ; https://l2central.info/essence/articles/317.html?lang=en
