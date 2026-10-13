# Phase 5 Plan: Economy and Crafting

**Status:** Planning; implementation and acceptance pending.

Author: D (agy, role `plan`). Architect: Claude. Independent review: Claude. This document specifies executable work and evidence; it does not attest that any story or implementation gate has passed.

## 1. Goal and locked decisions

Deliver server-authoritative economy and crafting systems for Nightfall: monster drops, spoil and sweep, herbs, ground items with ownership and decay timers, 5 party loot distribution modes, NPC vendors and buylists with castle tax hooks, multisell exchange engine, Dwarven and Common crafting (all 1,000 pinned recipes) with recipe books and masterwork support, player-to-player trade (two-phase atomic), private stores, mail with attachments and cash-on-delivery (COD), private warehouse (100 / 120 Dwarf) and freight (200), and item auction / manor movement on fixture inputs. Unreal submits intents and renders received state.

**§2 is the canonical contract section.** Stories reference it without introducing alternative mechanics. [Binding decisions](phase-5-architecture-decisions.md), including owner P1–P4, A1–A20 and approved defaults, win over research. All [Phase 3 decisions](phase-3-architecture-decisions.md) and [Phase 4 decisions](phase-4-architecture-decisions.md) remain binding except the explicit additions below.

| Decision | Authority | Plan reference |
|---|---|---|
| Scope: monster drops, spoil/sweep, herbs, ground items, 5 party loot modes, vendors/buylists, multisell, crafting (Dwarven + Common), private warehouse + freight, player trade, private stores, mail, auction + manor; clan warehouse Phase 7 | P1/A2 | §2.3/§2.4/§2.6/§8 |
| Configurable Global Rate Multiplier (default 1x, rational, hashed into Phase5Rules); scenarios assert base and multiplied values | P2/A5 | §2.4/§2.7 |
| Crafting: all 1,000 pinned recipes (830 Dwarven + 170 Common), Create Item 172 (10 ranks) / Create Common Item 1320 (9 ranks), pinned success rates, 100% loss on failure, CraftMasterwork=True (583 rare products), 50/50 recipe books, recipe consumed on register | P3/A10 | §2.4/§2.6/§2.7 |
| Storage: private warehouse (100 / 120 Dwarf; 30 adena deposit fee) + freight (200 slots, 1,000 adena fee); clan warehouse Phase 7 | P4/A13 | §2.5/§2.6/§2.7 |
| Server authoritative, 100 ms tick, per-zone ChaCha12, Q=1_000_000 i64 fixed point, no floats/HashMap/wall clock, additive wire, byte-exact legacy replay, critical mutation order | A1 | §2.1/§2.3/§2.5 |
| Zone-canonical authority for all simulation state: ground items, corpses, vendors, multisell, craft, trade, private stores; DB-only storage/mail with zone-mediated item movement | A2 | §2.3/§2.5 |
| Additive wire: packages/proto/nightfall/v1/economy.proto; intents 29+, WorldEvent 34+, RejectReason 38+, StatsChanged 27+, EntitySpawn 30+ GroundItem, GameService RPCs | A3 | §2.1 |
| Snapshot 11 / Record 9 / Binary V7 for Phase 5 epochs; separately content-hashed Phase5Rules bundle; PHASE5_ENABLED gate | A4 | §2.2 |
| Per-zone ChaCha12 fixed in-tick RNG order; precise-drop algorithm; 5/10/10% level-gap window; champion x8 | A5 | §2.3/§2.7 |
| Drops: 59 death XMLs, 52 corpse XMLs; killer = top-damage online player (maxDealer); 15s protection; 600s decay items / 60s herbs; player drops not auto-destroyed; 68 herbs ex_immediate_effect; 5 party loot modes; adena split 1400 units | A6 | §2.3/§2.4/§2.6 |
| Spoil/Sweep: Spoil 254 calcMagicSuccess; Sweeper 42 (500ms hit / 500ms reuse / 3 MP); corpse decay 7s default + 10s spoil extension | A7 | §2.3/§2.6/§2.7 |
| Vendors: 195 buylists with restock; buy price = trunc(price × (1 + castleTax)); sell = price / 2; refund tab max 12; radius 3 | A8 | §2.3/§2.6/§2.7 |
| Multisell: 196 lists; D37/D52 exchange audit; maintainEnchantment copies enchant + augment + elemental attributes | A9 | §2.4/§2.6 |
| Trade: two-phase atomic confirm; validation at commit; capacity checked; private stores 3/4 (Dwarf 4/5); OfflineTradeEnable=False | A11 | §2.3/§2.6 |
| Mail: DB mailbox; peace zone send; 8 attachments; 100 + 1,000/slot fee; 15-day expiry; 12h COD; SendMailInterval 10s | A12 | §2.3/§2.5/§2.6 |
| Persistence: migrations from 000012; Phase5Checkpoint in CharacterCheckpoint; outbox nightfall.item.*; economy_counters rollups | A15 | §2.5 |
| Harness/evidence: scenarios 5-*.nfs; Python Fraction economy_oracle.py; C11/C14/C15/C16 producing and consuming verified evidence | A16 | §2.8/§5 |
| Client: ground item rendering, pickup, loot settings, vendor/multisell/craft/trade/store/warehouse/mail dialogs; bot DSL | A17 | §2.1/§4/§5 |
| Performance budgets unchanged: tick p99 < 20 ms in 8-client soak; 1k-NPC death-with-drops bench; ground items bounded by decay | A18 | §5.4 |
| Tooling: Z/C critical sim, API, Unreal, harness = Claude subagents; D = agy gemini-3.8-flash for data/docs with Claude verification | A20 | §3 |

No calendar estimates, invented tuning or completion marks. Numbers that describe Nightfall versions, tags, capacities of test workloads and gates are architect/API contracts, not claims about L2J. Every L2J numeric rule below cites the pinned source; exact decimal literals mean rational values, never runtime floating-point contracts.

## 2. Source authority and canonical contracts

Pinned repositories, verified at plan authoring:

- `GJ/` = l2j-game `abfde0490ac52a2106e884b7242c0276c44cdf76`, `src/main/java/com/l2jserver/gameserver/`.
- `GC/` = that game revision's `src/main/resources/config/`.
- `DP/` = l2j-dp `3ca488dd2bd0bfaca43e378886a3c2e37968153a`, `src/main/resources/data/`.
- `DPJ/` = that datapack revision's `src/main/java/com/l2jserver/datapack/`.

Clones reside under `/home/matt-woodruff/.cache/nightfall/l2j/`; they are build-time provenance, never replay/runtime dependencies. Source ledgers must retain full revision, exact path, literal line range, symbol, input/output hashes and rounding sites. [05 research](../planning/05-economy-and-crafting.md) is unpinned; its disagreements are enumerated in §2.7 and the preserved plan-progress notes. Engineering/API/database standards take precedence over its SQL, stream, UUID, actor and hot-reload sketches.

**Phase 4 reconciliation:** Phase 5 consumes Phase 4 `ItemInstanceId(u64)` ranges (`D8`/`D19`), `item_ledger` schema and ops (`D29`/`D73`), inventory capacity checks `can_accept(template, count)` (`PcInventory.java:772-785`), `ItemExchanged` outbox facts (`D52`), item catalogue flags (`is_tradable`, `is_sellable`, `is_droppable`, `is_destroyable`, `ex_immediate_effect`), out-of-world `GrantItem` via zone runtime port (`D30`/`D48`), effective stats and penalties (overload 4270, grade 6209/6213), and integer evaluation without floats (`A8`/`D21`). Phase 5 widens catalogue admission to include herbs, crafting materials, key materials, recipes, and vendor goods under `D5`/`D54`.

### 2.1 Additive wire allocation and visibility

C publishes the first proto/RPC/error/idempotency contract; Z reviews before dependents. New economy types live in `packages/proto/nightfall/v1/economy.proto`; the existing ClientMessage, ServerMessage, WorldEvent and GameService remain the only envelopes/service channel. No SubscribeInventory or additional stream (A2).

| Tag space | Append-only allocation (A3/owner-decisions) | Contract |
|---|---|---|
| ClientMessage intent | 29..54 (A3) | PickUp=29, VendorBuy=30, VendorSell=31, VendorRefund=32, MultisellChoose=33, RegisterRecipe=34, CraftItem=35, TradeInitiate=36, TradeRespond=37, TradeAddItem=38, TradeRemoveItem=39, TradeConfirm=40, TradeCommit=41, TradeCancel=42, SetPrivateStore=43, CancelPrivateStore=44, PrivateStoreBuy=45, PrivateStoreSell=46, PrivateStoreManufacture=47, DepositWarehouse=48, WithdrawWarehouse=49, DepositFreight=50, WithdrawFreight=51, SendMail=52, CancelMail=53, SetLootMode=54 |
| WorldEvent | 34..44 (A3) | GroundItemSpawn=34, GroundItemDespawn=35, CorpseStateChanged=36, PartyLootDistributed=37, TradeSessionChanged=38, PrivateStoreChanged=39, CraftResult=40, VendorTransactionResult=41, MailNotification=42, WarehouseResult=43, FreightResult=44 |
| RejectReason | 38..65 (A3) | GROUND_ITEM_NOT_FOUND=38, GROUND_ITEM_PROTECTED=39, PICKUP_TOO_FAR=40, VENDOR_NOT_IN_RANGE=41, ITEM_NOT_SELLABLE=42, REFUND_EMPTY=43, MULTISELL_LIST_NOT_FOUND=44, RECIPE_ALREADY_REGISTERED=45, RECIPE_BOOK_FULL=46, CRAFT_LEVEL_TOO_LOW=47, INSUFFICIENT_MATERIALS=48, TRADE_TARGET_INVALID=49, TRADE_ALREADY_TRADING=50, TRADE_TOO_FAR=51, TRADE_ITEM_NOT_TRADABLE=52, TRADE_SLOTS_EXCEEDED=53, STORE_SLOT_LIMIT=54, STORE_ALREADY_OPEN=55, MAIL_NOT_IN_PEACE_ZONE=56, MAIL_ATTACHMENTS_EXCEEDED=57, MAIL_RECEIVER_NOT_FOUND=58, MAIL_INSUFFICIENT_ADENA=59, WAREHOUSE_FULL=60, FREIGHT_FULL=61, NOT_SPOILED=62, ALREADY_SPOILED=63, SWEEP_NOT_OWNER=64, NOT_PARTY_LEADER=65 |
| StatsChanged fields | 27..31 | Owner adena_amount27, warehouse_used_slots28, freight_used_slots29, dwarven_recipe_count30, common_recipe_count31 |
| EntityKind | 3 | ENTITY_KIND_GROUND_ITEM = 3 |
| EntitySpawn fields | 30 | GroundItemData ground_item = 30 (template_id, count, owner_id, protection_until_tick, is_herb) |
| GameService | after GrantItem | ListBuylist, GetRecipeBook, GetWarehouse, GetFreight, ListMail, ReadMail, TakeMailAttachments, ListItemAuctions, GetManorData |
| Replay PbOutput / PbCommandKind / PbPlayerLoad | 42 / 32 / 11 | Outputs42..52 economy events; commands32..57 economy intents; PlayerLoad11 strict Phase5 admission |

