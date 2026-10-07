# Phase 3: Combat and skills

Status: planning reference. Depends on Phase 1 (derived stats: P.Atk, M.Atk, P.Def, M.Def, accuracy, evasion, crit rate, attack/cast speed, stat bonus tables) and Phase 2 (classes, skill trees). Feeds Phase 4 (weapons supply P.Atk, random damage, attack range/angle, reuse) and Phase 6 (monster AI consumes the aggro model).

## 1. Purpose and scope

Delivers the authoritative server combat loop and the skill system:

- Combat core: auto-attack scheduling, hit/miss, critical, shield block, physical and magical damage, positional modifiers, weapon-type behaviour (bow reuse, polearm sweep, dual hits, dagger blows).
- Skill system: data format, learning with SP, casting pipeline (cost, cast time, interruption, reuse), targeting, effect lists, enchant routes.
- Buffs/debuffs: durations, slot limits, stacking groups, dispel/cancel.
- Status effects and their land-rate formula.
- Elemental attributes.
- Targeting and aggro (hate lists), leash, social aggro.
- Death, XP loss, resurrection.
- PvP rules: flag, karma, PK counter, chaotic drops, zone types.
- Pets and servitors (upkeep, XP share).
- Nightfall's chosen formulas, the Rust tick/combat loop, the skill data format and combat proto events.

Excluded: item stats and enchant (Phase 4), monster AI decision-making beyond hate selection (Phase 6), Olympiad/siege rules (Phase 7), VFX/animation (Phase 8).

## 2. Reference: how Lineage 2 does it

Primary source: L2J High Five `Formulas.java` and related classes; Interlude differences from the L2J C6 fork. Where the two eras differ, both are given.

### 2.1 Physical damage

**Normal attack (H5 L2J `calcPhysDam`):**

```
defence   = target.pDef (+ target.shldDef if shield block succeeded; perfect block -> damage = 1)
proximity = behind ? 1.2 : front ? 1.0 : 1.1       // side +10%, back +20% (L2J marks as unconfirmed)
damage    = attacker.pAtk * (soulshot ? 2 : 1)
if crit:
    damage = 2 * CRITICAL_DAMAGE_mult * CRITICAL_DAMAGE_POS * target.DEFENCE_CRITICAL_DAMAGE
             * (76 * damage * proximity / defence)
             + CRITICAL_DAMAGE_ADD * 77 / defence + target.DEFENCE_CRITICAL_DAMAGE_ADD
else:
    damage = 76 * damage * proximity / defence
damage *= weaponTraitBonus (sword/blunt/dagger/... vulnerability)
damage *= randomDamageMultiplier          // 1 + rnd(-r, r)/100, r = weapon.randomDamage (10 for fists: 5 + sqrt(level))
damage *= PVP_PHYSICAL_DMG (if PvP)
damage *= attributeBonus                  // 2.9
damage *= npcLevelPenalty (if target NPC >= 2 levels higher)
return max(damage, 1)
```

Interlude L2J uses **70** instead of 76/77: `damage = 70 * damage / defence`, with soulshot `damage *= 2` applied before, crit as `damage *= 2` (plus CRITICAL_DAMAGE additive), and the random spread as `damage += rnd * damage / 10` (0..+10%, not symmetric). Both are "the L2 formula"; the retail constant changed between chronicles, which is why community write-ups quote 70, 76 or 77.

**Physical skill (H5 `calcSkillPhysDam`):**

```
baseMod = 77 * (skill.power + pAtk * (ss ? 2 : 1)) / defence
damage  = baseMod * proximity * pvpBonus * attackTraitBonus * attributeBonus * randomDamageMultiplier * npcPenalty
damage  = PHYSICAL_SKILL_POWER stat applied (e.g. +x% skill power buffs)
```

Note that in H5 the soulshot doubles pAtk but not skill power; in Interlude `ssBoost` was a per-skill multiplier applied to power. Skill crits are decided separately (`calcSkillCrit`: `STR_bonus * skill.critChance > rnd(100)`); when a physical skill crits the damage is doubled by the effect handler (`PhysicalAttack` effect), not by the formula above.

**Dagger blows (`calcBlowDamage`):** `77 * (power + pAtk * (ss ? 1.458 : 1)) / defence`, multiplied by `CRITICAL_DAMAGE` and position (`CRITICAL_DAMAGE_POS`), i.e. a blow is always a critical hit. Blow success (`calcBlowSuccess`): `rate = blowChance * DEX_bonus * sideMod` with `sideMod` 1.0 front / 1.5 side / 2.0 back; Mortal Blow has `blowChance = 20`. Backstab additionally requires being behind.

**Critical rate:** `critRate = baseCritRate(4) * DEX_bonus * 10` in permille, modified by `CRITICAL_RATE` and `CRITICAL_RATE_POS` (position buffs), capped at `MaxPCritRate = 500` (50%). Roll: `rate > rnd(1000)`. Critical damage multiplier is 2x base, further multiplied by `CRITICAL_DAMAGE` (Focus, dyes) and reduced by target `DEFENCE_CRITICAL_DAMAGE`.

**Shield block (`calcShldUse`):** requires a shield in the left hand; `shldRate = SHIELD_RATE * DEX_bonus`; block only if attacker is within the shield angle (`SHIELD_DEFENCE_ANGLE + 120` degrees in front); bows get `shldRate *= 1.3`; `PerfectShieldBlockRate = 10` (%) of successful blocks become perfect blocks (damage 1). On normal block, `shldDef` is added to P.Def (or, with `ShieldBlocks = True`, subtracted from damage).

### 2.2 Magic damage

**H5 `calcMagicDam`:**

```
mDef   = target.mDef (+ shldDef * shieldDefensePercent/100 if blocked)   // Wind Strike: 40%
mAtk   = attacker.mAtk * (blessedSpiritshot ? 4 : spiritshot ? 2 : 1)
damage = 91 * sqrt(mAtk) / mDef * skill.power
if magicFailures and !calcMagicSuccess(attacker, target, skill):
    // second roll: half damage ("attack failed") if level gap <= 9, else damage = 1 ("resisted")
    damage = calcMagicSuccess(...) && (target.level - attacker.level <= 9) ? damage / 2 : 1
elif mcrit:
    damage *= (PvP ? 2.5 : 3) * MAGIC_CRIT_DMG
damage *= randomDamageMultiplier
damage *= PVP_MAGICAL_DMG (if PvP)
damage *= attributeBonus
damage *= npcLevelPenalty
```

`sqrt(mAtk)` is why M.Atk is quadratic in INT (`mAtk = base * INT_bonus^2 * levelMod^2`): the two cancel to a linear INT scaling at the damage level. Interlude: same `91 * sqrt(mAtk) / mDef * power`, with `MAGIC_CRITICAL_POWER` config (default 3) for the crit multiplier and no PvP 2.5 distinction.

**Magic critical rate:** `mCrit = 1 * WIT_bonus * 10` permille (base rate 1 == 10 permille at WIT bonus 1.0), modified by `MCRITICAL_RATE`, capped at `MaxMCritRate = 200` (20%).

**Magic success / "resisted" (`calcMagicSuccess`):**

```
lvlDifference = target.level - (skill.magicLevel > 0 ? skill.magicLevel : attacker.level)
lvlModifier   = 1.3 ^ lvlDifference
rate          = 100 - round(lvlModifier * npcPenalty * MAGIC_SUCCESS_RES)
success       = rnd(100) < rate
```

So using a `magicLevel` 40 nuke on a level 50 target gives `1.3^10 = 13.8` -> 86% success; on a level 60 target `1.3^20 = 190` -> rate clamps negative -> always fails. The skill's `magicLvl` table (e.g. Wind Strike 1/4/7/11/14 for levels 1-5) is therefore the real "level" of a spell. Interlude used the same `1.3^diff` but rolled against 10000 with `rate = pow(1.3, diff) * 100`.

