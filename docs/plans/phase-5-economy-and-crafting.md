# Phase 5 Plan: Economy and Crafting

**Status:** Planning. No implementation, story or gate has run; nothing in this document attests a pass.

**Authorship.** Plan rev 6 was a full rewrite by the Claude design author (AD5 B23), replacing agy rev 5 (9d54c2a), which plan-review-3 rejected. Rev 7 applies plan-review-4 (accept with bounded fixes; E43–E46, B24). Rev 8 renumbers Phase 5 migrations to 000015..000019 (after Phase 4's `m20261014_000014`), refreshes the design SHA to 91fc056, adds the E56 freight fixture note, adds client story 5.8 (Phase 4 D120, close 4423) and adds the Phase 4 process lesson (lane-wide no-stub and end-to-end smoke conditions; a latent-stub audit before the first integration gate run). Architect: Claude. Independent review: a fresh Claude reviewer (author ≠ reviewer, A20).

**Sources and precedence.**

| Source | Role |
|---|---|
| [`phase-5-zone-design.md`](phase-5-zone-design.md) @ **91fc056** (rev 10; section numbers unchanged since 0d525c7) | The **frozen** design (`codex/phase5-design`, AD5 B22). The file lives on that branch until integration merges it, so this link resolves only after the merge. Cited as **D §n**. Every contract, number and formula is normative there. This plan never restates a formula differently: where it summarises, the design wins. |
| [`phase-5-architecture-decisions.md`](phase-5-architecture-decisions.md) | Binding decisions P1–P4, A1–A20, B1–B26 and E1–E56. Re-synced from the architect notes in this commit: verbatim body plus a banner (B12). |
| `codex/phase5-data` @ **3410877** | The final Epic 1 data branch (E56). Stories 1.1–1.7 are complete there. |
| Final Phase 4 `main` SHA: **5abae32** (merged 2026-10-24) | All Phase 5 worktrees branch from this SHA. |
| [`05-economy-and-crafting.md`](../planning/05-economy-and-crafting.md) | Owner scope (P1). It is unpinned research. Its numbers never override the pinned sources or the design. |

Pinned sources: l2j-game `abfde0490ac52a2106e884b7242c0276c44cdf76` and l2j-dp `3ca488dd2bd0bfaca43e378886a3c2e37968153a`, under `/home/matt-woodruff/.cache/nightfall/l2j/`. They are build-time provenance, never a runtime or replay dependency.

## 1. Goal and locked decisions

Deliver the complete owner scope P1 as server-authoritative, replay-exact V7 zone behaviour:

| Area | Scope (owner decision / binding decision) | Design |
|---|---|---|
| Monster drops | Pinned death and corpse lists; binary64-faithful chances and exact 64-bit draws (E15, E22, E30); global multiplier G (P2, E21, E37, E42) | D §3.1–3.5 |
| Drop actor | maxDealer with lastAttacker fallback, ties to lowest EntityId (X7) | D §3.6 |
| Ground items | Entities carrying an ItemInstanceId (B5); two protection mechanisms (E7); 5 s-grid decay; no cap, gauge only (E25) | D §1.2, §3.7, §3.8 |
| PK drops | Pinned single loop with exclusion filter; **no** protection (E24) | D §3.9 |
| Pickup and herbs | 36-unit pawn range; capacity check slots-only, solo/FINDERS_KEEPERS only (P7); herbs with multi-skill early abort (E36, E40) | D §4.1, §4.3 |
| Minimal party (B9) | Invite/answer/leave/kick/leader; 5 loot modes; member vote (B15); even split; sweep routing | D §1.4, §4.2, §4.4, §4.6 |
| XP/SP under V7 (B16) | Pinned damage-share solo and party distribution | D §4.5 |
| Spoil and sweep | Through the cast pipeline (B11); Sweeper + ConsumeBody | D §5 |
| Vendors | Merchant AI-script buylists (E2, E9); binary64 unit-price table; restock (E18); refund tab (B6) | D §6 |
| Multisell | List-level maintainEnchantment; attributes no-op (B10) | D §7 |
| Crafting (P3) | Dwarven + Common; pinned check order; masterwork; recipe book with a single home (E17) | D §8 |
| Trade and private stores | Pinned TradeList semantics (E4, E10); stores close on departure (B2); **no buy-store escrow** | D §9 |
| Warehouse and freight (P4) | Same-account freight (E8); augmented items allowed (E17) | D §10.1–10.2 |
| Mail | Peace zone iff attachments; COD; reject/cancel/delete/expiry; pending deliveries (E27, E41) | D §10.3–10.4 |
| Item auction (B4, E16) | DB-only state; zone-hosted transitions | D §10.5 |
| Manor (B1, E13, E28) | Sow/Harvest via `UseItem`; Gludio fixture lists only | D §10.6 |
| Containers (E31, E33, E34, E38, E39, E42) | Single writer; lease and frozen-input contract | D §10.0 |
| Ledger (B13, E26) | Owner kinds; identity/merge/split transfers; rule T1 | D §11 |
| Version boundary (A4) | V7: Snapshot 11 / Record 9 / BinaryV7; `PHASE5_ENABLED` | D §12 |
| Wire (E20) | Additive only; the design is the tag source | D §13 |

**Defaults stated by the design (B12):** AutoLoot/AutoLootRaids/AutoLootHerbs = False and the per-template `auto_destroy_time` (one row, item 9849) are hash inputs (D §1.10 item 3). The clan-war exemption is vacuous until Phase 7 (D §3.9).

Closed architect answers that bind this plan: E24 (no PK protection), E25 (no cap, gauge only), E26 (T1 ratified), E27 as amended by E41 (per-delivery queue bound), E28 (Gludio, castle id 1) and E29 (G not applied to party bonus rates). There are no open QUESTIONs.

## 2. Canonical contracts this plan implements

### 2.1 Wire (D §13; verified equal to the design by plan-review-3)

New message types live in `packages/proto/nightfall/v1/economy.proto`. There are no new envelopes or streams (A3).

| Tag space | Allocation |
|---|---|
| ClientMessage intent | 29..69 (41 intents): PickUpItem=29, VendorBuy=30, VendorSell=31, VendorRefund=32, MultisellChoose=33, RegisterRecipe=34, DeleteRecipe=35, CraftItem=36, TradeRequest=37, TradeAnswer=38, TradeAddItem=39, TradeConfirm=40, TradeCancel=41, SetPrivateStoreSell=42, SetPrivateStoreBuy=43, SetPrivateManufacture=44, ClosePrivateStore=45, PrivateStoreBuy=46, PrivateStoreSell=47, PrivateManufactureOrder=48, PartyInvite=49, PartyAnswer=50, PartyLeave=51, PartyKick=52, PartyChangeLeader=53, PartyLootChangeRequest=54, PartyLootChangeAnswer=55, WarehouseDeposit=56, WarehouseWithdraw=57, FreightSend=58, FreightWithdraw=59, MailSend=60, MailTakeAttachments=61, MailReject=62, MailCancel=63, MailDelete=64, ManorBuySeed=65, ManorSellCrop=66, ItemAuctionBid=67, ItemAuctionCancelBid=68, MailMarkRead=69. Sow and Harvest use the existing `UseItem` (21). |
| WorldEvent | 34..45 (12 events): EconomyResult=34, CorpseStateChanged=35, PartyStateChanged=36, PartyLootVote=37, PartyInviteReceived=38, TradeRequestReceived=39, TradeSessionChanged=40, PrivateStoreChanged=41, MailNotification=42 (sent by the API gateway; not a PbOutput), RecipeBookSync=43, ItemAuctionStatusChanged=44, LootDistributed=45 |
| RejectReason | 38..88, extending the existing enum (D §13.1 lists all 51 names). Existing reasons are reused: OUT_OF_RANGE=10, RATE_LIMITED=5, DEAD_ACTOR=7, INSUFFICIENT_ITEMS=28, INVENTORY_FULL=29, ITEM_CAPABILITY_DEFERRED=36. |
| EntityKind / EntitySpawn | `ENTITY_KIND_GROUND_ITEM = 3`; `GroundItemDetail ground_item = 30`. A ground item despawns through the existing `EntityDespawn`, which has no reason field. |
| ItemOperation | 14..42: the 29 receipt scopes of D §11.3, same order |
| ItemLocation | 3..8: WAREHOUSE, FREIGHT, MAIL, AUCTION_BID, REFUND, DELIVERY |
| StatsChanged | no new field |
| GameService RPCs | 9 reads, appended after GrantItem: ListBuylist, GetMultisellList, GetRecipeBook, GetWarehouse, GetFreight, ListMail, ReadMail, ListItemAuctions, GetManorData. Messages as in D §13.4. ReadMail writes nothing (E31). `ListBuylist` returns `unit_price` from the D §6.2 table at castle 0, `limited`/`stock`/`max_stock`, and the caller's `refund_tab`; because stock and the tab come from the zone read port, it refuses `FAILED_PRECONDITION` unless the caller is admitted and `UNAVAILABLE` without a zone. Per-RPC statuses are D §13.4's table. |
| PbCommandKind | 32..72 = client tag + 3 (41 intents); system kinds 73 DeliverPending, 74 AuctionSchedule, 75 EpochReconcile, 76 MailExpire |
| PbOutput | 42 `Phase5Enriched` (V7-only EntitySpawn ground-item enrichment, D26 pattern); 43..54 carry WorldEvents 34..45, except 51, which is reserved and never emitted (MailNotification is API-only); 55 `EconomyCheckpoint` (the durable non-character ledger rows, container deltas, lease releases and delivery changes of a tick) |
| PbPlayerLoad | 11 carries `Phase5Checkpoint` bytes |

Visibility follows D §1.13. Admission order is identity spawns (including ground items), reference hydration, `SkillStateSync`, `InventorySync`, `RecipeBookSync`, then the `PartyStateChanged` baseline. C2/C2′/C4/C12 fencing is preserved. Enum values carry their enum-name prefix, and UNSPECIFIED rejects. Every mutating intent carries an `idempotency_key`; rejections consume no key (D41–D43).

### 2.2 Version boundary (D §12.1)

| Snapshot | Digest | Record | Rules / policy |
|---|---|---|---|
| 4,5 | JsonV1 | 3 | Historical Phase 1 validation unchanged |
| 6..8 | JsonV1/BinaryV2 | 3/4 respectively | Existing no-class legal matrix unchanged |
| 7 | BinaryV3 | 5 | Phase 2 before tokens |
| 8 | BinaryV4 | 6 | Phase 2 tokens |
| 9 | BinaryV5 | 7 | Valid Phase3 block/hash and version_one admission |
| 10 | BinaryV6 | 8 | Valid Phase3 plus Phase4 block/rules/hash and version_two admission; D26 enriched events use PbOutput41, forbidden under Record≤7 |
| 11 | BinaryV7 | 9 | Valid Phase3, Phase4 and Phase5 blocks/rules/hashes and version_three admission; PbOutput 42..55 forbidden under Record≤8 |

Rows 1–6 are copied verbatim from the Phase 4 plan §2.2.

- **Snapshot magic** is `b"NFSNAP11"`.
- **`PHASE5_ENABLED`:** unset selects V7 (except historical simulation-fixture selection); `"0"` selects V6 behaviour; `"1"` selects V7; any other value refuses boot. The recorded version wins on replay.
- **Legacy rejection.** Every Phase 5 field, kind, tag, entity kind, V7 `ItemLocation` and ledger op is rejected recursively under Record ≤ 8 before permissive decoding.
- **Historical bytes.** `apps/api/fixtures/sessions/*.nfr` stay byte-exact (`replay-check.sh`).
- **All live suites**, including earlier phases, run production V7 with the reviewed bundles (C16; §4 story 6.3).

**Phase5Rules (D §1.10)** is validated and hash-referenced like Phase4Rules (D27/D31), never embedded. Its hash inputs are D §1.10 items 1–11, including:
- the G **decimal string**, `ExponentXp/Sp` (nonzero fails closed), and every flood interval including ServerBypass 5;
- the 22-row L table;
- the OpenJDK 21 pow table: `pow56` (79) / `pow13` (241, d = −120..120), sha256 `4386eed8ca269e85885dabaf9bae02c5327cf425013221dfbf7df962136201f6`;
- the vendor unit-price table.

### 2.3 Engine state and in-tick ordering (D §1, §2)

**State** is exactly D §1.11 `Phase5Snapshot`:
- `ground`, `npcs` (the NpcEconomy sidecar; corpse timing stays the single `NpcAi.corpse_until`);
- `parties` plus `next_party`;
- `requests` (one shared transaction-request slot for trade **and** party invites, D §1.5);
- `trades`, keyed by the `(EntityId, EntityId)` pair;
- `stores`;
- `stock`, keyed by `(buylist_id, template)`;
- `players`: `Phase5Player` with recipes, refund tab, party, trade and flood next-allowed ticks.

DB-only rows (warehouse, freight, mail, item auction, manor, deliveries) are **never** snapshot or digest state (D §1.11, §12.4). Each command that touches them carries its frozen pre-image (D §10.0).

**V7 corpse time** is `corpse_until = death_tick + 10 × T + E`, where `T` is the template `corpseTime` (default 7) and `E = 100` iff spoiled **or** seeded (D §1.3). A swept corpse ends at the sweep tick and despawns in the **next** tick's spawn phase.

**Phases** (the real Phase 3 phases 0–10; D §2.1):

| Phase | V7 amendment |
|---|---|
| 0 commands | All intents in strict command-ordinal order with a rebuild after each accepted mutation (F23). Every in-world economy intent resolves here. Each flood gate runs at its pinned step (§2.4). Departure hooks run at the departure command's ordinal: store close, party leave, trade/request cancel, refund-tab destroy (D §9.4). Spoil, Sweeper, Sow and Harvest only **stage** casts. |
| 1 spawn | The existing corpse expiry; a swept corpse is removed here on the next tick. |
| 2 expiry | In order: (a) loot-owner and DropProtection deadlines cleared, (b) ground decay when `tick % 50 == 0`, (c) expired transaction requests dropped, (d) expired loot votes finished as declined, (e) expired party invitations cleared, (f) vendor restocks due. |
| 3 periodic + regen | Unchanged. A periodic kill goes through `kill()` and so through `death_v7`. |
| 4 movement | Unchanged. There is no trade or store movement cancel. |
| 5 cast impacts | Spoil, Sweeper + ConsumeBody, Sow, Harvesting and herb item skills execute at impact. A kill here goes through `death_v7`. |
| 6 auto impacts | Unchanged. A kill here goes through `death_v7`. |
| 7 hit bookkeeping | **Unchanged.** Drop resolution does not move here. |
| 8 new starts | `canSweep`, the NPC_BODY target type and the Seed handler checks; these draw nothing. |
| 9 AI | Unchanged. |
| 10 flush/observe | One durable draft. Ledger rows in T1 order. Container deltas and lease releases in the checkpoint transaction. Output only after log ack and commit. |

**Death resolution (D §2.2).** `death_v7` runs **inside `kill()`**, between `EntityDied` and `npc_died`, in whichever phase calls `kill()` (3, 5 or 6):
- It takes the reward snapshot from the hate ledger before the ledger is removed.
- It rolls the corpse list (if spoiled), then the death list.
- It creates ground items with their position draws.
- For a player victim, it runs the PK loop.

The existing `npc_respawn_jitter` draw then follows **after** all drop and position draws. That is the only reordered existing draw, and it changes only in V7 epochs. `kill()` step 7 replaces `award_xp(credit)` with the B16 distribution from the stashed snapshot, which draws nothing. No Phase 5 draw happens in phases 1, 2, 4, 7, 8 or 9.

**Draws (D §2.3).**
- `roll_below(n)`: the existing unbiased rejection over `next_u32`. Used for quantity `roll_below(max'−min'+1)` (always drawn, even at min = max; rule P3), positions, party RANDOM, even-split remainders and the auction item.
- `draw_u64()`: high word first.
- `chance(num, den)`: success iff `r·(100·den) < num·2^64`, evaluated in **U256**, one `draw_u64` per call (E15/E22).
- `chance_bound(k, n)`: `chance(k·100, n)`.
- **Spoil** keeps the Phase 3 `roll_below(100) < rate` (`rate` is an int; D §2.3 #1). It does **not** use `draw_u64`.

**Killer** (D §3.6, X7): the HateLedger is visited in ascending EntityId. Only player rows with `damage > 1` within PartyRange count; the range test is `4·dist² ≤ (2·50,000 + npc_radius)²`. `maxDealer` is the first entry reaching the strict maximum, i.e. the lowest EntityId. There is **no** timestamp state. If the maxDealer is offline, the killing blow's attacker is used. A non-player actor means no drops.

**Ranges** (D "Source authority and conventions"): `m(L) = trunc0(L·1000/32)`, with 2D tests (X1).
- Pickup: `dx²+dy² ≤ (1,125 + 0)²`, where 0 is the engine's player collision radius (X13).
- Vendor, keeper and store: `m(150) = 4,687`, non-strict.
- Trade: `> 4,687` rejects at request and **cancels the whole trade** at confirm; there is no other distance check.
- PartyRange2: `4·dist² ≤ (2·43,750 + rad)²`, where `rad` is the origin radius (the NPC for sweep, 0 for the picker) plus the member's 0.
- Sweeper: `m(20) = 625` plus the Phase 3 range rule.

### 2.4 Flood gates (D §2.4, E35)

| FloodKind | Interval (ticks) | Pinned users → intents |
|---|---|---|
| Transaction | 10 (`floodprotector.properties:85`) | VendorBuy, VendorSell, VendorRefund, WarehouseDeposit, WarehouseWithdraw, FreightSend, FreightWithdraw, TradeConfirm, TradeCancel, PrivateStoreBuy, PrivateStoreSell, DeleteRecipe, MailTakeAttachments, MailReject, MailCancel, ItemAuctionBid (packet lines in D §2.4) |
| MultiSell | 1 (`:78`) | MultisellChoose |
| Manufacture | 3 (`:92`) | CraftItem (`RequestRecipeItemMakeSelf:42`), PrivateManufactureOrder (`RequestRecipeShopMakeItem:49`) |
| Manor | 30 (`:99`) | ManorBuySeed |
| SendMail | 100 (`:106`) | MailSend |
| ServerBypass | 5 (`:71`) | ItemAuctionCancelBid (after the NPC-distance validation) |

Every other intent has no pinned gate. The rules:
- Each gate runs at its pinned step.
- A check that fails **before** the gate leaves `flood` untouched.
- A refused gate returns `RATE_LIMITED`.
- Once the gate passes, `next` stays updated even if a later check rejects (E35). That update is digested zone state.

### 2.5 Persistence, containers, ledger and recovery (D §10.0, §11)

**Ledger op vocabulary (D §11.3).**
- New V7 `item_ledger.op` values: `loot`, `transfer_out`, `transfer_in`, `party_split`, `sweep`, `harvest`, `purchase`, `sale_proceeds`, `craft`, `cod_credit`, `auction_award`, `manor_reward`.
- Reused values: `consume`, `destroy`, `exchange`, `unequip`, `grant`.
- There is **no** `drop`, `pickup` or `spoil` op.
- The 29 receipt **scopes** (ItemOperation 14..42) go only into `item_mutation_receipts.operation`, never into `item_ledger.op`.

**B13 transfer pairs** (`transfer_out`/`transfer_in`, in identity, merge or split mode; D §11.2) apply **only** when an existing instance changes owner:
- PK drop;
- pickup that is not a split;
- vendor sell into the refund tab, and refund back;
- trade, and both legs (items and adena) of store sales and purchases;
- the craft fee (customer → crafter);
- warehouse, freight and mail moves;
- deliveries of existing items;
- auction bids and cancels.

Everything else is a fresh row, a merge, or a `consume`/`destroy` (D §11.3 table, implemented verbatim):

| Operation | Rows |
|---|---|
| NPC loot | One fresh `loot` row under `ground:<entity>` |
| Sweep | `sweep` fresh/merge with **no** source row |
| Harvest | `harvest` fresh/merge |
| Vendor buy | `consume` adena + `purchase` |
| Multisell | `consume` ingredients + `exchange` product (one ItemExchanged) |
| Craft | Fee transfer pair, `consume` materials, `craft` product |
| Party even split | Ground `destroy` + per-recipient `party_split` |
| Herb pickup | Ground `destroy` |
| Decay | Ground `destroy` |
| Manor | `consume` + `purchase` (seed) or `consume` crop/fee + `manor_reward` |
| Auction win | Escrow `destroy` (sink) |
| Purge | Escrow `destroy` |

**Rule T1 (E26).** Per (owner, tick, instance) the order is:
1. non-consume rows;
2. the Consume aggregate;
3. at most one terminal row: `transfer_out` with after = None, or `destroy`.

Ordinals are the row positions 0..n−1.

**Owner kinds (D §11.1):** `character`, `ground`, `refund`, `warehouse`, `freight`, `mail`, `delivery`, `auction_bid`.

**Containers (D §10.0, E31/E33/E34/E38/E39/E41/E42).**
- **Single writer:** only the leased zone checkpoint writes container rows.
- **Lease keys:** `<owner_kind>:<39-digit zero-padded decimal owner_id>`, compared bytewise; Postgres key columns use `COLLATE "C"`.
- **Acquisition:** all-or-nothing in key order. The lease table is the only lock; there are no `FOR UPDATE` data-row locks. `CONTAINER_BUSY` is returned if any key is held.
- **Checkpoint:** applies deltas as CAS in key order and deletes the leases in the same order.
- **Errors:** `40P01`/`40001` are transient. A CAS miss under a held lease is the permanent `economy_lease_violation` fence.
- **System jobs** refused busy are re-queued to the next tick holding nothing. Deliveries for one target run as a delivery-id queue, one per tick. Each completes within one checkpoint cadence after reaching the queue head (E41), and only the lowest-id delivery runs in the D7 admission slot.
- **System-job commit markers:** delivery `status = 'delivered'`, auction state reached or row deleted, message row absent.

Lease sets per command, verbatim from D §10.0:

| Command | Lease set, in acquisition order (bytewise key order; `{x, y}` = sorted by id) | Fresh rows inserted under |
|---|---|---|
| ItemAuctionBid | `auction:a` (current auction of the instance) | bid row, under `auction:a` |
| ItemAuctionCancelBid | `auction:a` for **each** non-CREATED auction of the instance with the actor's bid, as frozen, in id order | — |
| AuctionSchedule Create / Start / Purge (system) | `auction:a` | auction row (Create), under `auction:a` |
| AuctionSchedule Finish (system) | `auction:a`, `delivery:d` | delivery `d` and its row, under `delivery:d` |
| DeliverPending (system) | `delivery:d`, then `warehouse:c_target` for `mail_expiry_return` / `auction_award` | — |
| FreightSend | `freight:c_target` | — |
| FreightWithdraw | `freight:c_actor` | — |
| MailTakeAttachments | `delivery:d` (COD only), `mail:m` | delivery `d`, under `delivery:d` |
| MailMarkRead | `mail:m` | — |
| MailReject | `mail:m`, `mailbox:c_original_sender` | return message `m′`, under `mailbox:c_original_sender` |
| MailCancel | `mail:m`, `mailbox:{c_sender, c_receiver}` (the hard delete changes both counts) | — |
| MailDelete | `mail:m_i` for each message (id order), `mailbox:c_actor` | — |
| MailExpire (system) | `delivery:d` (when there are attachments), `mail:m`, `mailbox:{c_sender, c_receiver}` | delivery `d` and the re-homed attachment rows, under `delivery:d` |
| MailSend | `mailbox:{c_sender, c_receiver}` | the new message `m`, under `mailbox:c_receiver` |
| ManorBuySeed | `manor:manor_id` | — |
| ManorSellCrop | `manor:<each distinct entry manor_id>` (id order) | — |
| WarehouseDeposit / WarehouseWithdraw | `warehouse:c_actor` | — |
| EpochReconcile (system) | none (ledger only) | — |

**Migrations (D §11.4).**
- **Numbering.** Phase 4 API migrations end at `m20261014_000014` (`character_lane_quarantines`). Phase 5 migrations are therefore **000015..000019**. Verified at the final Phase 4 `main` SHA 5abae32: the last migration is `m20261014_000014`. Design §11.4 still shows 000014..000018: each number shifts by one. The design's first implementation-time touch (story 7.2) records this shift; the frozen design is not edited now.
- **Names:**
  - `m2026XXXX_000015_economy_ledger_owners`
  - `…_000016_economy_items`
  - `…_000017_economy_mail`
  - `…_000018_economy_auction_manor`
  - `…_000019_economy_control`
- **Contents** are exactly D §11.4: owner-widened ledger with T1 trigger; single item table for every durable location with the mail/delivery FK policy; `mail_messages` with checkpoint-computed `expires_at`; `item_auctions`/`item_auction_bids` with plain NULLable bidder columns; `manor_seed_production`/`manor_crop_procure`; `economy_leases`/`economy_deliveries`/`economy_counters`/`economy_counter_facts`.
- **Each migration** ships `.sql` + `_down.sql` + `.rs`, and `entities.sh` regenerates the entities.

**Recipe book (E17, D §1.8).** It has one home: the zone, plus `Phase5Checkpoint { schema_version: 1, rules_hash, recipes }` (`deny_unknown_fields`) in `characters.phase5_checkpoint`. There is **no** `character_recipes` table. `GetRecipeBook` serves the checkpoint out of world and returns `FAILED_PRECONDITION` while the character is admitted.

**Epoch close and reconcile (D §11.6).**
- **Graceful close:** destroy rows for every remaining ground item and refund entry (`epoch_close:<epoch>`).
- **Abandoned epoch:** `EpochReconcile` runs in the next epoch before admission, from the last fenced snapshot's manifest, writing `epoch_void:<epoch>` destroy rows and releasing that epoch's leases.

**Outbox:** subjects as in D §11.5. `economy_counters` is filled asynchronously and deduplicated by `fact_id` (B8).

**Critical mutation order (A1):** draft → log ack → fenced checkpoint + outbox → visible output. Crash windows are in D §10.0.

### 2.6 Domain rules that earlier drafts got wrong (normative pointers)

| Rule | Design |
|---|---|
| Pickup checks in order: alive (`DEAD_ACTOR`) → target (`GROUND_ITEM_NOT_FOUND`) → range (`OUT_OF_RANGE`) → DropProtection → capacity **slots only and only solo/FINDERS_KEEPERS** (P7; herbs exempt) → loot owner. A non-executable herb refuses `ITEM_CAPABILITY_DEFERRED` **before** removal. A receiver count cap (D68/D70) refuses `ITEM_COUNT_CAP` before any change, for pickup, trade (P15) and store buys alike. | D §4.1, §3.7 |
| The two protections are separate. **Loot owner:** online owner, `isInLooterParty`. **DropProtection:** owner or same party, no online requirement. Command channels do not exist before Phase 7, so the raid first-CC 900 s rights are vacuous and raid loot carries DropProtection only. | D §3.7 |
| The raid clamp and all killer modifiers are binary64 values from the frozen 22-row L table (d = −6 raid → `0.10000000000000009`). | D §3.3, §6.2 |
| G fold: `rate' = fl32(fl(rate × G))` for every G-multiplied rate (E37); G is the owner's decimal string. G does not touch amount multipliers, the party bonus rates or the siege price. | D §3.4 |
| Party bonus: `Bx = n < 2 ? 1 : fl(B[min(n−1, 8)] × RatePartyXp)`, then `⌊fl(e × Bx)⌋`; shares use `Math.round(fl(e × pre))`. | D §4.5 |
| Solo XP (B16): damage-share `calcExpSp` with `pow56` (E32, k ∈ [1, 79]); `x ≤ 0 ⇒ sp = 0` (36 SP-only NPCs award 0/0); float `penalty` product. | D §4.5 |
| Craft: every check (combat, recipe, rank, customer adena, **materials**, MP/HP) runs **before** any payment. Then MP → fee → materials → success draw → masterwork draw → product. A missing material is a plain reject with nothing spent. | D §8.2 |
| Buy stores do **not** escrow adena. Adena moves at sale, and sells to a buy store skip entries beyond the owner's adena. | D §1.6, §9.3 |
| Trade: `addItem` invalidates only the adder's confirmation. There is no remove packet. Distance is checked at request and confirm only. A validation failure cancels (P14). `UseItem` by either party cancels. Departure, disconnect or death cancels (B7). | D §9.1–9.2 |
| Freight capacity is base 200; the target's `FREIGHT_LIM` is ignored (`_owner = null`). | D §10.2 |
| Mail send requires the peace zone **iff** there are attachments. An expired mail (COD or not) returns its attachments through MailExpire → `delivery` → sender **warehouse**. | D §10.3 |
| Herb multi-skill loop: list order until the first failed condition, disabled or reuse check. Reuse is keyed by skill id + level. The herb is destroyed regardless. | D §4.3 |
| Corpse time: `+100` ticks iff spoiled or seeded (not additive). | D §1.3 |
| Vendor restock runs in phase 2(f). Stock is keyed `(buylist_id, template)`. | D §1.7, §6.4 |

### 2.7 Formula, rounding and adaptation ledger

The normative ledger is **D §14.2**: X1–X14 (X5/X6 withdrawn, X14 closed by E32), P1–P21 and T1. Rounding primitives are `round_p`, `fl`/`fl32`, the L table and E32's pow/round sites (D §6.2). No story may introduce a rounding site that is not in D §14.2. A new site is a design change, not a plan change.

### 2.8 Canonical scenario matrix (D §14.5, verbatim names; 36 files)

All files live under `apps/client-unreal/Scenarios/`. **Header syntax (E45, as Phase 4 plan §2.8):** each file starts with the exact `# covers: 5 E<story>, …` line given in its row, then `# fixture: <pack>`. `sim-traceability.py` parses these lines, and every `[client-visible]` story (1.8, 2.1, 3.3–3.9, 3.11, 3.15, 4.6, 4.9, 5.1–5.7, 6.6) appears in at least one header. A header alone never satisfies a story. Expected values come from the E1 oracle sidecars (story 1.8), never typed by hand. RNG-dependent outcomes are asserted on received state; exact outcomes go through the C14 export and C15 oracle gates (story 6.2). Every file is named by 1.8 (oracle expectations), 3.15 (server assertion sections), 5.7 (UI subscope) and 6.6 (live run); the seven container files (warehouse, freight, the two mail files, auction, manor, epoch reconcile) are also named by 4.9. Live execution of every file is owned by story **6.6** alone. Packs are §5.3 `phase5-*`.

| File | Exact `# covers:` header | Pack (`# fixture:`) | Received assertions (minimum) |
|---|---|---|---|
| `5-drop-basic.nfs` | `# covers: 5 E1.2, 5 E1.5, 5 E1.8, 5 E2.1, 5 E3.3, 5 E3.12, 5 E3.15, 5 E5.1, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-drops | Ground `EntitySpawn` kind 3 with GroundItemDetail; the NPC loot `loot` row; maxDealer is the lowest-EntityId max-damage player |
| `5-drop-precise-over-100.nfs` | `# covers: 5 E1.2, 5 E1.5, 5 E1.8, 5 E2.1, 5 E3.3, 5 E3.15, 5 E5.1, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-drops-precise | `> 100` ungrouped vs `≥ 100` group branches; quantity × mult |
| `5-drop-level-gap.nfs` | `# covers: 5 E1.2, 5 E1.5, 5 E1.8, 5 E2.1, 5 E3.3, 5 E3.15, 5 E5.1, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-drops-gap | Drops at d = −6…−10 using L-table chances (C15 checks draws) |
| `5-drop-rate-multiplier.nfs` | `# covers: 5 E1.2, 5 E1.5, 5 E1.8, 5 E2.1, 5 E2.3, 5 E3.3, 5 E3.15, 5 E5.1, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-rates | G string `"1.5"`: base and multiplied expectations both asserted (P2) |
| `5-pickup-protection.nfs` | `# covers: 5 E1.8, 5 E2.1, 5 E3.3, 5 E3.5, 5 E3.15, 5 E5.1, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-protection | Loot owner and DropProtection asserted separately (offline owner, party member, stranger, expiry) |
| `5-herb.nfs` | `# covers: 5 E1.2, 5 E1.4, 5 E1.8, 5 E2.1, 5 E3.5, 5 E3.15, 5 E5.1, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-herbs | Single and multi-skill (8612) herb; early abort under reuse; no inventory row |
| `5-decay.nfs` | `# covers: 5 E1.5, 5 E1.8, 5 E2.1, 5 E3.3, 5 E3.15, 5 E5.1, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-decay | Despawn on the 50-tick grid after the strict limit; PK drop never decays (the ground-item gauge is verified by 7.1's scrapes, not here) |
| `5-party-loot-finders.nfs` | `# covers: 5 E1.6, 5 E1.8, 5 E2.1, 5 E3.5, 5 E3.15, 5 E5.1, 5 E5.2, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-party | Picker receives; slot-only capacity check |
| `5-party-loot-random.nfs` | `# covers: 5 E1.6, 5 E1.8, 5 E2.1, 5 E3.5, 5 E3.15, 5 E5.1, 5 E5.2, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-party | Recipient in range; `LootDistributed` to the others |
| `5-party-loot-random-spoil.nfs` | `# covers: 5 E1.6, 5 E1.8, 5 E2.1, 5 E3.5, 5 E3.6, 5 E3.15, 5 E5.1, 5 E5.2, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-party-spoil | Sweep routed through RANDOM |
| `5-party-loot-by-turn.nfs` | `# covers: 5 E1.6, 5 E1.8, 5 E2.1, 5 E3.5, 5 E3.15, 5 E5.1, 5 E5.2, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-party | Cursor pre-increment and skip |
| `5-party-loot-by-turn-spoil.nfs` | `# covers: 5 E1.6, 5 E1.8, 5 E2.1, 5 E3.5, 5 E3.6, 5 E3.15, 5 E5.1, 5 E5.2, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-party-spoil | Sweep routed through BY_TURN |
| `5-party-adena-split.nfs` | `# covers: 5 E1.6, 5 E1.8, 5 E2.1, 5 E3.5, 5 E3.12, 5 E3.15, 5 E5.1, 5 E5.2, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-party | Even split with remainder; **no** `LootDistributed`; `party_split` rows |
| `5-party-vote.nfs` | `# covers: 5 E1.8, 5 E2.1, 5 E3.5, 5 E3.15, 5 E5.2, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-party | Request, accept to threshold, single decline, timeout, join cancels |
| `5-party-xp.nfs` | `# covers: 5 E1.6, 5 E1.8, 5 E2.1, 5 E3.4, 5 E3.15, 5 E5.2, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-party-xp | BONUS table, `Math.round` shares, highfive gaps |
| `5-solo-xp-damage-share.nfs` | `# covers: 5 E1.6, 5 E1.8, 5 E2.1, 5 E3.4, 5 E3.15, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-solo-xp | B16 damage share between two solo players; SP-only NPC awards 0/0 |
| `5-spoil-sweep.nfs` | `# covers: 5 E1.5, 5 E1.8, 5 E2.1, 5 E3.6, 5 E3.15, 5 E5.1, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-spoil | Spoil, corpse +10 s, Sweeper, ConsumeBody, despawn next tick |
| `5-sweep-party.nfs` | `# covers: 5 E1.6, 5 E1.8, 5 E2.1, 5 E3.5, 5 E3.6, 5 E3.15, 5 E5.1, 5 E5.2, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-party-spoil | Spoiler's party member sweeps; adena split from the NPC origin |
| `5-pk-drop.nfs` | `# covers: 5 E1.5, 5 E1.8, 5 E2.1, 5 E3.3, 5 E3.12, 5 E3.15, 5 E5.1, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-pk-drop | Unequip-before-roll; exclusions; break at 10; **no** protection (E24) |
| `5-vendor-buy-sell-refund.nfs` | `# covers: 5 E1.3, 5 E1.7, 5 E1.8, 5 E2.1, 5 E3.7, 5 E3.15, 5 E4.6, 5 E5.3, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-vendors | Unit price from the table; sell `⌊ref/2⌋·c`; refund `⌊c·ref/2⌋`; 13th entry destroyed |
| `5-vendor-restock.nfs` | `# covers: 5 E1.3, 5 E1.7, 5 E1.8, 5 E2.1, 5 E3.7, 5 E3.15, 5 E4.6, 5 E5.3, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-vendor-restock | Out-of-stock reject; the restock due tick and refill arithmetic asserted through the export + V-R oracle (E43). The live refill after 36,000 ticks (the shortest pinned restock, 60 min) runs only in the soak lane (6.7), unless 6.1 finds an existing harness tick fast-forward facility, in which case the ordinary run uses it |
| `5-multisell.nfs` | `# covers: 5 E1.3, 5 E1.7, 5 E1.8, 5 E2.1, 5 E3.7, 5 E3.15, 5 E4.6, 5 E5.3, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-multisell | maintainEnchantment copy; amount; one ItemExchanged |
| `5-craft-dwarf.nfs` | `# covers: 5 E1.3, 5 E1.7, 5 E1.8, 5 E2.1, 5 E3.8, 5 E3.15, 5 E4.6, 5 E5.3, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-craft | Register, craft, failure loss, missing-material reject with nothing spent |
| `5-craft-common-masterwork.nfs` | `# covers: 5 E1.3, 5 E1.7, 5 E1.8, 5 E2.1, 5 E3.8, 5 E3.15, 5 E4.6, 5 E5.3, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-craft-common | Common book; masterwork outcome via EconomyResult.craft |
| `5-private-manufacture.nfs` | `# covers: 5 E1.7, 5 E1.8, 5 E2.1, 5 E3.8, 5 E3.9, 5 E3.15, 5 E4.6, 5 E5.3, 5 E5.4, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-manufacture | Fee before roll and kept on failure; Manufacture 3-tick gate |
| `5-trade.nfs` | `# covers: 5 E1.8, 5 E2.1, 5 E3.9, 5 E3.12, 5 E3.15, 5 E5.4, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-trade | Add invalidates the adder only; both confirm; transfer pairs |
| `5-trade-cancel-disconnect.nfs` | `# covers: 5 E1.8, 5 E2.1, 5 E3.9, 5 E3.15, 5 E5.4, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-trade | Departure cancels; UseItem cancels; too far at confirm cancels |
| `5-store-sell-package.nfs` | `# covers: 5 E1.8, 5 E2.1, 5 E3.9, 5 E3.15, 5 E5.4, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-stores | Package must be bought whole; store closes when empty |
| `5-store-buy.nfs` | `# covers: 5 E1.8, 5 E2.1, 5 E3.9, 5 E3.15, 5 E5.4, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-stores | No escrow; entries beyond the owner's adena skipped |
| `5-warehouse.nfs` | `# covers: 5 E1.7, 5 E1.8, 5 E2.1, 5 E3.11, 5 E3.15, 5 E4.3, 5 E4.4, 5 E4.6, 5 E4.9, 5 E5.5, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-warehouse | 30 per entry; merge; augmented allowed; CONTAINER_BUSY under a held lease |
| `5-freight.nfs` | `# covers: 5 E1.7, 5 E1.8, 5 E2.1, 5 E3.11, 5 E3.15, 5 E4.3, 5 E4.4, 5 E4.6, 5 E4.9, 5 E5.5, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-freight | Same account; capacity 200; adena entry adds no slot |
| `5-mail-cod.nfs` | `# covers: 5 E1.7, 5 E1.8, 5 E2.1, 5 E3.11, 5 E3.15, 5 E4.3, 5 E4.4, 5 E4.5, 5 E4.6, 5 E4.9, 5 E5.5, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-mail | COD consume and `cod_credit` delivery; MailNotification |
| `5-mail-reject-cancel-expiry.nfs` | `# covers: 5 E1.8, 5 E2.1, 5 E3.11, 5 E3.15, 5 E4.3, 5 E4.4, 5 E4.5, 5 E4.6, 5 E4.9, 5 E5.5, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-mail | Reject return message; cancel; MailExpire → warehouse delivery |
| `5-auction-bid-cancel-award.nfs` | `# covers: 5 E1.7, 5 E1.8, 5 E2.1, 5 E3.11, 5 E3.15, 5 E4.3, 5 E4.4, 5 E4.5, 5 E4.6, 5 E4.9, 5 E5.5, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-auction | Equal bid accepted; difference payment; cancel; award to warehouse |
| `5-manor-sow-harvest-sell.nfs` | `# covers: 5 E1.3, 5 E1.7, 5 E1.8, 5 E2.1, 5 E3.6, 5 E3.11, 5 E3.15, 5 E4.3, 5 E4.4, 5 E4.6, 5 E4.9, 5 E5.5, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-manor | Gludio seed sown (castle_area 1); harvest; crop sale reward and fee |
| `5-epoch-reconcile.nfs` | `# covers: 5 E1.8, 5 E2.1, 5 E3.11, 5 E3.12, 5 E3.15, 5 E4.4, 5 E4.7, 5 E4.9, 5 E5.6, 5 E5.7, 5 E6.6` | phase5-epoch | Graceful close and abandoned-epoch destroy rows before admission |

The matrix count is 36, and story 6.6 asserts `files == 36` against this table.

## 3. Engineering architecture, ownership and review gates

Follow the domain → application ports → infrastructure/interface separation. Phase 5 zone modules are exactly D §1.1 and are registered in the SOURCES scan. The usual denials stay: unwrap/expect/panic, floats, HashMap and wall clock.

| Letter | Who (A20 as amended by E23/B23) | Owns |
|---|---|---|
| **Z** | Claude zone worker (opus) | E2.2, E2.3, E3, E6.4, E6.5 benchmarks, and all shared `state*.rs`/`command.rs`/`state_digest.rs`/replay codec edits. **Z serializes shared zone and codec edits.** Codec tag allocation is owned once, by 2.2. |
| **C** | Claude API/client/harness worker (opus) | E1.5–E1.8 oracle and expectations; E2.1 proto (the one proto editor); E4; E5; E6 except 6.4/6.5 benchmarks; E7.1 |
| **D** | agy (gemini) | Data generation drafts (E1.1–E1.4) and docs (E7.2, E7.3). Every D output is verified by Claude (A20). |
| **Root** | Claude architect | Integrates independently reviewed commits, runs gates, holds the lock for **all** Unreal build/run/cook/stage activity, and merges |

Author ≠ reviewer always: Z reviews C, and C reviews Z. Data output gets Claude verification against the pinned clones, file:line by file:line.

## 4. Epics, stories, models and Done When

All stories are pending. A story gets the `✅` prefix only after independent review plus named evidence, never on a documentation claim.

**Dependency order (acyclic; B12).** data (E1) → contracts (E2) → engine (E3) → API (E4) → client (E5) and harness (E6) → integration and close (§6).
- A story's Done When names only evidence its own epic or an **earlier** epic can produce.
- E1, E2 and E3 stories complete on unit, property, replay and oracle evidence. **None of them requires a `.nfs` run.**
- Their "Simulation scenario(s)" stories (1.8, 3.15) author expectation and assertion sections and are reviewed. They do not run them.
- Live `.nfs` passes are owned **once**, by 6.6. The client UI subscope is owned by 5.7.
- **Lane-wide Done When (Phase 4 process lesson).** Every lane's final story adds two conditions to its Done When:
  1. **No pending, fence or stub markers** remain in the lane's owned files (`TODO`, `todo!`, `unimplemented!`, stub returns, "pending"/"fence" placeholders). A grep over the owned files proves it.
  2. **One end-to-end smoke on the production binary**: grant → login → intent → evidence export. The smoke runs the lane's own feature path.

Each design §14 test family or item has exactly one owner:

| Design item | Owner |
|---|---|
| §14.1 families D-C, D-E, D-W, D-L, D-M, D-O, D-P, D-T | 1.5 |
| §14.1 families L-P, X, G | 1.6 |
| §14.1 families V-P, V-R, V-S, M, C, S, A, F, PW, LT, FL | 1.7 |
| §14.3 evidence contract v2: producer | 3.13 |
| §14.3 evidence contract v2: consumer and gates | 6.2 |
| §14.4 "Engine units" | the implementing engine story (3.1–3.12), each for its own section; except the `PHASE5_ENABLED` matrix (2.2), lease CAS (4.4) and V5/V6 historical byte-exact replay (3.14) |
| §14.4 "Properties" | 3.14 |
| §14.4 "Crash windows" | 4.8 |
| §14.4 "Migrations" | 4.1 |
| §14.5 scenario matrix (live) | 6.6 |

### Epic 1: Data generation and independent oracle (D drafts, C verifies and owns the oracle; Z review)

| Story | Model | Work / Done When |
|---|---|---|
| 1.1 Source ledger and `gen_economy.py` scaffold [required] | D | `packages/data/scripts/gen_economy.py` with `--check`, the provenance header, the DATAPACK_REVISION check, exact decimal strings, a SOURCES.md section and `check_all.py` integration. **Done When:** C verifies the pinned paths/revisions and `--check` against a clean regeneration. (Rust-parser agreement on the emitted TOML is 2.3's Done When, E46.) |
| 1.2 Drop, corpse, herb, ground and rate tables [required] | D | `tables-phase5/` for D §1.10 items 1–5 and 11: drop/corpse lists in document order (decimal string **and** `fl` value), herb table with item-skill lists, ground constants, rate configs with **double/float kinds**, G as a decimal string, `ExponentXp/Sp`, the 22-row L table, and the pow table. **Done When:** C re-counts against D §1.10 (14,589/3,032/744; 35/2,260/7,335; 234 corpseTime; 68/59/48/9 herbs; 30/29 NonDroppable, 11 PetItems; 36 SP-only templates). The L-table bits and the pow-table sha256 `4386eed8…` (320 entries) match. |
| 1.3 Merchant, multisell, recipe, storage, mail, flood, auction and manor tables [required] | D | D §1.10 items 6–10: 274 scripts / 20,236 entries with tax/delay/count literals; the precomputed unit-price table for castle {0, 5, 15, 25}; restock minutes; 196 multisell lists; 1,000 recipes with 582 rare; storage/mail/flood constants; the auction data and fixture schedule; seeds.xml; the **Gludio-only** fixture manor lists (E28). **Done When:** C re-counts (5,803 pairs, 2,145 limited, 14 applyTaxes / 31 maintainEnchantment / 21 maintainIngredient, 258 crops / 129 alternative / 32 Gludio). Zero-price 8187 is flagged. |
| 1.4 Catalogue admission closure [required] | D | Extend the D5/D54 closure: every template referenced by a Phase 5 table is **storable**; executability is a separate capability (D §1.10 item 5). **Done When:** the imported/storable/executable counts manifest is regenerated and reviewed. No row is dropped for a capability. Non-executable herbs are named. |
| 1.5 Oracle: drop families [required] | C | `economy_oracle.py` families D-C, D-E, D-W, D-L, D-M, D-O, D-P, D-T exactly as D §14.1. **Done When:** all vectors pass against the pinned sources. D-E reproduces the 3/4 threshold groups and the 1,127 fall-offs. D-W reports 141 bits at G = 1 and ≤ 255 at G = 2^±32. No Rust-authored expectations. |
| 1.6 Oracle: party, XP and ledger families [required] | C | Families L-P, X and G (D §14.1). **Done When:** vectors pass, including X for n = 5 with all e < 2·10⁶ (74,892 differences, e = 45 → 62), the 36 SP-only templates, and T1 orders. |
| 1.7 Oracle: vendor, craft, multisell, manor, auction, fee, pow, L-table and flood families [required] | C | Families V-P, V-R, V-S, M, C, S, A, F, PW, LT, FL (D §14.1). **Done When:** vectors pass. V-P equals binary64 over all 5,803 pairs × 4 castle percents (362 naive-truncation differences at castle 0). PW matches all 320 captured entries; its negative controls give 44/79 and 78/79. |
| 1.8 Simulation scenario(s) [client-visible] [required] | C | Generate oracle expectation sidecars for all 36 §2.8 files: `5-drop-basic`, `5-drop-precise-over-100`, `5-drop-level-gap`, `5-drop-rate-multiplier`, `5-pickup-protection`, `5-herb`, `5-decay`, `5-party-loot-finders`, `5-party-loot-random`, `5-party-loot-random-spoil`, `5-party-loot-by-turn`, `5-party-loot-by-turn-spoil`, `5-party-adena-split`, `5-party-vote`, `5-party-xp`, `5-solo-xp-damage-share`, `5-spoil-sweep`, `5-sweep-party`, `5-pk-drop`, `5-vendor-buy-sell-refund`, `5-vendor-restock`, `5-multisell`, `5-craft-dwarf`, `5-craft-common-masterwork`, `5-private-manufacture`, `5-trade`, `5-trade-cancel-disconnect`, `5-store-sell-package`, `5-store-buy`, `5-warehouse`, `5-freight`, `5-mail-cod`, `5-mail-reject-cancel-expiry`, `5-auction-bid-cancel-award`, `5-manor-sow-harvest-sell`, `5-epoch-reconcile` (`.nfs`). 1.8 also owns the **pack declarations** (E46): the oracle-relevant inputs of every §5.3 pack (roles, classes, levels, positions, inventories, recipes, party, container rows and the fixture schedule) as data files under `apps/client-unreal/Scenarios/fixtures/`. 6.1 implements the seeders that consume them and extends them with harness-only fields. **E56 fixture note:** freight packs use freightable templates only (20678 Gatekeeper Hat, 20725 Red Flame of Valakas, or other freightable templates). Adena (57) is not freightable, so no freight declaration contains adena except the refusal vector. **Done When:** each sidecar is derived from 1.5–1.7 functions over those declarations and reviewed by Z. **No live run is required** (that is 6.6). |

### Epic 2: Contracts, version boundary and rules (C proto; Z codec/rules)

| Story | Model | Work / Done When |
|---|---|---|
| 2.1 Additive proto contract [client-visible] [required] | C | `economy.proto` plus the `world`/`items`/`game` appends exactly as D §13.1–13.4 (all oneof lines, message bodies, enums, the 9 RPC messages, ItemLocation 3..8). C is the only proto editor. **Done When:** `buf lint` passes; `buf breaking` against the final Phase 4 integration SHA reports additive-only; generated Rust and Unreal C++ compile; a tag-table test asserts every number in §2.1 equal to D §13. Z reviews. |
| 2.2 Replay codec and V7 version boundary [required] | Z | **The single owner of codec tag allocation:** PbCommandKind 32..76, PbPlayerLoad 11, PbOutput 42..55 (51 reserved), `RECORD_SCHEMA_VERSION = 9`, `NFSNAP11`, `StateDigestVersion::BinaryV7`, the `PHASE5_ENABLED` parser (D §12). **Done When:** round-trip units pass for every new kind/tag. Recursive rejection holds under Record ≤ 8 (including present-but-empty fields). The `PHASE5_ENABLED` matrix (unset/`"0"`/`"1"`/other → boot refusal; recorded version wins) passes (§14.4 owner). C reviews. Historical replay byte-exactness is owned by 3.14. |
| 2.3 Phase5Rules loader and hash [required] | Z | `Phase5Rules::try_new` over the 1.2/1.3 tables. It validates every cross-reference, cross-checks the L table, unit-price table and pow table, fails closed on nonzero Exponent or an out-of-range G, and computes the hash (D §1.10). **Done When:** the Rust loader and the Python generator agree on every emitted TOML table (moved from 1.1, E46); hash-stability, any-field-mutation and mismatch-refusal units pass; the hash is referenced (never embedded) in the snapshot and in admission policy `version_three` (D27 pattern). |

### Epic 3: Zone engine (Z; C review)

Every story ends on unit, property, replay and oracle evidence. None runs `.nfs`. Each story owns the §14.4 "Engine units" for its design sections. Draw-order tests compare the evidence trace with the D §2.3 table.

| Story | Model | Work / Done When |
|---|---|---|
| 3.1 Phase5 sidecar, snapshot, digest and checkpoint DTO [required] | Z | `Phase5Zone`/`Phase5Snapshot`/`Phase5Player`/`Phase5Checkpoint` exactly as D §1.11; invariants; the D §12.4 digest order; V5/V6 digests untouched. **Done When:** snapshot round-trip, every-field digest-mutation, invariant-violation and legacy-isolation units pass. |
| 3.2 Wide arithmetic, binary rounding and draws [required] | Z | `wide.rs` U256, `binary64.rs` `round_p`/`fl`/`fl32` (fail closed outside the normal range), `draw_u64`, `chance`, `chance_bound`; `roll_below` reused (D §2.3, §6.2). **Done When:** units over the D §6.2 algorithm agree with the 1.5/1.7 oracle (D-E, LT, PW, V-P inputs). A `u128`/`i128` chance product is rejected by a source-scan test. |
| 3.3 `death_v7`: drops, ground items, protections, decay and PK drops [client-visible] [required] | Z | Implement D §2.2, §3 and §1.2–1.3: reward snapshot, maxDealer (X7), corpse then death lists with binary64 chances, positions, loot owner + DropProtection, the 5 s-grid decay, V7 corpse time, and the PK loop with no protection (E24). The jitter follows the drops. **Done When:** D-C/D-E/D-L/D-M/D-O/D-P/D-T actual-input oracle units pass. The draw-order trace equals D §2.3 rows 2–6. V5/V6 `kill()` is byte-identical. |
| 3.4 B16 XP/SP distribution [client-visible] [required] | Z | Replace `award_xp(credit)` in V7 `kill()` step 7 with the D §4.5 solo and party distribution (binary64/32 emulation, `pow56` with k ∈ [1, 79] fail-closed, `x ≤ 0 ⇒ sp = 0`, the dead-member second pass). V5/V6 keep killing-blow XP. **Done When:** family X vectors pass on actual inputs; level-up events are unchanged in shape; V6 replay is byte-exact. |
| 3.5 Pickup, herbs, party substrate and distribution [client-visible] [required] | Z | D §1.4, §1.5 (shared request slot), §4.1–4.4, §4.6: P7 pickup order, herbs with E40 early abort and id+level reuse, all five modes, even split without `LootDistributed`, the vote, party departure. **Done When:** L-P and pickup-order units pass, including `DEAD_ACTOR`, `ITEM_CAPABILITY_DEFERRED` and slot-only capacity; vote timeout and join-cancel units pass. |
| 3.6 Spoil, Sweeper, Sow and Harvesting effects [client-visible] [required] | Z | D §5 and §10.6 skills through the cast pipeline (B11): NPC_BODY, `canSweep`, Sweeper + ConsumeBody (holders cleared, despawn next tick), Sow/Harvest, and V7 `magic_success_rate` with base `fl(1.3)` per E32 (V5/V6 unchanged). **Done When:** units pass and agree with family **S** (Sow/Harvest success and harvest counts) on actual inputs. The Phase 3 full-domain magic-success oracle is **re-run with the V7 factor** and agrees. Spoil uses `roll_below(100)`. |
| 3.7 Vendors, refund tab, restock and multisell [client-visible] [required] | Z | D §6–7: unit-price table lookups, all VendorBuy/Sell/Refund checks, refund-tab overflow and departure destroy, phase-2 restock, multisell with amount, maintainEnchantment and `BlacksmithUseRecipes` consumption. **Done When:** V-P/V-R/V-S/M units pass on actual inputs. |
| 3.8 Crafting and recipe book [client-visible] [required] | Z | D §8: register/delete, and the craft order **all checks before any payment**, then MP → fee → materials → success → masterwork; manufacture for a customer. **Done When:** family C units pass, including "missing material rejects with nothing spent" and the fee kept on failure. |
| 3.9 Trade and private stores [client-visible] [required] | Z | D §9: shared request slot, TradeList semantics, P14 cancel, UseItem cancel, store open/close/buy/sell/package, **no escrow**, departure hooks (D §9.4). **Done When:** units pass for every check order and cancel path. |
| 3.10 Flood gates [required] | Z | D §2.4 (E35): per-kind `next` in `Phase5Player.flood` at the pinned step; set even on a later reject. **Done When:** FL units pass. The flood field is digested and survives snapshot round-trip. |
| 3.11 Container commands, system jobs and EpochReconcile (zone side) [client-visible] [required] | Z | D §10.1–10.6 and §11.6. The zone consumes frozen pre-images, emits `ContainerDelta`s and the whole-set `LeaseRelease`, and executes the system commands (using 2.2's kinds 73–76) and the epoch-close destroy rows. It never reads the DB. **Done When:** units over frozen inputs pass for every container command, incl. DeliverPending merge/insert and AuctionSchedule Finish/Purge; a rejected command emits only the release (plus any flood update); results agree with families **S** (crop sale reward and fee), **A** (auction) and **F** (warehouse/freight/mail fees) on actual inputs. |
| 3.12 Ledger planner and EconomyCheckpoint [required] | Z | D §11.1–11.3: owner kinds, identity/merge/split pairs, T1 ordinals, the D §11.3 table rows and the op vocabulary of §2.5; emits `EconomyCheckpoint` on 2.2's PbOutput 55. **Done When:** family G units pass; D67/D68/D74 cap and freshness checks pass on every receiving row. |
| 3.13 Evidence producer v2 [required] | Z | `nightfall-replay export --economy-evidence-out` with the D §14.3 shapes (`item_rng`, `econ_chance`, `econ_select`, `econ_formula`, extended `item_delta`, `econ_container`). **Done When:** the export of V7 sessions recorded **in-test by 3.13's own engine replay harness** (synthetic sessions covering every fact kind; E46) is deterministic and complete; 6.4 later re-runs the export over its verified fixtures. Missing or corrupt facts fail. The old exports are unchanged. |
| 3.14 Cross-cutting properties and legacy isolation [required] | Z | D §14.4 "Properties": count conservation, no negatives, endpoint caps, unique ground ids, party invariants; V5/V6 code paths and bytes untouched. **Done When:** property suites pass. Every historical fixture replays byte-exact. |
| 3.15 Simulation scenario(s) [client-visible] [required] | Z | Author the server-side assertion sections (export/oracle gate bindings) of every §2.8 file whose row lists an E3 story. **Done When:** sections are reviewed by C against the 1.8 sidecars and the D §14.3 evidence shapes. **No live run** (6.6 owns it). |

### Epic 4: API, persistence, leases and RPCs (C; Z review)

| Story | Model | Work / Done When |
|---|---|---|
| 4.1 Migrations and entities [required] | C | The §2.5 migrations 000015..000019 (the numbers are verified at the final Phase 4 `main` SHA 5abae32), with `.sql`/`_down.sql`/`.rs` and the `entities.sh` regeneration. **Done When:** 4.1 owns the D §14.4 "Migrations" item: up/down on an empty DB and on a seeded Phase 4 DB pass; the T1 trigger accepts the legacy D76 shape and the T1 shape and rejects others. `entities.sh --check` is clean. |
| 4.2 Checkpoint and admission adapter [required] | C | `characters.phase5_checkpoint` (the only durable recipe-book home), the PbPlayerLoad 11 producer, memory/Postgres parity. **Done When:** fingerprint parity and fail-closed unknown-field/version units pass on both adapters. No `character_recipes` table exists. |
| 4.3 Lease manager, preflight and frozen inputs [required] | C | D §10.0: per-command lease sets, canonical keys with bytewise order and `COLLATE "C"`, all-or-nothing acquisition, no data-row locks, pre-images and DB-time decisions per the D §12.2 frozen table, and the system-job `request_key` derivation. **Done When:** units pass for every lease set; opposite-role MailSend acquires in the same order; a held key gives `CONTAINER_BUSY` with nothing inserted. |
| 4.4 Economy checkpoint writer and outbox [required] | C | D §11.5: container CAS in key order, lease deletes in the same order, owner-kind ledger rows, delivery rows, `expires_at` computed in the insert, outbox subjects, `40P01`/`40001` transient, `economy_lease_violation` permanent. **Done When:** memory/Postgres adapter units pass, including forced 40P01 retry and a CAS-miss fence (4.4 owns the D §14.4 "lease CAS" engine-unit item). |
| 4.5 Schedulers and system-job injection [required] | C | D §10.3–10.5: delivery queue (E41; lowest id in the D7 slot), MailExpire detection, AuctionSchedule from the fixture schedule, economy host zone, busy re-queue to the next tick, and the API-only MailNotification gateway. **Done When:** queue-bound, re-queue and commit-marker units pass against a stub zone port. No API code path writes a lease-governed row (source-scan test, E31). |
| 4.6 Read RPCs [client-visible] [required] | C | The 9 RPCs of D §13.4 with Phase 4 RPC conventions and errors. **Done When:** D §13.4's per-RPC contract table passes on TestApp gRPC and both adapters, with auth/ownership/malformed cases for every RPC and these specific statuses:
- `GetRecipeBook`: `FAILED_PRECONDITION` while admitted;
- `ListBuylist`: `FAILED_PRECONDITION` unless admitted, and `UNAVAILABLE` without a zone;
- `ReadMail`: omits attachments and returns `FAILED_PRECONDITION` for a mail with attachments unless the zone read port confirms a peace square;
- `GetManorData`: empty for non-Gludio castles.

`ReadMail` writes nothing. |
| 4.7 Epoch close and EpochReconcile (API side) [required] | C | D §11.6: the graceful-close final tick, the abandoned-epoch manifest from the last fenced snapshot, `EpochReconcile` before admission, and lease release. **Done When:** units over an abandoned-epoch fixture produce exactly one terminal row per live ground/refund instance and release all its leases. |
| 4.8 Crash-window and migration durability suite [required] | C | D §14.4 "Crash windows" (all rows, incl. the rev 5–7 additions: delivery merge/insert with a concurrent deposit refused busy, Finish escrow destroy around log ack, racing MailSend → `CONTAINER_BUSY` then `MAILBOX_FULL`, flood update on a later reject, system-job lease recovery, forced 40P01) (migrations are 4.1's). **Done When:** every row passes on memory and Postgres (owned containers on non-default ports, worker-common). A skipped adapter test is not a pass. |
| 4.9 Simulation scenario(s) [client-visible] [required] | C | Author the API/container sections of `5-warehouse`, `5-freight`, `5-mail-cod`, `5-mail-reject-cancel-expiry`, `5-auction-bid-cancel-award`, `5-manor-sow-harvest-sell`, `5-epoch-reconcile` (`.nfs`). **Done When:** sections reviewed by Z; live passes are 6.6. |

### Epic 5: Unreal client and bot (C; Z review; root Unreal lock)

| Story | Model | Work / Done When |
|---|---|---|
| 5.1 Ground items, corpse state and pickup [client-visible] [required] | C | Ground-item placeholder mesh/icon from `EntitySpawn` kind 3; both protection deadlines shown; corpse spoiled/seeded/swept; pickup intent. **Done When:** named reducer/widget automation passes; no client authority over protection. |
| 5.2 Party UI [client-visible] [required] | C | Invite/answer, leave/kick/leader, the loot-mode vote, `LootDistributed` log. **Done When:** named automation passes; `PartyStateChanged` baseline/delta fencing tests pass. |
| 5.3 Vendor, multisell, crafting and recipe-book dialogs [client-visible] [required] | C | Buy/sell/refund tabs from `ListBuylist`; multisell from `GetMultisellList`; recipe book from `RecipeBookSync`; craft and manufacture. **Done When:** automation passes; the same request key is reused on retry; no client-side price or chance math. |
| 5.4 Trade and private-store windows [client-visible] [required] | C | Trade request/answer/add/confirm/cancel; store set/close/buy/sell/package/manufacture. **Done When:** automation passes. |
| 5.5 Warehouse, freight, mail, auction and manor dialogs [client-visible] [required] | C | Driven by the read RPCs and the WS intents; `CONTAINER_BUSY` retry UX. **Done When:** automation passes. |
| 5.6 Bot DSL and predicates [client-visible] [required] | C | The §5.3 command and predicate set, and only that set. **Done When:** parser, UNKNOWN and reset tests pass. |
| 5.8 Lane-quarantine close code (Phase 4 D120) [required] | C | The Unreal client treats WebSocket close **4423** `character_lane_quarantined` as **no-reconnect**, like 4409. It shows the reason and stops the reconnect loop. The bot records the close code in its diagnostics coverage. **Done When:** named NetClient and BotDiagnostics automation tests close a socket with 4423 and assert no reconnect attempt, the shown reason and the counted close code; the existing 4409 tests still pass. |
| 5.7 Simulation scenario(s) [client-visible] [required] | C | Client UI-subscope automation for every §2.8 file (all 36 names as listed in 1.8). **Done When:** `run-tests.sh --require-live-api` passes with **zero skips**, and cook/stage proves the assets. A `.nfs` run alone never proves pixels. The live `.nfs` pass is 6.6. |

### Epic 6: Harness, regression, replay, sizing, scenarios and soak (C; Z review)

| Story | Model | Work / Done When |
|---|---|---|
| 6.1 Predicate map, fixture dispatcher and packs [required] | C | First deliverable: the parser-tested §5.3 command/predicate → carrier map. Then `phase5-fixtures.py` and `infra/scripts/seed-phase5-*.py` for every §5.3 pack, consuming 1.8's pack declarations and extending them with harness-only fields (E46). The `phase5-freight` seeder seeds freightable templates only (20678, 20725 or other freightable templates); adena is not freightable (E56), seeded pre-admission via owned-container exec (no host psql). 6.1 also checks whether the harness has a tick fast-forward/injection facility for `5-vendor-restock` (E43) and records the answer. **Done When:** parser/stub/UNKNOWN/reset tests and seed idempotency tests pass before any scenario work. |
| 6.2 Evidence contract v2 consumer and oracle gates [required] | C | Strict consumer of the 3.13 export (D §14.3), with C14/C15 actual-input gates calling the 1.5–1.7 oracle functions. **Done When:** missing/corrupt/repeated/foreign-owner fact tests and every `econ_chance` exact recomputation pass. UNKNOWN is never allow-listed. Z reviews. |
| 6.3 Earlier-phase suites re-derived under production V7 [required] | C | C16: every Phase 0b–4 live scenario runs production V7. A machine-readable before/after report covers every scenario and variant, with the reasons for each changed value: B16 damage-share XP, the respawn-jitter reorder after the drop draws, and V7 corpse time. Changed expectations are re-derived with the oracle and re-cited in the header. **Done When:** the report is reviewed; no assertion is relaxed, quarantined or allow-listed; historical `.nfr` files stay byte-exact. |
| 6.4 V7 replay fixtures and digest coverage [required] | Z | Verified V7 `.nfr` fixtures with drops, pickups, party, vendor, craft, trade, containers and system jobs. Each canonical Phase 5 field is mutated to show digest divergence. Snapshot mid-action recovery. **Done When:** replay byte-exact; mutation and recovery tests pass; the 3.13 export over these fixtures is deterministic; C reviews. |
| 6.5 C10 measurement gate and benchmarks [required] | Z | D §1.12. Encode the real worst-case snapshot for the Phase 4 reference workload of **8 players**, with the stated soak assumption of 1,000 live NPC loot items plus 100 PK drops. Trades ≤ 4 and parties ≤ 4 at 8 players. Report the encoded/decoded bytes and the largest contributors against 1,047,552 / 33,554,432. Also the death-with-drops 1,000-NPC benchmark. **Done When:** the measured encoded/decoded bytes are within the limits; the gate **fails on overflow** (E44; D §1.12). An overflow is reported with the largest contributors, never truncated, and no limit is raised. C reviews the receipts. |
| 6.6 Simulation scenario suite [client-visible] [required] | C | Run every §2.8 file live: `5-drop-basic`, `5-drop-precise-over-100`, `5-drop-level-gap`, `5-drop-rate-multiplier`, `5-pickup-protection`, `5-herb`, `5-decay`, `5-party-loot-finders`, `5-party-loot-random`, `5-party-loot-random-spoil`, `5-party-loot-by-turn`, `5-party-loot-by-turn-spoil`, `5-party-adena-split`, `5-party-vote`, `5-party-xp`, `5-solo-xp-damage-share`, `5-spoil-sweep`, `5-sweep-party`, `5-pk-drop`, `5-vendor-buy-sell-refund`, `5-vendor-restock`, `5-multisell`, `5-craft-dwarf`, `5-craft-common-masterwork`, `5-private-manufacture`, `5-trade`, `5-trade-cancel-disconnect`, `5-store-sell-package`, `5-store-buy`, `5-warehouse`, `5-freight`, `5-mail-cod`, `5-mail-reject-cancel-expiry`, `5-auction-bid-cancel-award`, `5-manor-sow-harvest-sell`, `5-epoch-reconcile` (`.nfs`). **Done When:** every role/variant has nonvacuous assertions, JUnit, record, byte replay, the 6.2 export/oracle verdict, trace/contract/coverage and cleanup; the file count equals 36; zero required-live skips. |
| 6.7 Eight-client packaged soak [required] | C | Packaged soak with 8 clients exercising drops/pickups, vendor traffic, a craft and a trade per client, on top of the Phase 3/4 soak load. The soak lane also runs the live `5-vendor-restock` refill after 36,000 ticks (E43) unless 6.1 found a fast-forward facility. PostgreSQL and NATS volumes are on **tmpfs** (`SOAK_TMPFS_VOLUMES`, Phase 3 P6), and the storage backing is recorded with each run (`storage.txt`). **Done When:** tick p99 < 20 ms; no client errors; replay byte-exact; API and cleanup exit 0; storage record present. Root holds the lock. |

### Epic 7: Telemetry, documentation and outcome (C/D; independent review)

| Story | Model | Work / Done When |
|---|---|---|
| 7.1 Metrics and observability [required] | C | Bounded op/outcome/item-class/grade counters (A19) and the **only** new gauge, `nightfall_zone_ground_items{kind}` (E25); `infra/scripts/verify-observability.sh` updated. No alerts or thresholds are invented. **Done When:** pre/active/post scrapes agree with committed export facts; no identifier labels. |
| 7.2 Diagrams and operational docs [required] | D + Claude | D drafts the implemented flows and runbooks (lease and delivery queue, epoch reconcile). **Claude** (the design author, E23/B23) makes the first implementation-time design touch: the four cosmetic design-review-7 nits A–D (B22), and aligning the domain `ItemLocation` list (which includes `Ground`) with the six wire values in D §13.4. The touch also records the migration renumbering: Phase 5 migrations are 000015..000019, not D §11.4's 000014..000018, because Phase 4 ends at `m20261014_000014`. **Done When:** Claude review approves them against the implemented contracts. |
| 7.3 Outcome and limitations [required] | D | Fill §9 only after root acceptance. **Done When:** Claude review verifies the evidence links, measured sizes and residuals. |

## 5. Required acceptance evidence and fixtures

**Planned, not executed.** Preserve every Phase 1–4 gate and suite. Phase 5 adds coverage; it never replaces it.

### 5.1 Data, oracle and formula evidence

`packages/data/scripts/economy_oracle.py` is independent of the Rust code. It uses `Fraction` plus its own `fl`, with CPython `float` only as the binary64 reference. It reads the pinned sources or the `--check`-verified tables.
- **Coverage:** the 22 D §14.1 families, owned per §4 (1.5/1.6/1.7). Their minimum vectors are exactly D §14.1.
- **Oracle manifest:** records the source revisions with file:line, the actual inputs, the draw values and cursor order, every rounding site (D §14.2), the outputs and the vector hashes.
- **Fail-closed:** unsupported input fails. There are no tolerances and no finite-vector lookup pretending universal coverage.
- **Gates:** C15 gates call the committed oracle functions with **actual** resolution inputs from the 3.13 export.

### 5.2 Endpoint, durability and migration evidence

- **Endpoint matrix:** for every WS intent and RPC, in-memory use cases, real TestApp gRPC/WS and `DATABASE_URL`-gated PostgreSQL adapters cover:
  - every reachable reject and documented status;
  - auth/ownership, malformed/bounded input and no-write validation;
  - same-key frozen retry, and changed-body conflict before mutation.
- **Concurrency:** concurrent exactly-once leases.
- **Durability:** the D §10.0 and §14.4 crash windows and the migration up/down suite (owned by 4.8).
- **Skipped tests:** a skipped adapter test is not a pass.

### 5.3 Fixture packs, predicates and scenario receipts

**Packs.** Packs are seeded **before admission** by `infra/scripts/seed-phase5-*.py`, using owned-container exec (no host psql). Each declares the roles, accounts, classes, levels, positions, inventories/paperdolls, recipes, party, container rows, fixture schedule, ledger seed and `next_item_instance`. The full set is:

`phase5-drops`, `phase5-drops-precise`, `phase5-drops-gap`, `phase5-rates` (G `"1.5"`), `phase5-protection`, `phase5-herbs`, `phase5-decay`, `phase5-party`, `phase5-party-spoil`, `phase5-party-xp`, `phase5-solo-xp`, `phase5-spoil`, `phase5-pk-drop`, `phase5-vendors`, `phase5-vendor-restock`, `phase5-multisell`, `phase5-craft`, `phase5-craft-common`, `phase5-manufacture`, `phase5-trade`, `phase5-stores`, `phase5-warehouse`, `phase5-freight`, `phase5-mail`, `phase5-auction`, `phase5-manor` (Gludio, castle_area 1), `phase5-epoch`.

**Commands and predicates.** This is the **one** command/predicate set (stories 5.6 and 6.1). Every predicate is evaluated only from received carriers:

| Command / predicate | Received authoritative evidence |
|---|---|
| `nf.PickUp <ground>`; `ground_item_exists <template>` | EntitySpawn kind 3 / EntityDespawn |
| `inventory_has <template> >= <n>` | Owner InventorySync / InventoryChanged |
| `nf.Buy` / `nf.Sell` / `nf.Refund` / `nf.Multisell` | EconomyResult (vendor / multisell) plus inventory deltas |
| `nf.RegisterRecipe` / `nf.DeleteRecipe` / `nf.Craft`; `recipe_registered <id>`, `craft_result == <success\|fail\|masterwork>` | RecipeBookSync / EconomyResult (recipe, craft) |
| `nf.ManufactureOrder <owner> <recipe>` (PrivateManufactureOrder, E45); `craft_result == …` | EconomyResult (craft) for the customer and the crafter |
| `nf.TradeRequest/Answer/Add/Confirm/Cancel`; `trade_state == <phase>` | TradeRequestReceived / TradeSessionChanged |
| `nf.StoreSell/StoreBuy/StoreManufacture/StoreClose/StoreBuyFrom/StoreSellTo`; `store_state == <kind>` | PrivateStoreChanged / EconomyResult (store) |
| `nf.PartyInvite/Answer/Leave/Kick/Leader/LootVote/LootAnswer`; `party_member_count == <n>`, `loot_mode == <mode>` | PartyStateChanged / PartyLootVote / PartyInviteReceived |
| `nf.Deposit/Withdraw/FreightSend/FreightWithdraw` | EconomyResult (container) |
| `nf.MailSend/MailTake/MailReject/MailCancel/MailDelete/MailRead`; `mail_received <subject>` | MailNotification / EconomyResult (mail) |
| `nf.AuctionBid/AuctionCancel`; `auction_state == <state>` | ItemAuctionStatusChanged / EconomyResult (auction) |
| `nf.ManorBuySeed <manager> <manor_id> <seed> <n>` / `nf.ManorSellCrop <manager> <instance> <crop> <manor_id> <n>` (E45); `manor_result granted <template> >= <n>` | EconomyResult (manor) plus inventory deltas |
| `nf.UseItem` (Sow, Harvest, herbs) | Existing UseItem carriers plus CorpseStateChanged |

**C11 rules.**
- A predicate is UNKNOWN until its carrier is received, and it resets on generation/incarnation/tick/baseline.
- Unsupported syntax fails.
- UNKNOWN is never allow-listed.
- `.allow` sidecars declare precise expected rejection codes only.
- Server-only RNG, ledger and lease assertions go through the C14 export and the C15 oracle.

### 5.4 Replay, size, performance, soak and telemetry

- **Replay fixtures (6.4):** V7 `.nfr` fixtures with digest mutation and recovery. Historical fixtures stay byte-exact.
- **Pre-run limits:** the 256 KiB / 1,024 / 8 / B±1 / FIFO / oversize / no-rerun gates (Phase 3 design §4.5/§8.1) are preserved for `version_three`.
- **C10 gate (6.5):** as stated in §4. The workload is the Phase 4 reference of 8 players, with the design's stated assumption of 1,000 live NPC loot items plus 100 PK drops (an assumption, not a bound). The gate fails on overflow (E44).
- **Budgets (A18, unchanged):** 1,000 NPC mean < 2 ms; 1,000 players/observers mean < 10 ms; eight packaged clients tick p99 < 20 ms. The death-with-drops bench is added.
- **Soak (6.7):** tmpfs storage (`SOAK_TMPFS_VOLUMES`) and the storage record, per Phase 3 P6.
- **Telemetry (7.1):** bounded labels only.

## 6. Dependencies, order and integration gates

Phase 4 merged to `main` at 5abae32 without the soak gate. T54 (soak tick p99) is open, and its fix path is on `codex/phase4-server-api`. Phase 5 story 6.7 inherits that open item.

Phases 1–4 acceptance remains required. There is no whole-epic circular gate:

1. **Foundation:**
   - E1.1–1.4 data → E1.5–1.7 oracle → E1.8 expectations.
   - E2.1 proto (reviewed by Z) and E2.2 codec (reviewed by C) in parallel with E1.
   - E2.3 rules after E1.2/E1.3.
   - E6.1 predicate map and seeders after E2.1 and 1.8 (it consumes 1.8's pack declarations, E46).
2. **Engine:** E3 after E2. Stories 3.1–3.2 come first. 3.3–3.12 can run in parallel behind Z's serialization of shared files. Then 3.13 and 3.14, then 3.15.
3. **Adapters:** E4.1–4.4 after E2.1/E2.2 (4.2 also after 3.1); E4.5/4.7 after 3.11; E4.6 after 4.1–4.2; E4.8 after 4.1–4.7.
4. **Client and harness:** E5 skeleton after E2.1; live after E3/E4. E6.2 after 3.13; E6.3/6.4/6.5 after E3/E4. E6.6 after E4, E5.6 and E6.1/6.2. E6.7 after 6.5 and 6.6.
5. **Close (root).** Every item is required implementation evidence. Docs-only authoring is never a gate pass. **Before the first integration gate run**, root runs a **latent-stub audit** over the merged tree: no pending, fence or stub markers in any Phase 5 file, and every lane's end-to-end smoke receipt present. Root integrates independently reviewed commits and runs, on the merged tree:
   - `moon ci`;
   - `cargo fmt --check`;
   - strict `cargo clippy --all-targets -- -D warnings`;
   - the SOURCES/denial source scan;
   - the full Rust test suite with **dependency-backed** Postgres/NATS tests actually run;
   - data `--check`/`check_all.py`, `entities.sh --check`, `buf lint` and `buf breaking`;
   - `replay-check.sh` historical byte-exact and the V7 replay fixtures;
   - the sim gates: contract, traceability and coverage;
   - offline and **required-live** Unreal automation (`run-tests.sh --require-live-api`) with **zero skips**;
   - the **full ordinary scenario suite including every earlier phase re-derived under V7** (6.3) plus all 36 Phase 5 files (6.6);
   - Linux **Game and Editor builds**, cook and **stage**;
   - the C10 gate, benchmarks, the packaged soak on tmpfs with its storage record, and `verify-observability.sh`.

   The outcome links the preserved review receipts and actual run receipts.

## 7. Risks

| Risk | Consequence | Mitigation / acceptance |
|---|---|---|
| Binary64/binary32 emulation drift | Drop/XP/price outcomes diverge from pinned | One `round_p` implementation; the L, pow and unit-price tables are hash-bound and cross-checked at `try_new`; D-E/PW/LT/V-P oracle gates over all real rows |
| Unbounded ground state (E25) | Snapshot exceeds C10 | Measurement gate plus gauge; overflow is reported, never truncated |
| Lease contention | `CONTAINER_BUSY` storms; delayed deliveries | Bytewise key order, no data-row locks, busy re-queue, per-delivery bound (E41) and crash-window tests |
| V7 changes earlier-phase observables (B16, jitter order, corpse time) | Silent weakening of older suites | Story 6.3 before/after report and re-derivation; no relaxed assertion |
| Migration numbering collision with late Phase 4 migrations | Broken migration order | Phase 5 starts at 000015 after Phase 4's `m20261014_000014`; verified at the final Phase 4 `main` SHA 5abae32 |
| Unreal shared build state | Invalid evidence | Root lock; named automation, cook and stage receipts |

## 8. Out of scope

- **Phase 6:** castles, Seven Signs, sieges, castle-owner manor cycles and tax changes (only hashed ceilings exist now); raids beyond pinned drop rules; pets/servitors.
- **Phase 7:** clan warehouse, command channels (raid first-CC rights stay vacuous), cross-zone parties, block lists, clan war (the PK clan-war exemption stays vacuous).
- **Phase 6 sinks from 05:** teleport fees (travel) and Ancient Adena / Mammon (Seven Signs).
- **05's "Giran luxury shop":** it has no pinned buylist. The pinned NPC Alexandria (30098) uses an item-exchange AI script (`DPJ/ai/npc/Alexandria/Alexandria.java`), not a `BuySellList`, and is out of Phase 5 scope.
- **Never:** offline stores (`OfflineTradeEnable = False`); cursed weapons; elemental attributes (B10); champions (`ChampionEnable = False`); alternative crafting; floats, hot-reloaded rules or client-authoritative economy math.

## 9. Outcome

Filled only after root acceptance (story 7.3).
