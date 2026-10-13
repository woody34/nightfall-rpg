> **BINDING RECORD — Phase 5 zone engine and economy design (Claude author per AD5 E23).** This is the implementation contract for new Snapshot 11 / Record 9 / Binary V7 epochs. It incorporates owner decisions P1–P4 and the approved defaults, architect decisions A1–A20, dispositions B1–B16, and errata E1–E23. It replaces the two rejected agy drafts (e602c25 and 43c02a6) in full. Every finding of design-review-1 (F1–F36) and design-review-2 (R1–R44) is answered in §16. It does not claim that any implementation or acceptance gate passes.
>
> **Tag source of truth (E20).** The wire, codec, reject-reason and RPC allocations in §12–§13 are normative. The plan (`docs/plans/phase-5-economy-and-crafting.md`, `codex/phase5-plan`) syncs to them.
>
> **Engine base.** Engine citations use `codex/phase4-zone-engine` @ **80b8077** (`Z/`, `R/`, protos). Migration citations use `codex/phase4-server-api` @ **e6d37ec**. Neither branch contains the other. Every engine line must be re-checked at the final Phase 4 integration SHA before implementation starts. Engine line numbers move; contracts do not.

# Phase 5 zone engine: drops, loot, vendors, crafting, trade, storage, mail, auction and manor

## Source authority and conventions

| Prefix | Meaning |
|---|---|
| **Z/** | `apps/api/src/domain/zone/` at `codex/phase4-zone-engine` 80b8077 |
| **R/** | `apps/api/src/application/replay_log/` at the same SHA |
| **M/** | `apps/api/src/infrastructure/postgres/migrations/` at `codex/phase4-server-api` e6d37ec |
| **W / I / G / E** | `packages/proto/nightfall/v1/{world,items,game}.proto` at 80b8077; **E** = new `economy.proto` (§13) |
| **GJ/** | `/home/matt-woodruff/.cache/nightfall/l2j/l2j-game/src/main/java/com/l2jserver/gameserver/` @ `abfde0490ac52a2106e884b7242c0276c44cdf76` |
| **GC/** | that repository's `src/main/resources/config/` |
| **DP/** | `/home/matt-woodruff/.cache/nightfall/l2j/l2j-dp/src/main/resources/data/` @ `3ca488dd2bd0bfaca43e378886a3c2e37968153a` |
| **DPJ/** | that repository's `src/main/java/com/l2jserver/datapack/` |
| **DPS/** | that repository's `src/main/resources/sql/` |
| **AD5** | `/home/matt-woodruff/.config/Claude/side-session-notes/phase5-agy-2026-10-13/architecture-decisions.md` (§0 P1–P4, §1 A1–A20, §5 B1–B8, §6 E1–E6/B9–B12, §7 E7–E12/B13–B14, §8 E13–E17/B15, §9 E18–E23/B16) |
| **SN4** | `/home/matt-woodruff/.config/Claude/side-session-notes/phase4-codex-2026-10-12/architecture-decisions.md` (D1–D77) |
| **VR5 / DR1 / DR2** | that Phase 5 notes directory: `reference-brief-verification.md`, `design-review-1-claude.md`, `design-review-2-claude.md` |
| **P3Z / P4Z** | `docs/plans/phase-3-zone-design.md` / `docs/plans/phase-4-zone-design.md` |

Conventions, binding for the whole document:

- **Arithmetic.** Executable formulas use integers, Q = 1,000,000 fixed point (A1), or exact rationals. Where an intermediate exceeds `i128`, the design names **U256**: a fixed 256-bit unsigned type with checked multiply/add. U256 lives in a new `Z/wide.rs` and is never `f32`/`f64`. Floats, `HashMap`, wall clocks and unordered iteration are forbidden in `Z/` (A1; worker-common).
- **Java `double` sites.** These are reproduced by the **binary64 emulation** `fl(x)` in §6.2. It is defined on exact rationals and is integer-only. It is used only where the pinned code truncates a `double` that is observable in integers (vendor price; manor crop fee). Everywhere else, pinned `double` math becomes an **exact-rational adaptation**. Each one is named in the formula ledger (§14.2).
- **`Rnd`.** The pinned `com.l2jserver.commons.util.Rnd` lives in l2j-commons, which is **not in the pinned clones** (`grep -rn "import .*\.Rnd;"` resolves outside GJ/). Its exact semantics are therefore **UNVERIFIED**. The design treats `Rnd.get(n)` / `Rnd.nextInt(n)` as uniform on `[0, n)`, `Rnd.get(a, b)` as uniform on `[a, b]` inclusive, and `Rnd.nextDouble()` as uniform on `[0, 1)`. These are the conventional L2J meanings; the Nightfall adaptations below fix exact integer semantics regardless.
- **Distances.** Every pinned distance check used here is 3D (`calculateDistance(…, true, …)`, `isInsideRadius(…, checkZ=true, …)`, `Util.checkIfInRange(…, true)`). The pickup pawn check is 2D. **Nightfall zones are 2D (`Vec2Fixed`, `Z/entity.rs:71`).** Every check is the 2D projection with `dz = 0`. This is recorded once here as adaptation **X1** (R38).
- **L2 units → milli-tiles.** `m(L) = trunc0(L × 1000 / 32)`: integer division truncating toward zero. This is the `weapon_reach` conversion (`Z/combat.rs:303-313`, `L2_UNITS_PER_TILE = 32` at `Z/combat.rs:23`, `UNITS_PER_TILE = 1000` at `Z/fixed.rs:13`). Truncation toward zero makes negative offsets symmetric (adaptation **X2**, R39). Values: 20 → 625; 25 → 781; 36 → 1,125; 70 → 2,187; 150 → 4,687; 1,400 → 43,750; 1,600 → 50,000.
- **Radius tests** are squared and exact in `i128`. A non-strict pinned `<=` becomes `dx² + dy² ≤ R²`. A pinned `> R` reject becomes reject iff `dx² + dy² > R²`. `Util.checkIfInRange(range, a, b, true)` is `d − rad/2 ≤ range`, with `rad` the sum of the collision radii (`GJ/util/Util.java:208-232`). It becomes `4·(dx²+dy²) ≤ (2·R + rad)²`, with `R = m(range)` and `rad` the sum of the engine `collision_radius` values in milli-tiles. Players carry `collision_radius = 0` (`Z/state.rs:2165`); NPCs carry their template value.
- **Ticks.** 1 tick = 100 ms. `s` seconds = `10·s` ticks. 15 s = 150; 60 s = 600; 600 s = 6,000; 900 s = 9,000; 5 s = 50.
- **UNVERIFIED** marks any pinned fact the author could not confirm. **QUESTION:** items are collected in §15. **Adaptation Xn / Rule Pn** labels number the Nightfall-specific choices; §14.2 lists them all.

### Reconciliation of rejected or wrong claims

| Claim in earlier drafts, briefs or binding text | Verified pinned fact | Binding treatment here |
|---|---|---|
| u128 is wide enough for drop tests (43c02a6 §3.1) | With multiplier 1, the widest intermediate over every real row is **148 bits** (reduced-fraction scan of all 63,309 drop rows × every killer-modifier value; §14.1 family D-W) | U256 checked arithmetic. Normative width test (§3.2, E22). |
| Chance test always drawn / skipped at ≥ 100 % (two conflicting variants) | Ungrouped: chance draw always; precise branch iff `chance > 100`; remainder draw always inside it (`GJ/model/drops/strategy/IDropCalculationStrategy.java:32-47`). Multi-item group: group draw always, selection draw always on pass, remainder iff `normalized ≥ 100`, drawn even at remainder 0 (`IGroupedItemDropCalculationStrategy.java:59-85`) | §3.2 normative pseudo-code: every pinned draw is taken, including quantity at `min = max` (rule P3). |
| Group chance = group × rate × gap | Normalized chance `Σ_i item_i.getChance(victim) × group.chance × killerMod / 100` (`GJ/model/drops/GroupedGeneralDropItem.java:193-215`); 151 multi-item groups have inner sums ≠ 100 | §3.2 uses the pinned normalization. |
| Position draws interleaved with item rolls | All corpse and death rolls complete first (`GJ/model/actor/L2Attackable.java:838-842`). `dropItem` then runs per resulting holder, and per instance for non-stackables (`GJ/model/actor/L2Npc.java:1458-1496`) | §2.3 order. |
| PK drops get 15 s DropProtection (VR5 A39; approved default) | `onDieDropItem` calls the 4-argument `dropItem("DieDrop", item, killer, true)` (`GJ/model/actor/instance/L2PcInstance.java:4541`), which passes `protectItem = false` (`:3227-3229`). `Inventory.dropItem` clears the owner (`GJ/model/itemcontainer/Inventory.java:123-142`). **Pinned PK drops have neither protection.** | Pinned behaviour is the default; the approved-default 15 s rests on VR5's misreading → **QUESTION Q1**. |
| Sweeper never ends the corpse (DR1 F35, DR2) | Skill 42 carries a second effect, `ConsumeBody` (`DP/stats/skills/00000-00099.xml:1490-1492`). It calls `endDecayTask()` (`DPJ/handlers/effecthandlers/instant/ConsumeBody.java:42-47`; `GJ/model/actor/L2Npc.java:1128-1133`) | A swept corpse is removed at the sweep impact (§5.3). Both reviews were wrong here. |
| Restock dormant | 2,145 of 20,236 merchant-script entries carry `delay > 0` and `count > 0` (author's parse; E18) | §6.4 implements restock. |
| `1 + baseTax + castleTax` order | `price * (1 + (baseTaxRate + castleTaxRate))` (`GJ/network/clientpackets/RequestBuyItem.java:182`) | §6.2 emulates exactly. 362 of the 5,803 real (item, tax) pairs differ from naive exact-rational truncation. |
| Crops replace drops (B1) | Harvesting skill on the seeder's corpse (`DPJ/handlers/effecthandlers/instant/Harvesting.java:48-95`) | E13 binds; §10.6. |
| Mail expiry returns attachments to the sender's mailbox (A12) | Expiry moves attachments to the sender's **private warehouse** and hard-deletes the message (`GJ/instancemanager/tasks/MessageDeletionTask.java:51-74`; `GJ/model/itemcontainer/Mail.java:78-89`) | §10.3 follows pinned. A12's wording is superseded. |
| COD paid adena goes back via a mail | Receiver pays; sender online gets `addAdena`, sender offline gets an inventory adena row (`GJ/network/clientpackets/RequestPostAttachment.java:161-228`). No message is sent | §10.3 uses a pending-delivery container (rule P19) → QUESTION Q4. |
| Freight capacity uses the target's `FREIGHT_LIM` | `RequestPackageSend` builds `new PcFreight(objectId)` with `_owner = null`, so the capacity is the base 200 only (`GJ/network/clientpackets/RequestPackageSend.java:113,137`; `GJ/model/itemcontainer/PcFreight.java:34-38,66-68`) | §10.2. |
| 583 productionRare | 582 elements; 583 is the line count including the comment at `DP/recipes.xml:4` | 582 (E1). |
| SowSeed intent | A seed is an item use: handler `Seed` (`DPJ/handlers/itemhandlers/Seed.java:42-90`) → skill 2097 → effect `Sow` | Uses the existing `UseItem` intent (W tag 21) under V7 (§10.6); no new intent. |

## 1. Identity, canonical state and hydration

### 1.1 Modules and ownership

**New contract.** Add the following to `Z/mod.rs` and to its deterministic SOURCES scan (worker-common):

| Module | Contents |
|---|---|
| `wide.rs` | U256 |
| `binary64.rs` | `fl` emulation (§6.2) |
| `phase5.rs` | `Phase5Zone`, `Phase5Snapshot` |
| `phase5_rules.rs` | rules and hash |
| `phase5_checkpoint.rs` | per-character checkpoint |
| `phase5_projection.rs` | ledger and container projection |
| `ground.rs` | ground items |
| `drops.rs` | death and corpse resolution |
| `loot.rs` | pickup and party distribution |
| `party.rs` | |
| `vendor.rs` | |
| `multisell.rs` | |
| `craft.rs` | |
| `trade.rs` | |
| `store.rs` | private stores |
| `containers.rs` | warehouse, freight, mail, auction, manor, pending deliveries |
| `state_phase5_*.rs` | the actor-side phase hooks |

Extend `ZoneState` with `phase5: Option<Phase5Zone>` beside `phase3`/`phase4`, exactly as Phase 4 did (P4Z §1.1). Do not add fields to the legacy `Entity`, `NpcAi` or `CombatState`: those are digested by V5/V6 (`Z/state_digest.rs:451,504`; `Z/ai.rs:63-77`). Every Phase 5 field lives in the V7 sidecar. Reuse the single tick draft, the effective-stat cache, the checkpoint lane, the item allocator and the RNG stream. Add no economy actor, timer task or independent DB writer (A1, A2).

Authority (A2 as amended by B2/B6/E14):
- **Zone-canonical, in the snapshot:** ground items, the NPC economy sidecar, parties, transaction requests, trade sessions, private stores, vendor stock, refund tabs, recipe books, and pending loot-mode votes.
- **DB-only rows, mutated only inside a zone checkpoint transaction under an API lease with frozen pre-images (§10.0):** warehouse, freight, mail, item auction, manor, and pending deliveries.

### 1.2 Ground items (B5)

```rust
/// A ground item is a zone entity (EntityKind::GroundItem, wire ENTITY_KIND_GROUND_ITEM = 3)
/// that CARRIES exactly one ItemInstanceId (B5). It has no separate id space.
pub struct GroundItem {
    pub entity: EntityId,          // derived, rule P1
    pub item: ItemInstance,        // Phase 4 scalar record (Z/item.rs:192-205); location = Ground (§11.2)
    pub pos: Vec2Fixed,
    pub dropped_tick: Tick,
    pub source: GroundSource,      // NpcLoot { npc: EntityId } | PkDrop { victim: EntityId }
    pub loot_owner: Option<LootOwner>,        // ItemTable "loot" ownerId mechanism (§3.7)
    pub drop_protection: Option<DropProtect>, // DropProtection mechanism (§3.7)
    pub decay_due: Option<Tick>,   // None = never auto-destroyed (player drop)
}
pub struct LootOwner { pub owner: EntityId, pub until: Tick }   // ownerId + ResetOwner schedule
pub struct DropProtect { pub owner: EntityId, pub until: Tick }  // _owner + PROTECTED_MILLIS_TIME
```

**Rule P1 (ground entity identity).**
- `entity = uuid_v8(SHA-256("nightfall.ground.1\0" ‖ zone u32 LE ‖ epoch u64 LE ‖ item u64 LE ‖ dropped_tick u64 LE)[0..16])`, with RFC 9562 version 8 and the variant bits set.
- The incarnation is always 1: each ground appearance is a new entity.
- The derivation draws no RNG. It cannot repeat, because one `ItemInstanceId` reaches the ground at most once per tick.
- A collision with any live entity is an invariant failure (corruption, fail closed), never a retry.

`ItemInstance.count` follows pinned creation: a non-stackable item has count 1 (`GJ/datatables/ItemTable.java:226-228`). Herbs keep `count` and are destroyed on pickup (§4.3). They are ordinary instances with a real `ItemInstanceId` (DR2 R17).

### 1.3 NPC economy sidecar and corpses (F11, R25)

The engine already owns corpse timing through `NpcAi.corpse_until` (`Z/ai.rs:75-76`). It is set in `npc_died` and gates respawn at `≥ corpse_until.next()` (`Z/state_ai.rs:239-271`). Phase 5 keeps that single timer and adds a V7 sidecar keyed by the NPC's `EntityId`:

```rust
pub struct NpcEconomy {
    pub spoiler: Option<EntityId>,         // set by Spoil on the LIVING monster (Spoil.java:62)
    pub seed: Option<SeedBinding>,         // Seed handler: seed + seeder (L2Attackable.java:1289-1294)
    pub seeded: bool,                      // set by a successful Sow (L2Attackable.java:1257-1283)
    pub harvest: Option<(u32, u64)>,       // crop template, count; one-shot (:1001-1003)
    pub sweep: Option<Vec<(u32, u64)>>,    // ItemHolders rolled at death; None = nothing / taken (:838-840, :994-996)
    pub first_cc: (),                      // command channels do not exist before Phase 7: vacuous (L2Attackable.java:234-247)
}
```

- The sidecar exists from spawn and is cleared at respawn: `onSpawn` clears the harvest, seed and seeder (`GJ/model/actor/L2Attackable.java:1205-1212`).
- The spoiler is **not** stored with a party. `checkSpoilOwner` evaluates the sweeper's current party through `isInLooterParty(spoiler)`, which requires the spoiler to be online (`L2Attackable.java:1027-1035`; `GJ/model/actor/instance/L2PcInstance.java:6650-6663`).
- Sweep items are template/count holders. They have **no** `ItemInstanceId` until a sweep creates them (`DPJ/handlers/effecthandlers/instant/Sweeper.java:60-66`).

**V7 corpse time.** `corpse_until = death_tick + 10 × T + E`, where:
- `T` is the template `corpseTime`, or `DefaultCorpseTime = 7` (`GC/npc.properties:86`; `GJ/taskmanager/DecayTaskManager.java:55-59`). It is imported per NPC into Phase5Rules; the author counted 234 overrides.
- `E = 100` iff `spoiler.is_some() || seeded`, else `E = 0` (`SpoiledCorpseExtendTime = 10`, `GC/npc.properties:90`; `DecayTaskManager.java:60-64`).

Under V7, `npc_died` takes `corpse_ticks` from this rule instead of `brain.corpse_decay_ticks`. V5/V6 keep the existing line (`Z/state_ai.rs:251-252`). The respawn rule `respawn ≥ corpse_until.next()` is unchanged.

A **Sweeper** impact removes the corpse at that tick (`ConsumeBody`, reconciliation table): `corpse_until := tick`, and `EntityDespawn` follows in this tick's output. The respawn tick computed at death is **not** moved earlier (rule P2; it was already ≥ the old `corpse_until + 1`).

### 1.4 Party substrate (B9, B15, F4, R15)

```rust
pub struct Party {
    pub id: PartyId,                      // PartyId(u64): zone-local allocator in Phase5Snapshot, from 1
    pub members: Vec<EntityId>,           // pinned list order; index 0 is the leader (L2Party.java:90,848)
    pub distribution: LootMode,           // 0..4 (GJ/enums/PartyDistributionType.java:25-29)
    pub item_last_loot: u32,              // BY_TURN cursor; never adjusted on membership change (L2Party.java:98,175-192)
    pub loot_vote: Option<LootVote>,      // pending change request (L2Party.java:94-97,851-906)
    pub pending_invite: Option<Tick>,     // _pendingInviteTimeout (L2Party.java:139-151)
}
pub struct LootVote { pub mode: LootMode, pub answers: BTreeSet<EntityId>, pub expires: Tick }
```

`PartyId` is a `u64` from a zone-local monotone allocator. It is not a UUID, because parties never leave the zone (§4.6). The canonical member order is the pinned list order:
- join appends (`L2Party.java:287`);
- leave removes and shifts (`:350`);
- a leader change swaps positions 0 and p (`:451-482`).

This order is the index space for RANDOM selection, the BY_TURN cursor and the adena remainder (§4).

### 1.5 Transaction requests and trade sessions (E10, R18, R33)

The pinned client has one request slot per player, shared by trade and party invites:
- `onTransactionRequest`, `_requestExpireTime = ticks + 15·TICKS_PER_SECOND` (`GJ/model/actor/instance/L2PcInstance.java:354,5038-5041`);
- `setActiveRequester` (`:5002-5009`);
- `isProcessingRequest` / `isProcessingTransaction` (`:5023-5030`).

```rust
pub struct TxRequest { pub requester: EntityId, pub target: EntityId, pub kind: TxKind, pub expires: Tick }
pub enum TxKind { Trade, PartyInvite { mode: LootMode } }
pub struct TradeSession {
    pub a: EntityId, pub b: EntityId,     // a = requester; canonical pair key (min, max)
    pub list_a: TradeList, pub list_b: TradeList,
}
pub struct TradeList { pub items: Vec<(ItemInstanceId, u64)>, pub confirmed: bool, pub locked: bool }
```

- At most one `TxRequest` per target and one outstanding request per requester (pinned single-requester rule). The answer intents therefore need no requester id (R33).
- `expires` applies **only** to the pending request (`L2PcInstance.java:5047`). An open trade has no timeout (R18).
- A trade has no TradeLock step and no "remove item" packet (DR2 R18; client packets are only `TradeRequest`, `AnswerTradeRequest`, `AddTradeItem`, `TradeDone`).

### 1.6 Private stores (B2, R22)

```rust
pub enum StoreKind { Sell, PackageSell, Buy, Manufacture }   // PrivateStoreType SELL, PACKAGE_SELL, BUY, MANUFACTURE
pub struct PrivateStore {
    pub kind: StoreKind, pub title: String,                   // ≤ 29 chars (SetPrivateStoreMsgSell.java:31)
    pub sell: Vec<StoreSellEntry>,                            // (ItemInstanceId, count, price)
    pub buy: Vec<StoreBuyEntry>,                              // (template, enchant, count, price)
    pub manufacture: Vec<(u32 /*recipe*/, u64 /*cost*/)>,
}
```

Store state is zone-only and is closed on departure, disconnect or zone restart (B2, `OfflineTradeEnable = False` at `GC/customs.properties:190`). It is **not** restored from a zone snapshot taken while the owner is still admitted: snapshot recovery keeps it only because the owner is still admitted. A buy store has **no escrow**: adena moves at sale (`GJ/model/TradeList.java:867-877`).

### 1.7 Vendor stock (E18, R3)

```rust
pub struct VendorStock { pub count: i64, pub max: u64, pub restock_ticks: u64, pub due: Option<Tick> }
// key: (buylist_id: u64 = npcId·10^k+index per AbstractNpcAI.buildBuySellList, template: u32)
```

- `count` may go negative, as `AtomicLong` does, but is read clamped at 0 (`GJ/model/buylist/Product.java:94-100,109-119`).
- Stock is zone snapshot state only (A15). A new epoch starts every limited product at `max`.
- Pinned persistence in the `buylists` DB table (`Product.java:146-160`) uses wall-clock `next_restock_time`, which is forbidden in the zone. This is adaptation **X3**; it has no wire effect beyond the stock count.

### 1.8 Recipe books (single home, E17, F36)

`Phase5Player.recipes: { dwarven: BTreeSet<u32>, common: BTreeSet<u32> }` holds recipe ids (the `<item id>` of `DP/recipes.xml`). The zone is canonical. The only durable copy is `Phase5Checkpoint.recipes`, inside `characters.phase5_checkpoint`. There is **no** `character_recipes` table, so no dual projection exists. `GetRecipeBook` serves the checkpoint while the character is out of world and refuses `FAILED_PRECONDITION` while it is admitted (D15 pattern).

### 1.9 Refund tabs (B6)

`Phase5Player.refund: Vec<ItemInstance>` has location `Refund`, in the pinned `_items` order (oldest first).
- It is zone-only and never in the checkpoint.
- A 13th distinct entry destroys the oldest (`PcRefund.java:57-69`, `"ClearRefund"`).
- Departure destroys all entries (`PcRefund.java:77-89` via `L2PcInstance.clearRefund`, `:2539-2543,8678`).
- Zone snapshot recovery keeps the tab only for owners still admitted.
- A new epoch has no tabs. Its abandoned entries are closed by epoch reconciliation (§11.6).

### 1.10 Phase5Rules and catalogue admission (F13, R31)

`Phase5Rules::try_new` validates every cross-reference, then hashes canonical, length-delimited, typed encodings. The domain separator is `nightfall.phase5.rules.1\0`. The rules are hash-referenced exactly like Phase4Rules (D27/D31): snapshots and replay admission policy `version_three` carry only the hash, and the bytes come from the embedded `tables-phase5/` generated by E1. A hash mismatch refuses admission. Hash inputs, in this order:

1. **Drop lists.** These come from the 59 NPC files with `<death>` lists: 14,589 `<group>` blocks (3,032 of them single-item) and 744 ungrouped death items. They are kept in document order with node kind, exact decimal chance strings (≤ 20 fractional digits), min/max and template. Corpse lists come from 35 files covering 2,260 NPCs, with 7,335 ungrouped items. The totals are 63,309 rows; the min chance is `0.0003000000142492354`, the max chance `188.85499572753906`, the max count 18,000,000; no row has min 0, and 56,117 rows have `min = max`. All counts were re-counted by the author. Each NPC also carries its level, `isRaid`, `corpseTime`, `canBeSown`, the passive skill ids 4303–4310 relevant to harvest (`L2Attackable.java:1263-1272`), the `exp`/`sp` reward rationals for B16, and its collision radius.
2. **Rate chain** (§3.4). The pinned multipliers are Death/Corpse/Herb/Raid chance and amount 1, the per-item overrides `57,1` (`GC/rates.properties:16-37`), RateXp/RateSp/RatePartyXp/RatePartySp 1 (`:46-52`), RateDropManor 1 (`:53`) and RateSiegeGuardsPrice 1 (`:58`). Champion: `ChampionEnable = False` (`GC/customs.properties:10`), so no champion value is executable. Also hashed: the **global multiplier `G = num/den`** (P2; default 1/1; Nightfall config constraint `1 ≤ num, den < 2^32`, the bound used by the width proof in §3.2), and the deep-blue windows (`GC/npc.properties:187-218`).
3. **Ground constants.** Loot owner 15,000 ms (`GJ/datatables/ItemTable.java:212-216`). DropProtection 15,000 ms (`GJ/model/DropProtection.java:35`). Raid loot rights 900 s (`GC/character.properties:689`), with CC size 45 (`:693`, vacuous). AutoLoot/AutoLootRaids/AutoLootHerbs are all False (`:677,683,80`). Decay: 600 s (`GC/general.properties:258`), herbs 60 s (`:262`, seconds→ms via `GJ/config/GeneralConfiguration.java:216-218`), the per-template `auto_destroy_time` (one row, item 9849 = 10 s, `DP/stats/items/09800-09899.xml:923`; ×1000 at `GJ/model/items/L2Item.java:167`), an empty `ProtectedItems` (`:269`), `DestroyPlayerDroppedItem = False` and `DestroyEquipableItem = False` (`:306,312`), `SaveDroppedItem = False` (`:316`), `MultipleItemDrop = True` (`:354`), and the 5 s scanner (`GJ/ItemsAutoDestroy.java:35`). Offsets 70 (`GJ/model/actor/L2Npc.java:116`) and 25 (`L2PcInstance.java:3178`). The PK policy is `KarmaRateDrop 40`, Item 50, Equip 40, Weapon 10, limit 10 (`GC/rates.properties:115-127`), `MinimumPKRequiredToDrop = 6` (`GC/pvp.properties:27`), `NonDroppableItems` (31 ids, `:24`) and `PetItems` (`:20`), plus the Q1 policy flag `pk_drop_protection` (default `None`).
4. **Party constants.** Size cap 9 (`GJ/network/clientpackets/RequestJoinParty.java:149`), PartyRange 1,600 and PartyRange2 1,400 (`GC/character.properties:605,610`), even-distribute flags (`:616,624,631`), `LeavePartyLeader = False` (`:636`), the vote timeout of 15 s (`L2Party.java:88`), the BONUS_EXP_SP table (`:81-84`), and the cutoff `highfive` with gaps `0,9;10,14;15,99` and percents `100,30,0` (`GC/character.properties:769,785,793`).
5. **Herbs and catalogue closure.** There are 68 `ex_immediate_effect` EtcItems (`GJ/model/items/L2Item.java:188,743-745`). Of these, 59 carry handler `ItemSkills` with one `item_skill`, and 9 carry no handler (10213, 10648, 13009, 13029–13031, 14798–14800). On pickup these 9 are destroyed with no effect and the handler-absent log (`L2PcInstance.java:3860-3868`). The 59 handled herbs use 48 distinct item-skill ids, generated with their levels into `tables-phase5/herbs.toml`: 2244–2247, 2278–2285, 2357, 2485, 2486, 2512–2514, 2580, 2897–2903, 9161–9163, 22000–22005, 22029–22031, 22097, 22098, 22128, 22158–22164. Several herbs share a skill: 2278 ×5, 2279 ×4, 22128 ×3, 2280 ×2, 2281 ×2. Every template referenced by any drop, corpse, merchant, multisell, recipe, seed or auction row must be **storable** in the extended catalogue (D5 storability; R31). Its executability is a separate D5/D54 capability:
   - a herb whose skill is not executable makes pickup refuse `ITEM_CAPABILITY_DEFERRED` (W 36), and the herb stays on the ground until decay;
   - a non-executable recipe product is still a storable grant.
   No row is ever dropped from a table because of a capability.
6. **Merchants.** 274 AI scripts with 20,236 entries (`DPJ/ai/npc/**`, `AbstractNpcAI.buildBuySellList` at `DPJ/ai/npc/AbstractNpcAI.java:208-233`, record `BuySellList(int itemId, double tax, double delay, int count, int type)` at `:269`). Each entry carries its buylist id, NPC id, template, reference price (`getReferencePrice`), the exact `tax` literal, the `delay` literal and the count. The tax histogram is 20 % ×10,546, 10 % ×4,103, 15 % ×2,623, 100 % ×1,527, 50 % ×827, 30 % ×582, 60 % ×27 and 0 % ×1 (Harvester 5125). That gives 5,803 distinct (template, tax) pairs and 2,145 limited entries. There is one zero-price entry (8187), which is refused at purchase (§6.3). Castle tax percent is 0 (`DPS/castle.sql:17-26`, E3). The Strife ceilings 15/25/5 (`GJ/instancemanager/CastleManager.java:165-176`) are hashed as data for later phases.
7. **Multisell.** 196 lists. Hashed per list: `maintainEnchantment` (31 lists), `applyTaxes` (15), and NPC allow-lists. Hashed per entry: ingredients (incl. `isTaxIngredient`, which no pinned row sets, and `maintainIngredient`, set on 21 ingredients) and products (`GJ/model/multisell/*`, `GJ/data/xml/impl/MultisellData.java`).
8. **Recipes.** 1,000 recipes, 830 Dwarven and 170 Common. Hashed: id, recipeId, craftLevel, type, successRate, ingredients, production, productionRare (582, rarity 1 % ×31, 4 % ×315, 20 % ×236), statUse (MP/HP) and altStatChange (`DP/recipes.xml`; `GJ/data/xml/impl/RecipeData.java:61-171`). Also: Create Item 172 / Create Common Item 1320 rank tables, `CraftMasterwork = True` (`GC/character.properties:484`), recipe limits 50/50 (`:488-489`), and `AlternativeCrafting = False` (`:495`).
9. **Storage and mail.** Warehouse 100/120 (`GC/character.properties:325-326`), deposit fee 30 per entry (`GJ/network/clientpackets/SendWareHouseDepositList.java:118`), freight 200 (`:332`) at 1,000 per entry (`:335`), and `KarmaPlayerCanUseWareHouse = True` (`:443`). Mail constants: `RequestSendPost.java:60-68`, `Message.java:39-40`, and SendMailInterval 100 / TransactionInterval 10 / ManorInterval 30 (`GC/floodprotector.properties:85,99,106`).
10. **Item auction and manor.** The ItemAuctions data with its 3 instances (32320, 32321, 32322; 82 items each, all `auctionInitBid = 100000`, `auctionLength = 300`; `DP/ItemAuctions.xml:13,187,361`), the auction constants (§10.5), seeds.xml (258 crops, 129 alternative; `DP/seeds.xml`), the Sow/Harvesting skills 2097/2098 (`DP/stats/skills/02000-02099.xml:1537-1575`), and the **fixture manor schedule** (B1): seed production and crop procurement lists per castle for one approved period. Without castles, pinned ownerless castles have empty lists (`GJ/instancemanager/CastleManorManager.java:253-255,578-603`), so Phase 5 manor content exists **only** through the hashed fixture lists.
11. **Policy versions and adaptation set** X1–X12, P1–P21 and T1 (§14.2), the V7 formula revision, and the Phase 3 and Phase 4 rules hashes this epoch is bound to.

### 1.11 Snapshot, player sidecar and checkpoint

```rust
#[serde(deny_unknown_fields)]
pub struct Phase5Snapshot {
    pub rules: Arc<Phase5Rules>, pub rules_hash: [u8; 32],
    pub ground: BTreeMap<EntityId, GroundItem>,
    pub npcs: BTreeMap<EntityId, NpcEconomy>,
    pub parties: BTreeMap<PartyId, Party>, pub next_party: u64,
    pub requests: BTreeMap<EntityId /*target*/, TxRequest>,
    pub trades: BTreeMap<(EntityId, EntityId), TradeSession>,
    pub stores: BTreeMap<EntityId, PrivateStore>,
    pub stock: BTreeMap<(u64, u32), VendorStock>,
    pub players: BTreeMap<EntityId, Phase5Player>,
}   // DB-only rows (warehouse, freight, mail, auction, manor, deliveries) are NEVER snapshot state:
    // each command that touches them carries its frozen pre-image (§10.0).
