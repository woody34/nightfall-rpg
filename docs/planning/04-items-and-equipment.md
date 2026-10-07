# Phase 4: Items and Equipment

## 1. Purpose and scope

Phase 4 delivers everything a character can hold, wear, or consume:

- **Item templates** (static definitions loaded from data files) and **item instances** (rows owned by a character, with enchant level, augmentation, binding, and stack count).
- **Weapons** (type, P.Atk/M.Atk, attack speed, crit rate, random damage, range, shot consumption, MP cost for bows).
- **Armor** (heavy/light/robe tradeoffs, class mastery hooks, set bonuses), **jewelry** (M.Def, resistances), **shields**.
- The **grade system** (No/D/C/B/A/S), expertise levels, and the grade penalty applied when equipping above expertise.
- **Consumables**: soulshots, spiritshots, blessed spiritshots, potions, scrolls.
- **Enchanting** (safe levels, success curves, per-level stat bonus, blessed scrolls, crystallization on failure).
- **Augmentation** (life stones, option pools, skill chance) and **special abilities** (soul crystals).
- **Inventory** (slot and weight limits, overweight penalties), **warehouse** (private, clan), and **freight**.

Explicitly excluded: how items are *acquired* (drops, crafting, shops, trade) — Phase 5; combat formulas that consume the stats defined here — Phase 3; pet items and mounts — Phase 3/6; cosmetic items — Phase 8.

Phase 3 (combat) consumes the stat outputs of this phase (P.Atk, attack speed, shot multipliers) and Phase 5 (economy) consumes the identity model (stackability, tradeability, grade, crystal value).

## 2. Reference: how Lineage 2 does it

Numbers below are taken from the L2J High Five datapack and server source (the most complete open reimplementation), cross-checked against an Interlude-era L2J fork where the two eras differ. L2J's values are themselves reverse-engineered from retail and are a starting point, not canon.

### 2.1 Weapon types and base stats

Every weapon template carries `weapon_type`, `bodypart` (`rhand` = one-handed, `lrhand` = two-handed), `random_damage`, `attack_range`, `soulshots`/`spiritshots` (shots consumed per hit), optional `mp_consume` and `reuse_delay` (bows), and four stats: `pAtk`, `mAtk`, `critRate`, `pAtkSpd`. A survey of all ~4,000 weapon templates in the H5 datapack gives these per-type constants (the base attack speed is the same for every weapon of a type regardless of grade):

| Type (L2J `weapon_type`) | Slot | Base Atk.Spd | Crit rate | Random dmg % | Attack range | Notes |
|---|---|---|---|---|---|---|
| SWORD (1H) | rhand | 379 | 8 | 10 | 40 | |
| SWORD (2H) | lrhand | 325 | 8 | 10 | 40 | Higher P.Atk than 1H of same grade |
| BLUNT (1H) | rhand | 379 | 4 | 20 | 40 | `accCombat +4.75`; mage staves are 2H blunts flagged `is_magic_weapon` |
| BLUNT (2H) | lrhand | 325 | 4 | 20 | 40 | |
| DAGGER | rhand | 433 | 12 | 5 | 40 | Lowest P.Atk per grade, highest speed/crit |
| BOW | lrhand | 293 (227 for the slow "heavy" bows) | 12 | 5 | 500 | `accCombat -3.75`, `mp_consume` per shot, `reuse_delay 1500` ms between shots |
| DUAL | lrhand | 325 | 8 | 10 | 40 | Dual swords; P.Atk between 1H and 2H |
| DUALFIST | lrhand | 325 | 4 | 5 | 40 | `accCombat +4.75` |
| POLE | lrhand | 325 | 8 | 10 | 80 | `accCombat -3.75`; hits up to N targets in an arc (skill 3599) |
| FIST (bare hands) | — | 325 | 4 | 5 | 40 | P.Atk 4 at level 1 |
| RAPIER / ANCIENTSWORD / CROSSBOW (Kamael, Gracia+) | — | 406 / 350 / 303 | 10 / 8 / 10 | 40 / 15 / 10 | | Out of scope for Nightfall's era |

Representative P.Atk / M.Atk by grade (from the datapack):

