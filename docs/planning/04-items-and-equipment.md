# Phase 4: Items and Equipment

## 1. Purpose and scope

Phase 4 gives all the items that a character can hold, wear, or use up:

- **Item templates** (static definitions that the server loads from data files).
- **Item instances** (rows that a character owns, with enchant level, augmentation, binding, and stack count).
- **Weapons** (type, P.Atk/M.Atk, attack speed, crit rate, random damage, range, shot consumption, MP cost for bows).
- **Armor** (the tradeoffs of heavy/light/robe, class mastery hooks, set bonuses), **jewelry** (M.Def, resistances), **shields**.
- The **grade system** (No/D/C/B/A/S), the expertise levels, and the grade penalty. The server applies this penalty when a character equips an item above its expertise.
- **Consumables**: soulshots, spiritshots, blessed spiritshots, potions, scrolls.
- **Enchanting** (safe levels, success curves, the stat bonus for each level, blessed scrolls, crystallization after a failure).
- **Augmentation** (life stones, option pools, skill chance) and **special abilities** (soul crystals).
- **Inventory** (slot and weight limits, overweight penalties), **warehouse** (private, clan), and **freight**.

These items are not in the scope of this phase:

- How a character *gets* items (drops, crafting, shops, trade). This is Phase 5.
- The combat formulas that use the stats in this document. This is Phase 3.
- Pet items and mounts. This is Phase 3/6.
- Cosmetic items. This is Phase 8.

Phase 3 (combat) uses the stat outputs of this phase (P.Atk, attack speed, shot multipliers). Phase 5 (economy) uses the identity model (stackability, tradeability, grade, crystal value).

## 2. Reference: how Lineage 2 does it

The numbers below come from the L2J High Five datapack and server source. This is the most complete open reimplementation. Where the two eras are different, we compared the numbers with an Interlude-era L2J fork. L2J got its values from retail by reverse engineering. Thus these values are a start point, not canon.

### 2.1 Weapon types and base stats

Each weapon template has `weapon_type`, `bodypart` (`rhand` = one-handed, `lrhand` = two-handed), `random_damage`, and `attack_range`. It also has `soulshots`/`spiritshots` (the shots that one hit uses). Bows can also have `mp_consume` and `reuse_delay`. Each template has four stats: `pAtk`, `mAtk`, `critRate`, `pAtkSpd`. We examined all ~4,000 weapon templates in the H5 datapack. The result is the constants below for each type. The base attack speed is the same for all weapons of a type, at all grades:

| Type (L2J `weapon_type`) | Slot | Base Atk.Spd | Crit rate | Random dmg % | Attack range | Notes |
|---|---|---|---|---|---|---|
| SWORD (1H) | rhand | 379 | 8 | 10 | 40 | |
| SWORD (2H) | lrhand | 325 | 8 | 10 | 40 | More P.Atk than a 1H of the same grade |
| BLUNT (1H) | rhand | 379 | 4 | 20 | 40 | `accCombat +4.75`; mage staves are 2H blunts with the flag `is_magic_weapon` |
| BLUNT (2H) | lrhand | 325 | 4 | 20 | 40 | |
| DAGGER | rhand | 433 | 12 | 5 | 40 | Lowest P.Atk for each grade, highest speed/crit |
| BOW | lrhand | 293 (227 for the slow "heavy" bows) | 12 | 5 | 500 | `accCombat -3.75`, `mp_consume` for each shot, `reuse_delay 1500` ms between shots |
| DUAL | lrhand | 325 | 8 | 10 | 40 | Dual swords; P.Atk between 1H and 2H |
| DUALFIST | lrhand | 325 | 4 | 5 | 40 | `accCombat +4.75` |
| POLE | lrhand | 325 | 8 | 10 | 80 | `accCombat -3.75`; hits a maximum of N targets in an arc (skill 3599) |
| FIST (bare hands) | — | 325 | 4 | 5 | 40 | P.Atk 4 at level 1 |
| RAPIER / ANCIENTSWORD / CROSSBOW (Kamael, Gracia+) | — | 406 / 350 / 303 | 10 / 8 / 10 | 40 / 15 / 10 | | Not in the scope of the Nightfall era |

Typical P.Atk / M.Atk for each grade (from the datapack):

