# Phase 5: Economy and Crafting

## 1. Purpose and scope

Phase 5 defines how items enter, move through, and leave the world:

- **Sources**: monster drops (death drops and spoil/sweep), adena drops, raid and champion multipliers, level-gap penalties, party loot distribution.
- **Transformation**: the Dwarven recipe system (recipe books, Create Item skill level, materials, success chance, MP cost), material chains from raw materials to key materials to finished goods, masterwork/rare results.
- **Currencies and sinks**: adena, Ancient Adena and the Seven Signs cycle as a seasonal-economy reference, teleport fees, enchant/augment costs, taxes, NPC shop spreads.
- **Exchange**: NPC vendors and multisell, castle tax, player-to-player trade (direct trade, private stores, package sale, offline stores), mail with attachments and cash-on-delivery, and the later-chronicle auction house.
- **Health**: inflation controls, bot and RMT threats, mudflation, and the telemetry needed to tune all of the above.

Excluded: the item data model itself (Phase 4), monster placement and AI that decide *which* monsters exist (Phase 6), clan treasuries and siege rewards (Phase 7), the GM tooling that acts on telemetry (Phase 9).

## 2. Reference: how Lineage 2 does it

### 2.1 Drop tables: the group model

Interlude-era L2J stored drops as rows `(mobId, itemId, min, max, category, chance)` with `chance` on a 1,000,000 scale. Category `-1` meant "sweep only"; category `0` was adena; categories `1..n` were independent groups. Example (Interlude `droplist.sql`, mob 20543):

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

High Five L2J replaced this with explicit groups in the NPC XML, which is the model to copy:

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

1. For each group independently: compute the **effective group chance** = `groupChance * killerLevelModifier * rateMultiplier(champion, raid, item class)`. L2J does this by "normalising" the group: it sums `item.chance * groupChance * modifiers / 100` over the items, uses that sum as the group roll, and rescales item chances so they still sum to 100.
2. Roll `u ∈ [0,100)`. If `u < groupChance`, roll a second `u2` and walk the items' cumulative chances to select exactly one item.
3. Quantity = `uniform_int(min, max)` scaled by the amount multiplier.
4. **Precise drop calculation** (chances > 100% after rate multipliers): `amount *= floor(chance/100)` plus one more with probability `chance mod 100`. An alternative config rolls the group `floor(chance/100)` extra times and optionally aggregates stacks.
5. A group with a single item collapses to a plain roll at `item.chance * groupChance / 100`.

Each group yields at most one item per kill, so designers control "how many things drop" by the number of groups, and "which thing" by the item split within the group. Adena is always its own group.

**Rates and multipliers** (`rates.properties`, `customs.properties`): `DeathDropChanceMultiplier` / `DeathDropAmountMultiplier` (default 1), `CorpseDrop*` for spoil, `RaidDrop*`, `HerbDrop*`, a per-item-id override list (`57,1` for adena), and champion mobs (`ChampionHp 8x`, `ChampionRewardsChance 8x`, `ChampionRewardsExpSp 8x`, `ChampionAdenasRewardsChance 1x`, frequency and level window 20-78, configurable). Raid bosses use `RaidDropChanceMultiplier` and bypass the normal level-gap rule.

### 2.2 Level-gap ("deep blue") penalty

Interlude L2J, following the Prima guide: a monster is "deep blue" when the highest-level attacker is 9+ levels above it; `levelModifier = (highestLevel - (mobLevel + 8)) * 9` percent is subtracted from every chance, adena additionally divided by 3 (and by the server drop rate). So 9 levels above: -9%, 10: -18%, …, ~19 levels: 0%.

High Five L2J (`npc.properties`, `IKillerChanceModifierStrategy`): linear interpolation from 100% down to a floor over a window of level difference `d = mobLevel - killerLevel`:

| Item class | Full chance until `d ≥ -min` | Floor reached at `d ≤ -max` | Floor |
|---|---|---|---|
| Items | 5 levels above | 10 levels above | 10% |
| Adena | 8 levels above | 15 levels above | 10% |
| Raid bosses (`UseDeepBlueDropRulesRaid`) | `clamp(1 + 0.15 * d, 0, 1)` — a raid 7+ levels below the killer drops nothing | | 0% |

The "killer level" is the party member who did the most damage or the last hitter; Interlude used the highest-level attacker in the aggro list to stop high-level characters from "feeding" kills.