Prefix enum values (`LOOT_MODE_FINDERS_KEEPERS`, `TRADE_STATE_LOCKED`, `STORE_TYPE_SELL`); unspecified required enums reject. Item instances continue using typed u64 identity. No client chance, price, tax, or RNG inputs are accepted as authority.

| Carrier / fact | Owner | AOI observers |
|---|---|---|
| GroundItemSpawn (template, count, position, protection) | Public to all observers | Visible ground mesh/icon + protection timer |
| GroundItemDespawn (ground item id, reason: pickup / decay) | Public to all observers | Removes ground representation |
| CorpseStateChanged (entity id, spoiled flag, sweep active) | Public to all observers | Corpse glow / interaction state |
| PartyLootDistributed (picker, recipient, template, count, mode) | Party members only | Never |
| TradeSessionChanged (session id, state, offered items/adena) | Trade participants only | Never |
| PrivateStoreChanged (entity id, store type, title, items, fees) | Owner full state | Public title, item list and prices |
| CraftResult / VendorTransactionResult / WarehouseResult | Owner only (frozen receipt) | Never |
| MailNotification (sender, subject, has_attachments, cod_amount) | Receiver only | Never |
| StatsChanged / GetWarehouse / GetFreight / GetRecipeBook | Owner only | Never |
| ListBuylist / ListItemAuctions / GetManorData | Authenticated reader | Never (read-only reference) |

Preserve C2/C2′/C4/C12: identities precede reference hydration; GroundItemSpawn follows entity ordering with baseline_tick/generation fencing. Known observers receive only safe public projections. Baselines reset/upsert without transition counts; ordinary stale deltas are rejected.

### 2.2 Version matrix and legacy compatibility

| Snapshot | Digest | Record | Rules / policy |
|---|---|---|---|
| 4,5 | JsonV1 | 3 | Historical Phase 1 validation unchanged |
| 6..8 | JsonV1/BinaryV2 | 3/4 respectively | Existing no-class legal matrix unchanged |
| 7 | BinaryV3 | 5 | Phase 2 before tokens |
| 8 | BinaryV4 | 6 | Phase 2 tokens |
| 9 | BinaryV5 | 7 | Valid Phase3 block/hash and version_one admission |
| 10 | BinaryV6 | 8 | Valid Phase3 plus Phase4 block/rules/hash and version_two admission |
| 11 | BinaryV7 | 9 | Valid Phase3, Phase4 plus Phase5 block/rules/hash and version_three admission |

A4 requires `StateDigestVersion::BinaryV7`, explicit codec schema dispatch and `RECORD_SCHEMA_VERSION=9` while retaining every historical decoder. Reject mixed pairs and recursively reject presence of Phase 5 fields in old loads/snapshots before permissive decoding drops it. Historical `.nfr` container and fixture bytes never change.

`PHASE5_ENABLED`: unset selects V7 except historical simulation-fixture selection, `"0"` selects V6 behaviour, `"1"` selects V7, other values refuse boot (A4). All live suites explicitly use production V7 and reviewed bundle under C16.

Phase5Rules is validated, canonical and separately content-hashed: drop tables, spoil tables, herb tables, buylists, multisell lists, recipe dataset, global rate multiplier, ownership/decay constants, tax constants, party loot constants, storage limits, mail limits, auction/manor schedules. Hash-referenced like Phase4Rules (A4/D27), never embedded.

### 2.3 Engine state and in-tick ordering

Zone-canonical Phase5Snapshot includes ground items, corpses, active vendor sessions, trade sessions, private stores, recipe books, party loot modes, and restock timers:

- `ground_items: BTreeMap<GroundItemId, GroundItemInstance>`: allocated GroundItemId(u64), template_id, count, position, owner_id (top-damage player), party_id, protection_deadline_tick, decay_deadline_tick, is_herb, is_player_dropped.
- `corpses: BTreeMap<EntityId, CorpseState>`: spoiled flag, spoiler_id, spoiler_party_id, `sweep_items: Vec<SweepItem>`, decay_deadline_tick.
- `vendor_sessions: BTreeMap<EntityId, VendorSessionState>`: refund buffer (max 12 items, `PcRefund.java:60-64`).
- `active_trades: BTreeMap<TradeId, TradeSession>`: two-phase state machine (`TradeList.java:330-450`), participants, locked items, confirmed flags.
- `private_stores: BTreeMap<EntityId, PrivateStoreState>`: sell, buy, package sell, manufacture (`L2PcInstance.java:8000+`); slot limits 3/4 (Dwarf 4/5).
- `recipe_books: BTreeMap<EntityId, RecipeBookState>`: dwarven (max 50) and common (max 50) registered recipe sets (`GC/character.properties:488-489`).

In-tick schedule preserves Phase 3/4 boundaries and enforces the following fixed order (A5):

| Tick phase | Phase 5 additions and required barriers |
|---|---|
| Commands & intents | 1. Authenticate, rate-limit, check idempotency key.<br>2. Resolve PickUp intent: check range (<= 150 units), ownership protection, and `can_accept` capacity.<br>3. Resolve Sweeper (skill 42): checkSpoilOwner (spoiler or party member), hitTime 500ms, MP 3; transfer sweep items via `sweep` op.<br>4. Resolve CraftItem: check recipe registration & Create Item rank; deduct MP; roll success; roll masterwork; consume materials (100% loss on fail); grant product via `craft` op.<br>5. Resolve VendorBuy/VendorSell/VendorRefund: NPC proximity radius 3; tax calculation; refund buffer update (max 12).<br>6. Resolve Trade intents: 2-phase confirm state machine; capacity validation; atomic critical mutation.<br>7. Resolve PrivateStoreBuy/Sell/Manufacture. |
| Movement / chase | Movement update. Ground item collision and proximity checks read post-step positions. |
| Combat / cast impact | Spoil (skill 254): `calcMagicSuccess` roll at impact (`Spoil.java:40-42`). On success, set `isSpoiled = true` on target. If already spoiled, reject with ALREADY_SPOILED. |
| Death & drop resolution | 1. Identify killer = `maxDealer` (online player with top damage within 1600 units; fallback to lastAttacker; `L2Attackable.java:341-379`).<br>2. Calculate level gap `d = mobLevel - killerLevel`.<br>3. Roll `<death>` list: apply global rate multiplier; grouped items use 5/10/10% item level-gap window (`IKillerChanceModifierStrategy.java:33-43`); single-item collapse; multi-item walk; precise-drop roll if chance >= 100.<br>4. Spawn ground items: 15s protection to killer + party (`ItemTable.java:214`); 600s decay (`general.properties:258`); herbs (68 `ex_immediate_effect` items) 60s decay (`general.properties:262`).<br>5. Roll `<corpse>` list: ungrouped items rolled independently into corpse `_sweepItems` (`L2Attackable.java:838-840`).<br>6. Corpse decay: 7s default (`npc.properties:86`) + 10s spoil extension (`:90`).<br>7. Chaotic PK death drops: karma > 0, PK >= 6, outside PvP zone; unequip eligible gear before roll (`L2PcInstance.java:4531-4534`); weapon 10%, equip 40%, item 50%, limit 10; 15s protection; player drops not auto-destroyed (`general.properties:306,312`). |
| Decay & expiries | Scan decay deadlines: despawn expired ground items (herbs 60s, items 600s) and expired corpses. |
| Output & persistence | Log draft -> JetStream ack -> SeaORM fenced checkpoint + outbox -> visible output events. Outbox emits `nightfall.item.*` facts. |

### 2.4 Generated data and capability manifest

E1 generates `packages/data/tables-phase5/` through `gen_economy.py` with `--check`, exact decimal strings/rationals, provenance headers, full DATAPACK_REVISION check, SOURCES.md registration and `check_all.py` integration:

| File | Authoritative contents / closure | Source citations |
|---|---|---|
| `drops.toml` | 59 NPC XML files with `<death>` lists: 2,938 groups, 307 ungrouped items. Rates, min/max count, chance. | `DP/stats/npcs/*.xml`; `GJ/data/xml/impl/NpcData.java:455-467`; `GJ/model/drops/DropListScope.java:31` |
| `spoil.toml` | 52 NPC XML files with `<corpse>` lists: ungrouped items, rates, min/max count, chance. | `DP/stats/npcs/*.xml`; `GJ/model/drops/DropListScope.java:32`; `L2Attackable.java:838-840` |
| `herbs.toml` | 68 EtcItems with `ex_immediate_effect="true"`. Consumed on pickup, immediate buff, no inventory slot. | `GJ/model/items/L2Item.java:187-188,743-745`; `DP/stats/items/*.xml` |
| `buylists.toml` | 195 XML buylists: buylist ID, NPC associations, item templates, price, restock count and delay. | `DP/buylists/*.xml`; `GJ/data/xml/impl/BuyListData.java:100-117` |
| `multisell.toml` | 196 XML multisell lists: list ID, maintainEnchantment, applyTaxes, ingredients list, products list. | `DP/multisell/*.xml`; `GJ/data/xml/impl/MultisellData.java` |
| `recipes.toml` | 1,000 recipes: 830 Dwarven, 170 Common. Level, product ID, count, success rate, MP cost, materials. 583 masterwork products (`productionRare`). | `DP/recipes.xml:1-11800`; `GC/character.properties:484` |
| `rates.toml` | Global rate multiplier (default 1x, rational, TOML-tunable). DeathDropChance/Amount, CorpseDrop*, RaidDrop*, HerbDrop*. Champion multipliers (HP x8, rewards x8, exp/sp x8, adena x1; ChampionEnable=False). | `GC/rates.properties:16-37`; `GC/customs.properties:10,28,34,37,43` |
| `ownership_decay.toml` | Normal drop protection 15s (`ItemTable.java:214`). Raid protection 900s for first CC >= 45 (`character.properties:689,693`). Ground item decay 600s, herbs 60s (`general.properties:258,262`). Corpse decay 7s + 10s spoil extension (`npc.properties:86,90`). SaveDroppedItem=False (`general.properties:316`). | `GC/general.properties:258,262,316`; `GC/character.properties:689,693`; `GC/npc.properties:86,90` |
| `tax.toml` | Territory castle tax: default no-owner rate 15% (0–15% lord range; 25% Dawn, 5% Dusk). baseTax = 0. Aden/Rune skim constants. | `GJ/instancemanager/CastleManager.java:165-176`; `RequestBuyItem.java:182,234`; `BuyListData.java:119` |
| `party_loot.toml` | 5 loot modes: FINDERS_KEEPERS (0), RANDOM (1), RANDOM_INCLUDING_SPOIL (2), BY_TURN (3), BY_TURN_INCLUDING_SPOIL (4). PartyRange2 = 1400 units. PartyXpCutoffMethod = highfive (0-9: 100%, 10-14: 30%, 15-99: 0%). | `GJ/enums/PartyDistributionType.java:25-29`; `GC/character.properties:610,769-793`; `L2Party.java:591-628` |
| `storage.toml` | Private warehouse: 100 slots (120 Dwarf), deposit fee 30 adena/item. Freight: 200 slots, fee 1,000 adena/item. | `GC/character.properties:325-335`; `SendWareHouseDepositList.java:117-118`; `RequestPackageSend.java:109` |
| `mail.toml` | Max attachments 8. Base fee 100 adena + 1,000 adena/slot. Expiry 360h (15 days), COD 12h. Subject max 128, body max 512. SendMailInterval = 100 ticks (10s). Peace zone required. | `RequestSendPost.java:61-68,130,208,255`; `Message.java:39-40,98`; `GC/floodprotector.properties:106` |
| `auction.toml` | Item auction schedules and config (ItemAuctionEnabled=True). | `GC/general.properties:655` |
| `manor.toml` | Manor seed and crop definitions, purchase/reward ratios. | `DP/manor/` [UNVERIFIED: H5 manor script tables] |

### 2.5 Persistence, recovery and idempotency

Migrations start at `000012` (`architecture-decisions.md` A15; `000010`/`000011` belong to Phase 4):

- `000012_phase5_storage.sql`: `warehouse_items` (character FK, item template, count, enchant, augment, slot) and `freight_items` (account FK, destination character FK, destination town, item instance fields).
- `000013_phase5_mail.sql`: `mail_messages` (id, sender_id, receiver_id, subject varchar(128), body varchar(512), cod_adena bigint, is_read, sent_tick, expiry_tick) and `mail_attachments` (message_id FK, item instance fields).
- `000014_phase5_recipes_stores.sql`: `character_recipes` (character FK, recipe_id int, is_dwarven bool) and `private_stores` (character FK, store_type smallint, title varchar(128), items jsonb).
- `000015_phase5_economy_counters.sql`: `economy_counters` (bucket_hour timestamptz, template_id int, reason varchar(32), created_count bigint, destroyed_count bigint, adena_in bigint, adena_out bigint).

Ground items and corpses are zone snapshot state, NOT tables (A15). If a zone crashes, ground items restore from the zone snapshot.

Add `Phase5Checkpoint` (`deny_unknown_fields`) to `CharacterCheckpoint`:
```rust
struct Phase5Checkpoint {
    dwarven_recipes: Vec<u32>,
    common_recipes: Vec<u32>,
    warehouse_count: u32,
    freight_count: u32,
    active_store: Option<StoreCheckpointState>,
}
```

Outbox subjects (A15/C9 pattern):
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

API §2 idempotency keys: required client UUIDs on every mutating intent/RPC (`account_id, operation, key, fingerprint, response`), retained **24 hours** (`api-guidelines.md §2`).

| API §4 endpoint / transport matrix | Mandatory acceptance |
|---|---|
| ListBuylist gRPC | Authenticated read of NPC vendor buylist; returns items, prices, restock state. |
| VendorBuy / VendorSell / VendorRefund WS intents | Zone critical mutations; checks proximity radius 3, tax calculation, capacity; updates refund tab; returns VendorTransactionResult with frozen receipt. |
| MultisellChoose WS intent | Zone critical mutation; validates ingredient inventory; maintainEnchantment copies enchant/augment; returns ItemExchanged. |
| RegisterRecipe / CraftItem WS intents | Zone critical mutations; checks Create Item ranks; deducts MP; rolls success/masterwork; consumes materials; returns CraftResult. |
| TradeInitiate / Respond / AddItem / Confirm / Commit / Cancel WS intents | Two-phase atomic trade state machine; validates distance <= 150 units; mutual confirmation; executes as ONE critical mutation moving items between inventories. |
| SetPrivateStore / CancelPrivateStore / PrivateStoreBuy/Sell WS intents | Persistent store state in zone; checks slot limits (3/4 Dwarf 4/5); validates escrow and inventory. |
| GetWarehouse / DepositWarehouse / WithdrawWarehouse RPC + WS | Deposit/withdraw execute as zone critical mutations on inventory + container row change in same fenced transaction; 30 adena deposit fee. |
| GetFreight / DepositFreight / WithdrawFreight RPC + WS | Freight container for alt characters on same account; 1,000 adena deposit fee. |
| ListMail / ReadMail / SendMail / TakeMailAttachments RPC + WS | Send requires peace zone and tradeable unequipped items; max 8 attachments; fee 100 + 1,000/slot; 15d expiry; 12h COD. |
| GetRecipeBook gRPC | Returns registered Dwarven and Common recipes for character. |
| ListItemAuctions / GetManorData gRPC | Authenticated reads for fixture-backed auction and manor schedules. |

### 2.6 Domain rules

- **Monster drop resolution:** At monster death, killer is determined as `maxDealer`: the online player within PartyRange 1600 units who dealt the most damage (`GJ/model/actor/L2Attackable.java:341-379`). Fallback to lastAttacker only if no damage dealers are in range. Level gap `d = mobLevel - killerLevel`.
- **Level-gap window:** Grouped items (including all 2,309 adena entries, A29) use the 5/10/10% item window (`IKillerChanceModifierStrategy.java:33-43`, `GC/npc.properties:187-219`). Full chance until `d >= -5`; linear reduction to 10% floor at `d <= -10`.
- **Ground item ownership & decay:** Normal mob drops are protected for 15,000 ms (15 s) to killer and killer's party (`ItemTable.java:214`). Raid loot rights require first command channel with >= 45 members, protected for 900 s (`character.properties:689,693`); otherwise no protection. Decay: NPC drops decay in 600 s (`general.properties:258`); herbs decay in 60 s (`:262`). Player-dropped items (including PK death drops) are NOT auto-destroyed (`DestroyPlayerDroppedItem=False`, `DestroyEquipableItem=False`, `general.properties:306,312`).
- **Pickup validation:** Requires range <= 150 units, ownership protection check, and `can_accept` inventory slot and weight check.
- **Herbs:** The 68 EtcItems with `ex_immediate_effect="true"` (`L2Item.java:188,743-745`). Applied immediately on pickup without inventory slot allocation; destroyed immediately (`L2PcInstance.java:3860-3868`).
- **Spoil and sweep:** Spoil 254 success is calculated via `Formulas.calcMagicSuccess` (level-based; `Spoil.java:40-42`). On success, monster flagged `isSpoiled`. On death, `<corpse>` list rolled into `_sweepItems`. Sweeper 42 has hitTime 500 ms, reuse 500 ms, MP 3, castRange 20 (`DP/stats/skills/00000-00099.xml:1465-1480`). Spoiler or any spoiler party member may sweep (`L2Attackable.java:1027-1035`). Corpse decay is 7 s default (`npc.properties:86`) + 10 s spoil extension (`:90`).
- **Vendors and buylists:** 195 buylists (`BuyListData.java:100-117`). Buy price = `trunc(price * (1 + castleTax))` (`RequestBuyItem.java:182`). Sell price = `referencePrice / 2` (`RequestSellItem.java:142`). Refund tab holds at most 12 items (`PcRefund.java:60-64`), cleared on logout. Proximity radius 3.
- **Multisell:** Generic item-for-item exchange (`MultisellData.java`). `maintainEnchantment="true"` preserves enchant, augmentation, and elemental attributes (`MultiSellChoose.java:355-362`). `applyTaxes="true"` adds castle tax to adena ingredients (`PreparedEntry.java:49`).
- **Crafting:** Create Item 172 (10 ranks: levels 5, 20, 28, 36, 43, 49, 55, 62, 70, 82; `00100-00199.xml:1435`) and Create Common Item 1320 (9 ranks: levels 5..70; `01300-01399.xml:547`). Recipe limits: 50 Dwarven, 50 Common (`character.properties:488-489`). Recipe item consumed on registration. MP deducted at resolution. Failure loses 100% materials (`RecipeController.java:363-387`). CraftMasterwork=True enables rare products on 583 recipes.
- **Player trade:** Two-phase confirm state machine (`TradeList.java:330-450`). Lock ordering by `objectId` prevents deadlocks (`:353-365`). Both parties confirm; validation at commit; executed as ONE critical mutation moving items between zone inventories with capacity checks on both sides.
- **Private stores:** Sell, buy, package sell, manufacture. Slot limits: Dwarf 4 sell / 5 buy; Other 3 sell / 4 buy (`character.properties:306-313`). Offline trade disabled (`customs.properties:190`).
- **Mail:** DB-only mailbox. Send requires peace zone (`RequestSendPost.java:130,208`) and tradeable unequipped items (`:250`). Max 8 attachments (`:63`). Fee: 100 adena base + 1,000/slot (`:67-68`). 15-day expiry, 12h COD (`Message.java:39-40`). SendMailInterval = 100 ticks (10s; `floodprotector.properties:106`).
- **Storage:** Private warehouse: 100 slots (120 Dwarf; `character.properties:325-326`). Deposit fee: 30 adena per entry (`SendWareHouseDepositList.java:117-118`). Freight: 200 slots (`character.properties:332`), fee 1,000 adena per entry (`:335`).
- **Chaotic PK drops:** Trigger: karma > 0, PK >= 6 dying outside PvP zone (`L2PcInstance.java:4485-4550`). Base drop check: 40% (`KarmaRateDrop=40`, `rates.properties:118`). Eligible gear unequipped BEFORE each roll (`L2PcInstance.java:4531-4534`). Weapon 10%, equip 40%, item 50%, limit 10 items. 15s protection to dead player and party. Not auto-destroyed (`general.properties:306,312`).

