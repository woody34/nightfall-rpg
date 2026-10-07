# Phase 7: Social Systems

Party, friends and chat, mentoring, clans, alliances and clan wars, castle sieges, Olympiad.
Depends on Phases 0-6 (accounts and persistence, combat and PvP flags, items, drops, world
zones and NPC AI).

---

## 1. Purpose and scope

**Delivers**

- Party: up to 9 members, leader, loot modes, XP/SP sharing with size bonus and level-gap
  rules, party matching room, command channel for raids.
- Friends list, block list, chat channels with range/flood/ban rules.
- Mentor/mentee bond with rewards.
- Clans: levels 0-11 with costs, member caps, sub-units (Academy, Royal Guards, Order of
  Knights), reputation ledger, clan skills, crests, leader transfer, penalties, clan halls.
- Alliances (3 clans) and clan wars (declaration, mutual war, kill counting, reputation).
- Castle sieges: registration, two-week schedule, HQ flags, control towers, gates, siege
  weapons, artifact engraving, ownership benefits; fortresses in brief.
- Grand Olympiad: eligibility, points, match types, weekly/monthly cycle, hero selection and
  rewards; Noblesse as the gate.

**Excludes**

- PvP damage/karma formulas (Phase 3), clan warehouse item storage internals (Phase 4),
  manor/tax economics beyond rates (Phase 5), siege NPC AI (Phase 6), UI layout (Phase 8),
  GM moderation tooling (Phase 9).

---

## 2. Reference: how Lineage 2 does it

### 2.1 Party

- **Size and leadership.** Up to 9 members. The inviter is leader; leadership can be handed
  over (`/changepartyleader`) and passes to the next member if the leader leaves. Leader
  invites, dismisses, sets loot mode (only when the party is formed) and opens a command
  channel.
- **Loot distribution modes** (L2J `L2Party.ITEM_*`): *Finders Keepers* (`ITEM_LOOTER`),
  *Random* (`ITEM_RANDOM`), *Random including spoil* (`ITEM_RANDOM_SPOIL`), *By Turn*
  (`ITEM_ORDER`), *By Turn including spoil* (`ITEM_ORDER_SPOIL`). Adena is always split
  evenly among members in range. Herbs are always finders-keepers. Spoil goes to the spoiler
  unless an "including spoil" mode is active.
