# API Guidelines

## 1. Shape

- gRPC (tonic, exposed to browsers via tonic-web) for request/response. One RPC maps to one
  use case. Proto lives in `packages/proto/nightfall/v1/`.
- A WebSocket at `/ws` for the real-time world channel (Phase 0 §3.2). Its contract is
  section 3b.
- HTTP (axum) only for `/health`, `/ready`, and operator endpoints.

## 2. Idempotency

Every endpoint must be safe to retry. Clients retry on network failure, timeouts, and
`UNAVAILABLE`; they must never be able to create two characters, pay twice, or move twice by
retrying.

| Endpoint kind | How it is idempotent |
|---------------|----------------------|
| Reads (`Get*`, `List*`, `Ping`) | Naturally. No key. |
| Creates and other mutations | Carry `idempotency_key` (client-generated UUID, required). The server stores `(account_id, operation, key, fingerprint, response)` in the same transaction as the write. A retry with the same key and the same fingerprint returns the stored response and writes nothing. The same key with a different fingerprint is `FAILED_PRECONDITION`. Keys are scoped to the calling account and the operation, so two accounts (or two RPCs) never collide on a key. |
| Deletes | Deleting something already deleted returns success, not `NOT_FOUND`. |
| State transitions (`EquipItem`, `JoinParty`) | Either carry a key, or be defined so that applying them twice is a no-op (equip an already-equipped item succeeds). State the choice in the RPC comment. |

The fingerprint is the request's semantic content minus anything generated server-side and
minus the caller (the key is already scoped to the account). Version it (`v1|...`) so a change
in what counts as "the same request" does not break stored keys.

Keys live in `idempotency_keys` and are retained for 24 hours (Phase 9 cleanup job). The stored
`response` is what a retry returns: enough to rebuild the original reply exactly (for
`CreateCharacter` the character id; for `IssuePlayTicket` the whole response, ticket included).
See database-guidelines.md section 2a.

## 3. Errors

`application::AppError` maps to exactly one code. The interface never invents codes.

| `AppError` | gRPC | HTTP |
|------------|------|------|
| `InvalidArgument` | `INVALID_ARGUMENT` | 400 |
| `NotFound` | `NOT_FOUND` | 404 |
| `AlreadyExists` | `ALREADY_EXISTS` | 409 |
| `Unauthenticated` | `UNAUTHENTICATED` | 401 |
| `PermissionDenied` | `PERMISSION_DENIED` | 403 |
| `IdempotencyConflict` | `FAILED_PRECONDITION` | 409 |
| `Infrastructure` | `INTERNAL` with the message `internal error` | 500 |

Infrastructure errors are logged with `tracing::error!` at the mapping point and never
reach the client. Messages for the other variants are safe to show.

Validation happens at the edge before any port is called, so a bad request never opens a
transaction. The tests assert this (`bad_name_is_invalid_argument_and_writes_nothing`).

## 3a. Authentication

Identity comes from Keycloak (plan decision D1). The server never sees a password; it verifies
access tokens and nothing else.

**How the caller reaches a use case.**

1. The client sends `authorization: Bearer <access token>` as gRPC metadata.
2. `interface::grpc::AuthLayer`, a tower layer inside the telemetry layer, runs for every RPC
   not in `PUBLIC_METHODS`. It is a layer, not a tonic interceptor, because verification
   awaits (JWKS refresh, the account upsert).
3. The `Authenticate` use case calls the `TokenVerifier` port (`KeycloakVerifier` in
   production): RS256 only; `iss`, `aud`, `exp`, `sub` required; `aud` must contain
   `OIDC_AUDIENCE`; 30 s leeway. Keys come from `OIDC_ISSUER/protocol/openid-connect/certs`,
   loaded at boot and refreshed on an unknown `kid` at most once per 60 s.
4. `sub` must be a UUID; it *is* the `AccountId`. `EnsureAccount` upserts the `accounts` row
   (first login creates it; `last_login_at` moves at most once a minute, so most requests write
   nothing).
5. The layer inserts the `AccountId` into the request extensions and records it on the RPC span.
   Handlers read it with `interface::grpc::auth::caller(&req)` and pass it into the use case
   input. **No use case takes an account id from a request field**; the deprecated
   `CreateCharacterRequest.account_id` is ignored.

Any failure in steps 2-4 is `UNAUTHENTICATED` and the handler never runs; an unreachable JWKS or
database is `INTERNAL`. Ownership is a use-case rule: acting on another account's resource is
`PERMISSION_DENIED` (`GetCharacter`, `IssuePlayTicket`).

**Public RPCs:** `GameService.Ping` only. A token sent to a public RPC is ignored. Every new RPC
is authenticated by default; making one public means adding it to `PUBLIC_METHODS` with a
reason in review.

**Configuration.** `OIDC_ISSUER` unset: every non-public RPC is refused (`DisabledVerifier`),
so a misconfigured server fails closed. `AUTH_DEV_TOKENS=1` accepts unsigned
`test:<account_uuid>` tokens instead; local development only. Tests use the same
`TestTokenVerifier` through `common::TestApp` (`app.grpc` is authenticated as `ACCOUNT`,
`app.game_as(..)` as anyone, `app.anon_grpc()` as nobody).

