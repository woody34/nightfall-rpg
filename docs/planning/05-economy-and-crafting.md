# Phase 5: Economy and Crafting

## 1. Purpose and scope

Phase 5 defines how items come into the world, move in the world, and go out of the world:

- **Sources**: monster drops (death drops and spoil/sweep), adena drops, raid and champion multipliers, level-gap penalties, party loot distribution.
- **Transformation**: the Dwarven recipe system (recipe books, Create Item skill level, materials, success chance, MP cost). Material chains from raw materials to key materials to finished goods. Masterwork/rare results.
- **Currencies and sinks**: adena, and Ancient Adena and the Seven Signs cycle as a reference for a seasonal economy. Teleport fees, enchant/augment costs, taxes, NPC shop spreads.
- **Exchange**: NPC vendors and multisell, castle tax, and player-to-player trade (direct trade, private stores, package sale, offline stores). Mail with attachments and cash-on-delivery, and the auction house of later chronicles.
- **Health**: inflation controls, bot and RMT threats, mudflation, and the telemetry that is necessary to tune all of these items.

These items are not in the scope of this phase:

- The item data model (Phase 4).
- Monster placement and the AI that decide *which* monsters exist (Phase 6).
- Clan treasuries and siege rewards (Phase 7).
- The GM tooling that acts on telemetry (Phase 9).

## 2. Reference: how Lineage 2 does it

### 2.1 Drop tables: the group model

Interlude-era L2J kept drops as rows `(mobId, itemId, min, max, category, chance)`, with `chance` on a 1,000,000 scale. Category `-1` was "sweep only". Category `0` was adena. Categories `1..n` were independent groups. Example (Interlude `droplist.sql`, mob 20543):

```
(20543, 57,   105, 183, 0,  700000)  -- Adena: 70%, 105-183
(20543, 309,  1, 1,  1,  99)         -- category 1: Tears of Eva
(20543, 310,  1, 1,  1,  99)         --             Relic of the Saints
(20543, 1867, 1, 1,  2,  35826)      -- category 2: Animal Skin 3.58%
(20543, 1872, 1, 1,  2,  35826)      --             Animal Bone
(20543, 1882, 1, 1,  2,  5971)       --             Leather
(20543, 2139, 1, 1,  2,  5374)       --             Recipe: Steel Mold
(20543, 6037, 1, 1,  2,  53739)      --             Waking Scroll
(20543, 908,  1, 1, -1,  3822)       -- spoil: Necklace of Wisdom 0.38%
(20543, 1882, 1, 1, -1,  50535)      -- spoil: Leather 5.05%
```

High Five L2J replaced this with explicit groups in the NPC XML. Nightfall copies this model:

```xml
<dropLists>
  <death>
    <group chance="70">
      <item id="57" min="236" max="471" chance="100" />              <!-- Adena -->
    </group>
    <group chance="0.8385">
      <item id="433"  min="1" max="1" chance="1.1833" />            <!-- Elven Tunic -->
      <item id="1936" min="1" max="1" chance="37.2337" />           <!-- White Tunic Pattern -->
      <item id="1933" min="1" max="1" chance="53.5289" />           <!-- Dark Stockings Fabric -->
      <!-- ... item chances inside a group sum to 100 -->
    </group>
    <group chance="7.6011">
      <item id="1875" min="1" max="1" chance="2.9741" />            <!-- Stone of Purity -->
      <item id="955"  min="1" max="1" chance="0.1784" />            <!-- Scroll: Enchant Weapon (D) -->
      <item id="1834" min="1" max="1" chance="49.5692" />           <!-- Emergency Dressing -->
    </group>
    <group chance="42">                                              <!-- herbs: pick-up-only, auto-consume -->
      <item id="8600" min="1" max="1" chance="55" />
      <item id="8601" min="1" max="1" chance="38" />
      <item id="8602" min="1" max="1" chance="7" />
    </group>
  </death>
  <corpse>  <!-- spoil -->
    <group chance="30"> <item id="1882" min="1" max="2" chance="100" /> </group>
  </corpse>
</dropLists>
```

Resolution algorithm (`IGroupedItemDropCalculationStrategy.DEFAULT_STRATEGY`):

1. For each group, independently: calculate the **effective group chance** = `groupChance * killerLevelModifier * rateMultiplier(champion, raid, item class)`. L2J does this when it "normalises" the group. It adds `item.chance * groupChance * modifiers / 100` for all the items, and uses that sum as the group roll. Then it rescales the item chances, thus their sum is 100 again.
2. Roll `u ∈ [0,100)`. If `u < groupChance`, roll a second `u2`. Then go through the cumulative chances of the items to select exactly one item.
3. Quantity = `uniform_int(min, max)`, scaled by the amount multiplier.
4. **Precise drop calculation** (chances > 100% after rate multipliers): `amount *= floor(chance/100)`, plus one more with probability `chance mod 100`. An alternative config rolls the group `floor(chance/100)` more times, and can also aggregate stacks.
5. A group with only one item becomes a plain roll at `item.chance * groupChance / 100`.

Each group gives a maximum of one item for each kill. Thus designers use the number of groups to control "how many things drop". They use the item split in the group to control "which thing". Adena always has its own group.

**Rates and multipliers** (`rates.properties`, `customs.properties`):

- `DeathDropChanceMultiplier` / `DeathDropAmountMultiplier` (default 1).
- `CorpseDrop*` for spoil, `RaidDrop*`, `HerbDrop*`.
- An override list for each item id (`57,1` for adena).
- Champion mobs (`ChampionHp 8x`, `ChampionRewardsChance 8x`, `ChampionRewardsExpSp 8x`, `ChampionAdenasRewardsChance 1x`, frequency and level window 20-78, configurable).

Raid bosses use `RaidDropChanceMultiplier`, and the normal level-gap rule does not apply to them.

### 2.2 Level-gap ("deep blue") penalty

Interlude L2J follows the Prima guide. A monster is "deep blue" when the highest-level attacker is 9+ levels above it. Then the server subtracts `levelModifier = (highestLevel - (mobLevel + 8)) * 9` percent from each chance. The server also divides adena by 3 (and by the server drop rate). Thus 9 levels above: -9%, 10: -18%, …, ~19 levels: 0%.

High Five L2J (`npc.properties`, `IKillerChanceModifierStrategy`) uses linear interpolation from 100% down to a floor. The interpolation is over a window of level difference `d = mobLevel - killerLevel`:

| Item class | Full chance until `d ≥ -min` | Floor reached at `d ≤ -max` | Floor |
|---|---|---|---|
| Items | 5 levels above | 10 levels above | 10% |
| Adena | 8 levels above | 15 levels above | 10% |
| Raid bosses (`UseDeepBlueDropRulesRaid`) | `clamp(1 + 0.15 * d, 0, 1)`: a raid 7+ levels below the killer drops nothing | | 0% |

The "killer level" is the level of the party member who did the most damage, or of the last hitter. Interlude used the highest-level attacker in the aggro list. This rule stopped high-level characters when they tried to "feed" kills.

### 2.3 Spoil and sweep

Dwarf Scavenger line: **Spoil** (skill 254) marks a monster as spoiled by the caster. The skill data:

- 11 levels, lvl 10-72, MP 12-67, reuse 3 s, cast 1.8 s, effect power -138 to -646.
- The success roll uses the standard debuff land-rate formula against the level and MEN of the monster.

When a spoiled monster dies, the server rolls the `<corpse>` drop list one time and keeps the result on the corpse (`_sweepItems`). Then the spoiler uses **Sweeper** (skill 42) to move those items. A party member of the spoiler can also do this under an "including spoil" distribution mode. The corpse must still exist (decay 7 s default, +10 s extension for spoiled corpses). At mid levels, most key materials and enchant scrolls are in the spoil lists. For this reason, Dwarves are central to the economy.

### 2.4 Adena drops and party distribution

Adena is a stackable group with `chance` 70% and a `min..max` that scales with level. Examples: mob 20543 lvl ~20: 105-183; mob lvl ~40: 236-471; level 60 mobs drop ~1,000-2,000; raids drop tens of thousands. The server rate `RateDropAdena` multiplies the amount.

**Party loot modes** (`PartyDistributionType`): `FINDERS_KEEPERS` (0), `RANDOM` (1), `RANDOM_INCLUDING_SPOIL` (2), `BY_TURN` (3), `BY_TURN_INCLUDING_SPOIL` (4). Rules:

- **Adena** is split equally among the members in `PartyRange2` (1,400 units) of the corpse. As an option, all stackables are split too (`PartyEvenlyDistributeAllStackableItems`). The remainder goes to the picker.
- Other items go to `getActualLooter`: the picker (finders keepers), a random eligible member, or the next member in rotation (by turn). Eligibility = in range, alive, has an inventory slot and weight. Spoil items rotate only under the "including spoil" variants. In the other modes, they go to the Dwarf.
- The party XP/SP cutoff (`PartyXpCutoffGaps 0-9: 100%, 10-14: 30%, 15+: 0%`) is the social complement that stops high-level carries.
- Raid loot rights: the command channel or party with the most damage gets a 5-minute pickup priority (`RaidLootRightsInterval 900000` ms, `RaidLootRightsCCSize 45`).

### 2.5 Crafting: the Dwarven recipe system

Recipes are items (`Recipe: X`). When a Dwarf uses a recipe, the server registers it into the **recipe book** of the Dwarf. The limits are: Dwarven book 50, Common book 50 (`DwarfRecipeLimit/CommonRecipeLimit`). The registration consumes the recipe item. Each recipe (`recipes.xml`) has:

```xml
<item id="138" recipeId="5008" name="mk_pata_i" craftLevel="6" type="dwarven" successRate="60">
  <ingredient id="5008" count="1" />    <!-- the recipe itself? no: 5008 is Recipe: Pata; key ingredient list follows -->
  <ingredient id="4103" count="12" />   <!-- Pata Blade (key material) -->
  <ingredient id="1890" count="70" />   <!-- Mithril Alloy -->
  <ingredient id="1888" count="70" />   <!-- Synthetic Cokes -->
  <ingredient id="1885" count="35" />   <!-- High-Grade Suede -->
  <ingredient id="4042" count="35" />   <!-- Enria -->
  <ingredient id="1459" count="310" />  <!-- Crystal (C-Grade) -->
  <ingredient id="2132" count="55" />   <!-- Gemstone B -->
  <production id="264" count="1" />
  <statUse name="MP" value="171" />
</item>
```

Rules:

- **Create Item** (skill 172, 10 levels at class levels 5/20/28/36/43/49/55/62/70/82) must be ≥ `craftLevel`. Non-Dwarves learn **Create Common Item** (skill 1320) for common recipes only.
- **Success**: `Rnd.get(100) < successRate`. The distribution in the H5 recipe file follows:
  - Craft level 1-5 recipes are 100% (but one special recipe is 25%).
  - From craft level 6 and up, the top-tier weapon/armor of each grade is **60%** (38 of 147 level-6 recipes, 63 of 149 level-7, 35 of 85 level-8, 80 of 121 level-9, all 28 level-10).
  - Some recipes are 70%, and most materials/consumables stay at 100%.
  - Common recipes are 100% (a small number are 95%, 70%, 10%).

  The "60% on the good items" rule is the most important economic parameter, because a failed craft consumes **all** materials.
- **MP cost**: `statUse MP` scales with the recipe tier (30 MP for D-grade arrows/weapons, ~170 MP for C-grade top weapons, 300+ for A/S). Interlude also had `HP` stat use on some recipes.
- **Masterwork** (Gracia+): a `rareItemId` with `rarity` 1/4/20 (%) replaces the normal product after a success. This is not in Interlude.
- **Crafting for other players**: a Dwarf opens a **manufacture shop** (a private store variant) with a list of recipes and a fee. The customer supplies the materials and pays the fee in adena. The roll uses the skill of the Dwarf. The two players get the failure message.

**Material chains**. Raw materials drop in all areas (Iron Ore 1869, Coal 1870, Animal Bone 1872, Stem 1864, Varnish, Charcoal, Suede, Silver Nugget 1873, Mithril Ore 1876, Oriharukon Ore 1874, Adamantite Nugget 1877, Thread, Leather 1882, Animal Skin). Tier-1 recipes (100%) refine them: `Steel 1880 = Iron Ore + Coal`, `Cokes 1879 = Coal + Charcoal`, `Mithril Alloy 1890 = Mithril Ore + Steel`, `Synthetic Cokes 1888 = Cokes + Varnish`, `Oriharukon 1893`, `Durable Metal Plate 5550`, `Metallic Thread 5549`, `Compound Braid 1889`, `Steel Mold 1883`, `Silver Mold 1886`, `Blacksmith's Frame 1892`, `Artisan's Frame 1891`, `Crafted Leather 1894`, `Mold Lubricant 4040`, `Mold Hardener 4041`, `Enria 4042`, `Asofe 4043`, `Thons 4044`. **Key materials** (for example `Sword of Damascus Blade 4114`, `Pata Blade 4103`, `Zubei's Breastplate Part 4056`) drop or spoil from a small number of mobs in the level band of the item. In most conditions, one craft must have 10-15 of them (the 12 Pata Blades above). The finished recipe then needs these materials:

- The key material.
- The refined materials.
- **Crystals of the grade** (hundreds: 310 C-crystals for a C weapon, ~1,300 B-crystals for a B weapon).
- **Gemstones** (D for C/B gear, B for A/S).

Crystals come from the crystallization of surplus gear (Phase 4). Thus each crafted top item consumes many lesser items. The "Crystal + Gemstone" pattern is the built-in sink.

### 2.6 Currency and sinks

- **Adena** (item 57) is the only hard currency. It has a cap of 99,900,000,000 for each container. NPCs buy at `price / 2` of the template reference price, and sell at `price * (1 + castleTax)`. Only drops and quests make adena. NPC purchases, fees, taxes, and some quest turn-ins destroy adena.
- **Ancient Adena** (5575) comes only from the Seven Signs. Players collect seal stones (Blue 3, Green 5, Red 10 Ancient Adena each) in the Catacombs/Necropolis. They trade the stones to a Priest of Dawn/Dusk during the Seal Validation period. "Contribution points" at the same 3/5/10 values decide the winning cabal. Ancient Adena buys these services from the **Merchant/Blacksmith of Mammon**: SA infusion, enchant scroll exchange, weapon exchange between types of the same grade, grade-up exchanges. When `StrictSevenSigns` is on, only the winning cabal can get access to Mammon NPCs.
- **Seven Signs cycle** (`SevenSigns.java`): a one-week period starts on Monday 18:00. **Competition** (quest/recruiting) is ~1 week minus a 15-minute results window. Then **Seal Validation** is ~1 week minus 15 minutes. Dawn requires castle ownership or a participation fee. Dusk is open to players who do not own a castle. Seals:
  - **Avarice** (Mammon access, Ancient Adena exchange).
  - **Gnosis** (buff/teleport perks from the Dawn/Dusk priests).
  - **Strife** (castle gate/wall multipliers 1.1 for Dawn, 0.8 for Dusk). In the Interlude rule set, the castle tax ceiling is 15% when Dawn holds Strife, and 5% for Dusk.

  This is the reference for a *seasonal* economy. It is a two-week cycle that makes a full class of goods (SA, enchant exchange) available and then not available.
- **Teleport fees**: gatekeeper prices scale with distance and destination level: Dion→Giran 6,800; Giran→Oren 9,400; Giran→Hunters Village 4,100; Aden→Ivory Tower 12,000; town↔hunting-ground hops 1,000-4,000 (Interlude `teleport.sql`). Gracia+ made all gatekeeper teleports **free below level 41** (`Teleporter.java: if (player.getLevel() < 41) ammount = 0`). Seven Signs priest teleports are half price. `FreeTeleporting` is a server toggle. For active players, teleport fees are the largest steady adena sink.
- **Enchant scrolls** (NPC reference prices): Weapon D 50,000 / C 110,000 / B 500,000 / A 1,800,000 / S 5,000,000; Armor D 6,000 / C 15,000 / B 80,000 / A 240,000 / S 500,000; Blessed Weapon A 15,000,000. In some chronicles, NPCs sell scrolls only through Mammon/luxury shops. In the other chronicles, scrolls come only from drops. For this reason, scrolls are a tradeable second currency.
- **Augmentation**: life stones 5,000 / 20,000 / 200,000 / 1,000,000 reference price by grade. The removal fee is in adena, by item grade.
- **Shots**: 7-100 adena each, by grade (Phase 4 table). A bow user uses ~50 soulshots each minute. This is the largest sink for each player, and it scales with activity. For this reason, it is effective.
- **Repair**: L2 has no durability or repair. Equipment goes out of the world only through enchant failure, crystallization, and (rarely) PvP/PK drops.
- **Castle tax**: the clan that owns the castle sets `taxPercent` (0-15; the Seal of Strife sets the ceiling). Each NPC merchant in the territory of the castle applies `price * (1 + taxRate)`, and the tax goes to the castle treasury. Aden and Rune take a share of the treasuries of lower castles. The **Manor** system lets castle owners sell seeds and buy crops at the prices that they set. This makes a regional commodity loop.
- **Mail**: 100 adena for each message + 1,000 for each attached item slot. Messages expire in 15 days, and COD mails in 12 hours.
- **Freight**: 1,000 adena for each deposited item.

### 2.7 NPC shops and vendors

- **Buy lists** (`buylists/*.xml`) for each merchant: template ids with an optional restock count/time. Price = reference `price` × (1 + castle tax). The server validates weight and slot (`RequestBuyItem`).
- **Sell**: all `is_sellable` items at `price / 2`, no tax. A **refund** tab (`AllowRefund`) keeps the recent sells until logout. The player can buy them back at the same price.
- **Multisell** (`multisell/*.xml`): item-for-item exchanges (for example Mammon exchanges, grade-up shops), with optional `maintainEnchantment` and tax application. L2 uses multisell to implement vendors that are related to crafting and do not use adena.
- **Luxury shop** (Giran): sells B/A gear at the reference price. It is a large adena sink and the ceiling for player prices.

### 2.8 Player trading

**Direct trade** (`TradeList`): the states for each side are `items[]`, `confirmed`, `locked`. The sequence is:

1. A sends a request, and B accepts. The window opens for the two players, and they cannot move.
2. Each player adds items/adena. Each change calls `invalidateConfirmation()` on *both* lists.
3. Each player pushes Confirm (`confirm()`).
4. When the two players have confirmed, both lists `lock()`.
5. Each side `validate()`s: the owner is still online and still owns each item. Each count is still valid. The items are tradeable and not equipped.
6. Then `doExchange()` examines the weight and slot capacity of the two receivers, and moves the items atomically.

A failure at any step cancels the trade with a message. To prevent deadlock, L2J sets the sequence of the two locks by object id.