### 2.7 Formula and rounding ledger

Production uses Q **1_000_000**, i64 stored / checked i128 intermediate (A1), rational immutable literals and ordered folds; no floats.

| Family | Pinned formula / rounding / independent vectors | Source citations |
|---|---|---|
| F1 Monster Drop Chance | Single-item group: `effective_chance = (item.chance * group.chance) / 100`.<br>Multi-item group: `sumchance = Σ (item.chance * group.chance * modifiers) / 100`. Roll `u in [0, 100_000)`. If `u < sumchance`, roll `random in [0, 100_000)` and walk items accumulating `totalChance += item_chance / sumchance` until `totalChance > random`.<br>Precise drop: if effective chance >= 100%, `amountMultiply = floor(chance / 100)` + (1 if `(chance % 100) > Rnd.nextDouble() * 100` else 0). Quantity = `Rnd.get(min, max) * amountMultiply`. | `GJ/model/drops/strategy/IGroupedItemDropCalculationStrategy.java:48-86`; `GJ/model/drops/GroupedGeneralDropItem.java:193-215`; `GC/general.properties:341-350` |
| F2 Level-Gap Modifier | Level diff `d = mobLevel - killerLevel` (killer = `maxDealer`).<br>Normal items & grouped adena: `minDiff = 5, maxDiff = 10, minGap = 10%`.<br>If `d >= -5`: multiplier = 1.0.<br>If `d <= -10`: multiplier = 0.10.<br>If `-10 < d < -5`: `linear_map(d, -10, -5, 10, 100) / 100`.<br>Raid boss: `clamp((d * 0.15) + 1, 0, 1)`; raid 7+ levels below killer drops nothing. | `GJ/model/drops/strategy/IKillerChanceModifierStrategy.java:33-53`; `GC/npc.properties:187-219`; `GJ/util/Util.java:785-788` |
| F3 Spoil Success | Magic success check: `calcMagicSuccess` level-based calculation. Fails with ALREADY_SPOILED if corpse is already spoiled. Hate effectPoint: -138 to -646. | `DPJ/handlers/effecthandlers/instant/Spoil.java:40-42,57-60`; `DP/stats/skills/00200-00299.xml:1342-1364` |
| F4 Vendor Pricing & Tax | Buy price: `price = trunc(product.price * (1 + baseTaxRate + castleTaxRate))`. baseTaxRate = 0. castleTaxRate = 15% (no-owner rate). Integer per-mille arithmetic: `(price * (1000 + tax_permille)) / 1000`.<br>Castle treasury skim: `subTotal * castleTaxRate`.<br>Sell price: `item.referencePrice / 2`. | `GJ/network/clientpackets/RequestBuyItem.java:151,181-182,234`; `GJ/data/xml/impl/BuyListData.java:119`; `GJ/network/clientpackets/RequestSellItem.java:142`; `CastleManager.java:165-176` |
| F5 Crafting Roll & Failure | Roll `roll in [0, 100)`. If `roll < successRate`: SUCCESS, produce product.<br>If `roll >= successRate`: FAILURE, all materials consumed, 0 refund.<br>Masterwork roll (if success and `productionRare` defined): roll `mw_roll in [0, 100)`: if `< rareChance`, produce rare product.<br>MP cost deducted at resolution. | `GJ/RecipeController.java:350-387`; `DP/recipes.xml` |
| F6 Party Adena Split | In-range party members within `PartyRange2 = 1400` units of picker.<br>`member_count = in_range.len()`.<br>`base_share = total_adena / member_count`.<br>`remainder = total_adena % member_count`.<br>Remainder distributed 1-by-1 to random members: `keys.get(Rnd.get(keys.size()))`. | `GJ/model/L2Party.java:510-512,545-547,591-628`; `GC/character.properties:610` |
| F7 Ground & Corpse Decay | Ground item decay: `600 s` (6,000 ticks) for NPC drops.<br>Herb decay: `60 s` (600 ticks).<br>Player-dropped items: NOT auto-destroyed (`SaveDroppedItem=False`).<br>Corpse decay: `7 s` (70 ticks) default + `10 s` (100 ticks) spoil extension = 17 s (170 ticks). Overridden by per-NPC `corpseTime` if present. | `GC/general.properties:258,262,306,312,316`; `GC/npc.properties:86,90`; `GJ/taskmanager/DecayTaskManager.java:54-63`; `L2NpcTemplate.java:139` |
| F8 Fees & Limits | Private warehouse deposit fee: `30 adena` per item entry.<br>Freight fee: `1,000 adena` per item entry.<br>Mail fee: `100 adena` base + `1,000 adena` per attachment slot.<br>Max adena cap: `99,900,000,000` (99.9B). Stackables: `Integer.MAX_VALUE`. | `SendWareHouseDepositList.java:117-118`; `GC/character.properties:335`; `RequestSendPost.java:67-68,255`; `GC/character.properties:671` |
| F9 Chaotic PK Drops | Trigger: `karma > 0 && pkKills >= 6` outside PvP zone.<br>Check: `Rnd.get(100) < 40%`.<br>For each eligible equipped item: unequipped BEFORE roll. Roll weapon 10%, other equipped 40%, inventory items 50%.<br>Drop limit: max 10 items.<br>Protection: 15s to dead player and party. Not auto-destroyed. | `GJ/model/actor/instance/L2PcInstance.java:4485-4552`; `GC/rates.properties:116-128`; `DropProtection.java` |
| F10 Rate Multiplier Adaptation | `effective_chance = min(base_chance * multiplier, max_chance)`. Computed as exact rational `(base_chance_num * mult_num) / (base_chance_den * mult_den)` before conversion to integer draw comparison. | Owner decision P2; `architecture-decisions.md` A5 |

**Research numeric corrections (every discovered disagreement with pinned sources):**

| 05 location / claim | Pinned correction / authority |
|---|---|
| §2.6 Adena faucets: "mob drops and quest rewards only"; vendor spread is a sink | NPC sell **mints** adena (`addAdena("Sell", totalPrice...)`), so NPC sell is a faucet. Only NPC buy is a sink. `GJ/network/clientpackets/RequestSellItem.java:142,155`. |
| §2.6 Castle tax: "15% Dawn / 5% Dusk" | Dusk 5%, Dawn 25%, no owner 15% (`validateTaxes`). `GJ/instancemanager/CastleManager.java:165-176`. |
| §2.1/§4.1 Drop XML files: "124 XML files with `<death><group>` and `<corpse>`" | 124 XML files exist, but only **59** contain `<death>` and **52** contain `<corpse>`. `<corpse>` contains **only ungrouped** `<item>` rows (0 groups in any corpse list). `DP/stats/npcs/*.xml`; `GJ/data/xml/impl/NpcData.java:455-467`. |
| §3.2 Adena level gap: "adena window 8→15 applied to adena" | All 2,309 adena entries in the pinned NPC XML are inside a `<group>`, so DEFAULT_STRATEGY applies the **item window 5/10/10%**, not 8/15. `GJ/model/drops/strategy/IKillerChanceModifierStrategy.java:33-53`; `GroupedGeneralDropItem.java:50-51`. |
| §3.2 Killer identity: "last hitter / killer" | Drop killer is **maxDealer**: the online player with top damage within PartyRange 1600. Fallback to lastAttacker only if no damage dealers in range. `GJ/model/actor/L2Attackable.java:341-379`. |
| §2.2 Chaotic drops: "unequips before ground drop" | Every eligible equipped item is **unequipped BEFORE its roll**, so PK death strips all eligible gear even when nothing drops. `GJ/model/actor/instance/L2PcInstance.java:4531-4541`. |
| §2.3 Spoil power: "power -138 to -646" | These are `effectPoint` (hate/aggro) values, not power. `DP/stats/skills/00200-00299.xml:1345`. |
| §2.3 Spoil resistance: "MEN-resisted debuff" | Base Spoil success = `Formulas.calcMagicSuccess` (level-based). MEN `basicProperty` applies only to enchant routes. `DPJ/handlers/effecthandlers/instant/Spoil.java:40-42`. |
| §2.3 Sweeper 42: "instant cast" | Level 1, but **hitTime 500 ms, reuse 500 ms, MP 3, castRange 20, target NPC_BODY**. Only the effect is instant. `DP/stats/skills/00000-00099.xml:1465-1480`. |
| §2.1 Herbs: "immediate_effect='true'" | Herbs are flagged **`ex_immediate_effect="true"`** (68 items). `immediate_effect` is a different flag set on 16,877 items. `GJ/model/items/L2Item.java:187-188,743-745`. |
| §2.4 Party loot slot check: "must pass validateCapacityByItemId" | `validateCapacityByItemId` **always returns true** (inner result discarded), so only the PartyRange2 range check is effective. `GJ/model/itemcontainer/PcInventory.java:759-765`; `L2Party.java:159-219`. |
| §2.4 Raid loot rights: "top damage CC/party leader, 15 min" | Owner = leader of **first command channel that attacked**, and ONLY if CC members >= `RaidLootRightsCCSize=45`. Otherwise raid loot has **no ownership protection**. `ItemTable.java:203-211`; `L2Attackable.java:234-238`; `GC/character.properties:693`. |
| §2.3 Spoil sweep looter: "goes to spoiler unless including-spoil mode" | Sweep loot goes to the **sweeper** (`getActualLooter` returns sweeper). Any member of the spoiler's party may sweep. `L2Party.java:197-219`; `Sweeper.java:64-65`. |
| §2.7 Refund tab: "general.properties:18" | AllowRefund=True is at `:458`. The refund buffer is **capped at 12 items** (oldest destroyed) and cleared on logout. `GC/general.properties:458`; `PcRefund.java:60-64`. |
| §2.5 Gemstones in recipes: "D for C/B gear, B for A/S gear" | That is the **augmentation** rule. Recipes follow product grade: D uses D/C gems, C uses C/B, B uses B, A uses A, S uses S. `DP/recipes.xml`; `AbstractRefinePacket.java:379-385`. |
| §2.5 Recipe crystal costs: "310 C-crystals, ~1,300 B-crystals" | 310 C is correct (Pata). Maximum B-crystal count in any recipe is **421**; per-grade maxima are D 700, C 534, B 421, A 480, S 622. `DP/recipes.xml`. |
| §2.8 Mail rate limit: "1/min" | Pinned is `SendMailInterval = 100` game ticks ≈ **10 s** ("10s on retail"). `GC/floodprotector.properties:105-106`. |
| §2.4 Ground decay: "600s item / 60s herb" | NPC drops decay at 600s, but player-dropped items (incl. PK death drops) are **never auto-destroyed by default** (`DestroyPlayerDroppedItem=False`, `DestroyEquipableItem=False`). `GC/general.properties:258,306,312`. |
| Warehouse deposit fee | Pinned is **30 adena per item entry** (`SendWareHouseDepositList.java:117-118`). |
| Mail peace zone rule | Attachments require a **PEACE zone** (`RequestSendPost.java:130,208`), equipped or non-tradeable items cannot be attached (`:250`), subject max 128, body max 512 (`:61-62`). |
| Multisell maintainEnchantment | Also copies augmentation and elemental attributes (`MultiSellChoose.java:355-362`). |

