# API Guidelines

## 1. Shape

- Use gRPC (tonic, which tonic-web makes available to browsers) for request/response. One RPC maps to one
  use case. The proto is in `packages/proto/nightfall/v1/`.
- A WebSocket at `/ws` is the real-time world channel (Phase 0 §3.2). Section 3b gives its
  contract.
- Use HTTP (axum) only for `/health`, `/ready` and operator endpoints.

## 2. Idempotency

Each endpoint must be safe to try again. Clients try again after a network failure, a timeout or
`UNAVAILABLE`. A retry must never make two characters, pay two times or move two times.

| Endpoint kind | How it is idempotent |
|---------------|----------------------|
| Reads (`Get*`, `List*`, `Ping`) | Naturally. There is no key. |
| Creates and other mutations | The request must have an `idempotency_key` (a UUID that the client makes). The server stores `(account_id, operation, key, fingerprint, response)` in the same transaction as the write. A retry with the same key and the same fingerprint returns the stored response and writes nothing. The same key with a different fingerprint is `FAILED_PRECONDITION`. Each key has the scope of the calling account and the operation. Thus two accounts (or two RPCs) never collide on a key. |
| Deletes | A delete of an item that is already deleted returns success, not `NOT_FOUND`. |
| State transitions (`EquipItem`, `JoinParty`) | The request has a key, or the definition makes a second application a no-op (equip of an item that is already equipped succeeds). Write the selected option in the RPC comment. |

The fingerprint is the semantic content of the request. It does not include data that the server makes, and
it does not include the caller (the key already has the account scope). Put a version on it (`v1|...`). Thus a change
to the rule for "the same request" does not break stored keys.

The keys are in `idempotency_keys`, and the server keeps them for 24 hours (Phase 9 cleanup job). The stored
`response` is the value that a retry returns. It has sufficient data to make the original reply again exactly (for
`CreateCharacter`, the character id; for `IssuePlayTicket`, the full response with the ticket).
See database-guidelines.md section 2a.

## 3. Errors

`application::AppError` maps to exactly one code. The interface never makes new codes.

| `AppError` | gRPC | HTTP |
|------------|------|------|
| `InvalidArgument` | `INVALID_ARGUMENT` | 400 |
| `NotFound` | `NOT_FOUND` | 404 |
| `AlreadyExists` | `ALREADY_EXISTS` | 409 |
| `Unauthenticated` | `UNAUTHENTICATED` | 401 |
| `PermissionDenied` | `PERMISSION_DENIED` | 403 |
| `IdempotencyConflict` | `FAILED_PRECONDITION` | 409 |
| `Infrastructure` | `INTERNAL` with the message `internal error` | 500 |

The server logs infrastructure errors with `tracing::error!` at the mapping point. These errors never
go to the client. It is safe to show the messages for the other variants.

Validation occurs at the edge before the code calls a port. Thus a bad request never opens a
transaction. The tests assert this (`bad_name_is_invalid_argument_and_writes_nothing`).

## 3a. Authentication

Identity comes from Keycloak (plan decision D1). The server never sees a password. It does a check of
access tokens and nothing more.

**How the caller gets to a use case.**

1. The client sends `authorization: Bearer <access token>` as gRPC metadata.
2. `interface::grpc::AuthLayer` is a tower layer inside the telemetry layer. It runs for each RPC
   that is not in `PUBLIC_METHODS`. It is a layer and not a tonic interceptor, because the token check
   must await (JWKS refresh, the account upsert).
3. The `Authenticate` use case calls the `TokenVerifier` port (`KeycloakVerifier` in
   production). The rules are: RS256 only; `iss`, `aud`, `exp` and `sub` are necessary; `aud` must include
   `OIDC_AUDIENCE`; the leeway is 30 s. The keys come from `OIDC_ISSUER/protocol/openid-connect/certs`.
   The server loads them at boot, and refreshes them on an unknown `kid` a maximum of one time per 60 s.
4. `sub` must be a UUID. It *is* the `AccountId`. `EnsureAccount` upserts the `accounts` row
   (the first login makes it). `last_login_at` changes a maximum of one time each minute, thus most requests write
   nothing.
5. The layer inserts the `AccountId` into the request extensions and records it on the RPC span.
   Handlers read it with `interface::grpc::auth::caller(&req)` and give it to the input of the use case.
   **No use case takes an account id from a request field.** The server ignores the deprecated
   `CreateCharacterRequest.account_id`.