**Mana damage (`calcManaDam`):** `sqrt(mAtk) * power * (target.maxMp / 97) / mDef`.

### 2.3 Hit / miss

```
chance = (80 + 2 * (attacker.accuracy - target.evasion)) * 10          // permille
chance *= conditionBonus      // hitConditionBonus.xml: front 0%, side +5%, back +10%, high ground +3%, low -3%, night -10%, rain -3%
chance = clamp(chance, 200, 980)                                       // 20% .. 98%
hit = chance >= rnd(1000)
```

Identical in Interlude and H5. Accuracy = `sqrt(DEX) * 6 + level` (+ extra per level above 69 and 77); evasion = `sqrt(DEX) * 6 + level` (+ (level - 69), x1.2 above 78), capped `MaxEvasion = 250`. So at equal DEX/level, base hit chance is 80%; each point of accuracy over evasion adds 2%. Physical skills use the same roll; debuff/magic skills do not miss, they "resist" (2.2 / 2.6).

### 2.4 Attack speed, cast speed, reuse

- **Attack interval:** `timeBetweenAttacks_ms = 500000 / pAtkSpd`. At 300 pAtkSpd (naked template) that is 1667 ms; at the cap `MaxPAtkSpeed = 1500` it is 333 ms. The hit lands at roughly half the interval (animation hit frame); the next attack can start at the full interval.
- **Bow/crossbow reuse:** after a shot, additional `reuse_ms = weapon.reuseDelay * 333 / pAtkSpd` (retail bows have `reuseDelay` ~1500) during which no new shot can start; soulshot/MP consumption per shot (bows consume MP).
- **Dual weapons:** two hit rolls per attack, each with its own miss/crit/shield roll, each at half the normal damage split (L2J `doAttackHitByDual`).
- **Polearm:** `doAttackHitByPole` hits the primary target then up to `ATTACK_COUNT_MAX - 1` extra targets (base 1, Polearm Mastery raises it) inside `physicalAttackRange` and the weapon's `physicalAttackAngle` (polearm base angle 120 deg), excluding targets more than 650 z-units away; extra targets receive 85% ("attackpercent") damage, decreasing per target.
- **Cast time:** `castTime_ms = skill.hitTime * 333 / (isMagic ? mAtkSpd : pAtkSpd)`; 333 is the baseline mAtkSpd, so Wind Strike's `hitTime = 4000` is 4.0 s at 333 casting speed and 2.0 s at 666. Spiritshots cut magic cast time to 60%. Minimum 500 ms if the base hitTime > 500. `MaxMAtkSpeed = 1999`.
- **Reuse delay:** `skill.reuseDelay` ms, reduced by `REUSE_DELAY` stats (Interlude: cast speed reduced reuse for magic skills; H5 does not). Stored across logout (`StoreSkillCooltime = True`).
- **Cast interruption (`calcAtkBreak`):** when a caster takes damage, `init = 15 (if casting) + sqrt(13 * dmg) - (MEN_bonus * 100 - 100)`, clamped 1..99, rolled against 100. Bow attacks can also be interrupted (`CancelByHit = cast`).

### 2.5 Skill system

**Skill definition (L2J XML, `data/stats/skills/NNNNN-NNNNN.xml`).** One `<skill id levels name>` with per-level `<table>`s and `<set>` attributes. The important attributes:

| Attribute | Meaning | Example |
|---|---|---|
| `operateType` | A1 instant active, A2 instant + continuous (buff/debuff), A3/A4 continuous, CA1/CA5 channelled, DA1/DA2 directional/rush, P passive, T toggle | Power Strike A1, Stun Attack A2, Shield A2 |
| `targetType` | SELF, TARGET, ENEMY, ENEMY_ONLY, PC_BODY (dead player), NPC_BODY, GROUND, NONE... | Sleep: ENEMY_ONLY |
| `affectScope` | SINGLE, POINT_BLANK (around caster), RANGE (around target), FAN, SQUARE, PARTY, PLEDGE, DEAD_PARTY... with `affectRange`, `affectLimit` ("9-10" = 9 to 10 targets) and `affectObject` (NOT_FRIEND, FRIEND, ALL...) | Provoke: POINT_BLANK, range 500/700/900, limit 9-10 |
| `castRange` / `effectRange` | max distance to start the cast / to still land at the end | Wind Strike 600 / 1100, melee 40 / 400 |
| `hitTime` / `coolTime` / `reuseDelay` | cast animation, post-cast lock, cooldown (ms) | Power Strike 1080 / 720 / 3000 |
| `mpConsume1` / `mpConsume2` | MP at cast start / at completion | Sleep 5 / 17 |
| `hpConsume`, `itemConsumeId/Count` | HP cost (Dark Elf skills), items (summons eat crystals) | Summon Kat the Cat: 1-12 D-crystals |
| `magicLvl` | spell level for success/level-bonus formulas | Sleep lvl 1 = 23 |
| `isMagic` | 1 = uses mAtkSpd/M.Def path | |
| `isDebuff`, `abnormalType`, `abnormalLvl`, `abnormalTime` (s), `abnormalVisualEffect` | stacking group, stack order, duration | Shield: PD_UP, lvl 1/2/3, 1200 s |
| `activateRate`, `basicProperty`, `lvlBonusRate`, `trait` | land-rate inputs (2.6) | Stun Attack 50 / CON / 1 / SHOCK |
| `attributeType`, `attributePower` | element and attack attribute added by the skill | Wind Strike WIND 20 |
| `effectPoint` | aggro points (negative = hostile) | Power Strike -52..-86 |
| `nextActionAttack`, `overHit`, `hitCancelTime` | auto-attack after use; over-hit XP bonus; interruptible window | |
| `<cond>` | usage conditions: weapon kinds, player state | `<using kind="DAGGER"/>` |
| `<effects>` / `<enchant1Effects>` | effect list: `PhysicalAttack{power}`, `MagicalAttackRange{power, shieldDefensePercent}`, `FatalBlow{power, blowChance}`, `Stun`, `Sleep`, `Buff{mul/add stat}`, `DefenceTrait{SLEEP=100}`, `Summon{npcId, consumeItemId, lifeTime}`, `Resurrection{power}`... | |
| `enchantGroup1..N` + `<enchantN name=... val=...>` | enchant routes overriding attributes per +level | Sleep: route 1 raises `activateRate` 80->100, route 2 raises `abnormalTime` 31->60 |

**Learning.** `classSkillTree.xml` entries (Phase 2) with `getLevel` and `levelUpSp`; some need a spellbook item. SP comes from kills (`SpReward` per monster, split by damage share). Costs scale steeply: Power Strike lvl 1 = 50 SP, lvl 4 = 370 SP; 2nd-class skills cost 10^4..10^5; 3rd-class 10^6+.

**Enchanting (76+).** `enchantSkillGroups.xml`: each route has 30 levels; each level has `adena`, `sp` and `chanceNN` by character level 76..85. Group 1 (2nd-class buffs/debuffs): +1 costs 74,250 adena and 575,980 SP with 82% at level 76 / 97% at 78+; +10 costs 189,000 / 680,020 with 20% at 76 but 79% at 81+; +30 costs 430,650 / 943,620 with 24% at 85 and 1% below. Group 2 (attack skills) costs ~1.8x the adena/SP. A failed enchant with a normal Giant's Codex resets to +0; with Codex Mastery the level is kept. Route change (e.g. Power -> Time) costs 20% of the next level's SP/adena plus Codex: Discipline and may drop 1-3 levels. Routes typically available: Power (+damage/+rate), Cost (-MP), Time (+duration), Chance (+activateRate), plus element routes for nukes.

