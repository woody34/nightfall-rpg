# Phase 0b Plan: Connected Vertical Slice

**Status:** approved for refinement, not started. **Date:** 2026-10-07. **Owner:** Matt.
**Planning model:** this plan was produced by Claude Fable 5.1 at high effort. Per-story model and
effort recommendations are in each story.

## 1. Goal

A player logs into the Unreal client through Keycloak, enters a zone, sees another entity move,
and clicks to move with a server round trip. Every frame between client and server is logged to
NATS JetStream so any session can be replayed on a headless server and produce byte-identical
output. Telemetry flows to Grafana from the first commit.

## 2. Decisions (supersede the matching parts of docs/planning/00-foundations.md)

| # | Decision | Choice | Rejected | Why |
|---|----------|--------|----------|-----|
| D1 | Identity provider | **Keycloak**, self-hosted in Compose, realm config as code, device authorization grant for the client | Zitadel, Auth0/Clerk, Epic Online Services | Most mature OIDC server; device flow suits a game client (no embedded browser; CEF is broken on Linux/Wayland); federates Epic/Steam/Google later. The server never sees a password. |
| D2 | ORM | **SeaORM** on the existing sqlx pool; `sea-orm-migration` for migrations; entities generated in CI | Diesel, raw sqlx | Async, same pool, transactions with the same atomicity rules. Repository ports and their tests do not change. |
| D3 | Client request/response | **gRPC via TurboLink** in Unreal, tonic on the server (no tonic-web needed) | HTTP/JSON | One protocol everywhere; TurboLink bundles protoc, so its generated classes also replace the hand-written WebSocket codec. Cost: first-build fights on Linux (see R1). |
| D4 | Replay scope | **Server-side**: log inbound and outbound frames per session; headless replay asserts identical outbound bytes | Client-side frame recording too | Enough to reproduce any server bug; client recording is only useful once client prediction exists. |
| D5 | Session event log | **NATS JetStream**, one stream `NF_SESSIONS`, subjects `nightfall.session.<id>.in` / `.out`, retention 7 days | Postgres table, both | Already running; append-only with replay by sequence; keeps write load off the game DB. Archival to object storage is a later story. |
| D6 | Telemetry backend | **Grafana LGTM** all-in-one in Compose, OpenTelemetry OTLP from the server | Datadog, Honeycomb | Free, local, OTel-native; forward to a SaaS later by changing one endpoint. |
| D7 | Determinism | Fixed 100 ms tick; positions as `i32` fixed-point (1/1000 tile); `ChaCha12` RNG seeded per zone per session; every command stamped with the tick it is applied on | f32 positions | "Perfect replay" is impossible otherwise. Floats drift across builds and CPUs. |

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

Layering is unchanged: `domain` (zone rules, movement math), `application` (use cases:
IssuePlayTicket, JoinZone, MoveTo), `infrastructure` (Keycloak JWKS, SeaORM repos, JetStream log,
OTel), `interface` (gRPC services, `/ws` handler).

## 4. Epics, stories, tasks

Effort levels: **M** mechanical, **D** design-heavy. Model: **S** Sonnet medium effort,
**O** Opus high effort. Every story that adds an endpoint carries the test matrix from
`docs/engineering/api-guidelines.md` §4.

### Epic 1: Identity

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 1.1 Keycloak in Compose, realm as code | Add `keycloak` service (Postgres-backed, dev mode); `infra/keycloak/realm-nightfall.json` with a public client `nightfall-client` (device flow on, PKCE), test user; import on boot; document in README | S | `docker compose up` yields a realm; device flow works with `curl` |
| 1.2 JWT validation in the server | `infrastructure/auth/keycloak.rs`: JWKS fetch + cache + rotation; `application/ports::TokenVerifier`; tonic interceptor that puts `AccountId` in request extensions; config `OIDC_ISSUER`, `OIDC_AUDIENCE` | S | Unit tests with a generated RSA key; integration test with a signed test token; expired/wrong-aud/wrong-iss rejected |
| 1.3 Account on first login | `accounts` table (id = IdP `sub`, created_at, last_login); `EnsureAccount` use case, idempotent by construction | S | Two logins create one row |
| 1.4 Play ticket | `SessionService.IssuePlayTicket` (idempotency key); `play_tickets` table: single-use, 60 s, bound to account + character; consumed in the `/ws` handshake in one transaction | S | Matrix tests incl. reuse and expiry |
| 1.5 UE device-flow login | `UAuthSubsystem`: start device auth, show URL + code in a CommonUI screen, poll token endpoint, store refresh token with `FPlatformMisc` secure storage, attach bearer token to TurboLink channel | O | Player logs in on a browser, client gets a token, calls `IssuePlayTicket` |

