# Architecture

## 1. Layers

Clean architecture, four layers, dependencies point inward only:

```
apps/api/src/
  domain/          Entities, value objects, domain events, domain errors.
                   No framework, DB, transport, or runtime imports.
  application/     Use cases (one per endpoint) and ports (traits the use cases need).
                   Depends on domain only.
  infrastructure/  Adapters implementing ports: memory, postgres (sqlx), nats.
                   Depends on application and domain.
  interface/       Inbound adapters: http (axum), grpc (tonic). Parse, call one use case,
                   map result/error to the wire. Depends on application and domain.
  lib.rs           `Dependencies` (ports bound to adapters) and server builders.
  main.rs          Composition root: config -> adapters -> servers.
```

The rule, from Kigawas: routers thin, use cases thick, models slim. A handler that contains an
`if` about business state is in the wrong layer.

### 1.1 What goes where

| Question | Layer |
|----------|-------|
| Is a name valid? | domain (`CharacterName::new`) |
| What happens when a character is created? | application (`CreateCharacter::execute`) |
| How is a character stored? | infrastructure (`PgCharacterRepository`) |
| What gRPC code does "name taken" map to? | interface (`to_status`) |
| What stats does a Dwarf start with? | domain, from data files once Phase 1 lands |

### 1.2 Ports

`application::ports` defines:

- `CharacterRepository`: `get`, `list_by_account`, `create_idempotent`. Each method is one
  atomic unit of work.