| Grade | 1H sword | 2H sword | Dagger | Bow | Pole | Dual | Mage staff (2H blunt) |
|---|---|---|---|---|---|---|---|
| No (Short Sword lvl 1) | 8 / 6 | — | — | 16 / 6 (Short Bow) | — | — | — |
| C (Yaksa Mace / Deadman's Staff) | 156 / 83 (mace) | — | — | — | — | — | 152 / 122 |
| B (Sword of Damascus, Great Sword, Bow of Peril, Lance, Arthro Nail) | 194 / 99 | 213 / 91 | ~168 / 99 | 400 / 99 | 194 / 99 | — | — |
| A (Keshanberk*Keshanberk) | — | — | — | — | — | 259 / 107 | — |

At the same grade, the bow has 400 P.Atk and the sword has 194. This difference is intentional, because the bow has these costs:

- It fires at 293 speed.
- It uses `mp_consume` 1-6 MP for each shot, by grade (NG 1, D 2, C 3, B 4, A 5, S 6).
- It has `reuse_delay 1500` ms.
- At low grades, it uses more soulshots for each hit: D bows 6-10, C bows 8-10, B 3, A 2, S 1. Melee weapons use 2-3 at NG/D/C, and 1 at B/A/S.

For a normal attack, the damage for each hit is approximately `76 * P.Atk / P.Def`, scaled by random damage. Thus bows give less sustained DPS and use MP, but they give more burst and range.

### 2.2 Armor, shields, jewelry

Armor templates have these fields:

- `armor_type` (HEAVY, LIGHT, MAGIC = robe, or none for jewelry/shields).
- `bodypart` (`chest`, `legs`, `onepiece` = full body, `head`, `gloves`, `feet`, `lhand` = shield, `neck`, `rear;lear`, `rfinger;lfinger`).
- `pDef`/`mDef`, and an optional `maxMp`.
- Item skills for set/resist effects.

Comparison at the same grade (B grade):

| Piece | Heavy (Blue Wolf) | Light (Blue Wolf) | Robe (Blue Wolf Tunic / Avadon Robe) |
|---|---|---|---|
| Upper body | Breastplate 166 P.Def | Leather Armor (full body) 202 P.Def | Tunic 83 P.Def +377 MP |
| Lower body | Gaiters 104 P.Def | (included) | Stockings ~52 P.Def +MP |
| Full body alternative | Doom Plate 270 P.Def | — | Avadon Robe 127 P.Def +561 MP |
| Helmet | 66 P.Def (the same for all types) | | |

A grade: Majestic Plate 293, Majestic Leather 220, Majestic Robe 147 (+718 MP). The data shows a general rule: light body armor ≈ 75% of heavy, and robe ≈ 50% of heavy. Robes compensate with raw MP. Robes also compensate with casting speed and M.Def (through class passives).

The **class mastery passives** cause the tradeoff, not the items:

- **Heavy Armor Mastery** (skill 231, 50 levels, lvl 18-74): `+pDef` from 1.9 to 79.3, with the condition `<using kind="HEAVY"/>`. The enchanted versions also add HP regen.
- **Light Armor Mastery**, fighter version (227): `+pDef` 4.2 to 81.3 and `+evasion` 3 to 6, only while the character wears light armor. Rogue version (233): `+pDef` 1.3 to 65.6, `+evasion` 4 to 7, and critical damage received x0.85 / x0.75 / x0.65 by tier.
- **Robe Mastery** (mages): M.Def and casting speed bonuses, with the condition of MAGIC armor.
- If a character wears the incorrect type, it does not get the passive. In Interlude, mages in heavy armor also got casting-speed penalties through the same conditional mechanism.

Jewelry gives **M.Def only** (plus MP on later grades). Elemental/status resistances are on jewelry, through `item_skill`:

| Grade | Necklace | Earring | Ring |
|---|---|---|---|
| B (Black Ore) | 72 | ~54 | 36 |
| A (Majestic) | 85 (+33 MP) | 63 (+25 MP) | 42 (+17 MP) |
| S (Tateossian) | 95 (+42 MP) | ~71 | 48 (+21 MP) |

**Shields** (`lhand`) add P.Def and a block rate (`rShld`) with a block power (`shieldDef`). In Phase 3, the block roll occurs before P.Def.

**Armor sets** are in a separate data file (`stats/armorsets/{b_grade,a_grade,...}.xml`). A set has these parts:

- A list of the accepted item ids for each slot. This list includes alternative versions, for example "Light Use" variants.
- A `skill` that the character gets when it wears the full set. An example is `3520 Zubei's Leather Shirt Light Armor Set`: stat shifts of the +STR/-CON type, +evasion, and +MP regen, by set.
- An `enchant6skill` that the character gets when all pieces are +6 or higher. An example is `3618 Enchant Light Armor (B Grade)`, +P.Def %.

Full-body pieces count as chest+legs.

### 2.3 Grades, expertise and the grade penalty

Grades are the `crystal_type` of the template. The L2J `CrystalType` enum also keeps the crystal item and the increase of the crystal count for each enchant level:

| Grade | id | Crystal item | Crystal bonus per enchant (armor / weapon) | Level band (Expertise skill) |
|---|---|---|---|---|
| NONE | 0 | — | — | 1-19 |
| D | 1 | 1458 Crystal (D-Grade) | 11 / 90 | 20-39 |
| C | 2 | 1459 | 6 / 45 | 40-51 |
| B | 3 | 1460 | 11 / 67 | 52-60 |
| A | 4 | 1461 | 20 / 145 | 61-75 |
| S | 5 | 1462 | 25 / 250 | 76+ |
| S80 / S84 | 6 / 7 | 1462 | 25 / 250 | 80+ / 84+ (Gracia+) |

Expertise is a passive skill (id 239). A character learns it automatically at 20/40/52/61/76. Its level is equal to the highest grade id that the character can use without a penalty. The server calculates the penalty again at each equip change (`refreshExpertisePenalty`):

```
weaponPenalty = clamp(maxEquippedWeaponGradeId - expertiseLevel - bonus, 0, 4)
armorPenalty  = clamp(maxEquippedArmorGradeId  - expertiseLevel - bonus, 0, 4)   // armor, shield, jewelry; arrows exempt
```

Each penalty level that is not zero applies a hidden passive:

| Penalty level | Weapon Grade Penalty (skill 6209) | Armor Grade Penalty (skill 6213) |
|---|---|---|
| 1 | Accuracy -16 (flat), crit rate x0.9, crit dmg x0.9, atk.spd x0.9, P.Atk x0.9 | Evasion -2.5, atk.spd / cast.spd / run speed x0.8333 |
| 2 | Accuracy -16, crit x0.8, crit dmg x0.8, atk.spd x0.9, P.Atk x0.9 | Evasion -5, speeds x0.6944 |
| 3 | Accuracy -16, crit x0.7, crit dmg x0.7, … | Evasion -7.5, speeds x0.5787 |
| 4 | Accuracy -16, crit x0.6, crit dmg x0.6, … | Evasion -10, speeds x0.4823 |

The armor speed multipliers are `1 / 1.2^n`. In Interlude, the penalty was accuracy/evasion only (one level for each grade above the expertise). For this reason, "twinks" in over-grade gear were effective in Interlude, but not in Gracia+.

### 2.4 Consumables

**Soulshots / spiritshots / blessed spiritshots** are stackable `EtcItem`s. Their handler uses `weapon.soulshots` (or `spiritshots`) units and sets a charged flag. The next attack or spell uses this flag:

| Shot | Effect (H5 L2J) | NPC price per unit, by grade (NG/D/C/B/A/S) |
|---|---|---|
| Soulshot | Normal hit: P.Atk x2 (`ssboost = 2`). Physical skills: `77 * (power + P.Atk * 2) / P.Def` (the Interlude formula used x1.458 in skills) | 7 / 14 / 22 / 50 / 80 / 100 |
| Spiritshot | M.Atk x2 (damage ∝ sqrt(M.Atk), thus ≈ x1.41 damage); casting time x0.6 | 15 / 18 / 35 / 100 / 120 / 150 |
| Blessed Spiritshot | M.Atk x4 (≈ x2 damage); casting time x0.6 | 35 / 28 / 42 / 245 / 290 / 350 |

A shot must match the grade of the weapon. Monsters also have `shotChance`/`spiritChance` (for example 30%) to fire their own shots.

**Potions**: Healing Potion (+ HP over time) and the Greater/Quick variants. Mana Potions were *not* in Interlude retail (private servers added them). **Scrolls**:

- Scroll of Escape (teleport to the nearest town, cast time 20 s, damage interrupts it).
- Scroll of Resurrection (gives back a fraction of XP, by scroll tier).
- Blessed Scroll of Escape.
- Enchant scrolls (next section).

### 2.5 Enchanting

These rules are the same in all eras:

- The scroll must match the item grade and category (weapon or armor; jewelry uses armor scrolls).
- **Safe enchant: +3** for all items, **+4 for full-body armor** (`onepiece`). Below the safe level, success is 100%.
- A success increases the enchant by 1. The server broadcasts at armor +6 / weapon +7 and at +15.
- **Failure with a normal scroll**: the item is destroyed, and the server changes it into crystals of its grade (if `crystallizable`). The L2J count is `crystals = item.crystalCount(with enchant bonus) - (baseCrystalCount + 1) / 2`, minimum 1. For enchant > 3, the "with enchant bonus" term is `base + crystalEnchantBonus * (enchant - 3)`. Thus a +10 B weapon gives back many more B crystals than a +0 B weapon.
- **Failure with a blessed scroll**: the item is not destroyed, and the enchant goes back to +0 (the item is also unequipped). The list price of Blessed Scroll: Enchant Weapon (A) is 15,000,000 adena. The list price of the normal A scroll is 1,800,000.
- **Crystal scrolls** (Interlude/C4 era): after a failure, the item stays at its current level. Later versions removed these scrolls.
- When armor gets to +4 while it is equipped, it gives its `enchant4Skill` (for example +4 Doom helmet set bonuses). Full sets at +6 give `enchant6skill`.

The success chance is different for each era:

| Era | Weapon (after safe) | Armor / jewelry (after safe) |
|---|---|---|
| Interlude (community-reported retail) | flat ~66% | flat ~66% |
| L2J Interlude fork defaults | 68% | armor 52%, jewelry 54% |
| H5 L2J (`enchantItemGroups.xml`) | Fighter weapons: 70% for +3→+15, 35% after that. Mage weapons: 40% for +3→+15, 20% after that | +3→+4 66.67%, then `100/(n-1)`: 33.34, 25, 20, 16.67, 14.29, 12.5, 11.12, 10, 9.1, 8.34, 7.7, 7.15, 6.67, 6.25, 5.89, 5.56 (+19→+20), 0 after that. Full body moves one step (100% to +4) |

Stat bonus for each enchant level (`FuncEnchant`). For the base term, `enchant` has a cap of 3. The doubled term is `over = enchant - 3`:

| Stat | Grade | Per level ≤+3 | Per level >+3 |
|---|---|---|---|
| P.Def / M.Def (any armor, jewelry) | all | +1 | +3 |
| M.Atk (any weapon) | S | +4 | +8 |
| | A, B, C | +3 | +6 |
| | D, NG | +2 | +4 |
| P.Atk, 1H (sword, blunt, dagger, pole*, other) | S / A / B-C / D-NG | +5 / +4 / +3 / +2 | +10 / +8 / +6 / +4 |
| P.Atk, 2H (2H sword, 2H blunt, dual, dualfist) | S / A / B-C / D-NG | +6 / +5 / +4 / +2 | +12 / +10 / +8 / +4 |
| P.Atk, bow | S / A / B-C / D-NG | +10 / +8 / +6 / +4 | +20 / +16 / +12 / +8 |

(*L2J sets poles as `lrhand`, thus they use the 2H row. Retail descriptions put spears in the 1H row. Nightfall must select one of the two.) Example: Bow of Peril (B) at +12 gets `6*3 + 12*9 = 126` P.Atk on a base of 400. Armor at +6 gets `3 + 3*3 = 12` P.Def for each piece. At +3 it gets only 3. For this reason, +3 is cheap and +6 is the first important milestone.

### 2.6 Augmentation (life stones)

Augmentation adds two hidden "options" to a C+ weapon. Each option is an id in a 16-bit field, stored as `(stat34 << 16) | stat12`. Augmentation uses a Life Stone and Gemstones, and it has **100% success**. To remove an augmentation, the character pays adena by grade. The removal clears the two options.

| Life Stone grade | NPC reference price (lvl 46 stone) | L2J skill chance (`AugmentationXXSkillChance`) | Glow chance | Retail-like colour split yellow/blue/purple/red |
|---|---|---|---|---|
| No-grade (Life Stone) | 5,000 | 15% | 0% | 55 / 35 / 7 / 3 |
| Mid-grade | 20,000 | 30% | 40% | same |
| High-grade | 200,000 | 45% | 70% | same |
| Top-grade | 1,000,000 | 60% | 100% | same |

Purple and red results always have a skill. Thus the "retail-like" mode gives ~10% skill chance at all grades. The grade controls mostly the *strength* of the stat roll and the glow. The offset is a `lifeStoneLevel * 91` sub-block in a 3640-entry block for each colour. The life stone **level** (46, 49, 52, 55, 58, 61, 64, 67, 70, 76, then 80/82/84) must be ≤ the level of the character. This level scales the magnitude of the stat options. Base-stat modifiers (+1 STR/CON/INT/MEN) have a flat 1% chance, and the grade has no effect on this chance.

Option pools (yellow = plain stat, blue = better stat, purple = passive/chance skill, red = active skill):

- Stats: P.Atk, M.Atk, P.Def, M.Def, Max HP/MP/CP, HP/MP/CP regen, Accuracy, Evasion, Crit rate, Atk.Spd, Cast.Spd, Speed, STR/CON/INT/MEN +1.
- Passive skills: Reflect damage, Heal/Refresh/Duel Might (party or self stat %), Prayer, Agility, Empower, Might, Shield, and other skills.
- Chance skills (proc when the character hits or gets a hit): Poison, Stun, Paralyze, Silence, Hold, Heal, Refresh.
- Active skills (castable): Heal, Celestial Shield, Blessed Body/Soul, Haste, Might, Empower, Wild Magic, Cheap Shot, Guidance, Focus, Shield, Agility, Duel Might, Reflex, Stun, Blessed Escape, Bleed, Poison, Fire/Ice/Wind bolts, Hydro Blast, and other skills.

Gemstone cost for each attempt (`getGemStoneCount`): C 20 Gemstone D, B 30 Gemstone D, A 20 Gemstone C, S 25 Gemstone C (S80/S84 36 Gemstone B). Accessory augmentation (Freya+) uses accessory life stones and 200-480 gemstones.

### 2.7 Special abilities (soul crystals)

A character can exchange a weapon of C grade or higher for an SA variant at a Blacksmith of Mammon (for example `Sword of Damascus - Focus`, `item_skill 3010-6`). The character pays with the base weapon, a Soul Crystal of the matching stage and colour, Gemstones, and Ancient Adena. These are the SA names in the datapack (the counts are the number of weapon variants): Focus (240), Health (226), Haste (151), Mana Up (69), Acumen (66), Cheap Shot (61), Critical Damage (59), Light (58), Guidance (56), Anger (54), HP Drain (40), Critical Stun (39), Conversion (35), Magic Hold (33), Critical Poison (31), Critical Bleed (30), HP Regeneration (28), Critical Drain (28), Rsk. Focus (27), Critical Slow (25), Rsk. Haste (24), MP Regeneration (24), Evasion (23), Rsk. Evasion (22), Quick Recovery (21), Mental Shield (18), Back Blow (18), Magic Silence (17), Empower (17), M. Atk. (15), Great Gale (15), Towering Blow (14), Thunder (13), Miser (12), Magic Weakness (12).

The usual SA for each weapon type:

- Swords: Focus/Haste/Health (1H) or Focus/Health/Critical Damage (2H).
- Daggers: Focus/Back Blow/Critical Damage or Critical Bleed/Poison.
- Bows: Cheap Shot/Focus/Critical Slow or Guidance/Light/Miser.
- Blunts: Anger/Health/Rsk. Focus.
- Mage staves: Acumen/Conversion/Mana Up or Empower/Magic Hold/Magic Silence.
- Duals: Focus/Health/Critical Damage.
- Poles: Guidance/Light/Haste or Anger/Critical Stun/Towering Blow.

Soul crystals come in Red / Green / Blue, stages 1-13 (H5 extends to 16). The necessary stage by grade: C needs 7-8, B 9-10, A 11-12, S 13. A stage 1-10 crystal gets a higher level when the character kills specific monsters while the crystal is in the inventory. For each NPC, `levelUpCrystalData.xml` gives these data:

- The stage that the NPC can raise.
- The absorb mode, `LAST_HIT` or `FULL_PARTY`.
- If the Dwarf skill is necessary.
- A chance (default 5%).

A stage 11+ crystal must have raid bosses with `FULL_PARTY` absorb. At a high stage, a failed absorb can **break** the crystal into a broken crystal. This is the main sink for the SA market.

### 2.8 Item identity, flags, inventory and storage

Template flags (parsed in `L2Item`): `is_stackable` (default false), `is_sellable`, `is_droppable`, `is_destroyable`, `is_tradable`, `is_depositable` (default true), `is_questitem`, `is_freightable`, `enchant_enabled`, `element_enabled`, `duration` (minutes, shadow items), `time` (seconds, time-limited items), `crystal_type`, `crystal_count`, `price` (reference price for NPC sell = `price / 2`), `weight` (in 1/1000 units, thus a short sword with 1,600 "weight" weighs 1.6 kg). Each instance has an `object_id` (32-bit, unique in the world) that is different from the template `item_id`. A stackable is one instance with a count.

In classic L2, "Soulbound" in the retail sense (bind-on-equip) does not exist. An item is untradeable only if its template is untradeable (`is_tradable=false`, for example quest rewards, hero weapons, PvP items, Common Items). **Common Items** are crafted variants with a lower cost (for example `Common Item - Sword of Damascus`). They have the same stats as the original, but they have `crystal_count` 82 (the original has 1346). A character cannot enchant or augment them, and they are untradeable. **Shadow items** have a `duration`, and they disappear when it expires.

Capacity (character.properties):

| Container | Non-Dwarf | Dwarf | Notes |
|---|---|---|---|
| Inventory slots | 80 | 100 | GM 250; quest items have a separate 100-slot pool; client crashes above ~300 |
| Private warehouse | 100 | 120 | |
| Clan warehouse | 200 (150 in Interlude) | | Withdrawal is possible only with the clan privilege |
| Freight | 200 (20 in Interlude) | | 1,000 adena for each deposited item; moves items between characters of one account |
| Private store sell / buy slots | 3 / 4 | 4 / 5 | Interlude: 4 / 5 |
| Max adena | 99,900,000,000 | | for each container |

**Weight limit** (`getMaxLoad`): `floor(CON_bonus(CON) * 69000) * WeightLimitMultiplier`. `CON_bonus` is the same table that HP uses (≈1.0 at CON 43, which gives ~69,000 weight units ≈ 69 kg). Items also have a `weightLimit` stat hook, thus belts/passives can add to it. The server calculates the overweight penalty again at each inventory change (`refreshOverloaded`), with `load_pct = (currentLoad - bonusWeightPenalty) / maxLoad`:

| Load | Penalty level (skill 4270) | HP/MP regen | Run speed | Other |
|---|---|---|---|---|
| < 50% | 0 | — | — | |
| 50-66.6% | 1 | x0.5 | x1.0 | |
| 66.6-80% | 2 | x0.5 | x0.5 | |
| 80-100% | 3 | x0.5 | x0.5 | |
| ≥ 100% | 4 | x0.5 | x0 | `isOverloaded`: cannot attack, cast or pick up |

Dwarves get a higher base CON and the Dwarven "weight limit" passives (`weightLimit` stat). They do not use a different formula.

## 3. Design decisions for Nightfall

1. **Keep the six-grade ladder and the level bands** (NG 1-19, D 20-39, C 40-51, B 52-60, A 61-75, S 76+). Reason: the ladder is the backbone of the demand curve of the economy (Phase 5). Its vertical steps make crafting and enchant markets for each band. S80/S84 are not in the plan.
2. **Grade penalty = Gracia+ model, not Interlude**: separate weapon/armor penalty levels 0-4 with the multiplicative speed penalties from 2.3. This closes the "twink" loophole and makes expertise important. Penalty effects are data-driven buffs (Phase 3 status system), not hard-coded.
3. **Weapon type constants are data, not code**: base attack speed, crit, random damage, range, shot count, and MP cost are in the template file. The 2.1 table gives the defaults. Nightfall uses the 1H row for poles (retail description), not the 2H row of L2J.
4. **Armor type tradeoff through class passives**, the same as L2. Thus a class can "wear anything", but then it loses its mastery. Phase 2 defines which classes get which mastery.
5. **Enchant model**: safe +3 (+4 full body). After the safe level, armor uses an H5-style curve for each level (`100/(n-1)` from +4). Weapons use a curve that is flat and then halved (70%/35% fighter, 40%/20% mage). We considered the flat Interlude 66%, but we rejected it. The reason is that the armor curve gives a natural ceiling without a hard cap. Blessed scrolls set the item back to +0; normal scrolls crystallize the item. Weapons have no maximum enchant. Armor has a cap at +20 (0% beyond +19), as in the H5 table.
6. **Enchant bonus table** is the FuncEnchant table word for word (2.5). It is stored as data, with `(stat, grade, weapon_class)` as the key.
7. **Augmentation**: use the two-option model with 100% success, where the grade controls glow/strength. But use the "retail-like" 55/35/7/3 colour split, thus skill augments stay rare (~10%) at all grades. The life stone level controls the magnitude. The character pays for removal in adena (Phase 5 sink).
8. **Special abilities**: implement SA as a *template variant* (a separate `item_id`, as L2 does), not as an instance modifier. This is simpler for the client (icon/name for each variant) and for the economy (SA weapons are different market items). Soul crystal leveling is a Phase 6 monster-side function. This phase only defines the crystal items and the exchange.
9. **Item identity**: `ItemTemplate` (static, loaded from TOML, hot-reloadable in dev) and `ItemInstance` (Postgres row with `instance_id` UUIDv7). A stackable is one row with `count`. A non-stackable always has `count = 1`. Binding flags come from the template. Nightfall adds one instance-level flag, `bound_to_character`, for future bind-on-pickup rewards (default false, Phase 4 never sets it).
10. **Inventory limits**: 80 slots (+20 Dwarf), weight = `floor(CON_bonus * 69000)`, overweight levels as in 2.8. Warehouse 100/120, clan 200, freight 200 at 1,000 adena for each item. These are config values, not constants.
11. **Shots**: soulshot x2 P.Atk on hits, spiritshot x2 M.Atk, blessed x4 M.Atk and cast time x0.6. The grade of all shots must match. Auto-use is a client intent. The server validates grade, count, and charge state.
12. **No item durability / repair**: the same as L2. Enchant failure and crystallization are the item sink (Phase 5).

Alternatives that we considered:

- (a) One "item level" instead of grades. We rejected it, because grades give discrete market tiers.
- (b) Deterministic enchant with pity counters. We will decide this during tuning (Phase 9 telemetry).
- (c) Bind-on-equip for raid loot. We will decide this later (open question 8.4).

## 4. Data model

### 4.1 Entities

- **ItemTemplate**: a definition that does not change. `template_id: u32` identifies it (stable, never used again). Variants (SA, Common Item) are their own templates, with `base_template_id` to put them in groups for the display.
- **ItemInstance**: a row that a character owns: `instance_id`, `template_id`, `owner_character_id`, `location` (INVENTORY, EQUIPPED, WAREHOUSE, CLAN_WAREHOUSE, FREIGHT, MAIL, TRADE_ESCROW, GROUND), `slot`, `count`, `enchant`, `augment_a`, `augment_b`, `expires_at`, `bound_to_character`, `created_at`, `updated_at`, `version` (optimistic lock).
- **EquipSlot**: enum: RHAND, LHAND, LRHAND (virtual: uses the two hands), HEAD, CHEST, LEGS, FULLBODY (virtual: chest+legs), GLOVES, FEET, NECK, EAR_L, EAR_R, FINGER_L, FINGER_R, UNDERWEAR, CLOAK (reserved), BELT (reserved).
- **ArmorSet**: a list of template ids for each slot, the full-set skill id, the +6 skill id.
- **EnchantRateGroup** / **EnchantBonusTable** / **ExpertiseTable** / **WeightPenaltyTable**: data only.

### 4.2 Template definition (TOML)

There is one file for each grade and category in `packages/data/items/`, for example `weapons_b.toml`:

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

Stack merge rule: when the server adds a stackable template to a container, it does `UPDATE ... SET count = count + $n WHERE owner_id=$o AND template_id=$t AND location=$l AND enchant=0`. If no row matches, it does `INSERT`. For non-stackables, the server always does an insert.

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

Add these RPCs to `GameService`: `GetInventory`, `Equip`, `Unequip`, `UseItem`, `DestroyItem`, `Enchant`, `Augment`, and a server-streaming `SubscribeInventory(stream InventoryUpdate)`. Use the stream RPC until the general world-event stream exists (Phase 0).

## 5. Interfaces

**gRPC** (above). All mutating RPCs are idempotent for each `(character_id, request_id)` for 60 s. Thus client retries are safe.

**Server events** (internal bus, Phase 0): `ItemAdded`, `ItemRemoved`, `ItemEquipped`, `ItemUnequipped`, `EnchantResolved{seed, outcome}`, `AugmentResolved`, `WeightPenaltyChanged`, `GradePenaltyChanged`. Combat (Phase 3) subscribes to equip/unequip and penalty events, and then builds the stat sheet again. Economy telemetry (Phase 5/9) subscribes to all events.

**Validation on equip** (server-side, in this sequence):

1. The character owns the item, and the item is in the inventory.
2. The template `kind` is equipable.
3. The class/race restrictions from the template apply (Phase 2 hook).
4. A two-handed weapon unequips the shield.
5. FULLBODY unequips chest+legs.
6. A ring/earring goes into the free slot, or it swaps the left slot.
7. Arrows must match the bow grade.
8. Then the server calculates the grade penalty, weight penalty, and set bonuses again, and emits one `InventoryUpdate`.

**Client needs**: a static item catalogue (`template_id → name, icon, grade, kind, display stats`). The build (moon task) makes this catalogue as JSON from the TOML, and the client gets it with the build. Thus `Item` messages stay small.

## 6. Rust implementation notes

Module layout in `apps/api/src/items/`:

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

Concurrency: the authoritative inventory for an online character is in the actor task of that character (one `tokio` task for each session, Phase 0). Thus inventory mutations are single-threaded for each character, and they do not need locks. Moves between characters (trade, mail, Phase 5) go through Postgres transactions. These transactions use `SELECT ... FOR UPDATE` on the two `item_instances` rows, and the `version` check. Only repo transactions touch offline containers (warehouse, freight).

RNG: each enchant/augment roll uses `ChaCha12Rng::from_seed(hash(server_secret, character_id, request_id))`. The server writes the seed to `item_ledger.rng_seed`, thus a GM tool can replay each outcome. Do not use thread RNG for rolls that players can see.

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

Derived-stat contribution (Phase 3 uses it): `items::equip::StatContribution { p_atk, m_atk, p_def, m_def, atk_spd_base, crit, random_dmg, range, accuracy_mod, evasion_mod, max_mp, penalty_mults }`. The server calculates it one time for each equipment change, and caches it on the character.

## 7. Client implications

- Render the inventory as an 80/100-slot grid, and an equipment paper-doll with the 14 slots. Show the items above the expertise in grey, and show the penalty level badge (`weapon_grade_penalty`, `armor_grade_penalty` from `Inventory`).
- Show a weight bar with colour thresholds at 50/66/80/100%. Show the regen/speed penalty text from a client-side table.
- Enchant UI: the player drags a scroll onto an item. Show the safe level and the outcome that the server returns. Animate the +N glow tiers. In L2, the weapon glow starts at +4 and increases at +7/+10/+13. Use a Phaser tint/particle for each tier.
- Shots: show toggle buttons for each shot type. The client sends `UseItem` one time to toggle auto-use. The server consumes shots for each attack, and streams count deltas (in batches, not for each hit).
- Tooltips calculate display values (base + enchant bonus) from the catalogue and the `enchant` field. They use the same table as the server. Thus the client table and the server table must come from one TOML.
- Augmented weapons show option names from a client-side option catalogue, with the option id as the key.

## 8. Open questions

1. Expertise bonus (`getExpertisePenaltyBonus`): in L2, some Dwarf/Kamael skills gave +1 expertise. Do some Nightfall classes get it (Phase 2)?
2. Weapon enchant cap: no cap (H5), or a hard cap at +16/+20? This depends on Phase 5 sink tuning.
3. Pole enchant row (1H from the retail text, or 2H from L2J). The decision above is 1H. Make sure that this decision agrees with the combat balance in Phase 3.
4. Bind-on-pickup for raid loot: the schema supports it (`bound_to`), but it is not used. Decide with the Phase 6 raid design.
5. The schedule of augment removal fees by grade, and if removal gives back the life stone (L2: no).
6. Soul crystal break chance and stage ladder: make the final decision with the Phase 6 monster tables.
7. Mana potions: L2 Interlude retail had none; many private servers add them. The Phase 3 MP economy decides.
8. Do `Common Item` variants exist in Nightfall, or does crafting always give the full item (Phase 5)?
9. Shadow (time-limited) items: keep `expires_at`, but no content is in the plan. Make sure of this before you build expiry sweeps.

## 9. Sources

- L2J Server (High Five) game source, Bitbucket `l2jserver/l2j-server-game` (checked out 2026-09-13): `model/stats/functions/formulas/FuncEnchant.java` (enchant bonus table), `model/actor/instance/L2PcInstance.java` (`refreshOverloaded`, `refreshExpertisePenalty`, `getMaxLoad`), `model/items/type/CrystalType.java`, `model/items/L2Item.java` (template flags), `network/clientpackets/RequestEnchantItem.java` and `model/items/enchant/EnchantScroll.java` (enchant resolution, crystallization), `network/clientpackets/AbstractRefinePacket.java` (life stone table, gemstone costs), `data/xml/impl/AugmentationData.java` (option generation), `network/clientpackets/RequestCrystallizeItem.java`, `model/stats/Formulas.java` (shot multipliers), `src/main/resources/config/character.properties` (inventory, warehouse, freight, store slots, augmentation chances, expertise penalty). https://bitbucket.org/l2jserver/l2j-server-game
- L2J Datapack (High Five), Bitbucket `l2jserver/l2j-server-datapack`: `data/stats/items/*.xml` (we examined all weapon/armor/jewelry/consumable templates for the type table), `data/enchantItemGroups.xml` (enchant rate curves), `data/enchantItemData.xml` (scroll grades), `data/stats/skills/06200-06299.xml` (skills 6209/6213 grade penalty), `data/stats/skills/04200-04299.xml` (skill 4270 weight penalty), `data/stats/skills/00200-00299.xml` (armor masteries 227/231/233, weapon mastery 249, Create Item 172), `data/stats/armorsets/*.xml`, `data/levelUpCrystalData.xml` and quest `Q00350_EnhanceYourWeapon` (soul crystal leveling). https://bitbucket.org/l2jserver/l2j-server-datapack
- L2JBrasil Interlude server fork (GitHub `L2jBrasil/Server-Interlude`, 2018): `Config.java` (Interlude defaults: EnchantChanceWeapon 68, Armor 52, Jewelry 54, EnchantSafeMax 3, EnchantSafeMaxFull 4, warehouse 100/120/150, freight 20, store slots 5/4), `RequestEnchantItem.java` (flat chance, full armor +4 safe, crystal scrolls). https://github.com/L2jBrasil/Server-Interlude
- l2hub.info item database, "Scroll: Enchant Weapon (Grade S)" description (per-type P.Atk bonus and doubling from +4). https://l2hub.info/c4/items/scrl_of_ench_wp_s
- ludo.guide, "Enchanting / Upgrading equipment" and "Augmenting / Life Stones" (Lineage II). https://www.ludo.guide/guide/lineage-ii/enchanting-augmenting/enchanting-upgrading-equipment , https://www.ludo.guide/guide/lineage-ii/enchanting-augmenting/augmenting-life-stones
- L2DB.net guides, "Best Bows in Lineage 2: Interlude (CT0)" (bow MP consumption and shot usage context). https://l2db.net/guides/en/best-bows-in-lineage-2-interlude-ct0-l2dbnet-2
- Lineage II Classic 1.5 patch notes (basic combat stats glossary). https://www.lineage2.com/news/lineage-ii-classic-launch-patch-notes