### 2.6 Buffs, debuffs, status effects

**Slots (L2J `character.properties`):** `MaxBuffAmount = 20`, +4 from Divine Inspiration (`ENLARGE_ABNORMAL_SLOT`, max 24; `DivineInspirationSpBookNeeded = True`), `MaxDanceAmount = 12` (songs + dances share this separate pool since Gracia), `MaxTriggeredBuffAmount = 12`. Debuffs have no slot limit in L2J (retail shows 8 debuff icons). When the buff list is full, the *oldest* buff (first in queue) is removed to make room (`CharEffectList.add`). Toggles, 7-Signs, healing-potion effects and `SUMMON_CONDITION` do not count.

**Stacking.** Each skill has an `abnormalType` (the stack group: PD_UP, MD_UP, SPEED_UP, ATTACK_TIME_DOWN, STUN, SLEEP, ROOT_PHYSICALLY, SILENCE, ...) and `abnormalLvl` (stack order). Adding an effect whose group already exists: if `new.abnormalLvl >= existing.abnormalLvl` the old one is replaced (and the timer restarts); otherwise the new one is **rejected** silently. `abnormalType = NONE` skills stack with everything but replace their own skill id. This is why Shield lvl 3 (PD_UP lvl 3) overrides Shield lvl 1 but a lvl 1 recast cannot overwrite a lvl 3.

**Durations.** Standard 2nd-class buffs 1200 s (20 min), 1800 s with enchant +30 Time; songs/dances 120 s (300 s in later chronicles); debuffs 5-60 s. Debuff duration is reduced by the target's stat bonus (`time / basicPropertyBonus`), resistances and level difference, floored at 50%: `time = ceil(clamp(time * resMod * lvlBonusMod * elementMod / statMod, time * 0.5, time))`. Skill Mastery (crit cast) doubles buff duration.

**Dispel / cancel (`calcCancelEffects`).** Cancel-type skills remove up to `max` buffs scanning from the *newest* backwards, each with `rate / resMod` where `resMod = 1 - (CANCEL_VULN + CANCEL_PROF)/100`, and per-buff `rate *= 1 + (cancelMagicLvl - buffMagicLvl)/100`, clamped to the skill's min/max chance. Debuff cleansing (Cure skills) removes debuffs with plain `rnd(100) <= rate`. `canBeStolen`/`irreplaceableBuff` flags protect certain buffs. Death removes all effects except `isStayAfterDeath`; class switch removes all except `isStayOnSubclassChange`.

**Status effects list (abnormal type -> trait -> what it does):**

| Effect | Trait (resist) | Saving stat | Behaviour | Typical base rate / duration |
|---|---|---|---|---|
| Stun | SHOCK | CON | cannot move/act; broken by damage? No (retail stun is not broken by damage) | Stun Attack 50% / 9 s; Shield Stun 80% / 9 s |
| Sleep | SLEEP | MEN | cannot act; **any damage breaks it** | Sleep 80% / 30 s (grants SLEEP defence 100 while active: no re-sleep) |
| Root (hold) | HOLD / ROOT_PHYSICALLY / ROOT_MAGICALLY | MEN | cannot move, can act | Entangle 80% / 15 s |
| Silence | DERANGEMENT (magic) / PHYSICAL_BLOCKADE | MEN | cannot cast magic / cannot use physical skills | Silence 80% / 30 s |
| Fear | DERANGEMENT | MEN | runs away from caster, cannot act | 30 s |
| Paralysis | PARALYZE | CON | cannot move/act, not broken by damage | Medusa 60% / 20 s |
| Poison / Bleed | POISON / BLEED | CON | DoT per tick, stackable by abnormal level | Poison 10 dmg/2 s for 30 s |
| Slow | - (SPEED_DOWN) | DEX/MEN | movement speed x0.7 etc. | |
| Petrify / Turn to stone | TURN_STONE | CON | paralysis + invulnerability | |
| Mutation / Confusion | DERANGEMENT | MEN | random movement | |
| Disarm | DISARM | - | weapon unequipped | |

**Land-rate formula (H5 `calcEffectSuccess`):**

```
if skill.activateRate == -1 or basicProperty == NONE: always lands   // e.g. buffs
magicLevel = skill.magicLevel (or target.level + 3 if unset)
baseMod   = ((magicLevel - target.level + 3) * skill.lvlBonusRate + activateRate + 30) - targetBaseStat
elementMod = attributeBonus(attacker, target, skill)               // 2.9
traitMod   = generalTraitBonus(attacker, target, skill.trait)      // 1 + attackTrait - defenceTrait, clamp 0.05..2.0; 0 if target is immune
buffDebuffMod = 1 + target.DEBUFF_VULN / 100
mAtkMod    = isMagic ? sqrt(mAtk * (blessedSps ? 4 : 1)) / target.mDef * 11 : 1
rate       = baseMod * elementMod * traitMod * mAtkMod * buffDebuffMod
finalRate  = traitMod > 0 ? clamp(rate, skill.minChance (default 1), skill.maxChance (default 99)) : 0
lands      = finalRate > rnd(100)
```

Worked example: Stun Attack lvl 15 (`magicLvl` 36, `activateRate` 50, `lvlBonusRate` 1, CON) on a level 36 target with CON 43: `baseMod = (3*1 + 50 + 30) - 43 = 40` -> 40% before traits. Against CON 47 (Orc) 36%. Sleep lvl 12 (`magicLvl` 40, rate 80, `lvlBonusRate` 2, MEN) by a caster with mAtk 500 vs mDef 200 on a level 40 target with MEN 25: `baseMod = (3*2 + 80 + 30) - 25 = 91`, `mAtkMod = sqrt(500)/200*11 = 1.23` -> clamps to 99%. Versus a MEN 42 Orc Shaman with mDef 400: `baseMod 74 * 0.61 = 45%`.

Interlude (L2J C6 `calcEffectSuccess`) is the formula most "classic" players remember: `rate = effectPower * (1 / saveVs_statBonus)`, then for magic `rate *= 14 * sqrt(ssMod * mAtk) / mDef`, then a resistance modifier clamped to 0.5..0.9 when the target has vulnerability data, then `rate += levelDelta` where `levelDelta` is the level gap `(attacker/magic level - target level)` rounded away from zero to a multiple of 5, then clamp to `[minChance, maxChance]` (defaults 1..99; many servers use 5..95 or 10..90).

### 2.7 Targeting and aggro

L2J `L2Attackable` keeps `_aggroList: Map<attacker, AggroInfo{hate, damage}>`.

