# Architecture

## 1. Layers

[System architecture](../diagrams/system-architecture.html) shows the client, server and
external services with development ports. [Module dependencies](../diagrams/module-dependencies.html)
shows the four layers and their actual imports.

Dependencies should point inward: interface and infrastructure depend on application and
domain. `main.rs`, `lib.rs` and `zone_runtime.rs` compose adapters and servers; `config.rs`
reads process settings. Current exception: `interface::http` and `interface::grpc::auth`
import infrastructure telemetry helpers. Application orchestration uses Tokio and tracing;
domain remains independent of the runtime, transport and database.

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
- `EventLog`, `ZoneSnapshotStore`: the zone replay log and its Postgres epoch index (§2.5).

Adapters are chosen at the composition root and injected as `Arc<dyn Port>` via
`Dependencies`. Tests build `Dependencies` with in-memory adapters; `main` builds it from
`DATABASE_URL` and `NATS_URL`, falling back to in-memory with a warning when either is unset.

## 2. The event-bus core

The current runtime has two distinct paths, shown in the
[system diagram](../diagrams/system-architecture.html): transactional character creation
stages its domain event in the Postgres outbox; world commands enter the zone actor through an
in-process bounded `mpsc` queue. NATS JetStream persists domain events, applied ticks and
session audit frames. A NATS command bus is a future distribution option, not the current
zone-input path.

### 2.1 Why NATS

- Core NATS gives fan-out pub/sub with subject wildcards, which is exactly the shape of
  "every system that cares about character events subscribes to `nightfall.character.>`".
- JetStream adds persistence and replay when a consumer needs it (projections, audit), without
  changing publishers.
- One binary, trivial to run locally (`docker compose up -d`), Rust client is first-party
  (`async-nats`).

### 2.2 Subjects

`nightfall.<aggregate>.<event>` for domain events; currently `nightfall.character.created`.
A future command-subject convention is `nightfall.cmd.<aggregate>.<command>`; no zone commands
are published there today. Domain-event subjects come from `DomainEvent::subject`. Payload is JSON with a `type` tag today; switch to protobuf on the bus when a
non-Rust consumer appears.

### 2.3 Transactional outbox

[Persistence data model](../diagrams/data-model.html) shows every current table column,
composite key and declared foreign key. SeaORM adapters implement the repository ports.
Dashed account relationships in the diagram are application associations, not SQL constraints.

A process can die between committing a transaction and publishing its event. So:

1. The repository inserts the event into `outbox` in the same transaction as the state change.
2. The use case does not publish. The relay (`infrastructure/outbox/relay.rs`) is the only
   publisher of domain events, so there is a single path and no duplicate-publish race.
3. The relay polls `outbox WHERE published_at IS NULL ORDER BY id LIMIT 100 FOR UPDATE SKIP
   LOCKED` in a transaction (250 ms poll, exponential backoff to 5 s on failure), publishes each
   row to `JetStream` stream `NF_EVENTS` (`nightfall.*.*`) and waits for the ack, sending
   `Nats-Msg-Id = outbox.id` so the broker drops a retry after a crash between publish and
   mark. `published_at` is set only after the ack. Several relays can run side by side.
   `RelayStats` exposes `outbox_pending` and the last publish lag for telemetry.

Delivery is at-least-once with broker-side dedupe inside the stream's duplicate window;
consumers still dedupe on the event's aggregate id and a sequence.

### 2.4 The world simulation

Each zone is one tokio task, the **zone actor** (`application::zone_actor`), which owns a
`domain::zone::ZoneState`. Nothing else touches zone state: no `Arc<Mutex<World>>`. It receives
in-process commands and produces applied ticks. Moving it to another process would require a
transport adapter for that boundary. The simulation is deterministic (see `rust-guidelines.md` §7).

