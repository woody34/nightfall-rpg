# packages/proto

This package is the contract between the client and the server. All the wire types that the Rust server and the Unreal client send to each other have their definitions here.
Neither side writes wire types by hand.

## Files

| File | Contents |
|------|----------|
| `nightfall/v1/game.proto` | `GameService` (gRPC): `Ping`, `GetCharacter`, `CreateCharacter`, `ListMyCharacters`, and the character types. |
| `nightfall/v1/session.proto` | `SessionService` (gRPC): `IssuePlayTicket`, admission to the real-time channel. |
| `nightfall/v1/world.proto` | The envelopes for the real-time channel (`ClientMessage`, `ServerMessage`), one for each binary WebSocket frame. This is not gRPC. The header gives the resource limits and the tick/time rule. |

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

After you change the contract, `cargo build` makes the Rust code again through `build.rs`. If the proto tools
are installed, `apps/client-unreal/Scripts/gen-proto.sh --generate-only` makes the
committed TurboLink/C++ files again. It does not set up submodules, install libraries or do an Unreal build.