- **Hate from damage:** `addDamageHate(attacker, damage, aggro = damage * 100 / (mobLevel + 7))`. A level-20 mob gains 3.7 hate per point of damage; a level-80 mob 1.15. Hate caps at 999,999,999.
- **Hate from skills:** every skill has `effectPoint`; using a skill on/near a mob adds `|effectPoint|` hate (and for heals/buffs on the mob's target, heals add hate proportional to the heal to all mobs attacking the healed target). Aggression skills (Hate, Aggression, Provoke/"Challenge for Fate") add large flat hate (`Aggression` effect: `power` hate) and Provoke-style AoEs (POINT_BLANK, 500-900 range, up to 10 targets) also apply a short `REAL_TARGET` (target lock) abnormal for 10 s.
- **Most hated:** the target with the highest `hate` (ties keep current); dead/invisible/unknown targets are dropped (`checkHate`). Taunt swaps are immediate because hate is compared each AI tick.
- **Reduce hate:** `reduceHate(target, amount)`; `stopHating` on death/teleport. Agro decays only when the mob loses track.
- **Social aggro (clan call):** a mob under attack looks for NPCs within `clanHelpRange + collision` whose template `clans` overlap its own and whose AI is idle/active and within 600 z; they receive `EVT_AGGRESSION` with 1 hate (players) or inherit the caller's hate for the caller's target (NPC vs NPC).
- **Auto-aggro:** `autoAttackCondition`: target within `aggroRange` (template value, typically 300-1000), visible (geodata LOS), not in a peace zone, not in Silent Move unless the mob is a raid; mobs ignore players in fake death; guards aggro only players with karma > 0.
- **Leash / give up:** `MAX_ATTACK_TIMEOUT = 120 s` of attacking without being able to hit resets to ACTIVE and drops hate; `returnHome` teleports a mob back when it drifts beyond `MAX_DRIFT_RANGE` from spawn (L2J default 300 while idle); random walk 1/30 chance per think tick when idle.

### 2.8 Death and resurrection

**XP loss (`playerXpPercentLost.xml`, H5):** percentage of the XP *span of the current level* (`exp(lvl+1) - exp(lvl)`), not of total XP. 10.0% at level 1 falling by 0.125 per level to 4.0% at 49; 4.0% from 49 to 75; 2.5% at 76, 2.0% at 77, 1.5% at 78, 1.0% from 79 to 85. Interlude used brackets: <20: 10%, 20-39: 7%, 40-74: 4%, 75-80: 2%. Killed by a player during clan war or while a festival participant: loss / 4. Chaotic (karma > 0): `* RateKarmaExpLost` (1.0 default). Dying can de-level (`Delevel = True`). Dying to a monster (or to a player while chaotic) has `DeathPenaltyChance = 20` % to add a level of Death Penalty (skill 5076, max 15): lvl 1 is x0.9 P.Atk/M.Atk/atk speed/cast speed, x0.91 P.Def/M.Def, -5 accuracy/evasion, x0.95 speed; lvl 15 is x0.25 / x0.4 / -50 / x0.6. Not applied in PvP/siege zones, under level 10 with the Lucky passive, or after a Blessed resurrection.

**Resurrection restore (`Resurrection{power}`, `calculateSkillResurrectRestorePercent`):** Resurrection skill lvl 1-9 restores 0 / 20 / 30 / 40 / 50 / 55 / 60 / 65 / 70 % of lost XP (magic level 20..74; Mass Resurrection same ladder); Scroll of Resurrection 0% (Interlude) / 10%; Blessed Scroll 100%. The restore percent is multiplied by the caster's WIT bonus, capped at +20 points over base and at 90% total (100% stays 100%). Respawn to town restores 65% HP, 0% MP/CP (`RespawnRestoreHP = 0.65`). Spawn protection 600 game ticks (60 s) after respawn.

### 2.9 Elemental attributes

Six attributes: fire, water, wind, earth, holy, dark (unholy); opposing pairs fire/water, wind/earth, holy/dark. Attack attribute comes from the weapon (or skill `attributeType` + `attributePower`, e.g. Wind Strike +20 wind) and is a single element; defence attribute is per element, summed from armor. Introduced in Gracia Final; Interlude only had "fire/water/wind/earth resistance %" via traits.

`calcAttributeBonus` (H5) is a bracketed quadratic, not a simple ratio:

```
if skill: attack = attacker.attackElementValue + skill.attributePower (only if skill element == weapon element, else 1.0)
defence = target.defenseElementValue[attackElement]
if attack <= defence: return 1.0                      // never below 1.0 for the attacker side
pick (attackMod, defenceMod) coefficient pair by brackets of attack (>=450, >=300, >=150, >=-99) and defence (>=450, >=350, >=300, >=150, >=0)
   e.g. attack >=150 & defence >=0 : 0.25 / 0.2894 ;  attack >=300 & defence >=150 : 0.129 / 0.1473 ; both >=450 : 0.06909 / 0.078
diff = attack - defence
(min, max) = diff >= 300 ? (-50, 100) : diff >= 150 ? (-50, 70) : (-50, 40)   // percent caps
attackMod  = ((attack + 100)^2 / 144) * attackMod
defenceMod = ((defence + 100)^2 / 169) * defenceMod
result = 1 + clamp(attackMod - defenceMod, min, max) / 100
PvP: result = max(result, 1.0)
```

Rule of thumb from the community: roughly +1% damage per 1 point of (attack - defence) up to ~+20% at the first bracket, with diminishing returns; a 150 attack vs 0 defence weapon gives about +20%, 300 vs 0 about +40%, and the hard cap is +100% (diff >= 300 and high attack). Defence over attack gives no penalty to the attacker in PvP and only damage reduction via the `min` caps in PvE.

### 2.10 PvP rules

L2J `pvp.properties`: `PvPVsNormalTime = 120000` ms flag after attacking a non-flagged player (purple name), `PvPVsPvPTime = 60000` ms when both are flagged; the flag timer restarts on every hostile action and the flag drops when it expires. Retail Classic shortened the flag to 30 s. Attacking (not killing) a non-flagged player flags you; the victim does not flag unless they fight back.

**Karma (`calculateKarmaGain`):** killing a non-flagged, non-chaotic player gives `karma = ((pkCount * 0.5) + 1) * 60 * 12` for pkCount < 99 (720 for the first PK, 1080 for the second, ... 35,640 at 98), `((pkCount * 0.125) + 37.75) * 60 * 12` for 99..179, 43,200 flat from 180 PKs. Killing a summon gives at most 10,800. `pkCount` increments only for player kills. Karma > 0 = chaotic (red name); guards and friendly NPCs attack you; you cannot use gatekeepers (`KarmaPlayerCanUseGK = False`), can shop/trade/warehouse by default config, cannot enter peace zones safely only if `KarmaPlayerCanBeKilledInPeaceZone`.

**Karma decay (`calculateKarmaLost`):** every XP gain from monsters removes `karmaLost = |exp| / karmaMultiplier(level) / 30`, where `karmaMultiplier` is `pcKarmaIncrease.xml` (0.77 at level 1, 5.12 at 20, 13.16 at 40, 21.5 at 60, 31.4 at 76, 35.25 at 85). Dying also reduces karma (lost XP goes through the same function). There is no time-based decay.

**Chaotic item drop (`L2PcInstance.doDie` drop block):** a chaotic player with `pkKills >= MinimumPKRequiredToDrop (6)` who dies (in a non-PvP zone, killer may be NPC or player) has `KarmaRateDrop = 40` % to drop items: each equipped armor piece is rolled at `KarmaRateDropEquip = 40` %, equipped weapon at `KarmaRateDropEquipWeapon = 10` %, inventory items at `KarmaRateDropItem = 50` %, up to `KarmaDropLimit = 10` items; adena, quest items, shadow/time-limited, non-droppable and pet-control items are excluded. Normal players never drop to players; `PlayerRateDrop = 0` to NPCs by default. Retail thresholds varied (4+ PKs in some chronicles, 31+ in Classic).

**Zones (L2J `ZoneId`):** PEACE (no attacks, no mob aggro unless configured), PVP (battle zone: free PvP, no flag/karma, no XP loss debuff), SIEGE (during siege: PvP rules by alliance, resurrection restrictions), TOWN, CLAN_HALL, CASTLE, FORT, MOTHER_TREE (Elf regen), NO_SUMMON_FRIEND, NO_STORE, NO_ITEM_DROP, JAIL, LANDING/NO_LANDING, WATER, SWAMP (slow), DANGER_AREA, HQ, SCRIPT.

### 2.11 Pets and servitors

- **Pets** (Wolf, Hatchling, Strider, Baby pets): spawned from a control item (collar), level independently with their own XP table and `PetLevelData` per level (HP, P.Atk, P.Def, `max_meal`, `consume_meal_in_normal`/`_in_battle`, `soulshot_count`). Feeding: a `FeedTask` runs every 10 s and subtracts the per-level consumption (Wolf: 2 normal / 2 battle at level 1 with `max_meal` 248); when `currentFed < hungry_limit(55%) * max_meal` the pet is hungry and auto-eats matching food from its inventory (Wolf food item 2515); at 0 it becomes uncontrollable and eventually leaves/dies. XP: a pet gets XP as `get_exp_type` percent of what it would earn alone (Wolf 73%) only from its own kills/damage share; the owner's XP is not reduced by a pet.
- **Servitors** (summoner class summons: Kat the Cat, Shadow, Unicorn...): cast with `itemConsumeId` crystals (Summon Kat the Cat lvl 5+: 7..12 D-crystals up front, then `consumeItemCount` 1-2 crystals every 14 intervals over the summon's `lifeTime` of 3600 s; Interlude lifetime 1200 s), no feeding. The owner's XP from kills is multiplied by the servitor's `expMultiplier` penalty (Kat the Cat takes 30% at low levels falling to 18%; Mew/Silhouette 90%; Kai/Soulless 10% -> 5%; Feline Queen/Nightshade 5%). Summoner skills (Servitor Heal/Recharge/Haste/Shield) target the servitor; `Transfer Pain` moves part of the owner's damage to it. Interlude-era servitors also consumed owner MP over time on some servers (L2J "servitor MP consumption" is a config, not retail).

## 3. Design decisions for Nightfall

### 3.1 Chosen formulas

Start from H5 L2J and simplify where the simplification removes an opaque constant without changing feel. Everything below is tunable in `data/combat.toml`.

| Area | Nightfall formula | Why |
|---|---|---|
| Normal attack | `dmg = K_PHYS * pAtk * ss / pDef * prox * rnd(1 +/- weapon.rnd%)` with `K_PHYS = 76`; crit `*= 2 * critDmgMult`; shield: `pDef += shldDef` on block, `dmg = 1` on perfect block (10% of blocks) | Same as H5; the 70 vs 76 debate is a tuning knob. |
| Physical skill | `dmg = K_SKILL * (power + pAtk * ss) / pDef * prox`, `K_SKILL = 77`; skill crit chance `= STR_bonus * skill.critChance`, crit `*= 2` | H5. Soulshots double pAtk only. |
| Blow | as skill but always crit and `sideMod` 1 / 1.5 / 2 on blow chance | H5 |
| Position | front 1.0, side 1.1, back 1.2 (damage); hit bonus 0 / +5% / +10% | H5 values, marked for tuning |
| Hit chance | `clamp((80 + 2*(acc - eva)) * posBonus, 20, 98)%` | H5/Interlude identical |
| Crit rate | `baseCrit * DEX_bonus * 10` permille, cap 50%; magic crit `WIT_bonus * 10` permille, cap 20% | H5 caps |
| Magic damage | `K_MAG * sqrt(mAtk * sps) / mDef * power`, `K_MAG = 91`; mcrit `* 3` PvE, `* 2.5` PvP | H5 |
| Magic resist | `success = 100 - 1.3^(targetLvl - magicLvl)`; on fail: half damage if gap <= 9 else 1 damage | H5; keep `magicLvl` on every skill |
| Attack interval | `500000 / pAtkSpd` ms, hit at 50% | H5 |
| Cast time | `hitTime * 333 / castSpd`, min 500 ms; sps 0.6x | H5 |
| Debuff land rate | H5 `calcEffectSuccess` verbatim, clamp 10..90 by default (per-skill override) | Deterministic given level/stat; 10..90 avoids the "never lands / always lands" extremes of 1..99 |
| Debuff duration | `clamp(base * traitMod * lvlMod / statMod, 0.5*base, base)` | H5 |
| Elements | H5 bracketed quadratic, implemented as a lookup table generated at build time (attack -99..600 x defence -99..600 step 1 -> f32) | Exact, O(1), tunable by swapping the table |
| Aggro | `hate += dmg * 100 / (mobLvl + 7)`, skill `effectPoint` adds flat hate; heals add `heal * 100 / (mobLvl + 7)` to all mobs hating the healed target | H5 |
| XP loss | table copy of `playerXpPercentLost.xml` capped at our level cap; Death Penalty debuff 20% chance, max 5 levels in MVP | H5 |
| Resurrection | per-skill `restorePercent`, WIT bonus, cap 90; Blessed 100 | H5 |
| PvP flag | 60 s after hitting a non-flagged player, 30 s when both flagged, timer refreshes on each hostile act | Between Classic (30 s) and L2J (120 s) |
| Karma | H5 `calculateKarmaGain` and `calculateKarmaLost` verbatim; drop rules with `MinimumPKRequiredToDrop = 5` and the same per-slot percentages | Proven numbers |
| Buff slots | 20 (+4 via a passive), 12 song/dance, 8 debuff slots (oldest debuff replaced), replace oldest on overflow | H5 + retail debuff cap |

Simplifications relative to L2: no over-hit, no cubics, no NPC level-gap damage penalty in MVP (replace with Phase 6 "grey mob" XP rules), no PvE-vs-PvP separate damage multipliers except the magic crit, no soulshot grades (a single "soulshot" item doubling pAtk, grades return in Phase 4).

### 3.2 Server tick and combat loop

- **Tick:** 100 ms (`TICK_MS = 100`), matching L2J's 10 ticks/s. All combat timers (attack end, hit landing, cast end, reuse, effect ticks, flag expiry) are stored as absolute tick numbers (`u64`) in the zone's clock, never as wall time.
- **Ownership:** each zone (Phase 0) is a single-threaded simulation task owning all entities inside it; intents arrive via an `mpsc` channel, events leave via a broadcast channel per zone. No locks on entity state. Cross-zone effects (teleport while flagged) are messages.
- **Attack pipeline:**
  1. Client sends `AttackIntent{target_id}` (once; the server auto-repeats like L2's auto-attack).
  2. Validate: target exists, alive, attackable (zone rules, flag/karma, party/clan, not self), in range `atkRange + collision_a + collision_t + 20` slack, LOS, attacker not stunned/sleeping/casting/dead, weapon equipped and allowed.
  3. If out of range: switch intention to `MoveToThenAttack` (server pathing, Phase 6); client only renders.
  4. Schedule: `attack_end = now + 500000 / pAtkSpd`; `hit_at = now + (attack_end - now) / 2` (dual: two hits at 25%/75%; polearm: one roll per target at 50%); bow adds `reuse` after `attack_end` and consumes MP on start.
  5. At `hit_at`: re-validate target alive and within `range * 1.5` (L2 lets a hit land if the target walked out after the swing started), roll miss -> shield -> crit -> damage, apply, add hate, broadcast `AttackResult`.
  6. At `attack_end`: if intent still `Attack` and target alive, go to 2.
- **Cast pipeline:** `SkillCastIntent{skill_id, target_id?, ground?}` -> validate (learned, reuse ready, MP/HP/items for `mpConsume1`, conditions, range) -> consume `mpConsume1` -> broadcast `SkillCastStarted{cast_ms}` -> `cast_end = now + castTime` -> on damage during cast roll `calcAtkBreak`; on interrupt broadcast `SkillCastCancelled` -> at `cast_end`: re-check range vs `effectRange`, consume `mpConsume2` + items, resolve targets by `affectScope`, run effect list per target (hit roll for physical, resist roll for magic, land roll for debuffs), start reuse `reuse_until = cast_end + reuseDelay`, apply `coolTime` lock, broadcast `SkillCastResult`.
- **Effect ticking:** continuous effects are entries in a per-entity `BinaryHeap<(next_tick, effect_id)>`; DoTs tick every `effect.tick_interval` (default 1 s); expiries are the same heap. Buff list changes emit `StatusApplied/StatusRemoved`.
- **Client intent validation:** the client never sends damage, hit results, positions of others, or timings. Rate-limit intents (max 1 attack intent / 100 ms, 1 cast / 100 ms); reject intents referencing entities not in the client's known-list; range checks use server positions; a cast intent while a cast is in progress replaces it only if the current cast is interruptible (`hitCancelTime`). Movement intent during a cast cancels the cast (L2 behaviour) unless the skill is a toggle.
- **RNG:** one `ChaCha8Rng` per zone, seeded from server seed + zone id + tick at spawn; all rolls go through `Rng::roll_permille()` so replays/tests are deterministic.

### 3.3 Skill data format

TOML, one file per skill, mirroring L2J attributes but with explicit types and per-level arrays (`levels = N`, arrays of length N or scalars).

```toml
# data/skills/stun_attack.toml
id = 100
key = "stun_attack"
display_name = "Stun Attack"
levels = 15
operate = "active_instant_continuous"      # instant | instant_continuous | continuous | channel | passive | toggle
is_magic = false
target = "enemy"                            # self | target | enemy | enemy_only | friend | dead_player | ground
scope = "single"                            # single | point_blank | range | fan | party | pledge
cast_range = 40
effect_range = 400
hit_time_ms = 1080
cool_time_ms = 720
reuse_ms = 3000
hit_cancel_ms = 500
magic_level = [18,19,20,22,23,24,26,27,28,30,31,32,34,35,36]
mp_consume_end = [19,19,20,21,21,22,24,25,26,27,28,29,31,32,33]
effect_point = [-96,-100,-104,-111,-115,-119,-128,-132,-136,-145,-150,-154,-164,-169,-173]
next_action_attack = true
requires_weapon = ["blunt"]

[debuff]                                    # presence marks the skill as a debuff
abnormal_type = "stun"
abnormal_level = 1
duration_s = 9
activate_rate = 50
basic_property = "con"
level_bonus_rate = 1
trait = "shock"

[[effects]]
kind = "physical_attack"
power = [36,39,42,49,53,57,66,71,77,88,94,101,115,123,131]

[[effects]]
kind = "stun"

[[enchant_routes]]
route = "power"
levels = 30
overrides = { magic_level = "76,76,76,77,...", power = "794,799,803,...,928" }
```

Buff example (`shield.toml`): `[buff] abnormal_type = "p_def_up", abnormal_level = [1,2,3], duration_s = 1200`, `[[effects]] kind = "stat_mul", stat = "p_def", value = [1.08, 1.12, 1.15]`.

Rust model:

```rust
pub struct SkillDef {
    pub id: SkillId, pub key: String, pub levels: u8,
    pub operate: OperateType, pub is_magic: bool,
    pub target: TargetType, pub scope: AffectScope, pub affect_limit: Option<(u8,u8)>,
    pub cast_range: u16, pub effect_range: u16,
    pub hit_time_ms: PerLevel<u32>, pub cool_time_ms: u32, pub reuse_ms: PerLevel<u32>, pub hit_cancel_ms: u32,
    pub magic_level: PerLevel<u8>, pub mp_start: PerLevel<u16>, pub mp_end: PerLevel<u16>, pub hp_cost: PerLevel<u16>,
    pub item_cost: Option<ItemCost>, pub effect_point: PerLevel<i32>,
    pub abnormal: Option<Abnormal>,          // type, level, duration, is_debuff, land-rate inputs, trait
    pub element: Option<(Element, i16)>,
    pub effects: Vec<EffectDef>,             // enum: PhysicalAttack{power}, MagicalAttack{power, shield_pct}, FatalBlow{power, chance}, Stun, Sleep, Root, StatMul{stat,val}, StatAdd{..}, Dot{..}, Heal{..}, Resurrect{pct}, Summon{..}, Aggression{hate}, Dispel{..}
    pub enchant_routes: Vec<EnchantRoute>,
    pub conditions: Vec<Condition>,
}
pub enum PerLevel<T> { Same(T), ByLevel(Vec<T>) }  // serde untagged
```

### 3.4 Proto messages for combat

Realtime traffic rides the Phase 0 real-time channel: a WebSocket at `/ws` carrying one protobuf `ClientMessage` or `ServerEvent` per binary frame (see `00-foundations.md` §3.2; browsers cannot client-stream over gRPC-Web, so this is not a tonic bidi RPC). The oneof envelopes below are the frame payloads.

```proto
message ClientMessage {
  uint64 client_tick = 1;
  oneof intent {
    AttackIntent attack = 10;
    SkillCastIntent cast = 11;
    StopIntent stop = 12;              // cancel auto-attack / cast
    TargetIntent target = 13;          // soft target selection (for UI sync only)
    MoveIntent move = 14;              // Phase 0/6
  }
}
message AttackIntent { uint64 target_id = 1; }
message SkillCastIntent { uint32 skill_id = 1; uint64 target_id = 2; Position ground = 3; bool force_pvp = 4; }

message ServerEvent {
  uint64 server_tick = 1;
  oneof event {
    AttackResult attack_result = 10;
    SkillCastStarted cast_started = 11;
    SkillCastResult cast_result = 12;
    SkillCastCancelled cast_cancelled = 13;
    StatusApplied status_applied = 14;
    StatusRemoved status_removed = 15;
    VitalsChanged vitals = 16;
    EntityDied died = 17;
    EntityRevived revived = 18;
    PvpStateChanged pvp_state = 19;
    AggroTargetChanged aggro = 20;      // NPC switched target (for threat UI)
    SkillReuse reuse = 21;              // reuse_until tick for the owner
  }
}

enum HitFlag { HIT_FLAG_NONE = 0; HIT_FLAG_MISS = 1; HIT_FLAG_CRITICAL = 2; HIT_FLAG_SHIELD_BLOCK = 4; HIT_FLAG_PERFECT_BLOCK = 8; HIT_FLAG_SOULSHOT = 16; HIT_FLAG_BACKSTAB = 32; }
message Hit { uint64 target_id = 1; uint32 damage = 2; uint32 flags = 3; }  // flags = bitwise HitFlag
message AttackResult { uint64 attacker_id = 1; repeated Hit hits = 2; uint32 attack_ms = 3; uint32 hit_at_ms = 4; }

message SkillCastStarted { uint64 caster_id = 1; uint32 skill_id = 2; uint32 skill_level = 3; uint64 target_id = 4; Position ground = 5; uint32 cast_ms = 6; uint32 reuse_ms = 7; }
message SkillCastCancelled { uint64 caster_id = 1; uint32 skill_id = 2; CancelReason reason = 3; }
enum CancelReason { CANCEL_REASON_UNSPECIFIED = 0; CANCEL_REASON_INTERRUPTED = 1; CANCEL_REASON_MOVED = 2; CANCEL_REASON_OUT_OF_RANGE = 3; CANCEL_REASON_TARGET_LOST = 4; CANCEL_REASON_RESOURCES = 5; }

enum SkillOutcome { SKILL_OUTCOME_UNSPECIFIED = 0; SKILL_OUTCOME_HIT = 1; SKILL_OUTCOME_MISS = 2; SKILL_OUTCOME_RESISTED = 3; SKILL_OUTCOME_HALF = 4; SKILL_OUTCOME_LANDED = 5; SKILL_OUTCOME_IMMUNE = 6; }
message SkillTargetResult { uint64 target_id = 1; SkillOutcome outcome = 2; uint32 damage = 3; uint32 heal = 4; bool critical = 5; bool shield = 6; }
message SkillCastResult { uint64 caster_id = 1; uint32 skill_id = 2; uint32 skill_level = 3; repeated SkillTargetResult targets = 4; }

message StatusApplied { uint64 target_id = 1; uint32 skill_id = 2; uint32 skill_level = 3; string abnormal_type = 4; uint32 abnormal_level = 5; uint32 duration_ms = 6; uint64 caster_id = 7; bool is_debuff = 8; }
message StatusRemoved { uint64 target_id = 1; uint32 skill_id = 2; RemoveReason reason = 3; }
enum RemoveReason { REMOVE_REASON_UNSPECIFIED = 0; REMOVE_REASON_EXPIRED = 1; REMOVE_REASON_DISPELLED = 2; REMOVE_REASON_REPLACED = 3; REMOVE_REASON_BROKEN = 4; REMOVE_REASON_DEATH = 5; REMOVE_REASON_SLOT_OVERFLOW = 6; }

message VitalsChanged { uint64 entity_id = 1; uint32 hp = 2; uint32 max_hp = 3; uint32 mp = 4; uint32 max_mp = 5; uint32 cp = 6; uint32 max_cp = 7; }
message EntityDied { uint64 entity_id = 1; uint64 killer_id = 2; int64 exp_lost = 3; uint32 death_penalty_level = 4; repeated uint64 dropped_item_ids = 5; }
message EntityRevived { uint64 entity_id = 1; uint32 restore_percent = 2; Position at = 3; }
message PvpStateChanged { uint64 entity_id = 1; bool flagged = 2; uint32 flag_remaining_ms = 3; int32 karma = 4; uint32 pk_count = 5; uint32 pvp_count = 6; }
message AggroTargetChanged { uint64 npc_id = 1; uint64 target_id = 2; }
message SkillReuse { uint32 skill_id = 1; uint32 reuse_remaining_ms = 2; }
```

## 4. Data model

Server-side entities (zone-owned, not persisted except where noted):

```rust
pub struct Combatant {                 // component on players, NPCs, pets, servitors
    pub derived: DerivedStats,         // Phase 1 output, recomputed on equipment/buff change (cached, dirty flag)
    pub hp: f32, pub mp: f32, pub cp: f32,
    pub attack: Option<AttackState>,   // target, attack_end_tick, hit_ticks: SmallVec<[u64; 2]>, bow_reuse_until
    pub cast: Option<CastState>,       // skill, level, target, cast_end_tick, interruptible_until, mp_start_paid
    pub reuse: HashMap<SkillId, u64>,  // persisted for players (StoreSkillCooltime)
    pub effects: EffectList,           // buffs: VecDeque<Effect>, songs: VecDeque, debuffs: VecDeque, by_type: HashMap<AbnormalType, EffectId>, ticks: BinaryHeap
    pub flags: StateFlags,             // STUNNED | SLEEPING | ROOTED | SILENCED | FEARED | PARALYZED | INVUL | DEAD
    pub element: ElementState,         // attack: (Element, i16), defence: [i16; 6]
    pub traits: TraitState,            // attack/defence trait arrays [f32; TraitType::COUNT], invul set
}
pub struct Effect { pub id: EffectId, pub skill: SkillId, pub level: u8, pub caster: EntityId, pub abnormal: AbnormalType, pub abnormal_level: u8, pub ends_at: u64, pub next_tick: Option<u64>, pub mods: SmallVec<[StatMod; 4]> }
pub struct AggroList { pub entries: HashMap<EntityId, AggroInfo>, pub most_hated: Option<EntityId> }
pub struct AggroInfo { pub hate: u64, pub damage: u64, pub last_seen_tick: u64 }
pub struct PvpState { pub flag_until: Option<u64>, pub karma: i32, pub pk_count: u32, pub pvp_count: u32 }   // persisted
pub struct LearnedSkill { pub id: SkillId, pub level: u8, pub enchant: Option<(RouteId, u8)> }                 // persisted
```

Persistence (Phase 0 store): learned skills, SP, reuse timers > 5 s remaining, buffs with > 10 s remaining (restore on login like L2's `StoreSkillCooltime`), PvP state, death penalty level, pet level/XP/fed, servitor none.

## 5. Interfaces

- **Realtime stream** as in 3.4: the only path for combat. The server batches events per tick per client, in zone-visibility scope (Phase 0 interest management).
- **gRPC unary** for out-of-combat skill management: `ListLearnableSkills(character_id)` (tree walk with level/SP checks), `LearnSkill(character_id, skill_id)` (consumes SP, item), `EnchantSkill(character_id, skill_id, route)` (consumes adena/SP/codex, rolls chance by character level from the route table), `GetSkillCatalog()` (static, versioned, cached by client).
- **Server-internal events** (Rust channels, not proto): `DamageDealt`, `Killed`, `HateChanged`, `EffectApplied`, consumed by AI (Phase 6), quests (Phase 6), telemetry (Phase 9).
- **Client needs:** the static skill catalog (icons, names, ranges, cast times for cast bars), its own `SkillReuse` timers, vitals of itself/party/target, and every event above for entities in view. Not needed: formulas, hidden stats of others.

## 6. Rust implementation notes

```
combat/
  mod.rs            // CombatSystem::tick(zone, now) entry point; wiring of sub-systems
  clock.rs          // Tick type (u64 newtype), TICK_MS, ms<->ticks helpers
  formulas.rs       // pure fns: phys_damage(), skill_phys_damage(), blow_damage(), magic_damage(), hit_chance(), crit_roll(),
                    //           shield_roll(), magic_success(), effect_land_rate(), effect_duration(), attribute_bonus(), atk_break()
  attack.rs         // AttackState machine: begin(), on_hit_tick(), on_end_tick(); weapon-type dispatch (simple/dual/pole/bow)
  cast.rs           // CastState machine: begin(), interrupt(), on_end_tick(); target resolution by scope
  skill/
    def.rs          // SkillDef, EffectDef, PerLevel<T>, OperateType, TargetType, AffectScope
    loader.rs       // TOML -> SkillRegistry, validation (array lengths == levels, ranges, trait names, abnormal types)
    effects.rs      // EffectHandler trait + impls (PhysicalAttack, MagicalAttack, FatalBlow, Stun, Sleep, StatMul, Dot, Heal, Resurrect, Aggression, Dispel, Summon)
    learn.rs        // skill tree queries, SP spending, enchant tables (reuse Phase 2 ClassRegistry.ancestors)
  status.rs         // EffectList: add() with stacking/slot rules, expire(), tick DoTs, break-on-damage (sleep), flags recompute
  element.rs        // generated lookup table + Element enum
  aggro.rs          // AggroList, add_damage_hate(), most_hated(), social_call() hooks
  death.rs          // on_death(): exp loss table, death penalty roll, drops (karma rules), effect cleanup; resurrect()
  pvp.rs            // flag timers, karma gain/lost, pk counters, zone checks
  summon.rs         // Pet feeding task (every 100 ticks), servitor lifetime/upkeep, exp penalty
  validate.rs       // intent validation (range, LOS, state flags, rate limits)
```

- `formulas.rs` is `#![no_std]`-style pure code with `&dyn Rng` injected; property tests (`proptest`) assert bounds (hit chance in 20..98, damage >= 1, land rate in min..max) and golden tests pin a handful of worked examples from section 2 so future tuning is visible in diffs.
- Zone task loop: `loop { tick += 1; drain intents; combat.tick(); ai.tick(); flush events; sleep_until(next_tick) }` on a dedicated `tokio` task per zone; heavy zones can be moved to `std::thread` with `tokio::task::block_in_place` if the 100 ms budget is missed. Use `Instant`-based scheduling with drift correction, not `sleep(100ms)`.
- Time ordering inside a tick: expiries -> DoT ticks -> cast completions -> attack hits -> new intents -> AI. This makes "stun landed on the same tick as the swing" deterministic (stun wins).
- Hot data is `Vec<Combatant>` indexed by a generational `EntityId`; `EffectList` uses `SmallVec`/`VecDeque` to avoid per-buff allocations.
- Crates: `rand` + `rand_chacha`, `smallvec`, `bitflags`, `serde`/`toml`, `proptest` (dev), `criterion` (bench the damage pipeline: target < 2 us per hit).
- Ordering of float math matters for determinism across machines only if we ever lockstep; we do not, so plain `f32` is fine. Round damage with `floor` and clamp to `>= 1`.

## 7. Client implications

- Render from events only: play swing animation on `AttackResult` using `attack_ms`/`hit_at_ms` to time the impact; show floating numbers per `Hit` with miss/crit/block styling.
- Cast bar from `SkillCastStarted.cast_ms`; cancel on `SkillCastCancelled`.
- Buff bar: 20 (+4) buff slots, 12 song/dance, 8 debuff; sort by remaining time; show stack replacement via `REMOVE_REASON_REPLACED`.
- Skill bar: grey-out on `SkillReuse`; local prediction allowed only for the *reuse* countdown.
- Target frame: show flag/karma colour from `PvpStateChanged` (white, purple when flagged, red when chaotic); threat indicator from `AggroTargetChanged`.
- Death screen: `EntityDied.exp_lost`, "To village" button (respawn RPC), accept-resurrection prompt when a `Resurrect` offer event arrives (add `ResurrectOffer{caster_id, restore_percent}` event in implementation).
- Phaser: all hit timing is driven by server ms values so animation speed scales with attack speed (play the swing clip at `clip_ms / attack_ms` rate).

## 8. Open questions

1. 70 vs 76/77: start with H5 constants, but whether we also keep the Interlude asymmetric `+0..10%` random spread or the symmetric weapon `rnd` needs a feel test.
2. Debuff land-rate clamp 10..90 vs L2J 1..99 vs private-server 5..95: needs PvP playtesting with real buffs.
3. Elemental attributes: ship the full bracketed H5 system, or the simpler Interlude-style flat resistance percentages until Phase 4 adds attribute stones? Leaning Interlude-style for MVP with the table ready.
4. Sleep break-on-damage: L2 breaks sleep on any damage including DoT ticks; do DoTs applied before sleep break it on their next tick (retail yes)? Default yes.
5. Debuff slot limit of 8: oldest-replaced or newest-rejected? Retail replaces oldest.
6. Whether the PvP flag timer should differ for ranged/magic initiation (some servers do not flag healers who heal flagged players; L2 does flag them).
7. Servitor upkeep: crystals every 14 intervals (L2) vs continuous owner MP drain (common private-server rule). Data model supports both (`item_cost` and `mp_upkeep`).
8. Pet XP: L2 pets gain XP only from their own kills; a shared-XP model is friendlier but changes the summoner economy.
9. Batching of events over the stream per tick vs per event; depends on Phase 0 transport.

## 9. Sources

- L2J Server (High Five) `Formulas.java` (physical/magic/blow damage, hit/miss, shield, cast time, magic success, effect success, attribute bonus, cancel, abnormal time, karma gain/lost, resurrection restore): https://bitbucket.org/l2jserver/l2j-server-game/src/develop/src/main/java/com/l2jserver/gameserver/model/stats/Formulas.java
- L2J Server `L2Character.java` (attack interval `500000 / pAtkSpd`, bow reuse `reuse * 333 / pAtkSpd`, dual/pole hits, random damage multiplier, front/behind checks), `CharEffectList.java` (slot limits, stacking by abnormal type/level, oldest-buff removal), `L2Attackable.java` and `AggroInfo.java` (hate `damage * 100 / (level + 7)`, most hated), `L2AttackableAI.java` (auto-attack conditions, clan help call, 120 s attack timeout), `L2PcInstance.java` (death XP penalty, death penalty debuff chance, karma drop rules, PvP flag timers), `L2PetInstance.java` (feed task every 10 s, hunger limit), `L2ServitorInstance.java` (exp multiplier, item consume interval, lifetime), `CharStat.java` and `stats/functions/formulas/*.java` (accuracy, evasion, crit, speeds), `TraitType.java`, `AbnormalType.java`, `ZoneId.java`: https://bitbucket.org/l2jserver/l2j-server-game/src/develop/
- L2J Server config defaults `character.properties` (MaxBuffAmount 20, MaxDanceAmount 12, PerfectShieldBlockRate 10, MagicFailures, caps), `pvp.properties` (PvPVsNormalTime 120000, PvPVsPvPTime 60000, MinimumPKRequiredToDrop 6), `rates.properties` (Karma drop rates): https://bitbucket.org/l2jserver/l2j-server-game/src/develop/src/main/resources/config/
- L2J datapack (High Five) skill XML `data/stats/skills/00000-00099.xml` (Power Strike 3, Mortal Blow 16), `00100-00199.xml` (Stun Attack 100), `00200-00299.xml` (Provoke 286), `01000-01099.xml` (Resurrection 1016, Shield 1040, Sleep 1069, Haste 1086), `01100-01199.xml` (Summon Kat the Cat 1111, Wind Strike 1177), `01200-01299.xml` (Wind Walk 1204), `05000-05099.xml` (Death Penalty 5076); `data/stats/hitConditionBonus.xml`; `data/stats/chars/playerXpPercentLost.xml`; `data/stats/chars/pcKarmaIncrease.xml`; `data/enchantSkillGroups.xml`; `data/skillTrees/classSkillTree.xml`; `data/stats/pets/0001.xml` (Wolf): https://bitbucket.org/l2jserver/l2j-server-datapack/src/develop/
- L2J C6 Interlude fork `Formulas.java` (70 constant, Interlude `calcEffectSuccess`, `calcLvlDependModifier`, `calcMagicSuccess`) and `PlayerInstance.java` (Interlude XP loss brackets 10/7/4/2%): https://github.com/Hl4p3x/L2JServer_C6_Interlude
- L2J commit history on magic damage formula (`91 * sqrt(mAtk) / mDef`): https://git.vmatviienko.pp.ua/L2j/l2j-server-game/commit/012691dbf8cb7ed65bfe3de577415680c97cb22a
- L2J commit on Gracia dance/song slot separation (20 buffs + 4 Divine Inspiration, 12 dances): https://git.vmatviienko.pp.ua/L2j/l2j-server-game/commit/93098885324cfa3a98b8fbdf07af6adc2327ea5c
- l2hub.info Warlock and Phantom Summoner skill lists (servitor crystal costs and XP penalties): https://l2hub.info/classes/warlock, https://l2hub.info/il/classes/phantom_summoner
- Lineage 2 Wiki (fandom) Chaotic State (PK count thresholds, karma reduction by hunting): https://lineage2.fandom.com/wiki/Chaotic_State?oldid=4511
- Official Lineage II forum, attribute defence discussion (+/-20% practical range): https://forums.lineage2.com/topic/25004-attribute-defense-against-the-mobs-in-the-new-high-level-locations
- Official Lineage II forum, skill enchant mechanics (route change cost 20%, Codex Mastery): https://forums.lineage2.com/topic/15130-skill-enchants/
- Lineage II Classic Chronicle 1.5 patch notes (30 s PvP flag, classic PK rules): https://www.lineage2.com/news/lineage-ii-classic-launch-patch-notes
- ludo.guide, buffs and debuffs overview and death penalties: https://www.ludo.guide/guide/lineage-ii/skill-management-enchanting/buffs-debuffs, https://www.ludo.guide/guide/lineage-ii/main-walkthrough/death-penalties-resurrection