If a step from 2 to 4 fails, the result is `UNAUTHENTICATED` and the handler does not run. If the server cannot get to the JWKS or
the database, the result is `INTERNAL`. Ownership is a use-case rule: an action on a resource of a different account is
`PERMISSION_DENIED` (`GetCharacter`, `IssuePlayTicket`).

**Public RPCs:** `GameService.Ping` only. If a request to a public RPC has a token, the server ignores the token. Each new RPC
has authentication by default. To make an RPC public, add it to `PUBLIC_METHODS` and give a
reason in the review.

**Configuration.** If `OIDC_ISSUER` is not set, the server refuses each RPC that is not public (`DisabledVerifier`).
Thus a server with an incorrect configuration fails closed. `AUTH_DEV_TOKENS=1` accepts unsigned
`test:<account_uuid>` tokens. Use this only for local development. Tests use the same
`TestTokenVerifier` through `common::TestApp` (`app.grpc` has authentication as `ACCOUNT`,
`app.game_as(..)` as any account, `app.anon_grpc()` as no account).

**Play ticket lifecycle** (`SessionService.IssuePlayTicket`, plan Revision 1 items 7-9, 17):

| Step | What occurs |
|------|--------------|
| Issue | The caller must own the character. In one transaction, the server claims the idempotency key with the full response, increases `account_sessions.generation` and stores `play_tickets(ticket_hash = SHA-256(ticket), account, character, generation, expires_at = now + 60 s)`. The ticket is 32 random bytes, returned as base64url. Only its hash is in `play_tickets`. |
| Retry | Same key, same character: the identical response, also after the ticket is consumed or expired. Same key, a different character: `FAILED_PRECONDITION`. Each connection try uses a new key. |
| Present | `Authorization: Bearer <ticket>` on the `/ws` upgrade, never in the URL. Spans record only the path. `span_secrets.rs` asserts that a sentinel ticket is in no span or log. |
| Consume | `ConsumePlayTicket`: one transaction locks the row and marks it as consumed. Malformed, unknown, expired or already consumed: HTTP 401 before the upgrade. If the generation is older than the current generation of the account (the server issued a newer ticket): `Superseded`, HTTP 409. In the two conditions, the ticket is spent. Success gives `(account, character, generation)`. In the zone actor, the generation fences older sockets of the same account (Epic 4). |

## 3b. Real-time channel

Diagrams: [device login and zone admission](../diagrams/login-zone-sequence.html) ·
[session lifecycle and close codes](../diagrams/session-lifecycle.html).

`GET /ws` on the axum listener sends one protobuf message in each **binary** frame:
`nightfall.v1.ClientMessage` up, `nightfall.v1.ServerMessage` down
(`packages/proto/nightfall/v1/world.proto`). `interface::ws` does the handshake and the wire
format. `application::session` is the actor for each socket, and it enforces all the rules below.

**Handshake.** The client presents a new play ticket (section 3a) as
`Authorization: Bearer <ticket>` on the upgrade request, never in the URL. The server refuses all requests that it can
refuse before the upgrade, as an HTTP status, in this order:

| Check | Status |
|-------|--------|
| Not a WebSocket upgrade | 400 / 426 (axum) |
| More than 10 live sessions from the IP of the client (the server does this check before the ticket check, and the ticket is not spent) | 429 |
| No ticket, malformed, unknown, expired, or already used | 401 |
| The server issued a newer ticket for the account (`Superseded`) | 409 |
| The character of the ticket is not there now, or is not a character of the account | 404 / 403 |
| A port fails | 500 |