**Play ticket lifecycle** (`SessionService.IssuePlayTicket`, plan Revision 1 items 7-9, 17):

| Step | What happens |
|------|--------------|
| Issue | Caller must own the character. One transaction: claim the idempotency key with the full response, bump `account_sessions.generation`, store `play_tickets(ticket_hash = SHA-256(ticket), account, character, generation, expires_at = now + 60 s)`. The ticket is 32 random bytes, returned base64url; only its hash is in `play_tickets`. |
| Retry | Same key, same character: the identical response, even after the ticket was consumed or expired. Same key, other character: `FAILED_PRECONDITION`. Every connection attempt uses a new key. |
| Present | `Authorization: Bearer <ticket>` on the `/ws` upgrade, never in the URL. Spans record the path only; `span_secrets.rs` asserts a sentinel ticket appears in no span or log. |
| Consume | `ConsumePlayTicket`: one transaction locks the row and marks it consumed. Malformed, unknown, expired or already consumed: HTTP 401 before the upgrade. Generation older than the account's current one (a newer ticket was issued): `Superseded`, HTTP 409; the ticket is spent either way. Success yields `(account, character, generation)`; the generation fences older sockets of the same account in the zone actor (Epic 4). |

## 3b. Real-time channel

Diagrams: [device login and zone admission](../diagrams/login-zone-sequence.html) ·
[session lifecycle and close codes](../diagrams/session-lifecycle.html).

`GET /ws` on the axum listener carries one protobuf message per **binary** frame:
`nightfall.v1.ClientMessage` up, `nightfall.v1.ServerMessage` down
(`packages/proto/nightfall/v1/world.proto`). `interface::ws` does the handshake and the wire
format; `application::session` is the per-socket actor that enforces everything below.

**Handshake.** The client presents a fresh play ticket (section 3a) as
`Authorization: Bearer <ticket>` on the upgrade request; never in the URL. Everything that can
be refused is refused before the upgrade, as an HTTP status, in this order:

| Check | Status |
|-------|--------|
| Not a WebSocket upgrade | 400 / 426 (axum) |
| More than 10 live sessions from the client's IP (checked before the ticket, which is not spent) | 429 |
| No ticket, malformed, unknown, expired, or already used | 401 |
| A newer ticket was issued for the account (`Superseded`) | 409 |
| The ticket's character no longer exists, or is not the account's | 404 / 403 |
| Any port failing | 500 |

