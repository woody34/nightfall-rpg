> **BINDING RECORD — Phase 5 design, incorporating architect A1–A20, owner decisions P1–P4, binding errata E1–E12, and architect dispositions B1–B14; independent review 1 addressed in full.** This is the implementation contract for Snapshot 11 / Record 9 / Binary V7 epochs. It establishes exact implementation contracts for zone engine, economy, persistence, replay codec, wire contracts, and independent oracle verification. Note: Citations to `codex/phase4-zone-engine` reflect HEAD `10ca789`; the final Phase 4 integration SHA may move line numbers.

# Phase 5 zone engine: economy, drops, trade, and crafting

## Source authority and reconciliation

Citations use:
- **Z/** = `apps/api/src/domain/zone/` (evaluated against `codex/phase4-zone-engine` HEAD `10ca789`)
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
- **AD5** = `/home/matt-woodruff/.config/Claude/side-session-notes/phase5-agy-2026-10-13/architecture-decisions.md` (§0 P1–P4, §1 A1–A20, §5 B1–B8, §6 E1–E6 + B9–B12, §7 E7–E12 + B13–B14)
- **OD5** = `/home/matt-woodruff/.config/Claude/side-session-notes/phase5-agy-2026-10-13/owner-decisions.md`
- **VR5** = `/home/matt-woodruff/.config/Claude/side-session-notes/phase5-agy-2026-10-13/reference-brief-verification.md`
- **DR1** = `/home/matt-woodruff/.config/Claude/side-session-notes/phase5-agy-2026-10-13/design-review-1-claude.md` (Findings F1–F36)
- **P4Z** = `docs/plans/phase-4-zone-design.md`
- **SN4** = `/home/matt-woodruff/.config/Claude/side-session-notes/phase4-codex-2026-10-12/architecture-decisions.md` (D1–D75)

All executable formulas here use integers, $Q=1{,}000{,}000$ fixed-point arithmetic (`i64` with checked `i128`), or exact rational ratios (`ExactRatio` via `u128`/`i128`). Floating-point arithmetic, runtime wall clocks, `HashMap`, and un-ordered iteration are strictly forbidden (A1; worker-common).

| Area | Pinned H5 source & verified authority | Nightfall Phase 5 binding treatment |
|---|---|---|
| **Drop & spoil XML counts** | 124 NPC XML files exist; exactly **59** contain `<dropLists>/<death>` (14,589 `<group chance>` blocks and 744 ungrouped `<item>` rows); exactly **35** contain `<corpse>` (2,260 NPCs, 7,335 items; 234 per-NPC `corpseTime` overrides). `<corpse>` contains **only ungrouped** `<item>` rows (`DP/stats/npcs/*.xml`; `GJ/model/drops/DropListScope.java:31-32; L2Attackable.java:838-840`). (E1; VR5 A22, C20, C21). | P3/A6 amended per E1. 59 death XMLs, 35 corpse XMLs. Spoil list is rolled first on death if spoiled, followed by the single document-order death list mixing groups and ungrouped items. |
| **Drop chance precision** | Drop chances in XML have up to 20 decimal places (minimum `0.0003000000142492354`%; maximum `188.85499572753906`%). Multi-item group shares normalized via `GroupedGeneralDropItem.normalizeMe` (`:193-215`). | Represented as exact rational $num / 10^k$ ($k \le 20$) in `u128`. Draw consumes uniform 64-bit word $r \in [0, 2^{64})$ with exact test $r \times \text{den} < \text{num} \times 2^{64}$ (strict `<`). Multi-item selection walks normalized rational shares. (F1). |
| **Drop evaluation order & ground offsets** | Spoil rolled first; death list rolled in single document order (`NpcData.java:453-470; L2NpcTemplate.java:437-458`). Group roll: chance $\to$ [selection] $\to$ [remainder] $\to$ quantity (`IGroupedItemDropCalculationStrategy.java:62-78`). Ground offsets: NPC item offset is $Rnd.get(141) - 70$ for X and Y per created instance under `MultipleItemDrop = True` (`general.properties:354; L2Npc.java:116, 1460-1495`). Respawn jitter is drawn at death (`state_ai.rs:258-264`). | Pinned 5-step death sequence: 1. Kill credit (`maxDealer`); 2. Corpse list; 3. Death list; 4. Ground posX/posY draws per instance ($\pm 70$ L2 units = $\pm 2{,}187$ milli-tiles); 5. `npc_respawn_jitter` draw. (F2). |
| **Chaotic PK drops** | Single loop over `getInventory().getItems()` in canonical order (`L2PcInstance.java:4515-4550`). Base drop check: 40% (`KarmaRateDrop = 40`, `rates.properties:118`). Each equipped item is unequipped immediately BEFORE its own roll (`:4531-4541`). Weapon 10%, other equip 40%, inventory item 50%. Stops at 10 items (`KarmaDropLimit = 10`). Ground offsets: $Rnd.get(50) - 25$ for X and Y (`:3178`). Trigger: victim outside PvP zone (`!victim.isInsideZone(ZoneId.PVP) \|\| pk == null`). Clan war non-karma death drops are vacuous until Phase 7 (`:4479-4483`). | ONE-pass iteration in canonical `ItemInstanceId` ascending order. Unequip-then-roll per equipped item; ground posX/posY draws ($\pm 25$ L2 units = $\pm 781$ milli-tiles); stop at 10. Whole stack dropped. (F3). |
| **Minimal party substrate** | Party max 9 members (`character.properties:600`). 5 distribution modes (`PartyDistributionType.java:25-29`). Adena even split within `PartyRange2 = 1400` L2 units (43,750 milli-tiles) when count > 1 (`L2Party.java:496-505, 591-628`). Highfive level-gap cutoff: 0–9 gap 100%, 10–14 30%, 15–99 0% (`character.properties:769-793`). `BY_TURN` uses `_itemLastLoot` cursor (`L2Party.java:174-189`). Dynamic loot protection evaluation (`isInLooterParty`, `DropProtection.tryPickUp`). | Minimal party substrate implemented as zone-canonical state (B9): `PartyId(Uuid)`, join order, leader, 5 modes, `item_last_loot` cursor, adena split, sweep routing under including-spoil, and level-gap XP/SP cutoff. Included in V7 digest/snapshot. (F4). |
| **Owner-changing ledger transfers** | Phase 4 D67/D69 amended by B13: `ItemInstanceId` NEVER changes across owners (trade, drop/pickup, storage, mail, vendor). PAIRED transfer: `transfer_out` under old owner (`after = None`) and `transfer_in` under new owner (`before = None`), referencing each other with identical canonical fields. Ground items owned by `GroundItem` entity; containers have container IDs. | Paired `transfer_out`/`transfer_in` ledger rows. Operation enums extended additively. Stack merge on pickup/deposit terminates source instance. Spoil op dropped (no instance mutation). (F5, B13). |
| **Replay independence for containers** | Replaying a recording must never query live DB state (E12). RPC mutations (withdraw, mail claim, auction bid, manor turn-in) freeze DB-derived inputs into `PbCommandKind` entries. No `NOW()` or `BIGSERIAL` in zone state. Fenced checkpoint transaction commits container changes in the same transaction as checkpoint, ledger, and outbox. | Replay independence enforced: frozen manifest in replay record; deterministic IDs; identical crash recovery semantics to P4Z §9.2. (F6, E12). |
| **Recipes & masterwork** | 1,000 recipes (830 Dwarven, 170 Common). 582 recipes carry `productionRare` (E1). Rare roll: `Rnd.get(100) < rarity` with per-recipe rarity (1% for 31 recipes, 4% for 315, 20% for 236) and `rareCount` (`RecipeController.java:563-574`). Gated by `CraftMasterwork = True` (`character.properties:484`). MP deducted at `calculateStatUse(false, true)` (`:347, 467-498`). 100% material loss on failure (`:363-387`). | Per-recipe rarity (1/4/20% of 100) and `rareCount` hashed into `Phase5Rules`. Pinned execution order: MP $\to$ fee $\to$ materials $\to$ success roll $\to$ rare roll $\to$ product. (F7, E1, E11). |
| **Vendor buylist prices & taxes** | 195 buylist XMLs carry price 0 and no restock data (DP commit `2339b7369d`). Real merchants are 274 Java AI scripts (20,236 entries) with per-entry `baseTax` (10–100%) (`DPJ/ai/npc/**`). Buy price = template reference price $\times (1 + \text{baseTax} + \text{castleTax})$ with double-truncation rounding (`RequestBuyItem.java:181-183`). Castle tax default = 0% (pinned seed, `sql/castle.sql:17-26`). Sell price = reference price / 2 minting adena (`RequestSellItem.java:142, 155`). Refund buy-back = $\lfloor (\text{count} \times \text{referencePrice}) / 2 \rfloor$ (`RequestRefundItem.java:155`). 12-item FIFO buffer, cleared on logout (`L2PcInstance.java:2539-2543, 8678`). | Merchant data generated from 274 AI scripts (E2, E9). Castle tax default 0% (E3). Exact integer double-truncation emulation proven by oracle. 12-item refund tab is zone-only, cleared on departure/logout with destroy ledger rows (B6). (F8, F9, F17). |
| **Engine types & coordinates** | Zone positions use integer milli-tiles `Vec2Fixed` (`Fixed(i32)`, `UNITS_PER_TILE = 1000`, `Z/fixed.rs:18-40; entity.rs:72`). Floats are strictly prohibited (A1). `EntityId` is UUID (`Z/entity.rs:46-49`); `ItemInstanceId` is `u64` (`Z/item.rs:17-48`); template IDs are `u32`. Conversion: `floor(L2 * 1000 / 32)`. | All coordinates use `Vec2Fixed`. Pickup range: 36 L2 units = 1,125 milli-tiles (`L2PlayerAI.java:260`). Vendor interaction: 150 L2 units = 4,687 milli-tiles (`L2Npc.java:114`). Trade interaction: 150 L2 units = 4,687 milli-tiles (`TradeRequest.java:168`). PartyRange: 1600 L2 = 50,000 milli-tiles. PartyRange2: 1400 L2 = 43,750 milli-tiles. Sweeper: 20 L2 = 625 milli-tiles. (F10, F25, E11). |
| **Corpse lifecycle integration** | Engine already has `NpcAi.corpse_until` set from `brain.corpse_decay_ticks` at death, with respawn scheduled at $\ge \text{corpse\_until.next()}$ (`Z/state_ai.rs:236-268; state_digest.rs:536-549`). Living NPC receives Spoil debuff (`Spoil.java:59-67`). Corpse decay: 7 s default (`npc.properties:86`) + 10 s spoil extension (`:90`) = 17 s (234 per-NPC `corpseTime` overrides). Sweeper never despawns corpse immediately (`Sweeper.java:53-67`). | Reuse existing `NpcAi.corpse_until` under V7; living NPC tracks spoiler; death derives `corpse_until` as template `corpseTime` + (spoiled ? 100 ticks : 0); respawn $\ge \text{corpse\_until.next()}$. Corpse persists after sweep until `corpse_until`. Drop parallel corpse map. (F11, F35). |
| **Trade protocol** | Pinned trade interaction radius is 150 L2 units (4,687 milli-tiles) at request and confirm (`TradeRequest.java:168; TradeDone.java:91`). Request timeout is 15 s (`L2PcInstance.java:354, 5038-5048`). Add/remove items invalidates confirmations. No separate `TradeLock` step; when both confirm, atomic swap executes (`TradeList.java:340-399`). Disconnect cancels trade without penalty (B3, B7). | Pinned 150-unit radius; 15 s request expiry; no 120 s timeout; no TradeLock step; single critical mutation; cancel on disconnect/departure/death returns items with pinned notification. (F16, E10). |
| **Freight scope** | Pinned H5 freight transfers items to another character on the SAME ACCOUNT (`getAccountChars`, `RequestPackageSend.java:79-113`), within 150 L2 units of folk NPC. Capacity: 200 slots + `FREIGHT_LIM` stat (`PcFreight.java:66-68`). Fee: 1,000 adena (`GC/character.properties:335`). | Same-account freight (E8). Remove `destination_town_id`. `KarmaPlayerCanUseWareHouse` enforced. (F18, E8). |
| **Multisell tax & enchant maintenance** | `maintainEnchantment` is a `<list>` attribute (31 lists, e.g. `multisell/1003.xml:2; MultiSellChoose.java:352`). Tax applies ONLY to adena ingredients flagged `isTaxIngredient` when `applyTaxes = true`, as `Math.round(count * taxRate)` half-up (`PreparedEntry.java:44-71`). Elemental attributes copy is a no-op per B10. | List-level `maintainEnchantment` copies enchant level and augmentation (`stat34 << 16 \| stat12`); elemental copy is no-op with pinned citation. Tax is `Math.round(count * taxRate)` on `isTaxIngredient` adena only. (F19, B10). |
| **`maxDealer` killer attribution** | Only aggro entries with `damage > 1` within `PartyRange 1600` L2 units (50,000 milli-tiles) count (`L2Attackable.java:341-375`). Summon damage aggregates to master. Online check comes AFTER selection; if offline, fallback is `lastAttacker`. | Pinned `maxDealer` algorithm with deterministic tie-break: earliest tick damage reached maximum, then lowest `EntityId`. (F20). |
| **Drop protection mechanisms** | Two distinct mechanisms: (1) Loot owner protection: 15 s normal / 900 s raid CC $\ge 45$ (`ItemTable.java:203-216; L2PcInstance.java:3824-3842`). (2) DropProtection: 15 s to killer and current party on EVERY NPC drop incl. raids (`L2Npc.java:1476-1478; DropProtection.java:35, 52-63`). (E7). | Both modeled separately. Raid drops always have 15 s DropProtection; CC $\ge 45$ grants 900 s loot rights. Dynamic party check at pickup. (F21, E7). |
| **Subsystems (Manor, Auction, Stores, Mail)** | Manor: sowing seed replaces drop; crop turn-in via `ExchangeAudit`; manager on fixture schedule (B1). Auction: `ItemAuctionManager` state machine on fixture schedule; zone bidding and refunds; finalization awards item (B4). Private stores: zone-only; close on disconnect/departure/restart (B2). Mail: peace zone; 8 attachments; 100+1000 fee; 240 inbox/outbox; 15d expiry; 12h COD. Private manufacture: fee before roll. | Complete §-level contracts, intents, and codec kinds added for all subsystems. (F12, B1, B2, B4). |
| **Item catalogue & skill admission** | Phase 4 D5/D54 closure extended to admit herbs (68 templates), materials, recipes (1,000), buylists, and quest EtcItems. Skills admitted: Spoil 254, Sweeper 42 with `NPC_BODY` target type (B11). Herb immediate effect bindings. | Catalogue admission rule defined in §1.2. Spoil/Sweeper and herbs admitted through Phase 3 cast pipeline. (F13, B11). |
| **Global rate multiplier** | P2 global rate multiplier applies to drop, spoil, adena, and XP/SP rates (`rates.properties`). Applied where pinned rates enter algorithm as an exact rational factor on chance; excess chance $\ge 100\%$ converts to amount multiplier (B14). | Evaluated as exact rational on chance. Amount is not separately multiplied. Pinned defaults are 1x. (F32, B14). |

---

## 1. Identity, canonical state and hydration

### 1.1 Ownership and entity identity

Phase 5 establishes zone-canonical authority for all in-world economy entities and interactions: ground items, corpses, vendor sessions, trade sessions, private stores, and the minimal party substrate. Storage containers (warehouse, freight, mailboxes, auctions, manor schedules) are DB-only, but every transfer into or out of live player inventories executes through the zone actor's critical mutation path (A2).

Add `ground_item.rs`, `party.rs`, `vendor.rs`, `multisell.rs`, `craft.rs`, `trade.rs`, `private_store.rs`, `auction.rs`, `manor.rs`, `storage.rs`, `mail.rs`, `phase5.rs`, and `phase5_checkpoint.rs` to `apps/api/src/domain/zone/` and register them in `SOURCES.md`.

```rust
use uuid::Uuid;
use super::entity::EntityId;
use super::fixed::Vec2Fixed;
use super::item::ItemInstanceId;
use super::Tick;

/// Minimal party identity (B9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartyId(pub Uuid);

/// Ground item zone entity (B5).
///
/// Under B5, a GroundItem is a first-class zone ENTITY (in the `EntityId` UUID space)
/// that CARRIES an `ItemInstanceId` (allocated via the D8/I1 allocator).
/// Wire spawn/despawn and AOI tracking operate on `EntityId`; ledger records use `ItemInstanceId`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroundItem {
    pub entity_id: EntityId,
    pub instance_id: ItemInstanceId,
    pub template_id: u32,
    pub count: u64,
    pub enchant: u32,
    pub augment_id: Option<u32>,
    pub pos: Vec2Fixed,
    pub dropped_by: Option<EntityId>,
    /// Protection owner (maxDealer or dying player).
    pub protection_owner: Option<EntityId>,
    /// Protection deadline for killer/party (15 s normal / 900 s raid CC).
    pub protection_until_tick: Tick,
    /// Absolute decay deadline. None for player drops (DestroyPlayerDroppedItem=False).
    pub decay_tick: Option<Tick>,
    pub is_herb: bool,
    pub created_tick: Tick,
}
```

1. **GroundItem Identity Rule G1 (B5).** A `GroundItem` is an `Entity` with `kind = EntityKind::GroundItem` (`ENTITY_KIND_GROUND_ITEM = 3` on the wire). It carries its underlying `ItemInstanceId`. It has no separate `GroundItemId` allocator.
2. **Corpse Integration Rule G2 (F11).** Corpses are not tracked in a parallel map. Under V7, an NPC entering death retains its entity presence and AI sidecar (`NpcAi`, `Z/state_ai.rs:236-268`). The dead NPC stores:
   - `spoiler_id: Option<EntityId>` and `spoiler_party: Option<PartyId>` (set during life by Spoil 254);
   - `sweep_items: Vec<SweepItem>` (rolled once at death from `<corpse>` if spoiled);
   - `is_swept: bool` (set when Sweeper 42 resolves);
   - `corpse_until: Tick` (derived under V7 as $deathTick + 10 \times corpseTime + (spoiled ? 100 : 0)$).
   Respawn is scheduled at $\ge \text{corpse\_until.next()}$. Sweeper does NOT despawn the corpse; it remains until `corpse_until`.
3. **Decay Lifecycles.**
   - NPC dropped items decay after `AutoDestroyDroppedItemAfter = 600` s (6,000 ticks) (`GC/general.properties:258; GJ/ItemsAutoDestroy.java:76-85`).
   - Herbs decay after `AutoDestroyHerbTime = 60` s (600 ticks) (`GC/general.properties:262; GJ/ItemsAutoDestroy.java:67`).
   - Player dropped items (including PK death drops) **never auto-destroy** (`DestroyPlayerDroppedItem = False`, `DestroyEquipableItem = False`, `SaveDroppedItem = False`; `GC/general.properties:306, 312, 316`).
   - Per-template overrides: If template specifies `auto_destroy_time`, that value (in seconds $\times 10$) takes precedence (`ItemsAutoDestroy.java:57-62; DP/stats/items/09800-09899.xml:10`).
   - Normal corpses decay after `DefaultCorpseTime = 7` s (70 ticks) (`GC/npc.properties:86`), overridden by template `corpseTime` (234 templates; `L2NpcTemplate.java:139`).
   - Spoiled corpses receive an extension of `SpoiledCorpseExtendTime = 10` s (100 ticks) (`GC/npc.properties:90; DecayTaskManager.java:61-63`), totaling 17 s (170 ticks) default.
4. **Minimal Party Substrate (B9, F4).** Minimal zone-canonical party state:
   - `PartyId(Uuid)` assigned at party creation.
   - Ordered list of members (`Vec<EntityId>`) in join order (determines canonical order for RANDOM and remainder indexing). Max 9 members (`GC/character.properties:600`).
   - Party leader (`EntityId`).
   - Loot distribution mode: 0 = `FINDERS_KEEPERS`, 1 = `RANDOM`, 2 = `RANDOM_INCLUDING_SPOIL`, 3 = `BY_TURN`, 4 = `BY_TURN_INCLUDING_SPOIL` (`PartyDistributionType.java:25-29`).
   - `item_last_loot` cursor (`usize`) for `BY_TURN` round-robin progression (`L2Party.java:174-189`).
   - Distribution range: `PartyRange2 = 1400` L2 units = 43,750 milli-tiles (`GC/character.properties:610`).
   - Level-gap cutoff for XP/SP: Highfive cutoff (0–9 gap 100%, 10–14 30%, 15–99 0%; `GC/character.properties:769-793`).

### 1.2 Phase5Rules & item catalogue admission

`Phase5Rules::try_new` validates all referenced templates, recipes, buylists, and tax rates before computing a canonical SHA-256 digest with domain separator `nightfall.phase5.rules.1\0`. Phase5Rules is content-addressed and hash-referenced from snapshots and replay admissions, never embedded whole into snapshot records (A4, A16; P4Z §1.2; SN4 D27).

**Item Catalogue & Skill Admission Extension (F13, VR5 C5, B11):**
Under the Phase 4 D5/D54 closure rules, the item catalogue is extended to admit:
1. All 68 herb EtcItem templates with `ex_immediate_effect = true` (`L2Item.java:188, 743-745`).
2. All crafting materials, key materials, and ingredient EtcItems referenced by the 1,000 recipes.
3. All 1,000 recipe items (830 Dwarven, 170 Common; template IDs 10609..10849, etc.).
4. All merchant goods referenced across the 274 merchant AI scripts.
5. Admitted skills through the Phase 3 cast pipeline (B11):
   - Spoil 254 (11 ranks, magic level 10..72; `DP/stats/skills/00200-00299.xml:1342-1364`);
   - Sweeper 42 (level 1, target `NPC_BODY`, hitTime 500 ms, reuse 500 ms, MP 3, castRange 20 L2 units = 625 milli-tiles; `DP/stats/skills/00000-00099.xml:1465-1480`);
   - Target type `TargetType::NpcBody` admitted for corpse targeting;
   - All 68 herb item-skill bindings admitted for instant execution on pickup.

Canonical hash inputs, length-delimited and strictly typed:
1. **Drop tables (59 death XMLs):** 14,589 `<group chance>` blocks and 744 ungrouped `<item>` rows with rational chances $num / 10^k$ ($k \le 20$), min/max counts, and group normalization denominators.
2. **Corpse tables (35 corpse XMLs):** 7,335 ungrouped items across 2,260 NPCs with rational chances and 234 per-NPC `corpseTime` overrides.
3. **Herb catalogue:** 68 EtcItem templates with bound skill IDs and durations.
4. **Merchant tables (274 AI scripts):** 20,236 entries with template ID, reference price, and per-entry `baseTax` (10–100%).
5. **Multisell lists (196 XMLs):** Lists with `maintainEnchantment` (31 lists) and `applyTaxes` flags, ingredients, and products.
6. **Recipes (1,000 XML recipes):** 830 Dwarven, 170 Common; Create Item ranks; MP costs; success rates; 582 `productionRare` rare templates with per-recipe rarity (1%, 4%, 20%) and `rareCount`.
7. **Rate multipliers & policies:** Exact rational global rate multiplier $(num, den)$ (default 1/1, P2); champion multipliers (HP $\times 8$, rewards $\times 8$, XP/SP $\times 8$, adena $\times 1$; `GC/customs.properties:10-43; ChampionEnable = False`); deep blue level gap parameters (item 5/10/10%, raid clamp 7; `GC/npc.properties:187-219`); castle tax seed default 0% (`sql/castle.sql:17-26`); storage limits and fees (WH 100/120 @ 30 adena; freight 200 @ 1,000 adena; `GC/character.properties:325-335`); mail fees (100 base + 1,000/slot, max 8 attachments, flood interval 100 ticks = 10 s, 240 inbox/outbox capacity; `RequestSendPost.java:63-68; floodprotector.properties:106`).

### 1.3 Phase5Snapshot, Phase5Player and invariants

```rust
use std::collections::{BTreeMap, BTreeSet};

pub struct Phase5Snapshot {
    pub rules: Arc<Phase5Rules>,
    pub rules_hash: [u8; 32],
    pub ground_items: BTreeMap<EntityId, GroundItem>,
    pub parties: BTreeMap<PartyId, PartyState>,
    pub vendor_restocks: BTreeMap<(u32, u32), VendorRestockState>,
    pub trade_sessions: BTreeMap<u64, TradeSession>,
    pub private_stores: BTreeMap<EntityId, PrivateStoreState>,
    pub players: BTreeMap<EntityId, Phase5Player>,
}

pub struct PartyState {
    pub party_id: PartyId,
    pub leader: EntityId,
    pub members: Vec<EntityId>, // Canonical join order
    pub loot_mode: u8,          // 0..4
    pub item_last_loot: usize,  // BY_TURN cursor
}

pub struct SweepItem {
    pub template_id: u32,
    pub count: u64,
}

pub struct VendorRestockState {
    pub buylist_id: u32,
    pub template_id: u32,
    pub current_count: u32,
    pub max_count: u32,
    pub restock_delay_ticks: u32,
    pub last_restock_tick: Tick,
}

pub struct Phase5Player {
    pub dwarven_recipes: BTreeSet<u32>,
    pub common_recipes: BTreeSet<u32>,
    pub refund_tab: Vec<RefundEntry>, // FIFO, max 12; zone-only, NOT persisted in DB checkpoint
    pub active_trade_session: Option<u64>,
    pub party_id: Option<PartyId>,
}

pub struct RefundEntry {
    pub instance_id: ItemInstanceId,
    pub template_id: u32,
    pub count: u64,
    pub enchant: u32,
    pub augment_id: Option<u32>,
    // Pinned: refund buy-back price is calculated on the fly as (count * referencePrice) / 2
}

pub struct TradeSession {
    pub id: u64,
    pub actor_a: EntityId,
    pub actor_b: EntityId,
    pub items_a: BTreeMap<ItemInstanceId, u64>,
    pub items_b: BTreeMap<ItemInstanceId, u64>,
    pub confirmed_a: bool,
    pub confirmed_b: bool,
    pub start_tick: Tick,
    pub expires_at_tick: Tick, // start_tick + 150 (15 s REQUEST_TIMEOUT)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreType {
    Sell,
    Buy,
    PackageSell,
    Manufacture,
}

pub struct PrivateStoreState {
    pub store_type: StoreType,
    pub title: String,
    pub items: Vec<TradeItem>,
}

pub struct TradeItem {
    pub instance_id: Option<ItemInstanceId>,
    pub template_id: u32,
    pub count: u64,
    pub price: u64,
    pub enchant: u32,
}
```

Invariants:
1. Every `GroundItem` references a valid item template. If stackable, count is positive; if non-stackable, count is exactly 1.
2. Protection owner and party must be valid entities if set. Protection until tick is checked dynamically against current tick.
3. Corpse state exists on dead NPCs with `corpse_until > current_tick`. Sweeping an already-swept corpse is rejected.
4. Player refund tab has length $\le 12$. On the 13th addition, entry 0 is evicted and permanently destroyed (`PcRefund.java:60-64`). Refund tab clears on player logout (`L2PcInstance.java:2539-2543`).
5. Trade sessions link exactly two distinct in-world player entities within 150 L2 units (4,687 milli-tiles). Moving beyond 150 units, disconnect, or partner death cancels the trade session without penalty.
6. Recipe book capacities are bounded by base limits (50 Dwarven / 50 Common) plus effective stats `REC_D_LIM` / `REC_C_LIM` (`GC/character.properties:488-489; L2PcInstance.java:9057-9107`).
7. Private stores exist in zone state only. When a player disconnects, leaves the zone, or the zone restarts, their private store closes immediately (`OfflineTradeEnable = False`, B2).

### 1.4 Snapshot budget against C10

Under C10, the maximum encoded NATS snapshot payload limit is 1,047,552 bytes; the decoded limit is 33,554,432 bytes (`R/record.rs:221-223`). No artificial 1,000 ground-item cap is invented; ground-item buildup is bounded in steady-state by the pinned 600 s decay lifecycle (A18, B12).

Field-encoding bounds for Phase 5 mutable state:
- `GroundItem`: $16 \text{ B (EntityId)} + 8 \text{ B (ItemInstanceId)} + 4 \text{ B (template)} + 8 \text{ B (count)} + 4 \text{ B (enchant)} + 5 \text{ B (augment)} + 8 \text{ B (Vec2Fixed)} + 17 \text{ B (dropped_by)} + 17 \text{ B (owner)} + 8 \text{ B (protection tick)} + 9 \text{ B (decay tick)} + 1 \text{ B (herb)} + 8 \text{ B (created tick)} \approx 113\text{ bytes}$ per ground item.
- `PartyState`: $16 \text{ B (PartyId)} + 16 \text{ B (leader)} + 9 \times 16 \text{ B (members)} + 1 \text{ B (mode)} + 8 \text{ B (cursor)} \approx 185\text{ bytes}$ per party.
- `VendorRestockState`: $4 + 4 + 4 + 4 + 4 + 8 = 28\text{ bytes}$ per restock item.
- `TradeSession`: $8 + 16 + 16 + (8 \times 16 \text{ items}) + 1 + 1 + 8 + 8 \approx 186\text{ bytes}$ per session.
- `PrivateStoreState`: $1 + 32 \text{ (title)} + (5 \times 32 \text{ items}) \approx 193\text{ bytes}$ per store.
- `Phase5Player`: $100 \times 4 \text{ (recipes)} + 12 \times 32 \text{ (refund entries)} + 8 + 17 \approx 809\text{ bytes}$ per player.

Worst-case soak snapshot budget (8 players, 1,000 ground items, 500 corpses, 8 parties, 4 trades, 8 stores):
- Phase 4 player & combat state: $\approx 210{,}048\text{ bytes}$
- Ground items ($1{,}000 \times 113\text{ B}$): $113{,}000\text{ bytes}$
- Dead NPC corpse tracking ($500 \times 48\text{ B}$): $24{,}000\text{ bytes}$
- Parties ($8 \times 185\text{ B}$): $1{,}480\text{ bytes}$
- Vendor restocks ($200 \times 28\text{ B}$): $5{,}600\text{ bytes}$
- Trade sessions & private stores: $\approx 3{,}000\text{ bytes}$
- Player economy state ($8 \times 809\text{ B}$): $6{,}472\text{ bytes}$
- **Total estimated snapshot size:** $\approx 363{,}600\text{ bytes}$ (34.7% of the 1,047,552 byte limit).
Mandatory measurement gate (D4-style) asserts actual serialized snapshot sizes under 8-client soak benchmarks.

### 1.5 Hydration, AOI visibility and presentation boundaries

1. **Ground Items:** Broadcast to players in AOI via `EntitySpawn` with `kind = ENTITY_KIND_GROUND_ITEM = 3` and `GroundItemDetail` (tag 30). Carries entity ID, template ID, count, enchant level, and `is_protected` boolean. The `is_protected` bit is **observer-dependent**: it is false to the protection owner and their party, and true to non-owners (F30). Despawn is broadcast via `EntityDespawn`.
2. **Private Stores:** Spawns/updates emit `WorldEvent::PrivateStoreChanged` (tag 37) to AOI observers. Observers see player entity ID, store type, title, and public prices.
3. **Trade Sessions:** Mutual updates between trading partners only via `WorldEvent::TradeSessionChanged` (tag 36).
4. **Recipe Book:** Owner-only. Hydrated via `WorldEvent::RecipeBookSync` (tag 38). Craft results arrive via `WorldEvent::CraftResult` (tag 41).
5. **Vendor Buylists & Multisell:** Opened on-demand via proximity intent. `ListBuylist` RPC response is canonical for buylists (no duplicate window event; F30). Multisell dialog opens via `WorldEvent::MultisellWindow` (tag 40).

---

## 2. V7 in-tick ordering and RNG ledger

### 2.1 Schedule amendment

Extend the tick schedule driver (`Z/state_runtime.rs`, driver `run_tick_v5` at `:720`) with a V7 execution branch. V5 and V6 legacy replay paths remain byte-exact. Note: Final Phase 4 integration SHA may move line numbers.

| Phase | V7 execution rules (preserving P4Z §2.1 strict command ordinal order) |
|---|---|
| **0 — Receive-order commands** | Execute ALL Phase 4 and Phase 5 intents in strict receive (command-ordinal) order with per-intent state rebuild and validation (F23). Validates: <br>- `PickUpItem`: range $\le 36$ L2 units (1,125 milli-tiles), protection check, `can_accept` capacity. <br>- `VendorBuy`/`VendorSell`/`VendorRefund`: proximity $\le 150$ L2 units (4,687 milli-tiles), baseTax + castleTax, refund tab FIFO. <br>- `Multisell`: proximity $\le 150$ L2 units, ingredient consumption, product grant, `maintainEnchantment`. <br>- `CraftItem`/`RegisterRecipe`: recipe knowledge, Create Item rank, MP deduction, materials consumption, success & rare rolls. <br>- `TradeRequest`/`TradeAnswer`/`TradeAddItem`/`TradeRemoveItem`/`TradeConfirm`/`TradeCancel`: distance $\le 150$ L2 units, atomic two-phase commit on mutual confirm. <br>- `SetPrivateStore`/`BuyPrivateStore`/`ClosePrivateStore`: slot limits, escrow. <br>- `WarehouseDeposit`/`WarehouseWithdraw`/`FreightDeposit`/`FreightWithdraw`: fee deduction, container rows in same fenced transaction. <br>- `PartyInvite`/`PartyAnswer`/`PartyLeave`/`PartyKick`/`SetLootMode`: minimal party substrate updates. |
| **1 — Spawn** | Hydrate newly spawned entities (players, NPCs, GroundItems) into AOI grid. |
| **2 — Expiry** | Expire timed effects, item shared-reuse deadlines, and economy timers: <br>1. GroundItem decay: despawn items reaching `decay_tick`. <br>2. GroundItem protection: clear `protection_until_tick` when expired. <br>3. Dead NPC corpse decay: despawn corpses reaching `corpse_until`. <br>4. Trade session request timeouts: cancel sessions reaching `expires_at_tick` (15 s). |
| **3 — Periodic + regen** | Preserve V6 regen and periodic tick processing. |
| **4 — Chase / movement** | Update entity positions (`Vec2Fixed`) and facing vectors. |
| **5 — Cast launches / impacts** | Resolve spell impacts: <br>1. **Spoil (254)**: evaluate magic success (`skill_math::magic_success_rate`) BEFORE checking `ALREADY_SPOILED` (`Skill.java:935; Spoil.java:40-42, 57-61`). On success, set monster `is_spoiled = true` and record spoiler attribution. <br>2. **Sweeper (42)**: validate spoiler ownership and capacity; transfer `sweep_items` to sweeper/party; mark corpse `is_swept = true`. Corpse remains until `corpse_until`. |
| **6 — Auto impacts** | Resolve melee and projectile hits, landed damage, shot discharge. |
| **7 — Hit bookkeeping, deaths & drop resolution** | Unified landed-damage facts. When an entity dies: <br>1. Award kill credit and evaluate `maxDealer` (top-damage online player within `PartyRange 1600` L2 units = 50,000 milli-tiles). <br>2. If living NPC was spoiled: set `corpse_until = deathTick + 10 * corpseTime + 100` (17 s default); roll `<corpse>` list into `sweep_items`. <br>3. Resolve NPC drops: calculate level gap (5/10/10% item window), roll document-order death list, draw instance ground positions ($\pm 70$ L2 units), draw `npc_respawn_jitter`, spawn GroundItems with 15 s DropProtection. <br>4. If chaotic player (karma > 0, PK $\ge 6$) outside PvP zone: execute single-loop unequip-then-roll PK drops, draw ground positions ($\pm 25$ L2 units), spawn GroundItems with 15 s protection. |
| **8 — New starts** | Auto-attacks and skill casts. Preflight costs, auto-shot charges. |
| **9 — AI** | Monster hate and AI behavior. |
| **10 — Durable flush / observe** | Batch ledger rows (paired `transfer_out`/`transfer_in`), persist checkpoint + outbox, emit WorldEvents only after log acknowledgement and fenced DB commit. |

### 2.2 Normative random caller sequence (F1, F2, F3, F7)

Every RNG draw consumes the per-zone `ChaCha12` stream in deterministic in-tick order:

```rust
// Exact rational draw comparison:
// Chance is represented as num / den where den = 10^k (k <= 20).
// Draw uniform 64-bit integer r from ChaCha12 stream:
let r: u64 = rng.next_u64(); // (next_u32() << 32) | next_u32()
let passes: bool = (r as u128) * (den as u128) < (num as u128) * (1_u128 << 64);
```

#### Normative per-death draw sequence (F2):
1. **Kill credit:** Evaluate `maxDealer` (deterministic; zero RNG draws).
2. **Corpse list (Spoil items):** If and only if the NPC is spoiled (`is_spoiled == true`), iterate `<corpse>` items in document order:
   - For each item:
     - Effective chance $C_{\text{eff}} = \text{base\_chance} \times \text{rate\_mult} \times \text{level\_gap}$.
     - Draw 64-bit word $r$ for chance test. If $C_{\text{eff}} \ge 100\%$, amount multiplier $M = \lfloor C_{\text{eff}} / 100 \rfloor$, and if remainder $> 0$, draw $r$ for $+1$ multiplier.
     - Quantity draw: `next_u32() % (max - min + 1)`.
     - Append to NPC's `sweep_items`.
3. **Death drop list (Document Order):** Process the single document-order list of `<group>` and `<item>` nodes (`L2NpcTemplate.calculateDrops` `:437-458`):
   - For a `<group chance="...">` node:
     - Effective group chance $C_{\text{eff}} = \text{group.chance} \times \text{rate\_mult} \times \text{level\_gap} \times C$.
     - Draw $r$ for group chance test. If fail, skip group.
     - If multi-item group: draw $r$ across normalized rational shares $\sum \text{share}_i = 100\%$ to select exactly one item (`IGroupedItemDropCalculationStrategy.java:62-78; GroupedGeneralDropItem.java:193-215`).
     - Precise remainder roll: if $C_{\text{eff}} \ge 100\%$ and $(C_{\text{eff}} \bmod 100) > 0$, draw $r$ for $+1$ amount multiplier.
     - Quantity draw: `next_u32() % (max - min + 1)`.
   - For an ungrouped `<item chance="...">` node:
     - Effective item chance $C_{\text{eff}} = \text{item.chance} \times \text{rate\_mult} \times \text{level\_gap} \times C$.
     - Draw $r$ for chance test. If $C_{\text{eff}} \ge 100\%$, remainder draw if remainder $> 0$.
     - Quantity draw: `next_u32() % (max - min + 1)`.
4. **Ground position draws (F2):** For each created GroundItem instance in document order (under `MultipleItemDrop = True`, non-stackable quantity $N$ produces $N$ instances; `L2Npc.java:1460-1495`):
   - Draw `posX_offset = (next_u32() % 141) - 70` ($\pm 70$ L2 units = $\pm 2{,}187$ milli-tiles).
   - Draw `posY_offset = (next_u32() % 141) - 70` ($\pm 70$ L2 units = $\pm 2{,}187$ milli-tiles).
5. **Respawn jitter draw (F2):** If NPC slot has `respawn_random_secs > 0`, draw `npc_respawn_jitter = next_u32() % (respawn_random_secs + 1)` (`Z/state_ai.rs:258-264`).

#### Normative chaotic PK death drop sequence (F3):
If victim has `karma > 0`, `pkKills >= 6`, and dies outside a PvP zone (`!isInsideZone(ZoneId.PVP)`):
1. **Base drop check:** Draw `next_u32() % 100 < 40` (`KarmaRateDrop = 40%`). If fail, zero items drop.
2. **Single-loop inventory pass:** Iterate all items in victim inventory in canonical `ItemInstanceId` ascending order (`L2PcInstance.java:4515-4550`):
   - If item is equipped:
     - **Unequip item immediately into inventory BEFORE its roll.**
     - If weapon (`type2 == WEAPON`): draw `next_u32() % 100 < 10` (`KarmaRateDropEquipWeapon = 10%`).
     - If other equipment: draw `next_u32() % 100 < 40` (`KarmaRateDropEquip = 40%`).
   - If item is not equipped (inventory item):
     - Draw `next_u32() % 100 < 50` (`KarmaRateDropItem = 50%`).
   - If item drop roll succeeds:
     - Draw `posX_offset = (next_u32() % 50) - 25` ($\pm 25$ L2 units = $\pm 781$ milli-tiles; `L2PcInstance.java:3178`).
     - Draw `posY_offset = (next_u32() % 50) - 25` ($\pm 25$ L2 units = $\pm 781$ milli-tiles).
     - Spawn GroundItem with whole stack (`dropItem("DieDrop", ...)`).
     - Increment `dropped_count`. If `dropped_count == 10`, **break loop immediately** (remaining equipped items stay equipped).

#### Normative crafting sequence (F7, F27):
1. Pay MP cost (`calculateStatUse(false, true); RecipeController.java:347`).
2. Transfer manufacture fee if workshop (`:351-358`).
3. Consume materials from inventory (`:363`).
4. Draw `success_roll = next_u32() % 100`. Success if `success_roll < recipe.successRate` (`:367`). If fail, 100% loss (zero further draws).
5. If success AND recipe has `productionRare` AND `CraftMasterwork = True`:
   - Draw `rare_roll = next_u32() % 100`.
   - Success if `rare_roll < recipe.rarity` (1%, 4%, or 20% against 100; `:570`).
   - If success, grant `productionRare` with `rareCount`; else standard product with `count`.

#### Party adena split and random distribution:
- **Adena remainder:** For each 1 adena of remainder $R = \text{total} \bmod K$: draw `next_u32() % K` to award to `members[residue]` (using canonical join order).
- **Random item:** Draw `next_u32() % K` to select recipient from in-range members.

---

## 3. Drops and loot generation

### 3.1 Exact rational drop calculation algorithm (F1)

Pinned NPC XML drop chances carry up to 20 decimal places (minimum `0.0003000000142492354`%; maximum `188.85499572753906`%). Truncating or scaling to fixed integer basis (like 1,000 or 100,000) produces zero for small chances. Phase 5 evaluates all drop chances as exact rational numbers $Q = \frac{\text{num}}{\text{den}}$ using checked `u128` arithmetic (A1, F1).

Let $c_{\text{base}} = \frac{N_c}{D_c}$ be the parsed XML chance ($D_c = 10^k$, $k \le 20$), global rate multiplier $M = \frac{M_{\text{num}}}{M_{\text{den}}}$, level gap modifier $L = \frac{L_{\text{num}}}{L_{\text{den}}}$, and champion multiplier $C \in \{1, 8\}$.
The effective percentage chance is:
$$C_{\text{eff}} = \frac{N_c \times M_{\text{num}} \times L_{\text{num}} \times C}{D_c \times M_{\text{den}} \times L_{\text{den}}} = \frac{\text{Num}}{\text{Den}}$$

1. **Guaranteed Amount Multiplier:**
   - If $\text{Num} \ge 100 \times \text{Den}$ (Effective Chance $\ge 100\%$):
     - $M_{\text{base}} = \lfloor \text{Num} / (100 \times \text{Den}) \rfloor$.
     - $\text{Num}_{\text{rem}} = \text{Num} \bmod (100 \times \text{Den})$.
     - If $\text{Num}_{\text{rem}} > 0$: draw 64-bit integer $r \in [0, 2^{64})$.
       Test: $(r \text{ as u128}) \times (100 \times \text{Den}) < (\text{Num}_{\text{rem}} \text{ as u128}) \times 2^{64}$.
       If true, $M_{\text{total}} = M_{\text{base}} + 1$; else $M_{\text{total}} = M_{\text{base}}$.
     - If $\text{Num}_{\text{rem}} == 0$: $M_{\text{total}} = M_{\text{base}}$.
     - Item drops with quantity: $Q = (\text{min} + (\text{next\_u32}() \bmod (\text{max} - \text{min} + 1))) \times M_{\text{total}}$.
2. **Sub-100% Chance Roll:**
   - If $\text{Num} < 100 \times \text{Den}$ (Effective Chance $< 100\%$):
     - Draw 64-bit integer $r \in [0, 2^{64})$.
     - Test: $(r \text{ as u128}) \times (100 \times \text{Den}) < (\text{Num} \text{ as u128}) \times 2^{64}$.
     - If true: item drops with quantity $Q = \text{min} + (\text{next\_u32}() \bmod (\text{max} - \text{min} + 1))$.
     - If false: item does NOT drop.

### 3.2 Single document-order drop list pass (F2)

1. The NPC death drop list is processed in a **single document-order pass** mixing `<group>` and `<item>` nodes (`NpcData.java:453-470; L2NpcTemplate.java:437-458`).
2. Multi-item group selection:
   - Evaluated only when the group chance roll passes.
   - Pinned shares are normalized to exact rational fractions summing to 100% (`GroupedGeneralDropItem.normalizeMe :193-215`).
   - Draw 64-bit uniform integer $u \in [0, 2^{64})$ and walk cumulative normalized shares: select the first item where $\sum \text{share}_i > u$.
   - Selected item rolls precise remainder and quantity.

### 3.3 Level-gap modifier windows (Deep Blue)

Let killer level be $K = \text{maxDealer.level}$, mob level be $M_{\text{lvl}}$, and level difference be $d = M_{\text{lvl}} - K$ (`L2Attackable.java:341-379; GC/npc.properties:187-219`).

1. **Grouped Items and Adena (Item Window 5/10/10%):**
   - Parameters: `DropItemMinLevelDifference = 5`, `DropItemMaxLevelDifference = 10`, `DropItemMinLevelGapChance = 10` (10% floor).
   - If $d \ge -5$: no penalty, $L = 1/1$ (100%).
   - If $d \le -10$: floor applied, $L = 10/100$ (10%).
   - If $-10 < d < -5$: linear interpolation: $L_{\text{pct}} = 10 + (d + 10) \times 18$.
     Yields exact integer percentages: $d = -5 \to 100\%$, $-6 \to 82\%$, $-7 \to 64\%$, $-8 \to 46\%$, $-9 \to 28\%$, $-10 \to 10\%$.
   - All 2,309 adena entries in pinned NPC XML are grouped; thus adena uses this 5/10/10% item window (VR5 A29).
2. **Raid Bosses:**
   - Grouped raid drops: $\text{clamp}\left(\frac{d \times 15}{100} + 1, 0, 1\right)$ (`IKillerChanceModifierStrategy.java:37`). If $d \le -7$, modifier is 0.
   - Ungrouped raid items: use the level-gap WINDOW, not the clamp (`IKillerChanceModifierStrategy.java:44-53; F32`).

### 3.4 Rate multiplier precedence chain & global multiplier (F32, B14)

1. **Precedence Chain:** Pinned multiplier precedence is:
   `per-item-id override (57, 1)` > `herb` > `raid` > `Death/Corpse` (`IChanceMultiplierStrategy.java:49-66; IAmountMultiplierStrategy.java:38-56`).
2. **Global Rate Multiplier (B14):**
   - Applied exactly where pinned rates enter the algorithm, as an exact rational factor on chance ($M_{\text{num}} / M_{\text{den}}$).
   - Surplus chance above 100% multiplies dropped amounts via the pinned precise-drop loop. Amounts are not separately multiplied.
   - Pinned defaults are 1x.
3. **Champions:** `ChampionEnable = False` (`customs.properties:10`), so champion multiplier $C = 1$ default.

### 3.5 Killer determination and `maxDealer` (F20)

1. Track all damage contributors to the NPC during its life incarnation.
2. Only contributors with `damage > 1` within `PartyRange 1600` L2 units (50,000 milli-tiles) count (`L2Attackable.java:341-375`).
3. Summon damage aggregates to the master player.
4. Select the player with maximum total damage.
   - Deterministic tie-break: earliest tick damage reached maximum, then lowest `EntityId`.
5. Online check: evaluated AFTER selection. If selected `maxDealer` is offline, fallback is `lastAttacker`, NOT the next-best dealer (`:375-379`).

### 3.6 Chaotic PK drops (F3)

- Trigger: victim has `karma > 0`, `pkKills >= 6`, and dies outside a PvP zone (`!victim.isInsideZone(ZoneId.PVP)`). Clan war non-karma death drops are vacuous until Phase 7.
- Evaluated as a single pass over inventory in canonical `ItemInstanceId` ascending order.
- Unequip-then-roll: each equipped item is unequipped immediately before its own roll (`L2PcInstance.java:4531-4541`).
- Stop at 10 items. Whole stack moves on drop. Dropped items spawn with 15 s protection owned by victim and party. Never auto-destroyed (`DestroyPlayerDroppedItem = False`, `DestroyEquipableItem = False`).

### 3.7 Two distinct drop protection mechanisms & decay (F21, E7, F26)

1. **Loot Owner Protection (`ItemTable.java:203-216`):**
   - Normal monster loot: 15,000 ms (150 ticks) protection to `maxDealer` and party.
   - Raid loot rights: requires that the first attacker belonged to a command channel with $\ge 45$ members (`character.properties:693; L2Attackable.java:234-238`). Protected for 900 s (9,000 ticks) to CC leader and CC party.
   - Protection check (`isInLooterParty`): evaluated **dynamically at pickup**, checking the looter's current party/CC membership while the looter is online (`L2PcInstance.java:3824-3842, 6650-6663`).
2. **DropProtection (`L2Npc.java:1476-1478; DropProtection.java:35, 52-63`):**
   - Applied to **EVERY NPC drop**, including raid drops (E7).
   - Killer and killer's current party receive 15 s DropProtection.
   - Raid drops therefore keep 15 s DropProtection even if no CC $\ge 45$ existed.
3. **Decay Lifecycles (F26):**
   - NPC items: 600 s (6,000 ticks) (`GC/general.properties:258`).
   - Herbs: 60 s (600 ticks) (`:262`).
   - Per-template override: `auto_destroy_time` in XML takes precedence (`ItemsAutoDestroy.java:57-62`).
   - Player-dropped items (incl. PK drops): never auto-destroyed (`DestroyPlayerDroppedItem = False`, `DestroyEquipableItem = False`, `SaveDroppedItem = False; :306, 312, 316`).
   - `ProtectedItems` list is empty (`general.properties:269`).
   - Corpse decay: 7 s default (`npc.properties:86`) + 10 s spoil extension (`:90`) = 17 s (234 template overrides).

---

## 4. Pickup, party substrate and loot distribution

### 4.1 Pickup intent validation and capacity checks (F21, F25)

When a player sends `PickUpItemRequest`:
1. **Range check:** Player must be within 36 L2 units (1,125 milli-tiles) of the GroundItem (`L2PlayerAI.java:260`).
2. **Ownership check:** If `tick < protection_until_tick`, player must be the protection owner or currently in the protection owner's party (`L2PcInstance.java:3824`).
3. **Capacity check:** Evaluated ONLY when solo or under `FINDERS_KEEPERS` (`L2PcInstance.java:3810-3814`).
4. **Action:** Despawn GroundItem, execute paired `transfer_out`/`transfer_in` ledger rows, emit `EntityDespawn`.

### 4.2 Herbs and immediate-effect items

- The 68 EtcItems with `ex_immediate_effect = "true"` (`L2Item.java:188, 743-745`).
- Exempt from inventory capacity (`PcInventory.validateCapacity :746-752`).
- On pickup, skill effect applies immediately to picker via cast pipeline; item is destroyed without allocating inventory slot or `ItemInstanceId` (`L2PcInstance.java:3860-3868`).

### 4.3 Minimal party substrate (B9, F4)

- Party creation, invite, accept, leave, kick, change leader. Max 9 members.
- Five loot distribution modes (`PartyDistributionType.java:25-29; L2Party.java:197-220`):
  - 0: `FINDERS_KEEPERS`: item to picker; sweep to sweeper.
  - 1: `RANDOM`: item to random in-range member; sweep to sweeper.
  - 2: `RANDOM_INCLUDING_SPOIL`: item to random member; sweep items distributed randomly.
  - 3: `BY_TURN`: round-robin using `item_last_loot` cursor; sweep to sweeper.
  - 4: `BY_TURN_INCLUDING_SPOIL`: round-robin using `item_last_loot` cursor; sweep items distributed by turn.
- Range check: distance from picker (or NPC for sweep) $\le 1400$ L2 units (43,750 milli-tiles; `L2Party.java:162, 511, 545`).
- Note: `validateCapacityByItemId` always returns true in retail L2J (`PcInventory.java:759-765; VR5 A61`), so member eligibility is strictly distance.

### 4.4 Party adena split and remainder distribution (F22)

- Governed by `PartyEvenlyDistributeItems = 57` (`GC/character.properties:616-631`).
- **Applies ONLY when `count > 1` (`L2Party.java:505`).** If `count == 1`, adena follows the active item loot mode!
- Filter members within `PartyRange2 = 1400` L2 units (43,750 milli-tiles) of picker.
- Base share: $\lfloor \text{total} / K \rfloor$. Remainder: $R = \text{total} \bmod K$.
- Each member receives base share.
- For remainder $1..R$: select member via `next_u32() % K` using canonical join order and grant $+1$ adena (`L2Party.java:613`).
- Cap: capped at `MaxAdena = 99,900,000,000`.

---

## 5. Spoil and sweep

### 5.1 Spoil skill 254 resolution (F33, F35)

1. Properties: 11 levels (magic level 10–72), MP 12–67, reuse 3,000 ms, hitTime 1,800 ms (`DP/stats/skills/00200-00299.xml:1342-1364`). Hate `effectPoint` is -138 to -646 (`:1345`).
2. Execution:
   - Target must be alive attackable NPC.
   - Magic success evaluated via `skill_math::magic_success_rate` (`Z/skill_math.rs:710`) based on skill `magicLvl` vs NPC level.
   - **RNG draw order:** Magic success draw occurs BEFORE checking if monster is already spoiled (`Skill.java:935; Spoil.java:40-42, 57-61`). If already spoiled, cast terminates with `ALREADY_SPOILED`.
   - On success: monster flagged `is_spoiled = true`, spoiler entity ID and party ID recorded.

### 5.2 Sweeper skill 42 and corpse sweep resolution (F11, F35)

1. Properties: Level 1, hitTime 500 ms, reuse 500 ms, MP 3, castRange 20 L2 units (625 milli-tiles), target `NPC_BODY` (`DP/stats/skills/00000-00099.xml:1465-1480`).
2. Validation:
   - Target must be dead, spoiled, and not yet swept (`is_spoiled == true && is_swept == false`).
   - Must satisfy `isOldCorpse`: corpse time remaining $\ge \text{CorpseConsumeSkillAllowedTimeBeforeDecay} = 2{,}000$ ms (`GC/npc.properties:94`).
   - Caster must be spoiler OR in spoiler's current party (`L2Attackable.java:1027-1035`).
3. Execution:
   - Transfer items from NPC's `sweep_items`.
   - If party loot mode is 2 or 4 (`INCLUDING_SPOIL`), route items through party distribution; otherwise grant directly to sweeper (`L2Party.java:496-546; Sweeper.java:60-66`).
   - Paired ledger row `sweep` (`transfer_out` from corpse, `transfer_in` to looter).
   - Corpse marked `is_swept = true`. **Corpse is NOT despawned**; it remains until `corpse_until` (`Sweeper.java:53-67`).

### 5.3 Corpse decay lifecycle (F11)

- Managed by `NpcAi.corpse_until` (`Z/state_ai.rs:236-268`).
- Death calculates: `corpse_until = deathTick + 10 * corpseTime + (spoiled ? 100 : 0)`.
- Default `corpseTime` is 7 s (70 ticks); 234 NPC templates override it (`L2NpcTemplate.java:139`). Spoiled extension is 10 s (100 ticks).
- Slot respawn scheduled at $\max(\text{respawn\_delay}, \text{corpse\_until.next()})$.

---

## 6. NPC vendors and buylists

### 6.1 Buylist data and NPC proximity (F8, F25)

1. Merchant entries generated from the 274 Java AI scripts (20,236 entries) with per-entry `baseTax` (10–100%; `DPJ/ai/npc/**`). Pinned buylist XMLs carry price 0 and no restock data (dormant).
2. Proximity: player must be within 150 L2 units (4,687 milli-tiles) of merchant NPC (`L2Npc.java:114; RequestBuyItem.java:100; RequestSellItem.java:104`). `buyList.isNpcAllowed` checked (`:120-124`).

### 6.2 Buy price calculation, tax rounding and castle treasury (F8, F9, E2, E3, E9)

1. **Price Formula:**
   $$\text{unit\_price} = \lfloor \text{refPrice} \times (1.0 + \text{baseTaxRate} + \text{castleTaxRate}) \rfloor$$
   $$\text{total\_price} = \text{unit\_price} \times \text{count}$$
   `refPrice` is the item template's reference price (`item.getReferencePrice()`).
   `castleTaxRate` default is **0%** (pinned seed, `sql/castle.sql:17-26`). 15% is the no-owner ceiling.
   Double-truncation rounding: exact integer arithmetic emulates Java double truncation. Proven by oracle across all pairs (e.g. $100 \times 1.15 \to 114$).
2. **Castle Treasury Skim:** $\lfloor \text{subTotal} \times \text{castleTaxRate} \rfloor$ recorded as ledgered sink (`RequestBuyItem.java:234`).
3. **Special buylists:** Siege guard buylists 3960–4026 apply `RateSiegeGuardsPrice` (`:156-158`).

### 6.3 Sell price calculation and adena minting

1. Formula: $\text{sell\_price} = \lfloor \text{item.getReferencePrice}() / 2 \rfloor \times \text{count}$ (`RequestSellItem.java:142`). No tax markup.
2. Selling mints adena via `addAdena("Sell", ...)` (`:142, 155; VR5 A2`).
3. Item must be `is_sellable = true`. Cap check: adena $\le 99{,}900{,}000{,}000$.

### 6.4 Refund tab 12-item FIFO buffer (F17, B6)

1. Capped at 12 entries (`GC/general.properties:458; PcRefund.java:60-64`). Stackables merge into existing entries (`transferItem`).
2. Overflow: 13th entry evicts and destroys the oldest entry (index 0) with a `destroy` ledger row (`ItemTable.destroyItem("ClearRefund")`).
3. Buy-back price: $\lfloor (\text{count} \times \text{referencePrice}) / 2 \rfloor$ (`RequestRefundItem.java:155`). Note: differs from sell payout for odd price with count $\ge 2$!
4. Session lifetime: zone-only state. Cleared on logout/departure with `destroy` ledger rows (`L2PcInstance.java:2539-2543, 8678`). NOT persisted in DB checkpoint (B6).

---

## 7. Multisell and exchange engine

### 7.1 Generic exchange engine

Phase 5 generalizes the D37/D52 exchange engine across all 196 multisell lists (`DP/multisell/*.xml`). Emits a single `ItemExchanged` outbox fact (`nightfall.item.exchanged`).

### 7.2 maintainEnchantment semantics (F19, B10)

`maintainEnchantment` is a list-level attribute (31 lists; `multisell/1003.xml:2; MultiSellChoose.java:352`):
1. **Enchant Level:** Copied exactly from base ingredient to product.
2. **Augmentation:** The $i$-th consumed augment is copied to the $i$-th product (`augment_id = (stat34 << 16) | stat12; MultiSellChoose.java:355-362`).
3. **Elemental Attributes:** No-op per B10 with pinned citation recorded (attributes are Phase 3 A13 None / Phase 4 D5).

### 7.3 Tax application on multisell ingredients (F19)

When a multisell list has `applyTaxes = true`:
- Castle tax applies ONLY to adena ingredients flagged `isTaxIngredient = true` (`PreparedEntry.java:44-71`).
- Tax charged: $\text{tax\_charge} = \text{round}(\text{count} \times \text{taxRate})$ using round-half-up (`Math.round`).
- Non-tax adena ingredients are summed separately without tax markup.

---

## 8. Crafting and recipes

### 8.1 Recipe book registration and limits (F27, F36)

1. Capacities: base 50 Dwarven / 50 Common (`GC/character.properties:488-489`), raised by effective stats `REC_D_LIM` / `REC_C_LIM` (`L2PcInstance.java:9057-9107`).
2. Registration consumes recipe item permanently. Create Item rank must meet or exceed recipe level. Duplicate rejected with `REJECT_REASON_RECIPE_ALREADY_KNOWN`.
3. Dual projection: persisted in `character_recipes` table and `Phase5Checkpoint`. Committed in same transaction; mismatch is corruption (F36).

### 8.2 Create Item and Create Common Item skills

1. **Create Item (172):** 10 ranks learned at levels 5, 20, 28, 36, 43, 49, 55, 62, 70, 82 (`DP/stats/skills/00100-00199.xml:1435-1441`).
2. **Create Common Item (1320):** 9 ranks learned at levels 5..70 (`DP/stats/skills/01300-01399.xml:547`).

### 8.3 Craft resolution pinned order (F27)

In `finishCrafting` (`RecipeController.java:345-367`):
1. Deduct MP via `calculateStatUse(false, true)` (`:347, 467-498`). If MP insufficient, reject before consumption.
2. If private workshop, transfer manufacture fee from customer to Dwarf (`:351-358`). Fee is non-refundable.
3. Consume materials from inventory (`:363`). If materials missing at resolution, abort (MP and fee are spent).
4. Success roll: `next_u32() % 100 < recipe.successRate` (`:367`).
5. Failure loss: 100% material loss with zero refund (`:380-387`). No product granted.

### 8.4 Masterwork rare item generation (F7, E1, E11)

Pinned `CraftMasterwork = True` (`GC/character.properties:484`):
1. Exactly **582 recipes** carry `productionRare` (`recipes.xml`).
2. If base craft succeeds and recipe has `productionRare`:
   - Draw `rare_roll = next_u32() % 100`.
   - Success if `rare_roll < recipe.rarity` (`RecipeController.java:569-574`).
   - Rarity is 1% (31 recipes), 4% (315 recipes), or 20% (236 recipes).
   - If success, grant `productionRare` with `rareCount`; else standard product with `count`.

### 8.5 Private manufacture workshops (F12)

1. Dwarf opens workshop via `SetPrivateManufactureRequest` setting recipes and adena fees.
2. Customer sends `BuyPrivateManufactureRequest`. Fee transferred before roll. Evaluated on Dwarf's skill rank and per-zone RNG.

---

## 9. Player trade and private stores

### 9.1 Two-phase atomic trade state machine (F16, E10, B3, B7)

Protocol (`TradeList.java:330-450`):
1. **Initiation:** Player A sends `TradeRequest`, Player B answers. Request expires after 15 s (`REQUEST_TIMEOUT = 15; L2PcInstance.java:354, 5038-5048`).
2. **Distance guard:** Both players must be within 150 L2 units (4,687 milli-tiles) at request (`TradeRequest.java:168`) and at confirm (`TradeDone.java:89`).
3. **Offer updates:** Adding or removing items invalidates both confirmations (`AddTradeItem`, `TradeList.java:340-399`). No separate `TradeLock` step.
4. **Execution:** When both players confirm (`TradeDone 1`), zone validates trade lists and executes atomic critical mutation moving items between inventories.
5. **Abort (B7):** Disconnect, departure, death, or moving $> 150$ L2 units cancels session for both parties. Items remain with owners. No penalty. Pinned cancel notification emitted.
6. **Rate limit:** `TransactionInterval = 10` ticks (1 s) (`GC/floodprotector.properties:85`).

### 9.2 Critical mutation execution & paired ledger rows (F5, B13)

When both confirm:
1. Validate item existence, unequipped state, tradeability, and slot/weight capacity (`can_accept`) on both sides.
2. Execute atomic critical mutation:
   - For Player A's offered items: `transfer_out` under Player A, `transfer_in` under Player B.
   - For Player B's offered items: `transfer_out` under Player B, `transfer_in` under Player A.
   - All rows preserve `ItemInstanceId` and reference shared Trade receipt ID.

### 9.3 Private stores and closing rules (F12, B2)

1. Types: Sell, Buy, Package Sell, Manufacture.
2. Slot limits: Dwarf 4 Sell / 5 Buy; Others 3 Sell / 4 Buy (`GC/character.properties:306-313`), modified by effective stats `P_SELL_LIM` / `P_BUY_LIM`.
3. **Zone-state only (B2):** Private stores close immediately on disconnect, departure, or server restart (`OfflineTradeEnable = False`). No DB `private_stores` table.

---

## 10. Storage, mail, auction and manor movement contracts

### 10.1 DB-only containers with zone-mediated movement & replay independence (F6, E12)

Storage containers (warehouse, freight, mailboxes, auctions, manor) are DB-only. All inventory mutations execute through zone critical mutations.
**Replay Independence Rule (E12):** Every RPC-routed mutation freezes its DB-derived inputs into the replay record (`PbCommandKind` entries). Replay never queries live DB state. Zone state uses ticks and deterministic IDs; no `NOW()` or `BIGSERIAL` enter zone state. Container row changes commit in the SAME revision-fenced transaction as checkpoint, ledger, and outbox.

### 10.2 Private warehouse and freight contracts (F18, E8)

1. **Private Warehouse:**
   - Capacity: 100 slots (120 Dwarf) + `WH_LIM` stat (`GC/character.properties:325-326`).
   - Deposit fee: **30 adena per item entry** (`SendWareHouseDepositList.java:116-117`).
   - Gates: `isDepositable`, `KarmaPlayerCanUseWareHouse` (`:111-114`). Non-tradeable augmented items rejected (`L2ItemInstance.java:685-686`). Retains `ItemInstanceId`.
2. **Freight (E8):**
   - Transferred to alt character on SAME ACCOUNT (`getAccountChars`, `RequestPackageSend.java:79-113`). No destination town.
   - Capacity: 200 slots + `FREIGHT_LIM` stat (`PcFreight.java:66-68`). Fee: **1,000 adena per entry** (`GC/character.properties:335; RequestPackageSend.java:109`).
   - Proximity: within 150 L2 units of folk NPC. `KarmaPlayerCanUseWareHouse` enforced.

### 10.3 Mail system, attachments and COD (F12)

1. **Constraints (`RequestSendPost.java`):**
   - Sender must be in a **peace zone** (`:130, 208`).
   - Items must be unequipped and tradeable (`:250`). Max 8 attachments (`:63`).
   - Fee: 100 adena base + 1,000 adena per attachment slot (`:67-68, 255`).
   - Mailbox limit: 240 inbox / 240 outbox (`:63-64, 209-216`).
   - Flood interval: `SendMailInterval = 100` ticks (10 s) (`GC/floodprotector.properties:106`).
2. **Lifecycles (`Message.java:39-40, 98`):**
   - Regular expiry: 360 hours (15 days) returns uncollected attachments to sender's mailbox.
   - COD expiry: 12 hours returns mail to sender. Receiver pays COD adena to take attachments.
   - API scheduler with deterministic zone admission.

### 10.4 Item auction state machine (F12, B4)

- Driven by fixture schedule (`ItemAuctionManager.java; GC/general.properties:655`).
- Bidding: zone critical mutation debits adena and refunds previous highest bidder.
- Finalization: awards item to highest bidder via item path.

### 10.5 Manor mechanics (F12, B1)

- Seeds and crops (`DP/seeds.xml; GC/general.properties:582-602; CastleManorManager.java`).
- Sowing: `SowSeedRequest` consumes seed and marks monster seeded. On death, crop drop replaces normal mob drop.
- Crop turn-in: player exchanges crops at Manor Manager NPC via `ExchangeAudit`.

---

## 11. Persistence, migrations and ledger audit

### 11.1 Migrations from 000013 and table schemas (F28, B2, B8)

Phase 5 schema migrations start at `m20261013_000013_economy.rs`:

```sql
-- 000013: Phase 5 storage, mail, recipes, and economy counters

CREATE TABLE character_warehouses (
    character_id UUID PRIMARY KEY REFERENCES characters(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE warehouse_items (
    id BIGINT PRIMARY KEY, -- ItemInstanceId
    character_id UUID NOT NULL REFERENCES characters(id),
    template_id INTEGER NOT NULL,
    count BIGINT NOT NULL CHECK (count > 0),
    enchant INTEGER NOT NULL DEFAULT 0,
    augment_id BIGINT,
    charges SMALLINT NOT NULL DEFAULT 0,
    origin_zone INTEGER NOT NULL,
    origin_epoch BIGINT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    created_tick BIGINT NOT NULL
);
CREATE INDEX idx_warehouse_items_char ON warehouse_items(character_id);

CREATE TABLE character_freight (
    account_id UUID NOT NULL,
    character_id UUID PRIMARY KEY REFERENCES characters(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_character_freight_account ON character_freight(account_id);

CREATE TABLE freight_items (
    id BIGINT PRIMARY KEY, -- ItemInstanceId
    character_id UUID NOT NULL REFERENCES characters(id),
    template_id INTEGER NOT NULL,
    count BIGINT NOT NULL CHECK (count > 0),
    enchant INTEGER NOT NULL DEFAULT 0,
    augment_id BIGINT,
    charges SMALLINT NOT NULL DEFAULT 0,
    origin_zone INTEGER NOT NULL,
    origin_epoch BIGINT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    created_tick BIGINT NOT NULL
);
CREATE INDEX idx_freight_items_char ON freight_items(character_id);

CREATE TABLE mail_messages (
    id UUID PRIMARY KEY, -- Client-provided or API-deterministic ID (no BIGSERIAL; F6)
    sender_id UUID NOT NULL REFERENCES characters(id),
    receiver_id UUID NOT NULL REFERENCES characters(id),
    subject VARCHAR(128) NOT NULL,
    body VARCHAR(512) NOT NULL,
    is_cod BOOLEAN NOT NULL DEFAULT FALSE,
    cod_adena BIGINT NOT NULL DEFAULT 0,
    sent_tick BIGINT NOT NULL,
    expires_tick BIGINT NOT NULL,
    is_read BOOLEAN NOT NULL DEFAULT FALSE,
    is_returned BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX idx_mail_receiver ON mail_messages(receiver_id);

CREATE TABLE mail_attachments (
    id BIGINT PRIMARY KEY, -- ItemInstanceId
    message_id UUID NOT NULL REFERENCES mail_messages(id) ON DELETE CASCADE,
    template_id INTEGER NOT NULL,
    count BIGINT NOT NULL CHECK (count > 0),
    enchant INTEGER NOT NULL DEFAULT 0,
    augment_id BIGINT,
    charges SMALLINT NOT NULL DEFAULT 0,
    origin_zone INTEGER NOT NULL,
    origin_epoch BIGINT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0
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

-- Fact deduplication for async economy counters consumer (B8, F15)
CREATE TABLE economy_counter_facts (
    fact_id UUID PRIMARY KEY,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### 11.2 Phase5Checkpoint and CharacterCheckpoint integration (B6)

`CharacterCheckpoint` stores persistent character state across zone sessions. The 12-item refund tab is zone-only state cleared on logout (`L2PcInstance.java:2539-2543`) and is NOT stored in `Phase5Checkpoint` (B6).

```rust
pub struct Phase5Checkpoint {
    pub schema_version: u32,
    pub rules_hash: [u8; 32],
    pub dwarven_recipes: Vec<u32>,
    pub common_recipes: Vec<u32>,
}
```

### 11.3 Ledger operations under paired transfer rule (F5, B13)

Extend `ItemOperation` in `phase4_projection.rs` and `items.proto` additively:
`Drop`, `PickUp`, `Sweep`, `VendorBuy`, `VendorSell`, `Craft`, `TransferOut`, `TransferIn`, `Deposit`, `Withdraw`, `FreightDeposit`, `FreightWithdraw`, `MailSend`, `MailReceive`, `AuctionBid`, `AuctionOutbid`, `AuctionFinalize`, `ManorSow`, `ManorTurnIn`.
*(Note: `spoil` is dropped; D28 requires an instance mutation for ledger rows).*

**Paired Transfer Rule (B13):**
Whenever an item changes owner (trade, pickup, drop, warehouse, freight, mail, vendor sell/buy of existing instances):
1. `ItemInstanceId` is preserved.
2. Two paired ledger rows are emitted referencing each other and a common receipt ID:
   - `TransferOut` under old owner: `before = Some(instance)`, `after = None`.
   - `TransferIn` under new owner: `before = None`, `after = Some(instance)`.
3. D67 single-appearance rule is evaluated per owner-view; the paired transfer link exempts the cross-owner appearance. Canonical fields must match across the pair.
4. Ground items are owned by `GroundItem` entity (`EntityId`). DB-only containers are owned by container identities (`ContainerId(owner_id, container_kind)`).

### 11.4 Outbox subjects and async economy counters (B8, F15)

Outbox subjects published via NATS JetStream (C9 pattern):
- `nightfall.item.dropped`
- `nightfall.item.picked_up`
- `nightfall.item.swept`
- `nightfall.item.vendor_bought`
- `nightfall.item.vendor_sold`
- `nightfall.item.crafted`
- `nightfall.item.traded`
- `nightfall.item.deposited`
- `nightfall.item.withdrawn`
- `nightfall.item.mailed`
- `nightfall.item.auctioned`
- `nightfall.item.manor_exchanged`

The outbox relay asynchronously aggregates faucets and sinks into `economy_counters`, deduplicating via `economy_counter_facts.fact_id` in the consumer transaction (B8).

---

## 12. Replay, codec and digest coverage

### 12.1 Strict V7 version boundary (F29)

- Snapshot version: **11** (`MAGIC_V11 = 0x4E46_5331_310A_0000`)
- Replay record version: **9**
- State digest version: **`StateDigestVersion::BinaryV7`**
- `PHASE5_ENABLED` gate rules: unset $\to$ V7 for production; "0" $\to$ V6 legacy behaviour; "1" $\to$ V7; any other value $\to$ startup error. Recorded version wins. V7 requires Phase 3 + Phase 4 bundles.

### 12.2 Codec allocations (PbOutput, PbCommandKind, PbPlayerLoad) (F14, F29)

High-water marks: `PbOutput` 41 (reserved by D26 for Record8/V6 context); `PbCommandKind` 31; `PbPlayerLoad` 10.
Allocations start at **42**, **32**, and **11**:

| Target | Tag | Identifier | Frozen Payload / Semantics |
|---|---|---|---|
| **`PbOutput`** | 42 | `GroundItemSpawn` | EntityId, InstanceId, template, count, enchant, Vec2Fixed |
| | 43 | `GroundItemDespawn` | EntityId, reason |
| | 44 | `TradeSessionUpdate` | SessionId, actor_a, actor_b, items_a, items_b, confirmed_a, confirmed_b |
| | 45 | `PrivateStoreUpdate` | EntityId, store_type, title, items |
| | 46 | `RecipeBookSync` | EntityId, dwarven_recipes, common_recipes |
| | 47 | `PartyStateUpdate` | PartyId, leader, members, loot_mode, cursor |
| | 48 | `EconomyOperationResult` | ReceiptId, op_kind, outcome, reject_reason |
| | 49 | `Phase5Projection` | Checkpoint, ledger plans, outbox facts |
| | 50 | `V7EnrichedEvent` | Canonical domain event wrapper |
| **`PbCommandKind`** | 32 | `PickUp` | EntityId, instance_id, template, count |
| | 33 | `VendorBuy` | buylist_id, template, count, frozen_price, frozen_tax |
| | 34 | `VendorSell` | instance_id, template, count, frozen_payout |
| | 35 | `VendorRefund` | instance_id, template, count, frozen_cost |
| | 36 | `Multisell` | list_id, entry_id, frozen_ingredients, frozen_products |
| | 37 | `RegisterRecipe` | recipe_id, recipe_item_instance_id |
| | 38 | `CraftItem` | recipe_id, is_manufacture, fee, manufacturer_id |
| | 39 | `TradeRequest` | target_id |
| | 40 | `TradeAnswer` | accept (bool) |
| | 41 | `TradeAddItem` | instance_id, count |
| | 42 | `TradeRemoveItem` | instance_id, count |
| | 43 | `TradeConfirm` | - |
| | 44 | `TradeCancel` | - |
| | 45 | `SetPrivateStore` | store_type, title, items |
| | 46 | `BuyPrivateStore` | seller_id, index, count, price |
| | 47 | `ClosePrivateStore` | - |
| | 48 | `WarehouseDeposit` | instance_id, count, fee (30) |
| | 49 | `WarehouseWithdraw` | frozen_container_row |
| | 50 | `FreightDeposit` | instance_id, count, target_character_id, fee (1,000) |
| | 51 | `FreightWithdraw` | frozen_container_row |
| | 52 | `SendMail` | receiver_id, subject, body, is_cod, cod_adena, frozen_attachments |
| | 53 | `TakeMailAttachments` | frozen_mail_message, frozen_attachments |
| | 54 | `CancelMail` | frozen_mail_message |
| | 55 | `ItemAuctionBid` | auction_id, bid_adena |
| | 56 | `SowSeed` | target_npc_id, seed_instance_id |
| | 57 | `ManorSellCrop` | manager_npc_id, crop_instance_id, count |
| | 58 | `PartyInvite` | target_id |
| | 59 | `PartyAnswer` | accept (bool) |
| | 60 | `PartyLeave` | - |
| | 61 | `PartyKick` | member_id |
| | 62 | `SetLootMode` | loot_mode (0..4) |
| **`PbPlayerLoad`**| 11 | `Phase5PlayerLoad` | dwarven_recipes, common_recipes, party_id |

### 12.3 Binary digest coverage

State digest V7 incorporates domain-separated Phase 5 state:
- Active `GroundItem` entities (EntityId, instance ID, template, count, enchant, pos `Vec2Fixed`, protection owner, protection tick, decay tick).
- Dead NPC AI sidecars with `corpse_until > current_tick` (NPC EntityId, spoiled flag, spoiler ID, sweep items, swept flag, `corpse_until`).
- Active `PartyState` (PartyId, leader, members in join order, loot mode, `item_last_loot`).
- Active trade sessions (SessionId, actors, items, confirmation flags).
- Active private stores (owner EntityId, store type, title, items).
- Player recipe book sets.

---

## 13. Additive wire and API mapping

### 13.1 Intent $\to$ Command $\to$ Event $\to$ Output mapping table (F14)

| ClientMessage Intent | Tag | PbCommandKind | Result WorldEvent | Tag | PbOutput |
|---|---|---|---|---|---|
| `PickUpItemRequest` | 29 | `PickUp` (32) | `EconomyOperationResult` | 42 | `GroundItemDespawn` (43) |
| `VendorBuyRequest` | 30 | `VendorBuy` (33) | `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `VendorSellRequest` | 31 | `VendorSell` (34) | `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `VendorRefundRequest` | 32 | `VendorRefund` (35) | `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `MultisellRequest` | 33 | `Multisell` (36) | `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `RegisterRecipeRequest` | 34 | `RegisterRecipe` (37) | `RecipeBookSync` | 38 | `RecipeBookSync` (46) |
| `CraftItemRequest` | 35 | `CraftItem` (38) | `CraftResult` | 41 | `EconomyOperationResult` (48) |
| `TradeRequest` | 36 | `TradeRequest` (39) | `TradeSessionChanged` | 36 | `TradeSessionUpdate` (44) |
| `TradeAnswerRequest` | 37 | `TradeAnswer` (40) | `TradeSessionChanged` | 36 | `TradeSessionUpdate` (44) |
| `TradeAddItemRequest` | 38 | `TradeAddItem` (41) | `TradeSessionChanged` | 36 | `TradeSessionUpdate` (44) |
| `TradeRemoveItemRequest` | 39 | `TradeRemoveItem` (42) | `TradeSessionChanged` | 36 | `TradeSessionUpdate` (44) |
| `TradeConfirmRequest` | 40 | `TradeConfirm` (43) | `TradeSessionChanged` | 36 | `TradeSessionUpdate` (44) |
| `TradeCancelRequest` | 41 | `TradeCancel` (44) | `TradeSessionChanged` | 36 | `TradeSessionUpdate` (44) |
| `SetPrivateStoreRequest` | 42 | `SetPrivateStore` (45) | `PrivateStoreChanged` | 37 | `PrivateStoreUpdate` (45) |
| `BuyPrivateStoreRequest` | 43 | `BuyPrivateStore` (46) | `PrivateStoreChanged` | 37 | `PrivateStoreUpdate` (45) |
| `ClosePrivateStoreRequest` | 44 | `ClosePrivateStore` (47)| `PrivateStoreChanged` | 37 | `PrivateStoreUpdate` (45) |
| `WarehouseDepositRequest` | 45 | `WarehouseDeposit` (48)| `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `WarehouseWithdrawRequest`| 46 | `WarehouseWithdraw` (49)| `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `FreightDepositRequest` | 47 | `FreightDeposit` (50) | `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `FreightWithdrawRequest` | 48 | `FreightWithdraw` (51) | `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `SendMailRequest` | 49 | `SendMail` (52) | `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `TakeMailAttachmentsRequest`| 50 | `TakeMailAttachments` (53)| `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `CancelMailRequest` | 51 | `CancelMail` (54) | `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `ItemAuctionBidRequest` | 52 | `ItemAuctionBid` (55) | `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `SowSeedRequest` | 53 | `SowSeed` (56) | `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `ManorSellCropRequest` | 54 | `ManorSellCrop` (57) | `EconomyOperationResult` | 42 | `EconomyOperationResult` (48) |
| `PartyInviteRequest` | 55 | `PartyInvite` (58) | `PartyStateChanged` | 43 | `PartyStateUpdate` (47) |
| `PartyAnswerRequest` | 56 | `PartyAnswer` (59) | `PartyStateChanged` | 43 | `PartyStateUpdate` (47) |
| `PartyLeaveRequest` | 57 | `PartyLeave` (60) | `PartyStateChanged` | 43 | `PartyStateUpdate` (47) |
| `PartyKickRequest` | 58 | `PartyKick` (61) | `PartyStateChanged` | 43 | `PartyStateUpdate` (47) |
| `SetLootModeRequest` | 59 | `SetLootMode` (62) | `PartyStateChanged` | 43 | `PartyStateUpdate` (47) |

### 13.2 Proto definition: `packages/proto/nightfall/v1/economy.proto` (F14)

```protobuf
syntax = "proto3";

package nightfall.v1;

enum EconomyRejectReason {
  ECONOMY_REJECT_REASON_UNSPECIFIED = 0;
  ECONOMY_REJECT_REASON_GROUND_ITEM_PROTECTED = 38;
  ECONOMY_REJECT_REASON_GROUND_ITEM_NOT_FOUND = 39;
  ECONOMY_REJECT_REASON_NOT_SPOILED = 40;
  ECONOMY_REJECT_REASON_ALREADY_SPOILED = 41;
  ECONOMY_REJECT_REASON_NOT_SPOIL_OWNER = 42;
  ECONOMY_REJECT_REASON_VENDOR_OUT_OF_STOCK = 43;
  ECONOMY_REJECT_REASON_RECIPE_ALREADY_KNOWN = 44;
  ECONOMY_REJECT_REASON_RECIPE_BOOK_FULL = 45;
  ECONOMY_REJECT_REASON_CREATE_ITEM_RANK_TOO_LOW = 46;
  ECONOMY_REJECT_REASON_TRADE_TARGET_INVALID = 47;
  ECONOMY_REJECT_REASON_TRADE_DISTANCE_EXCEEDED = 48;
  ECONOMY_REJECT_REASON_STORE_SLOT_LIMIT = 49;
  ECONOMY_REJECT_REASON_WAREHOUSE_FULL = 50;
  ECONOMY_REJECT_REASON_MAIL_ATTACHMENTS_EXCEEDED = 51;
  ECONOMY_REJECT_REASON_NOT_IN_PEACE_ZONE = 52;
  ECONOMY_REJECT_REASON_ITEM_NOT_TRADEABLE = 53;
  ECONOMY_REJECT_REASON_MAILBOX_FULL = 54;
  ECONOMY_REJECT_REASON_INSUFFICIENT_ADENA = 55;
  ECONOMY_REJECT_REASON_INSUFFICIENT_MATERIALS = 56;
  ECONOMY_REJECT_REASON_INSUFFICIENT_MP = 57;
  ECONOMY_REJECT_REASON_KARMA_PLAYER_BLOCKED = 58;
  ECONOMY_REJECT_REASON_PARTY_FULL = 59;
  ECONOMY_REJECT_REASON_NOT_PARTY_LEADER = 60;
  ECONOMY_REJECT_REASON_AUCTION_EXPIRED = 61;
  ECONOMY_REJECT_REASON_BID_TOO_LOW = 62;
  ECONOMY_REJECT_REASON_ALREADY_SEEDED = 63;
}

enum EconomyOpKind {
  ECONOMY_OP_KIND_UNSPECIFIED = 0;
  ECONOMY_OP_KIND_PICKUP = 1;
  ECONOMY_OP_KIND_VENDOR_BUY = 2;
  ECONOMY_OP_KIND_VENDOR_SELL = 3;
  ECONOMY_OP_KIND_VENDOR_REFUND = 4;
  ECONOMY_OP_KIND_MULTISELL = 5;
  ECONOMY_OP_KIND_CRAFT = 6;
  ECONOMY_OP_KIND_TRADE = 7;
  ECONOMY_OP_KIND_WAREHOUSE_DEPOSIT = 8;
  ECONOMY_OP_KIND_WAREHOUSE_WITHDRAW = 9;
  ECONOMY_OP_KIND_FREIGHT_DEPOSIT = 10;
  ECONOMY_OP_KIND_FREIGHT_WITHDRAW = 11;
  ECONOMY_OP_KIND_MAIL_SEND = 12;
  ECONOMY_OP_KIND_MAIL_TAKE = 13;
  ECONOMY_OP_KIND_AUCTION_BID = 14;
  ECONOMY_OP_KIND_MANOR_SOW = 15;
  ECONOMY_OP_KIND_MANOR_TURN_IN = 16;
}

// Intent bodies with idempotency keys
message PickUpItemRequest {
  string idempotency_key = 1;
  string ground_entity_id = 2; // UUID
}

message VendorBuyRequest {
  string idempotency_key = 1;
  uint32 buylist_id = 2;
  uint32 item_template_id = 3;
  uint64 count = 4;
}

message VendorSellRequest {
  string idempotency_key = 1;
  uint64 item_instance_id = 2;
  uint64 count = 3;
}

message VendorRefundRequest {
  string idempotency_key = 1;
  uint64 item_instance_id = 2;
}

message MultisellRequest {
  string idempotency_key = 1;
  uint32 list_id = 2;
  uint32 entry_id = 3;
  uint64 prepare_item_id = 4; // for maintainEnchantment
}

message RegisterRecipeRequest {
  string idempotency_key = 1;
  uint32 recipe_id = 2;
  uint64 recipe_item_instance_id = 3;
}

message CraftItemRequest {
  string idempotency_key = 1;
  uint32 recipe_id = 2;
}

message TradeRequest {
  string idempotency_key = 1;
  string target_player_id = 2; // UUID
}

message TradeAnswerRequest {
  string idempotency_key = 1;
  bool accept = 2;
}

message TradeAddItemRequest {
  string idempotency_key = 1;
  uint64 item_instance_id = 2;
  uint64 count = 3;
}

message TradeRemoveItemRequest {
  string idempotency_key = 1;
  uint64 item_instance_id = 2;
  uint64 count = 3;
}

message TradeConfirmRequest {
  string idempotency_key = 1;
}

message TradeCancelRequest {
  string idempotency_key = 1;
}

message SetPrivateStoreRequest {
  string idempotency_key = 1;
  uint32 store_type = 2; // 0=Sell, 1=Buy, 2=Package, 3=Manufacture
  string title = 3;
  repeated TradeItemDto items = 4;
}

message BuyPrivateStoreRequest {
  string idempotency_key = 1;
  string seller_player_id = 2;
  uint32 item_index = 3;
  uint64 count = 4;
  uint64 price = 5;
}

message ClosePrivateStoreRequest {
  string idempotency_key = 1;
}

message WarehouseDepositRequest {
  string idempotency_key = 1;
  uint64 item_instance_id = 2;
  uint64 count = 3;
}

message WarehouseWithdrawRequest {
  string idempotency_key = 1;
  uint64 item_instance_id = 2;
  uint64 count = 3;
}

message FreightDepositRequest {
  string idempotency_key = 1;
  uint64 item_instance_id = 2;
  uint64 count = 3;
  string target_character_id = 4;
}

message FreightWithdrawRequest {
  string idempotency_key = 1;
  uint64 item_instance_id = 2;
  uint64 count = 3;
}

message SendMailRequest {
  string idempotency_key = 1;
  string receiver_character_id = 2;
  string subject = 3;
  string body = 4;
  bool is_cod = 5;
  uint64 cod_adena = 6;
  repeated uint64 attachment_instance_ids = 7;
}

message TakeMailAttachmentsRequest {
  string idempotency_key = 1;
  string message_id = 2;
}

message CancelMailRequest {
  string idempotency_key = 1;
  string message_id = 2;
}

message ItemAuctionBidRequest {
  string idempotency_key = 1;
  uint32 auction_id = 2;
  uint64 bid_adena = 3;
}

message SowSeedRequest {
  string idempotency_key = 1;
  string target_npc_id = 2;
  uint64 seed_instance_id = 3;
}

message ManorSellCropRequest {
  string idempotency_key = 1;
  uint64 crop_instance_id = 2;
  uint64 count = 3;
}

message PartyInviteRequest {
  string idempotency_key = 1;
  string target_player_id = 2;
}

message PartyAnswerRequest {
  string idempotency_key = 1;
  bool accept = 2;
}

message PartyLeaveRequest {
  string idempotency_key = 1;
}

message PartyKickRequest {
  string idempotency_key = 1;
  string member_player_id = 2;
}

message SetLootModeRequest {
  string idempotency_key = 1;
  uint32 loot_mode = 2; // 0..4
}

// Carrier DTOs
message TradeItemDto {
  uint64 item_instance_id = 1;
  uint32 item_template_id = 2;
  uint64 count = 3;
  uint64 price = 4;
  uint32 enchant = 5;
}

message GroundItemDetail {
  uint64 item_instance_id = 1;
  uint32 item_template_id = 2;
  uint64 count = 3;
  uint32 enchant = 4;
  bool is_protected = 5; // Observer-dependent
}

message EconomyOperationResult {
  string idempotency_key = 1;
  EconomyOpKind op_kind = 2;
  bool success = 3;
  EconomyRejectReason reject_reason = 4;
  uint64 receipt_id = 5;
}

message RecipeBookSync {
  repeated uint32 dwarven_recipes = 1;
  repeated uint32 common_recipes = 2;
}

message CraftResult {
  uint32 recipe_id = 1;
  bool success = 2;
  bool is_rare = 3;
  uint32 product_template_id = 4;
  uint64 product_count = 5;
}

message TradeSessionChanged {
  uint64 session_id = 1;
  string partner_id = 2;
  repeated TradeItemDto my_items = 3;
  repeated TradeItemDto partner_items = 4;
  bool my_confirmed = 5;
  bool partner_confirmed = 6;
  bool is_cancelled = 7;
}

message PrivateStoreChanged {
  string owner_id = 1;
  uint32 store_type = 2;
  string title = 3;
  repeated TradeItemDto items = 4;
  bool is_closed = 5;
}

message PartyStateChanged {
  string party_id = 1;
  string leader_id = 2;
  repeated string member_ids = 3;
  uint32 loot_mode = 4;
}

message MultisellWindow {
  uint32 list_id = 1;
  bool maintain_enchantment = 2;
  bool apply_taxes = 3;
}
```

---

## 14. Independent oracle, regression and acceptance plan

### 14.1 Python Fraction oracle vector families (F1, F7, F9, F17, F19, F31)

Author `packages/data/scripts/economy_oracle.py` using `fractions.Fraction` without floating-point arithmetic. Nine verification families:

1. **Family 1 — Precise Drop Calculations & Exact Rationals (F1):**
   - Rational chances down to $0.0003000000142492354$% and maximum $188.85499572753906$%.
   - Wide uniform 64-bit draw comparison: $r \times \text{den} < \text{num} \times 2^{64}$.
   - Exact amount multiplier $M_{\text{base}} = \lfloor \text{Num} / (100 \times \text{Den}) \rfloor$ and remainder roll.
   - Normalized multi-item CDF walk over normalized rational shares.
2. **Family 2 — Deep Blue Level Gap Windows:**
   - Level difference $d \in [-15, 5]$.
   - Assert exact 18-point percentage steps: $d \ge -5 \implies 100\%$, $-6 \implies 82\%$, $-7 \implies 64\%$, $-8 \implies 46\%$, $-9 \implies 28\%$, $d \le -10 \implies 10\%$.
   - Raid boss clamp with $d \le -7 \implies 0\%$. Ungrouped raid items using window (`IKillerChanceModifierStrategy.java:44-53`).
3. **Family 3 — Party Adena Split & Sweep Routing:**
   - Total adena from 1 to $99{,}900{,}000{,}000$ across party sizes $K \in \{2, 3, 5, 7, 9\}$.
   - When $\text{count} == 1$, follows item loot mode (no split).
   - When $\text{count} > 1$, even split with 1-by-1 remainder in canonical join order.
   - Sweep loot routing under `INCLUDING_SPOIL` modes (to party members, not sweeper).
4. **Family 4 — Vendor Pricing & Double-Truncation Rounding (F8, F9, F17):**
   - Integer emulation of IEEE double truncation for $(1.0 + \text{baseTax} + \text{castleTax})$ across all real AI script taxes and castle tax 0%, 5%, 15%, 25%.
   - Sell price: $\lfloor \text{refPrice} / 2 \rfloor \times \text{count}$.
   - Refund buy-back: $\lfloor (\text{count} \times \text{refPrice}) / 2 \rfloor$. Assert difference of 1 for odd price with even count.
   - 12-item FIFO buffer overflow eviction.
5. **Family 5 — Multisell Ingredient Scaling & Taxes (F19):**
   - Tax charge: $\text{round}(\text{count} \times \text{taxRate})$ half-up on `isTaxIngredient` adena ingredients.
   - Bitwise augmentation copy (`stat34 << 16 | stat12`) and enchant level copy under list-level `maintainEnchantment`.
   - Elemental attribute copy verified as no-op.
6. **Family 6 — Crafting Rolls & Masterwork Rarity (F7):**
   - Success rates across 100%, 70%, 60%, 25%, 10%.
   - Masterwork rolls: per-recipe rarity (1%, 4%, or 20%) against 100 and `rareCount` on all 582 rare recipes.
   - Pinned execution order MP $\to$ fee $\to$ materials $\to$ success $\to$ rare.
7. **Family 7 — Chaotic PK Death Drops (F3):**
   - Single loop in canonical `ItemInstanceId` ascending order.
   - Unequip-then-roll per equipped item. Weapon 10%, other equip 40%, inventory 50%, capped at 10 items.
   - Position draws $\pm 25$ L2 units.
8. **Family 8 — Timers & Decay Lifecycles (F26):**
   - Item decay = 6,000 ticks (600 s); herb decay = 600 ticks (60 s); per-template overrides (e.g. 10 s).
   - Corpse decay = 70 ticks (7 s) + 100 ticks (10 s) spoil = 170 ticks (17 s); 234 template overrides.
   - Normal protection = 150 ticks (15 s); raid CC protection = 9,000 ticks (900 s).
   - Trade request expiry = 150 ticks (15 s).
9. **Family 9 — Unit Conversions (F25, E11):**
   - Conversion formula: $\lfloor \text{L2} \times 1000 / 32 \rfloor$ milli-tiles.
   - Pickup: 36 L2 $\to 1{,}125$ milli-tiles.
   - Trade & Vendor interaction: 150 L2 $\to 4{,}687$ milli-tiles.
   - Sweeper cast range: 20 L2 $\to 625$ milli-tiles.
   - PartyRange2: 1400 L2 $\to 43{,}750$ milli-tiles.
   - PartyRange: 1600 L2 $\to 50{,}000$ milli-tiles.
   - NPC drop offset: $\pm 70$ L2 $\to \pm 2{,}187$ milli-tiles.
   - PK drop offset: $\pm 25$ L2 $\to \pm 781$ milli-tiles.

### 14.2 Evidence contract schema version 2 (F31)

Extend the Phase 4 evidence contract additively to schema version 2:
- Export facts include: `drop`, `pickup`, `spoil`, `sweep`, `vendor_buy`, `vendor_sell`, `craft`, `trade`, `mail`, `auction`, `manor`.
- RNG draw traces: `(tick, phase, resolution_id, event_kind, caller_site, bound, raw_u32, residue)`.
- Replay facts verified by oracle against actual inputs. UNKNOWN is never allow-listed (C11, C14, C15, C16).

### 14.3 Simulation scenario matrix (20 scenarios)

Scenarios located in `apps/client-unreal/Scenarios/`:
1. `5-ground-item-drop.nfs`: NPC death spawns GroundItem with 15 s protection; range $\le 36$ L2 units pickup.
2. `5-ground-item-decay.nfs`: 600 s item / 60 s herb decay; player drop non-decay rule.
3. `5-herb-pickup.nfs`: Immediate herb effect applied without inventory slot.
4. `5-spoil-sweep.nfs`: Spoil 254 debuff on living mob; Sweeper 42 corpse item collection; 17 s corpse decay.
5. `5-party-loot-modes.nfs`: 5 party distribution modes; adena even split within 1400 L2 units; including-spoil routing.
6. `5-vendor-buy-sell.nfs`: AI-script merchant buylist with double-truncation tax; sell at price/2; 12-item refund tab.
7. `5-multisell.nfs`: Generic exchange; `maintainEnchantment` preserves enchant + augment; castle tax on `isTaxIngredient`.
8. `5-craft-dwarven.nfs`: Dwarven recipe registration; Create Item ranks; MP $\to$ materials $\to$ roll; 582 masterwork rare items.
9. `5-craft-common.nfs`: Common recipe registration and craft for non-Dwarf.
10. `5-player-trade.nfs`: Two-phase atomic trade within 150 L2 units; 15 s expiry; cancel without penalty.
11. `5-private-store-sell.nfs`: Private sell store; zone-only state; close on disconnect.
12. `5-private-store-buy.nfs`: Private buy store with escrow; close on disconnect.
13. `5-private-manufacture.nfs`: Dwarf workshop; fee transfer before roll.
14. `5-warehouse.nfs`: Private warehouse deposit (30 adena fee) and withdraw; ItemInstanceId retained.
15. `5-freight.nfs`: Same-account freight transfer (1,000 adena fee) to alt character.
16. `5-mail-attachments.nfs`: Peace zone mail; 8 attachments; 100+1000 fee; COD payment and return.
17. `5-chaotic-pk-drop.nfs`: Chaotic player death; single-loop unequip-then-roll; max 10 drops; $\pm 25$ L2 offsets.
18. `5-rate-multiplier.nfs`: Base 1x vs multiplied exact rational drop chance and XP/SP rates.
19. `5-item-auction.nfs`: Item auction state machine; bidding, refunds, and finalization.
20. `5-manor-seed-crop.nfs`: Manor seed sowing; crop drop replacement; crop turn-in via ExchangeAudit.

---

## 15. Binding architect dispositions (superseding open questions)

All previous open questions are resolved by binding architect decisions in `AD5` (§5 B1–B8, §6 E1–E6 + B9–B12, §7 E7–E12 + B13–B14):
1. **GroundItem Identity (B5):** A `GroundItem` is an `Entity` in `EntityId` UUID space carrying an `ItemInstanceId`. No separate `GroundItemId` allocator.
2. **Refund Tab Persistence (B6):** The 12-item refund tab is zone-only state, cleared on logout (`L2PcInstance.java:2539-2543`). It is not persisted in `Phase5Checkpoint`. Items in it are destroyed per pinned semantics.
3. **Trade Disconnect / Abort (B3, B7):** Disconnect during trade cancels session for both parties. Items remain in owners' inventories with no penalty. Pinned cancel notification emitted. Pinned radius is 150 L2 units; request expiry is 15 s; no 120 s timeout; no TradeLock step.
4. **Economy Counters Rollup (B8):** `economy_counters` table is populated asynchronously by an outbox consumer, deduplicated by `fact_id` in `economy_counter_facts`. Never inside the zone checkpoint transaction.
5. **Party Substrate (B9):** Minimal party model in zone-canonical state with `PartyId`, canonical join order, 5 loot modes, `item_last_loot` cursor, adena split, and Highfive XP cutoff.
6. **Elemental Attributes (B10):** No-op with pinned citation recorded.
7. **Cast Pipeline Integration (B11):** Spoil 254, Sweeper 42 with `NPC_BODY` target type, and herb skills admitted through Phase 3 cast pipeline.
8. **Owner-Changing Transfers (B13):** `ItemInstanceId` never changes across owners. PAIRED `transfer_out`/`transfer_in` ledger rows emitted. Ground items and containers are owners with explicit identities.
9. **Rate Multiplier Placement (B14):** Global multiplier applied where pinned rates enter algorithm as an exact rational factor on chance. Surplus chance $\ge 100\%$ converts to amount multiplier.
10. **Replay Independence (E12):** RPC mutations freeze DB inputs into command record; no `NOW()` or `BIGSERIAL` in zone state.
