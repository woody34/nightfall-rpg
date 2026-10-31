# packages/proto

This package is the contract between the client and the server. All the wire types that the Rust server and the Unreal client send to each other have their definitions here.
Neither side writes wire types by hand.

## Files

| File | Contents |
|------|----------|
| `nightfall/v1/game.proto` | `GameService` (gRPC): `Ping`, `GetCharacter`, `CreateCharacter`, `ListMyCharacters`, and the character types. |
| `nightfall/v1/session.proto` | `SessionService` (gRPC): `IssuePlayTicket`, admission to the real-time channel. |
| `nightfall/v1/world.proto` | The envelopes for the real-time channel (`ClientMessage`, `ServerMessage`), one for each binary WebSocket frame. This is not gRPC. The header gives the resource limits and the tick/time rule. |
| `nightfall/v1/items.proto` | Phase 4 item identities, intents, inventory events and the item catalogue types. |
| `nightfall/v1/economy.proto` | Phase 5 economy intents, events, receipts and the nine read responses. `world.proto` and `game.proto` import it. |

## Conventions

For all the rules, refer to [docs/engineering/api-guidelines.md](../../docs/engineering/api-guidelines.md) §5.

- Fields are `snake_case`. Enum values are `SCREAMING_CASE`, with the enum name as a prefix. Value 0 is
  `_UNSPECIFIED`.
- Do not use a field number again and do not change the number of a field. Reserve the numbers of removed fields. To deprecate a field, use
  `[deprecated = true]` and keep the number.
- On the wire, IDs are UUID strings.
- Each RPC gives its idempotency behavior and the gRPC error codes that it returns.
- The identity of the caller comes from the verified bearer token. It never comes from a request field.

## Generating code

- **Server:** `apps/api/build.rs` compiles the files with `protox` (only Rust, no system
  `protoc`) and `tonic-prost-build`. Add new files to the list in `build.rs`.
- **Client:** Unreal uses the TurboLink `protoc-gen-turbolink`. Refer to
  [apps/client-unreal/Scripts/gen-proto.sh](../../apps/client-unreal/Scripts/gen-proto.sh).

## Phase 1 E2.1 combat contract

| Kind | Additions (wire numbers) | Semantics |
|------|-------------------------|-----------|
| Client intents | `SetTarget` 12, `Attack` 13, `StopAttack` 14, `Respawn` 15 | The actor comes from the session. An empty target removes the target. Repeats with a fresh seq are idempotent. |
| World events | `AttackResult` 4, `EntityDied` 5, `EntityRespawned` 6, `StatsChanged` 7, `XpGained` 8, `LevelUp` 9, `TargetChanged` 10 | Integer HP/MP/damage, u64 XP, 100 ms ticks, UUID identities. Only the owner gets stats, XP and selection. |
| Reject reasons | `DEAD_ACTOR` 7, `NON_ATTACKABLE_TARGET` 8, `TARGET_NOT_IN_AOI` 9, `OUT_OF_RANGE` 10, `PROTECTED` 11, `NOT_YET_IMPLEMENTED` 12 | A dead target is non-attackable. Range and protection are reserved for combat execution. |

E2.1 implemented target selection. E2.2/E2.3 implement Attack and StopAttack, and add these items:

| Kind | Additions (wire numbers) | Semantics |
|------|-------------------------|-----------|
| World events | `AttackStarted` 11, `AttackCancelled` 12 (`SwingCancelReason`) | The swing start, impact and ready ticks for animation. A cancellation has no impact and no roll. |
| `EntitySpawn` | `combatant` 6 … `pending_swing` 14 | The public combat state on AOI entry: template, life incarnation, dead, attackable, HP/max HP, level, and a swing in flight. |
| Facts | `AttackResult.target_incarnation` 7, `EntityDied.incarnation` 4 | The life that a fact refers to. |

E2.4/E2.6 implement Respawn and XP, and add `EntityRespawned.incarnation` 5 and
`StatsChanged.xp` 7 (the cumulative XP, only for the owner, which includes the death loss). If a living
actor sends a `Respawn`, the result is `INVALID` ("the actor is alive and cannot respawn"). The server does not send `NOT_YET_IMPLEMENTED` now.
Fixture NPCs from the zone `[[npcs]]` stay noncombat. Spawn-slot monsters are combatants.

## Phase 5 E2.1 economy contract

The wire delta is in `economy.proto` and in appended lines of `world.proto`, `items.proto` and
`game.proto`. The source of the numbers is `docs/plans/phase-5-zone-design.md` §13 (plan §2.1).
The additions are additive only. `buf breaking` against `main` 5abae32 reports no change.

