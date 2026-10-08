# packages/proto

The client/server contract. Everything the Rust server and the Unreal client exchange is
defined here; neither side hand-writes wire types.

## Files

| File | Contents |
|------|----------|
| `nightfall/v1/game.proto` | `GameService` (gRPC): `Ping`, `GetCharacter`, `CreateCharacter`, `ListMyCharacters`, plus character types. |
| `nightfall/v1/session.proto` | `SessionService` (gRPC): `IssuePlayTicket`, admission to the real-time channel. |
| `nightfall/v1/world.proto` | Real-time channel envelopes (`ClientMessage`, `ServerMessage`), one per binary WebSocket frame. Not gRPC. Header lists resource limits and the tick/time rule. |

## Conventions

Full rules: [docs/engineering/api-guidelines.md](../../docs/engineering/api-guidelines.md) §5.

- `snake_case` fields; enum values `SCREAMING_CASE` prefixed with the enum name; value 0 is
  `_UNSPECIFIED`.
- Never reuse or renumber a field; reserve removed numbers. Deprecate with
  `[deprecated = true]` and keep the number.
- Ids are UUID strings on the wire.
- Every RPC documents its idempotency behaviour and the gRPC error codes it returns.
- Caller identity comes from the verified bearer token, never from a request field.

## Generating code

- **Server:** `apps/api/build.rs` compiles the files with `protox` (pure Rust, no system
  `protoc`) and `tonic-prost-build`. Add new files to the list in `build.rs`.
- **Client:** Unreal uses TurboLink's `protoc-gen-turbolink`; see
  [apps/client-unreal/Scripts/gen-proto.sh](../../apps/client-unreal/Scripts/gen-proto.sh).

## Phase 1 E2.1 combat contract

| Kind | Additions (wire numbers) | Semantics |
|------|-------------------------|-----------|
| Client intents | `SetTarget` 12, `Attack` 13, `StopAttack` 14, `Respawn` 15 | Actor comes from the session; empty target clears. Fresh-seq repeats are idempotent. |
| World events | `AttackResult` 4, `EntityDied` 5, `EntityRespawned` 6, `StatsChanged` 7, `XpGained` 8, `LevelUp` 9, `TargetChanged` 10 | Integer HP/MP/damage, u64 XP, 100 ms ticks, UUID identities; stats/XP/selection are owner-only. |
| Reject reasons | `DEAD_ACTOR` 7, `NON_ATTACKABLE_TARGET` 8, `TARGET_NOT_IN_AOI` 9, `OUT_OF_RANGE` 10, `PROTECTED` 11, `NOT_YET_IMPLEMENTED` 12 | Dead target is non-attackable; range/protection are reserved for combat execution. |

E2.1 implements target selection and its validation. Attack/stop/respawn are decoded, fenced,
budgeted and durably recorded, then receive a `NotYetImplemented` disposition (one
`IntentRejected`, never an Ack followed by a rejection). Dead actors receive `DEAD_ACTOR`
for selection/attack/stop; respawn remains a stub. Combat calculations, cycle/state expansion,
incarnations/event indices and death/respawn transitions belong to E2.2–E2.4.
Existing NPC spawns are noncombat fixtures until explicitly opted into combat state.

After contract changes, `cargo build` regenerates Rust through `build.rs`. With the proto tools
already installed, `apps/client-unreal/Scripts/gen-proto.sh --generate-only` regenerates the
committed TurboLink/C++ files without submodule setup, library installation or an Unreal build.
