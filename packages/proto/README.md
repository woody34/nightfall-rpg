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