| Kind | Wire numbers | Semantics |
|------|--------------|-----------|
| Client intents | `ClientMessage` 29..69 (41 intents): `PickUpItem` 29, `VendorBuy` 30, `VendorSell` 31, `VendorRefund` 32, `MultisellChoose` 33, `RegisterRecipe` 34, `DeleteRecipe` 35, `CraftItem` 36, `TradeRequest` 37, `TradeAnswer` 38, `TradeAddItem` 39, `TradeConfirm` 40, `TradeCancel` 41, `SetPrivateStoreSell` 42, `SetPrivateStoreBuy` 43, `SetPrivateManufacture` 44, `ClosePrivateStore` 45, `PrivateStoreBuy` 46, `PrivateStoreSell` 47, `PrivateManufactureOrder` 48, `PartyInvite` 49, `PartyAnswer` 50, `PartyLeave` 51, `PartyKick` 52, `PartyChangeLeader` 53, `PartyLootChangeRequest` 54, `PartyLootChangeAnswer` 55, `WarehouseDeposit` 56, `WarehouseWithdraw` 57, `FreightSend` 58, `FreightWithdraw` 59, `MailSend` 60, `MailTakeAttachments` 61, `MailReject` 62, `MailCancel` 63, `MailDelete` 64, `ManorBuySeed` 65, `ManorSellCrop` 66, `ItemAuctionBid` 67, `ItemAuctionCancelBid` 68, `MailMarkRead` 69 | Each mutating intent carries `idempotency_key` (a UUID string). The same key with the same body replays the frozen receipt. The same key with a different body is rejected before mutation. A rejection consumes no key. Sow and Harvest use `UseItem` 21. |
| World events | `WorldEvent` 34..45 (12 events): `EconomyResult` 34, `CorpseStateChanged` 35, `PartyStateChanged` 36, `PartyLootVote` 37, `PartyInviteReceived` 38, `TradeRequestReceived` 39, `TradeSessionChanged` 40, `PrivateStoreChanged` 41, `MailNotification` 42, `RecipeBookSync` 43, `ItemAuctionStatusChanged` 44, `LootDistributed` 45 | `EconomyResult` is the owner-only frozen receipt. Its tags 1..10 are the D53 receipt fields; `outcome` is a oneof with tags 20..31. The API gateway sends `MailNotification`; it is never zone output. |
| Reject reasons | `RejectReason` 38..88 (51 values, from `GROUND_ITEM_NOT_FOUND` 38 to `NO_BID_TO_CANCEL` 88) | The existing enum is extended. Reused existing reasons: `RATE_LIMITED` 5, `DEAD_ACTOR` 7, `OUT_OF_RANGE` 10, `INSUFFICIENT_ITEMS` 28, `INVENTORY_FULL` 29, `ITEM_CAPABILITY_DEFERRED` 36. |
| Entities | `ENTITY_KIND_GROUND_ITEM` 3; `EntitySpawn.ground_item` 30 (`GroundItemDetail`) | A ground item is an entity that carries one item instance (B5). The detail is present only for that kind. A ground item despawns through the existing `EntityDespawn`. |
| Item operations | `ItemOperation` 14..42 (29 receipt scopes) | The order is the design §11.3 order. |
| Item locations | `ItemLocation` 3..8: `WAREHOUSE`, `FREIGHT`, `MAIL`, `AUCTION_BID`, `REFUND`, `DELIVERY` | Container views reuse `OwnedItem`. These values are V7 only. |
| gRPC reads | 9 RPCs after `GrantItem`: `ListBuylist`, `GetMultisellList`, `GetRecipeBook`, `GetWarehouse`, `GetFreight`, `ListMail`, `ReadMail`, `ListItemAuctions`, `GetManorData` | All are reads with no key. None writes a row. `ReadMail` does not set the read flag; `MailMarkRead` does. |

`StatsChanged` has no new field. The replay codec tags (`PbCommandKind` 32..76, `PbPlayerLoad` 11,
`PbOutput` 42..55) and the `PHASE5_ENABLED` gate are in story E2.2, not in this file.

Until the economy runtime (E3/E4) is installed, the API fails closed. The 41 intents get
`IntentRejected{NOT_YET_IMPLEMENTED}`. The nine reads return `UNAVAILABLE` after authentication.
`apps/api/tests/phase5_wire.rs` freezes every number in this table. `apps/api/tests/phase5_rollout.rs`
proves the fail-closed paths over real sockets.

After you change the contract, `cargo build` makes the Rust code again through `build.rs`. If the proto tools
are installed, `apps/client-unreal/Scripts/gen-proto.sh --generate-only` makes the
committed TurboLink/C++ files again. It does not set up submodules, install libraries or do an Unreal build.