### Epic 2: Persistence through SeaORM

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 2.1 Adopt SeaORM | Add `sea-orm` with the existing pool; `sea-orm-migration` wrapping current SQL; `infrastructure/postgres/entities/` generated by `sea-orm-cli` with a moon task `api:entities` and a CI check that it is up to date; port `PgCharacterRepository` keeping transaction and `ON CONFLICT` semantics | S | All existing Postgres adapter tests pass unchanged |
| 2.2 Outbox relay | `infrastructure/outbox/relay.rs` worker: poll `outbox WHERE published_at IS NULL` with `FOR UPDATE SKIP LOCKED`, publish to NATS, mark; backoff on failure; metrics `outbox_pending`, `outbox_lag_seconds` | S | Kill the process between commit and publish in a test; relay delivers after restart |

### Epic 3: Event-driven deterministic core

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 3.1 Zone actor | `domain/zone/`: `Fixed` (i32, 1/1000 tile), `Tick(u64)`, `Entity`, movement at speed with arrival; `application/zone_actor.rs`: owns one zone's state, `mpsc` command queue (bounded 1024), fixed 100 ms tick driven by an injected `TickSource` so tests and replay step manually; emits `WorldEvent`s; seeded `ChaCha12` from `(zone_id, session_epoch)` | O | Property test: same commands + seed -> identical event stream; 1,000 entities tick under 2 ms |
| 3.2 Session event log | `infrastructure/eventlog/jetstream.rs`: stream `NF_SESSIONS`; on each inbound frame publish `{session_id, seq, tick_applied, recv_unix_ms, bytes}`; on each outbound frame publish `{session_id, tick, bytes}`; ack-free fire-and-forget with a bounded buffer and a dropped-frames counter; `EventLog` port with an in-memory adapter | O | Integration test reads back a session in order; dropped-frame counter stays 0 under load test |
| 3.3 Replay tool | `apps/api/src/bin/nightfall-replay.rs`: args `--session <id>` or `--zone <id> --from <ts>`; loads zone snapshot (Story 3.4) and `.in` stream; runs the zone actor headless with the same seed; compares produced outbound frames to `.out` byte for byte; prints first divergence with tick, seq, decoded message diff | O | Replaying a recorded 2-session test run reports zero divergence; a deliberately injected float path is caught |
| 3.4 Zone snapshot | On session epoch start, persist zone initial state (entities, seed) to JetStream subject `nightfall.zone.<id>.snapshot` and Postgres `zone_snapshots` | S | Replay can start from any epoch |
| 3.5 Telemetry events | Domain events already on NATS; add `nightfall.telemetry.<kind>` for session_started/ended, move_rejected, with OTel span links | S | Events visible in Grafana via Loki |

### Epic 4: Real-time channel

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 4.1 `/ws` handler | axum upgrade on `/ws?ticket=`; consume ticket (Story 1.4); session actor per socket: decode `ClientMessage` with prost, validate seq monotonic, forward to zone actor; outbound `mpsc(256)`, disconnect on full with metric; `Ack` per intent | O | Integration test with `tokio-tungstenite`: bad ticket 4401, reused ticket 4401, seq regression disconnects |
| 4.2 Zone join and AOI | On connect: spawn entity at character position, send `EntitySpawn` for all entities in the 3x3 AOI cells, broadcast own spawn; despawn on disconnect; AOI cell 32 tiles | S | Two clients see each other; a third outside AOI does not |
| 4.3 MoveTo | Validate destination within zone bounds and max distance; set destination; zone actor moves at `speed`; emit `EntityMove` at 10 Hz only for moving entities and once on arrival | S | Matrix tests; client receives its own `EntityMove` with `server_time_ms` |
| 4.4 Load test | `examples/ws_load.rs`: N sessions random-walking; records tick duration, frames/s, dropped frames | S | 200 sessions, tick p99 under 20 ms on the dev box |

