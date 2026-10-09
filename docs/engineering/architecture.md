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

- `CharacterRepository`: `get`, `list_by_account`, `create_idempotent`, `load_for_admission`,
  `checkpoint`. Each method is one
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

`nightfall.<aggregate>.<event>` for domain events; `nightfall.character.created`, `.leveled` and `.died`.
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
  `Despawn`, `ReplaceSession`, `MoveTo` (inside bounds, at most 64 tiles), `StopMove`, and the
  combat commands below.
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
  outputs, state_digest }`. `commands` (with ordinals and sources) is the replay log's unit. `dispositions`
  records every refused command. `events` are the zone-wide facts. `outputs` is each player's
  ordered stream: responses to its own commands (`Accepted` for each applied command that
  carries a `seq`, `Rejected` for each refused one, in ordinal order), then AOI despawns,
  spawns and moves, each in entity-id order, then the combat facts it may see in event order.
  A session sends exactly this stream. The AOI is the 3x3 block of 32-tile cells, diffed
  every tick against what the player already knows. `state_digest` is SHA-256 of the
  canonical end-of-tick state (entities, hate, RNG, counters).
- **Snapshot.** `ZoneSnapshot` (schema 5) holds full entity state including combat and AI
  blocks, RNG state, next ordinal, AOI index, hate ledgers, spawn slots and the respawn
  scheduler, the safe point, the stat rules themselves,
  `time_origin_ms` and provenance (`schema_version`, `build_id`, `config_hash`, `rules_hash`,
  `first_log_seq`), plus application checkpoint lanes. Schema 4 remains readable for replay;
  earlier versions are refused. `ZoneState::from_snapshot` validates it. Replaying the logged
  drafts from it reproduces the same `AppliedTick`s.
- **Wire.** `interface::zone_mapping` converts to and from `nightfall.v1` world.proto, with
  one function per message (`spawn_to_pb`, `move_to_pb`, `despawn_to_pb`, `disposition_to_pb`,
  `observer_output_to_pb`). It is the only place zone values become floats. A spawn of a moving
  entity becomes `EntitySpawn` and then `EntityMove`. A stopped entity is sent with
  destination and speed zero. Domain reasons without their own wire value map to `INVALID`,
  with the domain reason in `detail`.

#### Combat

Phase 1 E2.2–E2.6. Flow: [combat sequence](../diagrams/combat-sequence.html). Code:
`domain::zone::{combat, state_combat}` on top of the stat engine (§2.7).

- **State.** `Entity.combat: Option<CombatState>`: role (player class and XP, or NPC template
  and XP reward), `StatSheet`, HP/MP, reach and body radius, life `incarnation`, `auto_attack`,
  `chasing`, the in-flight `Swing { target, target_incarnation, start, impact, ready }`,
  `ready_at`, `protected_until`. `ZoneState::with_safe_point` sets the town respawn point.
  `ZoneState.hate`: per NPC an ordered `HateLedger` of
  `{ hate, damage }`. Rules arrive with `ZoneState::with_rules` (bootstrap) or the snapshot.
- **Spawning.** `SpawnPlayer.load` = `PlayerLoad { class, level, xp, hp, mp, alive }` (`alive:
  false` spawns dead, so death survives reconnect), resolved
  against the rules with the starter weapon (`InvalidLoad` if they disagree). `SpawnNpc.combat`
  = `NpcCombat`, resolved once at bootstrap from the template (accuracy, evasion and crit from
  HF monster base stats; reach in milli-tiles). Spawn-slot monsters are spawned by the zone
  itself (NPC AI below), not by `SpawnNpc`.
- **Commands.** `SetTarget` (E2.1 checks); changing or clearing it, `MoveTo` and `StopMove`
  end the attack. `Attack` needs a live attackable target in AOI and is idempotent;
  `StopAttack` cancels the swing; both keep the selection. `AddAggro { npc, target }` (system,
  for the E3.2 AI) adds 1 hate. `Respawn` (dead players only; a living actor gets `NotDead`,
  wire `INVALID`). Dead actors get `DEAD_ACTOR` for every other intent. No command carries damage.
- **Tick.** Commands → spawn and AI phases (below) → chase (out of reach: head for the target) → movement →
  impacts by attacker id (with death, XP and level consequences) → next swings by attacker id →
  progression facts → AOI output. A swing starts in reach once
  `ready_at` has passed; impact and next swing come from `attack_timing` (Squire's Sword at
  DEX 30: +6/+12). An impact rechecks attacker, target life and reach (`AttackCancelled`
  otherwise, no draw), then draws hit, and on a hit crit and spread, applies damage, emits
  `AttackResult` and the target player's `StatsChanged`.
- **Hate and death.** A hit on an NPC adds `F(d*100/(L+7))`, a miss or zero 1, capped; the NPC
  targets its most hated (ties keep the current, then lowest id) and auto-attacks. HP 0 emits
  `EntityDied` once per life, cancels the victim's swing, clears every attacker's target and
  drops the victim from all ledgers; `Despawn` does the same. NPC corpse and respawn are below.
- **Death and respawn (E2.4).** Once per life. A player pays the HF loss
  `R((X[L+1]−X[L])*loss[L])`, de-levels by threshold search, stats recalculated, MP clamped,
  owner `StatsChanged`. `Respawn` moves a dead player to the safe point
  (`ZoneState::with_safe_point`): `HP = max(1, F(maxHP*65/100))`, MP 0, new incarnation,
  6000-tick protection that an accepted `Attack` ends; `EntityRespawned` + `StatsChanged`; no
  XP refund.
- **XP and level (E2.6).** An NPC's XP is credited once, on its death, to the living player whose
  validated attack landed the killing blow. Pending swings cancelled by the death earn no
  credit; historical damage never reallocates XP. `XpGained` (capped at `X[86]−1`), one
  `LevelUp` per level, one `StatsChanged`; HP/MP kept, clamped to new maxima.
- **Progression facts.** At tick end, one `ZoneEvent::Progression(ProgressionDelta)` per player
  whose XP, level or life changed (end values, `levels_gained`, `died`, `respawned`), via
  `AppliedTick::progression()`; recorded, never sent. The checkpoint lane persists these (§2.8).
- **Visibility.** Owner-only: `StatsChanged`, `XpGained`, `LevelUp`, `TargetChanged`.
  `AttackStarted`/`AttackResult`/`AttackCancelled` reach observers that know both sides;
  `EntityDied` those that know the entity; `HateChanged`, `Progression` nobody.
  `StatsChanged` carries XP (owner-only); `EntityRespawned` the new incarnation. Everything is still in
  `AppliedTick.events` and the record. `EntitySpawn` carries public combat state (life, HP,
  level, pending swing), so AOI entry and `ReplaceSession` rebuild the picture.

#### NPC AI

Phase 1 E3.2–E3.4. Flow: [NPC state machine](../diagrams/npc-state-machine.html). Code:
`domain::zone::{ai, state_ai}`; constants follow L2J `L2AttackableAI` at 100 ms ticks.

- **State.** `Entity.ai: Option<NpcAi>` on spawn-slot NPCs only: slot member, home,
  `Intention` (`Idle`, `Active`, `Attack`, `ReturnHome`, `Dead`), `last_hit`, `called_help`,
  `corpse_until`. `SpawnNpc` NPCs keep the bare E2.5 behaviour.
- **Spawn slots.** `slot_specs` resolves each zone slot once (`NpcCombat` + `NpcBrain`: aggro,
  clan, help range, leash, corpse decay; respawn delay/random); bootstrap installs them with
  `ZoneState::with_spawn_slots`. The scheduler keeps one `MemberState { entity, incarnation,
  respawn_at }` per member. Both are in the snapshot; members are in the state digest.
- **Tick.** Commands → spawn phase (expired corpses `Despawn`, then due members respawn in
  (slot, member) order) → AI phase (NPCs in id order) → chase → movement → impacts → hit
  bookkeeping → next swings → progression facts → AOI output.
- **Think.** Every 10 ticks at phase `EntityId % 10`. `Idle` → `Active` when a living player is
  in the NPC's AOI, back when none. `Active`: aggressive NPCs give the nearest eligible player in
  `aggro_range` 1 hate (equal distance → lowest id); otherwise a mobile NPC wanders on
  `roll_below(30) == 0` to home ± `MAX_DRIFT_RANGE` (300 L2 units = 9375 milli-tiles) per axis,
  clamped to the bounds. `Attack` re-selects (ties keep the current target).
- **Clan help.** Acquiring a target from `Idle`/`Active` (aggro, a player's hit, a call) enters
  `Attack`; the first engagement calls idle/active NPCs of the same clan within the caller's
  `clan_help_range`, in id order, 1 hate each. Helpers never call (no recursion).
- **Leash and timeout.** Checked every tick in `Attack`: no target, farther than
  `leash_radius` from home, or 1200 ticks since entering `Attack` or the last landed hit →
  `ReturnHome`: hate, target and swing cleared, unattackable (`SetTarget`
  `NON_ATTACKABLE_TARGET`, `AddAggro` `NotPermitted`), walks home on the integer movement; on
  exact arrival full HP/MP, attackable, `Active`. Overshoot is at most one step.
- **Death and respawn.** `Dead` with `corpse_until = death + corpse_decay_ticks`; jitter
  `0..=random` drawn once in the death consequences (no draw for 0); `respawn_at =
  max(death + 10·(delay + jitter), corpse_until + 1)`. The decayed corpse leaves the zone, so
  every reference is `UNKNOWN_ENTITY` until the member respawns with the same `EntityId`,
  `incarnation + 1`, full stats, `Idle` at home (`EntitySpawn`, `EntityRespawned`). A member
  whose entity is still in the zone is never spawned again.
- **RNG.** First lives draw their id; wander draws `roll_below(30)` then `dx`, `dy`; death draws
  the jitter. Order: spawn slots, then AI by NPC id, then impacts by attacker id.
- **Replay and visibility.** Every intention change emits `NpcIntentionChanged { tick, entity,
  from, to }`: in `AppliedTick.events` and the record (output tag 16), never sent to clients.
  No wire event announces `ReturnHome` or its heal: observers see cancelled swings, and the
  restored HP only in a later `EntitySpawn` or `AttackResult` (gap for E5.4/E6.2).
- **Hooks for E2.4.** `kill` calls `npc_died` (corpse deadline, jitter, schedule) and
  `reselect` calls `ai_target_changed`; death-consequence work keeps both calls.

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

**Data.** TOML under `packages/data/` (layout and units: its README) is parsed in
`infrastructure::{zone_data, npc_data}` into typed domain values (`NpcTemplate`, `SpawnSlot`, no
floats), fully validated at startup, and hashed with `DataHash` into `config_hash`.
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
the broker drops a retry inside its 2-minute duplicate window. **Retention:** messages expire
individually after seven days. The original replay snapshot
can expire before later records. A separate `.recovery` snapshot is refreshed every 23 hours
at an admitted boundary and on clean shutdown; its message ID includes the boundary tick and payload hash
so a shutdown flush at the same tick cannot deduplicate different checkpoint state.
`zone_snapshots` holds this latest baseline; `zone_epochs` is a durable, unpruned discovery
index inserted in the same transaction. It records start, first sequence, checkpoint progress
and closure independently of JetStream retention. `last_recorded_tick` is a conservative
upper bound written before log admission, so losing a final record cannot hide a pending save.

**Record.** `AppliedTickRecord { zone, epoch, tick, server_time_ms, commands (ordinal, source,
seq, command), dispositions, outputs, output_form }`, where `outputs` is each player's ordered
output encoded with `encode_outputs` (or its digest, below), in entity-id order. Replay re-runs `commands` from the snapshot
and compares the re-encoded record **byte for byte**. Encoding: protobuf through hand-derived
`prost` messages (schema in `replay_log/codec.rs`); serde with bincode was rejected because the
zone's internally tagged serde enums cannot be decoded by non-self-describing formats. The
snapshot and watermark are canonical JSON (readable in an incident);
`zone_snapshots.snapshot` holds the same bytes as the log message, plus
`jetstream_snapshot_seq`, `jetstream_first_seq` (filled once the first record is acked),
`time_origin_ms`, `build_id`, `config_hash` and `schema_version`.

**State digest versions (E6.5).** Snapshot schema 6 stores `digest_version`; absent in
schemas 4/5 means JSON v1. Record schema 3 retains JSON v1, schema 4 selects binary v2;
replay rejects a snapshot/record digest-version mismatch. New epochs use binary v2:
SHA-256 over an explicit little-endian, length-prefixed layout (`state_digest.rs`), with
ordered entities, hate and spawn members, plus counters and RNG. Legacy epochs keep the
JSON algorithm on restore; output bytes are unchanged and snapshots remain JSON.

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

**Durability rules.** Follow the [tick and replay diagram](../diagrams/deterministic-tick-replay.html)
for ordering and failure handling. Every applied tick record, including idle ticks, is
acknowledged before its outputs are released or the next tick is drafted. **Never sample
applied records.** Clean shutdown writes a completion watermark after the actor stops;
the watermark names only acknowledged ticks if shutdown interrupts a stalled gate.
An epoch without a completion watermark (crash, failure to stop, or failed watermark write)
is incomplete and **not replayable**. Per-session audit is separate and best effort (below).

**Replay** (the tool below, not automatic restart recovery) opens an epoch with `open_epoch`,
which refuses one without a watermark (`EpochStatus::Incomplete`, e.g. after a crash) or without
a snapshot (`Missing`), and checks as it streams that ticks are contiguous from the snapshot to
the watermark (`Gap`, `Truncated`).

**Audit.** `SessionAuditWriter::record_in / record_out` never block: frames go to a bounded
buffer (16 Ki) drained in pipelined batches. A full buffer or an unacknowledged frame is
dropped and counted in `eventlog_audit_dropped_total` (alert `nf-audit-dropped`). Replay never
depends on these frames.

#### Replay tool

Story 3.3. Flow: [replay tool diagram](../diagrams/replay-tool.html). Code:
`application::replay` (`verify_epoch`, `TickRunner`, `Divergence`),
`infrastructure::eventlog::Recording` (`.nfr` files), `src/bin/nightfall-replay.rs`.

```bash
nightfall-replay --zone 1 --epoch 3                 # or --latest; reads NATS_URL / --nats
nightfall-replay --source file --file two-players-v4.nfr
nightfall-replay ... --session <entity-id> --out /tmp/div   # verify one player; dump divergence
nightfall-replay export --zone 1 --epoch 3 --out apps/api/fixtures/sessions/two-players-v4.nfr
```

| Exit | Meaning |
|---|---|
| 0 | every tick matched; prints ticks, players, bytes compared, time |
| 1 | divergence: tick, ordinals, session, decoded recorded vs produced output |
| 2 | usage, I/O, missing epoch, gap or truncated log |
| 3 | epoch incomplete (no watermark); `export` refuses it too |

`--session` takes the player's entity id (its character id) and selects whose outputs are
compared; every command is still applied. `.nfr` = `NFREPLAY` magic, `u32` format version, zlib
protobuf of snapshot, records and watermark (`recording.rs`). `moon run api:replay-check`
replays both `apps/api/fixtures/sessions/two-players-v4.nfr` (movement, rejection,
`StopMove`) and `two-players-fight-v2.nfr` (target/chase, seeded hit/miss/crit, social
aggro/leash, kill/XP/level, corpse decay/NPC respawn, player death/delevel/protected respawn,
`StopAttack`, mid-fight disconnect). CI also runs epoch/mid-fight byte/digest replay, codec
round-trips, coefficient/RNG/AI/off-AOI mutations and incomplete/gap refusal tests. Both
fixtures retain snapshot schema 4, record schema 3 and `.nfr` format 1. New snapshots use
schema 5, adding checkpoint lanes (revision, cadence, dirty/fenced state, exact pending request
and events). Schema 4 remains readable for replay, with empty lanes; schemas 1–3 are refused.
Persistence recovery refuses legacy mid-fight snapshots containing players but no lanes.
Checkpoint metadata does not enter the simulation digest; record schema and fixtures are
unchanged. Movement v1 was retired
by E2.2, v2 by E3.4, v3 by E2.4/E2.6; older incompatible schemas are refused rather than migrated,
and the surviving v4 movement fixture is retained unchanged. Re-record only for intended
behaviour changes: movement uses `cargo run -p nightfall-api --example record_session`
against a dev-token API followed by graceful shutdown and `nightfall-replay export`; fight
uses `bash apps/api/fixtures/sessions/record-fight.sh` with compose services up. The latter
builds this clone, starts a fresh zone 6102 epoch on ports 3107/50107, drives real gRPC/WS,
stops gracefully, exports, verifies and checks coverage. Further combat lives are recorded
if needed to observe all three seeded attack outcomes (bounded to 12 lives). It copies `test_zone.toml` and the Keltir template
to a temporary directory, changing only zone ID and XP reward 28→68 (X[2]), so one kill
levels A and the subsequent 29-XP death loss delevels A; shipped balance data is unchanged.

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

### 2.7 Stat engine

[Stat derivation data flow](../diagrams/stat-derivation-data-flow.html) traces one value from
the pinned L2J source to a damage roll. Phase 1 plan §3.1 is the contract;
[`packages/data/SOURCES.md`](../../packages/data/SOURCES.md) records the source revisions and errata.

- **Data.** `packages/data/tables/{stat_bonus,formulas,experience,penalties,starter_weapon}.toml`
  and `classes/*.toml` are generated by `packages/data/scripts/gen_tables.py` (Python
  `decimal`) from L2J High Five. Each decimal is stored as its literal source string and as an
  exact integer at `Q = 1_000_000`; each formula carries its literal source lines.
- **Loading.** `infrastructure::rules_data` parses decimals exactly (TOML floats are type
  errors), checks literal against scaled value, coverage, monotone XP, bounds and cross-file
  references, then `domain::zone::StatRules::new` checks invariants and derives every class at
  every level. Any failure aborts startup with every error listed. The result is
  `ResolvedRules { rules, config_hash }`, injected with `ZoneBootstrap::with_rules`; the hash
  is SHA-256 of a canonical JSON of the parsed files. Embedded by default, `RULES_DIR`
  overrides.
- **Calculation.** `domain::zone` is pure integer math: `Scaled` (`i64` at `Q`), checked
  `i128` products divided once with explicit floor/ceil, `isqrt`. `StatSheet::for_player`
  derives P.Atk/P.Def, HP/MP, accuracy/evasion, crit and attack speed;
  `StatSheet::from_final` takes NPC template values as-is. `combat_math` covers hit/crit
  rolls (the caller draws), damage with K = 76, attack timing in ticks, hate and respawn;
  `progression` covers XP, level and death loss. No floats (lint plus source scan).

### 2.8 Progression checkpoints (Phase 1 §3.3)

`CheckpointService` consumes live ticks after the durable log gate, before broadcast. It saves
changed state every 50 ticks per player, immediately on death, level change or respawn, and
before applying disconnect/replacement commands. A lifecycle command after another command
for that player waits for the next tick boundary. The serial lane retains the exact request
through failures (100 ms–5 s backoff); it backpressures the actor rather than dropping facts.
`checkpoint_lag_seconds` and `checkpoint_failures_total` report this lane. This deliberately
stalls the zone during a database outage; DB completion time never enters simulation rules.

Admission and departure share the registry lock. Departure waits for final save and applied
Despawn before releasing admission; fresh SpawnPlayer carries committed XP, level, HP, MP,
life, position and revision from `load_for_admission`. Replacements retain the live entity and
its single checkpoint lane. A stale repository revision fences further writes from that lane.
The loaded revision is an optional replay-command field; old fixtures remain byte-identical.

One repository transaction writes progression, increments its revision, records the idempotency
response and stages every CharacterLeveled/CharacterDied fact. Checkpoint keys hash character,
zone, epoch and tick; event IDs add the fact ordinal. Event sequence is the ordered pair
`(committed revision, ordinal)`, preserving multiple level-ups and death delevel/XP loss.
Only the existing outbox relay publishes. Save acknowledgements are separate session audit
records (`nightfall.session.<id>.checkpoint`, JSON); audit failure cannot undo a committed save.

Before opening the next epoch, startup reads every unresolved epoch from `zone_epochs` and
reconstructs and validates its durable prefix
against its snapshot, including digest-only records, and projects the same checkpoint requests.
Known requests replay their stored responses; missing critical saves and outbox facts commit
once. The applied log is the durable pending queue, so recovery needs neither a save audit ack
nor an epoch watermark. Ordinary progress after the last checkpoint may roll back. This path
is separate from replay verification: `open_epoch` still refuses incomplete epochs and the
verifier has no persistence port. Runtime snapshots are answered only after pending saves
finish and include the lanes.
Recovery restores those lanes before replaying records after the baseline. Missing snapshots,
gaps, divergence or a prefix ending before the indexed attempted tick log an error and refuse
zone startup/admission. Even uncertainty about an unacknowledged final publish fails closed.
Only completed recovery or a successful final save and snapshot closes an index row; closed
epochs need no stream history for admission. The checkpoint service updates
`last_checkpointed_tick` after all critical saves through that tick complete.

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