pub struct Phase5Player {
    pub recipes: RecipeBook, pub refund: Vec<ItemInstance>,
    pub party: Option<PartyId>, pub trade: Option<(EntityId, EntityId)>,
    pub last_folk_npc: Option<EntityId>,               // getLastFolkNPC binding for vendor/warehouse intents
    pub flood: BTreeMap<FloodKind, Tick>,              // Transaction / SendMail / Manor / ItemAuction next-allowed ticks
}
#[serde(deny_unknown_fields)]
pub struct Phase5Checkpoint {           // characters.phase5_checkpoint jsonb (§11.4)
    pub schema_version: u32,            // 1
    pub rules_hash: String,
    pub recipes: RecipeBook,
}
```

`PbPlayerLoad` tag 11 (§12) carries `Phase5Checkpoint` bytes. A V7 player load requires the Phase 3, Phase 4 and Phase 5 blocks. Parties, trades, stores, requests and refund tabs are **never** in the checkpoint: departure ends them (§4.6, §9). Fresh admission starts with none of them.

**Invariants.** Validated at admission, snapshot restore and every commit:
- Every ground item references a catalogued template, has a valid count (non-stackable = 1), has a unique `ItemInstanceId` across all owners in the zone, and has a P1 entity id.
- Each sidecar NPC exists.
- A sweep holder exists only while the NPC is dead with `corpse_until > tick`.
- Every party has ≥ 2 members, ≤ 9 members, each member is an admitted player in exactly one party, and the leader is at index 0.
- Each trade pair is consistent with both players' `trade` field; listed instances are owned and available.
- At most one store per player; store sell entries reference owned unequipped tradeable instances.
- Refund length ≤ 12.
- Recipe counts ≤ base + `REC_D_LIM`/`REC_C_LIM`. Those stats do not exist in the Phase 3/4 stat pipeline, so they contribute 0 (R40). The same applies to `WH_LIM`, `FREIGHT_LIM`, `P_SELL_LIM` and `P_BUY_LIM`.
- Leases are disjoint.
- Allocator ownership is the Phase 4 I1 rule.

### 1.12 Snapshot budget against C10 (F24, R41)

The enforced limits are 1,047,552 encoded bytes and 33,554,432 decoded bytes (`R/record.rs:221-223`). Field-encoding bounds for the new mutable state follow. Prost varints are ≤ 10 bytes per u64 and ≤ 5 per u32; a UUID is 16 bytes plus 2 of framing.

| Component | Per-entry bound | Count bound |
|---|---|---|
| GroundItem | ≤ 196 B: entity 18 + Phase 4 instance ≤ 128 (P4Z §1.4) + pos 12 + dropped_tick 10 + source 20 + two owner pairs 2 × 30 (minus shared framing) + decay 11 | **No pinned cap.** NPC loot is bounded by decay (6,000-tick window, 5 s granularity). PK drops never decay and are bounded only by pickups and epoch end. |
| NpcEconomy | ≤ 18 + 18 + 30 + 2 + 16 + sweep (≤ 12 holders × 16) ≈ 300 B | One per AI NPC; inherits the Phase 3 C10 combatant bound (1,000) |
| Party | ≤ 10 + 9 × 18 + 2 + 5 + vote (12 + 9 × 18) + 11 ≈ 370 B | ≤ ⌊admitted players / 2⌋ |
| TradeSession | ≤ 36 + 2 × (items × 22 + 4) | ≤ admitted / 2. Items per list are unbounded in pinned (no maximum, `AddTradeItem`); the practical bound is the owner's inventory size. |
| PrivateStore | ≤ 31 (title) + entries × 32 | Sell entries ≤ 3/4 (Dwarf), buy ≤ 4/5 (`GC/character.properties:306-313`), manufacture ≤ `MAX_ITEM_IN_PACKET` (250) |
| VendorStock | ≤ 41 B | ≤ 2,145 limited products (rules) |
| Phase5Player | ≤ 100 recipes × 5 + 12 × 128 (refund) + 64 ≈ 2.1 KB | per admitted player |

With the Phase 4 reference workload (8 players) plus 1,000 corpses and parties, the fixed new state is about 8 × 2.1 KB + 1,000 × 300 B + 2,145 × 41 B ≈ 405 KB **before ground items**. This is not a proof that the snapshot fits, and the ground-item count has no static bound. The plan must keep the **mandatory C10 measurement gate (D4 style)**: encode the real worst-case soak snapshot, report the encoded and decoded bytes and the largest contributors, and fail if over the limits. The soak assumption is **an assumption, not a bound**: 1,000 live NPC loot items plus 100 PK drops. Snapshot overflow handling is the existing fail-closed path: no truncation, no NATS limit increase. Unbounded PK-drop accumulation is an availability risk recorded as **QUESTION Q2**.

### 1.13 Hydration, AOI and visibility

| Fact | Carrier | Owner | Observers |
|---|---|---|---|
| Ground item | `EntitySpawn` kind `ENTITY_KIND_GROUND_ITEM` + `GroundItemDetail` (W EntitySpawn field 30), `EntityDespawn` | public | public. Fields: item instance id, template, count, enchant, loot owner + until, drop-protect owner + until. These are static per appearance, so no observer-dependent bit and no re-emission is needed (R35). Clients compute "may I pick up" from their own party list. |
| Corpse spoiled/seeded/swept | `CorpseStateChanged` (WorldEvent 35) | public | public |
| Party | `PartyStateChanged` (36) baseline + deltas; `PartyLootVote` (37); `PartyInviteReceived` (38) | members / target | never |
| Trade | `TradeRequestReceived` (39) to the target; `TradeSessionChanged` (40) to both | participants | never |
| Private store | `PrivateStoreChanged` (41) | owner: full | AOI: kind, title, entries, prices (public in L2) |
| Mail arrival | `MailNotification` (42) | receiver only | never |
| Recipe book | `RecipeBookSync` (43) baseline after `InventorySync`, then on change | owner | never |
| Auction | `ItemAuctionStatusChanged` (44) | bidders + AOI of the auctioneer | public auction state |
| Party loot messages | `LootDistributed` (45) | party members | never |
| Operation receipts | `EconomyResult` (34) frozen receipt (D53 fields) | owner | never |
| Inventory/adena effects | existing `InventoryChanged` (W 27) / `WeightChanged` | owner | never |

Admission ordering extends P4Z §1.5:
1. identity spawns, including ground items in AOI;
2. reference hydration;
3. `SkillStateSync`;
4. `InventorySync`;
5. `RecipeBookSync`;
6. `PartyStateChanged` baseline (if any).

Ground item spawns follow the C2/C4/C12 entity fencing. No `StatsChanged` field is added; adena is inventory item 57 (F30).

## 2. V7 in-tick ordering and RNG ledger

### 2.1 Schedule amendment (real Phase 3 phases 0–10)

**New contract.** Add a V7 branch beside `execute_v5`. The driver is `run_tick_v5` / `execute_v5` (`Z/state_runtime.rs:720-900`), which runs these phases in order:
- commands `:772-785`;
- spawn `:786`;
- expiry `:787-791`;
- periodic and regen `:792-793`;
- chase and movement `:794-797`;
- cast completions and impacts `:800`;
- auto impacts `:801-804`;
- hit bookkeeping `:805`;
- new starts `:806-856`;
- AI `:857-859`;
- final flush and observe `:865-900`.

V5 and V6 keep their observable schedule, draws and bytes exactly. Every Phase 5 hook below runs only when `phase5.is_some()` under V7.

| Phase | V7 amendment |
|---|---|
| **0 — receive-order commands** | Authorize, fingerprint and deduplicate as in Phase 4 (D41–D43). Then resolve **all** Phase 4 and Phase 5 intents in strict command-ordinal order, rebuilding after each accepted mutation (F23). In-phase-0 economy resolutions: pickup (incl. herb item-skill execution and party distribution), vendor buy/sell/refund, multisell, recipe register/delete, **craft (synchronous: `AlternativeCrafting = False` runs `RecipeItemMaker.run()` inline, `GJ/RecipeController.java:141-148`)**, trade request/answer/add/confirm/cancel, store set/close/buy/sell/manufacture, party invite/answer/leave/kick/leader/vote, warehouse/freight/mail/manor/auction intents, and lease-bound system commands (pending delivery, auction schedule, epoch reconcile; §10.0). Spoil, Sweeper, Sow (via `UseItem` of a seed) and Harvest (via `UseItem` of 5125) only **stage** casts here, through the Phase 3/4 cast path. Departures run the departure hooks of §4.6/§9.5 at their command ordinal. |
| **1 — spawn** | Existing corpse expiry at `corpse_until` and due respawns (`Z/state_ai.rs:71`ff.). A swept corpse has `corpse_until = sweep tick` and is removed by the next spawn phase; its `EntityDespawn` is emitted in that phase. |
| **2 — expiry** | After the existing effect, reuse and policy expiry, in this order: (a) loot-owner and drop-protection deadlines reached (`until ≤ tick`) are cleared, with no event (§1.13: the spawn fields already carry the deadlines); (b) **ground decay**: when `tick % 50 == 0`, despawn every ground item with `decay_due ≤ tick` in `EntityId` order (§3.8); (c) expired transaction requests are dropped (pinned is lazy; no event); (d) expired loot votes finish as declined (`finishLootRequest(false)`, `L2Party.java:886-906`) with a `PartyLootVote{cancelled}` event; (e) expired pending party invitations are cleared; (f) vendor restocks with `due ≤ tick` refill. Every pass iterates in key order. |
| **3 — periodic + regen** | Unchanged. A periodic kill calls `kill()`, which carries the V7 death hook (§2.2). |
| **4 — chase / movement** | Unchanged. No trade or store movement cancel: pinned checks distance only at request and confirm (R18). |
| **5 — cast launches / impacts** | Existing total order. Effect handlers added by B11: **Spoil** (254), **Sweeper** (42, followed by **ConsumeBody**), **Sow** (2097), **Harvesting** (2098), and herb item skills. They execute at the impact in XML effect order (§5, §10.6). |
| **6 — auto impacts** | Unchanged. Deaths call `kill()` with the V7 hook. |
| **7 — hit bookkeeping** | Unchanged. Drop resolution does **not** move here: it runs inside `kill()` before the hate ledger is removed (§2.2), so no damage table outlives the kill and no existing draw is reordered relative to later impacts. |
| **8 — new starts** | The cast-start conditions of the new skills: `canSweep` (spoil owner, `!isOldCorpse` with 2,000 ms = 20 ticks, slots/weight for the spoil holders; `GJ/model/conditions/ConditionPlayerCanSweep.java:50-75`), the NPC_BODY target type, and the Seed handler checks. All of them draw nothing. |
| **9 — AI** | Unchanged. |
| **10 — durable flush / observe** | One durable draft per tick. Ledger rows in D69/D76 order as amended by rule T1 (§11.3). Container row changes and lease releases in the same revision-fenced transaction as the checkpoints and outbox (§10.0, §11.5). Visible output only after log ack and commit. Output order is the Phase 4 order (`InventoryChanged` → `EquipmentChanged` → `WeightChanged` → …), then `EconomyResult` receipts, then the Phase 5 events in emission order. |

### 2.2 The `kill()` amendment (death resolution placement, DR2 R2)

`kill()` at `Z/state_combat.rs:615-691` currently runs, in order:
1. `kill_credit` (`:629`);
2. `disengage` (`:630`);
3. dead flags and `EntityDied` (`:631-661`);
4. `npc_died` (`:663`, which draws `npc_respawn_jitter = roll_below(random + 1)` when `random > 0`, `Z/state_ai.rs:259-263`);
5. `hate.remove(&victim)` with `HateChanged` per row (`:664-675`);
6. `release_target` (`:676`);
7. the role match: `charge_death` (player) or `award_xp(credit)` (NPC) (`:677-690`).

Under V7 a single hook `death_v7(tick, victim, blow, events)` is inserted **between steps 3 and 4**: after `EntityDied`, before `npc_died`.

```text
death_v7(tick, victim, blow):
  if victim is an NPC with a sidecar:
     R := reward snapshot from self.hate[victim] (read-only, §4.5)          -- no draws
     actor := R.max_dealer if online(R.max_dealer) else acting_player(blow.attacker)   -- §3.6
     if actor is a player:
        holders := []
        if npc.spoiler.is_some(): npc.sweep := roll_list(CORPSE, victim, actor)        -- §3.2 draws
        holders := roll_list(DEATH, victim, actor)                                   -- §3.2 draws
        for h in holders (list order): spawn_npc_loot(h, actor)                      -- §3.7 position draws
     stash R for step 7
  if victim is a player: pk_drop_v7(victim, blow)                                    -- §3.9 draws
```

Then the existing `npc_died` runs. It draws the jitter **after** every drop and position draw, using the V7 corpse time (§1.3). Hate removal follows. Step 7 under V7 replaces `award_xp(credit)` with the B16 distribution computed from the stashed `R` (§4.5); it is draw-free. `charge_death` is unchanged; pinned order is drop before death penalty (`L2PcInstance.java:4406` before `:4426`). V5 and V6 keep the exact current function.

Consequences, stated as required by R2:
- the hate ledger is read before its removal, so no separate damage table survives;
- the only existing draw whose relative position changes is the respawn jitter. It now follows the new V7 drop draws of the same death, and this applies only in V7 epochs;
- the existing `roll_below` primitive (`Z/state.rs:1102-1112`, unbiased rejection over `next_u32`) is used for every bounded integer draw.

### 2.3 Normative per-callsite RNG draw order

Draw primitives, defined once:
- **`roll_below(n)`** is the existing primitive. It consumes ≥ 1 `next_u32` word.
- **`draw_u64()`** is new: `hi = next_u32(); lo = next_u32(); (hi << 32) | lo` (high word first). It is used **only** for E15 chance tests.
- **`chance(num, den)`**: given an exact positive rational chance in percent, `r = draw_u64()`; success iff `r · (100·den) < num · 2^64`, evaluated in U256. Exactly one `draw_u64` per call, even when the chance is 0 or ≥ 100 (rule P3).
- **`chance_bound(k, n)`**: the integer comparisons `Rnd.get(n) < k` and `Rnd.nextInt(n) < k`, converted under E15 to `chance(k·100, n)`. For example, `Rnd.nextInt(99) < 90` becomes success iff `r·99 < 90·2^64`.

Evidence for every draw uses the Phase 4 Rng fact shape extended in §14.3.

| # | Callsite (phase) | Draw(s), in order | Skipped when |
|---|---|---|---|
| 1 | Spoil 254 impact (5) | The Phase 3 MagicSuccess draw `roll_below(100) < rate` with `rate = magic_success_rate(...)` (`Z/skill_math.rs:710-739`; `GJ/model/stats/Formulas.java:1286-1313`), drawn **before** the ALREADY_SPOILED check (`Skill.java:935`; `Spoil.java:38-41` vs `:57`) | Cast cancelled before impact |
| 2 | Corpse list at death (`death_v7`) | Per `<item>` in document order: `chance` → (if `C > 100`) remainder `chance` → quantity `roll_below(max−min+1)` | `spoiler = None` (whole list); chance failure skips that item's remainder and quantity |
| 3 | Death list at death | Per node in document order: ungrouped item as #2. Single-item group: as #2 with the collapsed chance (§3.2). Multi-item group: group `chance` → (pass) selection `draw_u64` → (if `G ≥ 100`) remainder `chance` → quantity `roll_below` | Group failure skips selection, remainder and quantity |
| 4 | NPC loot positions | For each holder in list order, for each created instance (1 if stackable, else `count` instances, `MultipleItemDrop = True`): `dx = roll_below(141) − 70`, then `dy = roll_below(141) − 70` (`L2Npc.java:1460-1463`) | A holder with count 0 creates nothing (no pinned row has min 0) |
| 5 | Respawn jitter (existing) | `roll_below(random+1)` | `random == 0` |
| 6 | PK drop at player death (`death_v7`) | Gate `chance_bound(40, 100)`. Then per eligible inventory item in ascending `ItemInstanceId` order (adaptation X4): unequip if equipped (no draw) → item `chance_bound(p, 100)` → on success `dx = roll_below(50) − 25`, `dy = roll_below(50) − 25` (`L2PcInstance.java:3178`). Stop after the 10th drop (`:4549-4550`) | Trigger false (§3.9); filtered items draw nothing and are not unequipped (`:4524-4531`) |
| 7 | Herb item skill at pickup (0) | Handler-specific draws of the herb skill's effects in P3 handler order (§4.3) | No handler / not executable |
| 8 | Party RANDOM looter (0 or 5) | `roll_below(k)` over the in-range candidates in member order (`L2Party.java:159-170`) | Not RANDOM mode, or no candidate (looter = picker) |
| 9 | Even split remainder (0 or 5) | `leftOver` successive `roll_below(k)` draws (`L2Party.java:609-616`) | `leftOver = 0` |
| 10 | Craft success (0) | `chance_bound(successRate, 100)` (`GJ/RecipeController.java:367`) | Pre-roll aborts (§8) |
| 11 | Masterwork (0) | `chance_bound(rarity, 100)` (`:569-570`) | Craft failed, or no productionRare (rarity 0 still draws: `_rareItemId` is 0 ≠ −1 for non-rare recipes, `L2RecipeList.java:56,83-87`, so the gate passes and `Rnd.get(100) < 0` is drawn; rule P4) |
| 12 | Sow (5) | `chance_bound(basic, 99)` (`DPJ/handlers/effecthandlers/instant/Sow.java:87-116`) | Pre-check fails (`:58`); the seed is consumed before the draw (`:63-66`) |
| 13 | Harvest (5) | `chance_bound(basic, 99)` (`Harvesting.java:98-119`) | Seeder mismatch or not seeded (`:54,93`) |
| 14 | Auction item selection (0, schedule command) | `roll_below(items.len())` (`GJ/model/itemauction/ItemAuctionInstance.java:430-437`) | — |
| 15 | Manor crop fee shortfall | none: `RequestProcureCropList` draws nothing | — |
| 16 | Mature-crop rounding at period change | not executed in Phase 5: ownerless castles skip it (`CastleManorManager.java:253-255`); the fixture schedule has no castle owner | — |

Within one tick, the callsites interleave only through the phase order of §2.1 and the existing per-phase total orders. No Phase 5 draw happens in phases 1, 2, 4, 7, 8 or 9.

## 3. Monster drops, PK drops and ground lifecycle

### 3.1 Data model and pinned evaluation structure

- `L2NpcTemplate.calculateDrops(scope, victim, killer)` walks **one** list per scope in document order, with `<group>` and `<item>` nodes mixed (`GJ/model/actor/templates/L2NpcTemplate.java:437-458`; parse at `GJ/data/xml/impl/NpcData.java:452-479`).
- DEATH items use `IAmountMultiplierStrategy.DROP` / `IChanceMultiplierStrategy.DROP`. CORPSE items use `SPOIL`. Groups are created with the DEATH group factory (`GJ/model/drops/DropListScope.java:31-32`).
- `PreciseDropMultipleGroupRolls = False` selects `IGroupedItemDropCalculationStrategy.DEFAULT_STRATEGY` (`GroupedGeneralDropItem.java:50-52`; `GC/general.properties:341-350`). `PreciseDropCalculation = True` makes `isPreciseCalculated()` true (`IPreciseDeterminationStrategy.java:31`).
- `doItemDrop` rolls CORPSE first, only if spoiled, into `_sweepItems`. It then rolls the whole DEATH list into a holder list, and only then creates ground items holder by holder (`L2Attackable.java:824-856`).
- AutoLoot branches are dead under the pinned False flags (`:848-852`). Champion items are dead under `ChampionEnable = False` (`:869`).
- Cursed weapons (`CursedWeaponsManager.checkDrop`, `:836`) are **out of scope**: no cursed weapon exists in the Phase 5 catalogue, so the call is vacuous and draws nothing.

### 3.2 Exact-rational chance resolution (E15, E22; F1/F2, R1/R2)

Notation. `c` is a row's XML chance as an exact decimal rational `n/10^k` (k ≤ 20), reduced. `L` is the killer modifier (§3.3). `M(item)` is the chance multiplier chain (§3.4); it includes the global `G`. `A(item)` is the amount multiplier (§3.4). All are exact reduced rationals. `chance(N, D)` is defined in §2.3.

```text
roll_ungrouped(row, victim, killer):           // IDropCalculationStrategy.DEFAULT_STRATEGY (:32-47)
   C := c(row) × M(row) × L_nongroup(row)      // GeneralDropItem.getChance(victim,killer) (:184-186) with DEFAULT_NONGROUP_STRATEGY
   if !chance(C): return none                  // `chance > nextDouble·100`, always drawn
   mult := 1
   if C > 100:                                 // STRICT (:36)
       mult := floor(C / 100)                  // `(int) chance / 100`
       if chance(C − 100·floor(C/100)): mult += 1   // `chance % 100 > nextDouble·100`, always drawn in this branch
   q := min'(row) + roll_below(max'(row) − min'(row) + 1)   // Rnd.get(min', max'); drawn even when min' = max' (rule P3)
   return holder(template, q × mult)