### Epic 5: Telemetry

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 5.1 OTel pipeline | `tracing-opentelemetry` + OTLP exporter; `grafana/otel-lgtm` in Compose; JSON logs to stdout with `request_id`, `session_id`, `tick`; `/metrics` scraped | S | A `MoveTo` shows as one trace: ws frame -> zone actor -> broadcast |
| 5.2 Metric catalogue | `tick_duration_seconds` (histogram), `sessions_active`, `ws_frames_total{dir}`, `ws_dropped_frames_total`, `outbox_pending`, `db_query_seconds`, `eventlog_publish_seconds` | S | Dashboard JSON committed under `infra/grafana/` |
| 5.3 Alerts (dev) | Tick p99 > 50 ms, dropped frames > 0, outbox lag > 30 s | S | Alert rules in repo |

### Epic 6: Unreal client slice

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 6.1 TurboLink integration | Add plugin as a submodule under `apps/client-unreal/Plugins/TurboLink`; generate services from `packages/proto` with its codegen (moon task `client-unreal:gen-proto` replaces the shell script); `USessionClient` wrapper; delete `ProtoCodec.cpp` and encode WS frames with the generated classes | O | `Ping` round-trips from the editor; Linux build clean with warnings-as-errors |
| 6.2 Login and connect | Device-flow UI (Story 1.5), `IssuePlayTicket`, `Connect`; status HUD line | O | Click login -> browser -> back in game -> connected |
| 6.3 Blueprint content | `BP_RemoteEntity` with a placeholder skeletal mesh and idle/walk blend; `IMC_Default` with `IA_ClickMove`; `BP_NightfallPC`; `L_TestZone`: 64x64 tile flat ground, nav mesh, lights | O (in-editor) | Two editor instances see each other move |
| 6.4 Interpolation polish | Visual smoothing on `RemoteEntityActor`, rotation toward motion, arrival snap tolerance | S | No visible stutter at 10 Hz with 150 ms delay |

### Epic 7: Docs and CI

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 7.1 Decision records | Append D1-D7 to `docs/planning/00-foundations.md` as a "Revisions" section; update cross-cutting table | S | Links resolve |
| 7.2 Engineering guidelines | Add ORM rules (entities generated, never hand-edited; transactions via `TransactionTrait`), event-log rules (every frame logged, never sampled), determinism rules (no floats in `domain/zone`, lint via `clippy::float_arithmetic` deny in that module) | S | Lints enforce the determinism rule |
| 7.3 CI | Keycloak and LGTM as services; replay test runs on a recorded fixture session checked into `fixtures/sessions/` | S | `moon ci` green |

## 5. Order and estimate

1. Epic 2 and Epic 5 (foundations, mechanical, parallel): ~4 days.
2. Epic 1 (server stories) and Epic 3 (parallel): ~8 days.
3. Epic 4: ~4 days.
4. Epic 6, with 1.5 folded in: ~6 days.
5. Epic 7 alongside.

About four weeks of focused work. Stories marked S can run as parallel sessions with
`use_worktree: true`; stories marked O one at a time.

## 6. Risks

| # | Risk | Mitigation |
|---|------|------------|
| R1 | TurboLink is lightly maintained (last release late 2024) and may not build against UE 5.8 or the Linux toolchain | Story 6.1 first task is a spike: build the sample project on 5.8 Linux in one day. If it fails, fall back to vendoring grpc++ (plan B, +1 week) or to HTTP/JSON for unary calls (plan C, the rejected D3 alternative). |
| R2 | Determinism leaks (floats, HashMap iteration order, time reads) | `domain/zone` denies float arithmetic by lint; `BTreeMap` only; all time comes from `Tick`; replay test in CI on a fixture session. |
| R3 | JetStream publish latency on the tick path | Fire-and-forget through a bounded channel on a separate task; dropped-frame metric; replay reports gaps explicitly. |
| R4 | Keycloak device flow UX in Unreal | Fallback: paste-a-token developer login behind a config flag for local work. |
| R5 | SeaORM entity drift | CI check regenerates entities and fails on diff. |

## 7. Out of scope for this phase

Combat, items, chat, multiple zones, client prediction beyond the local move preview,
production deployment, client-side frame recording (D4), event-log archival to object storage.