**Private store sell / buy** (Interlude: 4 sell / 5 buy slots for Dwarves, 3 / 4 for other races in H5). The seller makes a list of items with prices, and sits. Buyers open the store and buy. The server validates each item again (`privateStoreBuy`). The list must not be locked, and the seller and buyer must be online. The seller must still own the items. The buyer must have the adena, weight, and slots. Then the server moves the items. **Package sale** sells the full list for the sum only. A **manufacture store** gives a list of recipes with fees. Stores show as a title above the character. Offline stores (private-server `OfflineTradeEnable`, later retail "offline shops") keep the character in the world for a maximum of N days.

**Mail with attachments / COD** (Gracia+): a maximum of 8 attachments, and the fee above. COD attaches a necessary adena amount, and the receiver must pay it to take the items. If the receiver does not pay, the mail goes back to the sender when it expires (12 h).

**Auction house** (Goddess of Destruction+, "Auctioneer"/"Item Broker" NPC): a player can put a maximum of N items on the list for 3/7 days. The listing fee is proportional to the price. For sold items, the seller gets the adena through mail. Later chronicles have a cap on the listings for each account, and add a commission. Earlier eras had only two auctions:

- The clan-hall auction (bids of adena for clan halls, a sink).
- The Interlude **Item Auction** (`ItemAuctionEnabled`, a timed NPC auction with 5/3-minute extensions).

### 2.9 Economy health

These are the failure modes that occurred in L2, and the NCsoft responses:

- **Bot farms and RMT** ("rice farmers"): bots run 24/7 in zones with low contest, and they flood the market with adena and raw materials. Material prices fall to the NPC sell price (`price / 2`). But fixed-price sinks (enchant scrolls, shots, teleport) stay constant. Thus the real cost of progression *falls* for RMT buyers and *rises* for all other players. NCsoft responded with mass bans, character-creation limits for each server (Lineage Classic 2024), and lower drop rates on open-world mobs.
- **Mudflation**: when the population gets higher levels, low-grade materials lose all value. Interlude used two solutions. The first was crystallization (each item has the value of its crystals, and crafting needs crystals). The second was the grade ladder (new bands need new materials). Gracia/H5 added item attributes and S80/S84, which moved the sink up. They also added instanced-dungeon drops with weekly limits.
- **Adena faucets without sinks**: raids and quests with large adena rewards had no related sink. Later chronicles added vendors that accept only adena (scroll exchange, buffs, teleport prices that scale with level).
- **Price discovery**: Interlude had no auction house. The private stores in Giran were the market. Offline shops, and later the auction house, made liquidity better, but they also made bot-to-player sales possible.

## 3. Design decisions for Nightfall

1. **Drop model = H5 grouped drops**: a monster has `death` and `corpse` (spoil) lists. Each list is an ordered set of groups. Each group rolls a maximum of one item. Adena is always group 0. The server normalises the item chances in a group at load time. Thus designers can write weights that are not normalised.
2. **Level-gap penalty = H5 linear window** (items 5→10 levels above, floor 10%; adena 8→15, floor 10%). The penalty uses the **highest-level contributor** in the kill (the Interlude rule against feeding). Raids: `clamp(1 + 0.15 * (raidLevel - killerLevel), 0, 1)`.
3. **Party loot**: all five L2 modes. Adena is always split equally among the members in range (1,400 units). A configurable option lets the server "split all stackables". Spoil goes to the spoiler, but not when an "including spoil" mode is active.
4. **Champion mobs**: supported as a multiplier set on the spawn (HP x8, drop chance x8, adena x1), but off by default. They are a tuning lever, not launch content.
5. **Crafting = Dwarven recipe model** with these rules:
   - Recipe books (limit 50 each).
   - The Create Item skill level must be sufficient for `craft_level`.
   - A failure consumes all materials.
   - Each recipe has an MP cost.
   - The start rates are 100% for materials and 60% for top gear of each grade.

   Common items are not in the plan (Phase 4 open question). Masterwork is for a later phase.
6. **Material chains**: three tiers (raw → refined → key material + crystals + gemstones → item). The crystals from crystallization are the intentional item sink. Chains are data, defined one time for each grade band in `recipes_*.toml`.
7. **Currency**: adena only at launch. The design includes a seasonal secondary currency, with Ancient Adena/Seven Signs as the model. But this currency waits for Phase 7, because it needs castles/cabals. The data model keeps `currency_kind` for it.
8. **Sinks at launch**: NPC spreads (`sell = price / 2`), teleport fees (free below 41, then the price changes with distance), shots, enchant failure, augmentation fees, mail fees, warehouse/freight fees, castle tax hook (0 until Phase 7). No durability.
9. **NPC shops**: buy lists with an optional restock, sell at half price, a refund tab for the session, multisell for item-for-item exchanges, a tax hook.
10. **Trade**: direct trade with the two-phase confirm state machine; private sell/buy/package stores; manufacture stores; mail with attachments and COD (12 h). Offline stores and an auction house are **not** in Phase 5 (open questions 8.5, 8.6). The two have very large bot/RMT effects, and they need telemetry first.
11. **All transfers are Postgres transactions** with row locks and version checks. The server copies the result into the in-memory inventories only after commit. Each transfer writes `item_ledger` rows for the two parties.
12. **RNG**: all loot, craft, and store rolls use a seeded `ChaCha12Rng`. The seed comes from `(server_secret, kill_id | craft_request_id)`. The server logs the seed, thus GMs can replay each drop.
13. **Telemetry-first**: each economic parameter in this document is a config value with a telemetry counter next to it (section 6). The launch numbers come from L2. Tuning is the work of Phase 9.

Alternatives that we considered:

- Independent rolls for each item. This is simpler, but it makes a "one of these" design impossible, and it makes drop counts too high at high rates.
- Bind-on-pickup to fight RMT. This only moves the problem to a later time, and it stops the player market. We rejected it for launch.
- Durability as a sink. This adds friction, but it does not give the player the agency of enchanting. We rejected it.

## 4. Data model

### 4.1 Drop table format (TOML, `packages/data/drops/<zone>.toml`)

```toml
[[monster]]
id = 20543
level = 20

# Each group yields at most one item. `chance` is percent of kills in which the group fires.
# `weight` within a group is relative; normalised to 100 at load.
[[monster.death]]
chance = 70
items = [{ id = 57, min = 105, max = 183 }]                       # adena

[[monster.death]]
chance = 0.0198
items = [{ id = 309, weight = 1 }, { id = 310, weight = 1 }]      # Tears of Eva, Relic of the Saints

[[monster.death]]
chance = 13.67
items = [
  { id = 1867, weight = 35826 },   # Animal Skin
  { id = 1872, weight = 35826 },   # Animal Bone
  { id = 1882, weight = 5971  },   # Leather
  { id = 2139, weight = 5374  },   # Recipe: Steel Mold
  { id = 6037, weight = 53739 },   # Waking Scroll
]

[[monster.corpse]]                 # spoil list
chance = 5.05
items = [{ id = 1882, min = 1, max = 2 }]

[[monster.corpse]]
chance = 0.38
items = [{ id = 908 }]             # Necklace of Wisdom
```