roll_group(group, victim, killer):              // IGroupedItemDropCalculationStrategy.DEFAULT_STRATEGY (:58-85)
   if group.items.len() == 1:                   // getSingleItem (:51-56)
       row' := row with c' = c(item) × c(group) / 100, killer strategy = the GROUP's (IKillerChanceModifierStrategy.DEFAULT_STRATEGY)
       return roll_ungrouped(row', victim, killer)          // uses the STRICT > 100 rule
   L := L_group(victim, killer)                 // IKillerChanceModifierStrategy.DEFAULT_STRATEGY (:33-43)
   w_i := c(item_i) × M(item_i)                 // item.getChance(victim) (:173-175)
   G := Σ_i w_i × c(group) × L / 100            // normalizeMe(victim, killer, true, 1) (:193-200)
   if !chance(G): return none                   // always drawn
   u := draw_u64()                              // the selection `random = nextDouble·100`, always drawn on pass
   S := Σ_i w_i                                 // normalized share_i = 100·w_i / S (:207-208)
   i* := the first i with  u · S < (Σ_{j≤i} w_j) · 2^64       // `totalChance > random` (:68-71), U256
   mult := 1
   if G ≥ 100:                                  // NON-STRICT (:73)
       mult := floor(G / 100)
       if chance(G − 100·floor(G/100)): mult += 1   // drawn even when the remainder is 0
   q := min'(i*) + roll_below(max'(i*) − min'(i*) + 1)
   return holder(template(i*), q × mult)
```

- `min'`/`max'` are `(long)(min × A)` and `(long)(max × A)`: `floor` of the exact product (`GeneralDropItem.java:138-157`). In normalized groups they are computed once with the victim's amount strategy (`GroupedGeneralDropItem.java:207`).
- `S > 0` whenever `G > 0`. When `G = 0` (e.g. `L = 0`) the group draw is still taken and always fails. The Java normalization divides by zero only in that branch, which never reaches the walk.
- The exact walk always selects (u < 2^64 and the prefix ends at S). The pinned double walk can fall off the end with probability ≈ 2^-53 and return nothing. Recorded as adaptation **X5**.
- **Quantity width.** The maximum pinned `max` is 18,000,000. With `A = 1` the span fits `roll_below`'s `u32` bound. A Phase5Rules validation rejects any row whose `max' − min' + 1 > 2^32` under the configured amount multipliers (pinned: all 1).

**Width proof obligation (E22).** Every comparison is evaluated in U256:
- `r · 100 · D < N · 2^64`, with `C = N/D` reduced;
- the walk `u · S < P · 2^64`, cross-multiplied over the reduced denominators.

The author scanned all 63,309 real rows against every `L` value of §3.3 with `G = 1`. The widest intermediate is **148 bits** for chance tests and **98 bits** for the walk. With the configuration bound `num, den < 2^32` on `G`, the bound is ≤ 148 + 32 + 3 = 183 bits; the + 3 is the dormant champion ×8. That is below 256 with margin. The normative unit test **D-W** (§14.1) recomputes the maximum bit width over all rows, all `L` and the extreme `G` values. It fails if any intermediate exceeds 255 bits, and it fails if any implementation uses `u128`/`i128` for these products.

### 3.3 Killer (deep-blue) modifier

`d = victim.level − killer.level` (`IKillerChanceModifierStrategy.java:34,46`), where `killer` is the drop actor of §3.6.

| Strategy | Victim | Exact rational `L` |
|---|---|---|
| Group (`DEFAULT_STRATEGY`, also single-item groups) | raid (`UseDeepBlueDropRulesRaid = True`, `GC/npc.properties:191`) | `clamp((15·d + 100)/100, 0, 1)` (`:35-37`; Java `d·0.15 + 1` → exact, adaptation X6) |
| Group | non-raid (`UseDeepBlueDropRules = True`, `:187`) | `map(d, −10, −5, 10, 100)/100` = `1` if d ≥ −5; `1/10` if d ≤ −10; else `(18·(d+10) + 10)/100` → 82/64/46/28 % at d = −6…−9 (`GJ/util/Util.java:785-788`; `GC/npc.properties:210-218`) |
| Ungrouped (`DEFAULT_NONGROUP_STRATEGY`) | any; adena (57) | `map(d, −15, −8, 10, 100)/100` = `(90·(d+15)/7 + 10)/100` in the window (`:47-49`; `GC/npc.properties:197-205`) — exact sevenths |
| Ungrouped | any; other items | the item window above (`:50`). Raids use the window, not the clamp (`:44-50`; F32) |

All 2,309 pinned adena rows are grouped (VR5 A29), so the adena window is reachable only by ungrouped adena rows. Pinned data has none; the window is implemented and oracle-tested.

### 3.4 Rate chain and global multiplier (B14, E21, R14)

Chance multiplier `M(item)` for a scope's default rate `R_scope` (DEATH: `DeathDropChanceMultiplier`; CORPSE: `CorpseDropChanceMultiplier`). Pinned `IChanceMultiplierStrategy.DEFAULT_STRATEGY` is at `IChanceMultiplierStrategy.java:48-66`.

```text
M(item) = champion × branch(item) × G
champion = 1                                            (ChampionEnable = False; victim.isChampion() never true)
branch(item) = override[item]   if item ∈ DropChanceMultiplierByItemId   (57 → 1, rates.properties:37)
             | HerbDropChanceMultiplier (1, :28)  else if template.ex_immediate_effect
             | RaidDropChanceMultiplier (1, :29)  else if victim.isRaid
             | R_scope (1, :24 / :26)              otherwise
```

`G` multiplies **every** branch: the override branch (adena), herb, raid, death and corpse (E21). It enters exactly where the pinned per-category rate enters, so the surplus above 100 % becomes the precise amount multiplier through `mult` (B14). The pinned `float multiplier` (`:50`) is replaced by exact rationals (adaptation X6). All pinned values are 1, so this is exact.

Amount multiplier `A(item)` (`IAmountMultiplierStrategy.java:38-56`) has the same branch order with `DropAmountMultiplierByItemId 57,1`, Herb/Raid/Death/Corpse amount 1 (`GC/rates.properties:16-21,36`). **G does not multiply A** (B14: amounts are not separately multiplied).

XP-type rates (P2): `G` multiplies `RateXp` and `RateSp` where they enter the NPC base reward (`L2Npc.getExpReward`/`getSpReward`, `GJ/model/actor/L2Npc.java:961-970`). `RatePartyXp`/`RatePartySp` (`GC/rates.properties:50-52`) are party-bonus scalers. They are **not** multiplied again (rule P5: no double application). `RateDropManor` (crop count, integer) and `RateSiegeGuardsPrice` are not drop chances and are not multiplied.

### 3.5 Holders and ground creation for NPC loot

For each holder `(template, q)` in list order, `spawn_npc_loot` mirrors `L2Npc.dropItem` (`:1458-1496`):
- for `i in 0..q`, and only once if the template is stackable (`MultipleItemDrop = True`, `:1491-1493`):
  1. take the two position draws (#4);
  2. allocate one `ItemInstanceId` from the zone range (P4Z I1). The count is `q` if stackable, else 1 (`ItemTable.java:226-228`);
  3. set the position `npc.pos + (m(dx), m(dy))` (X2). If the point is outside `ZoneBounds` (`Z/state.rs:152-188`), use `npc.pos` instead (rule P6; L2 has no zone edge; the draws are still consumed);
  4. set the loot owner and drop protection (§3.7) and the decay (§3.8);
  5. emit `EntitySpawn`.

Ledger: one `loot` row under owner `ground:<entity>`, `before = None` (fresh, D67) (§11.3).

### 3.6 Drop actor: maxDealer with lastAttacker fallback (F20, R23)

Pinned `calculateRewards` (`L2Attackable.java:332-379`):
- iterate the aggro entries;
- take `attacker = info.getAttacker().getActingPlayer()` (summons fold into owners);
- keep entries with `damage > 1` and attacker within `PartyRange` 1,600 of the NPC (`checkIfInRange(..., true)`);
- accumulate per player; track `maxDealer` with strict `>` (`:369-372`);
- then `doItemDrop(maxDealer != null && maxDealer.isOnline() ? maxDealer : lastAttacker)` (`:379`).

`doItemDrop` returns with no drops when the actor's acting player is null (`:825-834`).

**Nightfall V7 adaptation X7.**
- The aggro list is the NPC's `HateLedger` (`Z/combat.rs:323-327`), a `BTreeMap<EntityId, HateEntry>` with `damage: u64`.
- Entries are visited in ascending `EntityId` order. Only player entries with `damage > 1` and `4·dist² ≤ (2·50,000 + npc_radius)²` count.
- `maxDealer` is the first entry reaching the strict maximum. That is the lowest `EntityId` among equal maxima: the pinned first-max-wins rule over a defined order. No timestamp state is needed (R23).
- Summons do not exist in the engine, so the fold is vacuous.
- *online* means an admitted player entity with a live session binding in this zone at resolution; dead players are online.
- `lastAttacker` is `blow.attacker` of the killing `Blow` (`Z/state_combat.rs:655`). A `kill()` with `blow = None`, or a non-player killer, yields no drops.

### 3.7 The two protection mechanisms (F21, R12, E7)

| Mechanism | Pinned | Set at | Check at pickup | Expiry |
|---|---|---|---|---|
| **Loot owner** (`ownerId`, `ResetOwner`) | `ItemTable.createItem("Loot")`: non-raid with `AutoLoot = False` → owner = drop actor, 15,000 ms. Raid: owner = first-CC leader for 900 s **only** if a first CC exists; otherwise no owner (`ItemTable.java:203-216`) | NPC loot only. PK drops: `Inventory.dropItem` sets owner 0 (`Inventory.java:134`) | Reject iff `owner ≠ 0 ∧ owner ≠ picker ∧ ¬isInLooterParty(owner)`. `isInLooterParty` needs the owner **online** and accepts the owner's party, or its CC when in one (`L2PcInstance.java:3824-3842,6650-6663`) | `until = dropped_tick + 150` (raid: + 9,000). Cleared in phase 2 at `until ≤ tick` |
| **DropProtection** | `item.getDropProtection().protect(player)` for every NPC drop with a non-null player, raids included (`L2Npc.java:1476-1478`; E7). 15,000 ms (`DropProtection.java:35`). PK drops: not applied (4-argument `dropItem`, reconciliation) | NPC loot (and PK drops only if Q1 resolves to 15 s) | `tryPickUp`: pass iff unprotected, or picker == owner, or owner's party == picker's party. No online requirement, no CC (`DropProtection.java:52-70`) | `until = dropped_tick + 150` |

Pickup check order (pinned `doPickupItem`, `L2PcInstance.java:3798-3856`):
1. the item is visible: exists on the ground;
2. DropProtection;
3. capacity, **only** when solo or FINDERS_KEEPERS (`:3814`): `validateCapacity(item)`, which counts slots only, not weight (rule P7). Phase 4 `InventoryLimits` is used with weight unchecked;
4. loot owner;
5. combat flag: vacuous.

Command channels do not exist until Phase 7, so the CC branches and the raid first-CC rights are vacuous; raid NPC loot carries DropProtection only.

### 3.8 Decay (F26, R24)

| Ground item | Limit (ms) | Source |
|---|---|---|
| Template with `auto_destroy_time > 0` (item 9849: 10,000) | that value | `ItemsAutoDestroy.java:57-65`; `L2Item.java:167` |
| Herb (`ex_immediate_effect`) | 60,000 | `:66-74`; `GC/general.properties:262` |
| Other NPC loot | 600,000 | `:75-85`; `:258` |
| PK / player drops | never (not registered) | `L2PcInstance.java:3180-3184`; `DestroyPlayerDroppedItem = False` (`GC/general.properties:306`) |

The pinned scanner runs at a fixed rate of 5,000 ms (`ItemsAutoDestroy.java:35`) and destroys when `now − dropTime > limit` (strict). **Adaptation X8:** the scanner grid is zone ticks `≡ 0 (mod 50)` from epoch tick 0. `decay_due` is the smallest `T` with `T ≡ 0 (mod 50)` and `T − dropped_tick > limit_ticks`. Decay emits `EntityDespawn` and a ledger `destroy` row under the ground owner with cause `decay` (§11.3). `ProtectedItems` is empty (`:269`).

### 3.9 PK (chaotic) drops (F3, R13)

Pinned `onDieDropItem(killer)` (`L2PcInstance.java:4473-4553`):
- return if the victim is an event participant (vacuous) or `killer == null` (`:4474`);
- the clan-war exemption returns when `karma ≤ 0` and the killer's clan is at war (`:4479-4483`). Clans arrive in Phase 7, so it is vacuous;
- the guard is `(!isInsideZone(PVP) || pk == null) && (!isGM() || …)` (`:4485`). Nightfall has no PvP-zone type and no GMs, so the guard is always true;
- the **karma branch** applies iff `karma > 0 ∧ pk_count ≥ 6` (`:4496`; `Z/phase3_entity.rs:76-77`). The **NPC-killer branch** (`:4503`) uses `PlayerRateDrop = 0` (`GC/rates.properties:106`), so it never drops (its gate `dropPercent > 0` is false: no draw).

```text
pk_drop_v7(victim, blow):
   killer := blow?.attacker ; if None → return
   if !(karma > 0 ∧ pk_count ≥ 6) → return                      // no draw
   if !chance_bound(40, 100) → return                          // `Rnd.get(100) < dropPercent` (:4511)
   n := 0
   for item in victim.inventory ordered by ItemInstanceId:     // X4 (Java iteration order)
       if excluded(item) → continue                            // :4524-4531, no draw, no unequip
       p := equipped ? (type2 == WEAPON ? 10 : 40) : 50
       if equipped: unequip now (Phase 4 unequip path; ledger `unequip`, cause pk_drop:<death>)   // :4534
       if chance_bound(p, 100):                                // :4540
           dx := roll_below(50) − 25 ; dy := roll_below(50) − 25 // [−25, +24] (:3178)
           move the WHOLE instance to the ground (identity transfer, §11.3)
           n += 1 ; if n ≥ 10 → break                          // :4549-4550 — later equipped items stay equipped
```

`excluded(item)`:
- shadow or time-limited: excluded from the catalogue, so vacuous;
- `!isDroppable`, which also excludes augmented items (`L2ItemInstance.java:667-671`);
- adena;
- `type2 == QUEST`;
- the active pet control item: vacuous;
- `NonDroppableItems` (31 ids; `GC/pvp.properties:24`) or `PetItems` (`:20`).

PK ground items:
- no loot owner;
- no DropProtection, unless Q1 resolves to 15 s;
- `decay_due = None`;
- position `victim.pos + (m(dx), m(dy))` with the P6 fallback.

## 4. Pickup, party loot, party protocol and party XP

### 4.1 PickUpItem intent

`PickUpItemRequest{idempotency_key, ground_entity}` (W tag 29) resolves in phase 0. Its checks, in order:

1. **Actor.** The actor is alive (`isAlikeDead`, `L2PcInstance.java:3779`), else `DEAD_ACTOR`.
2. **Target.** The target is a live ground entity, else `GROUND_ITEM_NOT_FOUND`.
3. **Range.** Pinned pawn movement uses `maybeMoveToPawn(target, 36)` (`GJ/ai/L2PlayerAI.java:260`). The offset adds the actor's collision radius (`GJ/ai/L2CharacterAI.java:957-960`); items have none. The test is the 2D non-strict `isInsideRadius(target, offset, false, false)` (`:978`).
   - Nightfall: `dx² + dy² ≤ (1,125 + actor_radius)²`. `actor_radius` is the engine's player value 0 (`Z/state.rs:2165`). Otherwise `OUT_OF_RANGE` (W 10).
   - Server-side approach movement is **not** performed. The client walks first (rule P8).
4. **DropProtection**, then **capacity** (solo / FINDERS_KEEPERS only), then **loot owner**, as in §3.7.
   - Failures: `GROUND_ITEM_PROTECTED`, or `INVENTORY_FULL` (W 29).
   - A herb whose skill is not executable fails with `ITEM_CAPABILITY_DEFERRED` (W 36) **before** removal.
5. **Commit.** Remove the ground entity: `EntityDespawn`, ledger rows (§11.3). Then dispatch:
   - **Herb** (`ex_immediate_effect`): §4.3. No party distribution (`:3860-3868`).
   - **In a party:** `distributeItem(picker, item)` (§4.2).
   - **Solo, adena, picker already holds adena:** merge into the existing adena stack; the ground instance ends (`addAdena` + `destroyItem("Pickup")`, `:3894-3896`).
   - **Solo, otherwise:** `addItem("Pickup", instance)`. This is an identity move, or a merge into an existing stack of the same stackable template (`ItemContainer.transferItem` semantics, `GJ/model/itemcontainer/ItemContainer.java:336-395`).

Pinned arrow/bolt auto-equip (`:3897-3912`) is out of Phase 5 scope. It is a Phase 4 equipment behaviour and is not triggered: rule P9, recorded as a known divergence.

Count caps D68/D70/D74 are enforced on the receiver: adena ≤ 99,900,000,000 (`GC/character.properties:671`); other stackables ≤ `Integer.MAX_VALUE`. A pickup or distribution that would exceed a cap is refused **before** any change, with `ITEM_COUNT_CAP`.

### 4.2 Party distribution: five modes, even split, sweep routing (F22, R16)

`getActualLooter(picker, template, spoil, origin)` (`L2Party.java:197-220`):

| Mode (id) | `spoil = false` (pickup) | `spoil = true` (sweep) |
|---|---|---|
| FINDERS_KEEPERS (0) | picker | picker |
| RANDOM (1) | `getCheckedRandomMember` | picker |
| RANDOM_INCLUDING_SPOIL (2) | random | random |
| BY_TURN (3) | `getCheckedNextLooter` | picker |
| BY_TURN_INCLUDING_SPOIL (4) | next | next |

If no member qualifies, the looter is the picker (`:219`).

- **Candidate filter** (`:159-192`). A member qualifies iff `validateCapacityByItemId(template)` and `checkIfInRange(PartyRange2 = 1,400, origin, member, true)`. The capacity predicate **always returns true**, because its inner result is discarded (`GJ/model/itemcontainer/PcInventory.java:759-765`; VR5 A61). The filter is therefore the range test `4·dist² ≤ (2·43,750 + rad)²`, with `rad` the origin's radius (NPC for sweep, 0 for the picker) plus the member's 0.
- **RANDOM.** Candidates are taken in member order, then `roll_below(k)` (draw #8).
- **BY_TURN.** Repeat `members.len()` times: `item_last_loot += 1; if item_last_loot ≥ len → 0`; test `members[item_last_loot]`; the first qualifying member wins (`:175-192`). The cursor advances once per **tried** member, including skipped ones. A full lap with no winner leaves the cursor advanced by `len` and returns the picker.
- **Even split** (`isItemToEvenlyDistribute`, `:496-505`): applies iff `count > 1` and the template is in `PartyEvenlyDistributeItems` (pinned: 57). The flags `…AllStackableItems` and `…AllOtherItems` are both False (`GC/character.properties:616,624,631`).
  - Pickup path (`:510-520`): the range origin is the **picker**. The ground instance ends with `destroyItem("Party")`.
  - Sweep path (`:545-556`): the range origin is the **NPC**.
  - `distributeItemEvenly` (`:591-628`): the recipients are the in-range members in **member order** (adaptation X9 replaces the pinned identity-hash `HashMap` order). `each = amount / k`; `leftOver = amount % k`; then `leftOver` draws `roll_below(k)`, each adding 1 to the chosen recipient (with replacement; draw #9). Payout is per recipient in member order:
    - adena via `addAdena`, which merges into the recipient's adena stack or creates one;
    - other templates via `addItem(template, v)`;
    - recipients receiving 0 are skipped.
  - Each payout is a **fresh** grant or a merge into an existing stack. The source instance ends with a `destroy` row (§11.3; DR2 R6).
  - If no member is in range (possible only on the sweep path), nobody receives anything and the source is still destroyed. This is the pinned behaviour: `toReward` is empty and the loop does nothing (`:600`).
- **Sweep routing (R16).** In a party, **every** swept holder goes through `distributeItem(sweeper, holder, spoil = true, npc)` (`Sweeper.java:60-64`). Adena with count > 1 is therefore split in every mode; other items use the table above. Solo: `addItem("Sweeper")` to the sweeper.
- **Messages.** `LootDistributed{picker, recipient, template, count, mode, spoil}` (WorldEvent 45) goes to all party members except the recipient (`L2Party.java:510-535,545-570`).

### 4.3 Herbs

On pickup of a herb (`hasExImmediateEffect`):
1. If the template has handler `ItemSkills`, its `item_skill` executes on the picker through the Phase 4 item-cast path (P4Z §8) as an immediate item-origin skill. This mirrors the handler call `useItem(this, target, false)` (`L2PcInstance.java:3860-3866`). Its effects are applied at this resolution point in P3 handler order (draw #7). Herb skills have no item cost; `ItemSkills` does not consume a herb that is not in the inventory.
2. The ground instance is destroyed (`destroyItem("Consume")`, `:3867`). Ledger: ground `destroy`, cause `herb:<receipt>`.

Herbs never occupy an inventory slot. The capacity check uses `PcInventory.validateCapacity(L2ItemInstance)`, which exempts herbs (`PcInventory.java:746-752`).

### 4.4 Party protocol (B9, B15, R15)

| Intent (W tag) | Pinned source | Contract |
|---|---|---|
| `PartyInvite{target, mode}` (49) | `RequestJoinParty.java:48-202` | Checks, in pinned order:<br>1. the target exists in the zone and is visible;<br>2. the target is not already in a party (`PARTY_ALREADY_IN_PARTY`);<br>3. block list: vacuous (no block lists before Phase 7);<br>4. the target is not self;<br>5. if the requester is in a party: the requester is the leader, `members.len() < 9` (`:149`, `PARTY_FULL`), and there is no unexpired pending invitation;<br>6. the target is not processing a request (`C1_IS_BUSY`).<br>Sets `TxRequest{kind: PartyInvite{mode}}` with expiry +150. `mode` matters only when creating a party; an existing party keeps its mode (`:161`). Emits `PartyInviteReceived`. |
| `PartyAnswer{accept: i32}` (50) | `RequestAnswerJoinParty.java` | Uses the target's active request; an expired or absent one is a no-op reject (`NO_PENDING_REQUEST`).<br>`1`: re-check size < 9. Create the party as `[requester]` with the requested mode if needed, then append the target (`L2Party.java:120-124,287`). This **cancels any pending loot vote** (`:254-256`).<br>`0`/`-1`: deny.<br>Clears the request in all cases. |
| `PartyLeave` (51) | `RequestWithDrawalParty.java:50` → `removePartyMember(Left)` | Disband rule (`L2Party.java:344`): `len == 2`, or the leader leaves with `LeavePartyLeader = False` → disband (every member removed, `PARTY_DISPERSED`). Otherwise remove and shift. |
| `PartyKick{member}` (52) | `RequestOustPartyMember.java:44-45` | Leader only. Same removal rule, with message type Expelled. |
| `PartyChangeLeader{member}` (53) | `RequestChangePartyLeader.java:45`; `L2Party.java:446-482` | Leader only. The target must be a member other than self. Swap positions 0 and p. |
| `PartyLootChangeRequest{mode}` (54) | `RequestPartyLootModification.java:45-54`; `L2Party.java:851-864` | Leader only. `mode ≠ current`. No vote may be pending (`PARTY_LOOT_VOTE_IN_PROGRESS`). Opens `LootVote{expires: +150}`. Emits `PartyLootVote{requested}` to all members. |
| `PartyLootChangeAnswer{accept}` (55) | `AnswerPartyLootModification.java:45`; `L2Party.java:866-884` | A single decline finishes the vote as failed. An accept adds the answerer, if not already counted. When `answers.len() ≥ members.len() − 1`, the vote succeeds: set the mode and emit `PartyLootVote{applied}`. The leader is not excluded from answering (pinned). |

Voting rules:
- **Timeout:** phase 2 finishes the vote as failed (`finishLootRequest(false)`).
- **Membership changes:** a join cancels the vote (`:254-256`). A leave from a party that keeps ≥ 2 members does **not** cancel it; the threshold is re-evaluated at the next answer (pinned).
- **Disband** drops the vote with no broadcast, as pinned (`:402-425`).

### 4.5 Party and solo XP/SP under V7 (B16, R5)

The reward snapshot `R` is taken in `death_v7` from the hate ledger (§3.6 filter: player, `damage > 1`, within PartyRange 1,600 of the NPC). It contains each rewarded player's damage and `totalDamage = Σ damage`; out-of-range damage is excluded (`L2Attackable.java:346-376`).

Base rewards: `X = floor(lvl² × expRate × RateXp × G)` and `S = floor(spRate × RateSp × G)` (`L2Npc.java:961-970`). `expRate` and `sp` come from the NPC template as exact rationals in Phase5Rules; with `G = 1` this equals the Phase 3 `xp_reward` value. The adaptations below are labelled X10.

`calcExpSp(diff, dmg, total)` (`L2Attackable.java:1139-1176`):
1. `diff := max(diff, −5)`.
2. `x := X·dmg/total` and `s := S·dmg/total`, exact.
3. `ExponentXp = ExponentSp = 0` (`GC/character.properties:758-759`), so: if `diff > 5`, multiply both by `(5/6)^(diff−5)`, exact.
4. Return `(floor(x), floor(s))`. Negative results are clamped to 0.

Then, iterating over **members of R in ascending EntityId order**:
- **Solo branch** (the player is not in a party) (`:408-452`):
  - `(e, s) = calcExpSp(player.level − npc.level, dmg, total)`;
  - champion and over-hit are vacuous: over-hit does not exist in the engine (rule P10);
  - award if the player is alive.
- **Party branch** (`:453-535`), once per party:
  1. `rewarded` = every alive member within PartyRange 1,600 of the NPC, in **member order**.
  2. `partyDmg = Σ dmg` of members present in R; remove those members from R.
  3. `partyLvl = max level over rewarded`.
  4. `partyMul = partyDmg/total` if `partyDmg < total`, else 1.
  5. `(e, s) = calcExpSp(partyLvl − npc.level, partyDmg, total)`; then `e := floor(e × partyMul)`, `s := floor(s × partyMul)`. The pinned `float`/`double` products become exact (the double scaling by damage share is pinned and kept).
  6. If `partyDmg > 0`, apply `distributeXpAndSp(e, s, rewarded, partyLvl)` (`L2Party.java:644-714`):
     - `n = |rewarded|` (cutoff `highfive` keeps all members, `:734-788`);
     - `e := floor(e × B(n) × RatePartyXp)`, `s := floor(s × B(n) × RatePartySp)` when `n ≥ 2`, where `B` is `BONUS_EXP_SP[min(n−1, 8)]` = 1, 1.10, 1.20, 1.30, 1.40, 1.50, 2.0, 2.10, 2.20 (`:81-84,790-808`; long/int `*=` truncation);
     - `Σsq = Σ level²`;
     - for each alive member in rewarded order: `ae = round_half_up(e × lvl²/Σsq)` (`Math.round`); `as = floor(s × lvl²/Σsq)`;
     - **highfive cutoff:** `gap = partyLvl − lvl`; for the first configured gap `[lo, hi] ∋ gap` (0–9, 10–14, 15–99), award `floor(ae × pct/100)` and `floor(as × pct/100)` with pct 100/30/0 (`:695-714`). A gap outside every range (negative) awards nothing.

Vitality, Nevit and recommendation bonuses do not exist in the engine and are vacuous. XP awarding then uses the existing `award_xp`/SP paths (level-up events unchanged). The oracle vectors (§14.1 family X) cover solo, party, gaps 9/10/14/15, `Math.round` halves, and `diff = 6, 7`.

### 4.6 Departure and the zone boundary of parties

Pinned logout runs `removePartyMember(Disconnected)` (`L2PcInstance.java:5629-5634,8553-8560`): a disconnecting leader does not disband the party; the next member leads. Nightfall applies the same rule at **any** departure: logout, socket loss past the replacement window, zone transfer, or epoch end. Parties are zone-scoped. A cross-zone party is out of scope until Phase 7 (B9).

## 5. Spoil and sweep (B11, A7)

### 5.1 Admission through the cast pipeline

| Skill | Data | Executable contract |
|---|---|---|
| **Spoil 254** | 11 levels, magicLvl 10–72, MP 12–67, reuse 3,000, hitTime 1,800 (`DP/stats/skills/00200-00299.xml:1342-1364`). `effectPoint` −138…−646 is hate, not power (VR5 A42) | Instant effect `Spoil` (`DPJ/handlers/effecthandlers/instant/Spoil.java`). Its success is `calcSuccess` → `Formulas.calcMagicSuccess` (`:38-41`; `Formulas.java:1286-1313`) through the existing `magic_success_rate` (`Z/skill_math.rs:710-739`) and the Phase 3 magic-success draw (#1). The success draw precedes `onStart`. `onStart`: the target must be a living **monster** (`:51`, else INCORRECT_TARGET). An already-spoiled target returns `ALREADY_SPOILED` after the draw (`:57-60`). Otherwise `spoiler := caster` (`:62`) and an aggro notify (`:64`) through the existing hate path, with 0 damage. The enchant routes (`basicProperty=MEN`) are deferred named blockers (D6). |
| **Sweeper 42** | level 1, castRange 20, hitTime 500, reuse 500, MP 3, target `NPC_BODY`, condition `canSweep`, effects `Sweeper` then `ConsumeBody` (`DP/stats/skills/00000-00099.xml:1465-1493`) | New target type **NpcBody**: a dead NPC entity with `corpse_until > tick`, within the cast range `m(20) = 625` plus the Phase 3 range rule. Start condition `canSweep` (`ConditionPlayerCanSweep.java:50-75`): `checkSpoilOwner` ∧ `¬isOldCorpse(2,000 ms)`, i.e. `corpse_until − tick ≥ 20` (`L2Attackable.java:1012-1020`; `GC/npc.properties:94`) ∧ slots+weight for the holders (`checkInventorySlotsAndWeight`). Impact: (1) `Sweeper`: re-check the spoil owner and slots/weight (`Sweeper.java:53-57`), then `takeSweep()` and route each holder (§4.2); (2) `ConsumeBody`: end the corpse (§1.3). A failed `Sweeper` re-check still runs `ConsumeBody`, because effects are independent (pinned). |
| **Sowing 2097 / Harvesting 2098** | §10.6 | — |

`checkSpoilOwner(sweeper)` is true iff `sweeper == spoiler`, or `isInLooterParty(spoiler)`: the spoiler is online and in the sweeper's party, or its CC (vacuous) (`L2Attackable.java:1027-1035`). Otherwise `SWEEP_NOT_ALLOWED`.

### 5.2 Sweep items are fresh items

Each swept holder becomes a **fresh** instance, allocated at the sweep impact (`addItem("Sweeper", holder)` / `"Sweeper Party"`). It merges into an existing stack when one exists (`Sweeper.java:60-66`). Ledger op `sweep` uses `before = None` (fresh) or `before = Some` (merge), with cause `sweep:<cast_id>`. There is no corpse owner and no transfer pair (DR2 R6). A party adena split from a sweep uses `party_split` grants. Its source holder never existed as an instance, so it produces **no** destroy row.

### 5.3 Corpse lifecycle summary

- **Death.** `corpse_until` is set per §1.3: 7 s plus 10 s if spoiled or seeded.
- **Sweep impact.** The corpse ends (`ConsumeBody`) whether or not items were taken; its `EntityDespawn` is emitted in the next spawn phase.
- **No sweep.** The holders vanish with the corpse at `corpse_until`, with no ledger rows, because no instance existed.
- **Event.** `CorpseStateChanged{npc, spoiled, seeded, sweepable}` is emitted at death (when spoiled or seeded) and at sweep.

## 6. NPC vendors (E2, E9, E18; F8/F9, R3/R4)

### 6.1 Merchant binding and interaction

Merchants are the 274 AI-script buylists (§1.10 item 6). The XML buylists (all `price="0"`) are not a price source. `VendorBuy/VendorSell/VendorRefund` carry `merchant: EntityId`. Checks, in order:
1. flood gate `Transaction` (10 ticks, `GC/floodprotector.properties:85`; `RequestBuyItem.java:82-85`; `RATE_LIMITED` W 5);
2. karma: `KarmaPlayerCanShop = True` (`GC/character.properties:435`), so vacuous;
3. the merchant is a merchant NPC in the zone, within `m(150) = 4,687` (2D non-strict, `RequestBuyItem.java:100`; X1), and `buyList.isNpcAllowed(merchant)` (`:121`). Otherwise `OUT_OF_RANGE` / `INVALID_MERCHANT`.

Merchant NPCs are fixture NPCs in the zone fixture pack, with their pinned NPC ids. As for the Phase 4 blacksmith (D58), they keep incarnation 0, and binding keys on entity id plus geometry.

### 6.2 Exact Java-`double` price emulation (R4)

`fl(x)` is the IEEE-754 binary64 round-to-nearest-ties-to-even of an exact non-negative rational `x`:

```text
fl(0) = 0
fl(x > 0):
   e := ⌊log2 x⌋                         // from bit lengths of num/den, corrected by one comparison
   y := x · 2^(52 − e)                   // exact rational in [2^52, 2^53)
   m := round_half_even(y)               // integer
   if m == 2^53 { m := 2^52; e := e + 1 }
   return m · 2^(e − 52)                 // exact rational
```

There are no subnormal or overflow cases: every input lies in `[0.01, 2^53)`. Java (Java 17+ is strictfp, JEP 306) evaluates `price * (1 + (baseTaxRate + castleTaxRate))` (`RequestBuyItem.java:182`). Its operands come from:
- `baseTaxRate = fl(tax_literal) / 100.0` (`AbstractNpcAI.java:212`). The `tax` literal is a Java double from the source text; all pinned literals are integers;
- `castleTaxRate = taxPercent / 100.0` (`GJ/model/entity/Castle.java:479,555`).

The unit price is therefore:

```text
t  := fl( fl(tax_literal) / 100 )
c  := fl( castlePercent / 100 )          // pinned seed: 0 → 0
s  := fl( t + c )
u  := fl( 1 + s )
v  := fl( P · u )                        // P = reference price (long ≤ 34,319,000 < 2^53, exactly representable)
unit := ⌊v⌋                              // (long) truncation toward zero; v ≥ 0
```

Implementation: `Z/binary64.rs` holds exact rationals as `(U256 num, U256 den)` and computes `fl` by integer shifts. Phase5Rules **precomputes** `unit(template, tax, castlePercent)` for every merchant entry and every hashed castle percent: 0 now, and the ceilings 5/15/25 for later phases. The table is **cross-checked** at `try_new` against `fl` (fail closed on mismatch).

The siege-guard step `price *= RateSiegeGuardsPrice` for templates 3960–4026 (`RequestBuyItem.java:152-154`) is a `long *= float`. Its maximum pinned price is 170,000 < 2^24, so it is exact at rate 1. `try_new` fails if any such price is ≥ 2^24 or the rate ≠ 1, because float emulation is not implemented (rule P11).

Proof obligation, oracle family **V-P** (§14.1):
- over all **5,803** real `(template, tax)` pairs × castle percent ∈ {0, 5, 15, 25}, the Rust table equals the Fraction emulation;
- the emulation equals the hardware binary64 reference computed in the oracle (CPython `float` is IEEE binary64);
- the author's preliminary run: 0 mismatches between the emulation and binary64 at castle 0;
- 362 pairs differ from naive exact-rational truncation (e.g. 100 × 1.15 → 114).

### 6.3 VendorBuy (`RequestBuyItem.runImpl`, `:75-247`)

Request `{merchant, list_id, items: [(template, count)]}`, 1 ≤ items ≤ 250 (`MAX_ITEM_IN_PACKET`), count ≥ 1 (`:56-73`). Duplicate templates in one request are refused `INVALID` (rule P12). Pinned would charge for a second limited decrease that then fails (`:209,223-228`).

For each item, in request order:
1. the product exists (else `INVALID`);
2. a non-stackable template requires `count == 1` (`:145-149`, `INVALID_QUANTITY`);
3. a pre-tax price of 0 is refused `INVALID` (`OnlyGMItemsFree = True`, `GC/general.properties:755`; `:162-166`). This applies to template 8187;
4. limited stock requires `count ≤ max(stock, 0)` (`:168-174`, `VENDOR_OUT_OF_STOCK`);
5. `MaxAdena / count < pre-tax price` → `ADENA_LIMIT` (`:176-180`);
6. `unit := table[...]`; `subTotal += count × unit` (`:181-183`), and `subTotal > MaxAdena` → `ADENA_LIMIT` (`:185-188`);
7. accumulate weight `count × weight` and slots (+1 per template not already held, `:190-194`).

Then:
1. weight check (`WEIGHT_LIMIT`);
2. slot check (`INVENTORY_FULL`);
3. adena ≥ subTotal (`INSUFFICIENT_ADENA`) (`:196-213`);
4. **commit**: consume subTotal adena (cause `receipt:vendor_buy:<key>`). For each item in request order: decrement limited stock, start the restock timer if none is pending (§6.4), and add the item: fresh or merged stack, op `purchase`.

The treasury share is `(long)(subTotal × 0) = 0`: a no-op. A non-zero castle tax would be a ledgered adena sink (A14) when castles arrive (`:233-235`).

### 6.4 Restock (E18)

`Product(…, restockDelay = (long)(60 / delay) minutes, maxCount = count)` (`AbstractNpcAI.java:213-219`; `Product.java:54-64`). The delay literals are 0.04, 0.042, 0.083, 0.125, 0.166, 0.2, 0.25, 0.5 and 1.0. Their minute values are taken with Java `(long)(60 / fl(delay))`, i.e. `⌊fl(60 / fl(delay))⌋`, computed with `fl` and hashed: 0.04 → 1,500; 0.042 → 1,428; 0.083 → 722; 0.125 → 480; 0.166 → 361; 0.2 → 300; 0.25 → 240; 0.5 → 120; 1.0 → 60. Every pinned limited entry has both `delay > 0` and `count > 0`.

Semantics (`Product.java:109-137`):
- `decreaseCount` schedules a restock after `restockDelay × 60,000` ms only if none is pending. In ticks this is `due = tick + minutes × 600`.
- Restock refills `count := max` at once.
- Stock is per product of one buylist (global within the zone).
- Restock runs in phase 2 (§2.1).
- Stock appears in `ListBuylist` (§13.4). Under V7, `VendorStock` is digested.

### 6.5 VendorSell (`RequestSellItem`, `:75-165`) and the refund tab

Request `{merchant, list_id, items: [(instance, count)]}`. Pinned aborts mid-loop on overflow **after** earlier items moved, without paying (`:142-147`). Nightfall pre-validates the whole request, then commits all or nothing (rule P13).

For each item:
1. `checkItemManipulation(instance, count, "sell")`: owned, available, non-stackable count 1, count ≤ owned (`L2PcInstance.java:3302-3347`);
2. `isSellable`, which is `!augmented && template.is_sellable` (`L2ItemInstance.java:693-694`). Pinned skips unsellable items silently (`:138-140`). Nightfall refuses the whole request with `ITEM_NOT_SELLABLE`, so the receipt is never partial (rule P13);
3. `price = ⌊ref/2⌋` (`:142`); `total += price × count`; overflow → `ADENA_LIMIT`.

Commit:
- each item moves into the **refund tab** (`AllowRefund = True`, `GC/general.properties:458`), using `transferItem` semantics: identity, merge or split (§11.3);
- a 13th distinct entry destroys the oldest (`PcRefund.java:57-69`; op `destroy`, cause `clear_refund:<receipt>`);
- then `addAdena(total)`: fresh or merged adena, op `sale_proceeds`. NPC sell **mints** adena (VR5 A2).

### 6.6 VendorRefund (`RequestRefundItem`, `:70-195`)

Request `{merchant, list_id, refund_indices: [u32]}`, with indices into the tab order and duplicates refused (`:58-75,145-152`). The price is `Σ ⌊count × ref / 2⌋` per entry, always for the **whole** entry (`:155,182`). For an odd price `p` and count `c`, sell paid `c·⌊p/2⌋`, so the refund costs `⌊c·p/2⌋ − c·⌊p/2⌋ = ⌊c/2⌋` more (R19).

Checks: weight, slots, adena. Commit: consume adena, then each entry moves back into the inventory (identity or merge).

## 7. Multisell (A9, B10; F19, R20, R42)

`MultisellChoose{idempotency_key, npc, list_id, entry_id, amount}` mirrors pinned `MultiSellChoose` (`GJ/network/clientpackets/MultiSellChoose.java`). That handler reads `_listId`, `_entryId` and `_amount` (`:83`) and gates on the `MultiSell` flood protector at 1 tick (`:104`; `GC/floodprotector.properties:78`).

**Validation:**
- `1 ≤ amount ≤ 5000`, else `INVALID_QUANTITY` (`:109`);
- the list exists, and `isNpcAllowed(npc)` holds or the list is not NPC-only (`:115-123`);
- the NPC is within `m(150)` (`:127-130`);
- the entry exists, and a non-stackable entry requires `amount == 1` (`:135`);
- slots and weight for products × `amount` (`:151-155`);
- ingredients: `count × amount` is owned and available (`:201-212`).

Capacity and weight are evaluated after inputs are paid (Phase 4 `InventoryMutation`, `Z/item_inventory.rs:109-111`).

**Tax** (`GJ/model/multisell/PreparedEntry.java:44-71`). Only an adena ingredient flagged `isTaxIngredient`, on a list with `applyTaxes`, adds `Math.round(count × taxRate)` (half-up). No pinned ingredient sets `isTaxIngredient`, and the castle tax is 0, so multisell tax is **vacuous in pinned data**. The rate source would be the NPC's castle (`getCastle().getTaxRate()`, R42). It is still implemented with exact half-up rounding and oracle-tested.

**maintainIngredient** (`GJ/model/multisell/Ingredient.java:40`; 21 pinned ingredients). The keep-ingredient branch applies only when `BlacksmithUseRecipes = False`: `required = (blacksmithUseRecipes || !maintainIngredient) ? count × amount : count` (`:212`), and consumption is guarded the same way (`:247`). The pinned value is `BlacksmithUseRecipes = True` (`GC/character.properties:520`), so **every ingredient is consumed, including the 21 flagged ones**. This corrects DR2 R20, which assumed they are kept. The flag is hashed, and the `False` branch is implemented for completeness.

**maintainEnchantment** is a **list-level** attribute (31 lists; `:352`):
- **Enchant level:** the consumed source instance is chosen by exact enchant match (`:260-262`), and the product copies its enchant level (`:362`; `PreparedEntry.java:81-82`).
- **Augmentation:** the i-th consumed augment is copied to the i-th product (`:354-356`).
- **Elemental attributes:** the copy is a **no-op** (B10; `:358-361`), because Phase 4 `ItemInstance` has no attribute field (`Z/item.rs:192-205`).
- **Non-maintained lists:** the lowest-enchant instances are consumed (`:276-300`).

**Ledger.** One Exchange operation per choose (D37/D52/D57 pattern). Each consume carries the cause `receipt:multisell_choose:<key>`. Each product is a fresh `exchange` grant, or a merge into an existing stack. One `ItemExchanged` outbox fact is emitted per choose. Phase 4 SA exchanges keep their own path.

## 8. Crafting (P3, A10; F7/F27)

### 8.1 Recipe book

`RegisterRecipe{idempotency_key, recipe_instance}` follows the pinned `Recipes` item handler (`DPJ/handlers/itemhandlers/Recipes.java:36-94`). Checks, in order:
1. not in craft mode: vacuous, since crafting is synchronous;
2. the recipe item maps to a recipe (`getRecipeByItemId`);
3. not already registered (`RECIPE_ALREADY_REGISTERED`);
4. **ability**: Dwarven recipes need Create Item 172 rank ≥ 1, Common recipes need Create Common Item 1320 rank ≥ 1 (`CANT_REGISTER_NO_ABILITY` → `CRAFT_SKILL_MISSING`);
5. `craftLevel > rank` → `CRAFT_LEVEL_TOO_LOW`;
6. the book holds fewer than `limit = 50 + stat` entries (`L2PcInstance.java:9097-9107`; stat = 0, R40), else `RECIPE_BOOK_FULL`.

Commit: register the recipe, then consume the recipe item (count 1, cause `receipt:register_recipe:<key>`, `:93`). The rank is the learned skill level from the Phase 3 skill book, not a learn level (R36).

`DeleteRecipe{recipe_id}` (`RequestRecipeBookDestroy.java:46-63`) removes the recipe with no refund and is flood-gated by `Transaction`.

### 8.2 Craft resolution (synchronous, `AlternativeCrafting = False`)

`CraftItem{idempotency_key, recipe_id}` (self craft, `RequestRecipeItemMakeSelf` → `RecipeController.requestMakeItem`, `:112-150`). `PrivateManufactureOrder` (§9.3) uses the same maker with a customer.

Checks, in order:
1. not in combat (`:114`; the Phase 3 attack-stance state) → `IN_COMBAT`;
2. a valid recipe in the crafter's book (`:119-131`);
3. `RecipeItemMaker` constructor checks (`:172-264`): both parties alive; neither is processing a transaction; the recipe has ingredients; `craftLevel ≤ rank` (`:216`); for a customer, adena ≥ price (`:223-233`); materials are held by the **customer** (`listItems(false)`, `:237`, `MISSING_MATERIALS`); the MP check (`calculateStatUse(false, false)`, `:248`). The HP stat use requires `hp > value` and the MP stat use requires `mp ≥ value` (`:473,487`); failing either returns `INSUFFICIENT_HP` / `INSUFFICIENT_MP`.

Then `finishCrafting()` (`:345-394`) in pinned order:
1. **MP/HP paid by the crafter.** This is `calculateStatUse(false, true)` (`:347`), reducing MP by the `statUse` value through the Phase 3 resource path.
2. **Manufacture fee.** It is transferred customer → crafter **before** the materials and the roll, and is never refunded (`:351-360`). Adena moves by transfer or merge.
3. **Materials consumed.** They come from the customer (`listItems(true)`, `:363`; cause `receipt:craft_item:<key>` or `receipt:private_manufacture:<key>`).
4. **Success draw.** `chance_bound(successRate, 100)` (draw #10, `:367`).
5. **On success, `rewardPlayer()`** (`:562-576`):
   - the rare gate is `rareItemId ≠ −1 ∧ (rareItemId == itemId ∨ CraftMasterwork)`, which is always true for pinned data because a non-rare recipe has `rareItemId = 0`;
   - draw #11 `chance_bound(rarity, 100)`. On success the product is `productionRare(id, count)`, otherwise `production(id, count)`;
   - the product is a **fresh** grant (or a merge) to the **customer**, op `craft`, with **no capacity or weight check** (`:576`; rule P7b, pinned).
6. **On failure.** The materials and fee are lost (100 %, P3). `CraftResult` is emitted with `success = false`.

`AltStatChange` XP/SP is inactive (`AlternativeCrafting = False`). The outcome is reported in `EconomyResult.craft{success, masterwork, product_template, count}`.

## 9. Player trade and private stores (A11, B2, B3, B7, E4, E10; F16, R18, R22)

### 9.1 Trade request and answer

**`TradeRequest{target}` (CM 37).** Pinned `TradeRequest.java:57-176`. Checks, in order:
1. The target is a player entity known to the requester (AOI), else silent reject `UNKNOWN_ENTITY`.
2. The target is not self.
3. Karma: `KarmaPlayerCanTrade = True` (`GC/character.properties:439`), so vacuous. Jail: vacuous.
4. Neither side has a private store open (`CANNOT_TRADE_IN_STORE`).
5. The requester is not processing a transaction: no request out, and no trade (`ALREADY_TRADING`).
6. The target is not processing a request or transaction (`TARGET_BUSY`).
7. Trade refusal / block list: vacuous, as no such settings exist before Phase 7.
8. `distance > 150` → `TARGET_TOO_FAR`, i.e. `dx²+dy² > 4,687²` (`:168`; X1). Reuses `OUT_OF_RANGE` (W 10).

On success: `TxRequest{Trade, expires: tick + 150}` (`L2PcInstance.java:5038-5041`), and `TradeRequestReceived` to the target. The requester is not flood-gated (`TradeRequest` has no flood check).

**`TradeAnswer{accept}` (CM 38).** Pinned `AnswerTradeRequest.java:48-80`.
- The request must exist; if the requester is gone, `TradeSessionChanged{cancelled}` is sent and the request is cleared.
- If `accept == 1` and the request is not expired, start the trade: two empty lists, `TradeSessionChanged{started}` to both.
- An expired accept counts as a deny (`:69-72`).
- The request is cleared in every case.
- There is **no distance check on accept** (pinned).

### 9.2 Trade lists, confirmation and exchange

**`TradeAddItem{instance, count}` (CM 39)** (`AddTradeItem.java:58-95`; `TradeList.addItem`, `GJ/model/TradeList.java:180-229`). Rejected in these cases:
- a list is locked, or either side has confirmed (`CANNOT_ADJUST_AFTER_CONFIRM`; `:75`);
- `validateItemManipulation` fails (`L2PcInstance.java:8303-8333`);
- the item is not tradeable: `!augmented && template.is_tradable` (`L2ItemInstance.java:685-686`), and not a quest item (`TradeList.java:192`) — `ITEM_NOT_TRADABLE`;
- `count ≤ 0`, `count > owned`, or non-stackable with `count > 1`;
- the same instance is already listed (`:217-222`).

On success the entry is appended, then **`invalidateConfirmation()` resets only the adder's own `confirmed`** (`:228`). R18: the earlier draft reset both. There is **no remove-item intent** (no pinned packet), and no maximum number of entries.

**`TradeConfirm` (CM 40) = TradeDone(1).** `TradeDone.java:51-95`:
1. flood gate `Transaction` (`:51`; `RATE_LIMITED`);
2. if the actor's list is locked, no-op;
3. the partner must exist, else cancel;
4. `distance > 150` → **cancel the whole trade** (`:91-94`) — `TradeSessionChanged{cancelled, reason: too_far}`;
5. `confirm()` (`TradeList.java:340-399`): set `confirmed`. If the partner has also confirmed, lock both lists, `validate()` both, then `doExchange`. Otherwise notify the partner (`TradeSessionChanged{partner_confirmed}`).

**Validation failure** at the second confirm leaves both lists locked in pinned (`:369-374`). Nightfall rule **P14**: the session is cancelled for both sides (items never moved), with reason `invalid`. Leaving both players permanently busy has no pinned resolution.

**`doExchange` (`:501-537`)** checks weight both ways (`:505`) and slots both ways (`countItemsSlots`, `:456-475,508`). On failure it sends `WEIGHT_LIMIT` / `INVENTORY_FULL` to both and finishes the trade with no transfer (`onTradeFinish(false)` for both). On success it transfers. Pinned `TransferItems` ignores per-item failures and could partially apply (`:517-518`). Nightfall pre-validates every entry, so a valid exchange applies completely: **one critical mutation, one receipt per side, all or nothing** (A11, B7).

- **Ledger.** Paired `transfer_out` / `transfer_in` rows per moved entry, linked by a transfer id. Moves use identity, merge or split, by `transferItem` semantics (§11.3).
- **Cap.** The receiver's adena cap is checked (D68); pinned has no explicit check (UNVERIFIED in `Inventory.addItem`). A violation fails the exchange, rule P15, with `ITEM_COUNT_CAP`.

**`TradeCancel` (CM 41) = TradeDone(0)** cancels for both (`:97`). `TradeSessionChanged{cancelled}` is sent to both, with no transfer (B7).

**Automatic cancellation:**
- `UseItem` by either party cancels (`UseItem.java:111-112`, pinned).
- Departure, disconnect or death cancels (B7). Pinned `deleteMe` cancels only if a requester is active (`L2PcInstance.java:8594-8598`), and `doDie` does not cancel; B7 is the binding rule, recorded as **X11**.
- Duel start: vacuous.

### 9.3 Private stores

**Open** (`SetPrivateStoreSell{title, package, entries[(instance, count, price)]}` CM 42, `SetPrivateStoreBuy{title, entries[(template, enchant, count, price)]}` CM 43, `SetPrivateManufacture{title, entries[(recipe_id, cost)]}` CM 44).

Preconditions (`L2PcInstance.java:3934-3935`, `canOpenPrivateStore`): alive, not casting, not in a no-store zone (none exist in Phase 5). The `SetPrivateStoreList*` checks then apply, in order (`SetPrivateStoreListSell.java:50-130`, `SetPrivateStoreListBuy.java:48-140`, `RequestRecipeShopListSet.java:52-117`):
1. entry count `1..=250`, price ≥ 0;
2. not in combat stance (`IN_COMBAT`);
3. entry count ≤ limit: sell 3 (Dwarf 4) + 0, buy 4 (Dwarf 5) + 0 (`GC/character.properties:306-313`; `L2PcInstance.java:9070-9095`), else `STORE_SLOT_LIMIT`. Manufacture has no limit beyond the packet maximum;
4. per entry: `MaxAdena / count < price` → `ADENA_LIMIT`;
5. sell: `adena + Σ count·price > MaxAdena` → `ADENA_LIMIT` (`:112-122`);
6. buy: `Σ count·price > MaxAdena` → `ADENA_LIMIT`, and `Σ > owned adena` → `INSUFFICIENT_ADENA` (`:124-135`). The adena is **not** reserved;
7. manufacture: each recipe must be in the owner's book (`:100`), and `cost ≤ MaxAdena`.

Pinned `addItem`/`addItemByItemId` silently drops invalid sell/buy entries (`:158-159`). Nightfall refuses the whole open with `ITEM_NOT_TRADABLE` (rule P16; no silent subset). The title must be ≤ 29 characters (`SetPrivateStoreMsgSell.java:31,47`; `RequestRecipeShopMessageSet.java:46`), else `INVALID`.

On success: store state, the owner sits (presentation only), and `PrivateStoreChanged` goes to AOI.

**`ClosePrivateStore` (CM 45)** follows `RequestPrivateStoreQuitSell/Buy:42-44` and `RequestRecipeShopManageQuit:42-44`. A store also closes automatically:
- when its last sell entry empties (`RequestPrivateStoreBuy.java:136-139`) or its buy list empties (`RequestPrivateStoreSell.java:126`);
- on departure (B2).

**`PrivateStoreBuy{owner, entries[(instance, count, price)]}` (CM 46)** — buy from a SELL or PACKAGE_SELL store (`RequestPrivateStoreBuy.java:82-139`; `TradeList.privateStoreBuy`, `:545-724`). Checks:
1. flood gate;
2. distance ≤ `m(150)` (`:96`);
3. the store type matches;
4. **package rule:** a PACKAGE_SELL store requires the request to cover every listed entry at its full count (`:119-125`, `PACKAGE_INCOMPLETE`);
5. each entry matches the listed instance **and exact price** (`:570-571`), with the count clamped to the listed count (`:572-573`);
6. overflow checks;
7. the seller still owns each item and it is tradeable (`:607-608`);
8. buyer weight, slots and adena (`:626-636`).

Commit, all or nothing (rule P15; pinned can break midway without refund at `:664-675`):
1. buyer adena → seller, as a transfer pair (merge or split);
2. each item seller → buyer, as a transfer pair;
3. the store entries are decremented.

**`PrivateStoreSell{owner, entries[(instance, template, count, price)]}` (CM 47)** — sell into a BUY store (`RequestPrivateStoreSell.java:83-126`; `TradeList.privateStoreSell`, `:732-886`). Matching is by template and exact price, and the count is clamped. Pinned allows **partial success**: entries beyond the store owner's adena are skipped (`:788`), and items move before adena (`:818,867-877`). Nightfall keeps the pinned per-entry skip semantics, but evaluates them up front in request order. It skips any entry whose cumulative price exceeds the owner's current adena, then commits the accepted subset atomically. The result lists the accepted and skipped entries. Moves are transfer pairs.

**`PrivateManufactureOrder{owner, recipe_id}` (CM 48)** (`RequestRecipeShopMakeItem.java:49-77`):
- `Manufacture` flood gate (`GC/floodprotector.properties:99`, 30 ticks);
- the customer has no store open;
- the owner's store kind is MANUFACTURE;
- `checkIfInRange(150, customer, owner)`; out of range is a silent no-op in pinned, while Nightfall returns `OUT_OF_RANGE`.

The craft then runs §8.2 with crafter = owner, customer = actor and price = the listed cost. The crafter's MP pays; the customer's materials and fee are consumed.

### 9.4 Disconnect and departure

Departure hooks run in fixed order at the departure command's ordinal:
1. close the store;
2. leave the party (§4.6);
3. cancel any trade and request (B7);
4. clear the refund tab, with destroy rows (B6).

No penalty and no partial transfer occur. Any remaining partner receives `TradeSessionChanged{cancelled, reason: partner_left}` (B7).

## 10. DB-only containers: warehouse, freight, mail, deliveries, item auction, manor

### 10.0 Lease and frozen-input contract (A13, E12, E14; F6, R10, R32)

Every DB-only economy row follows one mechanism, so the zone never reads the database and replay never depends on it.

1. **Preflight (API).** In one DB transaction:
   - lock the rows the intent touches (`SELECT … FOR UPDATE` in a fixed key order);
   - refuse with `CONTAINER_BUSY` (W 55, retryable) if any of them carries an `economy_leases` row;
   - insert a lease `(lease_key, zone_id, epoch, account_id, operation, request_key, acquired_at)`;
   - read the **pre-image**: each row's columns plus its `revision`.

   The API then builds the command with `frozen = {pre-images, DB-derived decisions}`. The decisions are evaluated on DB time and on rows the zone does not own, and are listed per operation below. The frozen bytes are part of the request fingerprint. A mismatching retry of the same key is rejected as in D43.
2. **Zone.** The zone validates the intent against the frozen pre-image, its own canonical state (inventory, position, peace zone) and Phase5Rules. It never reads the DB.
   - **Accepted:** the tick draft carries `ContainerDelta`s (row inserts, updates with `expected_revision`, deletes) and a `LeaseRelease`.
   - **Rejected:** the draft carries only the `LeaseRelease`. No receipt is written, and no key is consumed (Phase 4 rejection rule).
3. **Commit.** The revision-fenced checkpoint transaction (§11.5) applies each `ContainerDelta` as a compare-and-set on `revision` and deletes the lease. A CAS miss under a held lease is impossible by construction. If one happens, it is a **permanent constraint error** `economy_lease_violation`: the lane is fenced and output withheld (the D77 pattern), never retried as transient.
4. **Lease recovery.**
   - If the log record was acknowledged, recovery replays it and the commit releases the lease.
   - If the command never reached an acknowledged record, the zone's epoch-fenced recovery releases every lease `(zone, epoch)` whose `request_key` has no committed receipt. This runs before the zone accepts new commands.
   - Epoch closure releases every lease of the closed epoch.

| Crash window | State after recovery |
|---|---|
| Before preflight commit | Nothing changed |
| After the preflight commit, before the zone accepts the command | The lease is released by recovery, and the client retry re-preflights |
| After draft, before log ack | Lease released; no receipt; retry re-executes; any RNG is re-drawn from the recovered stream, exactly as Phase 4 |
| After log ack, before DB commit | The record is replayed with identical frozen inputs, producing the identical draft and IDs; commit applies |
| After commit, before output | The frozen receipt is re-emitted (D42) |

The pre-image and decision fields are encoded with deterministic canonical JSON (integers only, like D26) inside the `PbCommandKind` bytes. **No `NOW()` or `BIGSERIAL` value enters zone state** (E12). Message and delivery ids are UUIDv5 values derived from request identity. DB timestamps exist only in DB rows and API decisions.

### 10.1 Private warehouse (P4, A13, E17)

`WarehouseDeposit{keeper, entries[(instance, count)]}` (CM 56). It follows `SendWareHouseDepositList.java:58-184`. Checks, in order:
1. flood gate `Transaction` (`:86`);
2. the keeper is a warehouse-type NPC within `m(150)` (`canInteract`, `GJ/model/actor/L2Npc.java:515-532`; `:97-100`);
3. no enchant in progress: vacuous;
4. karma: `KarmaPlayerCanUseWareHouse = True`, so vacuous (`:113`; R21);
5. **fee** = `entries.len() × 30`, adena entry included (`:118`);
6. per entry: `checkItemManipulation`;
7. slots: a non-stackable entry adds its count; a stackable entry adds 1 unless the warehouse already holds the template (`:122-138`);
8. capacity `|warehouse| + slots ≤ 100` (Dwarf 120; `GJ/model/itemcontainer/PcWarehouse.java:48-49`) → `WAREHOUSE_FULL`;
9. `adena − deposited adena ≥ fee`, else `INSUFFICIENT_ADENA` (`:147`);
10. no active trade.

Pinned charges the fee before steps 6–10 and then silently skips non-depositable entries without a refund (`:147-167`). Nightfall rule **P17**: every entry is validated with `isDepositable(true)` (not equipped, and the template is depositable — **augmented items allowed**, `L2ItemInstance.java:701-711`; E17) and `isAvailable(player, true, true)` (`:751-762`) **before** anything is charged. Any failure refuses the request with `ITEM_NOT_DEPOSITABLE`. There is no weight check (`ItemContainer.java:603-605`).

Commit:
- the fee is a consume, with cause `receipt:warehouse_deposit:<key>`;
- each entry moves inventory → warehouse container by `transferItem` semantics: identity, merge into an existing warehouse stack, or split (`ItemContainer.java:336-395`; R21);
- the ledger records transfer pairs with owner `warehouse:<character>`;
- the item rows change location to `warehouse` in the same transaction.

Frozen: the warehouse pre-image (all rows).

`WarehouseWithdraw{keeper, entries[(instance, count)]}` (CM 57) follows `SendWareHouseWithDrawList.java:85-173`:
1. flood gate, keeper and karma checks as above;
2. each entry exists in the warehouse with sufficient count;
3. the inventory must accept slots (`SLOTS_FULL` → `INVENTORY_FULL`) and weight (`WEIGHT_LIMIT`) (`:124-147`).

There is no fee. Moves are transfer pairs.

### 10.2 Freight (E8, A13)

`FreightSend{keeper, target_character, entries[(instance, count)]}` (CM 58). It follows `RequestPackageSend.java:79-176`:
1. `target_character` is another character of the **same account** (`getAccountChars`, `:79`). This is a frozen decision; failure is `FREIGHT_TARGET_INVALID`;
2. flood gate (`:83`);
3. the keeper is any NPC within `m(150)`, 2D, with no warehouse-type check (`:88-91`);
4. no active trade (`:99`);
5. karma: vacuous (`:104`);
6. **fee** = `entries.len() × 1,000` (`:109`);
7. per entry: `checkItemManipulation`; `isFreightable` (template flag, default false, `GJ/model/items/L2Item.java:182`; `L2ItemInstance.java:1665-1667`). Any non-freightable entry aborts the whole request (`:114-125`), with `ITEM_NOT_FREIGHTABLE`. Augmented freightable items are allowed (E17);
8. slots: the adena entry adds **no** slot; a non-stackable entry adds its count; a stackable entry adds 1 unless the target freight holds the template (`:126-134`);
9. capacity ≤ **200**: the target's `FREIGHT_LIM` is ignored because `_owner = null` (`:113,137`; `PcFreight.java:34-38,66-68`) → `FREIGHT_FULL`;
10. adena ≥ fee (`:144`).

Commit:
- the fee is a consume;
- entries move by transfer pairs into owner `freight:<target>`.

Frozen: the target's same-account membership and its freight pre-image. The target being admitted elsewhere does not matter: the lease serializes access.

`FreightWithdraw{keeper, entries}` (CM 59) is pinned as `SendWareHouseWithDrawList` with the active warehouse set to the actor's `PcFreight` (`DPJ/handlers/bypasshandlers/Freight.java:45-59`). It needs a warehouse-type keeper (`canInteract`) and applies the same checks as §10.1 withdraw. Moves are transfer pairs out of `freight:<actor>`.

### 10.3 Mail (A12; E17; F12, R9)

Message rows are DB-only (§11.4). Pinned limits (`GJ/network/clientpackets/RequestSendPost.java:60-68`):
- receiver name ≤ 16;
- subject ≤ 128;
- body ≤ 512;
- ≤ 8 attachments;
- inbox and outbox 240 each;
- fee 100 + 1,000 per attachment entry.

**Time base (D65).** Message times are DB `timestamptz` only: `created_at`, `expires_at = created_at + 360 h`, or + 12 h for COD (`GJ/model/entity/Message.java:39-40,98`). The zone never sees them. Expiry is decided on DB time by the API and enters zone state only as frozen decisions or delivery commands. **No tick is ever stored for a mailbox.**

`MailSend{receiver_name, subject, body, cod_adena, attachments[(instance, count)]}` (CM 60). Checks follow pinned order (`RequestSendPost.java:108-319`):
1. **peace zone iff there are attachments** (`:130`; Phase 3 `PeaceSquare`, `Z/state_pvp.rs:81`) → `PEACE_ZONE_REQUIRED`;
2. no active trade, no store (`:135-149`);
3. length limits (`INVALID`; `:150-164`);
4. attachments ≤ 8 (`:166`, `MAIL_ATTACHMENTS_EXCEEDED`);
5. `0 ≤ cod ≤ MaxAdena` (`:171`);
6. COD requires `cod > 0` **and** attachments (`:175-184`). A non-COD mail stores `cod = 0`; rule P18 closes the pinned `_reqAdena > 0` without-COD inconsistency;
7. the receiver exists (frozen: name → character id; `MAIL_RECEIVER_NOT_FOUND`);
8. not self (`:192`);
9. GM receiver and jail: vacuous;
10. the receiver's block list: vacuous;
11. sender outbox < 240 and receiver inbox < 240 (frozen counts; `MAILBOX_FULL`; `:219-224`);
12. flood gate `SendMail` 100 ticks (`:229`; `GC/floodprotector.properties:106`);
13. per attachment: `checkItemManipulation`, `isTradeable` (no augmented items), not equipped (`:249-250`) → `ITEM_NOT_TRADABLE`;
14. adena after attachments ≥ fee (`:264`, `INSUFFICIENT_ADENA`).

Commit:
- the fee is a consume;
- attachments move inventory → owner `mail:<message_id>` by transfer pairs (`:294-299`);
- a message row is inserted with `message_id = UUIDv5(NS_MAIL, account ‖ "mail_send" ‖ request_key)`, `sender`, `receiver`, `cod`, flags, `has_attachments`, `created_at = now()` (DB);
- after commit, the API sends `MailNotification` to the receiver if the receiver is admitted anywhere (`GJ/instancemanager/MailManager.java:139-154`).

`MailTakeAttachments{message_id}` (CM 61) follows `RequestPostAttachment.java:60-232`:
1. flood gate `Transaction`;
2. **peace zone** (`:78`);
3. no trade or store;
4. the actor is the receiver; the message has attachments and is not expired (frozen);
5. slots/weight for the attachments (`:141-156`);
6. COD: actor adena ≥ cod (`:161-165`).

Commit:
- COD: the receiver's adena is **consumed** (`reduceAdena("PayMail")`, `:161-165`; cause `receipt:mail_take:<key>`), and a **pending delivery** row `cod_payment{amount}` to the sender is inserted (§10.4). Pinned credits the sender with **new** adena, not with the same instance: `addAdena` online, or a freshly created adena item offline (`:208-222`);
- attachments move `mail:<m>` → inventory (transfer pairs);
- `has_attachments = false`; the message is kept (`:201,230`).

`MailReject{message_id}` (CM 62) follows `RequestRejectPostAttachment.java:49-90`:
- peace zone; actor is the receiver; attachments present; not a system mail;
- a **return message** is created: sender = receiver = the original sender, 15 days, no COD, `returned = true`, with the attachments re-homed (`Message.java:128-146`);
- attachments move `mail:<m>` → `mail:<m'>` by transfer pairs between container owners;
- the original keeps `has_attachments = false`.

`MailCancel{message_id}` (CM 63) follows `RequestCancelPostAttachment.java:59-195`:
- the actor is the sender; peace zone; no trade or store; attachments still present (`YOU_CANT_CANCEL_RECEIVED_MAIL` → `MAIL_ALREADY_TAKEN`);
- slots/weight;
- attachments move back to the sender's inventory;
- the message row is **hard-deleted** (`:192`).

`MailDelete{message_ids[]}` (CM 64) follows `RequestDeleteReceivedPost.java:58-81` / `RequestDeleteSentPost`:
- peace zone; ownership per side;
- a message with attachments aborts the remainder of the batch, as pinned (`:77`): the result lists the processed prefix;
- the side's soft-delete flag is set; the row is hard-deleted when both flags are set (`Message.java:239-262`).

These operations touch no inventory. They are still WS intents under the lease contract (E14), so the mailbox has a single writer path.

**Expiry processor (API, DB time).** It mirrors `MessageDeletionTask.java:51-74`. When `expires_at ≤ now()`, the processor takes the message lease and:
- if the message has attachments, inserts a **pending delivery** `mail_expiry_return` to the sender, targeting the sender's **private warehouse** (`Mail.returnToWh`, `GJ/model/itemcontainer/Mail.java:78-89`). The attachments stay owned by `mail:<m>` until the delivery executes;
- hard-deletes the message row (the attachments are referenced by the delivery, not the message).

There is no item move without a zone, so no ledger row is written by the API. Read RPCs hide expired messages immediately.

### 10.4 Pending deliveries (rule P19; QUESTION Q4)

Pinned credits that target a character outside the acting zone are delivered by a zone, never by DB-only inventory mutation (D7/D30; VR5 B11):
- COD payment: online → `addAdena`; offline → inventory row (`RequestPostAttachment.java:208-222`);
- mail-expiry returns: sender warehouse;
- auction awards: winner warehouse (`ItemAuctionInstance.java:400-408`).

The mechanism:
- **Table.** `economy_deliveries(id uuid, target_character uuid, kind, payload, status)` (§11.4).
- **Execution.** A system command `DeliverPending{frozen delivery + source pre-images}` executes in the zone where the target is admitted:
  - at admission, before visible output (the D7 materialization slot);
  - or, while admitted, injected by the API through the D30 runtime port immediately after the creating commit.
- **Destinations:**
  - `cod_payment`: adena **created** in the sender's **inventory** (op `cod_credit`, fresh or merged into the existing adena stack);
  - `mail_expiry_return`: attachments from `mail:<m>` into `warehouse:<sender>`;
  - `auction_award`: a **fresh** item into `warehouse:<winner>`, op `auction_award`.
- **Cap or capacity failure:** an adena cap (D68) or warehouse capacity failure leaves the delivery pending and reports it in progress facts. Warehouse capacity is **not** checked by pinned `returnToWh`/`addItem`; Nightfall follows pinned and does not check it (rule P20).

Observable deviation **X12**: an online sender in another zone is credited a few ticks after the payer's commit instead of in the same server call. Q4 asks the architect to confirm the inventory destination for an offline sender (pinned) against the alternatives.

### 10.5 Item auction (B4, E16; R8)

Data (§1.10 item 10):
- **Instances.** Three instances keyed by auctioneer NPC id. Their schedule is Friday, Monday and Wednesday at 17:00 server time (`AuctionDateGenerator.java:47-49`; `DP/ItemAuctions.xml:13,187,361`).
- **Fixture schedule.** Under B4 the **fixture schedule replaces the wall-clock date generator**: a hashed list of `(instance, auction_id, start_db_time, item_index_or_draw)` entries.
- **Items.** Each instance has 82 items. All have `initBid = 100,000` and `length = 300 min`.
- **Config.** `ItemAuctionExpiredAfter = 14` days and `ItemAuctionTimeExtendsOnBid = 0` (`GC/general.properties:659,665`).

All auction state is DB-only (`item_auctions`, `item_auction_bids`; §11.4).

| Transition (API scheduler on DB time → system command into the zone hosting the auctioneer NPC) | Pinned | Effect |
|---|---|---|
| `AuctionCreate{instance}` | `ItemAuctionInstance.java:430-437` | Draw #14 `roll_below(82)` picks the item, unless the fixture pins an item. Insert a CREATED row with start/end from the fixture. |
| `AuctionStart` | `:337-346` | CREATED → STARTED (state CAS) |
| `AuctionFinish` | `:380-419` | STARTED → FINISHED. If a highest bid exists, insert a delivery `auction_award{template, count, enchant default}` to the winner (fresh item at delivery; `AuctionItem.java:74-85`). With no bids, nothing is created. |

`ItemAuctionBid{instance, bid}` (CM 67) follows `RequestBidItemAuction.java:38-66` and `ItemAuction.registerBid` (`GJ/model/itemauction/ItemAuction.java:215-278`):
1. flood gate `Transaction` (`:51-54`);
2. `0 ≤ bid ≤ MaxAdena`;
3. `bid ≥ initBid` (`BID_TOO_LOW`);
4. `bid ≤ 100,000,000,000`;
5. the state is STARTED (frozen; `AUCTION_NOT_ACTIVE`);
6. `bid ≥ highest` (strict `<` rejects; **equal is accepted**, `:237`);
7. own previous bid:
   - none, or cancelled: pay `bid` in full;
   - active: `bid ≥ last` (`:253`), pay `bid − last` (`:259`);
8. adena ≥ payment (`INSUFFICIENT_ADENA`).

There is **no NPC distance check** (pinned packet).

Commit:
- the payment moves inventory → owner `auction_bid:<auction>:<character>` (transfer pair, split or merge);
- the bid row is upserted;
- if `bid > highest` (strict, `:283`), the highest is replaced, and the outbid player is notified if admitted. There is **no refund** (E16).

**Extension** (`ItemAuction.java:292-335`) uses a frozen decision `remaining ≤ 10 min` (DB time):
- INITIAL → +5 min (any bidder);
- EXTEND_5 → +3 min, only if the bidder ≠ the last tracked bidder (starts at 0);
- the configured extension (0) adds nothing further.

The new end is written to the row. Pinned keeps the extension in memory only; Nightfall persists it so that restart matches the live end time, rule P21.

`ItemAuctionCancelBid{instance}` (CM 68) follows the NPC bypass `ItemAuctionLink cancel` (`DPJ/handlers/bypasshandlers/ItemAuctionLink.java:95-105`) and `cancelBid` (`ItemAuction.java:354-406`). It requires the auctioneer NPC within `m(150)`. For each non-CREATED auction of the instance with the actor's bid:
- refused if it is FINISHED with start older than 14 days;
- refused if there is no highest bid, or no bid from the actor;
- if the actor is the highest bidder: refused when FINISHED; otherwise "reserve not met" with **no refund**;
- if the bid is already cancelled: skip;
- otherwise refund the full `last` bid by transfer pair `auction_bid:…` → inventory, then mark the bid cancelled (−1), or delete it if FINISHED.

**Unclaimed bids.** Once an auction is older than 14 days, pinned deletes the auction and its bids at the next start, and unreclaimed adena is lost (`ItemAuctionInstance.java:476-488`). Nightfall's API cleanup deletes the rows on DB time. A zone-executed `AuctionPurge` system command writes `destroy` rows for the escrowed adena, with cause `auction_expired:<auction>`.

### 10.6 Manor (B1, E13; R8)

**Seeding** uses `UseItem` (CM 21) on a seed item. The Seed handler is `DPJ/handlers/itemhandlers/Seed.java:42-90`. Its checks:
- manor allowed;
- the target is a living monster: not a raid, not a chest;
- not already seeded;
- the seed exists in seeds.xml;
- the **seed's castle equals the castle area of the player's position**: `MapRegionManager.getAreaCastle`. Its Nightfall source is a hashed `castle_area` per fixture zone; QUESTION Q5 asks which castle area the test zone maps to.

On success the handler sets the NPC's `seed` and `seeder` (`L2Attackable.java:1289-1294`) and casts skill 2097 (Sowing: castRange 150, hitTime 1,800, reuse 10,000, MP 4/6/8 by level, target ENEMY_ONLY; `DP/stats/skills/02000-02099.xml:1537-1557`).

**Sow impact** (`Sow.java:51-84`):
- guard: the target is not dead, `canBeSown`, not seeded, and the seeder matches the caster (`:58`);
- the **seed item is consumed (1) before the roll** (`:63-66`);
- draw #12 `chance_bound(basic, 99)`, where `basic = alternative ? 20 : 90`, −5 per level below `seed.level − 5`, −5 per level above `seed.level + 5`, −5 per player/target level difference beyond 5. There is **no** 1 % floor: pinned `Math.max` discards its result (`:114`);
- success → `seeded = true` and `harvest = (crop, count)`, where `count = Π(strong-type multipliers 4303→×2 … 4310→×9) + max(0, npc.level − seed.level − 5)`, then × `RateDropManor` (`L2Attackable.java:1257-1283`).

The corpse extension applies to seeded NPCs (§1.3).

**Harvest** uses `UseItem` on Harvester 5125. Its handler requires a dead monster (`DPJ/handlers/itemhandlers/Harvester.java:45-70`) and casts skill 2098 (castRange 20, hitTime 500, reuse 1,000, MP 3, NPC_BODY; `:1558-1575`). The impact (`Harvesting.java:48-95`):
- only the **seeder** may harvest; there is no party exception (`:54`);
- the target must be seeded;
- draw #13 `chance_bound(basic, 99)`, where `basic = 100 − 5·max(0, |Δlvl| − 5)`, floored at 1 (`:98-119`);
- success → `takeHarvest()` and a **fresh** crop grant, op `harvest`;
- failure → the harvest is kept for retry.

Normal drops are unchanged (E13).

**Fixture manor manager** (B1). Pinned ownerless castles have empty lists (§1.10 item 10). Phase 5 hashes a fixture **APPROVED** period: per castle, seed production `(seed, price, amount)` and crop procurement `(crop, price, amount, reward_type)`, validated against the pinned limits:
- seed limit = `limit_seed × RateDropManor`;
- prices within `[⌊0.6·ref⌋, 10·ref]` (`L2Seed.java:86-116`; `RequestSetSeed.java:86`).

The period never changes in Phase 5: there is no maintenance or modifiable cycle. The amounts are DB rows under the lease contract.

- **`ManorBuySeed{manager, manor_id, entries[(seed, count)]}` (CM 65)** follows `RequestBuySeed.java:52-168`:
  - `Manor` flood gate;
  - the manager's `manor_id` equals the request's (`:96-100`);
  - per entry: the production row exists with `price > 0` and `amount ≥ count`;
  - total ≤ MaxAdena; weight, slots and adena.

  Commit: adena is consumed (sink `castle_treasury:<castle>`, a ledgered sink, A14); the amount is decremented; seeds are fresh `purchase` grants.
- **`ManorSellCrop{manager, entries[(instance, crop, manor_id, count)]}` (CM 66)** follows `RequestProcureCropList.java:47-170`:
  - per entry: the procurement row of `manor_id` exists with `amount ≥ count`;
  - reward template = `seed.reward(reward_type)`, chosen by the first seed matching the crop (`getSeedByCrop` with no castle, `:133`; pinned);
  - `rewardCount = ⌊count × price / rewardRefPrice⌋`; < 1 refuses the entry;
  - fee = `0` if `manor_id` equals the manager's castle, else `⌊fl(count·price × 0.05)⌋` (the `(long)(price × 0.05)` double site, emulated by `fl`, §6.2) adena;
  - weight, slots and adena for the fee.

  Commit: the procurement amount is decremented; the crop is consumed (cause `receipt:manor_sell_crop:<key>`); the fee is consumed; the reward is a fresh `manor_reward` grant. **No adena is paid to the player** (pinned).

## 11. Ledger, ownership and persistence

### 11.1 Owners (B13; F5, R6, R7)

Every ledger row names an **owner**. All are `uuid`; there is no BIGSERIAL anywhere.

| `owner_kind` | `owner_id` | Lifetime |
|---|---|---|
| `character` | `characters.id` | durable |
| `ground` | ground `EntityId` (P1) | snapshot only (A15, E17: no `ground` row in `item_instances`) |
| `refund` | the character id | zone only (B6) |
| `warehouse` / `freight` | the character id | durable container rows |
| `mail` | `mail_messages.id` | durable |
| `auction_bid` | `UUIDv5(NS_AUCTION, auction_id ‖ character)` | durable |

### 11.2 Transfer semantics (B13; amends D67/D69 for V7)

An item moving between owners follows pinned `ItemContainer.transferItem` (`GJ/model/itemcontainer/ItemContainer.java:336-395`):

| Mode | When | Source row | Target row |
|---|---|---|---|
| **identity** | whole count, and the target has no stack of that stackable template | `transfer_out`, before = instance, after = None | `transfer_in`, **same id**, before = None |
| **merge** | stackable, and the target holds the template | `transfer_out` (whole → after None; partial → after = count − n) | `transfer_in` on the target's **existing** stack, before = Some, `count_delta = +n` |
| **split** | partial count, and the target has no stack | `transfer_out`, after = count − n | `transfer_in` on a **freshly allocated** id, before = None, D67 fresh |

- Both rows carry `transfer = {link: UUIDv5(NS_XFER, resolution_id ‖ ordinal), mode, peer_owner_kind, peer_owner_id, peer_instance}` and the same receipt or cause.
- In identity mode the canonical fields are identical across the pair.
- A `transfer_in` with before = None is exempt from D67's "never seen earlier" rule **only** when its link points to a matching identity `transfer_out` (B13).
- An instance's location field changes with the owner kind. `ItemLocation` gains the V7 values `Ground`, `Refund`, `Warehouse`, `Freight`, `Mail` and `AuctionBid` (V7 only; V5/V6 decoding rejects them).

**Rule T1 (terminal rows; amends D69/D76 for V7).** Per `(owner, tick, instance)`, rows are ordered as follows:
1. all non-consume, non-terminal rows (positive grants, merges, `transfer_in`, equip/unequip, split-mode `transfer_out` that leaves count > 0), in cause order;
2. the single Consume aggregate, if any;
3. **at most one terminal row**: `transfer_out` with after = None, or `destroy`.

Ordinals are the row positions 0..n−1, so the D76 contiguity check holds. The Consume aggregate is the highest ordinal, except that it is one below a terminal row. This case arises when a stack is partly consumed and then moved or destroyed whole in the same tick; for example, a shot stack consumed during the killing tick and then PK-dropped.

The `000014` trigger encodes T1. Under V5/V6 no terminal row ever follows a consume, so legacy recordings validate unchanged. **QUESTION Q3** asks the architect to ratify T1 as a D76 amendment.

### 11.3 Per-operation ledger table (R6, R34)

Notation:
- `fresh` = before None with a newly allocated id;
- `merge` = before Some on an existing stack;
- `C(…)` = a Consume-aggregate contribution with that D62/D63 cause string.

Client-intent receipts use `receipt:<operation>:<key>`. Automatic causes use `<kind>:<resolution id>`. Count caps (D68/D70/D74) are checked at acceptance on every receiving row. Ordinals follow T1.

| Operation | Rows (owner: op, identity) | Causes / receipt | Cap / notes |
|---|---|---|---|
| NPC loot creation | ground: `loot`, fresh | `loot:<death resolution_id>` | — |
| PK drop | victim: `unequip` (if equipped) then `transfer_out` identity, terminal; ground: `transfer_in` identity | `pk_drop:<death resolution_id>` | whole stack (`Inventory.dropItem`) |
| Pickup, solo | ground: `transfer_out` identity, terminal; picker: `transfer_in` identity or merge | `receipt:pick_up_item:<key>` | adena cap |
| Pickup, solo adena with an existing stack | ground: `transfer_out` → picker `transfer_in` merge | receipt | pinned `addAdena` + `destroyItem("Pickup")` is represented as a merge transfer (equivalent net state) |
| Pickup, party non-split | ground: `transfer_out` → looter: `transfer_in` identity or merge | receipt + `party:<resolution>` | looter may be another member |
| Pickup, party even split | ground: `destroy`, terminal; each recipient: `party_split` fresh or merge | receipt + `party_split:<resolution>` | per-recipient cap; refuse the whole pickup on violation |
| Herb pickup | ground: `destroy` | `herb:<receipt>` | no inventory row |
| Decay | ground: `destroy` | `decay:<tick>` | — |
| Sweep | sweeper or looter: `sweep` fresh/merge; party split: `party_split` | `sweep:<cast_id>` | no source row (holders) |
| Harvest | harvester: `harvest` fresh/merge | `harvest:<cast_id>` | — |
| Seed sow | sower: `C(sow:<cast_id>)` −1 seed | — | consumed before the roll |
| Vendor buy | buyer: `C(receipt:vendor_buy:<key>)` adena; `purchase` fresh/merge per item | receipt | stock is not an item |
| Vendor sell | seller: `transfer_out` (identity/merge/split); refund: `transfer_in`; seller: `sale_proceeds` adena fresh/merge; overflow: refund `destroy` oldest | receipt; `clear_refund:<receipt>` | adena cap |
| Vendor refund | buyer: `C(receipt:vendor_refund:<key>)`; refund: `transfer_out` → buyer `transfer_in` | receipt | — |
| Refund clear on departure | refund: `destroy` each | `clear_refund:departure:<tick>` | B6 |
| Multisell | actor: `C(receipt:multisell_choose:<key>)` per ingredient; `exchange` fresh/merge per product | receipt; one `ItemExchanged` | maintainEnchantment copies (§7) |
| Recipe register | actor: `C(receipt:register_recipe:<key>)` −1 | receipt | — |
| Craft | crafter: (fee in) `transfer_in` merge/fresh from the customer's `transfer_out` (split/merge); customer: `C(receipt:<craft_item / private_manufacture>:<key>)` materials; customer: `craft` fresh/merge on success | receipt | product unchecked (P7b) |
| Trade | per entry: `transfer_out` / `transfer_in` pair | `receipt:trade_confirm:<key>` for **both** sides: the second confirmer's receipt; the first confirmer's row carries `trade:<pair link>` | all or nothing |
| Store buy / sell | adena and item transfer pairs | buyer or seller receipt | — |
| Warehouse deposit | actor: `C(receipt:warehouse_deposit:<key>)` fee; entries: pairs → `warehouse` | receipt | — |
| Warehouse withdraw | pairs `warehouse` → actor | receipt | — |
| Freight send | `C` fee; pairs → `freight:<target>` | receipt | — |
| Freight withdraw | pairs `freight` → actor | receipt | — |
| Mail send | `C` fee; pairs → `mail:<m>` | receipt | — |
| Mail take | COD: receiver `C(receipt:mail_take:<key>)` adena; attachments: pairs `mail:<m>` → receiver | receipt | the COD amount travels in the delivery row, not as an item |
| Mail reject | pairs `mail:<m>` → `mail:<m'>` | receipt | container↔container |
| Mail cancel | pairs `mail:<m>` → sender | receipt | — |
| Delivery | sender: `cod_credit` fresh/merge (COD); `mail:<m>` → `warehouse:<sender>` pairs (expiry); `auction_award` fresh in `warehouse:<winner>` (award) | `delivery:<d>` | P20 |
| Auction bid | actor adena pair → `auction_bid:<a,c>` | receipt | — |
| Auction cancel | `auction_bid` → actor pair | receipt | — |
| Auction win | `auction_bid:<a,winner>`: `destroy` (sink) | `auction_win:<auction>` at finish | winning adena is consumed by the NPC (pinned, never refunded) |
| Auction purge | `auction_bid`: `destroy` | `auction_expired:<auction>` | — |
| Manor buy seed | `C(receipt:manor_buy_seed:<key>)` adena (sink `castle_treasury`); `purchase` fresh/merge | receipt | — |
| Manor sell crop | `C(…)` crop + fee; `manor_reward` fresh/merge | receipt | — |
| Epoch close / void | ground and refund: `destroy` | `epoch_close:<epoch>` / `epoch_void:<epoch>` | §11.6 |

New `item_ledger.op` values (V7 only): `loot`, `transfer_out`, `transfer_in`, `party_split`, `sweep`, `harvest`, `purchase`, `sale_proceeds`, `craft`, `cod_credit`, `auction_award`, `manor_reward`. The existing `consume`, `destroy`, `exchange`, `unequip` and `grant` are reused. `spoil` is **not** an op: spoiling changes no instance (D28).

New `ItemOperation` receipt scopes (I 14+; §13.5): pick_up_item, vendor_buy, vendor_sell, vendor_refund, multisell_choose, register_recipe, delete_recipe, craft_item, trade_confirm, store_buy, store_sell, private_manufacture, warehouse_deposit, warehouse_withdraw, freight_send, freight_withdraw, mail_send, mail_take, mail_reject, mail_cancel, mail_delete, manor_buy_seed, manor_sell_crop, auction_bid, auction_cancel_bid.

Non-item intents keep receipts without ledger rows (D28): party, trade request/answer/add/cancel, store set/close, delete_recipe, mail_delete.

### 11.4 Migrations (E5, E19; R7, R28)

The first free number is **000014**. `M/m20261012_000013_item_ledger_row_ordinal.sql` is D76's forward migration. The numbers must be re-verified against the merged Phase 4 set at implementation. Each migration ships `.sql` + `_down.sql` + `.rs` (the 000012 pattern), and entities are regenerated through `entities.sh`. Every FK to characters or accounts is `uuid`.

| Migration | Contents |
|---|---|
| `m2026XXXX_000014_economy_ledger_owners` | **item_ledger:**<br>• add `owner_kind text NOT NULL DEFAULT 'character'` with a CHECK over §11.1;<br>• add `owner_id uuid NULL`; make `character_id` NULLable, with CHECK `(owner_kind = 'character') = (character_id IS NOT NULL)` and `owner_kind = 'character' OR owner_id IS NOT NULL`;<br>• generated column `owner_key uuid = COALESCE(character_id, owner_id)`;<br>• extend `item_ledger_op` with the §11.3 values;<br>• replace `item_ledger_logical_key` with `UNIQUE(zone_id, epoch, tick, owner_kind, owner_key, instance_id, op, operation_ordinal)`;<br>• replace `item_ledger_consume_batch_key` and the D76 `item_ledger_row_ordinal()` trigger with owner-keyed versions that encode T1 (insert-only, so pre-V7 history is never re-validated).<br>**item_mutation_receipts:** extend the `operation` CHECK with the §11.3 scopes. |
| `…_000015_economy_items` | **item_instances:**<br>• add `owner_kind` and `container_id uuid NULL`;<br>• extend the `location` CHECK to `inventory, paperdoll, warehouse, freight, mail, auction_bid`;<br>• make `character_id` NULLable with CHECK `(location IN ('inventory','paperdoll','warehouse','freight')) = (character_id IS NOT NULL)` and `container_id` required for mail/auction_bid;<br>• slot CHECK unchanged.<br>All D19/D65 columns (`charge_bits`, `enchant`/`augment` bigint u32 CHECKs, `bound`, `created_tick`, `revision`, origin provenance, timestamps) apply unchanged: **one item table for every durable location** (R7, F28 preferred option).<br>**characters:** add `phase5_checkpoint jsonb NULL`. |
| `…_000016_economy_mail` | `mail_messages(id uuid PK, sender_id uuid NOT NULL REFERENCES characters(id), receiver_id uuid NOT NULL REFERENCES characters(id), subject text CHECK(char_length ≤ 128), body text CHECK(≤ 512), cod_adena bigint CHECK(0 ≤ cod ≤ 99900000000), has_attachments boolean, returned boolean, system_kind smallint CHECK(0..3), read boolean, deleted_by_sender boolean, deleted_by_receiver boolean, created_at timestamptz NOT NULL DEFAULT now(), expires_at timestamptz NOT NULL, revision bigint CHECK(≥ 0))`; indexes `(receiver_id) WHERE NOT deleted_by_receiver`, `(sender_id) WHERE NOT deleted_by_sender`, `(expires_at)` |
| `…_000017_economy_auction_manor` | `item_auctions(id bigint PK CHECK(> 0) /* fixture-declared */, instance_npc integer, item_index integer, state smallint CHECK(0..2), extend_state smallint, last_bidder uuid NULL, highest_bidder uuid NULL, highest_bid bigint NULL, start_at timestamptz, end_at timestamptz, revision bigint)`;<br>`item_auction_bids(auction_id bigint REFERENCES item_auctions, character_id uuid REFERENCES characters, last_bid bigint CHECK(-1 OR 1..100000000000), revision bigint, PRIMARY KEY(auction_id, character_id))`;<br>`manor_seed_production(castle_id smallint, seed_id integer, price bigint, start_amount bigint, amount bigint CHECK(0 ≤ amount ≤ start_amount), revision bigint, PK(castle_id, seed_id))`;<br>`manor_crop_procure(castle_id, crop_id, price, start_amount, amount, reward_type smallint CHECK(1..2), revision, PK(castle_id, crop_id))` |
| `…_000018_economy_control` | `economy_leases(lease_key text PK, zone_id integer, epoch bigint, account_id uuid REFERENCES accounts(id), operation text, request_key uuid, acquired_at timestamptz DEFAULT now())`;<br>`economy_deliveries(id uuid PK, target_character uuid REFERENCES characters(id), kind text CHECK IN ('cod_payment','mail_expiry_return','auction_award'), payload jsonb, status text CHECK IN ('pending','delivered'), delivered_zone integer NULL, delivered_epoch bigint NULL, delivered_tick bigint NULL, created_at timestamptz DEFAULT now(), revision bigint)`;<br>`economy_counters(day date, counter text, value numeric(39,0), PRIMARY KEY(day, counter))` and `economy_counter_facts(fact_id uuid PK, processed_at timestamptz)` (B8 dedupe) |

Down migrations drop the added objects in reverse order. They refuse when V7 rows exist, which is a CHECK in the down script.

### 11.5 Checkpoint transaction and outbox (A13)

Per tick, one revision-fenced SeaORM transaction contains:
- character checkpoints, including `phase5_checkpoint`;
- `item_instances` changes for character **and** container locations;
- ledger rows for every owner kind, in T1 order;
- container row deltas (CAS on `revision`);
- delivery inserts and status changes;
- lease deletions;
- receipts;
- outbox facts.

Ground and refund owners have ledger rows but **no** `item_instances` rows (snapshot only).

Outbox subjects, extending A20/C9: `nightfall.item.{looted, picked_up, dropped_pk, swept, harvested, purchased, sold, refunded, crafted, traded, deposited, withdrawn, freighted, mailed, mail_taken, mail_returned, delivered, auction_bid, auction_refunded, auction_awarded, manor_traded, decayed}`, plus the existing `destroyed` and `exchanged`. `economy_counters` is filled **asynchronously** by the outbox consumer, idempotent by `fact_id` (B8). Metric labels are bounded: operation kind, outcome, item class, grade (A19).

### 11.6 Epoch close and abandoned epochs

- **Graceful close.** A graceful epoch close emits, in its final tick, `destroy` rows for every remaining ground item and refund entry, with cause `epoch_close:<epoch>`. Pinned `SaveDroppedItem = False` drops ground items at shutdown.
- **Abandoned epoch** (crash with no successor recovery). The next epoch of the same zone executes a system command `EpochReconcile{abandoned_epoch, ground and refund manifest from the last fenced snapshot}` before admitting players. It writes `destroy` rows with cause `epoch_void:<epoch>`, so every live instance in the ledger has a terminal fact.
- Leases of the closed or abandoned epoch are released in the same transaction.

## 12. Replay, codec and digest

### 12.1 Strict V7 boundary (A4; F29, R26, R27)

| Item | Value |
|---|---|
| Digest | `StateDigestVersion::BinaryV7`, new variant after `BinaryV6` (`Z/state.rs:72-86`) |
| Record schema | `RECORD_SCHEMA_VERSION = 9` (`R/codec.rs:706` is 8) |
| Snapshot | version 11, magic **`b"NFSNAP11"`** (8-byte ASCII, following `b"NFSNAP9\0"` and `b"NFSNAP10"` at `R/record.rs:224-227`) |
| Runtime gate | **`PHASE5_ENABLED`**: unset → V7, except historical simulation-fixture selection; `"0"` → V6 behaviour; `"1"` → V7; any other value is a startup error. The recorded version wins on replay. V7 requires valid Phase 3 + Phase 4 + Phase 5 blocks and all three rules hashes. Replay admission policy `version_three`. |
| Legacy | Snapshot ≤ 10, Record ≤ 8 and BinaryV6 and older replay byte-exactly. `apps/api/fixtures/sessions/*.nfr` stay untouched (`replay-check.sh`). Decoders reject any Phase 5 command kind, output tag, player-load field, entity kind 3, V7 `ItemLocation`, or new ledger op under Record ≤ 8, and they check recursively before permissive decoding. |

High-water marks were checked at 80b8077:
- `PbCommandKind` top 31 (`R/codec.rs:164-201`);
- `PbPlayerLoad` top 10 (`:239-261`);
- `PbOutput` top **41**: D26's `Phase4Enriched` is now in code (`:470-505`), and tag 28 is a historical gap.

The first free tags are therefore 32 / 11 / 42.

### 12.2 Command kinds (PbCommandKind 32..74; payload `bytes` = canonical integer-only JSON)

The mapping is exact: `command = client_tag + 3` for the 40 WebSocket intents (DR2 R11 asked for a 1:1 table). System commands have no client tag.

| ClientMessage tag | Intent | PbCommandKind |
|---|---|---|
| 29 | PickUpItem | 32 |
| 30 | VendorBuy | 33 |
| 31 | VendorSell | 34 |
| 32 | VendorRefund | 35 |
| 33 | MultisellChoose | 36 |
| 34 | RegisterRecipe | 37 |
| 35 | DeleteRecipe | 38 |
| 36 | CraftItem | 39 |
| 37 | TradeRequest | 40 |
| 38 | TradeAnswer | 41 |
| 39 | TradeAddItem | 42 |
| 40 | TradeConfirm | 43 |
| 41 | TradeCancel | 44 |
| 42 | SetPrivateStoreSell | 45 |
| 43 | SetPrivateStoreBuy | 46 |
| 44 | SetPrivateManufacture | 47 |
| 45 | ClosePrivateStore | 48 |
| 46 | PrivateStoreBuy | 49 |
| 47 | PrivateStoreSell | 50 |
| 48 | PrivateManufactureOrder | 51 |
| 49 | PartyInvite | 52 |
| 50 | PartyAnswer | 53 |
| 51 | PartyLeave | 54 |
| 52 | PartyKick | 55 |
| 53 | PartyChangeLeader | 56 |
| 54 | PartyLootChangeRequest | 57 |
| 55 | PartyLootChangeAnswer | 58 |
| 56 | WarehouseDeposit | 59 |
| 57 | WarehouseWithdraw | 60 |
| 58 | FreightSend | 61 |
| 59 | FreightWithdraw | 62 |
| 60 | MailSend | 63 |
| 61 | MailTakeAttachments | 64 |
| 62 | MailReject | 65 |
| 63 | MailCancel | 66 |
| 64 | MailDelete | 67 |
| 65 | ManorBuySeed | 68 |
| 66 | ManorSellCrop | 69 |
| 67 | ItemAuctionBid | 70 |
| 68 | ItemAuctionCancelBid | 71 |
| — | DeliverPending (system, §10.4) | 72 |
| — | AuctionSchedule (system: Create/Start/Finish/Purge, §10.5) | 73 |
| — | EpochReconcile (system, §11.6) | 74 |

Every WS-intent payload contains the client body, the authenticated owner binding, the request key and fingerprint, and `frozen` (§10.0) when the intent is lease-bound. System kinds 72–74 carry only frozen API inputs. `UseItem` (existing 22) is extended under V7 with the seed and harvester handlers. It is not a new kind.

**Frozen payloads (E12, R10).**

| Kind | Frozen inputs |
|---|---|
| VendorBuy/Sell/Refund, Multisell, Craft, Trade, Store, Party, Pickup | none: everything is zone state |
| WarehouseDeposit/Withdraw | the warehouse pre-image `{revision, rows[]}` |
| FreightSend | the target's same-account membership (bool) and the target's freight pre-image |
| FreightWithdraw | own freight pre-image |
| MailSend | receiver resolution `{receiver_id or not_found, is_self, blocked = false}`, sender outbox count, receiver inbox count, `message_id` |
| MailTake/Reject/Cancel/Delete | message pre-image(s) incl. attachment rows and `expired` (DB time) |
| ManorBuySeed/SellCrop | production/procure row pre-images, manor mode (fixture APPROVED), seed↔crop reward resolution |
| ItemAuctionBid | auction row (state, highest, extend_state, last_bidder), the actor's bid row, `remaining_le_10min` (DB time) |
| ItemAuctionCancelBid | all non-CREATED auctions of the instance with the actor's bids, each with `older_than_14d` (DB time) |
| DeliverPending | delivery row plus source container pre-images |
| AuctionSchedule | auction row, fixture item list index, transition |
| EpochReconcile | abandoned-epoch ground/refund manifest (from the last fenced snapshot) |

### 12.3 Player load and outputs

`PbPlayerLoad` tag **11** `phase5: bytes` carries the `Phase5Checkpoint`.

`PbOutput` 42..55:

| Tag | Carrier |
|---|---|
| 42 | `Phase5Enriched` — the D26 pattern; carries the V7-only additions to existing events: `EntitySpawn.kind = GROUND_ITEM` + `GroundItemDetail`. Legacy `Spawn = 1` / `Despawn = 3` stay byte-identical for players and NPCs. A ground item despawn uses the existing `Despawn = 3`. |
| 43 | `EconomyResult` |
| 44 | `CorpseStateChanged` |
| 45 | `PartyStateChanged` |
| 46 | `PartyLootVote` |
| 47 | `PartyInviteReceived` |
| 48 | `TradeRequestReceived` |
| 49 | `TradeSessionChanged` |
| 50 | `PrivateStoreChanged` |
| 51 | `MailNotification` |
| 52 | `RecipeBookSync` |
| 53 | `ItemAuctionStatusChanged` |
| 54 | `LootDistributed` |
| 55 | `EconomyCheckpoint` — the durable non-character ledger rows, container deltas, lease releases and delivery changes of the tick (counterpart of `ItemCheckpoint` = 40, which keeps character inventories, with the V7 ops) |

Record 9 also admits tag 41 (`Phase4Enriched`) unchanged, so V7 epochs may emit Phase 4 enrichments as before.

### 12.4 Digest coverage (R29)

The V7 digest appends, after the V6 layout and in this order, using the explicit little-endian encodings of `Z/state_digest.rs`:
1. the Phase5Rules hash;
2. ground items in `EntityId` order (all `GroundItem` fields, including both protection pairs, `decay_due`, `dropped_tick` and `source`);
3. NPC sidecars (spoiler, seed, seeded, harvest, sweep holders);
4. parties (members in list order, mode, cursor, vote, pending invite) and `next_party`;
5. requests;
6. trades (lists, confirmed, locked);
7. stores;
8. vendor stock;
9. per player: recipes, refund tab and flood next-allowed ticks.

The RNG word position is already digested. DB-only rows are not zone state and are not digested. Their frozen pre-images are inside the recorded commands, which the record hash covers.

## 13. Additive wire and API mapping (E20 source of truth; F14/F30, R11, R33)

There are no new envelopes or streams (A3). New message types live in `packages/proto/nightfall/v1/economy.proto`, which `world.proto` imports. Tags were verified at 80b8077:
- ClientMessage top 28 (W:35-55);
- WorldEvent top 33 (W:188-222);
- RejectReason top 37 (W:146-185);
- EntityKind top 2 (W:225-229);
- EntitySpawn top 29 (W:266);
- StatsChanged top 26 (unused by Phase 5);
- ItemOperation top 13 (I:76-91).

### 13.1 Oneof and enum lines (complete)

```proto
// world.proto — ClientMessage.intent (append)
    PickUpItemRequest pick_up_item = 29;
    VendorBuyRequest vendor_buy = 30;
    VendorSellRequest vendor_sell = 31;
    VendorRefundRequest vendor_refund = 32;
    MultisellChooseRequest multisell_choose = 33;
    RegisterRecipeRequest register_recipe = 34;
    DeleteRecipeRequest delete_recipe = 35;
    CraftItemRequest craft_item = 36;
    TradeRequestRequest trade_request = 37;
    TradeAnswerRequest trade_answer = 38;
    TradeAddItemRequest trade_add_item = 39;
    TradeConfirmRequest trade_confirm = 40;
    TradeCancelRequest trade_cancel = 41;
    SetPrivateStoreSellRequest set_private_store_sell = 42;
    SetPrivateStoreBuyRequest set_private_store_buy = 43;
    SetPrivateManufactureRequest set_private_manufacture = 44;
    ClosePrivateStoreRequest close_private_store = 45;
    PrivateStoreBuyRequest private_store_buy = 46;
    PrivateStoreSellRequest private_store_sell = 47;
    PrivateManufactureOrderRequest private_manufacture_order = 48;
    PartyInviteRequest party_invite = 49;
    PartyAnswerRequest party_answer = 50;
    PartyLeaveRequest party_leave = 51;
    PartyKickRequest party_kick = 52;
    PartyChangeLeaderRequest party_change_leader = 53;
    PartyLootChangeRequest party_loot_change_request = 54;
    PartyLootChangeAnswer party_loot_change_answer = 55;
    WarehouseDepositRequest warehouse_deposit = 56;
    WarehouseWithdrawRequest warehouse_withdraw = 57;
    FreightSendRequest freight_send = 58;
    FreightWithdrawRequest freight_withdraw = 59;
    MailSendRequest mail_send = 60;
    MailTakeAttachmentsRequest mail_take_attachments = 61;
    MailRejectRequest mail_reject = 62;
    MailCancelRequest mail_cancel = 63;
    MailDeleteRequest mail_delete = 64;
    ManorBuySeedRequest manor_buy_seed = 65;
    ManorSellCropRequest manor_sell_crop = 66;
    ItemAuctionBidRequest item_auction_bid = 67;
    ItemAuctionCancelBidRequest item_auction_cancel_bid = 68;

// world.proto — WorldEvent.event (append)
    EconomyResult economy_result = 34;               // owner-only frozen receipt
    CorpseStateChanged corpse_state_changed = 35;    // public
    PartyStateChanged party_state_changed = 36;      // party members
    PartyLootVote party_loot_vote = 37;              // party members
    PartyInviteReceived party_invite_received = 38;  // invite target
    TradeRequestReceived trade_request_received = 39;// trade target
    TradeSessionChanged trade_session_changed = 40;  // both participants
    PrivateStoreChanged private_store_changed = 41;  // AOI
    MailNotification mail_notification = 42;         // receiver
    RecipeBookSync recipe_book_sync = 43;            // owner baseline/delta
    ItemAuctionStatusChanged item_auction_status_changed = 44; // bidders + auctioneer AOI
    LootDistributed loot_distributed = 45;           // party members except recipient

// world.proto — enum EntityKind (append)
  ENTITY_KIND_GROUND_ITEM = 3;

// world.proto — message EntitySpawn (append)
  GroundItemDetail ground_item = 30;    // present iff kind == ENTITY_KIND_GROUND_ITEM

// world.proto — enum RejectReason (append; EXTENDS the existing enum, never a parallel one — E20)
  REJECT_REASON_GROUND_ITEM_NOT_FOUND = 38;
  REJECT_REASON_GROUND_ITEM_PROTECTED = 39;   // DropProtection or loot owner
  REJECT_REASON_INVALID_MERCHANT = 40;        // not a merchant/keeper/manager, list not allowed
  REJECT_REASON_INVALID_QUANTITY = 41;
  REJECT_REASON_VENDOR_OUT_OF_STOCK = 42;
  REJECT_REASON_ADENA_LIMIT = 43;             // MaxAdena / price overflow guards
  REJECT_REASON_WEIGHT_LIMIT = 44;
  REJECT_REASON_INSUFFICIENT_ADENA = 45;
  REJECT_REASON_ITEM_NOT_SELLABLE = 46;
  REJECT_REASON_ITEM_COUNT_CAP = 47;          // D68/D70 receiver cap
  REJECT_REASON_RECIPE_ALREADY_REGISTERED = 48;
  REJECT_REASON_RECIPE_BOOK_FULL = 49;
  REJECT_REASON_CRAFT_SKILL_MISSING = 50;
  REJECT_REASON_CRAFT_LEVEL_TOO_LOW = 51;
  REJECT_REASON_MISSING_MATERIALS = 52;
  REJECT_REASON_IN_COMBAT = 53;
  REJECT_REASON_NO_PENDING_REQUEST = 54;
  REJECT_REASON_ALREADY_TRADING = 55;
  REJECT_REASON_TARGET_BUSY = 56;
  REJECT_REASON_CANNOT_TRADE_IN_STORE = 57;
  REJECT_REASON_CANNOT_ADJUST_AFTER_CONFIRM = 58;
  REJECT_REASON_ITEM_NOT_TRADABLE = 59;
  REJECT_REASON_STORE_SLOT_LIMIT = 60;
  REJECT_REASON_STORE_NOT_OPEN = 61;
  REJECT_REASON_PACKAGE_INCOMPLETE = 62;
  REJECT_REASON_PRICE_MISMATCH = 63;
  REJECT_REASON_PARTY_FULL = 64;
  REJECT_REASON_PARTY_ALREADY_IN_PARTY = 65;
  REJECT_REASON_NOT_PARTY_LEADER = 66;
  REJECT_REASON_NOT_PARTY_MEMBER = 67;
  REJECT_REASON_PARTY_LOOT_VOTE_IN_PROGRESS = 68;
  REJECT_REASON_CONTAINER_BUSY = 69;          // lease held (retryable)
  REJECT_REASON_WAREHOUSE_FULL = 70;
  REJECT_REASON_ITEM_NOT_DEPOSITABLE = 71;
  REJECT_REASON_FREIGHT_TARGET_INVALID = 72;
  REJECT_REASON_ITEM_NOT_FREIGHTABLE = 73;
  REJECT_REASON_FREIGHT_FULL = 74;
  REJECT_REASON_PEACE_ZONE_REQUIRED = 75;
  REJECT_REASON_MAIL_ATTACHMENTS_EXCEEDED = 76;
  REJECT_REASON_MAIL_RECEIVER_NOT_FOUND = 77;
  REJECT_REASON_MAILBOX_FULL = 78;
  REJECT_REASON_MAIL_NOT_FOUND = 79;          // absent, expired, or not the actor's
  REJECT_REASON_MAIL_ALREADY_TAKEN = 80;
  REJECT_REASON_SWEEP_NOT_ALLOWED = 81;       // not spoiler/party, old corpse, not spoiled
  REJECT_REASON_NOT_SEEDABLE = 82;            // Seed handler failures incl. wrong castle area
  REJECT_REASON_HARVEST_NOT_ALLOWED = 83;
  REJECT_REASON_MANOR_UNAVAILABLE = 84;
  REJECT_REASON_AUCTION_NOT_ACTIVE = 85;
  REJECT_REASON_BID_TOO_LOW = 86;
  REJECT_REASON_BID_TOO_HIGH = 87;            // > 100,000,000,000
  REJECT_REASON_NO_BID_TO_CANCEL = 88;

// items.proto — enum ItemOperation (append; receipt scopes, §11.3)
  ITEM_OPERATION_PICK_UP_ITEM = 14;  ITEM_OPERATION_VENDOR_BUY = 15;  ITEM_OPERATION_VENDOR_SELL = 16;
  ITEM_OPERATION_VENDOR_REFUND = 17; ITEM_OPERATION_MULTISELL_CHOOSE = 18; ITEM_OPERATION_REGISTER_RECIPE = 19;
  ITEM_OPERATION_DELETE_RECIPE = 20; ITEM_OPERATION_CRAFT_ITEM = 21;  ITEM_OPERATION_TRADE_CONFIRM = 22;
  ITEM_OPERATION_STORE_BUY = 23;     ITEM_OPERATION_STORE_SELL = 24;  ITEM_OPERATION_PRIVATE_MANUFACTURE = 25;
  ITEM_OPERATION_WAREHOUSE_DEPOSIT = 26; ITEM_OPERATION_WAREHOUSE_WITHDRAW = 27; ITEM_OPERATION_FREIGHT_SEND = 28;
  ITEM_OPERATION_FREIGHT_WITHDRAW = 29;  ITEM_OPERATION_MAIL_SEND = 30; ITEM_OPERATION_MAIL_TAKE = 31;
  ITEM_OPERATION_MAIL_REJECT = 32;   ITEM_OPERATION_MAIL_CANCEL = 33; ITEM_OPERATION_MAIL_DELETE = 34;
  ITEM_OPERATION_MANOR_BUY_SEED = 35; ITEM_OPERATION_MANOR_SELL_CROP = 36; ITEM_OPERATION_AUCTION_BID = 37;
  ITEM_OPERATION_AUCTION_CANCEL_BID = 38; ITEM_OPERATION_PARTY = 39;  ITEM_OPERATION_TRADE_SESSION = 40;
  ITEM_OPERATION_STORE_SETUP = 41;
```

### 13.2 Message bodies (`economy.proto`)

Every **mutating** intent carries `string idempotency_key = 1`, a UUID key, as in Phase 4 (`I:115-119`). The same key with the same body replays the frozen receipt; the same key with a different body is rejected before mutation; a rejection consumes no key (D41–D43). The owner always comes from the session. Entity references are UUID strings as in Phase 4. Ids are `uint64`; counts and prices are `uint64` with server range checks.

```proto
message PickUpItemRequest { string idempotency_key = 1; string ground_entity = 2; }
message ItemCount { uint64 instance_id = 1; uint64 count = 2; }
message TemplateCount { uint32 template_id = 1; uint64 count = 2; }
message VendorBuyRequest { string idempotency_key = 1; string merchant = 2; uint64 list_id = 3; repeated TemplateCount items = 4; }
message VendorSellRequest { string idempotency_key = 1; string merchant = 2; uint64 list_id = 3; repeated ItemCount items = 4; }
message VendorRefundRequest { string idempotency_key = 1; string merchant = 2; uint64 list_id = 3; repeated uint32 refund_indices = 4; }
message MultisellChooseRequest { string idempotency_key = 1; string npc = 2; uint32 list_id = 3; uint32 entry_id = 4; uint64 amount = 5; }
message RegisterRecipeRequest { string idempotency_key = 1; uint64 recipe_instance_id = 2; }
message DeleteRecipeRequest { string idempotency_key = 1; uint32 recipe_id = 2; }
message CraftItemRequest { string idempotency_key = 1; uint32 recipe_id = 2; }
message TradeRequestRequest { string idempotency_key = 1; string target = 2; }
message TradeAnswerRequest { string idempotency_key = 1; bool accept = 2; }          // single active requester (R33)
message TradeAddItemRequest { string idempotency_key = 1; uint64 instance_id = 2; uint64 count = 3; }
message TradeConfirmRequest { string idempotency_key = 1; }
message TradeCancelRequest { string idempotency_key = 1; }
message StoreSellEntry { uint64 instance_id = 1; uint64 count = 2; uint64 price = 3; }
message StoreBuyEntry { uint32 template_id = 1; uint32 enchant = 2; uint64 count = 3; uint64 price = 4; }
message ManufactureEntry { uint32 recipe_id = 1; uint64 cost = 2; }
message SetPrivateStoreSellRequest { string idempotency_key = 1; string title = 2; bool package_sale = 3; repeated StoreSellEntry entries = 4; }
message SetPrivateStoreBuyRequest { string idempotency_key = 1; string title = 2; repeated StoreBuyEntry entries = 3; }
message SetPrivateManufactureRequest { string idempotency_key = 1; string title = 2; repeated ManufactureEntry entries = 3; }
message ClosePrivateStoreRequest { string idempotency_key = 1; }
message PrivateStoreBuyRequest { string idempotency_key = 1; string store_owner = 2; repeated StoreSellEntry entries = 3; }
message PrivateStoreSellEntry { uint64 instance_id = 1; uint32 template_id = 2; uint64 count = 3; uint64 price = 4; }
message PrivateStoreSellRequest { string idempotency_key = 1; string store_owner = 2; repeated PrivateStoreSellEntry entries = 3; }
message PrivateManufactureOrderRequest { string idempotency_key = 1; string store_owner = 2; uint32 recipe_id = 3; }
enum LootMode { LOOT_MODE_UNSPECIFIED = 0; LOOT_MODE_FINDERS_KEEPERS = 1; LOOT_MODE_RANDOM = 2;
                LOOT_MODE_RANDOM_INCLUDING_SPOIL = 3; LOOT_MODE_BY_TURN = 4; LOOT_MODE_BY_TURN_INCLUDING_SPOIL = 5; }
                // wire = pinned PartyDistributionType id + 1; UNSPECIFIED rejects INVALID
message PartyInviteRequest { string idempotency_key = 1; string target = 2; LootMode mode = 3; }
message PartyAnswerRequest { string idempotency_key = 1; int32 answer = 2; }    // pinned -1 / 0 / 1
message PartyLeaveRequest { string idempotency_key = 1; }
message PartyKickRequest { string idempotency_key = 1; string member = 2; }
message PartyChangeLeaderRequest { string idempotency_key = 1; string member = 2; }
message PartyLootChangeRequest { string idempotency_key = 1; LootMode mode = 2; }
message PartyLootChangeAnswer { string idempotency_key = 1; bool accept = 2; }
message WarehouseDepositRequest { string idempotency_key = 1; string keeper = 2; repeated ItemCount entries = 3; }
message WarehouseWithdrawRequest { string idempotency_key = 1; string keeper = 2; repeated ItemCount entries = 3; }
message FreightSendRequest { string idempotency_key = 1; string keeper = 2; string target_character = 3; repeated ItemCount entries = 4; }
message FreightWithdrawRequest { string idempotency_key = 1; string keeper = 2; repeated ItemCount entries = 3; }
message MailSendRequest { string idempotency_key = 1; string receiver_name = 2; string subject = 3; string body = 4;
                          uint64 cod_adena = 5; repeated ItemCount attachments = 6; }
message MailTakeAttachmentsRequest { string idempotency_key = 1; string message_id = 2; }
message MailRejectRequest { string idempotency_key = 1; string message_id = 2; }
message MailCancelRequest { string idempotency_key = 1; string message_id = 2; }
message MailDeleteRequest { string idempotency_key = 1; repeated string message_ids = 2; bool sent_box = 3; }
message ManorBuySeedRequest { string idempotency_key = 1; string manager = 2; uint32 manor_id = 3; repeated TemplateCount seeds = 4; }
message CropSale { uint64 instance_id = 1; uint32 crop_id = 2; uint32 manor_id = 3; uint64 count = 4; }
message ManorSellCropRequest { string idempotency_key = 1; string manager = 2; repeated CropSale entries = 3; }
message ItemAuctionBidRequest { string idempotency_key = 1; uint32 instance_npc_id = 2; uint64 bid = 3; }
message ItemAuctionCancelBidRequest { string idempotency_key = 1; string auctioneer = 2; uint32 instance_npc_id = 3; }

message GroundItemDetail {
  uint64 item_instance_id = 1; uint32 template_id = 2; uint64 count = 3; uint32 enchant = 4;
  string loot_owner = 5; uint64 loot_until_tick = 6;            // empty / 0 when none
  string drop_protect_owner = 7; uint64 drop_protect_until_tick = 8;
  uint64 dropped_tick = 9; bool herb = 10;
}

// Frozen receipt: the D53 receipt fields of ItemOperationResult (I:342-355), same meanings, plus the operation outcome.
message EconomyResult {
  bytes entity_id = 1; uint32 incarnation = 2; uint64 tick = 3; uint64 inventory_revision = 4;
  string receipt_id = 5; ItemOperation operation = 6; repeated uint64 affected_instance_ids = 7;
  bytes request_key = 8; string request_fingerprint = 9; optional uint64 accepted_client_seq = 10;
  oneof outcome {
    PickupOutcome pickup = 20; VendorOutcome vendor = 21; CraftOutcome craft = 22; ExchangeOutcome multisell = 23;
    TradeOutcome trade = 24; StoreOutcome store = 25; ContainerOutcome container = 26; MailOutcome mail = 27;
    ManorOutcome manor = 28; AuctionOutcome auction = 29; PartyOutcome party = 30; RecipeOutcome recipe = 31;
  }
}
message PickupOutcome { uint32 template_id = 1; uint64 count = 2; string recipient = 3; bool herb = 4; bool split = 5; }
message VendorOutcome { uint64 adena_delta_abs = 1; bool adena_paid = 2; repeated uint32 refund_order = 3; }
message CraftOutcome { bool success = 1; bool masterwork = 2; uint32 product_template = 3; uint64 count = 4; uint64 fee = 5; }
message ExchangeOutcome { uint32 list_id = 1; uint32 entry_id = 2; uint64 amount = 3; }
message TradeOutcome { string partner = 1; repeated ItemCount given = 2; repeated ItemCount received = 3; }
message StoreOutcome { string counterparty = 1; repeated ItemCount items = 2; uint64 adena = 3; repeated uint32 skipped = 4; }
message ContainerOutcome { string container = 1; uint64 fee = 2; repeated ItemCount moved = 3; }
message MailOutcome { string message_id = 1; uint64 fee = 2; uint64 cod_paid = 3; uint32 processed = 4; }
message ManorOutcome { repeated TemplateCount granted = 1; uint64 adena = 2; uint64 fee = 3; }
message AuctionOutcome { uint64 auction_id = 1; uint64 bid = 2; uint64 paid = 3; uint64 refunded = 4; bool reserve_not_met = 5; }
message PartyOutcome { uint64 party_id = 1; }
message RecipeOutcome { uint32 recipe_id = 1; bool dwarven = 2; }

message CorpseStateChanged { string npc = 1; uint64 tick = 2; bool spoiled = 3; bool seeded = 4; bool sweepable = 5; bool swept = 6; }
message PartyMember { string entity = 1; uint32 level = 2; }
message PartyStateChanged { uint64 party_id = 1; repeated PartyMember members = 2; LootMode mode = 3; bool baseline = 4;
                            bool disbanded = 5; uint64 tick = 6; }
enum LootVotePhase { LOOT_VOTE_PHASE_UNSPECIFIED = 0; LOOT_VOTE_PHASE_REQUESTED = 1; LOOT_VOTE_PHASE_APPLIED = 2; LOOT_VOTE_PHASE_CANCELLED = 3; }
message PartyLootVote { uint64 party_id = 1; LootMode mode = 2; LootVotePhase phase = 3; uint64 expires_tick = 4; }
message PartyInviteReceived { string requester = 1; LootMode mode = 2; uint64 expires_tick = 3; }
message TradeRequestReceived { string requester = 1; uint64 expires_tick = 2; }
message TradeEntry { uint64 instance_id = 1; uint32 template_id = 2; uint64 count = 3; uint32 enchant = 4; }
enum TradePhase { TRADE_PHASE_UNSPECIFIED = 0; TRADE_PHASE_STARTED = 1; TRADE_PHASE_ITEM_ADDED = 2; TRADE_PHASE_PARTNER_CONFIRMED = 3;
                  TRADE_PHASE_COMPLETED = 4; TRADE_PHASE_CANCELLED = 5; }
message TradeSessionChanged { string partner = 1; TradePhase phase = 2;
                              repeated TradeEntry own = 3; repeated TradeEntry partner_items = 4; bool own_confirmed = 5;
                              bool partner_confirmed = 6; string cancel_reason = 7; uint64 tick = 8; }
enum StoreKind { STORE_KIND_UNSPECIFIED = 0; STORE_KIND_NONE = 1; STORE_KIND_SELL = 2; STORE_KIND_PACKAGE_SELL = 3; STORE_KIND_BUY = 4; STORE_KIND_MANUFACTURE = 5; }
message PrivateStoreChanged { string owner = 1; StoreKind kind = 2;
                              string title = 3; repeated StoreSellEntry sell = 4; repeated StoreBuyEntry buy = 5;
                              repeated ManufactureEntry manufacture = 6; uint64 tick = 7; }
message MailNotification { string message_id = 1; string sender_name = 2; string subject = 3; bool has_attachments = 4; uint64 cod_adena = 5; }
message RecipeBookSync { repeated uint32 dwarven = 1; repeated uint32 common = 2; uint32 dwarven_limit = 3; uint32 common_limit = 4;
                         bool baseline = 5; uint64 tick = 6; uint32 incarnation = 7; }
enum AuctionState { AUCTION_STATE_UNSPECIFIED = 0; AUCTION_STATE_CREATED = 1; AUCTION_STATE_STARTED = 2; AUCTION_STATE_FINISHED = 3; }
message ItemAuctionStatusChanged { uint32 instance_npc_id = 1; uint64 auction_id = 2; uint32 template_id = 3; uint64 highest_bid = 4;
                                   AuctionState state = 5; uint64 init_bid = 6; bool extended = 7; }
message LootDistributed { string picker = 1; string recipient = 2; uint32 template_id = 3; uint64 count = 4; LootMode mode = 5; bool spoil = 6; }
```

`DeleteRecipe`, `TradeRequest/Answer/Add/Cancel`, party intents and store set/close carry keys and receipts but write no ledger rows (D28). `EconomyResult` is **always** an accept. A refusal is the existing `IntentRejected{RejectReason}` (W:140-144) and consumes no key (Phase 4 pattern; R11).

### 13.3 Intent → command → event → output mapping (1:1)

The rows below are the **complete** set.

| Intent (CM tag) | PbCommandKind | Success events (WorldEvent → PbOutput) |
|---|---|---|
| PickUpItem (29) | 32 | EntityDespawn (existing 3) → existing; `EconomyResult` (34 → 43); `LootDistributed` (45 → 54) if in a party; `InventoryChanged`/`WeightChanged` (existing) |
| VendorBuy/Sell/Refund (30–32) | 33–35 | `EconomyResult`, inventory deltas |
| MultisellChoose (33) | 36 | `EconomyResult` |
| RegisterRecipe / DeleteRecipe (34–35) | 37–38 | `EconomyResult`, `RecipeBookSync` delta (43 → 52) |
| CraftItem (36) | 39 | `EconomyResult(craft)` |
| TradeRequest (37) | 40 | `TradeRequestReceived` to the target (39 → 48), `EconomyResult` |
| TradeAnswer / TradeAddItem / TradeConfirm / TradeCancel (38–41) | 41–44 | `TradeSessionChanged` (40 → 49) to both, `EconomyResult` |
| SetPrivateStore* / ClosePrivateStore (42–45) | 45–48 | `PrivateStoreChanged` (41 → 50) |
| PrivateStoreBuy / PrivateStoreSell / PrivateManufactureOrder (46–48) | 49–51 | `EconomyResult` for both parties, `PrivateStoreChanged` |
| Party* (49–55) | 52–58 | `PartyStateChanged` (36 → 45), `PartyLootVote` (37 → 46), `PartyInviteReceived` (38 → 47) |
| Warehouse/Freight (56–59) | 59–62 | `EconomyResult(container)` |
| Mail* (60–64) | 63–67 | `EconomyResult(mail)`; `MailNotification` (42 → 51) to the receiver after commit |
| ManorBuySeed / ManorSellCrop (65–66) | 68–69 | `EconomyResult(manor)` |
| ItemAuctionBid / CancelBid (67–68) | 70–71 | `EconomyResult(auction)`, `ItemAuctionStatusChanged` (44 → 53) |
| system DeliverPending | 72 | inventory deltas, `EconomyResult` with `operation = MAIL_TAKE` (COD) or the warehouse view refresh |
| system AuctionSchedule | 73 | `ItemAuctionStatusChanged` |
| system EpochReconcile | 74 | none (ledger only) |
| death (no intent) | — | ground `EntitySpawn` (42 enriched), `CorpseStateChanged` (35 → 44) |
| Spoil / Sweeper / Sow / Harvest casts | existing CastSkill / UseItem | `CorpseStateChanged`, inventory deltas, `EntityDespawn` of a swept corpse |

### 13.4 GameService RPCs (append after `GrantItem`, G:94)

All of these are reads (E14). None mutates zone-canonical inventory.

| RPC | Contract |
|---|---|
| `ListBuylist(merchant_npc_id) → entries {template, unit_price (castle 0), limited, stock}` | Static prices come from Phase5Rules. Stock comes from the zone runtime read port; UNAVAILABLE when the zone is absent. |
| `GetMultisellList(list_id)` | Static, from Phase5Rules. |
| `GetRecipeBook(character)` | Checkpoint view out of world; `FAILED_PRECONDITION` while admitted (D15 pattern: `RecipeBookSync` is the live baseline). |
| `GetWarehouse(character)` / `GetFreight(character)` | DB rows (owner only). |
| `ListMail(character, box)` / `ReadMail(message_id)` | DB rows; expired messages hidden. `ReadMail` marks the message read (a DB-only flag; no item change). Opening a mail **with attachments** requires a peace zone in pinned (`RequestReceivedPost.java:58`); the RPC returns `FAILED_PRECONDITION` unless the zone port confirms the reader is admitted inside a peace square, or out of world (no attachment details shown then). |
| `ListItemAuctions(instance_npc_id)` | DB rows. |
| `GetManorData(castle_id)` | Fixture lists plus current amounts. |

The plan's RPC-routed mutations (DepositWarehouse, WithdrawWarehouse, DepositFreight, WithdrawFreight, SendMail, TakeMailAttachments, CancelMail) are **withdrawn**. They are WS intents (E14), and the plan must sync.

### 13.5 Fold of plan names (E20)

The plan @ 8474b5c names these, which become:
- `TradeInitiate` → `TradeRequest`;
- `TradeRespond` → `TradeAnswer`;
- `CancelPrivateStore` → `ClosePrivateStore`;
- `PrivateStoreManufacture` → `PrivateManufactureOrder`;
- `RequestPartyLootModification` / `AnswerPartyLootModification` → `PartyLootChangeRequest` / `Answer`;
- `PartyAccept` → `PartyAnswer`;
- `SowSeed` → `UseItem` (existing);
- `ManorTurnInCrop` → `ManorSellCrop`.

Its events become:
- `GroundItemSpawn` / `GroundItemDespawn` → `EntitySpawn` / `EntityDespawn`;
- `CraftResult`, `VendorTransactionResult`, `WarehouseResult`, `FreightResult`, `ManorOperationResult` → `EconomyResult`;
- `PartyLootDistributed` → `LootDistributed`.

Its reject reasons are replaced by §13.1. It has no `StatsChanged` fields (recipe counts are in `RecipeBookSync`).

## 14. Independent oracle, regression and acceptance plan

### 14.1 Python Fraction oracle families (`economy_oracle.py`, E1-owned, independent of the Rust code)

Every family reads the pinned XML, Java and properties directly, or the E1 tables after `--check`, plus the oracle's own exact `Fraction` math. The oracle owns an independent `fl()` implementation (§6.2). As the binary64 **reference** only, it also uses CPython `float`, which is IEEE-754 binary64 with correctly rounded basic operations. No family uses tolerances.

| Family | Vectors (minimum) |
|---|---|
| **D-C chance resolution** | (a) ungrouped at C = 99.9…, 100 (no precise branch: `>` is strict), 100.000…1, 188.85499572753906, 0.0003000000142492354; (b) single-item group collapse; (c) multi-item group at G = 100 exactly (precise branch, remainder 0, **remainder draw still taken**); (d) every one of the 151 multi-item groups whose inner sum ≠ 100 (normalized shares); (e) min = max quantity draw taken; (f) the expected draw count per node, asserted against the engine evidence trace |
| **D-W width** | Recompute the maximum bit width of every chance and walk product over all 63,309 rows × every `L` × `G ∈ {1/1, (2^32−1)/1, 1/(2^32−1)}`. Assert ≤ 255 bits and report the measured value (author pre-run: 148 / 98 at G = 1). |
| **D-L killer modifier** | Item window d = −4…−11 → 100, 100, 82, 64, 46, 28, 10, 10 %; adena window sevenths at d = −9…−14; raid clamp d = −1…−7 → 85, 70, 55, 40, 25, 10, 0 %; ungrouped raid uses the window |
| **D-M rate chain** | Each branch (override 57, herb, raid, death, corpse) × `G ∈ {1, 3/2, 1/3}`; surplus → `mult`; amounts unchanged by G |
| **D-O order** | Whole-death traces: corpse → death → positions → jitter, with draw counts per node (§2.3) |
| **D-P PK** | Exclusion set (adena, NonDroppable, PetItems, quest, augmented); unequip-before-roll; break at 10 leaving later equipped items equipped; 40 % gate; offsets [−25, 24]; the NPC-killer branch with `PlayerRateDrop = 0` → no draw |
| **D-T decay** | `decay_due` for drops at ticks 0, 1, 49, 50, 51 with limits 100 / 600 / 6,000 ticks (strict `>` on the 50-tick grid); PK drop never |
| **L-P party loot** | RANDOM over in-range subsets; BY_TURN cursor pre-increment, wrap, skip-advance, full lap → picker, cursor unchanged by leave and join; even split k = 1…9 with remainders and draws with replacement; split at count 1 → mode path; sweep routing in all 5 modes |
| **X XP/SP** | Solo diff −6, −5, 0, 5, 6, 7 ((5/6)^n exact); party n = 1…9 with the BONUS table; `Math.round` halves; highfive gaps 9/10/14/15 and negative; `partyMul` < 1; dead members excluded |
| **V-P price** | All **5,803** `(template, tax)` pairs × castle % {0, 5, 15, 25}: Rust table = oracle `fl` = binary64 reference. Report the 362 pairs where naive exact truncation differs. |
| **V-R restock** | delay literal → minutes (1,500 / 1,428 / 722 / 480 / 361 / 300 / 240 / 120 / 60); due tick; refill; negative-count clamp |
| **V-S sell/refund** | Odd p, c = 1…5: sell `c·⌊p/2⌋`, refund `⌊c·p/2⌋`, difference `⌊c/2⌋` (p = 3, c = 4 → 4, 6, 2); 12-entry overflow destroys the oldest; merge into an existing refund stack does not grow the entry count |
| **M multisell** | amount 1 and 5,000; non-stackable with amount > 1 rejected; maintainEnchantment exact-match consumption and enchant/augment copy; synthetic `isTaxIngredient` half-up at .5; `BlacksmithUseRecipes = True` consumes maintainIngredient rows |
| **C craft** | `chance_bound` mapping for successRate 10/25/60/70/95/100; masterwork 1/4/20 and the non-rare draw (rarity 0); order MP → fee → materials → roll; fee kept on failure |
| **S manor** | Sow basic 90/20 with every penalty branch and **no floor** (negative basic → never succeeds); harvest floor 1; harvest count with strong-type multipliers 4303–4310 and the level bonus; crop sale reward and 5 % `fl` fee |
| **A auction** | Bid equal to highest accepted but not leading; increment-only payment; cancel matrix (leader, finished, 14-day, cancelled); extension 5 → 3 → stop; random item index |
| **F fees** | Warehouse 30 × entries (adena entry counted); freight 1,000 × entries (adena entry adds no slot); mail 100 + 1,000 × n |
| **G ledger** | T1 orders; identity / merge / split pairs; D67 link exemption; D68/D74 caps on merge receivers; conservation per owner and across pairs |

### 14.2 Formula, rounding and adaptation ledger

| Label | Statement | Section |
|---|---|---|
| X1 | 3D pinned distances → 2D | conventions |
| X2 | L2 units → milli-tiles with `trunc0` | conventions |
| X3 | Vendor stock is snapshot-only; it resets at a new epoch | §1.7 |
| X4 | PK inventory iteration in ascending `ItemInstanceId` order | §3.9 |
| X5 | The exact group walk always selects | §3.2 |
| X6 | Raid clamp and `float` multiplier → exact rationals | §3.3–3.4 |
| X7 | maxDealer over `HateLedger` in EntityId order; online = admitted | §3.6 |
| X8 | 5 s decay scanner on the zone 50-tick grid | §3.8 |
| X9 | Even-split recipient order = member order | §4.2 |
| X10 | XP/SP double math → exact rationals; `Math.round` = half-up | §4.5 |
| X11 | Trade cancelled at death and departure (B7) | §9.2 |
| X12 | Cross-zone credits arrive by delivery, a few ticks later | §10.4 |
| P1 | Ground EntityId derivation | §1.2 |
| P2 | Sweep does not advance respawn | §1.3 |
| P3 | Every pinned draw is taken (chance at 0 or ≥ 100, remainder 0, quantity at min = max) | §2.3 |
| P4 | Masterwork draw for non-rare recipes | §2.3 |
| P5 | G is not applied to party bonus rates | §3.4 |
| P6 | Out-of-bounds drop position → source position | §3.5 |
| P7 | Pickup capacity counts slots only | §3.7 |
| P7b | Craft product added without a capacity check | §8.2 |
| P8 | No server-side approach move for pickup | §4.1 |
| P9 | No arrow auto-equip on pickup | §4.1 |
| P10 | Over-hit and vitality are vacuous | §4.5 |
| P11 | Siege-guard float step is only valid at rate 1 and price < 2^24 | §6.2 |
| P12 | Duplicate templates in VendorBuy are refused | §6.3 |
| P13 | VendorSell is all or nothing | §6.5 |
| P14 | Trade validation failure cancels the session | §9.2 |
| P15 | Exchange / store buy is all or nothing; cap refusal | §9.2–9.3 |
| P16 | Store open refuses invalid entries | §9.3 |
| P17 | Warehouse prevalidation before the fee | §10.1 |
| P18 | Non-COD mail stores cod = 0 | §10.3 |
| P19 | Pending deliveries | §10.4 |
| P20 | Deliveries skip the warehouse capacity check (pinned) | §10.4 |
| P21 | Auction extension is persisted | §10.5 |
| T1 | Terminal ledger row after the Consume aggregate | §11.2 |

### 14.3 Evidence contract v2 (A16; R30, F31)

This additively extends the Phase 4 D46 sidecar contract (`apps/client-unreal/Scripts/phase4-evidence-contract.md` on `codex/phase4-sim-harness`) to **schema_version 2**. The export flag is `nightfall-replay export --economy-evidence-out PATH` (whole recording). Version 1 consumers refuse version 2 files; the version 2 consumer accepts both.

- **Manifest:** `kind = manifest`, `schema_version = 2`, `scope = whole_recording`, `recording_sha256`, `zone`, `epoch`, `watermark{…}`, `phase4_rules_hash`, `phase5_rules_hash`, `snapshot_schema = 11`, `record_schema = 9`, `digest_version = BinaryV7`.
- **Common fact envelope:** unchanged: `kind, entity, incarnation, tick, sequence, data`, plus optional `origin`.
- **`item_rng`:** unchanged (`callsite, word_before, word_after, bound, value, resolution_id`). It is used for every `roll_below` draw. The new callsites are `drop_quantity`, `npc_drop_dx`, `npc_drop_dy`, `pk_drop_dx`, `pk_drop_dy`, `party_random`, `party_split_remainder` and `auction_item`.
- **`econ_chance`** (new; one per `draw_u64` chance test): `{callsite, num, den, r, success, word_before, word_after, resolution_id}`.
  - `num`, `den` and `r` are **decimal strings**, because they can exceed 2^53.
  - `num/den` is the reduced percent chance. The consumer recomputes `success = r·100·den < num·2^64`.
  - Callsites: `corpse_item`, `death_item`, `death_remainder`, `group_chance`, `group_remainder`, `pk_gate`, `pk_item`, `craft_success`, `craft_rare`, `sow`, `harvest`.
- **`econ_select`** (new): `{callsite: group_select, r, shares: [{template, w_num, w_den}], selected_index, resolution_id}`.
- **`econ_formula`** (new, same shape as `item_formula`: `formula, inputs, actual, resolution_id, baseline = false`). Formulas:
  - `drop_chance` (inputs: c, M, L, G);
  - `vendor_unit_price` (template, tax, castle_percent);
  - `sell_price`; `refund_price`;
  - `party_xp`; `solo_xp`;
  - `even_split`;
  - `warehouse_fee`; `freight_fee`; `mail_fee`; `crop_reward`; `crop_fee`.
- **`item_delta`** (extended): adds `owner_kind`, `owner_id` (non-character owners) and `transfer{link, mode, peer_owner_kind, peer_owner_id, peer_instance}`. The new `operation` values are those of §11.3. Consume causes follow D62/D63 strings with the new cause kinds: `receipt:<scope>:<key>`, `sow:<cast_id>`, `pk_drop:<death resolution_id>` (unequip rows), `party_split:<resolution>`, `clear_refund:<receipt | departure:tick>`, `decay:<tick>`, `herb:<receipt>`, `delivery:<id>`, `auction_win:<id>`, `auction_expired:<id>`, `epoch_close:<epoch>`, `epoch_void:<epoch>`. Ordinals follow T1.
- **`econ_container`** (new): `{lease_key, row_kind, row_key, before_revision, after_revision, change: insert | update | delete}`, emitted per `ContainerDelta`.
- **Consumer rules:**
  - UNKNOWN is never allow-listed (A16);
  - the consumer refuses unknown kinds, unknown callsites, missing draws and fabricated draws on draw-free branches;
  - every chance result is recomputed exactly;
  - the C14/C15/C16 gates apply.

### 14.4 Rust unit, property and durability targets

- **Engine units.** One per draw row in §2.3 (draw count and order). Every check in §§4–10, in pinned order: first failing check wins. T1 ordinals. Every transfer mode. Lease CAS. Snapshot round-trip and V7 digest stability. V5/V6 byte-exact replay of all historical fixtures (`replay-check.sh`). `PHASE5_ENABLED` matrix.
- **Properties.** Conservation of item counts per template across all owners, except fresh creations and destroy/consume rows. No negative counts. Caps never exceeded at endpoints. Ground entity ids unique. Party invariants.
- **Crash windows.** The §10.0 table, plus the generic Phase 4 table (P4Z §9.2), for: a pickup tick, a party split, a trade, a warehouse deposit with merge, a mail send and take with COD (delivery created, then executed after a crash), an auction bid, a delivery, and an epoch reconcile. Run against memory and Postgres adapters; Postgres needs owned containers on non-default ports, per worker-common.
- **Migrations.** 000014–000018 up/down on an empty DB and on a seeded Phase 4 DB. The T1 trigger accepts the legacy D76 shape and the T1 shape, and rejects others.

### 14.5 Scenario matrix (`apps/client-unreal/Scenarios/5-*.nfs`, fixture packs `phase5-*`)

The plan owns the stories. The design requires at least these scenarios. Each scenario header lists exactly its stories. Expected values are computed from the oracle, never typed in.

- `5-drop-basic`
- `5-drop-precise-over-100`
- `5-drop-level-gap`
- `5-drop-rate-multiplier` (G = 3/2 fixture: base and multiplied values asserted, P2)
- `5-pickup-protection` (loot owner and DropProtection separately)
- `5-herb`
- `5-decay`
- `5-party-loot-{finders,random,random-spoil,by-turn,by-turn-spoil}`
- `5-party-adena-split`
- `5-party-vote`
- `5-party-xp`
- `5-solo-xp-damage-share`
- `5-spoil-sweep`
- `5-sweep-party`
- `5-pk-drop`
- `5-vendor-buy-sell-refund`
- `5-vendor-restock`
- `5-multisell`
- `5-craft-dwarf`
- `5-craft-common-masterwork`
- `5-private-manufacture`
- `5-trade`
- `5-trade-cancel-disconnect`
- `5-store-sell-package`
- `5-store-buy`
- `5-warehouse`
- `5-freight`
- `5-mail-cod`
- `5-mail-reject-cancel-expiry`
- `5-auction-bid-cancel-award`
- `5-manor-sow-harvest-sell`
- `5-epoch-reconcile`

RNG-dependent outcomes are asserted on received state. Exact outcomes are checked by the C14 export and C15 oracle gates (Phase 4 A17 rule).

## 15. QUESTIONs for the architect

- **QUESTION Q1 (PK drop protection).** The approved default says "15 s PK-drop protection", but pinned H5 applies **none**. `onDieDropItem` uses the 4-argument `dropItem("DieDrop", item, killer, true)` (`L2PcInstance.java:4541`), which passes `protectItem = false` (`:3227-3229`). `Inventory.dropItem` also clears the loot owner (`Inventory.java:134`). The default rests on VR5 A39's citation of `:3198-3201`, which runs only when `protectItem` is true.
  - The design defaults to pinned (no protection) behind the hashed flag `pk_drop_protection`.
  - Option: set it to `DropProtection15s`, owner = victim. This is a hash-input change only.
- **QUESTION Q2 (unbounded ground state).** Pinned has no ground-item cap. PK drops never decay (`DestroyPlayerDroppedItem = False`). The snapshot can therefore grow without bound until C10 fails closed.
  - The design keeps pinned behaviour plus the measurement gate.
  - Options: (a) accept as is; (b) a Nightfall per-zone cap with a documented overflow rule, such as "oldest PK drop decays", which would be a deviation.
- **QUESTION Q3 (rule T1).** The design amends D69/D76 for V7: at most one terminal row (`transfer_out` with after = None, or `destroy`) may follow the Consume aggregate, so the aggregate is not always the highest ordinal. Without T1, a stack consumed and moved whole in the same tick cannot be represented, for example shots fired in the killing tick followed by a PK drop.
  - Please ratify T1, or choose the alternative: force split semantics with a fresh id, which breaks B13 identity in that edge case.
- **QUESTION Q4 (pending deliveries).** COD payments, mail-expiry returns and auction awards target characters that may be offline or in another zone. The design delivers them through a zone-executed `DeliverPending` command into the pinned destinations: COD → inventory; expiry and award → warehouse. This happens at the target's admission or immediately while admitted. It avoids DB-only inventory mutation (D7/D30), at the cost of a short delay for online cross-zone targets (X12).
  - Please confirm the mechanism, and confirm that offline COD credit lands in the **inventory**, as pinned, rather than in the warehouse.
- **QUESTION Q5 (manor castle areas).** The Seed handler requires `seed.castle == MapRegionManager.getAreaCastle(player)` (`DPJ/handlers/itemhandlers/Seed.java`). Nightfall zones have no L2 map regions. Phase5Rules needs a hashed `castle_area` per fixture zone.
  - Which castle area (1–9) does the Phase 5 test zone represent?
  - Which castle(s) carry the fixture manor lists?
- **QUESTION Q6 (P2 scope for party XP rates).** P2 puts "XP-type rates" under the global multiplier. The design multiplies `RateXp`/`RateSp`, but not `RatePartyXp`/`RatePartySp`, to avoid applying G twice to party rewards (rule P5). Please confirm.

## 16. Review dispositions

Each design-review-1 (F) and design-review-2 (R) finding maps to the section that satisfies it. Two review claims were checked against the pinned source and found wrong; the pinned behaviour wins:
- **F35/DR2** "Sweeper never ends the decay task": `ConsumeBody` ends it (§1.3, §5.1).
- **R20** "maintainIngredient is not consumed": consumption is unconditional under the pinned `BlacksmithUseRecipes = True` (§7).

| Finding | Where satisfied |
|---|---|
| F1 / R1 exact rationals, U256 width, one normative test, scaled walk | §2.3 `chance`, §3.2 pseudo-code and width proof, §14.1 D-W |
| F2 / R2 draw order and counts, group formula, `kill()` placement, `roll_below` jitter | §2.2, §2.3, §3.2 |
| F3 / R13 PK single loop, exclusion filter, trigger, [−25, 24] | §3.9 |
| F4 / R15 party model, vote, cursor, order, departure | §1.4, §4.2, §4.4, §4.6 |
| R5 party/solo XP | §4.5 (B16) |
| F5 / R6 / R34 per-operation ledger, causes, fresh vs transfer, merges | §11.2, §11.3 |
| F6 / R10 / R32 replay independence, frozen inputs, crash windows | §10.0, §12.2 frozen table, §14.4 |
| F7 masterwork | §8.2, §2.3 #11 |
| F8 / R3 price source, restock | §6.1, §6.4 |
| F9 / R4 / R43 castle tax 0, double emulation, guards | §6.2, §6.3 |
| F10 integer positions, real types | §1.2–1.11 (`Vec2Fixed`, `EntityId`, `u32` templates, `PartyId(u64)`) |
| F11 / R25 corpse via `NpcAi.corpse_until`, seeded extension, spoiler not stored with a party, monster-only Spoil | §1.3, §5.1 |
| F12 / R8 / R9 manor, auction, mail contracts | §10.3, §10.5, §10.6 |
| F13 / R31 catalogue admission, herb skills enumerated | §1.10 items 5 and 8 |
| F14 / R11 complete wire, extended RejectReason, no holes, all intents and RPCs | §13 |
| F15 B-dispositions | B2 §1.6/§9.4; B5 §1.2; B6 §1.9; B7 §9.2/§9.4; B8 §11.4/§11.5 |
| F16 / R18 trade semantics | §1.5, §9.1, §9.2 |
| F17 / R19 refund price and vectors | §6.6, §14.1 V-S |
| F18 freight same-account; FREIGHT_LIM ignored by pinned | §10.2 |
| F19 / R20 / R42 multisell | §7 |
| F20 / R23 maxDealer and tie-break without timestamps | §3.6 |
| F21 / R12 / R35 two protections, static wire fields | §1.2, §3.7, §1.13 |
| F22 / R16 even split and sweep routing | §4.2 |
| F23 strict command order | §2.1 phase 0 |
| F24 / R41 budget as an assumption, measurement gate | §1.12, Q2 |
| F25 vendor 150 units, conversions | §6.1, conventions |
| F26 / R24 decay details and scanner | §3.8 |
| F27 craft order, limits through stats | §8.1, §8.2, §1.11 invariants |
| F28 / R7 / R28 schema, owners, 000014+ | §11.4 |
| F29 / R26 / R27 version, magic NFSNAP11, carriers | §12.1, §12.3 |
| F30 duplicated ground wire | §1.13, §13.3 (EntitySpawn only) |
| F31 / R30 evidence v2 shapes | §14.3 |
| F32 / R14 strategy distinctions, multiplier chain | §3.3, §3.4 (E21) |
| F33 / R36 citations | corrected throughout: party cap `RequestJoinParty.java:149`; recipe template ids come from data (no invented range); magicLvl vs learn level; Sweeper `:1465-1493` |
| F34 stale phrasing | header and §15 |
| F36 dual projections | §1.8: one home |
| R17 herbs carry ItemInstanceIds | §1.2, §4.3 |
| R21 warehouse augment and merge | §10.1 |
| R22 store gaps | §9.3 |
| R29 digest coverage | §12.4 |
| R33 answer intents | §1.5 (single active requester) |
| R37 AutoLoot / clan-war / champion defaults | §1.10 item 3, §3.1, §3.9 |
| R38 / R39 / R40 2D, rounding, limit stats | conventions, §1.11 |
| R44 progress table accuracy | `inbox/design-progress.md` reports only what this document contains |