### 2.8 Canonical scenario matrix

Files live under `apps/client-unreal/Scenarios/`. Middle cells give the **exact entire header** payload including `# covers:`; add `# fixture: phase5-<family>` next. No story tag is satisfied by a header alone. C11 asserts only received state; `export` means verified C14 replay evidence checked by C15 economy oracle on actual inputs. First ship **5-ground-item-drop** and **5-vendor-buy-sell**.

| File | Exact header | Authoritative assertions / fixture family |
|---|---|---|
| `5-ground-item-drop.nfs` | `# covers: 5 E1.1, 5 E1.6, 5 E2.1, 5 E2.9, 5 E4.1, 5 E4.6, 5 E5.1, 5 E5.6` | phase5-drops: NPC death spawns ground item with allocated ItemInstanceId and ledger op `drop`; 15s protection to killer; non-owner PickUp rejected with `REJECT_REASON_GROUND_ITEM_PROTECTED`; owner PickUp succeeds, item added to inventory with ledger op `pickup`, GroundItemDespawn observed. |
| `5-ground-item-decay.nfs` | `# covers: 5 E1.3, 5 E1.6, 5 E2.1, 5 E2.9, 5 E4.1, 5 E4.6, 5 E5.1, 5 E5.6` | phase5-decay: NPC drop ground item expires at 600s; herb expires at 60s; GroundItemDespawn emitted with reason decay. Player-dropped item persists past 600s without decay (`SaveDroppedItem=False`). |
| `5-herb-pickup.nfs` | `# covers: 5 E1.1, 5 E1.6, 5 E2.1, 5 E2.9, 5 E4.1, 5 E4.6, 5 E5.1, 5 E5.6` | phase5-herbs: Herb (flag `ex_immediate_effect`) picked up from ground; immediate effect applied without allocating inventory slot; ground item destroyed immediately; EffectApplied observed, inventory count unchanged. |
| `5-spoil-sweep.nfs` | `# covers: 5 E1.1, 5 E1.6, 5 E2.2, 5 E2.9, 5 E4.1, 5 E4.6, 5 E5.1, 5 E5.6` | phase5-spoil: Dwarf casts Spoil 254 (`calcMagicSuccess`); corpse flagged spoiled; corpse decay timer extended by 10s (total 17s); Dwarf or party member casts Sweeper 42 (500ms hit, 3 MP); items transferred to sweeper via `sweep` ledger op; corpse cleared. |
| `5-party-loot-modes.nfs` | `# covers: 5 E1.3, 5 E1.6, 5 E2.3, 5 E2.9, 5 E4.2, 5 E4.6, 5 E5.2, 5 E5.6` | phase5-party: 3-member party tests FINDERS_KEEPERS, RANDOM, BY_TURN modes. Adena drop split evenly among members within 1400 units; remainder 1-by-1 randomly distributed; item assigned per mode with capacity check. |
| `5-vendor-buy-sell.nfs` | `# covers: 5 E1.2, 5 E1.6, 5 E2.4, 5 E2.9, 5 E3.2, 5 E3.5, 5 E4.2, 5 E4.6, 5 E5.2, 5 E5.6` | phase5-vendors: NPC buylist buy at `trunc(price * (1 + 0.15))`; sell item at `referencePrice / 2` with adena minted; refund tab displays up to 12 sold items; buy back from refund at sell price; refund overflow drops oldest item. |
| `5-multisell.nfs` | `# covers: 5 E1.2, 5 E1.6, 5 E2.4, 5 E2.9, 5 E3.2, 5 E3.5, 5 E4.2, 5 E4.6, 5 E5.2, 5 E5.6` | phase5-multisell: Multisell exchange consumes exact ingredients; product granted; maintainEnchantment preserves enchant, augmentation, and elemental attributes; castle tax added to adena ingredients when `applyTaxes="true"`. |
| `5-craft-dwarven.nfs` | `# covers: 5 E1.2, 5 E1.6, 5 E2.5, 5 E2.9, 5 E3.3, 5 E3.5, 5 E4.3, 5 E4.6, 5 E5.3, 5 E5.6` | phase5-craft: Register Dwarven recipe into recipe book (recipe item consumed, 50-limit enforced); Create Item rank gates recipe level; craft execution deducts MP at resolution; roll success (60% recipe); success grants product; failure consumes all materials with 0 refund; masterwork roll verified on eligible recipe. |
| `5-craft-common.nfs` | `# covers: 5 E1.2, 5 E1.6, 5 E2.5, 5 E2.9, 5 E3.3, 5 E3.5, 5 E4.3, 5 E4.6, 5 E5.3, 5 E5.6` | phase5-craft-common: Non-Dwarf character registers recipe into Common recipe book (50 limit); Create Common Item 1320 rank gates craft; crafts common product. |
| `5-player-trade.nfs` | `# covers: 5 E1.3, 5 E1.6, 5 E2.6, 5 E2.9, 5 E3.2, 5 E3.5, 5 E4.4, 5 E4.6, 5 E5.3, 5 E5.6` | phase5-trade: 2 players within 150 units initiate trade; add items and adena; lock offers; both confirm; atomic commit moves items between inventories in single critical mutation; capacity checked; cancel resets offers. |
| `5-private-store-sell.nfs` | `# covers: 5 E1.3, 5 E1.6, 5 E2.6, 5 E2.9, 5 E3.2, 5 E3.5, 5 E4.3, 5 E4.6, 5 E5.3, 5 E5.6` | phase5-stores: Player opens private sell store (3 slots / Dwarf 4); buyer browses and purchases item; adena transferred to seller, item transferred to buyer; store updates remaining count; closes when empty. |
| `5-private-store-buy.nfs` | `# covers: 5 E1.3, 5 E1.6, 5 E2.6, 5 E2.9, 5 E3.2, 5 E3.5, 5 E4.3, 5 E4.6, 5 E5.3, 5 E5.6` | phase5-stores: Player opens private buy store (4 slots / Dwarf 5) with adena escrow locked; seller sells requested item; item transferred to buyer, adena transferred to seller. |
| `5-private-manufacture.nfs` | `# covers: 5 E1.2, 5 E1.6, 5 E2.5, 5 E2.6, 5 E2.9, 5 E3.3, 5 E3.5, 5 E4.3, 5 E4.6, 5 E5.3, 5 E5.6` | phase5-manufacture: Dwarf opens private manufacture workshop setting recipes and fees; customer provides materials and pays fee; fee transferred before roll; craft roll resolved on Dwarf skill; product to customer on success; fee retained on failure. |
| `5-warehouse.nfs` | `# covers: 5 E1.3, 5 E1.6, 5 E2.7, 5 E2.9, 5 E3.2, 5 E3.5, 5 E4.4, 5 E4.6, 5 E5.4, 5 E5.6` | phase5-warehouse: Deposit items into private warehouse; fee 30 adena per entry deducted; slot limit 100 (120 Dwarf); withdraw item back to inventory; DB row and inventory updated in same fenced transaction. |
| `5-freight.nfs` | `# covers: 5 E1.3, 5 E1.6, 5 E2.7, 5 E2.9, 5 E3.2, 5 E3.5, 5 E4.4, 5 E4.6, 5 E5.4, 5 E5.6` | phase5-freight: Deposit item to freight directed to character on same account; fee 1,000 adena deducted; destination character withdraws item at destination town warehouse. |
| `5-mail-attachments.nfs` | `# covers: 5 E1.3, 5 E1.6, 5 E2.7, 5 E2.9, 5 E3.4, 5 E3.5, 5 E4.4, 5 E4.6, 5 E5.4, 5 E5.6` | phase5-mail: Send mail in peace zone with up to 8 attachments and COD adena; fee 100 + 1,000/slot deducted; receiver receives notification; pays COD adena; attachments transferred to receiver inventory. |
| `5-chaotic-pk-drop.nfs` | `# covers: 5 E1.3, 5 E1.6, 5 E2.2, 5 E2.9, 5 E4.1, 5 E4.6, 5 E5.1, 5 E5.6` | phase5-pk-drop: Chaotic player (karma > 0, PK >= 6) killed in non-PvP zone; all eligible equipped items unequipped before roll; 40% base drop check; rolls per slot (weapon 10%, equip 40%, inventory 50%); dropped items protected for 15s to dead player/party; player drops not auto-destroyed. |
| `5-rate-multiplier.nfs` | `# covers: 5 E1.3, 5 E1.5, 5 E1.6, 5 E2.2, 5 E2.9, 5 E5.1, 5 E5.6` | phase5-rates: Monster killed under base 1x rate multiplier vs boosted 2x/3x multiplier; scenario asserts both base and multiplied expectations; evidence gates verify exact rational scaling. |

