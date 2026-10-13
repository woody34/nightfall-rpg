> **BINDING RECORD — Phase 5 design, incorporating architect A1–A20, owner decisions P1–P4, and verified datapack/game sources; independent review required.** This is the implementation contract for new Snapshot 11 / Record 9 / Binary V7 epochs. It does not claim that the implementation or acceptance gates already pass.
>
> **ARCHITECT ERRATA:** P1–P4 and A1–A20 in the Phase 5 decision record supersede Phase 4 where expressly amended. Verified facts from `reference-brief-verification.md` win over research documents `05-economy-and-crafting.md` and `reference-brief-v2.md`. Existing Phase 3 (C1–C18) and Phase 4 (D1–D75) binding records remain binding; this file only adds.

# Phase 5 zone engine: economy, drops, trade, and crafting

## Source authority and reconciliation

Citations use:
- **Z/** = `apps/api/src/domain/zone/`
- **R/** = `apps/api/src/application/replay_log/`
- **A/** = `apps/api/src/application/`
- **W** = `packages/proto/nightfall/v1/world.proto`
- **P** = `packages/proto/nightfall/v1/game.proto`
- **E** = `packages/proto/nightfall/v1/economy.proto`
- **I** = `packages/proto/nightfall/v1/items.proto`
- **GJ/** = `/home/matt-woodruff/.cache/nightfall/l2j/l2j-game/src/main/java/com/l2jserver/gameserver/`, revision `abfde0490ac52a2106e884b7242c0276c44cdf76`
- **GC/** = `/home/matt-woodruff/.cache/nightfall/l2j/l2j-game/src/main/resources/config/`
- **DP/** = `/home/matt-woodruff/.cache/nightfall/l2j/l2j-dp/src/main/resources/data/`, revision `3ca488dd2bd0bfaca43e378886a3c2e37968153a`
- **DPJ/** = `/home/matt-woodruff/.cache/nightfall/l2j/l2j-dp/src/main/java/com/l2jserver/datapack/`
- **AD5** = `/home/matt-woodruff/.config/Claude/side-session-notes/phase5-agy-2026-10-13/architecture-decisions.md`
- **OD5** = `/home/matt-woodruff/.config/Claude/side-session-notes/phase5-agy-2026-10-13/owner-decisions.md`
- **VR5** = `/home/matt-woodruff/.config/Claude/side-session-notes/phase5-agy-2026-10-13/reference-brief-verification.md`
- **P4Z** = `docs/plans/phase-4-zone-design.md`
- **SN4** = `/home/matt-woodruff/.config/Claude/side-session-notes/phase4-codex-2026-10-12/architecture-decisions.md` (D1–D75)

All executable formulas here use integers, $Q=1{,}000{,}000$ fixed-point arithmetic (`i64` with checked `i128`), or exact rational ratios (`ExactRatio`). Floating-point arithmetic, runtime wall clocks, `HashMap`, and un-ordered iteration are strictly forbidden (A1; worker-common §4).

| Conflict / old assumption | Verified binding Phase 5 treatment and evidence |
|---|---|
| Adena faucets & sinks: "mob drops and quest rewards only; vendor spread is a sink" | Selling items to an NPC **mints** adena at `item.getReferencePrice() / 2` via `addAdena("Sell", totalPrice, ...)` (`GJ/network/clientpackets/RequestSellItem.java:142,155`). Only NPC buy is a sink. Faucets: mob drops, quest rewards, NPC sell-back. Sinks: NPC buy (with castle tax), teleport, mail, freight, warehouse deposit (30 adena), enchant/augment costs, craft failure, consumables. (VR5 A2). |
| Castle tax ceiling: "0–15% default, capped at 15%" | Castle tax is set by the castle lord within Seal of Strife ceilings: **no-owner 15%** (default until castles exist in Phase 6), **Dusk 5%**, **Dawn 25%** (`GJ/instancemanager/CastleManager.java:165-176`). Buy price is `trunc(price × (1 + baseTax + castleTax))` with `baseTax = 0` (`GJ/data/xml/impl/BuyListData.java:119; GJ/network/clientpackets/RequestBuyItem.java:182`). Castle treasury receives `trunc(subTotal × castleTaxRate)` (`:234`). (VR5 A7, A17, C18). |
| Drop lists and corpse lists XML structure: "124 files with `<death><group>` and `<corpse><group>`" | 124 NPC XML files exist; only **59** contain `<dropLists>/<death>` and **52** contain `<corpse>` (`DP/stats/npcs/*.xml`). `<death>` contains 2,938 `<group chance>` blocks and 307 ungrouped `<item>` rows. `<corpse>` contains **only ungrouped** `<item>` rows (0 groups in any corpse list), each rolled independently (`GJ/model/drops/DropListScope.java:31-32; GJ/model/actor/L2Attackable.java:838-840`). (VR5 A22, C20, C21). |
| Killer identity for drops: "last hitter or highest level contributor" | Retail H5 resolves drop credit against **`maxDealer`**: the online player who dealt the most damage within `PartyRange 1600` (`GJ/model/actor/L2Attackable.java:341-379`). Fallback to `lastAttacker` occurs only if no dealer exists. Deep blue level gap uses `maxDealer.level`. (VR5 A31). |
| Adena level-gap window: "8/15 window applied to adena" | `DropAdenaMin/MaxLevelDifference = 8/15` exists in config (`GC/npc.properties:197-205`), but `DEFAULT_STRATEGY` for grouped drops always uses the **item window (5/10/10%)** (`GJ/model/drops/strategy/IKillerChanceModifierStrategy.java:33-53; GJ/model/drops/GroupedGeneralDropItem.java:50-51`). All 2,309 adena entries in pinned NPC XML are grouped; thus pinned H5 adena uses 5/10/10%. (VR5 A29). |
| PK chaotic drops: "unequips before ground drop only if dropped" | For a karma drop, **every eligible equipped item is unequipped BEFORE its roll** (`GJ/model/actor/instance/L2PcInstance.java:4531-4541`). A PK death strips eligible gear even if zero items drop. Ground drops have 15 s protection owned by the dying player and party (`GJ/datatables/ItemTable.java:214; GJ/model/actor/instance/L2PcInstance.java:3198-3201`). Player drops never auto-destroy by default (`GC/general.properties:306,312`). (VR5 A39). |
| Spoil mechanics: "power -138 to -646, MEN-resisted debuff" | Spoil 254 `effectPoint` is hate -138..-646, not power (`DP/stats/skills/00200-00299.xml:1345`). Success is level-based `Formulas.calcMagicSuccess` (`DPJ/handlers/effecthandlers/instant/Spoil.java:40-42`). Fails with `ALREADY_SPOILED` if monster is already spoiled (`:57-60`). (VR5 A42, A43). |
| Sweeper skill: "instant cast" | Sweeper 42 has `hitTime = 500 ms`, `reuse = 500 ms`, `mpConsume = 3`, `castRange = 20`, target `NPC_BODY` (`DP/stats/skills/00000-00099.xml:1465-1480`). Only the effect application is instant. Spoiler OR any member of spoiler's party may sweep (`GJ/model/actor/L2Attackable.java:1027-1035`). (VR5 A45, A46). |
| Herb flag and slot validation: "immediate_effect" and "validateCapacityByItemId" | Herbs are flagged **`ex_immediate_effect="true"`** (68 EtcItems, `GJ/model/items/L2Item.java:188,743-745`). Exemption from inventory slots is in `PcInventory.validateCapacity(L2ItemInstance)` (`:746-752`). `validateCapacityByItemId` always returns true (`:759-765`), so party RANDOM/BY_TURN is in practice a range check only. (VR5 A48, A51, A61). |
| Raid loot ownership: "15 min to top damage CC/party" | Raid loot protection applies **only if a command channel with at least `RaidLootRightsCCSize = 45` members attacked first** (`GC/character.properties:693; GJ/model/actor/L2Attackable.java:234-238; GJ/model/L2CommandChannel.java:165`). Protected for 900 s to CC leader and CC party (`GJ/datatables/ItemTable.java:205-210`). Otherwise, raid loot has **no ownership protection at all**. (VR5 A56, C14). |
| Buylist refund tab: "unbounded refund" | Refund buffer is **capped at 12 items**; 13th item pushes out and destroys the oldest item permanently (`GC/general.properties:458; GJ/model/itemcontainer/PcRefund.java:60-64`). Buffer is cleared on logout (`GJ/model/actor/instance/L2PcInstance.java:2539-2543,8678`). (VR5 A69). |
| Multisell maintainEnchantment: "copies enchant only" | `maintainEnchantment` copies enchant level, augmentation (`stat34<<16\|stat12`), AND elemental attributes (`GJ/model/multisell/PreparedEntry.java:81-83; GJ/network/clientpackets/MultiSellChoose.java:355-362`). (VR5 A72). |
| Crafting failure and masterwork | Materials consumed before success roll; failure loses 100% of materials with 0 refund (`GJ/RecipeController.java:363-387`). Pinned default has `CraftMasterwork = True` (`GC/character.properties:484; 583 recipes carry productionRare`). Recipe item consumed on registration; recipe books capped at 50 Dwarven / 50 Common (`:488-489`). (VR5 A82, §6.4). |
| Storage fees & limits | Warehouse: 100 slots (120 Dwarf), deposit fee **30 adena per item entry** (`GJ/network/serverpackets/SendWareHouseDepositList.java:117-118`). Freight: 200 slots, fee **1,000 adena per item entry** (`GC/character.properties:335; GJ/network/clientpackets/RequestPackageSend.java:109`). (VR5 A86, A88, §5.A2). |
| Authority model: "DB-first transfers with row locks" | REJECTED by A1, A2, B5. The zone actor is the sole writer and single state authority for all in-world economy mutations (ground items, pickup, vendors, crafting, multisell, trade, private stores). Warehouse/freight/mail storage containers are DB-only, but every transfer into/out of live inventory executes through the zone critical mutation pipeline. (AD5 A2; SN4 D7, D30, D41). |

---

## 1. Identity, canonical state and hydration

### 1.1 Ownership and entity identity

Phase 5 introduces `GroundItem` as a first-class zone entity, extends `NpcState` with corpse/sweep tracking, and manages vendor sessions, trade sessions, private stores, and recipe books directly in zone-canonical state.

Add `ground_item.rs`, `corpse.rs`, `vendor.rs`, `multisell.rs`, `craft.rs`, `trade.rs`, `private_store.rs`, `phase5.rs`, and `phase5_checkpoint.rs` to `apps/api/src/domain/zone/` and register them in the deterministic `SOURCES` scan list (`Z/mod.rs`).

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroundItemId(pub u64);

#[derive(Clone, Debug)]
pub struct GroundItem {
    pub id: GroundItemId,
    pub instance_id: ItemInstanceId,
    pub template_id: ItemTemplateId,
    pub count: u64,
    pub enchant: u32,
    pub augment_id: Option<u32>,
    pub position: Position,
    pub dropped_by: Option<EntityId>,
    pub protection_owner: Option<EntityId>,
    pub protection_party: Option<PartyId>,
    pub protection_until_tick: Tick,
    pub decay_tick: Option<Tick>, // None for player drops (SaveDroppedItem=False)
    pub is_herb: bool,
    pub created_tick: Tick,
}
```

1. **GroundItem Identity Rule G1.** `GroundItemId(u64)` is allocated from the same monotone per-zone allocator as `ItemInstanceId` (Phase 4 I1; D8/D19), or a dedicated disjoint subrange. Ground items map to `EntityKind::GroundItem` on the wire (wire enum `ENTITY_KIND_GROUND_ITEM = 3`).
2. **Corpse State Rule G2.** An NPC entering death retains its physical corpse presence in the zone for a bounded duration. The corpse holds the spoiled flag, spoiler attribution, sweep item inventory (rolled once at death from `<corpse>`), and decay deadline.
3. **Decay Lifecycles.**
   - NPC dropped items decay after `AutoDestroyDroppedItemAfter = 600` s (6,000 ticks) (`GC/general.properties:258; GJ/ItemsAutoDestroy.java:76-85`).
   - Herbs decay after `AutoDestroyHerbTime = 60` s (600 ticks) (`GC/general.properties:262; GJ/ItemsAutoDestroy.java:67`).
   - Player dropped items (including PK death drops) **never auto-destroy** (`DestroyPlayerDroppedItem = False`, `DestroyEquipableItem = False`; `GC/general.properties:306,312; SaveDroppedItem = False :316`).
   - Normal corpses decay after `DefaultCorpseTime = 7` s (70 ticks) (`GC/npc.properties:86`), overridden by template `corpseTime` (`GJ/model/actor/templates/L2NpcTemplate.java:139`).
   - Spoiled corpses receive an extension of `SpoiledCorpseExtendTime = 10` s (100 ticks) (`GC/npc.properties:90; GJ/taskmanager/DecayTaskManager.java:61-63`), totaling 17 s (170 ticks) default.

### 1.2 Phase5Rules

`Phase5Rules::try_new` validates all referenced templates, recipes, buylists, and tax rates before computing a canonical SHA-256 digest with domain separator `nightfall.phase5.rules.1\0`. Phase5Rules is content-addressed and hash-referenced from snapshots and replay admissions, never embedded whole into snapshot records (A4, A16; P4Z §1.2; SN4 D27).

Canonical hash inputs, length-delimited and strictly typed:
1. **Drop tables:** 59 death XML files. Ordered list of NPC templates with their `<group chance>` entries (normalized chances, min/max, precise calculation flags) and ungrouped item entries (`DP/stats/npcs/*.xml`).
2. **Corpse / Spoil tables:** 52 corpse XML files. Ordered ungrouped item entries with independent chances and min/max quantities (`DP/stats/npcs/*.xml`).
3. **Herb catalogue:** 68 EtcItem templates with `ex_immediate_effect = true` and their bound skill effects (`GJ/model/items/L2Item.java:188,743-745`).
4. **Buylists:** 195 buylist XML files with product template IDs, base prices, restock counts, and restock intervals (`DP/buylists/*.xml`).
5. **Multisell lists:** 196 multisell XML files with entry IDs, input ingredients, product outputs, `maintainEnchantment` flags, and `applyTaxes` flags (`DP/multisell/*.xml`).
6. **Recipes:** 1,000 recipes (830 Dwarven, 170 Common) with recipe IDs, level, recipe item template ID, craft skill rank, product template ID, product count, MP cost, success rate, `productionRare` template ID, and ingredient lists (`DP/recipes.xml`).
7. **Rate multipliers & policies:** Exact rational global rate multiplier `(num, den)`, default 1/1 (P2; A5; OD5 P2); champion multipliers (HP ×8, rewards ×8, adena ×1; `GC/customs.properties:10,28,34,37,43`); deep blue level gap configuration (item 5/10/10%, raid clamp 7; `GC/npc.properties:187-219`); territory castle tax baseline (no-owner 15%, Dawn 25%, Dusk 5%; `GJ/instancemanager/CastleManager.java:165-176`); storage limits and fees (WH 100/120 @ 30 adena, freight 200 @ 1,000 adena; `GC/character.properties:325-335`); mail fees (100 base + 1,000/slot, max 8 attachments, flood interval 100 ticks; `GJ/network/clientpackets/RequestSendPost.java:63-68,255; GC/floodprotector.properties:106`).

### 1.3 Phase5Snapshot, Phase5Player and invariants

```rust
pub struct Phase5Snapshot {
    pub rules: Arc<Phase5Rules>,
    pub rules_hash: [u8; 32],
    pub next_ground_item_id: u64,
    pub ground_items: BTreeMap<GroundItemId, GroundItem>,
    pub corpses: BTreeMap<EntityId, CorpseState>,
    pub vendor_restocks: BTreeMap<(u32, ItemTemplateId), VendorRestockState>,
    pub trade_sessions: BTreeMap<u64, TradeSession>,
    pub private_stores: BTreeMap<EntityId, PrivateStoreState>,
    pub players: BTreeMap<EntityId, Phase5Player>,
}

pub struct CorpseState {
    pub npc_id: EntityId,
    pub template_id: NpcTemplateId,
    pub position: Position,
    pub death_tick: Tick,
    pub decay_tick: Tick,
    pub is_spoiled: bool,
    pub spoiler_id: Option<EntityId>,
    pub spoiler_party: Option<PartyId>,
    pub sweep_items: Vec<SweepItem>,
    pub is_swept: bool,
}

pub struct SweepItem {
    pub template_id: ItemTemplateId,
    pub count: u64,
}

pub struct VendorRestockState {
    pub buylist_id: u32,
    pub template_id: ItemTemplateId,
    pub current_count: u32,
    pub max_count: u32,
    pub restock_delay_ticks: u32,
    pub last_restock_tick: Tick,
}

pub struct Phase5Player {
    pub dwarven_recipes: BTreeSet<u32>,
    pub common_recipes: BTreeSet<u32>,
    pub refund_tab: Vec<RefundEntry>, // FIFO, max 12
    pub active_trade_session: Option<u64>,
    pub active_private_store: Option<PrivateStoreState>,
}

pub struct RefundEntry {
    pub instance_id: ItemInstanceId,
    pub template_id: ItemTemplateId,
    pub count: u64,
    pub enchant: u32,
    pub augment_id: Option<u32>,
    pub price: u64, // purchase price = sold price
}

pub struct TradeSession {
    pub id: u64,
    pub actor_a: EntityId,
    pub actor_b: EntityId,
    pub items_a: BTreeMap<ItemInstanceId, u64>,
    pub items_b: BTreeMap<ItemInstanceId, u64>,
    pub locked_a: bool,
    pub locked_b: bool,
    pub confirmed_a: bool,
    pub confirmed_b: bool,
    pub start_tick: Tick,
    pub last_action_tick: Tick,
}

pub struct PrivateStoreState {
    pub store_type: StoreType, // Sell, Buy, PackageSell, Manufacture
    pub title: String,
    pub items: Vec<TradeItem>,
}

pub struct TradeItem {
    pub instance_id: Option<ItemInstanceId>,
    pub template_id: ItemTemplateId,
    pub count: u64,
    pub price: u64,
    pub enchant: u32,
}
```

Invariants:
1. Every `GroundItem` references a valid item template. If stackable, count is positive; if non-stackable, count is exactly 1.
2. Protection owner and party must be valid entities if set. Protection until tick must be monotonically checked against current tick.
3. Corpse state exists only for dead NPCs with `decay_tick > current_tick`. Sweeping an already-swept corpse is rejected.
4. Player refund tab has length $\le 12$. On the 13th addition, entry 0 is evicted and permanently destroyed (`PcRefund.java:60-64`). Refund tab clears on player logout (`L2PcInstance.java:2539-2543`).
5. Trade sessions link exactly two distinct in-world player entities within 150 units. Moving beyond 150 units or disconnecting cancels the trade session.
6. Recipe book capacities are strictly bounded: Dwarven $\le 50$, Common $\le 50$ (`GC/character.properties:488-489`).

### 1.4 Snapshot budget against C10

Under C10, the maximum encoded NATS snapshot payload limit is 1,047,552 bytes; the decoded limit is 33,554,432 bytes (`R/record.rs:221-223`).

Phase 4 proved that 8 fully equipped players occupy at most 210,048 bytes of mutable state (P4Z §1.4). Phase 5 introduces zone-level economy state:

| Component | Bound derived from scalar encoding |
|---|---:|
| Ground items: bounded at 1,000 per zone (decay limits steady-state) | $1{,}000 \times 64\text{ bytes} = 64{,}000\text{ bytes}$ |
| Corpses: bounded at 200 active corpses (7–17 s decay) | $200 \times 48\text{ bytes} = 9{,}600\text{ bytes}$ |
| Vendor restock tracking: bounded at 200 limited items | $200 \times 24\text{ bytes} = 4{,}800\text{ bytes}$ |
| Active trade sessions: at most 4 among 8 players | $4 \times 256\text{ bytes} = 1{,}024\text{ bytes}$ |
| Active private stores: at most 8 among 8 players | $8 \times 512\text{ bytes} = 4{,}096\text{ bytes}$ |
| Player recipe books: 8 players $\times$ 100 recipes $\times$ 4 bytes | $3{,}200\text{ bytes}$ |
| Player refund tabs: 8 players $\times$ 12 items $\times$ 32 bytes | $3{,}072\text{ bytes}$ |
| **Total Phase 5 state addition** | **$89{,}792\text{ bytes}$** |
| Combined Phase 4 ($210{,}048$) + Phase 5 ($89{,}792$) | **$299{,}840\text{ bytes}$** |
| **Remaining headroom before compression** | **$747{,}712\text{ bytes}$** |

The worst-case mutable state comfortably fits within the 1 MiB NATS envelope. An implementation gate will assert snapshot encoding size on soak benchmarks.

### 1.5 Hydration, AOI visibility and presentation boundaries

1. **Ground Items:** Broadcast to all players in the 3x3 AOI grid upon drop via `WorldEvent::GroundItemSpawn` (tag 34). Carries `ground_item_id`, `template_id`, `count`, `enchant`, `position`, and `is_protected` (boolean). Exact owner ID and protection timer are private to prevent target griefing; the client displays a protected icon tint if the local player is not the owner. Despawn is broadcast via `WorldEvent::GroundItemDespawn` (tag 35).
2. **Private Stores:** Spawns/updates emit `WorldEvent::PrivateStoreChanged` (tag 37) to AOI observers. Observers see player entity ID, store type, title, and public sell/buy prices.
3. **Trade Sessions:** Mutual updates between the two trading participants only via `WorldEvent::TradeSessionChanged` (tag 36). Observers outside the trade receive zero trade events.
4. **Recipe Book & Crafting:** Owner-only. Hydrated via `WorldEvent::RecipeBookSync` (tag 38) after `SkillStateSync` and `InventorySync`. Craft results arrive via `WorldEvent::CraftResult` (tag 41).
5. **Vendor Buylists & Multisell:** Opened on-demand via proximity intent, sent as owner-only `WorldEvent::BuylistWindow` (tag 39) or `WorldEvent::MultisellWindow` (tag 40).

---

## 2. V7 in-tick ordering and RNG ledger

### 2.1 Schedule amendment

Extend the tick schedule driver (`Z/state_runtime.rs:545-645`) with a V7 execution branch. V5 and V6 legacy replay paths remain byte-exact.

| Phase | V7 amendment to P4Z §2.1 |
|---|---|
| **0 — Receive-order commands** | Authorize/fingerprint/deduplicate. <br>1. Execute Phase 4 item intents (Equip, Unequip, Use, Destroy, Enchant, AutoShot, Refine, CancelRefine, ExchangeSA, Crystallize). <br>2. Execute **PickUpItem** intents: validate range (40 units), protection timer, and inventory capacity `can_accept`. If valid, remove GroundItem and grant item/adena via party distribution or direct looter. <br>3. Execute **VendorBuy** / **VendorSell** / **VendorRefund** intents: check NPC proximity (radius 3), price/tax calculation, adena minting/spending, refund FIFO update. <br>4. Execute **Multisell** intents: check NPC proximity, consume ingredients, grant product with copied enchant/augment/attributes. <br>5. Execute **CraftItem** / **RegisterRecipe** intents: check recipes, Create Item ranks, deduct MP, consume materials, draw success and masterwork rolls, grant product or record failure. <br>6. Execute **Trade** intents (Request, Answer, AddItem, RemoveItem, Lock, Confirm, Cancel): when both confirm, execute atomic two-phase swap in a single mutation. <br>7. Execute **PrivateStore** intents (Open, Close, Buy, Manufacture). <br>8. Execute **Storage** intents (Deposit, Withdraw) for Warehouse and Freight. <br>9. Recompute effective stats, weight, and penalties immediately for affected actors. |
| **1 — Spawn** | Hydrate newly spawned players/NPCs, new GroundItems, and active corpses into AOI grids. |
| **2 — Expiry** | Expire timed effects, item shared-reuse deadlines, and **economy timers**: <br>1. Check GroundItem decay: despawn items reaching `decay_tick`. <br>2. Check GroundItem protection: clear `protection_until_tick` when expired. <br>3. Check Corpse decay: remove NPC corpses reaching `decay_tick`. <br>4. Check Trade session idle timeouts (120 s). |
| **3 — Periodic + regen** | Preserve V6 regen and periodic tick processing. |
| **4 — Chase / movement** | Update positions and facing vectors. |
| **5 — Cast launches / impacts** | Resolve spell impacts: <br>1. **Spoil (254)**: on impact, evaluate `calcMagicSuccess`. On success, set corpse `is_spoiled = true` and record spoiler attribution. <br>2. **Sweeper (42)**: on impact, validate spoiler ownership and capacity; transfer `sweep_items` to sweeper/party; mark corpse `is_swept = true`. |
| **6 — Auto impacts** | Resolve melee and projectile hits, landed damage, shot discharge. |
| **7 — Hit bookkeeping, deaths & drop resolution** | Unified landed-damage facts. When an entity dies: <br>1. Award kill credit and evaluate `maxDealer` (top-damage online player within `PartyRange 1600`). <br>2. If NPC: generate Corpse state. If spoiled, roll `<corpse>` items into `_sweepItems`. Set corpse decay deadline (7 s default / 17 s spoiled). <br>3. Resolve NPC drops: calculate deep blue level gap, roll grouped and ungrouped death drop lists using per-zone RNG, create GroundItems with protection (15 s normal / 900 s raid CC). <br>4. If player with karma > 0 and pkKills ≥ 6: resolve chaotic PK drops (unequip eligible items before roll, check KarmaRateDrop 40%, roll slots, spawn GroundItems with 15 s protection). |
| **8 — New starts** | Auto-attacks and skill casts. Preflight costs, auto-shot charges. |
| **9 — AI** | Monster hate and AI behavior. |
| **10 — Durable flush / observe** | Batch ledger rows (Phase 4 D66/D69 order), persist checkpoint + outbox, emit WorldEvents (GroundItemSpawn, TradeSessionChanged, CraftResult, etc.) only after log acknowledgement and fenced DB commit. |

### 2.2 Exact random caller order and skip conditions

Every RNG draw in V7 consumes the per-zone `ChaCha12` stream in deterministic in-tick order:

```rust
let residue = rng.next_u32() % bound;
```

Replay records audit facts for every draw: `(tick, phase, resolution_id, event_kind, caller_site, bound, raw_u32, residue)`.

| Event / Caller | Draw order and conditional skip logic |
|---|---|
| **Spoil skill 254 impact** | 1. Check target alive, attackable, not already spoiled. If already spoiled, fail with `ALREADY_SPOILED` (no draw). <br>2. Level-based magic success: `calcMagicSuccess` draw bound 100. Residue $< \text{magicSuccessRate}$ succeeds (`DPJ/handlers/effecthandlers/instant/Spoil.java:40-42`). |
| **Sweeper skill 42 impact** | **Zero draws.** Sweep items were already rolled deterministically at NPC death into `_sweepItems` (`GJ/model/actor/L2Attackable.java:839`). |
| **NPC death drops: Grouped list** | Process `<group chance>` blocks in deterministic XML file and group order: <br>1. Compute effective group chance $C_{\text{eff}} = \text{base\_chance} \times \text{rate\_mult} \times \text{level\_gap} \times \text{champ\_mult}$. <br>2. If $C_{\text{eff}} \ge 100$: guaranteed amount multiplier $M_{\text{base}} = \lfloor C_{\text{eff}} / 100 \rfloor$. If $(C_{\text{eff}} \bmod 100) > 0$, draw `next_u32() % 100_000` for extra +1 multiplier (`IGroupedItemDropCalculationStrategy.java:73-78`). <br>3. If $C_{\text{eff}} < 100$: draw `next_u32() % 100_000 < C_{\text{eff}} \times 1000`. If fail, skip group. <br>4. If group succeeds: <br>   a. Multi-item group: draw `next_u32() % sumchance` to select exactly one item via cumulative normalized walk (`GroupedGeneralDropItem.java:197-211`). <br>   b. Quantity roll: draw `next_u32() % (max - min + 1)` to pick base quantity, multiply by amount multiplier. |
| **NPC death drops: Ungrouped list** | Process ungrouped `<item>` rows in XML order: <br>1. Effective chance $C_{\text{eff}} = \text{item.chance} \times \text{rate\_mult} \times \text{level\_gap} \times \text{champ\_mult}$. <br>2. Precise amount roll if $C_{\text{eff}} \ge 100$. <br>3. Chance roll if $C_{\text{eff}} < 100$: `next_u32() % 100_000 < C_{\text{eff}} \times 1000`. <br>4. Quantity roll: `next_u32() % (max - min + 1)`. |
| **NPC corpse drops (Spoil items)** | At NPC death, if `is_spoiled = true`, process `<corpse>` ungrouped `<item>` rows in XML order: <br>1. Effective chance $C_{\text{eff}} = \text{item.chance} \times \text{rate\_mult} \times \text{level\_gap}$. <br>2. Amount roll if $C_{\text{eff}} \ge 100$. <br>3. Chance roll if $C_{\text{eff}} < 100$: `next_u32() % 100_000 < C_{\text{eff}} \times 1000`. <br>4. Quantity roll: `next_u32() % (max - min + 1)`. Store rolled items into `CorpseState.sweep_items`. |
| **Chaotic PK death drops** | If killer is in non-PvP zone, victim has `karma > 0` and `pkKills >= 6`: <br>1. Base drop roll: `next_u32() % 100 < 40` (`KarmaRateDrop = 40%`). If fail, zero items drop. <br>2. For each eligible equipped item (unequipped before roll): <br>   a. If weapon: draw `next_u32() % 100 < 10`. <br>   b. If other equipment: draw `next_u32() % 100 < 40`. <br>   Stop if 10 items dropped. <br>3. If dropped count < 10, iterate eligible inventory items: draw `next_u32() % 100 < 50` until 10 items reached. |
| **Crafting roll** | 1. Success roll: `next_u32() % 100 < success_rate`. If fail, all materials lost (zero further draws). <br>2. If success AND recipe has `productionRare`: masterwork draw `next_u32() % 1000 < 30` (30 per-mille = 3%). If success, grant rare product; else standard product. |
| **Party adena split remainder** | When splitting adena among $K$ in-range members with remainder $R = \text{total} \bmod K$: <br>For each 1 adena of remainder ($R$ times): draw `next_u32() % K` to award that 1 adena to `members[residue]`. |
| **Party random item distribution** | When an item is awarded under `RANDOM` or `RANDOM_INCLUDING_SPOIL` among $K$ eligible members: draw `next_u32() % K` to select recipient. |

---

## 3. Drops and loot generation

### 3.1 Precise drop calculation algorithm

In High Five, drop rates are configured with `PreciseDropCalculation = True`, `PreciseDropMultipleGroupRolls = False`, and `PreciseDropMultipleRollsAggregateDrops = False` (`GC/general.properties:341-350; GJ/model/drops/strategy/IGroupedItemDropCalculationStrategy.java:73-78`).

Let base chance be $c_{\text{base}}$ (expressed in percentage points, where 100.0% is 100), global rate multiplier $M = \frac{M_{\text{num}}}{M_{\text{den}}}$, level gap modifier $L = \frac{L_{\text{num}}}{L_{\text{den}}}$, and champion multiplier $C \in \{1, 8\}$.

The effective percentage chance is:
$$C_{\text{eff}} = c_{\text{base}} \times \frac{M_{\text{num}}}{M_{\text{den}}} \times \frac{L_{\text{num}}}{L_{\text{den}}} \times C$$

To evaluate this without floating-point arithmetic, express the chance scaled to $100{,}000$ units (where 100% = $100{,}000$, and 0.001% = 1):
$$\text{chance\_scaled} = \left\lfloor \frac{c_{\text{base}} \times 1{,}000 \times M_{\text{num}} \times L_{\text{num}} \times C}{M_{\text{den}} \times L_{\text{den}}} \right\rfloor$$

1. **If $\text{chance\_scaled} \ge 100{,}000$ (Effective Chance $\ge 100\%$):**
   - Base multiplier: $M_{\text{base}} = \lfloor \text{chance\_scaled} / 100{,}000 \rfloor$.
   - Remainder: $R_{\text{chance}} = \text{chance\_scaled} \bmod 100{,}000$.
   - If $R_{\text{chance}} > 0$: draw $r = \text{next\_u32}() \bmod 100{,}000$. If $r < R_{\text{chance}}$, amount multiplier $M_{\text{total}} = M_{\text{base}} + 1$; else $M_{\text{total}} = M_{\text{base}}$.
   - If $R_{\text{chance}} == 0$: $M_{\text{total}} = M_{\text{base}}$.
   - Quantity dropped: $Q = (\text{min} + (\text{next\_u32}() \bmod (\text{max} - \text{min} + 1))) \times M_{\text{total}}$.
2. **If $\text{chance\_scaled} < 100{,}000$ (Effective Chance $< 100\%$):**
   - Draw $r = \text{next\_u32}() \bmod 100{,}000$.
   - If $r < \text{chance\_scaled}$: item/group drops with amount multiplier 1. Quantity $Q = \text{min} + (\text{next\_u32}() \bmod (\text{max} - \text{min} + 1))$.
   - If $r \ge \text{chance\_scaled}$: item/group does NOT drop.

### 3.2 Grouped vs ungrouped drop lists

1. **Grouped Drop Lists (`<group chance="...">`):**
   - Single-item group: collapses to effective chance $C_{\text{eff}} = \frac{\text{item.chance} \times \text{group.chance}}{100} \times M \times L \times C$ (`IGroupedItemDropCalculationStrategy.java:51-61`).
   - Multi-item group:
     - Group roll succeeds if effective group chance passes the precise draw in §3.1.
     - On group success, exactly ONE item from the group drops.
     - Selection uses normalized chance: $\text{sumchance} = \sum_{i} \text{item}_i\text{.chance}$. Draw $u = \text{next\_u32}() \bmod \text{sumchance}$. Walk items cumulatively until $\sum \text{item}_i\text{.chance} > u$ (`GroupedGeneralDropItem.java:197-211`).
     - Quantity of the selected item is determined by that item's min/max and the group's amount multiplier.
2. **Ungrouped Drop Lists (`<item id="..." chance="...">`):**
   - Each item is rolled completely independently using the precise algorithm in §3.1. Multiple ungrouped items may drop on a single kill.

### 3.3 Level-gap modifier windows (Deep Blue)

Let killer level be $K = \text{maxDealer.level}$, mob level be $M_{\text{lvl}}$, and level difference be $d = M_{\text{lvl}} - K$ (`GJ/model/actor/L2Attackable.java:341-379; GC/npc.properties:187-219`).

1. **Normal Items and Adena (Item Window 5/10/10%):**
   - Pinned parameters: `DropItemMinLevelDifference = 5`, `DropItemMaxLevelDifference = 10`, `DropItemMinLevelGapChance = 10` (10% floor).
   - If $d \ge -5$: no penalty, $L = 1$ (100%).
   - If $d \le -10$: floor applied, $L = \frac{10}{100}$ (10%).
   - If $-10 < d < -5$: linear interpolation using exact integer arithmetic:
     $$L_{\text{pct}} = 10 + (d + 10) \times 18$$
     This yields exact integer percentages:
     - $d = -5 \implies 100\%$ ($L = 1$)
     - $d = -6 \implies 82\%$ ($L = 82/100$)
     - $d = -7 \implies 64\%$ ($L = 64/100$)
     - $d = -8 \implies 46\%$ ($L = 46/100$)
     - $d = -9 \implies 28\%$ ($L = 28/100$)
     - $d = -10 \implies 10\%$ ($L = 10/100$)
   - In pinned H5, **all 2,309 adena entries are grouped**, so adena drops use this exact 5/10/10% item window (VR5 A29; `GJ/model/drops/strategy/IKillerChanceModifierStrategy.java:33-53`).
2. **Raid Bosses:**
   - Formula: $\text{clamp}\left(\frac{d \times 15}{100} + 1, 0, 1\right)$ (`IKillerChanceModifierStrategy.java:37`).
   - If $d \le -7$ (killer 7 or more levels above raid): drop chance modifier is 0. Raid drops nothing.

### 3.4 Champion monsters and global rate multiplier

1. **Champion Monsters:**
   - When spawned as champion (`customs.properties:10,28,34,37,43`): HP multiplier $\times 8$, reward drop chance multiplier $\times 8$, XP/SP multiplier $\times 8$, adena drop chance multiplier $\times 1$ (adena quantity scales via amount multiplier if chance $> 100\%$).
2. **Global Rate Multiplier:**
   - Tunable in TOML and hashed into `Phase5Rules` (P2; OD5 P2).
   - Applied as an exact rational $M_{\text{num}} / M_{\text{den}}$ prior to the integer modulo draw.

### 3.5 Killer determination and contributor tracking

1. **`maxDealer` Algorithm:**
   - Track all damage dealt to the NPC during its current life incarnation.
   - At death, filter damage contributors to online players currently within `PartyRange = 1600` units (`GJ/model/actor/L2Attackable.java:341-379`).
   - Find the player with the highest total damage. This player is the `maxDealer`.
   - Fallback: if no valid player in range, fall back to the `lastAttacker`.
2. **Loot Rights:**
   - If `maxDealer` is in a party, loot rights belong to that party.
   - If `maxDealer` is solo, loot rights belong solely to that player.

### 3.6 Chaotic PK drops and unequip-before-roll rule

Chaotic drop trigger (`GJ/model/actor/instance/L2PcInstance.java:4485-4550; GC/rates.properties:116-128; GC/pvp.properties:27`):
- Victim has `karma > 0` and `pkKills >= 6` and dies outside a PvP zone.
- Clan war death exemption: players dying in clan war without karma do not drop items (`L2PcInstance.java:4479-4483`).

Execution:
1. Base check: `next_u32() % 100 < 40` (`KarmaRateDrop = 40%`). If false, abort (0 drops).
2. **Unequip-Before-Roll Rule (VR5 A39; `L2PcInstance.java:4531-4541`):**
   - For every equipped item eligible for drop (excluding non-droppable, quest, shadow, time-limited, and pet items):
   - **The item is immediately unequipped into inventory before any drop roll occurs!**
   - Even if the drop roll subsequently fails, the gear remains unequipped.
3. Drop rolls:
   - For each formerly equipped weapon: `next_u32() % 100 < 10` (`KarmaRateDropEquipWeapon = 10%`).
   - For each other formerly equipped item: `next_u32() % 100 < 40` (`KarmaRateDropEquip = 40%`).
   - Maximum 10 items dropped total (`KarmaDropLimit = 10`).
   - If fewer than 10 dropped, iterate eligible inventory items: `next_u32() % 100 < 50` (`KarmaRateDropItem = 50%`).
4. Dropped items spawn as GroundItems with **15 s drop protection** assigned to the dead player and their party (`L2PcInstance.java:3198-3201; DropProtection.java`).

### 3.7 Ownership protection timers and decay lifecycle

1. **Normal Monster Loot:**
   - Protected for 15,000 ms (150 ticks) to `maxDealer` and party (`GJ/datatables/ItemTable.java:214`).
   - Non-owners receive `REJECT_REASON_GROUND_ITEM_PROTECTED` on pickup attempts (`L2PcInstance.java:3824`).
2. **Raid Boss Loot:**
   - Requires that the first attacker belonged to a command channel with $\ge 45$ members (`character.properties:693; L2Attackable.java:234-238`).
   - Protected for 900 s (9,000 ticks) to that CC leader and CC party (`character.properties:689; ItemTable.java:205-210`).
   - If no such CC attacked first, raid loot drops with **zero protection** (VR5 A56).

---

## 4. Pickup and party loot distribution

### 4.1 Pickup intent validation and capacity checks

When a player sends `PickUpItemRequest`:
1. **Range check:** Player must be within 40 units (1.25 tiles) of the GroundItem (`L2PcInstance.java:3780-3800`).
2. **Ownership check:** If `tick < protection_until_tick`, player must be the protection owner or a member of the protection party (`:3824`).
3. **Capacity check:** If picking up for self, validate `can_accept(template, count)` (`PcInventory.java:772-785`).
4. **Action:** Despawn GroundItem, emit `WorldEvent::GroundItemDespawn`, record ledger op `pickup`.

### 4.2 Herbs and immediate-effect items

Herbs are the 68 EtcItems with `ex_immediate_effect="true"` (`L2Item.java:188,743-745`):
1. **Exempt from slot validation:** Checked via `PcInventory.validateCapacity(L2ItemInstance)` (`:746-752`).
2. **Instant consumption:** On pickup, handler applies the skill effect to the picking player and destroys the item immediately without allocating an inventory slot or `ItemInstanceId` (`L2PcInstance.java:3860-3868`).
3. **Decay:** Herbs decay after 60 s (600 ticks) (`GC/general.properties:262`).

### 4.3 Party loot modes (0 through 4)

Loot distribution modes (`PartyDistributionType.java:25-29; L2Party.java:197-220`):

| ID | Mode | Non-Adena Item Behavior | Spoil / Sweep Behavior |
|---|---|---|---|
| 0 | `FINDERS_KEEPERS` | Item goes directly to picker. | Sweep goes to sweeper. |
| 1 | `RANDOM` | Random eligible party member within `PartyRange2 = 1400`. | Sweep goes to sweeper. |
| 2 | `RANDOM_INCLUDING_SPOIL` | Random eligible party member within `PartyRange2 = 1400`. | Sweep items distributed randomly. |
| 3 | `BY_TURN` | Round-robin turn order among in-range members. | Sweep goes to sweeper. |
| 4 | `BY_TURN_INCLUDING_SPOIL` | Round-robin turn order among in-range members. | Sweep items distributed by turn. |

Party member eligibility check:
- Distance from picker $\le 1400$ units (`L2Party.java:162`).
- Note: `validateCapacityByItemId` always returns true in L2J (`PcInventory.java:759-765; VR5 A61`), so the check is strictly distance. If no member is eligible, item falls back to picker.

### 4.4 Adena party split and remainder distribution

Adena is never distributed by item mode; it is always split evenly (`L2Party.java:591-628`):
1. Collect all party members within `PartyRange2 = 1400` units of picker (or NPC if sweep/autoloot).
2. Count in-range members $K$. If $K = 1$, all adena goes to that player.
3. If $K > 1$:
   - Base share: $\text{share} = \lfloor \text{total} / K \rfloor$.
   - Remainder: $R = \text{total} \bmod K$.
   - Each in-range member receives $\text{share}$ adena.
   - For each unit of remainder $1..R$: select random member via `next_u32() % K` and grant +1 adena (`L2Party.java:613`).
4. Cap check: each player capped at `MaxAdena = 99,900,000,000`.

---

## 5. Spoil and sweep

### 5.1 Spoil skill 254 resolution and effect

1. **Skill Properties:** 11 levels (character level 10–72), MP cost 12–67, reuse 3000 ms, hitTime 1800 ms (`DP/stats/skills/00200-00299.xml:1342-1364`). Hate effectPoint is -138 to -646 (`:1345`).
2. **Execution:**
   - Target must be alive NPC. If already spoiled (`target.is_spoiled == true`), cast fails with `ALREADY_SPOILED` (`DPJ/handlers/effecthandlers/instant/Spoil.java:57-60`).
   - Magic success rate evaluated via `Formulas.calcMagicSuccess` based on skill magic level vs NPC level (`:40-42`).
   - If success: monster state set to `is_spoiled = true`, spoiler entity ID and party ID recorded.

### 5.2 Sweeper skill 42 and corpse sweep resolution

1. **Skill Properties:** Level 1, hitTime 500 ms, reuse 500 ms, MP 3, castRange 20, target `NPC_BODY` (`DP/stats/skills/00000-00099.xml:1465-1480`).
2. **Validation:**
   - Target NPC must be dead and spoiled (`is_spoiled == true && is_swept == false`).
   - Target must satisfy spoil ownership: caster must be the spoiler OR a member of the spoiler's party (`GJ/model/actor/L2Attackable.java:1027-1035; DPJ/handlers/effecthandlers/instant/Sweeper.java:53-57`).
   - Sweeper inventory capacity and weight checked (`:57`).
3. **Item Transfer:**
   - Transfer all items from `CorpseState.sweep_items`.
   - If party loot mode is 2 or 4 (`INCLUDING_SPOIL`), route items through party distribution; otherwise grant directly to sweeper (`L2Party.java:197-219; VR5 A62`).
   - Ledger op `sweep` recorded for each granted instance.
   - Corpse marked `is_swept = true` and despawned immediately.

### 5.3 Corpse decay lifecycle and spoil extension

1. **Base Decay:** Normal corpse decay is `DefaultCorpseTime = 7` s (70 ticks) (`GC/npc.properties:86`).
2. **Spoiled Extension:** If monster is spoiled, decay timer is extended by `SpoiledCorpseExtendTime = 10` s (100 ticks), giving 17 s (170 ticks) total (`GC/npc.properties:90; GJ/taskmanager/DecayTaskManager.java:61-63`).
3. **Template Overrides:** If NPC template specifies `corpseTime`, that template value replaces `DefaultCorpseTime` (`L2NpcTemplate.java:139`).

---

## 6. NPC vendors and buylists

### 6.1 Buylist data and NPC proximity

1. **195 XML Buylists:** (`DP/buylists/*.xml`).
2. **NPC Proximity Guard:** Vendor intents (`VendorBuy`, `VendorSell`, `VendorRefund`) require that the player is within radius 3 tiles ($\le 3{,}000$ millitiles) of a valid merchant NPC, matching the Blacksmith / Class Master proximity guard (P4Z §7.1; SN4 D10).

### 6.2 Buy price calculation, tax rounding and castle treasury

1. **Tax Formula:**
   $$\text{buy\_price} = \text{trunc}\left(\text{price} \times \left(1 + \text{baseTaxRate} + \text{castleTaxRate}\right)\right)$$
   In pinned H5, `baseTaxRate` is hard-coded to 0 (`BuyListData.java:119`).
   Castle tax rate is the no-owner rate of 15% (150/1000) until castles exist in Phase 6 (`CastleManager.java:165-176; RequestBuyItem.java:182`).
   In integer per-mille arithmetic:
   $$\text{tax\_amount} = \left\lfloor \frac{\text{price} \times 150}{1{,}000} \right\rfloor$$
   $$\text{final\_buy\_price} = \text{price} + \text{tax\_amount}$$
2. **Castle Treasury Skim:**
   The castle treasury receives `trunc(subTotal * castleTaxRate)` (`RequestBuyItem.java:234`). In Phase 5, this skim is recorded as a ledgered sink.
3. **Inventory Validation:**
   Slots and weight capacity validated before transaction (`RequestBuyItem.java:196-206`). Ledger op `vendor_buy`.

### 6.3 Sell price calculation and adena minting

1. **Sell Price Formula:**
   $$\text{sell\_price} = \left\lfloor \frac{\text{item.getReferencePrice}()}{2} \right\rfloor$$
   No tax is deducted from NPC sales (`GJ/network/clientpackets/RequestSellItem.java:142`).
2. **Adena Minting Faucet:**
   Selling to an NPC mints adena via `addAdena("Sell", totalPrice, ...)` (`:142, 155; VR5 A2`).
3. **Validation:**
   - Item must have `is_sellable = true`.
   - Total adena after sale must not exceed `MaxAdena = 99,900,000,000` (`:144`).
   - Ledger op `vendor_sell`.

### 6.4 Refund tab 12-item FIFO buffer

1. **Buffer Size:** Capped at exactly 12 items (`GC/general.properties:458; GJ/model/itemcontainer/PcRefund.java:60-64`).
2. **FIFO Eviction:** When a 13th item is sold, the oldest item (index 0) is destroyed permanently (`:60-64`).
3. **Buyback Price:** Buying back an item from the refund tab costs exactly `sell_price` (the amount received when sold), with no tax markup.
4. **Session Lifetime:** The refund buffer is cleared upon player logout (`L2PcInstance.java:2539-2543, 8678`).

### 6.5 Restock schedules and package sales

1. **Limited Buylist Items:** Items specifying `count` and `restock_delay` replenish inventory periodically. Tracked in `Phase5Snapshot.vendor_restocks`.
2. **Package Sale:** Supports selling pre-packaged goods directly from inventory.

---

## 7. Multisell and exchange engine

### 7.1 Generic exchange engine and ExchangeAudit reuse

Phase 5 generalizes the Phase 4 D37/D52 special-ability exchange engine into a generic multisell engine across all 196 multisell lists (`DP/multisell/*.xml`).

1. **Atomicity:** All ingredient consumptions and product grants execute in a single atomic critical mutation.
2. **Audit & Ledger:** Emits a single `ItemExchanged` outbox fact (subject `nightfall.item.exchanged`) referencing the multisell list ID and entry ID. Ledger records ordered non-consume grant rows first, followed by the single net Consume aggregate row for ingredients (Phase 4 D66/D69 order).

### 7.2 maintainEnchantment semantics

When an entry has `maintainEnchantment = true` (`GJ/model/multisell/PreparedEntry.java:81-83; GJ/network/clientpackets/MultiSellChoose.java:355-362`):
1. **Enchant Level:** Copied exactly from the base weapon/armour to the new product instance.
2. **Augmentation:** Copied exactly (`augment_id = (stat34 << 16) | stat12`).
3. **Elemental Attributes:** Attack and defence element attributes copied to product.

### 7.3 Tax application on multisell ingredients

When a multisell list has `applyTaxes = true`:
- Castle tax applies only to adena ingredients flagged `isTaxIngredient = true` (`PreparedEntry.java:44-53, 68-70`).
- Tax added to ingredient count: $\text{count}_{\text{taxed}} = \text{count} + \text{round}(\text{count} \times \text{taxRate})$.

---

## 8. Crafting and recipes

### 8.1 Recipe book registration and limits

1. **Recipe Books:**
   - Dwarven Recipe Book: capacity 50 (`DwarfRecipeLimit = 50; GC/character.properties:488`).
   - Common Recipe Book: capacity 50 (`CommonRecipeLimit = 50; :489`).
2. **Registration:**
   - Registration consumes the recipe item permanently from inventory.
   - Player Create Item rank must meet or exceed recipe's minimum level.
   - Once registered, recipe ID is recorded in player's recipe book. Registering a duplicate recipe is rejected with `REJECT_REASON_RECIPE_ALREADY_KNOWN`.

### 8.2 Create Item and Create Common Item skills

1. **Create Item (172):** 10 ranks learned at levels 5, 20, 28, 36, 43, 49, 55, 62, 70, 82 (`DP/stats/skills/00100-00199.xml:1435-1441`).
2. **Create Common Item (1320):** 9 ranks learned at levels 5..70 (`DP/stats/skills/01300-01399.xml:547`).

### 8.3 Craft resolution, MP cost and 100% failure loss

When a player invokes `CraftItemRequest`:
1. **Preflight:** Recipe must be known; player must have required Create Item rank; player must have sufficient inventory capacity.
2. **MP Cost:** Deducted at craft resolution (`GJ/RecipeController.java:374`). If current MP is insufficient, craft is rejected before consumption.
3. **Materials Consumption:** All recipe ingredients are consumed from inventory **BEFORE** the success roll (`:363-387`).
4. **Success Roll:**
   - Draw $r = \text{next\_u32}() \bmod 100$.
   - Success condition: $r < \text{recipe.successRate}$.
5. **Failure Loss:**
   - If $r \ge \text{recipe.successRate}$, craft fails.
   - **100% material loss with zero refund** (`:380-387`). No product granted.
6. **Ledger:** Op `craft` records material consumptions and product grant.

### 8.4 Masterwork rare item generation

Pinned default has `CraftMasterwork = True` (`GC/character.properties:484`):
1. Exactly 583 recipes carry `productionRare` in `DP/recipes.xml`.
2. If the base craft roll succeeds AND the recipe has `productionRare`:
   - Draw $m = \text{next\_u32}() \bmod 1{,}000$.
   - Success condition: $m < 30$ (30 per-mille = 3%).
   - If success: grant the `productionRare` template instance instead of the standard product.

### 8.5 Private manufacture workshops

1. Dwarf sets up private manufacture store specifying fee and craftable recipes from their Dwarven recipe book.
2. Customer interacts with workshop, provides ingredients and pays the adena fee.
3. Fee is transferred to Dwarf **before** the craft roll and is not refunded on failure (`RecipeController.java:350-387`).
4. Roll is evaluated against the Dwarf's skill rank and per-zone RNG.

---

## 9. Player trade and private stores

### 9.1 Two-phase atomic trade state machine

Trade protocol (`GJ/model/TradeList.java:330-450`):
1. **Phase 1: Offer & Lock**
   - Player A sends `TradeRequest`, Player B accepts.
   - Players add/remove items to/from trade lists.
   - Both players must press Lock (`TradeLock`). Once locked, offers cannot be modified.
2. **Phase 2: Confirmation & Execution**
   - Both players must press Confirm (`TradeConfirm`).
   - If either player cancels or moves $> 150$ units, session aborts and items remain with original owners.
3. **Deadlock Prevention:**
   - Acquire entity locks in strict ascending `entity_id` UUID order (`TradeList.java:353-365`).

### 9.2 Critical mutation execution and capacity checks

When both confirm:
1. Re-validate item existence, unequipped state, tradeability, and ownership.
2. Validate inventory slot and weight capacity (`can_accept`) for both players simultaneously.
3. Execute single atomic critical mutation:
   - Move offered items from Player A $\to$ Player B (ledger op `trade_out` for A, `trade_in` for B).
   - Move offered items from Player B $\to$ Player A (ledger op `trade_out` for B, `trade_in` for A).
   - All rows reference a single shared Trade receipt ID.

### 9.3 Private stores

1. **Store Types:** Sell, Buy, Package Sell, Manufacture.
2. **Slot Limits (`GC/character.properties:306-313`):**
   - Dwarf: 4 Sell / 5 Buy.
   - Other classes: 3 Sell / 4 Buy.
3. **Offline Stores Disabled:** `OfflineTradeEnable = False` (`GC/customs.properties:190; A11`). Stores terminate on logout.

### 9.4 Session distances, timeouts and flood protection

- Nightfall specifies a maximum trade distance of 150 units and a 120 s idle timeout (deviation from retail L2J, which used client packet intervals).
- Trade actions are rate-limited via `TransactionInterval = 10` ticks (1 s) (`GC/floodprotector.properties:85`).

---

## 10. Mail, storage, auction and manor movement contracts

### 10.1 DB-only containers with zone-mediated movement

Under A2, storage containers (warehouse, freight, mailbox, auction house, manor) are DB-only tables. However, **all live item mutations into or out of player inventories must execute through the zone actor's critical path**.

### 10.2 Private warehouse and freight contracts

1. **Private Warehouse:**
   - Capacity: 100 slots for non-Dwarves, 120 slots for Dwarves (`GC/character.properties:325-326`).
   - Deposit Fee: **30 adena per item entry** (`GJ/network/serverpackets/SendWareHouseDepositList.java:117-118`).
   - Deposit: zone critical mutation consumes item and 30 adena from inventory, writes row to `warehouse_items`. Ledger op `deposit`.
   - Withdraw: zone critical mutation checks inventory capacity, deletes row from `warehouse_items`, grants item to inventory. Ledger op `withdraw`.
2. **Freight:**
   - Capacity: 200 slots (`GC/character.properties:332`).
   - Fee: **1,000 adena per item entry** (`:335; GJ/network/clientpackets/RequestPackageSend.java:109`).
   - Destination: target character on same account in designated town.

### 10.3 Mail system, attachments and COD

1. **Constraints (`GJ/network/clientpackets/RequestSendPost.java`):**
   - Sender must be in a **peace zone** (`:130, 208`).
   - Items must be unequipped and tradeable (`:250`).
   - Maximum 8 attachments (`:63`).
   - Fee: 100 adena base + 1,000 adena per attached item slot (`:67-68, 255`).
   - Subject max 128 chars, body max 512 chars (`:61-62`).
   - Flood interval: `SendMailInterval = 100` ticks ($\approx 10$ s) (`GC/floodprotector.properties:106`).
2. **Lifecycles (`GJ/model/entity/Message.java:39-40, 98`):**
   - Standard mail expiry: 360 hours (15 days).
   - COD payment expiry: 12 hours (`COD_EXPIRATION = 12`).
   - Expiry returns attachments to sender's mailbox.

### 10.4 Item auction and manor hooks

- `ItemAuctionEnabled = True` (`GC/general.properties:655`).
- Manor seeds and crop loops run on fixture schedules; payouts and deliveries route through zone critical mutations.

---

## 11. Persistence, migrations and ledger audit

### 11.1 Migrations from 000012 and table schemas

Phase 5 schema migrations start at `m20261013_000013_economy.rs`:

```sql
-- 000013: Phase 5 storage, mail, recipes, and private stores

CREATE TABLE character_warehouses (
    character_id UUID PRIMARY KEY REFERENCES characters(id),
    slot_limit INTEGER NOT NULL DEFAULT 100,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE warehouse_items (
    id BIGINT PRIMARY KEY, -- ItemInstanceId
    character_id UUID NOT NULL REFERENCES characters(id),
    template_id INTEGER NOT NULL,
    count BIGINT NOT NULL CHECK (count > 0),
    enchant BIGINT NOT NULL DEFAULT 0,
    augment_id BIGINT,
    created_tick BIGINT NOT NULL
);
CREATE INDEX idx_warehouse_items_char ON warehouse_items(character_id);

CREATE TABLE character_freight (
    character_id UUID PRIMARY KEY REFERENCES characters(id),
    slot_limit INTEGER NOT NULL DEFAULT 200,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE freight_items (
    id BIGINT PRIMARY KEY,
    character_id UUID NOT NULL REFERENCES characters(id),
    destination_town_id INTEGER NOT NULL,
    template_id INTEGER NOT NULL,
    count BIGINT NOT NULL CHECK (count > 0),
    enchant BIGINT NOT NULL DEFAULT 0,
    augment_id BIGINT,
    created_tick BIGINT NOT NULL
);
CREATE INDEX idx_freight_items_char ON freight_items(character_id);

CREATE TABLE mail_messages (
    id BIGSERIAL PRIMARY KEY,
    sender_id UUID NOT NULL REFERENCES characters(id),
    receiver_id UUID NOT NULL REFERENCES characters(id),
    subject VARCHAR(128) NOT NULL,
    body VARCHAR(512) NOT NULL,
    is_cod BOOLEAN NOT NULL DEFAULT FALSE,
    cod_adena BIGINT NOT NULL DEFAULT 0,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    is_read BOOLEAN NOT NULL DEFAULT FALSE,
    is_returned BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX idx_mail_receiver ON mail_messages(receiver_id);

CREATE TABLE mail_attachments (
    id BIGINT PRIMARY KEY,
    message_id BIGINT NOT NULL REFERENCES mail_messages(id) ON DELETE CASCADE,
    template_id INTEGER NOT NULL,
    count BIGINT NOT NULL CHECK (count > 0),
    enchant BIGINT NOT NULL DEFAULT 0,
    augment_id BIGINT
);

CREATE TABLE character_recipes (
    character_id UUID NOT NULL REFERENCES characters(id),
    recipe_id INTEGER NOT NULL,
    is_dwarven BOOLEAN NOT NULL,
    learned_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (character_id, recipe_id)
);

CREATE TABLE economy_counters (
    bucket_hour TIMESTAMPTZ NOT NULL,
    template_id INTEGER NOT NULL,
    reason VARCHAR(32) NOT NULL,
    created_count BIGINT NOT NULL DEFAULT 0,
    destroyed_count BIGINT NOT NULL DEFAULT 0,
    adena_in BIGINT NOT NULL DEFAULT 0,
    adena_out BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (bucket_hour, template_id, reason)
);
```

### 11.2 Phase5Checkpoint and CharacterCheckpoint integration

Extend `CharacterCheckpoint` with:
```rust
pub struct Phase5Checkpoint {
    pub schema_version: u32,
    pub rules_hash: [u8; 32],
    pub dwarven_recipes: Vec<u32>,
    pub common_recipes: Vec<u32>,
    pub refund_tab: Vec<RetainedRefundItem>,
}
```

### 11.3 Ledger operations under Phase 4 D66/D69 order

Phase 5 registers new `item_ledger` operation kinds:
- `drop`, `pickup`, `spoil`, `sweep`, `vendor_buy`, `vendor_sell`, `craft`, `trade_out`, `trade_in`, `deposit`, `withdraw`, `mail_out`, `mail_in`.

**D66/D69 Canonical Order Rule:**
For each `(owner, tick, instance)`:
1. All non-consume rows (grants, refunds, trades-in, pickups, vendor-buys, craft products, sweeps) execute FIRST in cause order.
2. The single net `Consume` aggregate row executes LAST, with cause list detailing all consumptions and contributing receipts.

### 11.4 Outbox subjects and economy counters rollups

Outbox subjects published via NATS JetStream (C9 pattern):
- `nightfall.item.dropped`
- `nightfall.item.picked_up`
- `nightfall.item.spoiled`
- `nightfall.item.swept`
- `nightfall.item.vendor_bought`
- `nightfall.item.vendor_sold`
- `nightfall.item.crafted`
- `nightfall.item.traded`
- `nightfall.item.deposited`
- `nightfall.item.withdrawn`
- `nightfall.item.mailed`

---

## 12. Replay, codec and digest coverage

### 12.1 Strict V7 version boundary

- Snapshot schema version: **11**
- Replay record schema: **9**
- State digest version: **StateDigestVersion::BinaryV7**
- `PHASE5_ENABLED` gate mirrors `PHASE4_ENABLED`: unset $\to$ V7 for production; "0" $\to$ V6 legacy behaviour; "1" $\to$ V7. Historical recordings remain byte-exact.

### 12.2 Codec allocations (PbOutput, PbCommandKind, PbPlayerLoad)

Read from `apps/api/src/application/replay_log/codec.rs`:
- High-water mark for `PbOutput`: **41** (Phase 4 V6 enriched event, P4Z §10.1; D26).
- High-water mark for `PbCommandKind`: **31** (Phase 4 range extension, P4Z §10.1).
- High-water mark for `PbPlayerLoad`: **10** (Phase 4 admission DTO, P4Z §10.1).

Phase 5 allocates the next free tags:

| Target | Allocated Tags | Semantic Usage |
|---|---|---|
| **`PbOutput`** | 42 | `GroundItemSpawn` |
| | 43 | `GroundItemDespawn` |
| | 44 | `TradeSessionUpdate` |
| | 45 | `PrivateStoreUpdate` |
| | 46 | `RecipeBookSync` |
| | 47 | `VendorBuylistSync` |
| | 48 | `EconomyOperationResult` |
| | 49 | `Phase5Projection` (checkpoint, economy ledger plans, receipts) |
| | 50 | `V7EnrichedEvent` |
| **`PbCommandKind`** | 32 | `PickUp` |
| | 33 | `VendorBuy` |
| | 34 | `VendorSell` |
| | 35 | `VendorRefund` |
| | 36 | `Multisell` |
| | 37 | `RegisterRecipe` |
| | 38 | `CraftItem` |
| | 39 | `TradeRequest` |
| | 40 | `TradeAnswer` |
| | 41 | `TradeAddItem` |
| | 42 | `TradeRemoveItem` |
| | 43 | `TradeLock` |
| | 44 | `TradeConfirm` |
| | 45 | `TradeCancel` |
| | 46 | `SetPrivateStore` |
| | 47 | `BuyPrivateStore` |
| | 48 | `WarehouseDeposit` |
| | 49 | `WarehouseWithdraw` |
| | 50 | `SendMail` |
| **`PbPlayerLoad`** | 11 | `Phase5PlayerLoad` (recipes, refund entries, active store) |

### 12.3 Binary digest and evidence facts

State digest V7 incorporates domain-separated Phase 5 state:
- All active GroundItems (ID, instance ID, template, count, position, protection owner, protection tick, decay tick).
- All active Corpses (ID, template, spoiled flag, spoiler ID, sweep items, decay tick).
- Vendor restock counts and last restock ticks.
- Active trade session states and locked offers.
- Active private stores.
- Player recipe book sets.

---

## 13. Additive wire and API mapping

Verified against `packages/proto/nightfall/v1/{world,game,items}.proto`:
- `ClientMessage.oneof intent` ended at **28** (`CrystallizeItemRequest`). Next free: **29**.
- `RejectReason` ended at **37** (`REJECT_REASON_CRYSTALLIZE_LEVEL_TOO_LOW`). Next free: **38**.
- `WorldEvent.oneof event` ended at **33** (`ItemOperationResult`). Next free: **34**.
- `EntitySpawn` fields ended at **29** (`repeated VisibleEquipment visible_equipment`). Next free: **30**.
- `StatsChanged` fields ended at **26** (`uint64 inventory_revision`). Next free: **27**.

New proto file: `packages/proto/nightfall/v1/economy.proto`.

### 13.1 ClientMessage intent payloads (tag 29+)

```protobuf
// packages/proto/nightfall/v1/world.proto
message ClientMessage {
  uint32 seq = 1;
  oneof intent {
    // 10..28 Phase 1-4 intents...
    PickUpItemRequest pick_up_item = 29;
    VendorBuyRequest vendor_buy = 30;
    VendorSellRequest vendor_sell = 31;
    VendorRefundRequest vendor_refund = 32;
    MultisellRequest multisell = 33;
    RegisterRecipeRequest register_recipe = 34;
    CraftItemRequest craft_item = 35;
    TradeRequest trade_request = 36;
    TradeAnswerRequest trade_answer = 37;
    TradeSetItemRequest trade_set_item = 38;
    TradeLockRequest trade_lock = 39;
    TradeConfirmRequest trade_confirm = 40;
    TradeCancelRequest trade_cancel = 41;
    SetPrivateStoreRequest set_private_store = 42;
    BuyPrivateStoreRequest buy_private_store = 43;
    WarehouseDepositRequest warehouse_deposit = 44;
    WarehouseWithdrawRequest warehouse_withdraw = 45;
    SendMailRequest send_mail = 46;
  }
}
```

### 13.2 WorldEvent payloads (tag 34+)

```protobuf
// packages/proto/nightfall/v1/world.proto
message WorldEvent {
  oneof event {
    // 1..33 Phase 1-4 events...
    GroundItemSpawn ground_item_spawn = 34;
    GroundItemDespawn ground_item_despawn = 35;
    TradeSessionChanged trade_session_changed = 36;
    PrivateStoreChanged private_store_changed = 37;
    RecipeBookSync recipe_book_sync = 38;
    BuylistWindow buylist_window = 39;
    MultisellWindow multisell_window = 40;
    CraftResult craft_result = 41;
    EconomyOperationResult economy_operation_result = 42;
  }
}
```

### 13.3 RejectReason enum values (tag 38+)

```protobuf
enum RejectReason {
  // 0..37 Phase 1-4 reasons...
  REJECT_REASON_GROUND_ITEM_PROTECTED = 38;
  REJECT_REASON_GROUND_ITEM_NOT_FOUND = 39;
  REJECT_REASON_NOT_SPOILED = 40;
  REJECT_REASON_ALREADY_SPOILED = 41;
  REJECT_REASON_NOT_SPOIL_OWNER = 42;
  REJECT_REASON_VENDOR_OUT_OF_STOCK = 43;
  REJECT_REASON_RECIPE_ALREADY_KNOWN = 44;
  REJECT_REASON_RECIPE_BOOK_FULL = 45;
  REJECT_REASON_CREATE_ITEM_RANK_TOO_LOW = 46;
  REJECT_REASON_TRADE_TARGET_INVALID = 47;
  REJECT_REASON_TRADE_LOCKED = 48;
  REJECT_REASON_STORE_SLOT_LIMIT = 49;
  REJECT_REASON_WAREHOUSE_FULL = 50;
  REJECT_REASON_MAIL_ATTACHMENTS_EXCEEDED = 51;
  REJECT_REASON_NOT_IN_PEACE_ZONE = 52;
  REJECT_REASON_ITEM_NOT_TRADEABLE = 53;
}
```

### 13.4 EntitySpawn (tag 30+) and StatsChanged (tag 27+)

```protobuf
// In EntitySpawn:
message EntitySpawn {
  // ... tags 1..29 ...
  GroundItemDetail ground_item_detail = 30; // present iff kind == ENTITY_KIND_GROUND_ITEM
}

// In StatsChanged:
message StatsChanged {
  // ... tags 1..26 ...
  uint64 adena_count = 27; // owner-only adena summary
}
```

### 13.5 GameService RPC additions

```protobuf
// packages/proto/nightfall/v1/game.proto
service GameService {
  // ... Phase 1-4 RPCs ...
  rpc ListBuylist(ListBuylistRequest) returns (ListBuylistResponse);
  rpc GetWarehouse(GetWarehouseRequest) returns (GetWarehouseResponse);
  rpc GetRecipeBook(GetRecipeBookRequest) returns (GetRecipeBookResponse);
  rpc ListMail(ListMailRequest) returns (ListMailResponse);
  rpc GetMail(GetMailRequest) returns (GetMailResponse);
  rpc ListAuctions(ListAuctionsRequest) returns (ListAuctionsResponse);
  rpc GetManorInfo(GetManorInfoRequest) returns (GetManorInfoResponse);
}
```

---

## 14. Independent oracle, regression and acceptance plan

### 14.1 Python Fraction oracle vector families

Implement `economy_oracle.py` using Python's `fractions.Fraction` without floats:

1. **Family 1 — Precise Drop Calculations:**
   - Test base chances from 0.001% to 850%.
   - Assert exact $M_{\text{base}}$ guaranteed multipliers and remainder draw boundaries.
   - Enumerate all residue cases for single-item groups and multi-item normalized CDF walks.
2. **Family 2 — Deep Blue Level Gap Windows:**
   - Test level difference $d \in [-15, 5]$.
   - Assert exact integer percentages: $d \ge -5 \implies 100\%$, $d \le -10 \implies 10\%$, and exact 18-point steps for $d \in \{-9, -8, -7, -6\}$.
   - Assert raid boss level gap zeroing out at $d \le -7$.
3. **Family 3 — Adena Party Split:**
   - Test total adena from 1 to $99{,}900{,}000{,}000$ across party sizes $K \in \{2, 3, 5, 7, 9\}$.
   - Assert integer base shares and remainder residue distribution.
4. **Family 4 — Vendor Pricing & Castle Taxes:**
   - Test item base reference prices from 1 to $100{,}000{,}000$.
   - Assert sell price is exactly $\lfloor \text{price} / 2 \rfloor$.
   - Assert buy price with 15% castle tax: $\text{price} + \lfloor \text{price} \times 150 / 1{,}000 \rfloor$.
   - Assert FIFO eviction for refund buffer at 13 items.
5. **Family 5 — Multisell Ingredient Scaling:**
   - Test ingredient counts with and without `applyTaxes`.
   - Assert `maintainEnchantment` bitwise copy of augment and enchant levels.
6. **Family 6 — Crafting Rolls & Masterwork:**
   - Test success rates @ 100%, 70%, 60%, 25%, 10%.
   - Assert exact residue boundaries and masterwork 30/1000 roll.
7. **Family 7 — PK Chaotic Death Drops:**
   - Test victim karma $> 0$, PK kills $\ge 6$.
   - Assert all equipped items unequipped before roll.
   - Assert weapon 10%, equip 40%, inventory 50%, capped at 10 items.
8. **Family 8 — Timers & Decay:**
   - Assert normal item decay = 6,000 ticks (600 s).
   - Assert herb decay = 600 ticks (60 s).
   - Assert normal corpse decay = 70 ticks (7 s); spoiled corpse decay = 170 ticks (17 s).
   - Assert normal protection = 150 ticks (15 s); raid CC protection = 9,000 ticks (900 s).

### 14.2 Unit, property, durability and integration targets

1. **Determinism:** Two identical replay runs from seed produce byte-identical digests, GroundItem positions, drops, and craft results.
2. **Conservation of Items:** Sum of instances in ground items + player inventories + destroyed items + sold items is strictly conserved.
3. **Capacity Boundaries:** No player can ever exceed inventory slots (80/100) or weight limits without entering overload penalties.
4. **Replay Fixture Check:** `replay-check.sh` passes byte-exact on all historical fixtures.

### 14.3 Simulation scenario matrix

Client-facing simulation scenarios in `apps/client-unreal/Scenarios/`:
- `5-drop-keltir.nfs`: Kill monster, verify drop chance calculation, observe ground spawn, pick up item.
- `5-party-loot-modes.nfs`: Multi-client party testing all 5 party distribution modes and adena even split.
- `5-spoil-sweep.nfs`: Dwarf casts Spoil 254, kills mob, casts Sweeper 42, receives sweep items.
- `5-vendor-buy-sell.nfs`: Buy from vendor with castle tax, sell back at price/2, inspect refund tab, evict 13th item.
- `5-multisell-exchange.nfs`: Exchange weapon via multisell with `maintainEnchantment=true`, verify enchant + augment preserved.
- `5-craft-dwarf.nfs`: Dwarf registers recipe, crafts item, verifies 100% material loss on failure, verifies masterwork.
- `5-player-trade.nfs`: Two-phase atomic trade between two players, both confirm, atomic transfer.
- `5-private-store.nfs`: Open private sell store, second player purchases, store updates.
- `5-mail-attachment.nfs`: Send mail with attachment in peace zone, pay 1,100 adena fee, receiver claims attachment.
- `5-warehouse-freight.nfs`: Deposit item in warehouse (30 adena fee), withdraw; freight item to character.
- `5-chaotic-pk-drop.nfs`: PK player dies, strips gear, drops up to 10 items on ground with 15 s protection.

---

## 15. Open architect questions

- **QUESTION: GroundItem entity ID space:** Does `GroundItemId` draw from the same monotone per-zone u64 allocator range as `ItemInstanceId` (P4Z I1; D8), or does it maintain a distinct entity range? (Recommendation: GroundItem references an underlying `ItemInstanceId` directly and uses the standard entity ID for spatial AOI tracking).
- **QUESTION: Refund buffer persistence across reconnects:** Retail L2J clears `PcRefund` upon logout (`L2PcInstance.java:2539-2543`). Should Nightfall persist the 12-item refund tab in `Phase5Checkpoint` during brief disconnects within the same epoch, or clear it on disconnect to match retail? (Recommendation: Retain in `Phase5Checkpoint` during active zone session re-admission, clear on explicit character logout/quit).
- **QUESTION: Trade session abort penalty on disconnect:** If a player disconnects during an active trade session that has been locked but not confirmed, does the engine emit a cancel rejection or automatically roll back? (Recommendation: Automatically roll back and unlock items on disconnect).
- **QUESTION: Economy counters aggregation cadence:** Are `economy_counters` table rollups populated synchronously in the checkpoint transaction or asynchronously via NATS JetStream outbox consumer? (Recommendation: Asynchronously via Phase 9 outbox relay consumer to maintain tick performance budget).
