# API Guidelines

## 1. Shape

- gRPC (tonic, exposed to browsers via tonic-web) for request/response. One RPC maps to one
  use case. Proto lives in `packages/proto/nightfall/v1/`.
- A WebSocket at `/ws` for the real-time world channel (Phase 0 §3.2). Not covered here.
- HTTP (axum) only for `/health`, `/ready`, and operator endpoints.

## 2. Idempotency

Every endpoint must be safe to retry. Clients retry on network failure, timeouts, and
`UNAVAILABLE`; they must never be able to create two characters, pay twice, or move twice by
retrying.

| Endpoint kind | How it is idempotent |
|---------------|----------------------|
| Reads (`Get*`, `List*`, `Ping`) | Naturally. No key. |
| Creates and other mutations | Carry `idempotency_key` (client-generated UUID, required). The server stores `(key, fingerprint, result)` in the same transaction as the write. A retry with the same key and the same fingerprint returns the stored result and writes nothing. The same key with a different fingerprint is `FAILED_PRECONDITION`. |
| Deletes | Deleting something already deleted returns success, not `NOT_FOUND`. |
| State transitions (`EquipItem`, `JoinParty`) | Either carry a key, or be defined so that applying them twice is a no-op (equip an already-equipped item succeeds). State the choice in the RPC comment. |

The fingerprint is the request's semantic content minus anything generated server-side.
Version it (`v1|...`) so a change in what counts as "the same request" does not break stored keys.

Keys live in `idempotency_keys` and are retained for 24 hours (Phase 9 cleanup job).

## 3. Errors

`application::AppError` maps to exactly one code. The interface never invents codes.

| `AppError` | gRPC | HTTP |
|------------|------|------|
| `InvalidArgument` | `INVALID_ARGUMENT` | 400 |
| `NotFound` | `NOT_FOUND` | 404 |
| `AlreadyExists` | `ALREADY_EXISTS` | 409 |
| `IdempotencyConflict` | `FAILED_PRECONDITION` | 409 |
| `Infrastructure` | `INTERNAL` with the message `internal error` | 500 |

Infrastructure errors are logged with `tracing::error!` at the mapping point and never
reach the client. Messages for the other variants are safe to show.

Validation happens at the edge before any port is called, so a bad request never opens a
transaction. The tests assert this (`bad_name_is_invalid_argument_and_writes_nothing`).

## 4. Required tests per endpoint

A pull request that adds or changes an endpoint is incomplete without all of these. Reviewers
check the list.

**Unit (in the use case module, in-memory ports, no I/O):**

1. Happy path.
2. Every `AppError` variant the use case can return, each triggered.
3. For mutations: same key replays with no second write and no second event; same key with a
   different body conflicts.
4. Validation failure writes nothing and publishes nothing.

**Integration (`apps/api/tests/<transport>_<endpoint>.rs`, real socket via `common::TestApp`):**

5. Happy path through the wire, asserting every response field.
6. Each error code the endpoint documents, asserted by `tonic::Code` or HTTP status.
7. For mutations: the retry case through the wire.
8. Read-after-write where applicable (`create` then `get`).

**Adapter (`apps/api/tests/postgres_*.rs`, gated on `DATABASE_URL`):**

9. Any new repository method: round trip, constraint violations map to typed errors, and
   atomicity (a failed step leaves no partial rows).
10. Any method with a uniqueness or idempotency guarantee: a concurrent test
    (`concurrent_retries_with_same_key_create_exactly_one`).

Existing examples: `grpc_create_character.rs` (7 cases), `postgres_character_repository.rs`
(5 cases including concurrency).

## 5. Proto conventions

- `snake_case` fields, `SCREAMING_CASE` enum values with the enum name as prefix, value 0 is
  `_UNSPECIFIED` and is rejected by the server for required enums.
- Never reuse or renumber a field. Reserve removed numbers.
- Ids are strings holding UUIDs on the wire; typed on the server.
- Document every RPC with its idempotency behaviour and the error codes it returns.
- `buf breaking` runs in CI against `main` (Phase 9 §3).