## 3. Engineering architecture, ownership and review gates

Follow domain → application ports → infrastructure/interface separation. Add `economy.rs`, `ground_item.rs`, `corpse.rs`, `vendor.rs`, `multisell.rs`, `craft.rs`, `trade.rs`, `store.rs`, `storage.rs`, `mail.rs` to zone engine. API adds storage/mail/store persistence repositories, outbox handlers, and economy counters rollups. Register every production zone source in SOURCES and retain denied unwrap/expect/panic, floats, HashMap and wall-clock checks.

- **Z = Claude (opus subagent):** zone economy engine (E2), ground items, drops, spoil/sweep, vendors, multisell, craft, trade, storage/mail movement, V7 boundary, replay codec, export producer.
- **C = Claude (opus subagent):** API/persistence/RPCs (E3), Unreal client & bot (E4), harness/scenarios/soak (E5), telemetry integration (E6), independent review of Z and D.
- **D = agy (gemini-3.8-flash):** data generation scripts and tables-phase5 drafting (E1), diagrams, operational docs; **every D output is reviewed and verified by Claude against pinned sources**. No unverified D deliverables.
- Root integrates reviewed commits, runs gates, merges/pushes and owns the lock for **all** Unreal build/run/cook activity. Author never solely reviews own work.

## 4. Epics, stories, models and Done When

All stories are pending. Future completion prefix is `✅` in the title cell after independent review/evidence, never a documentation claim. Each client-facing epic ends with a simulation story naming its files; §2.8 supplies exact coverage/assertions. Model letters follow §3.

### Epic 1: Data generation and independent oracle (D generates; C/Z review & verify)

| Story | Model | Work / Done When |
|---|---|---|
| 1.1 Monster drops, spoil, and herbs data generation [required] | D | Generate `drops.toml` (59 death XMLs, 2,938 groups, 307 ungrouped items), `spoil.toml` (52 corpse XMLs, ungrouped), and `herbs.toml` (68 `ex_immediate_effect` items) with provenance headers and `--check` / **Done When:** Claude verifies all XML counts, group normalizations, rate/chance fields, and herb flags against pinned clones; `check_all.py` passes; zero unverified values. |
| 1.2 Buylists, multisell, and recipes data generation [required] | D | Generate `buylists.toml` (195 lists with restock), `multisell.toml` (196 lists with maintainEnchantment/applyTaxes), and `recipes.toml` (1,000 recipes: 830 Dwarven, 170 Common, 583 masterwork) / **Done When:** Claude verifies file counts, recipe tier/success/MP fields, multisell ingredients/products, and buylist prices against pinned clones; `check_all.py` passes. |
| 1.3 Economy configuration and rates data generation [required] | D | Generate `rates.toml` (global multiplier default 1x, death/corpse/raid multipliers, champion x8), `ownership_decay.toml` (15s/900s protection, 600s/60s decay, 7s+10s corpse), `tax.toml` (15% no-owner castle tax), `party_loot.toml` (5 modes, 1400 units, highfive XP cutoff), `storage.toml` (WH 100/120 @ 30 adena, freight 200 @ 1,000 adena), `mail.toml` (8 attachments, 100+1000 fee, 15d expiry, 12h COD, 10s flood interval), `auction.toml`, `manor.toml` / **Done When:** Claude verifies all config literals against pinned `.properties` files; TOML schema validation passes. |
| 1.4 Independent economy oracle (Fraction) [required] | C | Build independent `packages/data/scripts/economy_oracle.py` using Python Fraction for exact drop chance resolution (precise algorithm, single/multi group collapse), level gap penalty (5/10/10% window, raid 7-level cutoff), vendor buy tax rounding, sell price halving, craft success roll, adena party split with remainder, and decay tick conversion / **Done When:** Oracle passes standalone test suite covering all F1–F10 formula vectors; zero float conversions; Claude reviews. |
| 1.5 Rate multiplier and level-gap verification suite [required] | C | Author oracle verification test suite asserting drop/spoil chance scaling across 1x, 2x, 3x multipliers and level gaps d = -4, -5, -7, -10, -12 / **Done When:** Oracle proves exact rational scaling and deep-blue clamp behavior; outputs match pinned Java strategy vectors. |
| 1.6 Simulation scenario(s) [client-visible] [required] | C | Supply oracle assertions for 5-ground-item-drop.nfs, 5-ground-item-decay.nfs, 5-herb-pickup.nfs, 5-spoil-sweep.nfs, 5-party-loot-modes.nfs, 5-vendor-buy-sell.nfs, 5-multisell.nfs, 5-craft-dwarven.nfs, 5-craft-common.nfs, 5-player-trade.nfs, 5-private-store-sell.nfs, 5-private-store-buy.nfs, 5-private-manufacture.nfs, 5-warehouse.nfs, 5-freight.nfs, 5-mail-attachments.nfs, 5-chaotic-pk-drop.nfs, 5-rate-multiplier.nfs / **Done When:** §2.8 mapped received assertions and actual-input oracle gates pass; foundation handoff precedes live engine completion. |

### Epic 2: Zone economy engine, V7 boundary, and replay (Z; C review)

| Story | Model | Work / Done When |
|---|---|---|
| 2.1 Ground items, decay, and PickUp intent resolution [client-visible] [required] | Z | Implement zone-canonical `ground_items` map, GroundItemId allocator, GroundItemSpawn/Despawn events, 15s protection timer to killer/party, 600s item / 60s herb decay, player-drop non-decay rule, and PickUp intent with range <= 150 and `can_accept` capacity check / **Done When:** Ground item lifecycle, protection refusal, and pickup ledger op `pickup` pass; verified against 5-ground-item-drop.nfs and 5-ground-item-decay.nfs. |
| 2.2 Monster death drop lists, killer attribution, and spoil/sweep [client-visible] [required] | Z | Implement killer attribution to `maxDealer` within 1600 units; level-gap window 5/10/10% on grouped drops and adena; precise-drop roll; corpse `_sweepItems` roll; Spoil 254 `calcMagicSuccess` impact hook; Sweeper 42 (500ms hit, 3 MP) execution; 7s default + 10s spoil extension corpse decay; chaotic PK drops (unequip before roll, 40% base, limit 10) / **Done When:** Drops spawn on ground with killer ownership; spoil/sweep transfers items to sweeper via `sweep` op; chaotic PK death strips gear and drops items; 5-spoil-sweep.nfs and 5-chaotic-pk-drop.nfs pass. |
| 2.3 Party loot distribution modes and adena split [client-visible] [required] | Z | Implement 5 party loot modes (FINDERS_KEEPERS, RANDOM, RANDOM_INCLUDING_SPOIL, BY_TURN, BY_TURN_INCLUDING_SPOIL) and adena even split among party members in PartyRange2 (1400 units) with 1-by-1 random remainder distribution / **Done When:** Loot assignment follows active mode; adena splits evenly; party loot notifications emitted; 5-party-loot-modes.nfs passes. |
| 2.4 NPC vendors, buylists, castle tax, and multisell exchange [client-visible] [required] | Z | Implement NPC proximity radius 3; buylist purchase with 15% castle tax; sell-back at `referencePrice / 2` minting adena; refund tab (max 12 items, FIFO drop on overflow, cleared on logout); multisell exchange engine with `maintainEnchantment` copying enchant + augment + elementals and `applyTaxes` / **Done When:** Vendor buy/sell/refund and multisell transactions execute with receipts; 5-vendor-buy-sell.nfs and 5-multisell.nfs pass. |
| 2.5 Dwarven and Common crafting engine and recipe books [client-visible] [required] | Z | Implement Dwarven (max 50) and Common (max 50) recipe books; recipe registration consuming item; Create Item rank gates; MP deduction at resolution; success roll (100% loss on failure); masterwork roll on 583 recipes; private manufacture workshop / **Done When:** Crafting resolves with exact material loss/product grant; recipe books enforce limits; 5-craft-dwarven.nfs, 5-craft-common.nfs, and 5-private-manufacture.nfs pass. |
| 2.6 Player-to-player trade and private stores [client-visible] [required] | Z | Implement two-phase atomic trade state machine (`TradeList.java:330-450`, lock ordering, distance <= 150, capacity checks on both sides, single critical mutation); private sell/buy/package stores with slot limits 3/4 (Dwarf 4/5) and escrow / **Done When:** Trade executes atomically without item duplication or loss; private stores execute purchases/sales; 5-player-trade.nfs, 5-private-store-sell.nfs, and 5-private-store-buy.nfs pass. |
| 2.7 Storage and mail zone-mediated item movement [client-visible] [required] | Z | Implement zone-mediated item/currency movement for private warehouse (100/120 Dwarf, 30 adena fee) and freight (200, 1,000 adena fee); mail send attachment debit (peace zone requirement) and receive attachment credit / **Done When:** Storage and mail item transfers execute as zone critical mutations with ledger ops `deposit`, `withdraw`, `mailed`; 5-warehouse.nfs, 5-freight.nfs, and 5-mail-attachments.nfs pass. |
| 2.8 V7 snapshot, digest, record, and replay evidence [required] | Z | Implement Snapshot 11, Record 9, Binary V7 codec; Phase5Rules content hashing; digest divergence on ground items/corpses/economy state; nightfall-replay export extended with drop/spoil/craft/vendor/trade/mail facts / **Done When:** Legacy V6 and older replay byte-exact via `replay-check.sh`; V7 replay bit-exact; C14 export facts verified by oracle. |
| 2.9 Simulation scenario(s) [client-visible] [required] | Z | Author engine assertions in 5-ground-item-drop.nfs, 5-ground-item-decay.nfs, 5-herb-pickup.nfs, 5-spoil-sweep.nfs, 5-party-loot-modes.nfs, 5-vendor-buy-sell.nfs, 5-multisell.nfs, 5-craft-dwarven.nfs, 5-craft-common.nfs, 5-player-trade.nfs, 5-private-store-sell.nfs, 5-private-store-buy.nfs, 5-private-manufacture.nfs, 5-warehouse.nfs, 5-freight.nfs, 5-mail-attachments.nfs, 5-chaotic-pk-drop.nfs, 5-rate-multiplier.nfs / **Done When:** §2.8 cases pass; C reviews Z outputs. |