Validation at load:

- `0 < chance ≤ 100`.
- `min ≤ max`.
- Each `id` exists in the item catalogue.
- A monster has exactly one adena group.
- The server calculates the total expected adena for each kill, and writes it to the telemetry baseline table.

### 4.2 Recipe format (`packages/data/recipes/<grade>.toml`)

```toml
[[recipe]]
id            = 138
name          = "Pata"
recipe_item   = 5008          # the Recipe: Pata item that teaches it
book          = "dwarven"     # dwarven | common
craft_level   = 6             # Create Item skill level required
success_pct   = 60
mp_cost       = 171
product       = { id = 264, count = 1 }
ingredients   = [
  { id = 4103, count = 12 },  # Pata Blade (key material)
  { id = 1890, count = 70 },  # Mithril Alloy
  { id = 1888, count = 70 },  # Synthetic Cokes
  { id = 1885, count = 35 },  # High-Grade Suede
  { id = 4042, count = 35 },  # Enria
  { id = 1459, count = 310 }, # Crystal (C-Grade)
  { id = 2132, count = 55 },  # Gemstone B
]
```

### 4.3 Postgres additions

```sql
-- Recipe books
CREATE TABLE character_recipes (
  character_id UUID NOT NULL, recipe_id INTEGER NOT NULL, book SMALLINT NOT NULL,
  learned_at TIMESTAMPTZ NOT NULL DEFAULT now(), PRIMARY KEY (character_id, recipe_id));

-- Trade sessions survive only in memory; completed trades are ledger rows.
-- Private stores (persisted so a crash does not lose listings)
CREATE TABLE private_stores (
  character_id UUID PRIMARY KEY, kind SMALLINT NOT NULL,            -- 1 sell, 2 buy, 3 package, 4 manufacture
  title TEXT, opened_at TIMESTAMPTZ NOT NULL DEFAULT now());
CREATE TABLE private_store_entries (
  character_id UUID NOT NULL REFERENCES private_stores ON DELETE CASCADE,
  instance_id UUID,            -- sell: the escrowed item; NULL for buy/manufacture
  template_id INTEGER NOT NULL, count BIGINT NOT NULL, price BIGINT NOT NULL CHECK (price >= 0),
  recipe_id INTEGER);          -- manufacture entries

CREATE TABLE mail (
  mail_id UUID PRIMARY KEY, sender_id UUID, receiver_id UUID NOT NULL,
  subject TEXT, body TEXT, cod_adena BIGINT NOT NULL DEFAULT 0,
  sent_at TIMESTAMPTZ NOT NULL DEFAULT now(), expires_at TIMESTAMPTZ NOT NULL,
  attachments_taken BOOLEAN NOT NULL DEFAULT false, returned BOOLEAN NOT NULL DEFAULT false);
-- attachments are item_instances with location = 'mail' and a mail_id column added to item_instances.

CREATE TABLE economy_counters (            -- hourly rollups fed from item_ledger
  bucket TIMESTAMPTZ NOT NULL, template_id INTEGER NOT NULL, reason TEXT NOT NULL,
  created BIGINT NOT NULL, destroyed BIGINT NOT NULL, adena_in BIGINT NOT NULL, adena_out BIGINT NOT NULL,
  PRIMARY KEY (bucket, template_id, reason));
```

### 4.4 Proto sketch (`packages/proto/nightfall/v1/economy.proto`)

```proto
syntax = "proto3";
package nightfall.v1;
import "nightfall/v1/items.proto";

// Sent to the client when a corpse it may loot appears; also drives the on-screen drop list.
message DropList {
  string corpse_id = 1;
  repeated DroppedItem items = 2;
  int64 expires_at_ms = 3;
  string loot_owner_id = 4;               // who may pick up during the priority window ("" = anyone)
  bool spoiled = 5;                       // corpse has sweepable loot (shown only to the spoiler/party)
}
message DroppedItem { string ground_id = 1; uint32 template_id = 2; int64 count = 3; uint32 enchant = 4; }
message PickupRequest { string ground_id = 1; }
message SweepRequest  { string corpse_id = 1; }

enum PartyLootMode { PARTY_LOOT_MODE_UNSPECIFIED = 0; FINDERS_KEEPERS = 1; RANDOM = 2;
                     RANDOM_INCLUDING_SPOIL = 3; BY_TURN = 4; BY_TURN_INCLUDING_SPOIL = 5; }

// Direct trade
message TradeOffer {
  string trade_id = 1;
  string partner_id = 2;
  repeated Item my_items = 3;      // instance_id + count only are meaningful
  int64  my_adena = 4;
  repeated Item their_items = 5;
  int64  their_adena = 6;
  bool   i_confirmed = 7;
  bool   they_confirmed = 8;
  TradeState state = 9;
}
enum TradeState { TRADE_STATE_UNSPECIFIED = 0; REQUESTED = 1; OPEN = 2; CONFIRMED_ONE = 3;
                  LOCKED = 4; COMPLETED = 5; CANCELLED = 6; }
message TradeRequest   { string partner_id = 1; }
message TradeRespond   { string trade_id = 1; bool accept = 2; }
message TradeAddItem   { string trade_id = 1; string instance_id = 2; int64 count = 3; }
message TradeSetAdena  { string trade_id = 1; int64 adena = 2; }
message TradeConfirm   { string trade_id = 1; }
message TradeCancel    { string trade_id = 1; }

// Crafting
message CraftRequest   { uint32 recipe_id = 1; string manufacturer_id = 2; }   // manufacturer "" = self
message CraftResult    {
  enum Outcome { OUTCOME_UNSPECIFIED = 0; SUCCESS = 1; FAILED = 2; REJECTED = 3; }
  Outcome outcome = 1; uint32 product_template_id = 2; int64 count = 3; string reason = 4;
}
message LearnRecipeRequest { string recipe_item_instance_id = 1; }
message RecipeBook { repeated uint32 dwarven = 1; repeated uint32 common = 2; uint32 dwarven_limit = 3; uint32 common_limit = 4; }

// NPC shops
message BuyListRequest { string npc_id = 1; }
message BuyList { string npc_id = 1; repeated BuyEntry entries = 2; uint32 tax_percent = 3; }
message BuyEntry { uint32 template_id = 1; int64 price = 2; int64 stock = 3; }   // stock -1 = unlimited
message BuyRequest  { string npc_id = 1; repeated ItemQty items = 2; }
message SellRequest { string npc_id = 1; repeated Item items = 2; }
message ItemQty { uint32 template_id = 1; int64 count = 2; }

// Private stores
message OpenStoreRequest { PrivateStoreKind kind = 1; string title = 2; repeated StoreEntry entries = 3; }
enum PrivateStoreKind { PRIVATE_STORE_KIND_UNSPECIFIED = 0; SELL = 1; BUY = 2; PACKAGE = 3; MANUFACTURE = 4; }
message StoreEntry { string instance_id = 1; uint32 template_id = 2; int64 count = 3; int64 price = 4; uint32 recipe_id = 5; }
message StoreBuyRequest { string seller_id = 1; repeated StoreEntry entries = 2; }

// Mail
message SendMailRequest { string receiver_name = 1; string subject = 2; string body = 3;
                          repeated Item attachments = 4; int64 cod_adena = 5; }
```

