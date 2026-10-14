# Phase 0b Plan: Connected Vertical Slice

**Status:** COMPLETE 2026-10-08 (Story 6.4 interpolation polish is deferred. It is not necessary, because the 10 Hz movement was smooth in play tests.)
**Planning model:** Claude Fable 5.1 at high effort made this plan. Each story gives its recommended model and
effort.

## 1. Goal

A player logs in to the Unreal client through Keycloak and goes into a zone. The player sees a different entity move,
and clicks to move with a server round trip. The system logs each frame between client and server to
NATS JetStream. Thus you can replay each session on a headless server, and the output is byte-identical.
Telemetry goes to Grafana from the first commit.

## 2. Decisions (supersede the matching parts of docs/planning/00-foundations.md)

| # | Decision | Choice | Rejected | Why |
|---|----------|--------|----------|-----|
| D1 | Identity provider | **Keycloak**, self-hosted in Compose, realm configuration as code, device authorization grant for the client | Zitadel, Auth0/Clerk, Epic Online Services | It is the most mature OIDC server. The device flow is applicable to a game client (no embedded browser; CEF does not operate correctly on Linux/Wayland). Later, it can federate Epic/Steam/Google. The server never sees a password. |
| D2 | ORM | **SeaORM** on the sqlx pool that we have now; `sea-orm-migration` for migrations; CI makes the entities | Diesel, raw sqlx | It is async, it uses the same pool, and its transactions have the same atomicity rules. The repository ports and their tests do not change. |
| D3 | Client request/response | **gRPC through TurboLink** in Unreal, tonic on the server (tonic-web is not necessary) | HTTP/JSON | There is one protocol in all locations. TurboLink includes protoc, thus its generated classes also replace the WebSocket codec that we wrote manually. Cost: the first builds on Linux are difficult (see R1). |
| D4 | Replay scope | **Server-side**: log the inbound and outbound frames for each session; the headless replay makes sure that the outbound bytes are identical | Client-side frame recording also | This is sufficient to reproduce all server bugs. Client recording is useful only after client prediction exists. |
| D5 | Session event log | **NATS JetStream**, one stream `NF_SESSIONS`, subjects `nightfall.session.<id>.in` / `.out`, retention 7 days | Postgres table, both | It runs now. It is append-only and lets you replay by sequence. It keeps the write load off the game DB. Archival to object storage is a later story. |
| D6 | Telemetry backend | **Grafana LGTM** all-in-one in Compose, OpenTelemetry OTLP from the server | Datadog, Honeycomb | It is free, local, and OTel-native. Later, you can forward to a SaaS if you change one endpoint. |
| D7 | Determinism | Fixed 100 ms tick; positions as `i32` fixed-point (1/1000 tile); `ChaCha12` RNG with a seed for each zone and session; each command has a stamp with the tick on which it is applied | f32 positions | If not, a "Perfect replay" is not possible. Floats drift between builds and CPUs. |

## 3. Architecture additions

```
                 Keycloak (OIDC) ──── JWT ────┐
                                              ▼
 UE client ──gRPC (TurboLink)──▶ tonic: AuthService / CharacterService / SessionService
            ──WS /ws?ticket────▶ axum: session actor ──commands──▶ zone actor (deterministic tick)
                                      │   ▲                          │ events
                                      ▼   │                          ▼
                              JetStream NF_SESSIONS         NATS nightfall.<aggregate>.<event>
                              (.in / .out per session)      + outbox relay from Postgres
                                      │
                                      ▼
                              nightfall-replay (headless zone actor, asserts .out == recorded)

 tracing + OTel ──OTLP──▶ Grafana LGTM (Tempo traces, Prometheus metrics, Loki logs)
```

The layers do not change:
- `domain` (zone rules, movement math)
- `application` (use cases: IssuePlayTicket, JoinZone, MoveTo)
- `infrastructure` (Keycloak JWKS, SeaORM repos, JetStream log, OTel)
- `interface` (gRPC services, `/ws` handler).

## 4. Epics, stories, tasks