- `AccountRepository`: `record_login` (idempotent upsert).
- `SessionRepository`: `issue_ticket_idempotent`, `consume_ticket`.
- `TokenVerifier`: `verify(token) -> Claims` (Keycloak JWKS in production).
- `SecretGenerator`: `play_ticket()` (OS CSPRNG in production, fixed in tests).
- `EventBus`: `publish(&DomainEvent)`.
- `SessionAudit`: `record_in(session, seq, frame)`, `record_out(session, frame)`; never blocks
  (the real-time channel's per-session audit log).
- `Clock`: `now()`.

Adapters are chosen at the composition root and injected as `Arc<dyn Port>` via
`Dependencies`. Tests build `Dependencies` with in-memory adapters; `main` builds it from
`DATABASE_URL` and `NATS_URL`, falling back to in-memory with a warning when either is unset.

## 2. The event-bus core

Following jdno's design: the API does not mutate the world directly. Two buses connect three
independent systems.

```
  client ──gRPC/WS──▶ API (interface + application)
                        │ commands                     ▲ events
                        ▼                              │
                    command bus ──▶ world simulation ──▶ event bus ──▶ API, projections,
                    (NATS)            (tick loop)         (NATS)        chat, telemetry
```

- **Command**: an intent that may be rejected. `MoveTo`, `UseSkill`, `CreateCharacter`.
  Validated by a use case, then either applied in a transaction (account/character data) or
  forwarded to the simulation (world state).
- **Event**: a fact that already happened. `CharacterCreated`, `EntityMoved`, `SkillCast`.
  Published after the state change is durable. Consumers are idempotent because delivery is
  at-least-once.

### 2.1 Why NATS

- Core NATS gives fan-out pub/sub with subject wildcards, which is exactly the shape of
  "every system that cares about character events subscribes to `nightfall.character.>`".
- JetStream adds persistence and replay when a consumer needs it (projections, audit), without
  changing publishers.
- One binary, trivial to run locally (`docker compose up -d`), Rust client is first-party
  (`async-nats`).

### 2.2 Subjects

`nightfall.<aggregate>.<event>` for events, `nightfall.cmd.<aggregate>.<command>` for commands.
Snake case. The subject is a method on the event (`DomainEvent::subject`), so it cannot drift
from the type. Payload is JSON with a `type` tag today; switch to protobuf on the bus when a
non-Rust consumer appears.

### 2.3 Transactional outbox

A process can die between committing a transaction and publishing its event. So:

1. The repository inserts the event into `outbox` in the same transaction as the state change.
2. The use case does not publish. The relay (`infrastructure/outbox/relay.rs`) is the only
   publisher of domain events, so there is a single path and no duplicate-publish race.
3. The relay polls `outbox WHERE published_at IS NULL ORDER BY id LIMIT 100 FOR UPDATE SKIP
   LOCKED` in a transaction (250 ms poll, exponential backoff to 5 s on failure), publishes each
   row to `JetStream` stream `NF_EVENTS` (`nightfall.>`) and waits for the ack, sending
   `Nats-Msg-Id = outbox.id` so the broker drops a retry after a crash between publish and
   mark. `published_at` is set only after the ack. Several relays can run side by side.
   ``RelayStats` exposes `outbox_pending` and the last publish lag for telemetry.

Delivery is at-least-once with broker-side dedupe inside the stream's duplicate window;
consumers still dedupe on the event's aggregate id and a sequence.

### 2.4 The world simulation

Each zone is one tokio task, the **zone actor** (`application::zone_actor`), which owns a
`domain::zone::ZoneState`. Nothing else touches zone state: no `Arc<Mutex<World>>`. It consumes
commands and produces events like every other system, so it can move to its own process later
without changing the API. The simulation is deterministic (see `rust-guidelines.md` §7).

```
 session / admission ──ZoneHandle::send(ZoneInput)──▶ mpsc(1024) ──▶ zone actor
                                                                       │ every TickSource beat:
                                                                       │ 1 draft  (ordinals, ≤8/session)
                                                                       │ 2 TickGate::admit(draft)
                                                                       │ 3 ZoneState::run_tick
 subscribers ◀──broadcast(64) Arc<AppliedTick>─────────────────────────┘
 telemetry   ◀──watch TickStats
```

- **Inputs.** `ZoneInput { source, seq, command }`. `source` is `System` (admission, eviction,
  NPCs) or `Session { entity, generation }`. A session may only steer or remove its own entity,
  and only while its generation is current (`ReplaceSession` fences older sockets). Commands:
  `SpawnPlayer` (with the loaded character state), `SpawnNpc` (id from the zone RNG),
  `Despawn`, `ReplaceSession`, `MoveTo` (inside bounds, at most 64 tiles), `StopMove`.
- **Handle.** `ZoneHandle::send` never blocks: a full queue returns `ZoneSendError::Full` and
  the session reports `OVERLOADED`. `send_traced` also carries a `TraceCarrier` (never part of
  the input or the log) so the actor's `zone.apply` span joins the sender's trace.
  `send_wait` waits for room; only lifecycle commands (spawn, replace, despawn) use it. `subscribe()` gives one `Arc<AppliedTick>` per non-idle
  tick. `snapshot().await` answers at the next tick boundary with nothing deferred. `stats()` is
  a watch of `TickStats { tick, duration_micros, entities, commands_applied, commands_deferred,
  gate_holds }`.
- **Tick.** `TickSource` drives it: `IntervalTicks` (100 ms, missed beats run back to back) in
  production, `manual_ticks()` for tests and replay. Inputs received before a beat are drafted
  in receive order, up to 8 per session (the excess waits in order), and get consecutive
  ordinals. The `TickGate` is awaited before anything is applied. `OpenGate` admits everything.
  Story 3.2's gate is the acknowledged `JetStream` write of the draft. A refused draft holds the
  tick: nothing is applied and the same inputs are retried on the next beat.
- **Output.** `AppliedTick { epoch, tick, server_time_ms, commands, dispositions, events,
  outputs }`. `commands` (with ordinals and sources) is the replay log's unit. `dispositions`
  records every refused command. `events` are the zone-wide facts. `outputs` is each player's
  ordered stream: responses to its own commands (`Accepted` for each applied command that
  carries a `seq`, `Rejected` for each refused one, in ordinal order), then AOI despawns,
  spawns and moves, each in entity-id order. A session sends exactly this stream. The AOI is the 3x3 block of 32-tile cells, diffed every tick against what the player
  already knows.
- **Snapshot.** `ZoneSnapshot` holds full entity state, RNG state, next ordinal, AOI index,
  `time_origin_ms` and provenance (`schema_version`, `build_id`, `config_hash`,
  `first_log_seq`). `ZoneState::from_snapshot` validates it. Replaying the logged drafts from
  it reproduces the same `AppliedTick`s.
- **Wire.** `interface::zone_mapping` converts to and from `nightfall.v1` world.proto, with
  one function per message (`spawn_to_pb`, `move_to_pb`, `despawn_to_pb`, `disposition_to_pb`,
  `observer_output_to_pb`). It is the only place zone values become floats. A spawn of a moving
  entity becomes `EntitySpawn` and then `EntityMove`. A stopped entity is sent with
  destination and speed zero. Domain reasons without their own wire value map to `INVALID`,
  with the domain reason in `detail`.

### 2.5 Sessions and zones

`application::session` runs one actor per WebSocket (api-guidelines.md section 3b);
`interface::ws` supplies the axum socket halves and the protobuf codec through the
`FrameSource`, `FrameSink` and `SessionCodec` traits, so the session logic has no transport or
wire dependency. `SessionRegistry` records which session owns each player entity and queues
lifecycle commands under one lock, so the zone sees admissions and departures in the order the
registry decided them. `application::zone_registry::ZoneRegistry` starts the one fixture zone
(256x256 tiles, in memory, a new epoch per start) until Story 3.4's bootstrap replaces it.

## 3. Request lifecycle: `CreateCharacter`

0. `AuthLayer` verifies the bearer token and puts the caller's `AccountId` in the request
   extensions (api-guidelines.md section 3a). No token, no handler.
1. `interface::grpc` reads the caller from the extensions and parses `idempotency_key` and
   `race` into typed values. Any failure is `INVALID_ARGUMENT` and nothing else runs.
2. `CreateCharacter::execute` validates the name (domain), builds the aggregate, computes the
   idempotency fingerprint.
3. `PgCharacterRepository::create_idempotent` runs one transaction: claim
   `(account, create_character, key)`, insert character, stage outbox row, commit. Unique
   violations map to typed errors.
4. The outbox relay publishes `CharacterCreated` to JetStream after the commit.
5. The interface maps the result to `nightfall.v1.Character` or a status code.

Every step has a test: domain constructors (unit), use case with in-memory ports (unit),
repository against Postgres (adapter), the endpoint through a real socket (integration).

## 3a. Request lifecycle: `MoveTo` over the WebSocket

0. **Handshake** (`interface::ws::upgrade`, once per socket). Per-IP slot (429), play ticket
   from `Authorization: Bearer` consumed by `ConsumePlayTicket` (401/409), character loaded by
   `GetCharacter` and converted to zone units by `zone_mapping::player_spawn`. Upgrade; the
   session actor subscribes to the zone's broadcast, then `SessionRegistry::admit` queues
   `SpawnPlayer` or `ReplaceSession`. Nothing is forwarded until that command's tick arrives.
1. **Frame in** (session actor). The binary frame is counted (`ws_frames_total{in}`) and
   audited (`SessionAudit::record_in`). Then, in order: decode with prost (only if at most
   4096 bytes); `seq` must exceed the last (else close 4400); token bucket (else
   `RATE_LIMITED`); size, binary and decode checks (else `INVALID`); `zone_mapping` turns
   `MoveToRequest` into `ZoneCommand::MoveTo`, rounding the float destination to milli-tiles
   (non-finite: `OUT_OF_BOUNDS`). Every refusal here is an `IntentRejected` queued at once.
2. **To the zone.** A root span `ws.frame{seq}` is opened and its `TraceCarrier` rides with
   `ZoneInput::session(entity, generation, seq, MoveTo)` through `ZoneHandle::send_traced`. A
   full queue is `OVERLOADED`.
3. **Tick** (zone actor, next 100 ms beat). The input is drafted with an ordinal (at most 8 per
   session per tick), admitted by the `TickGate`, and applied by `ZoneState::run_tick`:
   authorised against the session's generation, checked against bounds and the 64-tile limit,
   then the movement phase takes the first step and the AOI diff runs. The player's output
   for the tick is `Accepted{seq, tick}` (or `Rejected`) followed by its `EntityMove`s. The
   actor records `zone.apply` under the carried span and broadcasts the `AppliedTick`.
4. **Frames out** (every session in range). Each session takes its player's output from the
   tick, encodes it with `zone_mapping` (`server_time_ms = time_origin_ms + tick * 100`, never a
   clock read), records `ws.deliver` under the intent's trace, audits each frame
   (`record_out`) and queues it on its outbound `mpsc(256)`. A full queue drops the frame,
   counts it, and closes 4429.
5. **Socket** (writer task). Frames are written in queue order and counted
   (`ws_frames_total{out}`). Later ticks emit one `EntityMove` per tick while the entity walks
   and a final one on arrival (destination and speed zero).

Every step has a test: domain (`state_tests.rs`: acks and order), actor (`session_tests.rs`:
admission, 4429, `OVERLOADED`, replacement), wire (`ws_session.rs`, `ws_trace.rs`).

## 4. Adding a feature

1. Domain: add or extend the entity and its invariants. Unit tests.
2. Proto: add the RPC and messages. Mutating RPCs get `idempotency_key`.
3. Application: add a port method if needed, then one use case. Unit tests with memory adapters.
4. Infrastructure: implement the port in `memory` and `postgres`. Adapter tests.
5. Interface: wire the RPC to the use case. Integration tests per `api-guidelines.md`.
6. If the feature emits an event: add a `DomainEvent` variant, stage it in the outbox.