### Epic 3: API, persistence, and RPCs (C; Z review)

| Story | Model | Work / Done When |
|---|---|---|
| 3.1 Migrations 000012–000015, storage/mail/store tables, and Phase5Checkpoint [required] | C | Implement SeaORM migrations `000012_phase5_storage.sql`, `000013_phase5_mail.sql`, `000014_phase5_recipes_stores.sql`, `000015_phase5_economy_counters.sql`; add `Phase5Checkpoint` to `CharacterCheckpoint`; regenerate entities via `entities.sh --check` / **Done When:** Schema migrations apply cleanly, roll back, and support concurrent transactions; Phase5Checkpoint serialization passes round-trip tests. |
| 3.2 Additive proto contracts, GameService RPCs, and vendor/storage endpoints [client-visible] [required] | C | Freeze additive proto contracts in `economy.proto`, `world.proto` (intents 29+, WorldEvent 34+, RejectReason 38+), and `game.proto`; implement `ListBuylist`, `GetWarehouse`, `GetFreight` RPCs and WS adapter handlers / **Done When:** Proto buf lint and breaking change checks pass; full API §4 endpoint matrix for buylist and storage passes; Z reviews. |
| 3.3 Crafting and recipe book persistence and RPCs [client-visible] [required] | C | Implement `GetRecipeBook` RPC, recipe book persistence in `character_recipes`, and private manufacture store adapters / **Done When:** Recipe book queries return registered recipes; Create Item ranks validated; recipe registration and crafting API §4 matrix passes. |
| 3.4 Mail service, outbox publishing, and economy counters [client-visible] [required] | C | Implement `ListMail`, `ReadMail`, `TakeMailAttachments`, `SendMail` RPCs; peace-zone attachment validation; outbox relay publishing for `nightfall.item.*`; economy counters hourly aggregation rollup worker / **Done When:** Mail workflows (send, COD, receive, expire) pass; outbox facts published; `economy_counters` table aggregates faucets and sinks accurately. |
| 3.5 Simulation scenario(s) [client-visible] [required] | C | Author 5-vendor-buy-sell.nfs, 5-multisell.nfs, 5-craft-dwarven.nfs, 5-craft-common.nfs, 5-player-trade.nfs, 5-private-store-sell.nfs, 5-private-store-buy.nfs, 5-warehouse.nfs, 5-freight.nfs, 5-mail-attachments.nfs API assertions and `.allow` sidecars / **Done When:** §2.8 live wire assertions and adapter retry/crash cases pass independently reviewed. |

### Epic 4: Unreal client and bot DSL (C; Z review)

| Story | Model | Work / Done When |
|---|---|---|
| 4.1 Ground item rendering, interaction, and pickup UI [client-visible] [required] | C | Render ground items (mesh/icon placeholder, stack count, protection badge); handle click/interaction; submit PickUp intent; display loot protection feedback / **Done When:** Dropped items visible in Unreal world; picking up item triggers animation and updates inventory; herb pickup displays immediate buff; 5-ground-item-drop.nfs and 5-herb-pickup.nfs pass. |
| 4.2 Vendor, multisell, and party loot dialogs [client-visible] [required] | C | Code-built vendor window (buy, sell, refund tabs with tax display); multisell exchange dialog; party loot mode configuration UI for party leader / **Done When:** Vendor buy/sell/refund interactions submit valid intents and update local state; multisell preview shows maintainEnchantment; 5-vendor-buy-sell.nfs and 5-party-loot-modes.nfs pass. |
| 4.3 Crafting window, recipe book, and private store UI [client-visible] [required] | C | Recipe book UI (Dwarven and Common tabs); crafting progress and result dialog; private store setup (sell, buy, manufacture) and browsing UI / **Done When:** Recipes displayed by tier; craft button submits intent and renders result; private stores render titles and permit transactions; 5-craft-dwarven.nfs and 5-private-store-sell.nfs pass. |
| 4.4 Trade window, warehouse, freight, and mailbox UI [client-visible] [required] | C | Two-party trade window (offer slots, lock, confirm buttons); private warehouse dialog (deposit/withdraw); freight dialog with alt selection; mailbox UI (inbox, read, compose with attachments, COD) / **Done When:** Direct trade flow completes visually; warehouse/freight deposit/withdraw updates inventory; mail send/receive functional; 5-player-trade.nfs, 5-warehouse.nfs, and 5-mail-attachments.nfs pass. |
| 4.5 Bot economy DSL and received-state predicates [client-visible] [required] | C | Implement bot commands `nf.PickUp`, `nf.Buy`, `nf.Sell`, `nf.Refund`, `nf.Multisell`, `nf.Craft`, `nf.Trade*`, `nf.Store*`, `nf.Deposit`, `nf.Withdraw`, `nf.Mail*`, `nf.SetLootMode`; implement received-state predicates `ground_item_exists`, `adena_count`, `recipe_registered`, `in_trade`, `store_open`, `mail_count` / **Done When:** Bot scripts can execute full economy scenarios; predicates evaluate strictly from received wire messages (C11). |
| 4.6 Simulation scenario(s) [client-visible] [required] | C | Run 5-ground-item-drop.nfs, 5-herb-pickup.nfs, 5-spoil-sweep.nfs, 5-party-loot-modes.nfs, 5-vendor-buy-sell.nfs, 5-multisell.nfs, 5-craft-dwarven.nfs, 5-craft-common.nfs, 5-player-trade.nfs, 5-private-store-sell.nfs, 5-private-store-buy.nfs, 5-private-manufacture.nfs, 5-warehouse.nfs, 5-freight.nfs, 5-mail-attachments.nfs, 5-chaotic-pk-drop.nfs / **Done When:** Unreal client test automation passes with zero skips; asset cooking completes cleanly. |

### Epic 5: Harness, scenarios, and soak (C; Z review)

| Story | Model | Work / Done When |
|---|---|---|
| 5.1 Ground item, drop, spoil, and herb scenario suite [client-visible] [required] | C | Implement `phase5-drops`, `phase5-decay`, `phase5-herbs`, `phase5-spoil`, `phase5-pk-drop`, `phase5-rates` fixture packs and execute scenarios 5-ground-item-drop.nfs, 5-ground-item-decay.nfs, 5-herb-pickup.nfs, 5-spoil-sweep.nfs, 5-chaotic-pk-drop.nfs, 5-rate-multiplier.nfs / **Done When:** All drop, spoil, herb, decay, and PK drop scenarios pass; verified by export and oracle. |
| 5.2 Party loot, vendor, and multisell scenario suite [client-visible] [required] | C | Implement `phase5-party`, `phase5-vendors`, `phase5-multisell` fixture packs and execute scenarios 5-party-loot-modes.nfs, 5-vendor-buy-sell.nfs, 5-multisell.nfs / **Done When:** All party distribution, vendor transaction, refund, and multisell exchange scenarios pass. |
| 5.3 Crafting, trade, and store scenario suite [client-visible] [required] | C | Implement `phase5-craft`, `phase5-craft-common`, `phase5-trade`, `phase5-stores`, `phase5-manufacture` fixture packs and execute scenarios 5-craft-dwarven.nfs, 5-craft-common.nfs, 5-player-trade.nfs, 5-private-store-sell.nfs, 5-private-store-buy.nfs, 5-private-manufacture.nfs / **Done When:** All crafting, trade, and private store scenarios pass with exact material conservation. |
| 5.4 Storage and mail scenario suite [client-visible] [required] | C | Implement `phase5-warehouse`, `phase5-freight`, `phase5-mail` fixture packs and execute scenarios 5-warehouse.nfs, 5-freight.nfs, 5-mail-attachments.nfs / **Done When:** Warehouse deposit/withdraw, freight transfer, and mail send/receive/COD scenarios pass. |
| 5.5 Packaged 8-client soak with economy traffic and 1k-NPC drop bench [required] | C | Run 8-client packaged soak including drops, pickups, vendor buy/sell, trade, and craft traffic alongside combat; run 1k-NPC death-with-drops benchmark / **Done When:** 8-client soak maintains tick p99 < 20 ms; 1k-NPC death bench meets throughput budget (> 10,000 kills/s on one core); zero memory leaks or unhandled drops. |
| 5.6 Simulation scenario(s) [client-visible] [required] | C | Execute all 18 §2.8 scenario files with JUnit reports, replay recordings, and export verification / **Done When:** Every scenario passes; zero skips; no UNKNOWN allow-listing. |

### Epic 6: Documentation, telemetry, and fraud observability (C/D; independent review)

| Story | Model | Work / Done When |
|---|---|---|
| 6.1 Economy metrics, counters, and Grafana dashboard [required] | C | Instrument economy metrics (drops, pickups, vendor buy/sell, craft success/fail, trade volume, fees) with bounded labels; build Grafana dashboard / **Done When:** Metrics scrape cleanly; dashboards visualize faucets vs sinks in real time; `verify-observability.sh` updated and passing. |
| 6.2 Anti-exploit fraud alerts and telemetry validation [required] | C | Implement telemetry alerts for adena velocity spikes, extreme trade asymmetry (> 10x value mismatch), and multi-account freight transfers / **Done When:** Synthetic exploit test triggers alert; normal gameplay produces zero false positives. |
| 6.3 Sequence diagrams and operational documentation [required] | D | Document end-to-end sequences for ground item lifecycle, vendor transaction, two-phase trade, crafting, mail attachment escrow, and warehouse storage; update operator runbooks / **Done When:** Claude verifies diagrams match implemented contracts; documentation passes review. |
| 6.4 Outcome and limitations template [required] | D | Fill §9 outcome template only after root acceptance of Phase 5 / **Done When:** Preserved commit SHAs, test receipts, performance measurements, and architect dispositions recorded. |

## 5. Required acceptance evidence and fixtures