Effort levels: **M** mechanical, **D** much design work. Model: **S** Sonnet medium effort,
**O** Opus high effort. Each story that adds an endpoint has the test matrix from
`docs/engineering/api-guidelines.md` §4.

> [!NOTE]
> The `[client-visible]` tag identifies a contract and state behavior that the client can see. The simulation traceability checker examines completed tagged stories against the scenario coverage (`.nfs`). Parts that are only for rendering keep their automation and cook validation. But these stories keep their tags, and the checker reports the cases that have no coverage. New plans must add tags independently before you write scenarios.

### Epic 0: Connected-client contract

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 0.1 Contract first [client-visible] | Session/ticket RPC, list of characters, intent rejection, applied-tick Ack and StopMove contract (§8 #11) | S | The contract and the generated client/server bindings agree |

### Epic 1: Identity

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 1.1 Keycloak in Compose, realm as code | Add the `keycloak` service (Postgres-backed, dev mode); `infra/keycloak/realm-nightfall.json` with a public client `nightfall-client` (device flow on, PKCE) and a test user; import at boot; write the documentation in README | S | `docker compose up` makes a realm; the device flow operates with `curl` |
| ✅ 1.2 JWT validation in the server | `infrastructure/auth/keycloak.rs`: JWKS get + cache + rotation; `application/ports::TokenVerifier`; a tonic interceptor that puts `AccountId` in the request extensions; configuration `OIDC_ISSUER`, `OIDC_AUDIENCE` | S | Unit tests with a generated RSA key; an integration test with a signed test token; the server rejects expired/wrong-aud/wrong-iss tokens |
| ✅ 1.3 Account on first login [client-visible] | `accounts` table (id = IdP `sub`, created_at, last_login); `EnsureAccount` use case, idempotent because of its construction | S | Two logins make one row |
| ✅ 1.4 Play ticket [client-visible] | `SessionService.IssuePlayTicket` (idempotency key); `play_tickets` table: one use only, 60 s, bound to account + character; the `/ws` handshake uses it in one transaction | S | Matrix tests, which include reuse and expiry |
| ✅ 1.5 UE device-flow login [client-visible] | `UAuthSubsystem`: start the device auth, show URL + code in a CommonUI screen, poll the token endpoint, keep the refresh token in `FPlatformMisc` secure storage, attach the bearer token to the TurboLink channel | O | The player logs in on a browser, the client gets a token, and calls `IssuePlayTicket` |
| ✅ 1.6 Verified identity and ownership | The caller identity comes from verified extensions; ownership checks on the character and ticket use cases (§8 #1) | S | The tests for forged ids and access between accounts pass |

### Epic 2: Persistence through SeaORM

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 2.1 Adopt SeaORM | Add `sea-orm` with the pool that we have now; `sea-orm-migration` wraps the current SQL; `sea-orm-cli` makes `infrastructure/postgres/entities/`, with a moon task `api:entities` and a CI check that the entities are up to date; port `PgCharacterRepository` and keep the transaction and `ON CONFLICT` semantics | S | All the Postgres adapter tests that we have now pass with no change |
| ✅ 2.2 Outbox relay | `infrastructure/outbox/relay.rs` worker: poll `outbox WHERE published_at IS NULL` with `FOR UPDATE SKIP LOCKED`, publish to NATS, mark; backoff on failure; metrics `outbox_pending`, `outbox_lag_seconds` | S | A test stops the process between commit and publish; the relay delivers after the restart |

### Epic 3: Event-driven deterministic core

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 3.1 Zone actor | `domain/zone/`: `Fixed` (i32, 1/1000 tile), `Tick(u64)`, `Entity`, movement at speed with arrival; `application/zone_actor.rs`: it controls the state of one zone, an `mpsc` command queue (bounded 1024), a fixed 100 ms tick from an injected `TickSource`, so that tests and replay can step manually; it emits `WorldEvent`s; a `ChaCha12` seed from `(zone_id, session_epoch)` | O | Property test: the same commands + seed -> an identical event stream; 1,000 entities tick in less than 2 ms |
| ✅ 3.2 Session event log | `infrastructure/eventlog/jetstream.rs`: stream `NF_SESSIONS`; for each inbound frame, publish `{session_id, seq, tick_applied, recv_unix_ms, bytes}`; for each outbound frame, publish `{session_id, tick, bytes}`; fire-and-forget with no ack, with a bounded buffer and a counter of dropped frames; an `EventLog` port with an in-memory adapter | O | An integration test reads back a session in order; the counter of dropped frames stays at 0 during the load test |
| ✅ 3.3 Replay tool | `apps/api/src/bin/nightfall-replay.rs`: arguments `--session <id>` or `--zone <id> --from <ts>`; loads the zone snapshot (Story 3.4) and the `.in` stream; runs the zone actor headless with the same seed; compares the outbound frames that it makes to `.out` byte for byte; prints the first divergence with tick, seq, and the decoded message diff | O | A replay of a recorded test run with 2 sessions reports zero divergence; it finds a float path that we inject on purpose |
| ✅ 3.4 Zone snapshot | When a session epoch starts, store the initial zone state (entities, seed) to the JetStream subject `nightfall.zone.<id>.snapshot` and to the Postgres `zone_snapshots` | S | Replay can start from each epoch |
| 3.5 Telemetry events | The domain events are on NATS now; add `nightfall.telemetry.<kind>` for session_started/ended and move_rejected, with OTel span links | S | You can see the events in Grafana through Loki |

### Epic 4: Real-time channel

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 4.1 `/ws` handler [client-visible] | axum upgrade on `/ws?ticket=`; use the ticket (Story 1.4); one session actor for each socket: decode `ClientMessage` with prost, validate that seq is monotonic, forward to the zone actor; outbound `mpsc(256)`, disconnect when it is full, with a metric; one `Ack` for each intent | O | Integration test with `tokio-tungstenite`: a bad ticket gives 4401, a reused ticket gives 4401, a seq regression disconnects |
| ✅ 4.2 Zone join and AOI [client-visible] | On connect: spawn an entity at the character position, send `EntitySpawn` for all entities in the 3x3 AOI cells, broadcast the spawn of the player; despawn on disconnect; the AOI cell is 32 tiles | S | Two clients see each other; a third client "outside AOI" does not |
| ✅ 4.3 MoveTo [client-visible] | Validate that the destination is in the zone bounds and in the maximum distance; set the destination; the zone actor moves at `speed`; emit `EntityMove` at 10 Hz only for entities that move, and one time on arrival | S | Matrix tests; the client receives its own `EntityMove` with `server_time_ms` |
| ✅ 4.4 Load test | `examples/ws_load.rs`: N sessions that walk at random; it records the tick duration, frames/s, and dropped frames | S | 200 sessions, tick p99 less than 20 ms on the dev box |

### Epic 5: Telemetry

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 5.1 OTel pipeline | `tracing-opentelemetry` + OTLP exporter; `grafana/otel-lgtm` in Compose; JSON logs to stdout with `request_id`, `session_id`, `tick`; scrape `/metrics` | S | A `MoveTo` shows as one trace: ws frame -> zone actor -> broadcast |
| ✅ 5.2 Metric catalogue | `tick_duration_seconds` (histogram), `sessions_active`, `ws_frames_total{dir}`, `ws_dropped_frames_total`, `outbox_pending`, `db_query_seconds`, `eventlog_publish_seconds` | S | The dashboard JSON is committed in `infra/grafana/` |
| ✅ 5.3 Alerts (dev) | Tick p99 > 50 ms, dropped frames > 0, outbox lag > 30 s | S | The alert rules are in the repo |

### Epic 6: Unreal client slice

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 6.1 TurboLink integration [client-visible] | Add the plugin as a submodule in `apps/client-unreal/Plugins/TurboLink`; make the services from `packages/proto` with its codegen (the moon task `client-unreal:gen-proto` replaces the shell script); `USessionClient` wrapper; remove `ProtoCodec.cpp` and encode WS frames with the generated classes | O | `Ping` does a round trip from the editor; the Linux build is clean with warnings-as-errors |
| ✅ 6.2 Login and connect [client-visible] | Device-flow UI (Story 1.5), `IssuePlayTicket`, `Connect`; a status line on the HUD | O | Click login -> browser -> back in the game -> connected |
| ✅ 6.3 Blueprint content [client-visible] | `BP_RemoteEntity` with a placeholder skeletal mesh and an idle/walk blend; `IMC_Default` with `IA_ClickMove`; `BP_NightfallPC`; `L_TestZone`: flat ground of 64x64 tiles, nav mesh, lights | O (in-editor) | Two editor instances see each other move |
| ✅ 6.3b Click-to-move moves the player [client-visible] | Player controller raycast and local preview, `MoveTo` intent round trip, reconciliation of the position of the player pawn with the server `EntityMove` through `OwnEntityComponent` | O | The player clicks to move, sends `MoveTo`, receives the server `Ack` and `EntityMove`, and the pawn moves and reconciles |
| 6.4 (deferred) Interpolation polish | Visual smoothing on `RemoteEntityActor`, rotation in the direction of motion, snap tolerance on arrival | S | No stutter that you can see at 10 Hz with 150 ms delay |

### Epic 7: Docs and CI

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 7.1 Decision records | Append D1-D7 to `docs/planning/00-foundations.md` as a "Revisions" section; update the cross-cutting table | S | The links resolve |
| 7.2 Engineering guidelines | Add ORM rules (entities are generated and never changed manually; transactions through `TransactionTrait`), event-log rules (each applied tick has a durable acknowledgement before release, and is never sampled; the session audit is best effort), determinism rules (no floats in `domain/zone`; a `clippy::float_arithmetic` deny lint in that module) | S | Lints enforce the determinism rule |
| 7.3 CI | Keycloak and LGTM as services; the replay test runs on a recorded fixture session that is checked in to `fixtures/sessions/` | S | `moon ci` is green |

## 5. Order and estimate

1. Epic 2 and Epic 5 (foundations, mechanical, in parallel): ~4 days.
2. Epic 1 (server stories) and Epic 3 (in parallel): ~8 days.
3. Epic 4: ~4 days.
4. Epic 6, which includes 1.5: ~6 days.
5. Epic 7 at the same time as the other epics.

The total is approximately four weeks of focused work. Stories with the mark S can run as parallel sessions with
`use_worktree: true`. Stories with the mark O run one at a time.

## 6. Risks

| # | Risk | Mitigation |
|---|------|------------|
| R1 | The last TurboLink release (v1.4.2, 2025-11-24) targets UE 5.6 and 5.7; 5.8 and the Linux clang-20 toolchain are not verified | The first task of Story 6.1 is a spike: build the sample project on 5.8 Linux in one day. If it fails, use one of these fallbacks: vendor grpc++ (plan B, +1 week), or use HTTP/JSON for unary calls (plan C, the rejected D3 alternative). **Retired 2026-10-08:** The Story 6.1 spike built TurboLink v1.4.2 on UE 5.8.3 Linux / clang 20, and Ping does a round trip. See the spike log in `apps/client-unreal/README.md`. Residual: TurboLink uses an API that is deprecated in 5.8. Thus it must have a patch before the next engine upgrade. |
| R2 | Determinism leaks (floats, HashMap iteration order, time reads) | `domain/zone` denies float arithmetic with a lint; only `BTreeMap`; all time comes from `Tick`; a replay test in CI on a fixture session. |
| R3 | JetStream publish latency on the tick path | Fire-and-forget through a bounded channel on a separate task; a dropped-frame metric; replay reports gaps explicitly. |
| R4 | UX of the Keycloak device flow in Unreal | Fallback: a developer login where you paste a token, behind a configuration flag, for local work. |
| R5 | SeaORM entity drift | A CI check makes the entities again and fails if there is a diff. |

## 7. Out of scope for this phase

Combat, items, chat, more than one zone, client prediction other than the local move preview,
production deployment, client-side frame recording (D4), event-log archival to object storage.

## 8. Revision 1 (2026-10-07): dispositions of the independent review

Codex did an independent read-only review (`reviews/phase-0b-codex-review-2026-10-07.md`). The result was
"major revision" with 19 findings. Decisions D1-D7 stay. The dispositions below are binding, and they
amend the stories above. The wave-2 briefs come from this section.

| # | Finding (short) | Disposition | Amendment |
|---|-----------------|-------------|-----------|
| 1 | Handlers trust `account_id` in requests; no ownership checks | **Accept** | New Story 1.6: `AccountId` comes only from verified request extensions. `GetCharacter`, `CreateCharacter`, `IssuePlayTicket` enforce ownership. `AppError::Unauthenticated` / `PermissionDenied` map to `UNAUTHENTICATED` / `PERMISSION_DENIED`. Tests for forged ids and for the character of a different account. |
| 2 | A fire-and-forget event log cannot make sure of retention | **Accept** | Story 3.2 is written again: JetStream publishes are **acknowledged**. The implemented order is **draft → run_tick → durable ack → release outputs**. The record has the calculated outputs, but no output goes out before the record is durable. The next draft also waits for the ack. Each epoch log has a **completion watermark**. Replay refuses recordings that are not complete. Tests: broker down, broker slow, process stopped in the middle of a tick. |
| 3 | Seqs for each session cannot reconstruct the order between sessions | **Accept** | The **zone actor is the only writer** of the replay log: subject `nightfall.zone.<zone>.<epoch>.applied`, one record for each tick, with `(epoch, tick, [commands in application order with ordinal])`. The `.in` / `.out` logs for each session stay for audit, and to select the session output to verify (`--session` selects; it never filters inputs). |
| 4 | The system does not record lifecycle inputs (join, load, disconnect, eviction) | **Accept** | Each state change is a `ZoneCommand` (Spawn has the loaded character state; Despawn; Replace-session). Thus each state change is in the applied log. Rejected or malformed inputs get a `Disposition` record (tick seen, reason) instead of `tick_applied`. |
| 5 | Epoch and snapshot are not defined | **Accept** | There is one **epoch for each zone**. It starts when the zone actor starts (restart = new epoch). The snapshot occurs at a tick boundary with no pending commands. It has: entities with the full movement state, RNG state bytes, the next command ordinal, the AOI index (derivable but included), the JetStream sequence of the first log record, `time_origin_ms`, build id, configuration hash, schema version. It is published to `nightfall.zone.<zone>.<epoch>.snapshot` **before** the first applied record. Retention: the logs and snapshots of an epoch stay together for ≥ 7 days. |
| 6 | Deterministic events do not cause identical bytes | **Accept** | `server_time_ms = time_origin_ms + tick × 100`, never a clock read. The entity ids for players are their character ids (inputs). All other ids come from the seeded RNG. The broadcast stage of the zone actor makes **one ordered output stream for each session** (acks, rejections, events), and replay compares that stream. Socket delivery has separate metering. `SystemClock` is forbidden in `domain/zone` and `application/zone_actor`. |
| 7 | The query-string ticket leaks into trace spans | **Accept** | The client sends the play ticket in the `Authorization: Bearer` header of the WebSocket upgrade, not in the URL (the Unreal `IWebSocket` supports upgrade headers). The HTTP span builder records only the path, never the query or headers. A test uses a sentinel ticket and makes sure that it is in no span or log. Tickets are 32 random bytes, stored as a hash. TLS is necessary outside loopback. |
| 8 | One-use tickets do not enforce one session for each account | **Accept** | An `account_sessions` reservation with a **generation** number. A new admission for an account increases the generation, and the zone actor serializes the replacement (despawn the old one, spawn the new one, as commands). The generation fences stale sockets, and their disconnect cannot remove the replacement. Tests: two concurrent valid tickets; failure after use. |
| 9 | Idempotent ticket issue against reconnect | **Accept** | A generic `idempotency_keys` table with the key `(account_id, operation, key)` and the stored response. A retry returns the same ticket, also after the ticket is used. Each connection attempt requests a new ticket with a new key. The client never uses a used ticket again (this repairs the reconnect loop that we have now). |
| 10 | The fixed-point conversion is not specified | **Accept** | For now, the wire keeps the float `Position`. After finite/range validation, the boundary quantizes to the nearest milli-tile, and rejects NaN/inf/out-of-bounds. Postgres keeps integer milli-tiles (the migration changes `pos_x/pos_y` to `integer`). Distances use i64 intermediate values. Speed is milli-tiles for each tick, with exact arrival. Direction normalisation is integer (no sqrt: step along the vector with i64 scaled math and a documented rounding rule). Tests at i32 extremes, diagonals, and speeds less than one unit. |
| 11 | The connected-client contract is not complete | **Accept** | New **Story 0.1 (contract first)**: `session.proto` (`SessionService.IssuePlayTicket`), `game.proto` gets `ListMyCharacters`, `world.proto` gets `IntentRejected{seq, reason}`, documented `Ack` semantics (applied against accepted), and `StopMove` handling. It merges before the codegen step of Story 6.1 and before Epic 4. |
| 12 | No resource bounds for each client | **Accept** | Maximum frame 4 KiB (reject larger frames before decode), 30 frames/s for each session (token bucket), idle timeout 60 s, 10 concurrent upgrades for each IP. If the queue is full → `IntentRejected{overloaded}`, not disconnect. A command budget for each tick for each session (8); the excess goes to the next tick, in order. A flood test with a neighbour that behaves correctly. |
| 13 | AOI only at join | **Accept** | At each tick, the zone actor keeps a set of known entities for each session. It emits ordered `EntitySpawn`/`EntityDespawn` diffs when observers or observed entities go across cells. A spawn for an entity that moves has its destination and speed. The fixture zone is 256×256 tiles, so you can test "outside AOI". |
| 14 | Relay publication is not durable; direct + relay publish makes duplicates | **Accept** | **Only** the relay publishes domain events (the use case does not publish directly now). The relay uses an acknowledged JetStream publish with `Nats-Msg-Id = outbox.id` for dedupe on the broker. The relay marks rows only after the ack. Test: crash after publish and before mark. |
| 15 | JWT verification cannot await in a sync interceptor | **Accept** | Verification is an async tower layer, not a tonic interceptor. The JWKS cache loads at boot, with a bounded refresh on an unknown kid. The integration test gets a real token from the Compose Keycloak through the device flow automation from Story 1.1. |
| 16 | Migration baseline; replay read outside the transaction | **Accept** | Story 2.1 adds a baseline migration that does nothing on databases that are at the current schema. `create_idempotent` reads the replayed character in the same transaction. |
| 17 | HTTP and WebSocket error semantics are mixed | **Accept** | Invalid, expired, or reused ticket → HTTP 401 before upgrade. Account connected → 409, and the replacement continues as in #8. Close codes only after upgrade. |
| 18 | The telemetry ingestion path is not complete | **Accept** | Logs go to LGTM over OTLP (tracing-opentelemetry log bridge), not through stdout scraping. Zone commands carry the trace context, so a `MoveTo` trace goes from socket → actor → broadcast. Acceptance: one real trace, one Loki line, one metric sample, one alert that fires, all verified by API query. |
| 19 | Sequence and CI | **Accept in part** | The TurboLink spike runs first now. Story 0.1 comes before codegen and Epic 4. Replay acceptance moves after Epic 4. The NATS in CI gets `--jetstream`. Replay CI runs on a new multi-session capture with failure injection, and also on the checked-in fixture. |

**Effect on work in progress.** The owners of this work in progress got a direct notification:
- Story 3.1 (ordinal stamping, applied-log shape, snapshot contents, AOI diffs, fixed-point specification)
- Epic 2 (relay durability, one publish path, replay read in the same transaction, baseline migration)
- Epic 5 (spans with only the path, OTLP logs, acceptance by query).

This revision has no effect on Story 1.1 and Story 6.1.

## 9. Outcome

On 2026-10-08, live play tests verified the connected vertical slice from one end to the other:
- **Verified in play:** Keycloak device-flow login, character create, play ticket, WebSocket admission, zone entry, click-to-move round trip, logout/login replacement.
- **Test count:** 314 server tests, a replay fixture with zero divergence.
- **Three lessons:**
  1. Integration loop timing.
  2. Sandboxed agents cannot write .git.
  3. The dev task must load .env.