- **XP/SP sharing.** On a kill, members within party range (L2J `ALT_PARTY_RANGE = 1600`
  units) and alive are candidates. Each valid member receives
  `reward * (member_level / sum(valid_member_levels)) * party_bonus`. L2J's `getValidMembers`
  has four cut-off modes: *level* (within `PARTY_XP_CUTOFF_LEVEL = 20` levels of the highest),
  *percentage* (member `level^2 / sum(level^2) >= 3%`), *auto* (gap by party size) and
  *highfive* (retail High Five table). Retail bonus tables by member count
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

  High Five level-gap rule: gap 0-9 full XP; 10-14 the lowest member gets 30%; 15+ the lowest
  gets 0. Essence/Classic servers use 0-5 / 6-9 / 10+ at 100% / 30% / 0%
  ([Steam thread](https://steamcommunity.com/app/373700/discussions/0/485624149158226564)).
  The mob-vs-party level penalty uses the *highest* party level (`partyLvl`), so a level 75
  player dragging a level 20 along gets normal XP and the 20 gets nothing.
- **Party matching.** A leader opens a *party room* (name, level range, loot mode, max 12
  members in room, C4) listed per region; players flag themselves on a waiting list and can be
  searched by class ([C4 notes](https://lineage2wiki.org/c4/patch-notes/), High Five added
  name/class search).
- **Command channel.** A party leader opens one if they have the *Clan Imperium* skill (clan
  level 5 passive, 0 reputation) or, from Interlude, by consuming a *Strategy Guide* item
  ([Interlude notes](https://lineage2wiki.org/interlude/patch-notes/),
  [pmfun clan skills](https://lineage.pmfun.com/list/skillclan)). The channel leader can see
  member parties, dismiss parties, and receives raid loot priority when the channel is large
  enough (18+ raid, 36+ boss, 225+ Antharas in Interlude). Chat prefix `` ` ``.

### 2.2 Friends, block list, chat

- **Friends**: `/friendinvite`, mutual accept, online status, friend chat (`L2FRIEND`).
  **Block**: `/block name` silences whispers, trades, invites from that player; High Five shows
  a notice when whispering a blocker.
- **Chat channels** (L2J `Say2` ids and client prefixes): General `ALL` (0, no prefix,
  ~1250-unit radius), Shout `SHOUT` (1, `!`, whole region: L2J "region" mode sends to the
  player's region and neighbours; "global" mode is a server option), Whisper `TELL` (2,
  `"name`), Party `PARTY` (3, `#`), Clan `CLAN` (4, `@`), Trade `TRADE` (8, `+`, same range as
  shout), Alliance `ALLIANCE` (9, `$`), Party room `PARTYMATCH_ROOM` (14), Command channel
  leader `PARTYROOM_COMMANDER` (15, yellow) and all `PARTYROOM_ALL` (16, `` ` ``), Hero
  `HERO_VOICE` (17, `%`, server-wide, 10 s cooldown
  [legacy-lineage2](https://legacy-lineage2.com/Knowledge/hero.html)), plus system types
  `GM`, `PETITION_*`, `ANNOUNCEMENT`, `BOAT`, `BATTLEFIELD`, `MPCC_ROOM`, `NPC_ALL/SHOUT`.
- **Flood and bans.** L2J's `FloodProtector` throttles `GlobalChat`/`Shout` at roughly one
  message per 2 s and client-side the same text cannot be repeated instantly; GMs apply
  `CHAT_BAN` punishments (timed or permanent) that reject every `Say2`. Retail Classic added a
  level gate (general/shout chat from level 20) to blunt spam bots; Interlude had none.

### 2.3 Mentor and mentee (Goddess of Destruction+)

Mentors are level 85+ (awakened); mentees are 1-84. The pair gets *Mentor's Guidance* buffs
while both are online (P./M. Atk/Def +5%, XP/SP bonuses in event variants), the mentee earns
*Mentee's Mark* currency as they level, the mentor earns Mentor Points and a *Mentor
Certificate* when the mentee graduates at 85/86, and both spend marks at a Mentoring Manager
NPC for enchant scrolls, crystals and consumables. Terminating early forfeits rewards and
applies a re-pairing cooldown (community figures: 2 days for the mentor, 5 for the mentee;
retail later removed Diplomas/Certificates and replaced them with a graduation box)
([ludo.guide](https://www.ludo.guide/guide/lineage-ii/community-clan-benefits/mentoring-system),
[NCWest dev tracker](https://devtrackers.gg/lineageii/p/4d1c7012-mentor-mentee)).

### 2.4 Clan system

**Creation**: character level 10+, 16-character alphanumeric name, not within 10 days of
dissolving a clan ([l2scroll Classic](https://www.l2scroll.com/2015/10/clan-in-classic.html)).

**Levels.** Levels 1-5 are bought with SP + adena/items by the leader; 6+ need reputation and
member counts. Interlude values ([legacy-lineage2](https://legacy-lineage2.com/Knowledge/clans.html),
[C5 notes](https://lineage2wiki.org/c5/patch-notes/), High Five changes
[lineage2h5](http://lineage2h5.blogspot.com/p/clan-adjustments.html)):

| Level | Cost (Interlude) | Reputation | Min members | Main-unit cap | Unlocks |
|---|---|---|---|---|---|
| 0 | create | - | 1 | 10 | clan chat |
| 1 | 30,000 SP + 650,000 adena | - | 1 | 15 | warehouse (deposit) |
| 2 | 150,000 SP + 2,500,000 adena | - | 1 | 20 | clan hall bidding, notice board |
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

**Units** (sub-pledges). *Academy* (level 5+, 20 members, characters under level 40 without a
second class; graduation on second class transfer gives the clan reputation and the graduate a
circlet and no re-join penalty). *Royal Guards* (level 6+, two units, 20 each, led by a
member appointed by the leader). *Order of Knights* (level 7+: two units; level 8+: four, each
attached to a Royal Guard, 10 each). Unit captains get a title and rank; privileges are a
bitmask per rank (invite, dismiss, warehouse withdraw, crest, hall functions, siege
registration, war declaration).

**Reputation.** Earned only by clans level 3+ (C5: 5+). L2J `Clan.properties` defaults, which
track retail: take castle +1500, defend castle +750, lose castle -3000; take fortress +200;
member becomes Hero +1000; Academy graduate +190..650 (more the earlier they graduate);
Festival of Darkness win +200; mutual-war or siege kill +1 to killer's clan and -1 to the
victim's (none if the victim clan is at 0); Blood Alliance item +500, Blood Oath +200, Knight's
Epaulette +20; Classic also grants +2..25 per member level-up and -500 for surrendering a war
([wiki.l2db.club](https://wiki.l2db.club/classic/Clans%20-%20Clan%20Reputation.html),
[C5 notes](https://lineage2wiki.org/c5/patch-notes/)). Spent on level-ups (table above), clan
skills, and reputation-priced items. Positive reputation shows the clan name in blue, negative
in red.

**Clan skills** (passives for every member of the required rank; reputation costs from
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

Learning a skill also consumes raid-boss items (Blood Crystals etc.) in C5-Interlude.

**Clan halls.** Auction halls in towns (C3 grades: A in Aden 50M base / 1.5M deposit / 1M
weekly rent; B in Gludio/Gludin 20M / 1M / 500k; C in Dion 8M / 500k / 200k); clan level 2+
bids from the clan warehouse, one bid per clan, cancel returns 90%, auction runs one week (two
weeks in older versions), rent is debited weekly and non-payment evicts
([C3 notes](https://lineage2wiki.org/c3/patch-notes/),
[NCWest dev tracker](https://devtrackers.gg/lineageii/p/ec06cab7-clan-hall-auctions)).
Functions bought with adena: HP/MP regen, XP restore on death, teleport list, buffs by tier,
item creation. Siegeable halls (Bandit Stronghold, Devastated Castle, Fortress of Resistance,
Fortress of the Dead, Rainbow Springs, Wild Beast Reserve) require clan level 4 and cannot be
held together with a castle.

**Crests and titles.** Clan crest 16x12 px 256-colour bitmap; alliance crest 8x12; castle
insignia 64x64 (C4). Leader sets member titles.

**Leader transfer, leaving, dismissal, dissolution.** Transfer is requested at an NPC and
applies at the next maintenance (weekly in retail). Leaving: cannot join another clan for 24 h
(C5 reduced from 5 days). Dismissal: the member waits 24 h and the clan cannot recruit for
24 h. Dissolution: 7-day countdown (cancellable), leader loses XP as one death, clan skills
removed, warehouse lost, 10-day re-creation cooldown. L2J config names:
`ALT_CLAN_JOIN_DAYS = 1`, `ALT_CLAN_CREATE_DAYS = 10`, `ALT_CLAN_DISSOLVE_DAYS = 7`,
`ALT_ACCEPT_CLAN_DAYS_WHEN_DISMISSED = 1` ([legacy-lineage2](https://legacy-lineage2.com/Knowledge/clans.html),
[l2scroll](https://www.l2scroll.com/2015/10/clan-in-classic.html)).

### 2.5 Alliances and clan wars

**Alliance**: a clan level 5+ leader creates one at an NPC; maximum **3 clans** including the
founder from C5/Interlude onward (C1 had raised the cap to 5, and some sources quote larger
early limits; nothing after C5 allows 12). Rules: a clan is in at most one alliance, clans at
war with each other cannot ally, allies cannot declare war on each other, an expelled or
withdrawing clan cannot join another alliance for 24 h and the alliance cannot add a clan for
24 h, dissolution is immediate but blocked during a siege, ally crest and `$` chat
([C5 notes](https://lineage2wiki.org/c5/patch-notes/),
[l2scroll](https://www.l2scroll.com/2015/10/clan-in-classic.html),
[C1 notes](https://lineage2wiki.org/c1/patch-notes/)). L2J: `ALT_MAX_NUM_OF_CLANS_IN_ALLY = 3`,
`ALT_ALLY_JOIN_DAYS_WHEN_LEAVED/DISMISSED = 1`, `ALT_CREATE_ALLY_DAYS_WHEN_DISSOLVED = 1`.

**Clan war** (C4 replaced alliance wars): declaring clan must be level 3+ with 15+ members;
a clan may declare on up to 30 clans and receive unlimited declarations; no consent needed.
A *one-sided* war lets the declarer's members attack the target without PK karma but they lose
full XP on death; the war becomes *mutual* when the target accepts or when 5+ of the declarer's
members are killed by the target. In a mutual war both sides fight without karma, death costs
1/4 XP, each kill gives +1 reputation to the killer's clan and -1 to the victim's (none if the
victim clan is at or below 0), war icons show over names, and the war ends by surrender (the
surrendering clan loses reputation, Classic: -500 / 5,000 CRP cease-fire), truce, or later
by inactivity ([C4 notes](https://lineage2wiki.org/c4/patch-notes/),
[l2scroll](https://www.l2scroll.com/2015/10/clan-war-and-siege.html),
[Ertheia guide](https://forum.lineage2ertheia.com/guides/clan-wars-system-rules-guide/)).

### 2.6 Castle sieges

**Castles**: Gludio, Dion, Giran, Oren, Aden (C1); Innadril (C2); Goddard (C3); Rune (C4);
Schuttgart (C5). Nine in Interlude/High Five.

**Schedule.** Each castle is sieged every **2 weeks**; the owner picks the hour (retail: Sunday
at 16:00 or 20:00; the first siege after a change of ownership defaults). Registration opens
when the date is set and **closes 24 h before**; the siege lasts **2 hours**
([l2-servera Gludio](https://l2-servera.com/en/kategorii/wiki/castles/gludio-castle/),
[C1 notes](https://lineage2wiki.org/c1/patch-notes/)). L2J `Siege.properties`:
`SiegeLength = 120`, `SiegeCycle = 2` weeks, `AttackerMaxClans = 500`, `DefenderMaxClans = 500`,
`MaxFlags = 1`, `SiegeClanMinLevel = 4`, `AttackerRespawn = 0` ms
([L2C4 siege manager](https://mintlify.wiki/fermanzolido/L2C4/api/managers/siege),
[L2C4 castle siege](https://mintlify.wiki/fermanzolido/L2C4/systems/castle-siege)).

**Registration.** Clan level 4+ (3+ on Classic). Attackers self-register; defenders request
and the owning clan approves; the owner is auto-defender; a clan that owns a castle cannot
attack another; alliances of the owner are auto-approved defenders. Non-registered players are
teleported to the nearest town when the siege starts. Death in the siege zone costs reduced XP
(1/4 in C1 "75% less").

**Battlefield objects.**
- *Headquarters* (siege flag NPC): the attacking clan leader casts *Build Headquarters*
  (consumes crystals/gemstones; Classic: 500 D-crystals) outside the walls, one flag per clan;
  attackers respawn at their flag; *Build Advanced Headquarters* (Noblesse) doubles HP for 300
  C-gemstones ([C4 notes](https://lineage2wiki.org/c4/patch-notes/)).
- *Life Control Towers*: 3 per castle, 10,000 HP each; while any stands, defenders respawn in
  the castle every 30 s and mercenaries/guards re-spawn; when all are destroyed the respawn
  delay becomes 8 minutes and guards stop coming
  ([l2-servera](https://l2-servera.com/en/kategorii/wiki/castles/gludio-castle/),
  [C1 notes](https://lineage2wiki.org/c1/patch-notes/)).
- *Flame Control Towers*: 2 per castle; when funded by the lord (3,000,000 adena per zone)
  they activate damage and slow zones at the gates.
- *Gates and doors*: 9 fortified doors (Gludio), closed at siege start; the lord can buy door
  HP upgrades and the defending side cannot destroy its own gates (Interlude). Gates take full
  damage only from siege weapons.
- *Siege weapons*: Dwarven *Summon Siege Golem* (Warsmith/Artisan; 300 C-gemstones to summon,
  60/minute upkeep in C1), *Wild Hog Cannon* and *Swoop Cannon*; Noblesse *Strider Siege
  Assault*. Golems dismiss when the summoner teleports.
- *Mercenaries*: the lord buys tickets (sword, spear, bow, healer, mage; later Nephilim/Seven
  Signs variants) with castle funds and places them before the siege; count capped per castle.

**Capture.** The attacking clan leader casts *Seal of Ruler* (skill 246) on the castle's *Holy
Artifact*: 180 s fixed cast, range 140, interrupted by any hit and must restart, 5 s reuse
([Seal of Ruler](https://l2-servera.com/en/kategorii/wiki/skill-desc/seal-of-ruler/)). Success
is a *mid-victory*: the caster's clan becomes owner, the previous owner (and any attacker not
allied with the new owner) is re-flagged as attacker, the castle's doors/towers are not reset,
and the fight continues until the 2-hour clock expires. The clan still owning the castle at the
end wins; if the castle was NPC-owned and nobody engraved, it stays NPC-owned
([L2C4](https://mintlify.wiki/fermanzolido/L2C4/systems/castle-siege),
[strategywiki](https://strategywiki.org/wiki/Lineage_II/Castle_Sieges)).

**Ownership benefits.** Tax rate on all NPC shops in the castle's territory (lord sets 0-15%;
Seven Signs caps 5% if Dusk holds the Seal of Strife, raises to 25% for Dawn), Aden additionally
taxes the other castles; manor seeds/crops; castle warehouse and treasury; castle gatekeeper
and *Clan Gate* summoning; wyvern mounts; *Lord's Crown* and member circlets granting
residence skills; +1500 clan reputation on capture and +750 per successful defence (and a
*Blood Alliance* item in Gracia); Castle siege wins feed Olympiad/Hero monuments
([l2-servera](https://l2-servera.com/en/kategorii/wiki/castles/gludio-castle/),
[C3 notes](https://lineage2wiki.org/c3/patch-notes/),
[Interlude notes](https://lineage2wiki.org/interlude/patch-notes/)).

**Fortresses (Kamael+)**: 21 small forts; any clan level 4+ registers for 250,000 adena and the
siege starts about an hour later (max one siege per fort every 4 hours); 60-minute battle;
attackers destroy/disable the three *power units* in the control room to open the commander's
room, then the clan leader raises the flag at the flagpole; the owner signs an *independent* or
*castle contract* (pays a share of income to the castle for guards/upgrades); fort skills via
*Knight's Epaulettes*.

### 2.7 Grand Olympiad and Heroes

- **Eligibility** (C4-High Five): *Noblesse* status, competing on the **main class** after the
  third class transfer (so effectively level 76+), not in a subclass at registration, inventory
  under 80% (legacy: fewer than 64 of 80 slots), no other event registration. Classic 2.x
  servers lowered the entry to level 55+ with second class because Noblesse/third class did not
  exist there; Essence differs again ([legacy-lineage2](https://legacy-lineage2.com/Knowledge/hero.html),
  [C4 notes](https://lineage2wiki.org/c4/patch-notes/)).
- **Noblesse** is obtained by reaching level 75 on a subclass and finishing the "Path to a
  Noblesse" chain (Possessor of a Precious Soul 1-4, Caradine's Letter, the Barakiel raid),
  granting the Noblesse Tiara, *Blessing of Noblesse*, Noblesse teleports and Olympiad access
  ([ludo.guide](https://www.ludo.guide/guide/lineage-ii/side-quests-activities/noblesse-questline)).
- **Points.** Interlude: 18 points at the start of each monthly period, +3 every week
  (L2J `ALT_OLY_START_POINTS = 18`, `ALT_OLY_WEEKLY_POINTS = 3`); the loser transfers a share
  of their points to the winner (Interlude L2J: loser points / 3, capped at 10; High Five
  retail: one fifth), draws cost both sides, a player at 0 points cannot register, and at
  period end points over 50 convert 1:1000 into Noblesse Gate Passes (High Five: Olympiad
  tokens). High Five reworked to 10 points on becoming Noblesse, +10 weekly up to 50, rank
  rewards 100/75/55/40/30 by percentile and a 200-point Hero bonus
  ([lineage2h5](https://lineage2h5.blogspot.com/p/blog-page.html),
  [High Five notes](https://lineage2wiki.org/hi-five/patch-notes/)).
- **Match types.** *Class-based* 1v1 (same class id, buffs allowed in C5), *non-class* 1v1,
  and (Gracia+) *3v3 team*. Minimum registrants to start a round: Interlude 5 class / 9
  non-class; High Five 11 / 11 / 6 teams. Registration needs 3 points (classed) or 5
  (non-classed) in Mobius C4 ([L2C4 olympiad](https://mintlify.wiki/fermanzolido/L2C4/api/managers/olympiad)).
  Weekly caps in High Five: 70 total, 60 non-class, 30 class, 10 team; Interlude had none.
- **Schedule.** Competition daily 18:00-00:00 server time (L2J `ALT_OLY_START_TIME = 18`,
  `ALT_OLY_CPERIOD = 6 h`); points granted weekly (`ALT_OLY_WPERIOD = 7 d`); the period lasts
  one month (`ALT_OLY_PERIOD = MONTH`) followed by a 24 h validation period
  (`ALT_OLY_VPERIOD = 24 h`) in which heroes are computed and rewards claimed.
- **Match rules.** Teleport to a stadium (22 in Interlude, 160 in High Five), 60 s (High Five:
  25 s) preparation, HP/MP/CP restored, buffs stripped (except in class matches from C5),
  no recall items, only shots/echo crystals/energy stones; 6-minute limit (C4: 3), first to 0
  HP loses, otherwise damage dealt decides; hero and clan skills disabled.
- **Hero.** At validation, for each class the Noblesse with the most points who fought at
  least **9 matches** (C4: 5; High Five: 15) and **won at least 1** becomes Hero for the month;
  ties break on match count then win rate ([legacy-lineage2](https://legacy-lineage2.com/Knowledge/hero.html)).
  Heroes receive: hero weapons (Infinity series, untradeable, one month, strong SA),
  five hero skills (Heroic Miracle, Heroic Berserker, Heroic Valor, Heroic Grandeur, Heroic
  Dread), the hero aura, `%` hero chat, +1000 clan reputation, and Monument of Heroes
  activation on re-election.

---

## 3. Design decisions for Nightfall

### 3.1 Scope and numbers

- Party: 9 members, five loot modes, XP share by level ratio, **High Five bonus table**
  (+10%..+120%) and **gap rule 0-9 / 10-14 (30%) / 15+ (0%)**, range 1,536 px (one AOI ring).
  Command channel: up to 8 parties; opened by a party leader whose clan is level 5+ *or* who
  consumes a Strategy Guide item.
- Chat: General (radius 40 tiles), Shout and Trade (whole map file), Party, Clan, Alliance,
  Command channel, Whisper, Hero (server-wide, 10 s cooldown), System. Flood: token bucket 1
  msg/2 s with burst 3 per channel class; 500-char limit; chat bans with expiry.
- Mentoring: mentor at level cap - 5 or higher, mentee below level 40 (our cap is lower than
  L2's), max 3 mentees, shared buff while both online, marks on mentee level-ups, graduation
  certificate; 2-day/5-day re-pair cooldowns.
- Clans: levels 0-8 at launch (9-11 reserved); Interlude cost table with items replaced by
  "clan quest" tokens; member caps 10/15/20/30/40/40/100/120/140 (units as in L2); reputation
  ledger with the L2J amounts as starting values; the pmfun skill list as the initial clan
  skill tree; crest as a 16x12 indexed PNG uploaded and validated server-side.
- Alliance: 3 clans, level 5 founder, 24 h cooldowns.
- War: level 3 + 15 members, max 30 declarations, mutual on accept or 5 kills, +1/-1 per kill,
  1/4 XP loss, surrender costs 500 reputation, auto-expire after 21 days without a kill.
- Sieges: start with 4 castles (one per region tier); 2-week cycle, Sunday 20:00 server time
  (owner may choose 16:00), registration closes T-24 h, 120 min battle, 1 HQ per clan, 3 life
  towers (10k HP, 30 s -> 8 min respawn), 2 flame towers, gates damageable only by siege
  weapons, engrave 180 s channel at range 4 tiles interrupted by damage, mid-victory swaps
  sides, tax 0-15%.
- Olympiad: Noblesse gate kept as a quest chain; entry at level 70+ (our cap is 80 in Phase 1),
  main class; 18 start points, +3/week, loser pays 1/3 capped at 10; class and non-class 1v1;
  daily window 18:00-00:00; monthly period, 24 h validation; hero needs 9 matches + 1 win.

### 3.2 Social data model: in-memory vs. persisted

| System | Storage | Rationale |
|---|---|---|
| Party, party room, command channel | **In-memory** in the world process, `HashMap<PartyId, Party>`; reconnect grace 120 s keeps the slot | Lifetime is a session; L2J also keeps `L2Party` in memory. Only `loot_mode` history is logged for disputes |
| Friends, block list | Postgres | Cross-session |
| Mentorship | Postgres | Days-long contract with rewards |
| Clan, members, units, ranks, skills, reputation ledger, crest, halls, wars, alliance | Postgres (source of truth) + warm cache in the world process | Must survive restarts; reputation is money |
| Siege schedule and registrations | Postgres; live battle state in memory with periodic checkpoints | Scheduler must survive restarts; towers/gates HP checkpointed every 30 s |
| Olympiad points, matches, heroes | Postgres | Monthly economy |
| Chat | Not persisted (except whisper mailbox for offline friends, optional) | Volume; moderation gets a ring buffer per channel (Phase 9) |

### 3.3 Chat as a separate streaming service

Chat is its own tonic service (`ChatService`) in the same binary at first, isolated behind a
trait so it can move to its own process. It owns channel subscriptions and flood control and
never touches world state; range-limited channels (General, Shout/Trade per map) ask the world
service for the sender's `(map_id, cell)` via an internal channel and publish to the matching
topic. Internally: `tokio::sync::broadcast` per topic (`party:{id}`, `clan:{id}`,
`ally:{id}`, `cc:{id}`, `map:{id}`, `hero`, `system`) plus a per-session fan-in task that
filters by block list and local radius. Swap the broadcast layer for NATS/Redis pub/sub when
sharding (Phase 0 decides).

### 3.4 Siege scheduler

A `SiegeScheduler` task per castle with a persisted state machine:

```
Idle --(owner sets hour or default)--> Registration --(T-24h)--> Locked
Locked --(T-0)--> Battle(120m, checkpoints) --(T+120m)--> Resolution --> Idle(next = T+14d)
Battle --(engrave success)--> Battle (mid_victory: swap owner/attackers)
```

Each transition writes `castle_sieges(castle_id, siege_at, state, owner_clan_id,
mid_victories jsonb)`; on boot every castle is re-entered at its stored state with the correct
remaining time. Battle-time events (`TowerDestroyed`, `GateDestroyed`, `EngraveStarted`,
`EngraveInterrupted`, `Engraved`) are world events consumed by the scheduler and broadcast as
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

Server events consumed from other phases: `Died{victim, killer}` (war kill counting, siege
respawn), `ExpGained{character, amount, source}` (party split happens in Phase 5's reward path
by calling `party::distribute`), `LevelUp` (clan reputation, mentor marks, academy graduation),
`ZoneEntered(SIEGE)` (teleport-out at siege start), `ClassTransfer` (academy graduation),
`DamageTaken{target: Artifact engrave caster}` (interrupt). Emitted: `PartyChanged`,
`ClanChanged`, `WarChanged`, `SiegePhaseChanged`, `HeroCrowned`, `ReputationChanged`.

Client needs: `PartyState` pushes on any change plus 1 Hz HP/MP/CP for members; `ClanInfo` on
open and deltas; `ChatMessage` stream; `SiegeStatus` on entering a siege zone and on structure
changes; `OlympiadStanding` on demand; invitation popups with expiry.

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

- **Concurrency**: party and command-channel state lives in the world tick's owned state like
  entities (no locks); `PartyRegistry` is read by the reward path synchronously. Clan data is
  loaded into an `Arc<RwLock<ClanCache>>` warmed at boot and invalidated by `ClanChanged`
  events; all writes go through `clan::repo` transactions and the reputation ledger is
  append-only (`delta`, `reason`) with the cached total derived from it.
- **XP share**: `fn split(reward: u64, members: &[(Level, Alive, InRange)]) -> Vec<(CharId, u64)>`
  implemented as pure functions with unit tests against the High Five table.
- **War expiry and siege scheduling** use `tokio::time::sleep_until` tasks re-created from
  Postgres on boot; never rely on in-process timers surviving restarts.
- **Engrave**: a Phase 3 channelled skill with `interrupt_on_damage = true`, `fixed_cast_ms =
  180_000`; completion emits `Engraved{castle, clan}` which the siege battle handles as
  mid-victory (owner swap, side re-flag, announce).
- **Olympiad matches** run inside Phase 6 instances (`stadium_{n}`), with a `MatchRules`
  component that strips buffs, caps items and ends on HP 0 or timer; results go to
  `olympiad_matches` and standings in one transaction.
- Crates: `sqlx`, `dashmap` (party rooms list only), `tokio`, `image` (crest PNG decode, strict
  16x12 check), `governor` or hand-rolled token bucket for chat, `chrono-tz` for server-time
  schedules.

---

## 7. Client implications

- Party HUD: member bars from `PartyState`, leader crown, loot-mode icon, map pins from
  `PartyMember.position` when on the same map; invite modal with countdown.
- Party matching window: room list with filters; command channel panel with per-party rows.
- Chat window: tabs per channel class, prefix parsing (`!`, `+`, `#`, `@`, `$`, `` ` ``, `%`,
  `"name`), colour per channel, local flood hint from `ChatError.retry_after_ms`, block
  management.
- Clan window: roster by unit, ranks/privileges editor (leader), reputation ledger, skills with
  costs, crest upload (16x12 PNG, validated client-side first), war list with kill counters,
  alliance tab.
- Siege UI: siege clock and phase banner on entering the zone, structure HP bars over towers
  and gates (`SiegeStructure`), engrave progress bar, registration dialog at the castle NPC.
- Olympiad UI: standing, registration buttons gated by points, match countdown, spectator
  mode later.
- Crests render as small textures above names (`EntitySpawn.clan_id` lookup, cached PNG).

---

## 8. Open questions

1. Alliance size: L2 settled on 3 after C5; early chronicles allowed 5 (C1 notes) and some
   community pages claim larger caps that could not be verified. We ship 3.
2. Command channel size cap: L2J has no hard limit in code that could be confirmed; 8 parties
   is our guess.
3. Chat level gate: L2 Classic restricts general/shout chat below level 20 for anti-spam; do we
   want it at launch or only when bots appear?
4. Mentoring level bands depend on the Phase 1 level cap; the L2 "awakened mentor" concept has
   no analogue here.
5. Clan quest items for levels 3-5 (Blood Mark, Alliance Manifesto, Seal of Aspiration) need
   Phase 6 quests; until then level-ups cost SP + adena only.
6. Whether Interlude's loser-pays-1/3 or High Five's 1/5 Olympiad transfer gives a healthier
   ladder with a small population; simulate before launch.
7. Castle count and which maps host sieges (needs the Phase 6 world layout); fortresses
   deferred until castles are proven.
8. Death XP loss in war/siege (1/4) must be wired into Phase 3's death penalty.
9. L2J config constants quoted from memory (`Clan.properties` reputation values, `Olympiad`
   timings, `Siege.properties`) should be verified against the repository.

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