On success the socket is upgraded and the session joins the zone: `SpawnPlayer` for the first
session of the character, `ReplaceSession` when an older generation owns that same player
entity. The older socket is closed with 4409 at once; the zone fences its commands and its late `Despawn`
by generation, so it can never move or remove the replacement (plan §8 #8). The first frames a
session receives are its AOI: its own `EntitySpawn` (with `session_generation`), then every
entity in the 3x3 cells around it.

**Ordered output.** Per tick, a session sends exactly the zone actor's output for its player,
one `ServerMessage` per frame, in the actor's order: responses to its intents (`Ack` or
`IntentRejected`, in the order the intents were applied), then AOI despawns, spawns and moves.
The session never reorders, merges or adds to it; its only other frames are the refusals it
decides itself (below), sent when the refused frame arrives. This stream is what the audit log
records and what replay compares (plan §8 #6). Every intent gets exactly one `Ack` or one
`IntentRejected` with its `seq`.

**Limits** (`application::session::SessionLimits::default`; transport caps in `interface::ws`):

| Limit | Value | When exceeded |
|-------|-------|---------------|
| Frame size | 4096 bytes | `IntentRejected{INVALID, seq 0}` before decoding; session stays open. Frames over 64 KiB break the connection. |
| Rate | 30 frames/s per session, burst 30 (token bucket) | `IntentRejected{RATE_LIMITED}` |
| `seq` | strictly increasing per session | close 4400 |
| Idle | 60 s without any inbound frame (pings count) | close 4408 |
| Zone queue | 1024 inputs per zone | `IntentRejected{OVERLOADED}`; session stays open |
| Per-tick budget | 8 intents per session per tick | deferred in order to later ticks (a later `Ack.tick`) |
| Outbound queue | 256 frames per session; 100 ms grace for the writer to drain | If still full after grace: frame dropped, `ws_dropped_frames_total` +1, close 4429 |
| Sessions per IP | 10 by default (`SessionLimits::max_sessions_per_ip`; includes upgrades in progress) | HTTP 429 before the upgrade |

Other refusals: non-binary, undecodable, or empty-intent frames are `INVALID`; a non-finite
coordinate is `OUT_OF_BOUNDS` at the edge; the zone answers `OUT_OF_BOUNDS` (outside the
256x256 fixture zone), `TOO_FAR` (more than 64 tiles) and `UNKNOWN_ENTITY` itself.

**Combat contract (Phase 1 E2.1).** These intents use the same authentication, session identity,
generation fence, seq/rate checks, queue and per-tick budget as movement. A malformed target
UUID is rejected at the edge; an empty string clears selection. The zone checks actor life,
target existence, AOI and attackability in that order. Players and dead/noncombat NPCs are
non-attackable. Repeating a valid selection with a fresh seq Acks without another event.
Target selection is owner-only and follows responses and AOI output in the ordered stream.

| Kind | New messages / reasons | Contract |
|------|------------------------|----------|
| Intents | `SetTarget`, `Attack`, `StopAttack`, `Respawn` | No client actor/stat/damage inputs. Attack enables repeats on the current target; repeated enables never reset a cycle. |
| Events | `AttackResult`, `EntityDied`, `EntityRespawned`, `StatsChanged`, `XpGained`, `LevelUp`, `TargetChanged` | Whole HP/MP/damage, u64 XP, ticks of 100 ms; stats, XP and selection are owner-only. |
| Reasons | `DEAD_ACTOR`, `NON_ATTACKABLE_TARGET`, `TARGET_NOT_IN_AOI`, `OUT_OF_RANGE`, `PROTECTED`, `NOT_YET_IMPLEMENTED` | Unknown target uses `UNKNOWN_ENTITY`; malformed UUID uses `INVALID`; dead target uses `NON_ATTACKABLE_TARGET`. |

Attack/stop/respawn are E2.1 stubs: they reach the zone's applied log and return exactly one
`NOT_YET_IMPLEMENTED` rejection, with no combat state change. Selection/attack/stop by a dead
actor return `DEAD_ACTOR`. Range/protection are mapped contract values for E2.3/E2.4, not
active checks yet; selection alone neither attacks nor checks melee range. Full cycle/state,
life incarnation and event-index publication follows in E2.2. No repository port is added;
§4 adapter tests are therefore inapplicable to this story.

**Close codes** (only after the upgrade):

| Code | Meaning | Client should |
|------|---------|---------------|
| 1001 | Server shutting down | reconnect with backoff |
| 1011 | Admission refused by the zone (for example a saved position outside it) or zone stopped | report; reconnect with backoff |
| 4400 | `seq` did not increase | fix the client; reconnect |
| 4408 | Idle timeout | reconnect when the player acts |
| 4409 | Stale generation or replaced by a newer session of the same player entity | stop; do not reconnect automatically |
| 4429 | Not reading fast enough (outbound queue full, or a whole broadcast buffer behind) | reconnect with a fresh ticket; the AOI is resent |

The writer allows 1 s to send a close frame before dropping the socket. A transport failure
or client disconnect sends no application close code.

Every reconnect uses a new ticket from `IssuePlayTicket` with a new idempotency key.

**Audit and telemetry.** Every inbound frame (before validation, with its decoded `seq` if any)
and every outbound frame goes to the `SessionAudit` port, keyed by a per-socket `SessionId`
(plan D5; Story 3.2 supplies the `JetStream` adapter). Metrics: `sessions_active`,
`ws_frames_total{direction}`, `ws_dropped_frames_total`. Each intent is one trace:
`ws.frame` (root, in the session) → `zone.apply` (in the zone actor, via the `TraceCarrier` on
the zone input) → `ws.deliver` (back in the session when its response is queued).

**Tests** for the channel follow section 4 through real sockets with `tokio-tungstenite`
(`tests/ws_session.rs`, `tests/ws_trace.rs`, `tests/span_secrets.rs`): every status and close
code above, each limit, AOI visibility, replacement, byte-for-byte output order, and the audit.
Cases a socket cannot reproduce deterministically (a client that stops reading, a full zone
queue) are unit tests of the actor (`application/session_tests.rs`).

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
6. Each error code the endpoint documents, asserted by `tonic::Code` or HTTP status,
   including `UNAUTHENTICATED` without a token and, for resources with an owner,
   `PERMISSION_DENIED` as another account.
7. For mutations: the retry case through the wire.
8. Read-after-write where applicable (`create` then `get`).

**Adapter (`apps/api/tests/postgres_*.rs`, gated on `DATABASE_URL`):**

9. Any new repository method: round trip, constraint violations map to typed errors, and
   atomicity (a failed step leaves no partial rows).
10. Any method with a uniqueness or idempotency guarantee: a concurrent test
    (`concurrent_retries_with_same_key_create_exactly_one`).

Existing examples: `grpc_create_character.rs` (10 cases, including a forged `account_id`),
`grpc_issue_play_ticket.rs`, `postgres_character_repository.rs` and
`postgres_session_repository.rs` (including concurrency).

## 5. Proto conventions

- `snake_case` fields, `SCREAMING_CASE` enum values with the enum name as prefix, value 0 is
  `_UNSPECIFIED` and is rejected by the server for required enums.
- Never reuse or renumber a field. Reserve removed numbers.
- Ids are strings holding UUIDs on the wire; typed on the server.
- Document every RPC with its idempotency behaviour and the error codes it returns.
- `buf breaking` runs in CI against `main` (Phase 9 §3).