## 5. Interfaces

**gRPC additions to `GameService`**: `Pickup`, `Sweep`, `SetPartyLootMode` (Phase 7 is responsible for the party, Phase 5 for the mode), `TradeRequest/Respond/AddItem/SetAdena/Confirm/Cancel`, `Craft`, `LearnRecipe`, `GetRecipeBook`, `GetBuyList`, `Buy`, `Sell`, `Refund`, `OpenStore`, `CloseStore`, `StoreBuy`, `SendMail`, `ListMail`, `TakeAttachments`. The server pushes trade and store state changes as `TradeOffer` snapshots on the inventory stream from Phase 4.

**Server events**: `MonsterKilled{kill_id, monster, contributors[], top_damage, last_hit}` (from Phase 3) → `drops::roll` → `ItemDropped{corpse, items, owner, priority_until}`; `CorpseSpoiled`; `TradeCompleted`; `CraftResolved{seed, outcome}`; `NpcSale`; `NpcPurchase`; `MailSent`. All these events go into the `item_ledger`.

**Loot-roll algorithm (Rust)**:

```rust
pub struct KillContext<'a> { pub kill_id: Uuid, pub monster_level: u16, pub killer_level: u16 /* highest contributor */,
                             pub is_raid: bool, pub is_champion: bool, pub rates: &'a DropRates }

pub fn roll_drops(table: &DropTable, scope: Scope, ctx: &KillContext, secret: &[u8; 32]) -> Vec<Drop> {
    let mut rng = ChaCha12Rng::from_seed(seed_for(secret, ctx.kill_id, scope));   // replayable
    let mut out = Vec::new();
    for group in table.groups(scope) {
        let has_adena = group.items.iter().any(|i| i.id == ADENA);
        let gap = level_gap_modifier(ctx, has_adena);                      // 0.0..=1.0
        let rate = ctx.rates.chance_multiplier(scope, ctx, has_adena);     // champion / raid / per-item overrides
        let chance = group.chance * gap * rate;                            // may exceed 100
        if rng.gen_range(0.0..100.0) >= chance.min(100.0) { continue; }
        let picked = pick_weighted(&group.items, &mut rng);                // weights pre-normalised at load
        let mut mult = 1;
        if chance > 100.0 {                                                // "precise" handling
            mult = (chance / 100.0).floor() as i64;
            if rng.gen_range(0.0..100.0) < chance % 100.0 { mult += 1; }
        }
        let count = rng.gen_range(picked.min..=picked.max) * mult * ctx.rates.amount_multiplier(scope, ctx, picked.id);
        out.push(Drop { template_id: picked.id, count });
    }
    out
}

fn level_gap_modifier(ctx: &KillContext, adena: bool) -> f64 {
    let d = ctx.killer_level as i32 - ctx.monster_level as i32;              // levels the killer is above
    if ctx.is_raid { return (1.0 - 0.15 * d as f64).clamp(0.0, 1.0); }
    let (min, max) = if adena { (8, 15) } else { (5, 10) };
    if d <= min { 1.0 } else if d >= max { 0.10 } else { 1.0 - 0.90 * (d - min) as f64 / (max - min) as f64 }
}
```

**Party distribution**: `drops::distribute(party, picker, drop)` follows these rules:

- Adena and the configured stackables are split `count / n` to each member in range. The remainder goes to the picker.
- Other items go to `picker`, `rng.choose(eligible)`, or `party.next_looter()`, by the mode. The last option is a rotating index that the party keeps.
- Spoil obeys the "including spoil" flags.
- The looter must pass `inventory.can_accept(template, count)` (slots and weight). If not, the server tries the next candidate.

**Trade state machine** (one `TradeSession` actor for each trade; the two players do not control it):

```
REQUESTED --accept--> OPEN --(add/remove item | set adena)--> OPEN   (clears both confirms)
OPEN --confirm(A)--> CONFIRMED_ONE --confirm(B)--> LOCKED --commit ok--> COMPLETED
any state --cancel | disconnect | distance > 150 | timeout 120s--> CANCELLED
LOCKED --validation/commit failure--> CANCELLED (both notified with the reason)
```

Commit (`trade::commit`) runs one Postgres transaction:

```sql
BEGIN;
SELECT ... FROM item_instances WHERE instance_id = ANY($ids) FOR UPDATE;     -- both sides, ordered by id
-- for each item: assert owner, location='inventory', count >= offered, version = expected, template tradeable
-- for each receiver: assert slots_used + incoming_nonstack <= slots_max and weight + incoming <= weight_max
UPDATE item_instances SET owner_id = $to, version = version + 1 WHERE instance_id = $id;         -- whole item
-- or split: UPDATE ... SET count = count - $n; INSERT new row for receiver (or merge into existing stack)
UPDATE characters SET adena = adena - $a WHERE id = $from AND adena >= $a;   -- affected rows must be 1
UPDATE characters SET adena = adena + $a WHERE id = $to;
INSERT INTO item_ledger (...) -- one row per moved stack per side, reason 'trade', counterparty set
COMMIT;
```