### 2.3 Spoil and sweep

Dwarf Scavenger line: **Spoil** (skill 254, 11 levels, lvl 10-72, MP 12-67, reuse 3 s, cast 1.8 s, effect power -138 to -646, success rolled through the standard debuff land-rate formula against the monster's level and MEN) marks a monster as spoiled by the caster. On death, if spoiled, the server rolls the `<corpse>` drop list once and stores the result on the corpse (`_sweepItems`). **Sweeper** (skill 42) by the spoiler (or their party member under a "including spoil" distribution mode) transfers those items; the corpse must still exist (decay 7 s default, +10 s extension for spoiled corpses). Spoil lists are where most key materials and enchant scrolls live at mid levels, which is what makes Dwarves economically central.

### 2.4 Adena drops and party distribution

Adena is a stackable group with `chance` 70% and a level-scaled `min..max` (mob 20543 lvl ~20: 105-183; mob lvl ~40: 236-471; level 60 mobs drop ~1,000-2,000; raids tens of thousands). Server rate `RateDropAdena` multiplies amount.

**Party loot modes** (`PartyDistributionType`): `FINDERS_KEEPERS` (0), `RANDOM` (1), `RANDOM_INCLUDING_SPOIL` (2), `BY_TURN` (3), `BY_TURN_INCLUDING_SPOIL` (4). Rules:

- **Adena** (and optionally all stackables, `PartyEvenlyDistributeAllStackableItems`) is split evenly among members within `PartyRange2` (1,400 units) of the corpse; remainder goes to the picker.
- Other items go to `getActualLooter`: the picker (finders keepers), a random eligible member, or the next member in rotation (by turn). Eligibility = in range, alive, has inventory slot and weight. Spoil items only rotate under the "including spoil" variants; otherwise they go to the Dwarf.
- Party XP/SP cutoff (`PartyXpCutoffGaps 0-9: 100%, 10-14: 30%, 15+: 0%`) is the social complement that stops high-level carries.
- Raid loot rights: the command channel or party with the most damage gets a 5-minute pickup priority (`RaidLootRightsInterval 900000` ms, `RaidLootRightsCCSize 45`).

### 2.5 Crafting: the Dwarven recipe system

Recipes are items (`Recipe: X`) that, when used, are registered into the Dwarf's **recipe book** (Dwarven book, limit 50; Common book, limit 50; `DwarfRecipeLimit/CommonRecipeLimit`). The recipe item is consumed on registration. Each recipe (`recipes.xml`) has:

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
- **Success**: `Rnd.get(100) < successRate`. Distribution across the H5 recipe file: craft level 1-5 recipes are 100% (except one 25% special); from craft level 6 upward the top-tier weapon/armor of each grade is **60%** (38 of 147 level-6 recipes, 63 of 149 level-7, 35 of 85 level-8, 80 of 121 level-9, all 28 level-10), a few are 70%, and most materials/consumables stay 100%. Common recipes are 100% (a handful 95%, 70%, 10%). The "60% on the good stuff" rule is the single most important economic parameter: a failed craft consumes **all** materials.
- **MP cost**: `statUse MP` scales with recipe tier (30 MP for D-grade arrows/weapons, ~170 MP for C-grade top weapons, 300+ for A/S). Interlude also had `HP` stat use on a few recipes.
- **Masterwork** (Gracia+): a `rareItemId` with `rarity` 1/4/20 (%) replaces the normal product on success. Not in Interlude.
- **Crafting for others**: a Dwarf opens a **manufacture shop** (private store variant) listing recipes and a fee; the customer supplies materials, pays the fee in adena, and the roll happens on the Dwarf's skill. Failure message goes to both.

**Material chains**. Raw materials drop everywhere (Iron Ore 1869, Coal 1870, Animal Bone 1872, Stem 1864, Varnish, Charcoal, Suede, Silver Nugget 1873, Mithril Ore 1876, Oriharukon Ore 1874, Adamantite Nugget 1877, Thread, Leather 1882, Animal Skin). Tier-1 recipes (100%) refine them: `Steel 1880 = Iron Ore + Coal`, `Cokes 1879 = Coal + Charcoal`, `Mithril Alloy 1890 = Mithril Ore + Steel`, `Synthetic Cokes 1888 = Cokes + Varnish`, `Oriharukon 1893`, `Durable Metal Plate 5550`, `Metallic Thread 5549`, `Compound Braid 1889`, `Steel Mold 1883`, `Silver Mold 1886`, `Blacksmith's Frame 1892`, `Artisan's Frame 1891`, `Crafted Leather 1894`, `Mold Lubricant 4040`, `Mold Hardener 4041`, `Enria 4042`, `Asofe 4043`, `Thons 4044`. **Key materials** (e.g. `Sword of Damascus Blade 4114`, `Pata Blade 4103`, `Zubei's Breastplate Part 4056`) drop or spoil from a handful of mobs in the item's level band and are usually required 10-15 per craft (the 12 Pata Blades above). The finished recipe then needs key material + refined materials + **Crystals of the grade** (hundreds: 310 C-crystals for a C weapon, ~1,300 B-crystals for a B weapon) + **Gemstones** (D for C/B gear, B for A/S). Crystals come from crystallizing surplus gear (Phase 4), so every crafted top item consumes several lesser items — the "Crystal + Gemstone" pattern is the built-in sink.

### 2.6 Currency and sinks

- **Adena** (item 57) is the only hard currency; caps at 99,900,000,000 per container. NPCs buy at `price / 2` of the template reference price and sell at `price * (1 + castleTax)`. Adena is created only by drops and quests; it is destroyed by NPC purchases, fees, taxes, and some quest turn-ins.
- **Ancient Adena** (5575) is earned only from the Seven Signs: seal stones (Blue 3, Green 5, Red 10 Ancient Adena each) are collected in the Catacombs/Necropolis and traded to a Priest of Dawn/Dusk during the Seal Validation period; "contribution points" at the same 3/5/10 values decide the winning cabal. Ancient Adena buys from the **Merchant/Blacksmith of Mammon**: SA infusion, enchant scroll exchange, weapon exchange between same-grade types, grade-up exchanges. Mammon NPCs are reachable only by the winning cabal when `StrictSevenSigns` is on.
- **Seven Signs cycle** (`SevenSigns.java`): one-week period starting Monday 18:00; **Competition** (quest/recruiting) for ~1 week minus a 15-minute results window, then **Seal Validation** for ~1 week minus 15 minutes. Dawn requires castle ownership or a participation fee; Dusk is open to non-owners. Seals: **Avarice** (Mammon access, Ancient Adena exchange), **Gnosis** (Dawn/Dusk priests buff/teleport perks), **Strife** (castle gate/wall multipliers 1.1 for Dawn, 0.8 for Dusk; castle tax ceiling: 15% when Dawn holds Strife, 5% for Dusk in the Interlude rule set). This is the reference for a *seasonal* economy: a two-week cycle that moves a whole class of goods (SA, enchant exchange) in and out of reach.
- **Teleport fees**: gatekeeper prices scale with distance and destination level: Dion→Giran 6,800; Giran→Oren 9,400; Giran→Hunters Village 4,100; Aden→Ivory Tower 12,000; town↔hunting-ground hops 1,000-4,000 (Interlude `teleport.sql`). Gracia+ made all gatekeeper teleports **free below level 41** (`Teleporter.java: if (player.getLevel() < 41) ammount = 0`); Seven Signs priest teleports are half price; `FreeTeleporting` is a server toggle. Teleport fees are the largest steady adena sink for active players.
- **Enchant scrolls** (NPC reference prices): Weapon D 50,000 / C 110,000 / B 500,000 / A 1,800,000 / S 5,000,000; Armor D 6,000 / C 15,000 / B 80,000 / A 240,000 / S 500,000; Blessed Weapon A 15,000,000. Scrolls are sold by NPCs only through Mammon/luxury shops in some chronicles; otherwise they are drop-only, which is what makes them a tradeable second currency.
- **Augmentation**: life stones 5,000 / 20,000 / 200,000 / 1,000,000 reference price by grade; removal fee in adena by item grade.
- **Shots**: 7-100 adena each by grade (Phase 4 table); a bow user burns ~50 soulshots a minute. This is the biggest per-capita sink and scales with activity, which is why it works.
- **Repair**: there is no durability or repair in L2. Equipment leaves the world only through enchant failure, crystallization, and (rarely) PvP/PK drops.
- **Castle tax**: the castle-owning clan sets `taxPercent` (0-15, ceiling set by Seal of Strife); every NPC merchant in the castle's territory applies `price * (1 + taxRate)` and the tax goes to the castle treasury; Aden and Rune skim a share of lower castles' treasuries. The **Manor** system lets castle owners sell seeds and buy crops at prices they set, creating a regional commodity loop.
- **Mail**: 100 adena per message + 1,000 per attached item slot; messages expire in 15 days, COD mails in 12 hours.
- **Freight**: 1,000 adena per item deposited.

### 2.7 NPC shops and vendors

- **Buy lists** (`buylists/*.xml`) per merchant: template ids with optional restock count/time. Price = reference `price` × (1 + castle tax). Weight and slot are validated server-side (`RequestBuyItem`).
- **Sell**: any `is_sellable` item at `price / 2`, no tax; recent sells are kept in a **refund** tab (`AllowRefund`) for buy-back at the same price until logout.
- **Multisell** (`multisell/*.xml`): item-for-item exchanges (e.g. Mammon exchanges, grade-up shops) with optional `maintainEnchantment` and tax application; this is how L2 implements crafting-adjacent vendors without adena.
- **Luxury shop** (Giran): sells B/A gear at reference price; a major adena sink and the ceiling for player prices.

### 2.8 Player trading

**Direct trade** (`TradeList`): states per side — `items[]`, `confirmed`, `locked`. Flow: A requests, B accepts (window opens for both, neither can move), each adds items/adena (any change calls `invalidateConfirmation()` on *both* lists), each presses Confirm (`confirm()`); when both are confirmed, both lists `lock()`, each side `validate()`s (owner still online, every item still owned and count still valid, items tradeable and not equipped), then `doExchange()` checks weight and slot capacity of both receivers and transfers atomically; any failure cancels the trade with a message. L2J orders the two locks by object id to avoid deadlock.

**Private store sell / buy** (Interlude: 4 sell / 5 buy slots for Dwarves, 3 / 4 others in H5): the seller lists items with prices and sits; buyers open the store and buy; the server re-validates each item (`privateStoreBuy`: list not locked, seller and buyer online, seller still owns items, buyer has adena, weight, slots) and performs the transfer. **Package sale** sells the whole list for the sum only. **Manufacture store** lists recipes with fees. Stores are visible as a title above the character; offline stores (private-server `OfflineTradeEnable`, later retail "offline shops") keep the character in-world for up to N days.

**Mail with attachments / COD** (Gracia+): up to 8 attachments, fee above; COD attaches a required adena amount that the receiver must pay to take the items, otherwise returns to sender on expiry (12 h).

**Auction house** (Goddess of Destruction+, "Auctioneer"/"Item Broker" NPC): list up to N items for 3/7 days with a listing fee proportional to price; sold items pay adena via mail. Later chronicles cap listings per account and add a commission. Earlier eras had only the clan-hall auction (bidding adena for clan halls, a sink) and the Interlude **Item Auction** (`ItemAuctionEnabled`, a timed NPC auction with 5/3-minute extensions).

### 2.9 Economy health

Observed failure modes in L2 and how NCsoft addressed them:

- **Bot farms and RMT** ("rice farmers"): bots run 24/7 in low-contest zones, flooding adena and raw materials. Prices of materials collapse to NPC sell price (`price / 2`), while fixed-price sinks (enchant scrolls, shots, teleport) stay constant, so the real cost of progression *falls* for RMT buyers and *rises* for everyone else. NCsoft responded with mass bans, character-creation limits per server (Lineage Classic 2024), and drop-rate reductions on open-world mobs.
- **Mudflation**: as the population levels, low-grade materials lose all value; Interlude's answer was crystallization (every item is worth its crystals, which are needed for crafting) and the grade ladder (new bands need new materials). Gracia/H5 added item attributes and S80/S84, moving the sink upward, and instanced-dungeon drops with weekly limits.
- **Adena faucets without sinks**: raids and quests with large adena rewards had no corresponding sink; later chronicles added adena-only vendors (scroll exchange, buffs, teleport pricing scaled with level).
- **Price discovery**: Interlude had no auction house; private stores in Giran acted as the market, with offline shops and later the auction house improving liquidity but also enabling bot-to-player sales.

## 3. Design decisions for Nightfall

1. **Drop model = H5 grouped drops**: a monster has `death` and `corpse` (spoil) lists; each list is an ordered set of groups; each group rolls at most one item. Adena is always group 0. Item chances within a group are normalised at load time so designers can write unnormalised weights.
2. **Level-gap penalty = H5 linear window** (items 5→10 levels above, floor 10%; adena 8→15, floor 10%), using the **highest-level contributor** in the kill (Interlude anti-feeding rule). Raids: `clamp(1 + 0.15 * (raidLevel - killerLevel), 0, 1)`.
3. **Party loot**: all five L2 modes, adena always split evenly among in-range members (1,400 units), with configurable "split all stackables". Spoil goes to the spoiler unless an "including spoil" mode is active.
4. **Champion mobs**: supported as a multiplier set on the spawn (HP x8, drop chance x8, adena x1) but off by default; they are a tuning lever, not launch content.
5. **Crafting = Dwarven recipe model** with recipe books (limit 50 each), Create Item skill levels gating `craft_level`, full material consumption on failure, MP cost per recipe, 100% for materials and 60% for top gear per grade as the starting rates. Common items are not planned (Phase 4 open question); masterwork is deferred.
6. **Material chains**: three tiers (raw → refined → key material + crystals + gemstones → item), with crystals from crystallization as the deliberate item sink. Chains are data, defined once per grade band in `recipes_*.toml`.
7. **Currency**: adena only at launch. A seasonal secondary currency modelled on Ancient Adena/Seven Signs is designed in but gated behind Phase 7 (needs castles/cabals). The data model reserves `currency_kind`.
8. **Sinks at launch**: NPC spreads (`sell = price / 2`), teleport fees (free below 41, then distance-priced), shots, enchant failure, augmentation fees, mail fees, warehouse/freight fees, castle tax hook (0 until Phase 7). No durability.
9. **NPC shops**: buy lists with optional restock, sell at half price, refund tab for the session, multisell for item-for-item exchanges, tax hook.
10. **Trade**: direct trade with the two-phase confirm state machine; private sell/buy/package stores; manufacture stores; mail with attachments and COD (12 h). Offline stores and an auction house are **not** in Phase 5 (open questions 8.5, 8.6) — both have outsized bot/RMT implications and need telemetry first.
11. **All transfers are Postgres transactions** with row locks and version checks, mirrored into the in-memory inventories only after commit. Every transfer writes `item_ledger` rows for both parties.
12. **RNG**: all loot, craft, and store rolls use a seeded `ChaCha12Rng` derived from `(server_secret, kill_id | craft_request_id)`; seed is logged so any drop can be replayed by GMs.
13. **Telemetry-first**: every economic parameter in this document is a config value with a telemetry counter next to it (section 6). Launch numbers are L2's; tuning is Phase 9's job.

Alternatives considered: per-item independent rolls (simpler, but makes "one of these" design impossible and inflates drop counts at high rates); bind-on-pickup to fight RMT (defers the problem, kills the player market; rejected for launch); durability as a sink (adds friction without the player agency of enchanting; rejected).

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

Validation at load: `0 < chance ≤ 100`, `min ≤ max`, every `id` exists in the item catalogue, a monster has exactly one adena group, total expected adena per kill is computed and written to the telemetry baseline table.

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

**gRPC additions to `GameService`**: `Pickup`, `Sweep`, `SetPartyLootMode` (Phase 7 owns the party, Phase 5 the mode), `TradeRequest/Respond/AddItem/SetAdena/Confirm/Cancel`, `Craft`, `LearnRecipe`, `GetRecipeBook`, `GetBuyList`, `Buy`, `Sell`, `Refund`, `OpenStore`, `CloseStore`, `StoreBuy`, `SendMail`, `ListMail`, `TakeAttachments`. Trade and store state changes are pushed over the inventory stream from Phase 4 as `TradeOffer` snapshots.

**Server events**: `MonsterKilled{kill_id, monster, contributors[], top_damage, last_hit}` (from Phase 3) → `drops::roll` → `ItemDropped{corpse, items, owner, priority_until}`; `CorpseSpoiled`; `TradeCompleted`; `CraftResolved{seed, outcome}`; `NpcSale`; `NpcPurchase`; `MailSent`. All feed the `item_ledger`.

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

**Party distribution**: `drops::distribute(party, picker, drop)` — adena and configured stackables are split `count / n` to each in-range member with the remainder to the picker; other items go to `picker`, `rng.choose(eligible)`, or `party.next_looter()` (a rotating index persisted on the party) according to the mode; spoil obeys the "including spoil" flags. The looter must pass `inventory.can_accept(template, count)` (slots and weight) or the next candidate is tried.

**Trade state machine** (one `TradeSession` actor per trade, owned by neither player):

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

Only after `COMMIT` returns does the session send `InventoryUpdate` to both character actors; on any error the transaction rolls back and the trade is `CANCELLED`. Private store purchases use the same commit path with the seller's items pre-escrowed (`location = trade_escrow`) when the store opens, so a seller cannot move a listed item. Mail attachments move to `location = mail` at send time; COD moves adena on `TakeAttachments` in one transaction.

**Craft flow**: validate recipe known, `craft_level ≤ CreateItem level`, MP ≥ cost, materials present (manufacturer: customer's materials, fee ≤ customer adena); `BEGIN`; lock material rows; consume all; roll `rng.gen_range(0..100) < success_pct`; on success insert product (merging stacks); transfer fee; ledger rows with reason `craft_success`/`craft_fail`; `COMMIT`; then deduct MP in the actor. Materials are consumed regardless of outcome, exactly as in L2.

## 6. Rust implementation notes

Module layout under `apps/api/src/economy/`:

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

Concurrency: ground items live in the zone actor (Phase 6) so pickup is serialised per zone; cross-character transfers never touch two character actors' in-memory state directly — they go DB-first, then notify. The `TradeSession` actor sends `InventoryUpdate` messages to both character actors after commit; actors apply the delta idempotently (by `ledger_id`). Deadlock avoidance: lock item rows in ascending `instance_id` order and character rows in ascending `character_id` order in every transaction.

Determinism and audit: `seed_for(secret, kill_id, scope)` = `blake3(secret || kill_id || scope)`; stored on the ledger row of every drop. A GM tool can re-run `roll_drops` with the stored seed and the table version (tables carry a content hash recorded in `economy_counters`).

Performance budget: a kill with 8 groups costs ~8 RNG draws and no allocation beyond the output `Vec`; loot tables are `Arc`-shared and immutable. Target: 10,000 kills/s on one core with no DB writes on the hot path (ledger writes are batched through a channel to a writer task; ground items are not persisted unless `save_dropped_items` is on).

**Telemetry-driven parameters** (each has a config key, a Prometheus series, and an hourly rollup in `economy_counters`):

| Parameter | Launch value | Watch metric | Tune when |
|---|---|---|---|
| `drops.chance_multiplier`, `drops.amount_multiplier` | 1.0 / 1.0 | items created per player-hour by template band | material prices < 2x NPC sell price for a band |
| `drops.adena_multiplier` | 1.0 | adena created vs destroyed per hour (faucet/sink ratio) | ratio drifts outside 0.9-1.1 for a week |
| `drops.level_gap` windows and floors | 5-10 / 8-15, 10% | share of kills by level gap bucket | > 20% of items come from gap ≥ 8 (farming by overlevelled chars/bots) |
| `craft.success_pct` per tier | 100 / 60 | crafts attempted vs succeeded, materials burned per success | top-gear supply per week vs. active players in band |
| `shop.sell_ratio` | 0.5 | adena destroyed via NPC sell | mass-vendoring patterns (bot signature) |
| `teleport.free_below_level`, price table | 41, L2 prices | adena destroyed via teleport per active player | sink share < 15% of total sinks |
| `mail.fee`, `mail.fee_per_slot`, `cod_expiry_h` | 100, 1000, 12 | mail volume, COD completion rate | COD used as RMT escrow (many small CODs from few accounts) |
| `party.loot_range` | 1400 | adena split count per kill | leech patterns |
| `champion.*` | disabled | — | content need |
| `tax.castle_percent` cap | 15 (Phase 7) | tax collected per castle | Phase 7 |
| `store.max_sell_slots`, `max_buy_slots` | 4/5 Dwarf, 3/4 other | active stores per town | market congestion |

Fraud signals to build in from day one: per-account adena velocity (creation vs transfer out), item transfers between accounts sharing an IP/HWID, trade value asymmetry (one side gives > 10x value by reference price), repeated identical kill cadence (bot timing).

## 7. Client implications

- **Drop list UI**: show `DropList` for the targeted corpse; items with `loot_owner_id` ≠ self rendered locked during the priority window; sweepable corpses glow for the spoiler.
- **Party loot mode** selector for the leader; broadcast message on change.
- **Trade window**: two panes bound to `TradeOffer`; local edits are intents only — the pane re-renders from server snapshots; both confirm buttons reset visibly when either side changes anything.
- **Craft window**: recipe book tabs (Dwarven/Common with `n/50`), ingredient availability colouring from the local inventory, success % and MP cost displayed from the recipe catalogue, result toast from `CraftResult`.
- **Shops**: buy list with tax line, sell tab with half-price preview, refund tab; multisell shows item-for-item rows.
- **Private stores**: title bubble over the seller, store window on click, quantity picker for stackables; manufacture store shows fee per recipe.
- **Mail**: inbox/outbox with attachment slots (8), COD pay/decline flow.
- The client ships generated JSON catalogues for recipes and buy lists (static), the same way as items in Phase 4.

## 8. Open questions

1. Launch rate multipliers: x1 L2 rates assume L2's population density and session length; Nightfall may want x2-x3 drops with lower crystal counts in recipes instead. Needs a target "hours to craft a top B weapon" number from design.
2. Whether crafting consumes materials on failure (L2) or refunds a fraction — the former is harsher but is the main material sink; decide with the drop multiplier.
3. Highest-level contributor vs. top-damage dealer as the "killer level" for the gap rule.
4. Adena split to AFK/dead party members in range (L2 splits to anyone in range).
5. Offline private stores: strong convenience, strong bot amplifier. Proposal: not at launch; revisit with telemetry on store counts.
6. Auction house: not at launch; if added, listing fee ≥ 1% of price and ≤ 10 listings per account, with sales paid by mail.
7. Seasonal currency (Ancient Adena analogue) and Mammon-style exchange shops depend on Phase 7 castles; confirm whether a castle-less seasonal event can host it earlier.
8. PK/PvP item drops as a sink (L2 `KarmaRateDrop 40%`) — a Phase 3/9 decision with major economy impact.
9. Herbs (pick-up-only, instant-use drops) — in H5 they are a large share of drop groups; include or drop them?
10. Crystal counts in recipes: L2's hundreds of crystals per craft require a healthy crystallization market; verify against the Phase 4 crystal refund formula with a spreadsheet before locking recipe data.

## 9. Sources

- L2J Server (High Five) game source, Bitbucket `l2jserver/l2j-server-game` (checked out 2026-09-13): `model/drops/GroupedGeneralDropItem.java`, `model/drops/GeneralDropItem.java`, `model/drops/strategy/IGroupedItemDropCalculationStrategy.java`, `IDropCalculationStrategy.java`, `IChanceMultiplierStrategy.java`, `IKillerChanceModifierStrategy.java` (group roll, normalisation, precise calculation, level-gap and champion multipliers); `model/actor/L2Attackable.java` (death/corpse drop scopes, sweep state); `model/L2Party.java` and `enums/PartyDistributionType.java` (loot modes, adena split, even distribution); `RecipeController.java` (craft validation, success roll, manufacture fee, masterwork); `model/TradeList.java` (trade confirm/lock/validate/exchange, private store buy); `network/clientpackets/RequestSellItem.java` (`price / 2`), `RequestBuyItem.java` and `model/entity/Castle.java` (castle tax application and treasury); `network/clientpackets/RequestSendPost.java` and `model/entity/Message.java` (mail fees, COD, expiry); `SevenSigns.java` (period lengths, seal stone values); config files `npc.properties` (deep blue windows and floors), `rates.properties`, `customs.properties` (champion settings, offline trade), `character.properties` (party ranges, XP cutoff, recipe limits, store slots, raid loot rights), `general.properties` (precise drop options, item auction, mail toggles), `sevensigns.properties`. https://bitbucket.org/l2jserver/l2j-server-game
- L2J Datapack (High Five), Bitbucket `l2jserver/l2j-server-datapack`: `data/recipes.xml` (all recipes; success-rate distribution computed from it), `data/stats/npcs/*.xml` (grouped drop lists), `data/stats/items/*.xml` (reference prices for scrolls, shots, life stones; material item names), `data/stats/skills/00200-00299.xml` (Spoil skill 254, Create Item 172), `handlers/effecthandlers/instant/Sweeper.java`, `ai/npc/Teleporter/Teleporter.java` (free teleport below level 41, half price for Seven Signs). https://bitbucket.org/l2jserver/l2j-server-datapack
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