[Deterministic tick and replay](../diagrams/deterministic-tick-replay.html) shows the live
pipeline and reconstruction path. The actor drafts, runs, awaits the durable log ack, then
releases output. A failed append holds the same record and prevents the next draft or snapshot.

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
- **Tick.** `IntervalTicks` supplies 100 ms beats (missed beats run back to back);
  `manual_ticks()` drives tests. Drafts take receive-order inputs, at most 8 per session,
  with consecutive ordinals; excess waits. `OpenGate` is the test gate and
  `DurableTickGate` waits for JetStream (§2.5). Idle ticks are logged but not broadcast.
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

### 2.5 Replay log

Stories 3.2 and 3.4 (plan §8 #2-#6). The zone actor is the **single writer** of its zone's
replay log; per-session logs are audit only. Code: `application::replay_log` (records, codec,
`EventLog` port, `DurableTickGate`, `SessionAuditWriter`, `open_epoch`),
`application::zone_bootstrap`, `infrastructure::eventlog` (`JetStream`, memory),
`infrastructure::postgres::PgZoneSnapshotStore`.

**Epoch lifecycle.** One epoch per zone run; a restart is a new epoch, numbered one past the
highest the log or `zone_snapshots` knows. `ZoneBootstrap::start` loads the zone definition
(`packages/data/zones/*.toml`, hashed into `config_hash`), builds tick 0's state, writes the
**snapshot** to the log and its row to `zone_snapshots`, and only then builds the gate (it needs
the `EpochStarted` proof that writing the snapshot returns) and spawns the actor. Starting NPCs
enter as `SpawnNpc` commands on the first tick, so they are in the applied log too.
`RunningZone::shutdown` stops the actor after its current tick and writes the **watermark**
(last acknowledged tick, record count, reason `shutdown` or `epoch_end`).

```
 nightfall.zone.<zone>.<epoch>.snapshot   ── once, before anything else of the epoch
 nightfall.zone.<zone>.<epoch>.applied    ── one record per tick, ticks contiguous from the
                                             snapshot's tick, idle ticks included
 nightfall.zone.<zone>.<epoch>.watermark  ── once, after the actor stopped
 nightfall.session.<session>.in / .out    ── audit frames (NF_SESSIONS), best effort
```

**Streams.** `NF_ZONES` (`nightfall.zone.*.*.*`) and `NF_SESSIONS` (`nightfall.session.*.*`),
file storage, `max_age` 7 days, created or updated on startup. `NF_EVENTS` is
`nightfall.*.*` (domain events only), so no two streams overlap. Zone publishes carry
`Nats-Msg-Id` (`<zone>/<epoch>/<tick>` for records; the subject for snapshot and watermark) and
the broker drops a retry inside its 2-minute duplicate window. **Retention:** an epoch's
snapshot and its log live in the same stream and age out together, so an epoch is replayable
for at least 7 days; `zone_snapshots` rows are kept at least as long.

**Record.** `AppliedTickRecord { zone, epoch, tick, server_time_ms, commands (ordinal, source,
seq, command), dispositions, outputs, output_form }`, where `outputs` is each player's ordered
output encoded with `encode_outputs` (or its digest, below), in entity-id order. Replay re-runs `commands` from the snapshot
and compares the re-encoded record **byte for byte**. Encoding: protobuf through hand-derived
`prost` messages (schema in `replay_log/codec.rs`); serde with bincode was rejected because the
zone's internally tagged serde enums cannot be decoded by non-self-describing formats. The
snapshot and watermark are canonical JSON (written once per epoch, readable in an incident);
`zone_snapshots.snapshot` holds the same bytes as the log message, plus
`jetstream_snapshot_seq`, `jetstream_first_seq` (filled once the first record is acked),
`time_origin_ms`, `build_id`, `config_hash` and `schema_version`.

**Record size bound.** A record grows with players × AOI population (≈ 48 KB for 50 players
each seeing 20 moves); around a thousand entities it would pass NATS `max_payload` (1 MiB by
default, payload plus headers), and an append that can never fit would stall and then pause
the zone forever. So before the first attempt the gate calls `AppliedTickRecord::bounded`
with `EventLog::max_record_bytes()` (JetStream: the `max_payload` the server last announced,
less 1 KiB for headers): if the encoding is over it, every player's output is replaced by its
**SHA-256** and the record carries `output_form = SHA256` (protobuf field 8; per-player digests
in `PlayerOutput.sha256`, field 3). Commands and dispositions are always stored in full, so
replay still re-runs the tick; `AppliedTickRecord::reproduced_by` digests the re-run's outputs
for such a record and compares the encodings, so a single changed output byte is still a
divergence. What is lost is the ability to *show* a digested tick's outputs from the log alone
(re-run it to see them). Each one counts `eventlog_digested_records_total`. Full-output records
encode exactly as before (field 8 is omitted at its default), and old records decode as
`ENCODED`. If even the digested record is over the limit (only commands and dispositions are
left, bounded by the 1024-input queue) the gate logs an error and the append fails like any
other. Chunking a record across several messages was rejected: it would turn one acked append
into several, complicate dedupe, `epoch_status` and `read_epoch`, and the full outputs of a tick
that large are not worth the stall risk.

**The gate.** The [tick diagram](../diagrams/deterministic-tick-replay.html) carries the
ordering and failure path. The record includes outputs, so the tick runs before the ack;
computed state is private until release. Append retries back off from 10 ms to 1 s. After
5 s the zone pauses (`zones_paused`, alert `nf-zone-paused`), and sessions answer
`IntentRejected{OVERLOADED}`; the next ack resumes it. Shutdown interrupts a stalled gate,
so the watermark names only acknowledged ticks.

**Replay** (library path exercised in `tests/replay_log.rs`, not automatic restart recovery)
opens an epoch with `open_epoch`, which refuses one without a watermark
(`EpochStatus::Incomplete`, e.g. after a crash) or without a snapshot (`Missing`), and checks as
it streams that ticks are contiguous from the snapshot to the watermark (`Gap`, `Truncated`).

**Audit.** `SessionAuditWriter::record_in / record_out` never block: frames go to a bounded
buffer (16 Ki) drained in pipelined batches. A full buffer or an unacknowledged frame is
dropped and counted in `eventlog_audit_dropped_total` (alert `nf-audit-dropped`). Replay never
depends on these frames.

### 2.6 Sessions and zones

[Device login → zone admission](../diagrams/login-zone-sequence.html) follows the access
token, single-use play ticket and ticket-in-header upgrade.
[Session lifecycle](../diagrams/session-lifecycle.html) separates HTTP refusals, admission,
recoverable rate limiting and WebSocket close codes.

`application::session` runs one actor per WebSocket (api-guidelines.md section 3b);
`interface::ws` supplies the axum socket halves and the protobuf codec through the
`FrameSource`, `FrameSink` and `SessionCodec` traits, so the session logic has no transport or
wire dependency. `SessionRegistry` records which session owns each player entity and queues
lifecycle commands under one lock, so the zone sees admissions and departures in the order the
registry decided them. The registry is keyed by player `EntityId`: a newer generation replaces
that entity's current socket. Ticket generations are per account; this is not a live,
account-wide eviction across different characters. `ZoneBootstrap` starts the fixture zone;
`ZoneRegistry` retains its handle, and `start_realtime` currently binds sessions to
`zones.fixture()`. Restart opens a new epoch.

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

0. **Admission.** Follow the [login sequence](../diagrams/login-zone-sequence.html);
   the [session state machine](../diagrams/session-lifecycle.html) covers refusal and closure.
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
   session per tick) and applied by `ZoneState::run_tick`:
   authorised against the session's generation, checked against bounds and the 64-tile limit,
   then the movement phase takes the first step and the AOI diff runs. The player's output
   for the tick is `Accepted{seq, tick}` (or `Rejected`) followed by its `EntityMove`s. The
   actor records `zone.apply` under the carried span, awaits `TickGate::admit` with the
   finished record, then broadcasts the `AppliedTick` after the ack.
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