The session sends `InventoryUpdate` to the two character actors only after `COMMIT` returns. If an error occurs, the transaction does a rollback and the trade is `CANCELLED`. Private store purchases use the same commit path. When the store opens, the server moves the items of the seller into escrow (`location = trade_escrow`). Thus a seller cannot move an item that is on the list. At send time, mail attachments move to `location = mail`. On `TakeAttachments`, COD moves the adena in one transaction.

**Craft flow**:

1. Validate these conditions: the character knows the recipe, `craft_level ≤ CreateItem level`, MP ≥ cost, and the materials are present. For a manufacturer, the materials of the customer must be present, and the fee must be ≤ the adena of the customer.
2. `BEGIN`.
3. Lock the material rows.
4. Consume all the materials.
5. Roll `rng.gen_range(0..100) < success_pct`.
6. If the roll is a success, insert the product (merge the stacks).
7. Move the fee.
8. Write the ledger rows with the reason `craft_success`/`craft_fail`.
9. `COMMIT`.
10. Then subtract the MP in the actor.

The craft consumes the materials for all outcomes, exactly as in L2.

## 6. Rust implementation notes

Module layout in `apps/api/src/economy/`:

```
economy/
  mod.rs
  drops/
    table.rs        // DropTable, DropGroup, DropEntry; TOML loading + normalisation + validation
    roll.rs         // roll_drops, level_gap_modifier, DropRates (config-backed multipliers)
    distribute.rs   // party loot modes, adena split, looter rotation
    ground.rs       // ground items per zone: spawn, pickup priority window, decay (600 s), pickup
    spoil.rs        // corpse spoil state, sweep transfer
  craft/
    recipe.rs       // Recipe, RecipeBook limits; TOML loading
    craft.rs        // validate + transactional craft, manufacture fee
  shop/
    buylist.rs      // NPC buy lists, restock timers, tax hook
    sell.rs         // sell at price/2, refund buffer per session
    multisell.rs    // item-for-item exchange lists
  trade/
    session.rs      // TradeSession actor, state machine, timeouts
    commit.rs       // sqlx transaction for two-party transfer (shared by store, mail COD)
    store.rs        // private sell/buy/package/manufacture stores, escrow
  mail/
    mail.rs         // send, list, take, return-on-expiry sweep (tokio interval)
  telemetry.rs      // counters -> economy_counters rollup; Prometheus gauges
  config.rs         // EconomyConfig (all tunables, serde from TOML, hot-reload via ArcSwap)
```

Crates: `sqlx` (transactions with `FOR UPDATE`), `rand_chacha`, `uuid` v7, `arc-swap`, `metrics` + `metrics-exporter-prometheus` for counters, `tokio` intervals for ground decay, mail expiry, restock.

Concurrency: ground items are in the zone actor (Phase 6), thus each zone serialises its pickups. Transfers between characters never touch the in-memory state of two character actors directly. They write to the DB first, and then they send a notification. After commit, the `TradeSession` actor sends `InventoryUpdate` messages to the two character actors. The actors apply the delta idempotently (by `ledger_id`). To prevent deadlock, each transaction must lock item rows in ascending `instance_id` sequence, and character rows in ascending `character_id` sequence.

Determinism and audit: `seed_for(secret, kill_id, scope)` = `blake3(secret || kill_id || scope)`. The server keeps the seed on the ledger row of each drop. A GM tool can run `roll_drops` again with the stored seed and the table version. Each table has a content hash, and `economy_counters` records it.

Performance budget: a kill with 8 groups costs ~8 RNG draws, and no allocation other than the output `Vec`. Loot tables are `Arc`-shared and do not change. Target: 10,000 kills/s on one core, with no DB writes on the hot path. The ledger writes go in batches through a channel to a writer task. If `save_dropped_items` is off, the server does not keep ground items in the DB.

**Telemetry-driven parameters** (each parameter has a config key, a Prometheus series, and an hourly rollup in `economy_counters`):

| Parameter | Launch value | Watch metric | Tune when |
|---|---|---|---|
| `drops.chance_multiplier`, `drops.amount_multiplier` | 1.0 / 1.0 | items created for each player-hour, by template band | material prices < 2x NPC sell price for a band |
| `drops.adena_multiplier` | 1.0 | adena created and destroyed for each hour (faucet/sink ratio) | the ratio goes outside 0.9-1.1 for a week |
| `drops.level_gap` windows and floors | 5-10 / 8-15, 10% | share of kills by level gap bucket | > 20% of items come from gap ≥ 8 (farming by overlevelled chars/bots) |
| `craft.success_pct` per tier | 100 / 60 | crafts attempted and crafts that succeeded, materials used for each success | top-gear supply for each week, compared with the active players in the band |
| `shop.sell_ratio` | 0.5 | adena destroyed through NPC sell | mass-vendoring patterns (bot signature) |
| `teleport.free_below_level`, price table | 41, L2 prices | adena destroyed through teleport for each active player | sink share < 15% of total sinks |
| `mail.fee`, `mail.fee_per_slot`, `cod_expiry_h` | 100, 1000, 12 | mail volume, COD completion rate | COD used as RMT escrow (many small CODs from a small number of accounts) |
| `party.loot_range` | 1400 | adena split count for each kill | leech patterns |
| `champion.*` | disabled | — | content need |
| `tax.castle_percent` cap | 15 (Phase 7) | tax collected for each castle | Phase 7 |
| `store.max_sell_slots`, `max_buy_slots` | 4/5 Dwarf, 3/4 other | active stores for each town | market congestion |

Build in these fraud signals from the first day:

- Adena velocity for each account (creation compared with transfer out).
- Item transfers between accounts that have the same IP/HWID.
- Trade value asymmetry (one side gives > 10x value by reference price).
- A kill cadence that is the same many times (bot timing).

## 7. Client implications

- **Drop list UI**: show `DropList` for the targeted corpse. During the priority window, show items with `loot_owner_id` ≠ self as locked. Sweepable corpses glow for the spoiler.
- **Party loot mode**: a selector for the leader. Broadcast a message when the mode changes.
- **Trade window**: two panes bound to `TradeOffer`. Local edits are only intents. The pane renders again from the server snapshots. When one side changes an item, the two confirm buttons reset, and the players can see this.
- **Craft window**: recipe book tabs (Dwarven/Common with `n/50`), ingredient availability colours from the local inventory, success % and MP cost from the recipe catalogue, and a result toast from `CraftResult`.
- **Shops**: a buy list with a tax line, a sell tab with a half-price preview, and a refund tab. Multisell shows item-for-item rows.
- **Private stores**: a title bubble above the seller, a store window on click, and a quantity picker for stackables. A manufacture store shows the fee for each recipe.
- **Mail**: inbox/outbox with attachment slots (8), and a COD pay/decline flow.
- The client has generated JSON catalogues for recipes and buy lists (static), the same as for items in Phase 4.