**Planned, not executed.** Preserve all Phase 1–4 gates and test suites. This phase adds economy contracts without weakening any existing requirement.

### 5.1 Data, oracle and formula evidence

Independent `packages/data/scripts/economy_oracle.py` uses Python Fraction over pinned literals, not Rust or generated Q outputs. Cover F1–F10 across all economy systems:

- **Drop resolution:** Single-item group collapse `(item.chance * group.chance) / 100`; multi-item group normalisation and cumulative walk; precise-drop calculation for chance >= 100%.
- **Level gap:** 5/10/10% window interpolation `linear_map(d, -10, -5, 10, 100) / 100` with floor 10% for d <= -10; raid boss clamp `clamp(d * 0.15 + 1, 0, 1)` with cutoff at d <= -7.
- **Vendor pricing:** `trunc(price * (1 + 0.15))` buy price; `referencePrice / 2` sell price; 12-item FIFO refund buffer.
- **Crafting:** Success roll against recipe successRate (715@100%, 244@60%, 27@70%, 12@10%, 1@25%, 1@95%); 100% material loss on failure; masterwork roll on 583 rare recipes.
- **Party adena split:** Even division among in-range members (1400 units); 1-by-1 random remainder allocation.
- **Fees:** 30 adena warehouse deposit fee; 1,000 adena freight fee; 100 adena base + 1,000 adena/slot mail fee.
- **Decay ticks:** 600s = 6,000 ticks (items); 60s = 600 ticks (herbs); 7s = 70 ticks + 10s = 100 ticks (spoil corpse).
- **Chaotic PK drops:** 40% base roll; equip unequipped before roll; weapon 10%, equip 40%, item 50%, max 10 items.
- **Rate multiplier:** Base 1x vs multiplied (2x, 3x) exact rational evaluation.

### 5.2 Endpoint, durability and migration evidence

Implement the complete API §4 matrix in §2.5: in-memory use cases, real TestApp gRPC/WS and DATABASE_URL-gated PostgreSQL adapters:

- **Migrations 000012–000015:** Apply cleanly, roll back, enforce foreign keys and check constraints; bigint item instance IDs; max adena check `MaxAdena <= 99,900,000,000`.
- **Atomic critical mutations:** Ground pickup, vendor buy/sell, craft, trade, storage deposit/withdraw, mail send/receive execute with single draft -> JetStream ack -> SeaORM fenced checkpoint + outbox pattern.
- **Idempotency:** 24-hour retention; duplicate request with same key returns frozen receipt without second mutation; conflicting body returns FAILED_PRECONDITION.
- **Outbox facts:** Verified publication of `nightfall.item.{dropped,picked_up,spoiled,swept,vendor_bought,vendor_sold,crafted,traded,deposited,withdrawn,mailed}`.

### 5.3 Fixture packs, predicates and scenario receipts

Fixture packs prefixed `phase5-*` seeded pre-admission via `infra/scripts/seed-phase5-*.py` using owned Postgres container exec pattern (no host psql):

- `phase5-drops`: NPC death drops, killer attribution, protection timers.
- `phase5-decay`: 600s item decay, 60s herb decay, player-drop persistence.
- `phase5-herbs`: Herb immediate effect without inventory allocation.
- `phase5-spoil`: Spoil debuff, corpse spoiled flag, Sweeper execution.
- `phase5-party`: 5 party loot modes, 1400-unit adena split.
- `phase5-vendors`: Buylists, castle tax, refund tab.
- `phase5-multisell`: Item exchange, maintainEnchantment.
- `phase5-craft`: Dwarven recipe book, Create Item ranks, crafting failure/success.
- `phase5-craft-common`: Common recipe book, Create Common Item.
- `phase5-trade`: Two-phase atomic trade, capacity validation.
- `phase5-stores`: Private sell/buy stores, slot limits.
- `phase5-manufacture`: Private manufacture workshop.
- `phase5-warehouse`: Private warehouse deposit/withdraw, 30 adena fee.
- `phase5-freight`: Account freight transfer, 1,000 adena fee.
- `phase5-mail`: Mail with attachments and COD, peace zone rule.
- `phase5-pk-drop`: Chaotic PK death gear strip and drops.
- `phase5-rates`: Global rate multiplier scaling.

Harness predicate mapping:
- `ground_item_exists <template>`: GroundItemSpawn received in AOI.
- `inventory_has <template> >= <n>`: Owner InventorySync/InventoryChanged received.
- `adena_count >= <n>`: Owner StatsChanged `adena_amount` received.
- `recipe_registered <recipe_id>`: GetRecipeBook or CraftResult received.
- `trade_state == <state>`: TradeSessionChanged received.
- `store_state == <state>`: PrivateStoreChanged received.
- `mail_received <subject>`: MailNotification received.

Apply C11: UNKNOWN until carrier received; unsupported syntax fails. Never allow-list UNKNOWN.

### 5.4 Replay, size, performance, soak and telemetry

- **Replay:** V7 `.nfr` replay bit-exact; legacy V6 and older replay byte-exact via `bash apps/api/fixtures/sessions/replay-check.sh`.
- **State digest divergence:** Mutate any canonical field in GroundItemInstance, CorpseState, TradeSession, or StoreState and confirm digest divergence; rebuild indexes without divergence.
- **Snapshot sizing:** Full Phase5Snapshot (ground items, corpses, trade sessions, store states, recipe books) fits within NATS 1 MiB envelope and 32 MiB decoded limit.
- **Performance budgets:**
  - 8-client packaged soak with drops/pickups/vendor traffic maintains tick p99 < 20 ms.
  - 1,000-NPC death-with-drops benchmark exceeds 10,000 kills/s on one core.
- **Telemetry:** Bounded labels only (op kind, outcome, item class, grade); `economy_counters` table rollups verified against outbox facts; `infra/scripts/verify-observability.sh` passes.

## 6. Dependencies, order and integration gates

Inherit the complete Phase 3/4 gate sequence. Never weaken any prior gate:

1. **Foundation:** reviewed Z economy design and D/C plan → C additive proto/errors/keys contract reviewed by Z; D generated data verified by Claude; C economy oracle reviewed by Z; C predicate/fixture foundation reviewed by Z.
2. **Runtime:** reviewed definitions/proto → Z E2 zone engine, V7 dispatcher/codec/export/snapshot reviewed by C. Snapshot/pre-run budget measured before fixture/schema freeze.
3. **Adapters/Persistence:** C publishes E3.1 migrations, storage/mail/store repositories, and Phase5Checkpoint; E3.2/E3.3/E3.4 RPCs and outbox handlers. Z reviews. Full endpoint, atomicity, ack/crash/fence/recovery tests required.
4. **Client/Harness:** proto enables E4 Unreal client widgets and bot DSL; live requires reviewed engine and API. C authors E4/E5, Z reviews, root holds Unreal lock. First 5-ground-item-drop and 5-vendor-buy-sell, then complete §2.8 scenario matrix.
5. **Live:** each scenario role/variant executes with live UI/cook, V7 and historical byte replay, coverage/contract/traceability, strict export/actual-input oracle, endpoint/actor/adapter crash/concurrency, and all earlier phase live suites pass.
6. **Close:** C reviews Z measurements before root-locked packaged soak; root integrates only independently reviewed commits and runs complete repository/Phase 1–4 gates plus Phase 5 additions: `moon ci`, data/proto checks, Rust tests, SOURCES scan, strict clippy (`-D warnings`), `cargo fmt --check`, legacy replay, sim-gates, required-live zero-skip Unreal tests, cook, perf/soak, observability. Docs-only authoring is not an implementation-gate pass. Outcome links preserved reviews and actual receipts after root acceptance.

## 7. Risks

| Risk | Consequence | Mitigation / acceptance |
|---|---|---|
| Ground item explosion per zone | Memory exhaustion or tick budget overrun | Strict 600s item / 60s herb decay; ground item limit per zone with oldest non-protected item cleanup if threshold reached. |
| In-memory trade/store deadlocks | Zone actor thread hang | Lock ordering by `objectId` (`TradeList.java:353-365`); all trades execute within single zone actor without cross-thread mutexes. |
| Economic mudflation (faucets > sinks) | Adena devalued, bot inflation | Bounded adena cap 99.9B; castle tax on purchases; warehouse/freight/mail sinks; recipe material destruction; telemetry monitors faucet/sink ratio via `economy_counters`. |
| Crafting/trade duplication exploits | Free item generation | All mutations are critical with `item_ledger` rows; capacity checks before mutation; two-phase trade commits atomically in single transaction. |
| Replay divergence from drop RNG | Breaks deterministic replay | Per-zone ChaCha12 stream drawn in strict fixed in-tick order (death resolution phase); seed and draw cursor recorded in snapshot/digest V7. |
| DB-first bypass of zone authority | Broken determinism and lost updates | Zone actor is single writer for in-world state; DB storage/mail containers mediated strictly via zone critical mutations. |

## 8. Out of scope

- **Phase 6 (World & Content):** Monster spawning runtime, AI aggro tables, broader travel/teleport routes, quests, raid bosses and raid instance dungeons.
- **Phase 7 (Social Systems):** Clan warehouse (200 slots, clan rank access controls), clan halls, castle sieges, Seven Signs competition cycles, and Olympiad.
- **Offline trade / private stores:** Pinned `OfflineTradeEnable=False` (`GC/customs.properties:190`). Offline trading is explicitly excluded from launch.
- **Item Auction live bidding:** Item auction schedules and data are fixture-backed; live auction house bidding is Phase 9 / Live Operations.
- **Durability, item binding, and Common equipment:** Pinned retail defaults exclude item durability, bind-on-pickup/equip, and un-enchantable Common items.
- **Clan hall / castle tax distribution:** Castle tax is collected into a ledgered sink at the no-owner rate (15%); distribution to castle lords activates in Phase 7.

## 9. Outcome

| Field | Evidence / disposition |
|---|---|
| Accepted commits and independent reviews | |
| Data/closure counts and source corrections | |
| API/actor/adapter/recovery evidence | |
| Client/scenario/legacy replay/oracle receipts | |
| Snapshot/performance/soak/observability measurements | |
| Limitations and architect dispositions | |
| Root integration and phase acceptance | |