| Grade | 1H sword | 2H sword | Dagger | Bow | Pole | Dual | Mage staff (2H blunt) |
|---|---|---|---|---|---|---|---|
| No (Short Sword lvl 1) | 8 / 6 | — | — | 16 / 6 (Short Bow) | — | — | — |
| C (Yaksa Mace / Deadman's Staff) | 156 / 83 (mace) | — | — | — | — | — | 152 / 122 |
| B (Sword of Damascus, Great Sword, Bow of Peril, Lance, Arthro Nail) | 194 / 99 | 213 / 91 | ~168 / 99 | 400 / 99 | 194 / 99 | — | — |
| A (Keshanberk*Keshanberk) | — | — | — | — | — | 259 / 107 | — |

The bow's 400 P.Atk vs. the sword's 194 at the same grade is deliberate: the bow fires at 293 speed, pays `mp_consume` 1-6 MP per shot by grade (NG 1, D 2, C 3, B 4, A 5, S 6), has `reuse_delay 1500` ms, and consumes more soulshots per hit at low grades (D bows 6-10, C bows 8-10, B 3, A 2, S 1; melee weapons 2-3 at NG/D/C, 1 at B/A/S). Damage per hit for a normal attack is roughly `76 * P.Atk / P.Def` scaled by random damage, so bows trade sustained DPS and MP for burst and range.

### 2.2 Armor, shields, jewelry

Armor templates carry `armor_type` (HEAVY, LIGHT, MAGIC = robe, or none for jewelry/shields), `bodypart` (`chest`, `legs`, `onepiece` = full body, `head`, `gloves`, `feet`, `lhand` = shield, `neck`, `rear;lear`, `rfinger;lfinger`), `pDef`/`mDef`, optional `maxMp`, and item skills for set/resist effects. Same-grade comparison (B grade):

| Piece | Heavy (Blue Wolf) | Light (Blue Wolf) | Robe (Blue Wolf Tunic / Avadon Robe) |
|---|---|---|---|
| Upper body | Breastplate 166 P.Def | Leather Armor (full body) 202 P.Def | Tunic 83 P.Def +377 MP |
| Lower body | Gaiters 104 P.Def | (included) | Stockings ~52 P.Def +MP |
| Full body alternative | Doom Plate 270 P.Def | — | Avadon Robe 127 P.Def +561 MP |
| Helmet | 66 P.Def (shared across types) | | |

A grade: Majestic Plate 293, Majestic Leather 220, Majestic Robe 147 (+718 MP). Rule of thumb from the data: light body armor ≈ 75% of heavy, robe ≈ 50% of heavy, with robes compensating with raw MP and (via class passives) casting speed and M.Def.

The tradeoff is enforced through **class mastery passives**, not the items:

- **Heavy Armor Mastery** (skill 231, 50 levels, lvl 18-74): `+pDef` from 1.9 to 79.3, conditional on `<using kind="HEAVY"/>`. Enchanted versions add HP regen.
- **Light Armor Mastery**, fighter version (227): `+pDef` 4.2 to 81.3 and `+evasion` 3 to 6, only while wearing light. Rogue version (233): `+pDef` 1.3 to 65.6, `+evasion` 4 to 7, and critical damage received x0.85 / x0.75 / x0.65 by tier.
- **Robe Mastery** (mages): M.Def and casting speed bonuses conditional on MAGIC armor.
- Wearing the wrong type simply loses the passive; in Interlude mages in heavy additionally received casting-speed penalties via the same conditional mechanism.

Jewelry provides **M.Def only** (plus MP on later grades) and is where elemental/status resistances live via `item_skill`:

| Grade | Necklace | Earring | Ring |
|---|---|---|---|
| B (Black Ore) | 72 | ~54 | 36 |
| A (Majestic) | 85 (+33 MP) | 63 (+25 MP) | 42 (+17 MP) |
| S (Tateossian) | 95 (+42 MP) | ~71 | 48 (+21 MP) |

**Shields** (`lhand`) add P.Def and a block rate (`rShld`) with a block power (`shieldDef`); block is rolled before P.Def in Phase 3.

**Armor sets** are a separate data file (`stats/armorsets/{b_grade,a_grade,...}.xml`). A set lists accepted item ids per slot (including alternate versions such as "Light Use" variants), a `skill` granted when the full set is worn (e.g. `3520 Zubei's Leather Shirt Light Armor Set`: +STR/-CON style stat shifts, +evasion, +MP regen depending on set) and an `enchant6skill` granted when every piece is +6 or higher (e.g. `3618 Enchant Light Armor (B Grade)`, +P.Def %). Full-body pieces count as chest+legs.

### 2.3 Grades, expertise and the grade penalty

Grades are the `crystal_type` of the template. L2J's `CrystalType` enum also stores the crystal item and the crystal-count growth per enchant level:

| Grade | id | Crystal item | Crystal bonus per enchant (armor / weapon) | Level band (Expertise skill) |
|---|---|---|---|---|
| NONE | 0 | — | — | 1-19 |
| D | 1 | 1458 Crystal (D-Grade) | 11 / 90 | 20-39 |
| C | 2 | 1459 | 6 / 45 | 40-51 |
| B | 3 | 1460 | 11 / 67 | 52-60 |
| A | 4 | 1461 | 20 / 145 | 61-75 |
| S | 5 | 1462 | 25 / 250 | 76+ |
| S80 / S84 | 6 / 7 | 1462 | 25 / 250 | 80+ / 84+ (Gracia+) |

Expertise is a passive skill (id 239) auto-learned at 20/40/52/61/76; its level equals the highest grade id usable without penalty. The penalty is recomputed on every equip change (`refreshExpertisePenalty`):

```
weaponPenalty = clamp(maxEquippedWeaponGradeId - expertiseLevel - bonus, 0, 4)
armorPenalty  = clamp(maxEquippedArmorGradeId  - expertiseLevel - bonus, 0, 4)   // armor, shield, jewelry; arrows exempt
```

Each non-zero penalty level applies a hidden passive:

| Penalty level | Weapon Grade Penalty (skill 6209) | Armor Grade Penalty (skill 6213) |
|---|---|---|
| 1 | Accuracy -16 (flat), crit rate x0.9, crit dmg x0.9, atk.spd x0.9, P.Atk x0.9 | Evasion -2.5, atk.spd / cast.spd / run speed x0.8333 |
| 2 | Accuracy -16, crit x0.8, crit dmg x0.8, atk.spd x0.9, P.Atk x0.9 | Evasion -5, speeds x0.6944 |
| 3 | Accuracy -16, crit x0.7, crit dmg x0.7, … | Evasion -7.5, speeds x0.5787 |
| 4 | Accuracy -16, crit x0.6, crit dmg x0.6, … | Evasion -10, speeds x0.4823 |

The armor speed multipliers are `1 / 1.2^n`. In Interlude the penalty was accuracy/evasion only (one level per grade over), which is why "twinks" in over-grade gear were viable there and not in Gracia+.

### 2.4 Consumables

**Soulshots / spiritshots / blessed spiritshots** are stackable `EtcItem`s whose handler consumes `weapon.soulshots` (or `spiritshots`) units and sets a charged flag that the next attack or spell consumes:

| Shot | Effect (H5 L2J) | NPC price per unit, by grade (NG/D/C/B/A/S) |
|---|---|---|
| Soulshot | Normal hit: P.Atk x2 (`ssboost = 2`). Physical skills: `77 * (power + P.Atk * 2) / P.Def` (Interlude formula used x1.458 inside skills) | 7 / 14 / 22 / 50 / 80 / 100 |
| Spiritshot | M.Atk x2 (damage ∝ sqrt(M.Atk), so ≈ x1.41 damage); casting time x0.6 | 15 / 18 / 35 / 100 / 120 / 150 |
| Blessed Spiritshot | M.Atk x4 (≈ x2 damage); casting time x0.6 | 35 / 28 / 42 / 245 / 290 / 350 |

A shot must match the weapon's grade. Monsters also carry `shotChance`/`spiritChance` (e.g. 30%) to fire their own shots.

**Potions**: Healing Potion (+ HP over time), Greater/Quick variants, Mana Potions were *not* in Interlude retail (private-server addition). **Scrolls**: Scroll of Escape (teleport to nearest town, cast time 20 s, interrupted on damage), Scroll of Resurrection (restores XP fraction by scroll tier), Blessed Scroll of Escape, enchant scrolls (next section).

### 2.5 Enchanting

Rules common to all eras:

- Scroll must match item grade and category (weapon vs. armor; jewelry uses armor scrolls).
- **Safe enchant: +3** for everything, **+4 for full-body armor** (`onepiece`). Below the safe level success is 100%.
- Success increments enchant by 1. The server broadcasts at armor +6 / weapon +7 and at +15.
- **Failure with a normal scroll**: item is destroyed and converted into crystals of its grade (if `crystallizable`). L2J's count: `crystals = item.crystalCount(with enchant bonus) - (baseCrystalCount + 1) / 2`, minimum 1. The "with enchant bonus" term is `base + crystalEnchantBonus * (enchant - 3)` for enchant > 3, so a +10 B weapon returns far more B crystals than a +0 one.
- **Failure with a blessed scroll**: item survives, enchant resets to +0 (and the item is unequipped). Blessed Scroll: Enchant Weapon (A) lists at 15,000,000 adena vs 1,800,000 for the normal A scroll.
- **Crystal scrolls** (Interlude/C4 era): fail → stays at current level. Removed later.
- Armor reaching +4 while equipped grants its `enchant4Skill` (e.g. +4 Doom helmet set bonuses); +6 full sets grant `enchant6skill`.

Success chance differs by era:

| Era | Weapon (after safe) | Armor / jewelry (after safe) |
|---|---|---|
| Interlude (community-reported retail) | flat ~66% | flat ~66% |
| L2J Interlude fork defaults | 68% | armor 52%, jewelry 54% |
| H5 L2J (`enchantItemGroups.xml`) | Fighter weapons: 70% for +3→+15, 35% beyond. Mage weapons: 40% for +3→+15, 20% beyond | +3→+4 66.67%, then `100/(n-1)`: 33.34, 25, 20, 16.67, 14.29, 12.5, 11.12, 10, 9.1, 8.34, 7.7, 7.15, 6.67, 6.25, 5.89, 5.56 (+19→+20), 0 beyond. Full body shifted one step (100% to +4) |

Stat bonus per enchant level (`FuncEnchant`). `enchant` is capped at 3 for the base term and `over = enchant - 3` is the doubled term:

| Stat | Grade | Per level ≤+3 | Per level >+3 |
|---|---|---|---|
| P.Def / M.Def (any armor, jewelry) | all | +1 | +3 |
| M.Atk (any weapon) | S | +4 | +8 |
| | A, B, C | +3 | +6 |
| | D, NG | +2 | +4 |
| P.Atk, 1H (sword, blunt, dagger, pole*, other) | S / A / B-C / D-NG | +5 / +4 / +3 / +2 | +10 / +8 / +6 / +4 |
| P.Atk, 2H (2H sword, 2H blunt, dual, dualfist) | S / A / B-C / D-NG | +6 / +5 / +4 / +2 | +12 / +10 / +8 / +4 |
| P.Atk, bow | S / A / B-C / D-NG | +10 / +8 / +6 / +4 | +20 / +16 / +12 / +8 |

(*L2J treats poles as `lrhand`, so they use the 2H row; retail descriptions list spears with the 1H row. Decide one way in Nightfall.) Example: Bow of Peril (B) at +12 gains `6*3 + 12*9 = 126` P.Atk on a 400 base. Armor at +6 gains `3 + 3*3 = 12` P.Def per piece; at +3 it gains only 3, which is why +3 is cheap and +6 is the first meaningful milestone.

### 2.6 Augmentation (life stones)

Augmentation adds two hidden "options" (each an id in a 16-bit field, stored as `(stat34 << 16) | stat12`) to a C+ weapon, consuming a Life Stone and Gemstones, with **100% success**. Removing an augmentation costs adena by grade and clears both options.

| Life Stone grade | NPC reference price (lvl 46 stone) | L2J skill chance (`AugmentationXXSkillChance`) | Glow chance | Retail-like colour split yellow/blue/purple/red |
|---|---|---|---|---|
| No-grade (Life Stone) | 5,000 | 15% | 0% | 55 / 35 / 7 / 3 |
| Mid-grade | 20,000 | 30% | 40% | same |
| High-grade | 200,000 | 45% | 70% | same |
| Top-grade | 1,000,000 | 60% | 100% | same |

Purple and red results always carry a skill, so the "retail-like" mode yields ~10% skill chance regardless of grade, with grade mostly controlling the *strength* of the stat roll (offset = `lifeStoneLevel * 91` sub-block inside a 3640-entry block per colour) and the glow. Life stone **level** (46, 49, 52, 55, 58, 61, 64, 67, 70, 76, then 80/82/84) must be ≤ the character's level and scales the magnitude of stat options. Base-stat modifiers (+1 STR/CON/INT/MEN) have a flat 1% chance independent of grade.

Option pools (yellow = plain stat, blue = better stat, purple = passive/chance skill, red = active skill):

- Stats: P.Atk, M.Atk, P.Def, M.Def, Max HP/MP/CP, HP/MP/CP regen, Accuracy, Evasion, Crit rate, Atk.Spd, Cast.Spd, Speed, STR/CON/INT/MEN +1.
- Passive skills: Reflect damage, Heal/Refresh/Duel Might (party or self stat %), Prayer, Agility, Empower, Might, Shield, etc.
- Chance skills (proc on hit/being hit): Poison, Stun, Paralyze, Silence, Hold, Heal, Refresh.
- Active skills (castable): Heal, Celestial Shield, Blessed Body/Soul, Haste, Might, Empower, Wild Magic, Cheap Shot, Guidance, Focus, Shield, Agility, Duel Might, Reflex, Stun, Blessed Escape, Bleed, Poison, Fire/Ice/Wind bolts, Hydro Blast, etc.

Gemstone cost per attempt (`getGemStoneCount`): C 20 Gemstone D, B 30 Gemstone D, A 20 Gemstone C, S 25 Gemstone C (S80/S84 36 Gemstone B). Accessory augmentation (Freya+) uses accessory life stones and 200-480 gemstones.

### 2.7 Special abilities (soul crystals)

A C-grade or better weapon can be exchanged at a Blacksmith of Mammon for an SA variant (e.g. `Sword of Damascus - Focus`, `item_skill 3010-6`) by paying the base weapon, a Soul Crystal of matching stage and colour, Gemstones, and Ancient Adena. SA names observed across the datapack (counts are number of weapon variants): Focus (240), Health (226), Haste (151), Mana Up (69), Acumen (66), Cheap Shot (61), Critical Damage (59), Light (58), Guidance (56), Anger (54), HP Drain (40), Critical Stun (39), Conversion (35), Magic Hold (33), Critical Poison (31), Critical Bleed (30), HP Regeneration (28), Critical Drain (28), Rsk. Focus (27), Critical Slow (25), Rsk. Haste (24), MP Regeneration (24), Evasion (23), Rsk. Evasion (22), Quick Recovery (21), Mental Shield (18), Back Blow (18), Magic Silence (17), Empower (17), M. Atk. (15), Great Gale (15), Towering Blow (14), Thunder (13), Miser (12), Magic Weakness (12).

Weapon type conventions: swords get Focus/Haste/Health (1H) or Focus/Health/Critical Damage (2H); daggers Focus/Back Blow/Critical Damage or Critical Bleed/Poison; bows Cheap Shot/Focus/Critical Slow or Guidance/Light/Miser; blunts Anger/Health/Rsk. Focus; mage staves Acumen/Conversion/Mana Up or Empower/Magic Hold/Magic Silence; duals Focus/Health/Critical Damage; poles Guidance/Light/Haste or Anger/Critical Stun/Towering Blow.

Soul crystals come in Red / Green / Blue, stages 1-13 (H5 extends to 16). Required stage by grade: C needs 7-8, B 9-10, A 11-12, S 13. Stage 1-10 crystals are leveled by killing specific monsters while the crystal is in inventory (`levelUpCrystalData.xml` lists per NPC: which stage it can raise, `LAST_HIT` vs `FULL_PARTY` absorb mode, whether the Dwarf skill is needed, and a chance, default 5%); stage 11+ requires raid bosses with `FULL_PARTY` absorb. A failed absorb at high stage can **break** the crystal into a broken crystal, which is the main sink for the SA market.

### 2.8 Item identity, flags, inventory and storage

Template flags (parsed in `L2Item`): `is_stackable` (default false), `is_sellable`, `is_droppable`, `is_destroyable`, `is_tradable`, `is_depositable` (default true), `is_questitem`, `is_freightable`, `enchant_enabled`, `element_enabled`, `duration` (minutes, shadow items), `time` (seconds, time-limited items), `crystal_type`, `crystal_count`, `price` (reference price for NPC sell = `price / 2`), `weight` (in 1/1000 units, so a 1,600 "weight" short sword weighs 1.6 kg). Every instance has an `object_id` (32-bit, unique across the world) separate from the template `item_id`; stackables are one instance with a count.

"Soulbound" in the retail sense (bind-on-equip) does not exist in classic L2; untradeable items are untradeable from the template (`is_tradable=false`, e.g. quest rewards, hero weapons, PvP items, Common Items). **Common Items** (lower-cost crafted variants, e.g. `Common Item - Sword of Damascus`) share stats with the original but have `crystal_count` 82 vs 1346, cannot be enchanted or augmented, and are untradeable. **Shadow items** have `duration` and vanish when it expires.

Capacity (character.properties):

| Container | Non-Dwarf | Dwarf | Notes |
|---|---|---|---|
| Inventory slots | 80 | 100 | GM 250; quest items have a separate 100-slot pool; client crashes above ~300 |
| Private warehouse | 100 | 120 | |
| Clan warehouse | 200 (150 in Interlude) | | Withdrawal gated by clan privilege |
| Freight | 200 (20 in Interlude) | | 1,000 adena per deposited item; delivers between characters of one account |
| Private store sell / buy slots | 3 / 4 | 4 / 5 | Interlude: 4 / 5 |
| Max adena | 99,900,000,000 | | per container |

**Weight limit** (`getMaxLoad`): `floor(CON_bonus(CON) * 69000) * WeightLimitMultiplier`, where `CON_bonus` is the same table used for HP (≈1.0 at CON 43, giving ~69,000 weight units ≈ 69 kg). Items also have a `weightLimit` stat hook so belts/passives can add to it. Overweight penalty is recomputed on every inventory change (`refreshOverloaded`), with `load_pct = (currentLoad - bonusWeightPenalty) / maxLoad`:

| Load | Penalty level (skill 4270) | HP/MP regen | Run speed | Other |
|---|---|---|---|---|
| < 50% | 0 | — | — | |
| 50-66.6% | 1 | x0.5 | x1.0 | |
| 66.6-80% | 2 | x0.5 | x0.5 | |
| 80-100% | 3 | x0.5 | x0.5 | |
| ≥ 100% | 4 | x0.5 | x0 | `isOverloaded`: cannot attack, cast or pick up |

Dwarves get higher base CON and the Dwarven "weight limit" passives (`weightLimit` stat), not a different formula.

## 3. Design decisions for Nightfall

1. **Keep the six-grade ladder and the level bands** (NG 1-19, D 20-39, C 40-51, B 52-60, A 61-75, S 76+). Rationale: the ladder is the backbone of the economy's demand curve (Phase 5); its vertical steps create crafting and enchant markets per band. S80/S84 are not planned.
2. **Grade penalty = Gracia+ model, not Interlude**: separate weapon/armor penalty levels 0-4 with the multiplicative speed penalties from 2.3. This closes the "twink" loophole and makes expertise meaningful. Penalty effects are data-driven buffs (Phase 3 status system), not hard-coded.
3. **Weapon type constants are data, not code**: base attack speed, crit, random damage, range, shot count, MP cost live in the template file, with the 2.1 table as the defaults. Nightfall uses the 1H row for poles (retail description) rather than L2J's 2H row.
4. **Armor type tradeoff via class passives**, exactly as L2 does, so a class can "wear anything" at the cost of its mastery. Phase 2 defines which classes get which mastery.
5. **Enchant model**: safe +3 (+4 full body), then an H5-style per-level curve for armor (`100/(n-1)` from +4) and a flat-then-halved curve for weapons (70%/35% fighter, 40%/20% mage). Flat Interlude 66% was considered and rejected because the armor curve gives a natural ceiling without a hard cap. Blessed scrolls reset to +0; normal scrolls crystallize. Max enchant is uncapped for weapons and capped at +20 for armor (0% beyond +19), as in the H5 table.
6. **Enchant bonus table** is the FuncEnchant table verbatim (2.5), stored as data keyed by `(stat, grade, weapon_class)`.
7. **Augmentation**: adopt the two-option model with 100% success and grade-driven glow/strength, but use the "retail-like" 55/35/7/3 colour split so skill augments stay rare (~10%) at every grade. Life stone level gates magnitude. Removal is paid in adena (Phase 5 sink).
8. **Special abilities**: implement SA as a *template variant* (separate `item_id`, as L2 does) rather than an instance modifier. Simpler for the client (icon/name per variant) and for the economy (SA weapons are distinct market items). Soul crystal leveling is a Phase 6 monster-side feature; this phase only defines the crystal items and the exchange.
9. **Item identity**: `ItemTemplate` (static, loaded from TOML, hot-reloadable in dev) vs `ItemInstance` (Postgres row with `instance_id` UUIDv7). Stackables are one row with `count`; non-stackables always `count = 1`. Binding flags come from the template; Nightfall adds one instance-level flag `bound_to_character` reserved for future bind-on-pickup rewards (default false, never set in Phase 4).
10. **Inventory limits**: 80 slots (+20 Dwarf), weight = `floor(CON_bonus * 69000)`, overweight levels as in 2.8. Warehouse 100/120, clan 200, freight 200 at 1,000 adena per item. These are config values, not constants.
11. **Shots**: soulshot x2 P.Atk on hits, spiritshot x2 M.Atk, blessed x4 M.Atk and cast time x0.6, all grade-matched. Auto-use is client intent; the server validates grade, count, and charge state.
12. **No item durability / repair**: as in L2. Enchant failure and crystallization are the item sink (Phase 5).

Alternatives considered: (a) a single "item level" instead of grades — rejected, grades give discrete market tiers; (b) deterministic enchant with pity counters — deferred to tuning (Phase 9 telemetry); (c) bind-on-equip for raid loot — deferred (open question 8.4).

## 4. Data model

### 4.1 Entities

- **ItemTemplate** — immutable definition. Identified by `template_id: u32` (stable, never reused). Variants (SA, Common Item) are their own templates with `base_template_id` for display grouping.
- **ItemInstance** — owned row: `instance_id`, `template_id`, `owner_character_id`, `location` (INVENTORY, EQUIPPED, WAREHOUSE, CLAN_WAREHOUSE, FREIGHT, MAIL, TRADE_ESCROW, GROUND), `slot`, `count`, `enchant`, `augment_a`, `augment_b`, `expires_at`, `bound_to_character`, `created_at`, `updated_at`, `version` (optimistic lock).
- **EquipSlot** — enum: RHAND, LHAND, LRHAND (virtual: occupies both), HEAD, CHEST, LEGS, FULLBODY (virtual: chest+legs), GLOVES, FEET, NECK, EAR_L, EAR_R, FINGER_L, FINGER_R, UNDERWEAR, CLOAK (reserved), BELT (reserved).
- **ArmorSet** — list of template ids per slot, full-set skill id, +6 skill id.
- **EnchantRateGroup** / **EnchantBonusTable** / **ExpertiseTable** / **WeightPenaltyTable** — pure data.

### 4.2 Template definition (TOML)

One file per grade and category under `packages/data/items/`, e.g. `weapons_b.toml`:

```toml
[[item]]
id            = 79
name          = "Sword of Damascus"
kind          = "weapon"            # weapon | armor | jewelry | shield | etc
grade         = "B"
icon          = "weapon_sword_of_damascus"
weight        = 1350                 # grams
price         = 10_091_400           # reference price; NPC sell = price/2
crystal_count = 1346
stackable     = false
tradeable     = true
sellable      = true
droppable     = true
destroyable   = true
depositable   = true
enchantable   = true
augmentable   = true

[item.weapon]
type          = "sword"              # sword|blunt|dagger|bow|pole|dual|dualfist|fist
hands         = 1                    # 1 | 2 (bows, poles, duals are 2)
p_atk         = 194
m_atk         = 99
# Optional overrides; omitted fields fall back to weapon_type_defaults below
# atk_spd = 379, crit = 8, random_dmg = 10, range = 40, soulshots = 1, spiritshots = 1

[[item]]
id            = 287
name          = "Bow of Peril"
kind          = "weapon"
grade         = "B"
weight        = 1700
price         = 10_091_400
crystal_count = 1346

[item.weapon]
type          = "bow"
hands         = 2
p_atk         = 400
m_atk         = 99
soulshots     = 3
mp_per_shot   = 4
reuse_ms      = 1500
```

Type defaults, `packages/data/items/weapon_types.toml`:

```toml
[sword]    atk_spd = 379 crit = 8  random_dmg = 10 range = 40 accuracy = 0
[sword2h]  atk_spd = 325 crit = 8  random_dmg = 10 range = 40 accuracy = 0
[blunt]    atk_spd = 379 crit = 4  random_dmg = 20 range = 40 accuracy = 4.75
[blunt2h]  atk_spd = 325 crit = 4  random_dmg = 20 range = 40 accuracy = 4.75
[dagger]   atk_spd = 433 crit = 12 random_dmg = 5  range = 40 accuracy = 0
[bow]      atk_spd = 293 crit = 12 random_dmg = 5  range = 500 accuracy = -3.75 reuse_ms = 1500
[pole]     atk_spd = 325 crit = 8  random_dmg = 10 range = 80 accuracy = -3.75 max_targets = 3
[dual]     atk_spd = 325 crit = 8  random_dmg = 10 range = 40 accuracy = 0
[dualfist] atk_spd = 325 crit = 4  random_dmg = 5  range = 40 accuracy = 4.75
[fist]     atk_spd = 325 crit = 4  random_dmg = 5  range = 40 accuracy = 0
```

Armor example:

```toml
[[item]]
id = 2380  name = "Blue Wolf Breastplate"  kind = "armor"  grade = "B"
weight = 7820  price = 2_475_000  crystal_count = 330
[item.armor]
type = "heavy"        # heavy | light | robe
slot = "chest"        # chest | legs | fullbody | head | gloves | feet
p_def = 166
m_def = 0
set_id = 31
```

Enchant tables, `packages/data/items/enchant.toml`:

```toml
[safe]                  default = 3   fullbody = 4
[rates.armor]           "0-2" = 100  "3" = 66.67  "4" = 33.34  "5" = 25  "6" = 20  "7" = 16.67  "8" = 14.29  "9" = 12.5  "10" = 11.12  "11" = 10  "12" = 9.1  "13" = 8.34  "14" = 7.7  "15" = 7.15  "16" = 6.67  "17" = 6.25  "18" = 5.89  "19" = 5.56  "20+" = 0
[rates.fullbody]        "0-3" = 100  "4" = 66.67  "5" = 33.34  # ... shifted by one
[rates.weapon_fighter]  "0-2" = 100  "3-14" = 70   "15+" = 35
[rates.weapon_mage]     "0-2" = 100  "3-14" = 40   "15+" = 20
[bonus.def]             base = 1  over = 3          # P.Def / M.Def all grades
[bonus.m_atk]           S = [4, 8]  A = [3, 6]  B = [3, 6]  C = [3, 6]  D = [2, 4]  NG = [2, 4]
[bonus.p_atk.one_hand]  S = [5, 10] A = [4, 8]  B = [3, 6]  C = [3, 6]  D = [2, 4]  NG = [2, 4]
[bonus.p_atk.two_hand]  S = [6, 12] A = [5, 10] B = [4, 8]  C = [4, 8]  D = [2, 4]  NG = [2, 4]
[bonus.p_atk.bow]       S = [10, 20] A = [8, 16] B = [6, 12] C = [6, 12] D = [4, 8] NG = [4, 8]
```

### 4.3 Postgres schema (sketch)

```sql
CREATE TYPE item_location AS ENUM ('inventory','equipped','warehouse','clan_warehouse',
                                   'freight','mail','trade_escrow','ground');

CREATE TABLE item_instances (
  instance_id      UUID PRIMARY KEY,                 -- UUIDv7, time-ordered
  template_id      INTEGER NOT NULL,                 -- validated against loaded templates at boot
  owner_id         UUID,                             -- character_id; NULL when on ground / clan wh
  clan_id          UUID,                             -- set when location = clan_warehouse
  account_id       UUID,                             -- set when location = freight
  location         item_location NOT NULL,
  slot             SMALLINT,                         -- EquipSlot when equipped, else NULL
  count            BIGINT NOT NULL CHECK (count > 0),
  enchant          SMALLINT NOT NULL DEFAULT 0 CHECK (enchant >= 0),
  augment_a        INTEGER,                          -- option id or NULL
  augment_b        INTEGER,
  bound_to         UUID,                             -- character_id if bound
  expires_at       TIMESTAMPTZ,                      -- shadow / time-limited
  version          INTEGER NOT NULL DEFAULT 0,       -- optimistic concurrency
  created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON item_instances (owner_id, location);
CREATE INDEX ON item_instances (clan_id) WHERE location = 'clan_warehouse';
CREATE UNIQUE INDEX one_item_per_slot ON item_instances (owner_id, slot)
  WHERE location = 'equipped';

-- Append-only audit of every mutation; the economy telemetry in Phase 5/9 reads this.
CREATE TABLE item_ledger (
  ledger_id    BIGSERIAL PRIMARY KEY,
  instance_id  UUID NOT NULL,
  template_id  INTEGER NOT NULL,
  delta_count  BIGINT NOT NULL,
  reason       TEXT NOT NULL,       -- 'drop','craft','enchant_fail','trade','npc_sell',...
  actor_id     UUID,
  counterparty UUID,
  rng_seed     BYTEA,               -- present for RNG-driven events (enchant, augment)
  at           TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Stack merge rule: when adding a stackable template to a container, `UPDATE ... SET count = count + $n WHERE owner_id=$o AND template_id=$t AND location=$l AND enchant=0` else `INSERT`. Non-stackables always insert.

### 4.4 Proto sketch (`packages/proto/nightfall/v1/items.proto`)

```proto
syntax = "proto3";
package nightfall.v1;

enum ItemGrade { ITEM_GRADE_UNSPECIFIED = 0; NG = 1; D = 2; C = 3; B = 4; A = 5; S = 6; }

enum EquipSlot {
  EQUIP_SLOT_UNSPECIFIED = 0; RHAND = 1; LHAND = 2; LRHAND = 3; HEAD = 4; CHEST = 5; LEGS = 6;
  FULLBODY = 7; GLOVES = 8; FEET = 9; NECK = 10; EAR_L = 11; EAR_R = 12; FINGER_L = 13; FINGER_R = 14;
}

enum ItemLocation { ITEM_LOCATION_UNSPECIFIED = 0; INVENTORY = 1; EQUIPPED = 2; WAREHOUSE = 3;
                    CLAN_WAREHOUSE = 4; FREIGHT = 5; }

message Item {
  string instance_id = 1;     // UUID
  uint32 template_id = 2;     // client resolves name/icon/stats from its item catalogue
  int64  count = 3;
  uint32 enchant = 4;
  uint32 augment_a = 5;
  uint32 augment_b = 6;
  ItemLocation location = 7;
  EquipSlot slot = 8;         // only when EQUIPPED
  int64  expires_at_ms = 9;   // 0 = never
  bool   bound = 10;
}

message Inventory {
  repeated Item items = 1;
  uint32 slots_used = 2;
  uint32 slots_max = 3;
  uint32 weight = 4;          // grams
  uint32 weight_max = 5;
  uint32 weight_penalty_level = 6;   // 0-4
  uint32 weapon_grade_penalty = 7;   // 0-4
  uint32 armor_grade_penalty = 8;    // 0-4
  int64  adena = 9;
}

message EquipRequest   { string instance_id = 1; }        // server picks the slot (and swaps rings/earrings)
message UnequipRequest { EquipSlot slot = 1; }
message UseItemRequest { string instance_id = 1; string target_id = 2; }   // potions, scrolls, shots toggle
message DestroyItemRequest { string instance_id = 1; int64 count = 2; }

message EnchantRequest { string item_instance_id = 1; string scroll_instance_id = 2; }
message EnchantResult  {
  enum Outcome { OUTCOME_UNSPECIFIED = 0; SUCCESS = 1; FAILED_DESTROYED = 2; FAILED_RESET = 3; FAILED_SAFE = 4; }
  Outcome outcome = 1; uint32 new_enchant = 2; uint32 crystal_template_id = 3; int64 crystal_count = 4;
}

message AugmentRequest { string item_instance_id = 1; string life_stone_instance_id = 2; string gemstone_instance_id = 3; }
message AugmentResult  { uint32 option_a = 1; uint32 option_b = 2; }

// Server -> client deltas; the client never recomputes derived stats itself.
message InventoryUpdate { repeated Item added = 1; repeated Item modified = 2; repeated string removed_instance_ids = 3;
                          Inventory summary = 4; }
```

Add to `GameService`: `GetInventory`, `Equip`, `Unequip`, `UseItem`, `DestroyItem`, `Enchant`, `Augment`, and a server-streaming `SubscribeInventory(stream InventoryUpdate)` until the general world-event stream exists (Phase 0).

## 5. Interfaces

**gRPC** (above). All mutating RPCs are idempotent per `(character_id, request_id)` for 60 s to survive client retries.

**Server events** (internal bus, Phase 0): `ItemAdded`, `ItemRemoved`, `ItemEquipped`, `ItemUnequipped`, `EnchantResolved{seed, outcome}`, `AugmentResolved`, `WeightPenaltyChanged`, `GradePenaltyChanged`. Combat (Phase 3) subscribes to equip/unequip and penalty events to rebuild the stat sheet; economy telemetry (Phase 5/9) subscribes to everything.

**Validation on equip** (server-side, in order): item owned and in inventory; template `kind` equipable; class/race restrictions from the template (Phase 2 hook); two-handed weapon unequips shield; FULLBODY unequips chest+legs; ring/earring picks the free slot or swaps the left; arrows must match bow grade; then recompute grade penalty, weight penalty, set bonuses, and emit a single `InventoryUpdate`.

**Client needs**: a static item catalogue (`template_id → name, icon, grade, kind, display stats`) shipped as JSON generated from the TOML at build time (moon task), so `Item` messages stay small.

## 6. Rust implementation notes

Module layout under `apps/api/src/items/`:

```
items/
  mod.rs            // pub use; wires gRPC handlers in grpc.rs
  template.rs       // ItemTemplate, WeaponSpec, ArmorSpec, enums; serde from TOML
  catalog.rs        // ItemCatalog: Arc<HashMap<u32, ItemTemplate>>, loaded at boot, ArcSwap for dev reload
  instance.rs       // ItemInstance (sqlx FromRow), ItemLocation, EquipSlot
  repo.rs           // sqlx queries: load_inventory, add_stack, split_stack, move_location, set_enchant
  inventory.rs      // in-memory Inventory per online character: slot/weight accounting, equip rules
  equip.rs          // slot resolution (two-hand, fullbody, ring swap), grade penalty calc
  enchant.rs        // EnchantTables, roll_enchant(rng) -> Outcome, crystal_refund()
  augment.rs        // option pools, roll_augment(rng, grade, level) -> (u32, u32)
  shots.rs          // charge/consume soulshot & spiritshot state
  weight.rs         // max_load(con), penalty_level(load_pct)
  tables.rs         // weapon type defaults, expertise bands, enchant bonus lookup
  events.rs         // ItemEvent enum published on the server bus
```

Crates: `sqlx` (Postgres, `runtime-tokio`, `uuid`, `time`), `uuid` (v7), `serde` + `toml`, `arc-swap` for catalog hot-reload, `rand` + `rand_chacha` for the seeded RNG, `thiserror` for `ItemError`.

Concurrency: the authoritative inventory for an online character lives in that character's actor task (one `tokio` task per session, Phase 0), so inventory mutations are single-threaded per character and need no locks. Cross-character moves (trade, mail, Phase 5) go through Postgres transactions with `SELECT ... FOR UPDATE` on both `item_instances` rows plus the `version` check. Offline containers (warehouse, freight) are only touched via repo transactions.

RNG: every enchant/augment roll uses `ChaCha12Rng::from_seed(hash(server_secret, character_id, request_id))`. The seed is written to `item_ledger.rng_seed` so any outcome can be replayed in a GM tool. Never use thread RNG for player-visible rolls.

Enchant resolution pseudocode:

```rust
pub fn roll_enchant(t: &EnchantTables, item: &ItemInstance, tpl: &ItemTemplate, scroll: &ScrollSpec, rng: &mut impl Rng) -> Outcome {
    let safe = if tpl.is_fullbody() { t.safe_fullbody } else { t.safe_default };
    let chance = if item.enchant < safe { 100.0 } else { t.rate_for(tpl, item.enchant) };
    if rng.gen_range(0.0..100.0) < chance { return Outcome::Success(item.enchant + 1); }
    match scroll.kind {
        ScrollKind::Blessed => Outcome::FailedReset,
        ScrollKind::Normal  => Outcome::FailedDestroyed { crystals: crystal_refund(tpl, item.enchant) },
    }
}

pub fn crystal_refund(tpl: &ItemTemplate, enchant: u16) -> u32 {
    let over = enchant.saturating_sub(3) as u32;
    let with_bonus = tpl.crystal_count + tpl.grade.crystal_bonus(tpl.kind) * over;
    (with_bonus - (tpl.crystal_count + 1) / 2).max(1)
}
```

Derived-stat contribution (consumed by Phase 3): `items::equip::StatContribution { p_atk, m_atk, p_def, m_def, atk_spd_base, crit, random_dmg, range, accuracy_mod, evasion_mod, max_mp, penalty_mults }` computed once per equipment change and cached on the character.

## 7. Client implications

- Render inventory as an 80/100-slot grid plus equipment paper-doll with the 14 slots; grey out items above expertise and show the penalty level badge (`weapon_grade_penalty`, `armor_grade_penalty` from `Inventory`).
- Weight bar with colour thresholds at 50/66/80/100%; show the regen/speed penalty text from a client-side table.
- Enchant UI: drag scroll onto item, display safe level and the server-returned outcome; animate +N glow tiers (weapon glow starts at +4, intensifies at +7/+10/+13 in L2; Phaser tint/particle per tier).
- Shots: toggle buttons per shot type; client sends `UseItem` once to toggle auto-use, server consumes per attack and streams count deltas (batched, not per hit).
- Tooltips compute display values (base + enchant bonus) from the catalogue and the `enchant` field using the same table as the server, so the two must be generated from one TOML.
- Augmented weapons show option names from a client-side option catalogue keyed by option id.

## 8. Open questions

1. Expertise bonus (`getExpertisePenaltyBonus`) — L2 gave some Dwarf/Kamael skills +1 expertise. Do any Nightfall classes get it (Phase 2)?
2. Weapon enchant cap: leave uncapped (H5) or hard-cap at +16/+20? Depends on Phase 5 sink tuning.
3. Pole enchant row (1H per retail text vs 2H per L2J). Decision above is 1H; confirm with combat balance in Phase 3.
4. Bind-on-pickup for raid loot: supported by schema (`bound_to`), not used. Decide with Phase 6 raid design.
5. Augment removal fee schedule by grade and whether removal returns the life stone (L2: no).
6. Soul crystal break chance and stage ladder — finalize with Phase 6 monster tables.
7. Mana potions: L2 Interlude retail had none; many private servers add them. Phase 3 MP economy decides.
8. Whether `Common Item` variants exist at all in Nightfall or whether crafting always yields the full item (Phase 5).
9. Shadow (time-limited) items: keep `expires_at` but no content planned; confirm before building expiry sweeps.

## 9. Sources

- L2J Server (High Five) game source, Bitbucket `l2jserver/l2j-server-game` (checked out 2026-09-13): `model/stats/functions/formulas/FuncEnchant.java` (enchant bonus table), `model/actor/instance/L2PcInstance.java` (`refreshOverloaded`, `refreshExpertisePenalty`, `getMaxLoad`), `model/items/type/CrystalType.java`, `model/items/L2Item.java` (template flags), `network/clientpackets/RequestEnchantItem.java` and `model/items/enchant/EnchantScroll.java` (enchant resolution, crystallization), `network/clientpackets/AbstractRefinePacket.java` (life stone table, gemstone costs), `data/xml/impl/AugmentationData.java` (option generation), `network/clientpackets/RequestCrystallizeItem.java`, `model/stats/Formulas.java` (shot multipliers), `src/main/resources/config/character.properties` (inventory, warehouse, freight, store slots, augmentation chances, expertise penalty). https://bitbucket.org/l2jserver/l2j-server-game
- L2J Datapack (High Five), Bitbucket `l2jserver/l2j-server-datapack`: `data/stats/items/*.xml` (all weapon/armor/jewelry/consumable templates surveyed for the type table), `data/enchantItemGroups.xml` (enchant rate curves), `data/enchantItemData.xml` (scroll grades), `data/stats/skills/06200-06299.xml` (skills 6209/6213 grade penalty), `data/stats/skills/04200-04299.xml` (skill 4270 weight penalty), `data/stats/skills/00200-00299.xml` (armor masteries 227/231/233, weapon mastery 249, Create Item 172), `data/stats/armorsets/*.xml`, `data/levelUpCrystalData.xml` and quest `Q00350_EnhanceYourWeapon` (soul crystal leveling). https://bitbucket.org/l2jserver/l2j-server-datapack
- L2JBrasil Interlude server fork (GitHub `L2jBrasil/Server-Interlude`, 2018): `Config.java` (Interlude defaults: EnchantChanceWeapon 68, Armor 52, Jewelry 54, EnchantSafeMax 3, EnchantSafeMaxFull 4, warehouse 100/120/150, freight 20, store slots 5/4), `RequestEnchantItem.java` (flat chance, full armor +4 safe, crystal scrolls). https://github.com/L2jBrasil/Server-Interlude
- l2hub.info item database, "Scroll: Enchant Weapon (Grade S)" description (per-type P.Atk bonus and doubling from +4). https://l2hub.info/c4/items/scrl_of_ench_wp_s
- ludo.guide, "Enchanting / Upgrading equipment" and "Augmenting / Life Stones" (Lineage II). https://www.ludo.guide/guide/lineage-ii/enchanting-augmenting/enchanting-upgrading-equipment , https://www.ludo.guide/guide/lineage-ii/enchanting-augmenting/augmenting-life-stones
- L2DB.net guides, "Best Bows in Lineage 2: Interlude (CT0)" (bow MP consumption and shot usage context). https://l2db.net/guides/en/best-bows-in-lineage-2-interlude-ct0-l2dbnet-2
- Lineage II Classic 1.5 patch notes (basic combat stats glossary). https://www.lineage2.com/news/lineage-ii-classic-launch-patch-notes