## 8. Open questions

1. Launch rate multipliers: x1 L2 rates are correct for the population density and session length of L2. Nightfall can use x2-x3 drops with lower crystal counts in recipes. This decision needs a target number from design: "hours to craft a top B weapon".
2. Does crafting consume the materials after a failure (L2), or give back a fraction? The first option is harder for players, but it is the main material sink. Decide this together with the drop multiplier.
3. Highest-level contributor or top-damage dealer as the "killer level" for the gap rule.
4. Adena split to AFK/dead party members in range (L2 splits to all members in range).
5. Offline private stores: these are very convenient, but they also help bots very much. Proposal: not at launch. Examine this again with telemetry on store counts.
6. Auction house: not at launch. If we add it, the listing fee is ≥ 1% of price, and there are ≤ 10 listings for each account. The seller gets the sales through mail.
7. Seasonal currency (an Ancient Adena analogue) and Mammon-style exchange shops depend on Phase 7 castles. Find out if a seasonal event without castles can host it before Phase 7.
8. PK/PvP item drops as a sink (L2 `KarmaRateDrop 40%`): a Phase 3/9 decision with a large effect on the economy.
9. Herbs (pick-up-only drops that the character uses immediately): in H5 they are a large share of drop groups. Include them or remove them?
10. Crystal counts in recipes: L2 uses hundreds of crystals for each craft, and this needs a healthy crystallization market. Before you lock the recipe data, use a spreadsheet to compare the counts with the Phase 4 crystal refund formula.

## 9. Sources

- L2J Server (High Five) game source, Bitbucket `l2jserver/l2j-server-game` (checked out 2026-09-13): `model/drops/GroupedGeneralDropItem.java`, `model/drops/GeneralDropItem.java`, `model/drops/strategy/IGroupedItemDropCalculationStrategy.java`, `IDropCalculationStrategy.java`, `IChanceMultiplierStrategy.java`, `IKillerChanceModifierStrategy.java` (group roll, normalisation, precise calculation, level-gap and champion multipliers); `model/actor/L2Attackable.java` (death/corpse drop scopes, sweep state); `model/L2Party.java` and `enums/PartyDistributionType.java` (loot modes, adena split, even distribution); `RecipeController.java` (craft validation, success roll, manufacture fee, masterwork); `model/TradeList.java` (trade confirm/lock/validate/exchange, private store buy); `network/clientpackets/RequestSellItem.java` (`price / 2`), `RequestBuyItem.java` and `model/entity/Castle.java` (castle tax application and treasury); `network/clientpackets/RequestSendPost.java` and `model/entity/Message.java` (mail fees, COD, expiry); `SevenSigns.java` (period lengths, seal stone values); config files `npc.properties` (deep blue windows and floors), `rates.properties`, `customs.properties` (champion settings, offline trade), `character.properties` (party ranges, XP cutoff, recipe limits, store slots, raid loot rights), `general.properties` (precise drop options, item auction, mail toggles), `sevensigns.properties`. https://bitbucket.org/l2jserver/l2j-server-game
- L2J Datapack (High Five), Bitbucket `l2jserver/l2j-server-datapack`: `data/recipes.xml` (all recipes; we calculated the success-rate distribution from it), `data/stats/npcs/*.xml` (grouped drop lists), `data/stats/items/*.xml` (reference prices for scrolls, shots, life stones; material item names), `data/stats/skills/00200-00299.xml` (Spoil skill 254, Create Item 172), `handlers/effecthandlers/instant/Sweeper.java`, `ai/npc/Teleporter/Teleporter.java` (free teleport below level 41, half price for Seven Signs). https://bitbucket.org/l2jserver/l2j-server-datapack
- L2JBrasil Interlude server (GitHub `L2jBrasil/Server-Interlude`, 2018): `L2Attackable.java` (Interlude deep-blue rule: 9+ levels, `(highest - (mob + 8)) * 9`%, adena /3), `L2Party.java` (adena split by `ALT_PARTY_RANGE2`, Interlude loot mode ids), `sql/droplist.sql` (category model, 1,000,000 chance scale), `sql/teleport.sql` (gatekeeper prices), `Config.java` (champion rewards x8, offline shop toggles). https://github.com/L2jBrasil/Server-Interlude
- L2J Server issue tracker, "Major issue for rates above x1" (group chance > 100% handling) and pull request "L2JServer Most Precise Drop Strategy". https://bitbucket.org/l2jserver/l2j-server-game/issues/337/major-issue-for-rates-above-x1 , https://bitbucket.org/l2jserver/l2j-server-game/pull-requests/35
- StrategyWiki, "Lineage II/Crafting" (Dwarf-only crafting, recipe books). https://strategywiki.org/wiki/Lineage_II/Crafting
- ludo.guide, "Crafting professions" (Lineage II). https://www.ludo.guide/guide/lineage-ii/crafting-materials/crafting-professions
- L2DB.net, "How to Make Adena Fast in Lineage 2 Interlude (CT0)" (teleport free until 40, adena sources). https://www.l2db.net/guides/en/how-to-make-adena-fast-il
- Inven Global, "Under Siege by Bot Farms: Lineage Classic's Growing Crisis" (bot-driven inflation, RMT, price collapse to NPC floor). https://www.invenglobal.com/articles/20251/under-siege-by-bot-farms-lineage-classics-growing-crisis
- Inven Global, "Lineage Classic Imposes Character Creation Limits Across All Servers" (anti-bot measures). https://www.invenglobal.com/articles/21416/mul-ori-water-duck-go-away-lineage-classic-imposes-character-creation-limits-across-all-servers
- Lineage II forums, "Update on Adena and drop rates" (official drop-rate adjustments). https://forums.lineage2.com/topic/9170-update-on-adena-and-drop-rates/page/9/
- Engadget, "Lineage II update talks auction house, Goddess of Destruction zones" (auction house introduced with Goddess of Destruction). https://www.engadget.com/2011-09-22-lineage-ii-update-talks-auction-house-goddess-of-destruction-zo.html
- Massively OP, "Lineage II Classic increases drop rates across the board". https://massivelyop.com/?p=190135