If the handshake succeeds, the server upgrades the socket and the session joins the zone. It uses `SpawnPlayer` for the first
session of the character. It uses `ReplaceSession` when an older generation controls that same player
entity. The server closes the older socket with 4409 immediately. The zone fences its commands and its late `Despawn`
by generation. Thus the older socket can never move or remove the replacement (plan §8 #8). The first frames that a
session receives are its AOI: its own `EntitySpawn` (with `session_generation`), then each
entity in the 3x3 cells around it.

**Ordered output.** For each tick, a session sends exactly the output of the zone actor for its player.
It sends one `ServerMessage` per frame, in the order of the actor. First come the responses to its intents (`Ack` or
`IntentRejected`, in the order that the actor applied the intents), then AOI despawns, spawns and moves.
The session never changes the order of this output, merges it or adds to it. Its only other frames are the refusals that it
decides itself (below). It sends a refusal when the refused frame arrives. The audit log
records this stream, and replay compares it (plan §8 #6). Each intent gets exactly one `Ack` or one
`IntentRejected` with its `seq`.

**Limits** (`application::session::SessionLimits::default`; transport caps in `interface::ws`):

| Limit | Value | When exceeded |
|-------|-------|---------------|
| Frame size | 4096 bytes | `IntentRejected{INVALID, seq 0}` before decoding; the session stays open. Frames over 64 KiB break the connection. |
| Rate | 30 frames/s per session, burst 30 (token bucket) | `IntentRejected{RATE_LIMITED}` |
| `seq` | strictly increasing per session | close 4400 |
| Idle | 60 s without an inbound frame (pings count; clients without pings send a no-intent `ClientMessage`, and the answer is `INVALID`) | close 4408 |
| Zone queue | 1024 inputs per zone | `IntentRejected{OVERLOADED}`; the session stays open |
| Per-tick budget | 8 intents per session per tick | moved to later ticks, in the same order (a later `Ack.tick`) |
| Outbound queue | 256 frames per session; 100 ms grace for the writer to drain | If the queue is full after the grace: the frame is dropped, `ws_dropped_frames_total` +1, close 4429 |
| Sessions per IP | 10 by default (`SessionLimits::max_sessions_per_ip`; includes upgrades that are in progress) | HTTP 429 before the upgrade |

Other refusals: frames that are not binary, that the server cannot decode, or that have an empty intent are `INVALID`. A
coordinate that is not finite is `OUT_OF_BOUNDS` at the edge. The zone itself answers `OUT_OF_BOUNDS` (outside the
256x256 fixture zone), `TOO_FAR` (more than 64 tiles) and `UNKNOWN_ENTITY`.

**Combat contract (Phase 1 E2.1).** These intents use the same authentication, session identity,
generation fence, seq/rate checks, queue and per-tick budget as movement. The edge rejects a malformed target
UUID. An empty string clears the selection. The zone does a check of actor life,
target existence, AOI and attackability in that order. Players and dead/noncombat NPCs are
not attackable. If the client sends a valid selection again with a new seq, the server Acks it and sends no more events.
Only the owner gets the target selection. It comes after the responses and the AOI output in the ordered stream.

| Kind | New messages / reasons | Contract |
|------|------------------------|----------|
| Intents | `SetTarget`, `Attack`, `StopAttack`, `Respawn` | No actor/stat/damage inputs from the client. Attack turns on repeats on the current target. A repeated Attack never resets a cycle. |
| Events | `AttackResult`, `EntityDied`, `EntityRespawned`, `StatsChanged`, `XpGained`, `LevelUp`, `TargetChanged` | Whole-number HP/MP/damage, u64 XP, ticks of 100 ms. Only the owner gets stats, XP and selection. |
| Reasons | `DEAD_ACTOR`, `NON_ATTACKABLE_TARGET`, `TARGET_NOT_IN_AOI`, `OUT_OF_RANGE`, `PROTECTED`, `NOT_YET_IMPLEMENTED` | An unknown target uses `UNKNOWN_ENTITY`. A malformed UUID uses `INVALID`. A dead target uses `NON_ATTACKABLE_TARGET`. |

Phase 1 E2.2/E2.3 implement Attack and StopAttack (architecture.md §2.4, "Combat"). For Attack,
the target must be selected, alive and attackable, and in the AOI (else `UNKNOWN_ENTITY` with no target,
`NON_ATTACKABLE_TARGET`, `TARGET_NOT_IN_AOI`, `PROTECTED`). A repeat with a new seq Acks and does not start a
second swing. StopAttack Acks and cancels the pending swing. MoveTo, StopMove and a target
change stop the attack. From a dead actor, each intent other than Respawn returns `DEAD_ACTOR`.
Respawn is implemented. A dead player goes back to the safe point with:

- 65% of maximum HP (rounded down, minimum 1)
- zero MP
- a new incarnation
- 6000 ticks of spawn protection.

An accepted
Attack stops the protection early. Respawn emits EntityRespawned and owner StatsChanged. The player does not get back the lost
XP. A living player receives `INVALID`. `OUT_OF_RANGE` is never a
rejection. If a target is out of reach at impact, the swing is cancelled (`AttackCancelled`
`OUT_OF_RANGE`). New events: `AttackStarted` (start/impact/ready ticks) and `AttackCancelled`.
`EntitySpawn` has new combat fields 6–14. `AttackResult.target_incarnation` and
`EntityDied.incarnation` hold the life. Event indices are not on the wire now. There is no new repository
port, thus the §4 adapter tests do not apply.

**Close codes** (only after the upgrade):

| Code | Meaning | Client must |
|------|---------|---------------|
| 1001 | Server shutdown | reconnect with backoff |
| 1011 | The zone refused admission (for example, a saved position outside it), or the zone stopped | report; reconnect with backoff |
| 4400 | `seq` did not increase | repair the client; reconnect |
| 4408 | Idle timeout | reconnect when the player does an action |
| 4409 | Stale generation, or a newer session of the same player entity replaced it | stop; do not reconnect automatically |
| 4423 | The checkpoint lane of the character is quarantined (reason `character_lane_quarantined`). An operator must clear it. | stop; do not reconnect automatically |
| 4429 | The client does not read fast enough (outbound queue full, or a full broadcast buffer behind) | reconnect with a new ticket; the server sends the AOI again |

The writer gives 1 s to send a close frame before it drops the socket. A transport failure
or a client disconnect sends no application close code.

Each reconnect uses a new ticket from `IssuePlayTicket` with a new idempotency key.

**Audit and telemetry.** Each inbound frame (before validation, with its decoded `seq` if there is one)
and each outbound frame goes to the `SessionAudit` port. The key is a `SessionId` for each socket
(plan D5; Story 3.2 supplies the `JetStream` adapter). Metrics: `sessions_active`,
`ws_frames_total{direction}`, `ws_dropped_frames_total`. Each intent is one trace:

1. `ws.frame` (root, in the session) →
2. `zone.apply` (in the zone actor, through the `TraceCarrier` on the zone input) →
3. `ws.deliver` (back in the session when its response goes into the queue).

**Tests** for the channel obey section 4 through real sockets with `tokio-tungstenite`
(`tests/ws_session.rs`, `tests/ws_trace.rs`, `tests/span_secrets.rs`). They include each status and close
code above, each limit, AOI visibility, replacement, the byte-for-byte output order and the audit.
A socket cannot make some cases again deterministically (a client that stops reading, a full zone
queue). These cases are unit tests of the actor (`application/session_tests.rs`).

## 4. Required tests per endpoint

A pull request that adds or changes an endpoint is not complete without all of these tests. Reviewers
examine the list.

**Unit (in the use case module, in-memory ports, no I/O):**

1. Happy path.
2. Each `AppError` variant that the use case can return. Each test must cause its variant.
3. For mutations: the same key replays with no second write and no second event. The same key with a
   different body causes a conflict.
4. A validation failure writes nothing and publishes nothing.

**Integration (`apps/api/tests/<transport>_<endpoint>.rs`, real socket through `common::TestApp`):**

5. Happy path through the wire. Assert each response field.
6. Each error code that the endpoint documents. Assert it by `tonic::Code` or HTTP status.
   Include `UNAUTHENTICATED` without a token. For resources with an owner, include
   `PERMISSION_DENIED` as a different account.
7. For mutations: the retry case through the wire.
8. Read-after-write where applicable (`create` then `get`).

**Adapter (`apps/api/tests/postgres_*.rs`, which run only when `DATABASE_URL` is set):**

9. Each new repository method: round trip, constraint violations map to typed errors, and
   atomicity (a failed step leaves no partial rows).
10. Each method that makes sure of uniqueness or idempotency: a concurrent test
    (`concurrent_retries_with_same_key_create_exactly_one`).

Examples that are in the repo now: `grpc_create_character.rs` (10 cases, with a forged `account_id`),
`grpc_issue_play_ticket.rs`, `postgres_character_repository.rs` and
`postgres_session_repository.rs` (with concurrency).

## 5. Proto conventions

- Use `snake_case` fields. Use `SCREAMING_CASE` enum values with the enum name as prefix. Value 0 is
  `_UNSPECIFIED`, and the server rejects it for required enums.
- Never use a field number again or change its number. Reserve removed numbers.
- On the wire, existing ids are strings that hold UUIDs. On the server, they are typed.
- Phase 4 item messages use exactly 16 canonical UUID bytes for character/entity ids.
- Item instance ids are positive `uint64` values that are safe as a signed bigint. They come from
  durable zone allocator ranges, and they are not UUID identities. Refer to phase-4-zone-design.md §1.1/§11.
- Document each RPC with its idempotency behaviour and the error codes that it returns.
- `buf breaking` runs in CI against `main` (Phase 9 §3).
