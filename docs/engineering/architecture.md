# Architecture

## 1. Layers

The [System architecture](../diagrams/system-architecture.html) diagram shows the client, the server and the
external services, with the development ports. The [Module dependencies](../diagrams/module-dependencies.html) diagram
shows the four layers and their real imports.

Dependencies must point inward: interface and infrastructure depend on application and
domain. `main.rs`, `lib.rs` and `zone_runtime.rs` put together the adapters and the servers. `config.rs`
reads the process settings. There is one exception now: `interface::http` and `interface::grpc::auth`
import infrastructure telemetry helpers. The application orchestration uses Tokio and tracing.
The domain stays independent of the runtime, the transport and the database.

The rule comes from Kigawas: thin routers, thick use cases, slim models. If a handler has an
`if` about business state, the handler is in the wrong layer.

### 1.1 What goes where

| Question | Layer |
|----------|-------|
| Is a name valid? | domain (`CharacterName::new`) |
| What occurs when a character is made? | application (`CreateCharacter::execute`) |
| How is a character stored? | infrastructure (`PgCharacterRepository`) |
| Which gRPC code does "name taken" map to? | interface (`to_status`) |
| What stats does a Dwarf start with? | domain, from data files after Phase 1 merges |

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
- `SessionAudit`: `record_in(session, seq, frame)`, `record_out(session, frame)`. It never blocks.
  (This is the audit log for each session of the real-time channel.)
- `Clock`: `now()`.
- `EventLog`, `ZoneSnapshotStore`: the zone replay log and its Postgres epoch index (§2.5).

The composition root selects the adapters and injects them as `Arc<dyn Port>` through
`Dependencies`. Tests build `Dependencies` with in-memory adapters. `main` builds it from
`DATABASE_URL` and `NATS_URL`. When PostgreSQL is configured, a `NATS_URL` that is not empty is
strictly necessary. If `NATS_URL` is absent, empty, or whitespace-only, the startup fails before epoch recovery,
baseline initialization, actor creation, or admission. When persistence is configured, there is no volatile log fallback.
A fully ephemeral in-memory composition (the two variables not set) is still
supported for development. It gives an explicit warning that data is lost on exit, and it makes no in-memory claims about replay durability.

## 2. The event-bus core

The current runtime has two different paths. The
[system diagram](../diagrams/system-architecture.html) shows them:

- The transactional character creation
  puts its domain event in the Postgres outbox.
- World commands go into the zone actor through a bounded
  in-process `mpsc` queue.

NATS JetStream keeps the domain events, the applied ticks and the
session audit frames. A NATS command bus is a possible future distribution option. It is not the current
zone-input path.

### 2.1 Why NATS

- Core NATS gives fan-out pub/sub with subject wildcards. This is exactly the shape of
  "every system that cares about character events subscribes to `nightfall.character.>`".
- JetStream adds persistence and replay when a consumer must have them (projections, audit). The
  publishers do not change.
- NATS is one binary, and it is easy to run locally (`docker compose up -d`). The Rust client is first-party
  (`async-nats`).

### 2.2 Subjects

Domain events use `nightfall.<aggregate>.<event>`: `nightfall.character.created`, `.leveled` and `.died`.
A possible future command-subject convention is `nightfall.cmd.<aggregate>.<command>`. Now, the code publishes no zone commands
there. The domain-event subjects come from `DomainEvent::subject`. Now, the payload is JSON with a `type` tag. When a
consumer that is not Rust occurs, change to protobuf on the bus.

### 2.3 Transactional outbox

The [Persistence data model](../diagrams/data-model.html) diagram shows each current table column,
composite key and declared foreign key. SeaORM adapters implement the repository ports.
The dashed account relations in the diagram are application associations. They are not SQL constraints.

A process can stop unexpectedly after it commits a transaction and before it publishes its event. Thus:

1. The repository inserts the event into `outbox` in the same transaction as the state change.
2. The use case does not publish. The relay (`infrastructure/outbox/relay.rs`) is the only
   publisher of domain events. Thus there is only one path, and no race that publishes two times.
3. The relay polls `outbox WHERE published_at IS NULL ORDER BY id LIMIT 100 FOR UPDATE SKIP
   LOCKED` in a transaction.
   The poll interval is 250 ms. After a failure, the exponential backoff goes up to 5 s. The relay publishes each
   row to the `JetStream` stream `NF_EVENTS` (`nightfall.*.*`) and waits for the ack. It sends
   `Nats-Msg-Id = outbox.id`. Thus, if a crash occurs between the publish and the
   mark, the broker drops the retry. The relay sets `published_at` only after the ack. More than one relay can run at the same time.
   `RelayStats` gives `outbox_pending` and the last publish lag to telemetry.

The delivery is at-least-once, with broker-side dedupe in the duplicate window of the stream.
Consumers still dedupe on the aggregate id of the event and a sequence.

### 2.4 The world simulation

Each zone is one tokio task, the **zone actor** (`application::zone_actor`). This task is responsible for one
`domain::zone::ZoneState`. No other code touches the zone state: there is no `Arc<Mutex<World>>`. The actor receives
in-process commands and makes applied ticks. To move the actor to a different process, a
transport adapter for that boundary is necessary. The simulation is deterministic (refer to `rust-guidelines.md` §7).

The [Deterministic tick and replay](../diagrams/deterministic-tick-replay.html) diagram shows the live
pipeline and the reconstruction path. The actor drafts, runs, waits for the durable log ack, and then
releases the output. If an append fails, the actor holds the same record and prevents the next draft or snapshot.

- **Inputs.** `ZoneInput { source, seq, command }`. `source` is `System` (admission, eviction,
  NPCs) or `Session { entity, generation }`. A session can only steer or remove its own entity,
  and only while its generation is current (`ReplaceSession` fences older sockets). Commands:
  `SpawnPlayer` (with the loaded character state), `SpawnNpc` (id from the zone RNG),
  `Despawn`, `ReplaceSession`, `MoveTo` (in the bounds, a maximum of 64 tiles), `StopMove`, and the
  combat commands below.
- **Handle.** `ZoneHandle::send` never blocks. A full queue returns `ZoneSendError::Full`, and
  the session reports `OVERLOADED`. `send_traced` also carries a `TraceCarrier` (never part of
  the input or the log). Thus the `zone.apply` span of the actor joins the trace of the sender.
  `send_wait` waits until the queue has space. Only lifecycle commands (spawn, replace, despawn) use it. `subscribe()` gives one `Arc<AppliedTick>` for each
  tick that is not idle. `snapshot().await` answers at the next tick boundary, and nothing is deferred. `stats()` is
  a watch of `TickStats { tick, duration_micros, entities, commands_applied, commands_deferred,
  gate_holds }`.
- **Tick.** `IntervalTicks` gives 100 ms beats (missed beats run one after the other).
  `manual_ticks()` operates the ticks in tests. Drafts take the inputs in the order they are received, a maximum of 8 for each session,
  with consecutive ordinals. The remaining inputs wait. `OpenGate` is the test gate, and
  `DurableTickGate` waits for JetStream (§2.5). The actor logs idle ticks, but it does not broadcast them.
- **Output.** `AppliedTick { epoch, tick, server_time_ms, commands, dispositions, events,
  outputs, state_digest }`. `commands` (with ordinals and sources) is the replay log's unit. `dispositions`
  records each refused command. `events` are the zone-wide facts. `outputs` is the ordered stream of each player:

  1. The responses to its own commands, in ordinal order: `Accepted` for each applied command that
     has a `seq`, and `Rejected` for each refused command.
  2. AOI despawns, spawns and moves, each in entity-id order.
  3. The combat facts that the player can see, in event order.

  A session sends exactly this stream. The AOI is the 3x3 block of 32-tile cells. At each tick, the actor compares the AOI
  with the data that the player knows. `state_digest` is the SHA-256 of the
  canonical state at the end of the tick (entities, hate, RNG, counters).
- **Snapshot.** `ZoneSnapshot` (current: schema 9, record 7, binary V5) holds these items:
  - The full entity state, which includes the combat and AI blocks.
  - The RNG state, the next ordinal, the AOI index, the hate ledgers, the spawn slots and
    the respawn scheduler.
  - The safe point and the stat rules.
  - `time_origin_ms` and the provenance (`schema_version`, `build_id`, `config_hash`, `rules_hash`,
    `first_log_seq`).
  - The application checkpoint lanes, progression, skills, effects, cooldowns,
    and PvP state.

  Current snapshots use the compressed `NFSNAP9\0` envelope. The replay can still read the historical snapshot
  schemas 4..8 (records 3..6, JsonV1/BinaryV2/BinaryV3/BinaryV4) under
  their historical rules. The replay refuses earlier versions (schemas 1–3).
  `ZoneState::from_snapshot` validates the snapshot. If you replay the logged drafts from the snapshot, you get the same
  `AppliedTick`s again.
- **Wire.** `interface::zone_mapping` converts to and from `nightfall.v1` world.proto, with
  one function for each message (`spawn_to_pb`, `move_to_pb`, `despawn_to_pb`, `disposition_to_pb`,
  `observer_output_to_pb`). It is the only place where zone values become floats. A spawn of a moving
  entity becomes `EntitySpawn` and then `EntityMove`. The actor sends a stopped entity with
  destination and speed zero. Domain reasons that do not have their own wire value map to `INVALID`,
  with the domain reason in `detail`.

#### Combat

Phase 1 E2.2–E2.6. Flow: [combat sequence](../diagrams/combat-sequence.html). Code:
`domain::zone::{combat, state_combat}`, on top of the stat engine (§2.7).

- **State.** `Entity.combat: Option<CombatState>` has these items: the role (player class and XP, or NPC template
  and XP reward), `StatSheet`, HP/MP, the reach and the body radius, the life `incarnation`, `auto_attack`,
  `chasing`, the in-flight `Swing { target, target_incarnation, start, impact, ready }`,
  `ready_at`, and `protected_until`. `ZoneState::with_safe_point` sets the town respawn point.
  `ZoneState.hate` has an ordered `HateLedger` of
  `{ hate, damage }` for each NPC. The rules come with `ZoneState::with_rules` (bootstrap) or with the snapshot.
- **Spawning.** `SpawnPlayer.load` = `PlayerLoad { class, level, xp, hp, mp, alive }` (`alive:
  false` spawns dead).
  Thus the death stays after a reconnect. The load is resolved
  against the rules with the starter weapon (`InvalidLoad` if they do not agree). `SpawnNpc.combat`
  = `NpcCombat`, resolved one time at bootstrap from the template.
  The accuracy, evasion and crit come from the HF monster base stats. The reach is in milli-tiles. The zone spawns the spawn-slot monsters
  itself (refer to NPC AI below). `SpawnNpc` does not spawn them.
- **Commands.** `SetTarget` (E2.1 checks). These commands stop the attack: a change or a clear of the target, `MoveTo` and `StopMove`.
  `Attack` must have a live attackable target in the AOI, and is idempotent.
  `StopAttack` cancels the swing. The two commands keep the selection. `AddAggro { npc, target }` (system,
  for the E3.2 AI) adds 1 hate. `Respawn` is only for dead players. A living actor gets `NotDead`
  (wire `INVALID`). Dead actors get `DEAD_ACTOR` for all other intents. No command carries damage.
- **Tick.** The tick has these phases, in this order:
  1. Commands.
  2. Spawn and AI phases (below).
  3. Chase (out of reach: go to the target).
  4. Movement.
  5. Impacts by attacker id (with death, XP and level consequences).
  6. Next swings by attacker id.
  7. Progression facts.
  8. AOI output.

  A swing starts in reach after
  `ready_at` is in the past. The impact and the next swing come from `attack_timing` (Squire's Sword at
  DEX 30: +6/+12). An impact examines the attacker, the target life and the reach again. If one of them is not correct, the
  result is `AttackCancelled` and there is no draw. If they are correct, the impact draws hit. On a hit, it draws crit and spread, applies the damage, and emits
  `AttackResult` and the `StatsChanged` of the target player.
- **Hate and death.** A hit on an NPC adds `F(d*100/(L+7))`. A miss or zero adds 1. The hate has a cap. The NPC
  targets the entity with the most hate (if two are equal, it keeps the current target, then the lowest id) and auto-attacks it. HP 0 emits
  `EntityDied` one time for each life. It cancels the swing of the victim, clears the target of each attacker and
  removes the victim from all ledgers. `Despawn` does the same. Refer to NPC AI below for the NPC corpse and respawn.
- **Death and respawn (E2.4).** One time for each life. A player pays the HF loss
  `R((X[L+1]−X[L])*loss[L])` and loses levels by a threshold search. The stats are calculated again, MP is clamped, and the
  owner gets `StatsChanged`. `Respawn` moves a dead player to the safe point
  (`ZoneState::with_safe_point`): `HP = max(1, F(maxHP*65/100))`, MP 0, a new incarnation, and
  6000-tick protection that an accepted `Attack` stops. The actor emits `EntityRespawned` + `StatsChanged`. There is no
  XP refund.
- **XP and level (E2.6).** At the death of an NPC, its XP goes one time to the living player.
  This player is the player whose validated attack made the kill. Pending swings that the death cancels get no
  credit. Historical damage never moves XP to a different player. `XpGained` (with a cap at `X[86]−1`), one
  `LevelUp` for each level, and one `StatsChanged`. HP/MP stay the same, but are clamped to the new maximum values.
- **Progression facts.** At the end of the tick, `AppliedTick::progression()` gives one `ZoneEvent::Progression(ProgressionDelta)` for each player
  whose XP, level or life changed.
  The fact has the end values, `levels_gained`, `died` and `respawned`. The actor records these facts, but never sends them. The checkpoint lane keeps them (§2.8).
- **Visibility.** Only the owner gets: `StatsChanged`, `XpGained`, `LevelUp`, `TargetChanged`.
  `AttackStarted`/`AttackResult`/`AttackCancelled` go to the observers that know the two sides.
  `EntityDied` goes to the observers that know the entity. `HateChanged` and `Progression` go to no observer.
  `StatsChanged` carries XP (owner-only). `EntityRespawned` carries the new incarnation. All of these items are still in
  `AppliedTick.events` and the record. `EntitySpawn` carries the public combat state (life, HP,
  level, pending swing). Thus AOI entry and `ReplaceSession` make the full picture again.

#### NPC AI

Phase 1 E3.2–E3.4. Flow: [NPC state machine](../diagrams/npc-state-machine.html). Code:
`domain::zone::{ai, state_ai}`. The constants follow L2J `L2AttackableAI` at 100 ms ticks.

- **State.** `Entity.ai: Option<NpcAi>` is only on spawn-slot NPCs. It has the slot member, the home,
  `Intention` (`Idle`, `Active`, `Attack`, `ReturnHome`, `Dead`), `last_hit`, `called_help`, and
  `corpse_until`. `SpawnNpc` NPCs keep the basic E2.5 behaviour.
- **Spawn slots.** `slot_specs` resolves each zone slot one time (`NpcCombat` + `NpcBrain`: aggro,
  clan, help range, leash, corpse decay; respawn delay/random). The bootstrap installs them with
  `ZoneState::with_spawn_slots`. The scheduler keeps one `MemberState { entity, incarnation,
  respawn_at }` for each member.
  The slots and the scheduler are in the snapshot. The members are in the state digest.
- **Tick.** The tick has these phases, in this order:
  1. Commands.
  2. Spawn phase: expired corpses `Despawn`, then the due members respawn in
     (slot, member) order.
  3. AI phase (NPCs in id order).
  4. Chase, movement, impacts.
  5. Hit bookkeeping.
  6. Next swings, progression facts, AOI output.
- **Think.** The NPC thinks each 10 ticks, at phase `EntityId % 10`. `Idle` changes to `Active` when a living player is
  in the AOI of the NPC. It changes back when no player is there. In `Active`, an aggressive NPC gives 1 hate to the nearest eligible player in
  `aggro_range` (equal distance → lowest id). If the NPC gets no player this way, a mobile NPC wanders on
  `roll_below(30) == 0`. It goes to home ± `MAX_DRIFT_RANGE` (300 L2 units = 9375 milli-tiles) on each axis,
  clamped to the bounds. `Attack` selects the target again (if two are equal, it keeps the current target).
- **Clan help.** When an NPC gets a target in `Idle`/`Active` (aggro, a hit from a player, a call), it goes into
  `Attack`. The first engagement calls the idle/active NPCs of the same clan in the
  `clan_help_range` of the caller, in id order, with 1 hate each. Helpers never call (no recursion).
- **Leash and timeout.** In `Attack`, the NPC examines these conditions at each tick:
  - No target.
  - A distance from home that is more than `leash_radius`.
  - 1200 ticks since it went into `Attack` or since the last hit that landed.

  If one condition is true, the NPC goes to
  `ReturnHome`. Its hate, target and swing are cleared. It cannot be attacked (`SetTarget`
  `NON_ATTACKABLE_TARGET`, `AddAggro` `NotPermitted`). It walks home with the integer movement. When it
  arrives exactly at home, it has full HP/MP, it can be attacked, and it is `Active`. The overshoot is a maximum of one step.
- **Death and respawn.** `Dead` with `corpse_until = death + corpse_decay_ticks`. The jitter
  `0..=random` is drawn one time in the death consequences (no draw for 0). `respawn_at =
  max(death + 10·(delay + jitter), corpse_until + 1)`.
  The decayed corpse goes out of the zone. Thus
  each reference is `UNKNOWN_ENTITY` until the member respawns with the same `EntityId`,
  `incarnation + 1`, full stats, and `Idle` at home (`EntitySpawn`, `EntityRespawned`). The zone never spawns a member again
  while its entity is still in the zone.
- **RNG.** A first life draws its id. A wander draws `roll_below(30)`, then `dx`, `dy`. A death draws
  the jitter. Order: spawn slots, then AI by NPC id, then impacts by attacker id.
- **Replay and visibility.** Every intention change emits `NpcIntentionChanged { tick, entity,
  from, to }`: in `AppliedTick.events` and the record (output tag 16), never sent to clients.
  No wire event tells about `ReturnHome` or its heal. Observers see the cancelled swings. They see the
  restored HP only in a later `EntitySpawn` or `AttackResult` (a gap for E5.4/E6.2).
- **Hooks for E2.4.** `kill` calls `npc_died` (corpse deadline, jitter, schedule), and
  `reselect` calls `ai_target_changed`. The death-consequence work keeps the two calls.

### 2.5 Replay log

Stories 3.2 and 3.4 (plan §8 #2-#6). The zone actor is the **single writer** of the
replay log of its zone. The logs for each session are only for audit. Code: `application::replay_log` (records, codec,
`EventLog` port, `DurableTickGate`, `SessionAuditWriter`, `open_epoch`),
`application::zone_bootstrap`, `infrastructure::eventlog` (`JetStream`, memory),
`infrastructure::postgres::PgZoneSnapshotStore`.

**Epoch lifecycle.** There is one epoch for each zone run. A restart is a new epoch. Its number is one more than the
highest number that the log or `zone_snapshots` knows. `ZoneBootstrap::start` does these steps:

1. It loads the zone definition
   (`packages/data/zones/*.toml`, hashed into `config_hash`).
2. It builds the state of tick 0.
3. It writes the
   **snapshot** to the log and its row to `zone_snapshots`.
4. Only then, it builds the gate. The gate must have
   the `EpochStarted` proof that the snapshot write returns.
5. It spawns the actor.

The starting NPCs
come in as `SpawnNpc` commands on the first tick. Thus they are also in the applied log.

**Data.** `infrastructure::{zone_data, npc_data}` parses the TOML under `packages/data/` (for the layout and units, refer to its README)
into typed domain values (`NpcTemplate`, `SpawnSlot`, no
floats). The startup fully validates the data, and `DataHash` hashes it into `config_hash`.
`RunningZone::shutdown` stops the actor after its current tick and writes the **watermark**
(the last acknowledged tick, the record count, and the reason `shutdown` or `epoch_end`).

```
 nightfall.zone.<zone>.<epoch>.snapshot   ── once, before anything else of the epoch
 nightfall.zone.<zone>.<epoch>.applied    ── one record per tick, ticks contiguous from the
                                             snapshot's tick, idle ticks included
 nightfall.zone.<zone>.<epoch>.watermark  ── once, after the actor stopped
 nightfall.session.<session>.in / .out    ── audit frames (NF_SESSIONS), best effort
```

**Streams.** `NF_ZONES` (`nightfall.zone.*.*.*`) and `NF_SESSIONS` (`nightfall.session.*.*`) use
file storage and `max_age` 7 days. The startup makes or updates them. `NF_EVENTS` is
`nightfall.*.*` (only domain events). Thus no two streams overlap. Zone publishes carry
`Nats-Msg-Id` (`<zone>/<epoch>/<tick>` for records; the subject for the snapshot and the watermark). The
broker drops a retry in its 2-minute duplicate window. **Retention:** each message expires
separately after seven days. The original replay snapshot
can expire before later records. A separate `.recovery` snapshot is made again each 23 hours
at an admitted boundary, and on a clean shutdown. Its message ID includes the boundary tick and the payload hash.
Thus a shutdown flush at the same tick cannot deduplicate a different checkpoint state.
`zone_snapshots` holds this latest baseline. `zone_epochs` is a durable discovery index that is not pruned.
The code inserts its rows in the same transaction. It records the start, the first sequence, the checkpoint progress
and the closure independently of the JetStream retention. The zone writes `last_recorded_tick` after
the log acknowledges the tick and before its checkpoint or output (D134). Thus
`last_recorded_tick = T` means that record T was acknowledged. Each output, RPC reply,
checkpoint, outbox row, receipt and `last_checkpointed_tick` advance is at or below it.
Recovery requires the history through `last_recorded_tick`. A shorter history is log loss and
fails closed. A crash after the acknowledgement and before the index write leaves the index at
T-1. Recovery then replays record T from the log and settles its checkpoint. One process owns a
zone at a time. The index and the applied publish do not fence a second, stale owner.

**Record.** `AppliedTickRecord { zone, epoch, tick, server_time_ms, commands (ordinal, source,
seq, command), dispositions, outputs, output_form }`, where `outputs` is each player's ordered
output, encoded with `encode_outputs` (or its digest, below), in entity-id order. Replay runs `commands` again from the snapshot
and compares the encoded record again **byte for byte**. Encoding: protobuf through
`prost` messages that are derived manually (the schema is in `replay_log/codec.rs`). We did not use serde with bincode, because formats that do not describe themselves cannot decode the
internally tagged serde enums of the zone.

Current
Phase 3 snapshots use the compressed `NFSNAP9\0` envelope:

- An 8-byte magic `NFSNAP9\0`.
- A little-endian u32 expanded length.
- zlib level 9 compression.

The envelope has a versioned protobuf body with
immutable rule registries (deduplicated by content), typed sidecars, and shared effect headers. The historical
snapshots 4–6 used uncompressed canonical JSON (`serde_json`). Schemas 7 and 8 put complete JSON in
versioned zlib envelopes (`NFSNAP7\0`, `NFSNAP8\0`). The watermark is canonical JSON.
`zone_snapshots.snapshot` holds the same bytes as the log message. It also holds
`jetstream_snapshot_seq`, `jetstream_first_seq` (filled after the first record is acked),
`time_origin_ms`, `build_id`, `config_hash` and `schema_version`.

**State digest versions (E6.5).** Current Phase 3 epochs use **Binary V5** (record schema 7,
snapshot schema 9). It covers skills, active effects, cooldowns, regeneration, and PvP state, with
an explicit little-endian binary layout (`state_digest.rs`). Snapshot schema 6 added
`digest_version`. If it is absent in the legacy schemas 4/5, it means JSON v1. The code keeps the historical record and digest versions
for byte-exact replay:

- Record schema 3 / snapshot schemas 4–5: JsonV1 (canonical JSON digest)
- Record schema 4 / snapshot schema 6: Binary V2 (little-endian layout of ordered entities, hate, spawn, counters, RNG)
- Record schema 5 / snapshot schema 7: Binary V3 (Phase 2 identity, class ledger, transfer receipts)
- Record schema 6 / snapshot schema 8: Binary V4 (once-ever token supply policy)
- Record schema 7 / snapshot schema 9: Binary V5 (current: Phase 3 combat, skills, effects, regen, PvP)

Replay refuses a snapshot/record digest-version mismatch. On restore, legacy epochs keep their historical digest
algorithms. The output bytes do not change, and the historical formats replay byte-exact.

**Record size bound.** A record becomes larger with players × AOI population (≈ 48 KB for 50 players
that each see 20 moves). At approximately a thousand entities, it goes above the NATS `max_payload` (1 MiB by
default, payload plus headers). An append that can never fit stalls, and then pauses
the zone permanently. Thus, before the first attempt, the gate calls `AppliedTickRecord::bounded`
with `EventLog::max_record_bytes()`. For JetStream, this is the `max_payload` that the server last announced,
minus 1 KiB for headers. If the encoding is larger, the gate replaces the output of each player with its
**SHA-256**. The record then has `output_form = SHA256` (protobuf field 8; the digests for each player are
in `PlayerOutput.sha256`, field 3).

The record always stores the full commands and dispositions. Thus
replay still runs the tick again. For this type of record, `AppliedTickRecord::reproduced_by` makes digests of the outputs of the new run
and compares the encodings. Thus, if one output byte changes, this is still a
divergence. We lose only one function: you cannot *show* the outputs of a digested tick from the log alone
(to see them, run the tick again). Each digested record counts in `eventlog_digested_records_total`. Records with full outputs
encode exactly as before (field 8 is not included at its default), and old records decode as
`ENCODED`.

The digested record can also be larger than the limit. In that record, only commands and dispositions are
left, and the 1024-input queue limits them. If this occurs, the gate logs an error and the append fails as a usual failure. We did not
divide a record across more than one message. That method changes one acked append
into many appends, and makes dedupe, `epoch_status` and `read_epoch` more complex. Also, the full outputs of a tick
of that size do not have a value that is more than the stall risk.

**Durability rules.** For the order and the failure procedures, follow the [tick and replay diagram](../diagrams/deterministic-tick-replay.html).
Each applied tick record, idle ticks included, must be
acknowledged before the actor releases its outputs or drafts the next tick. **Never sample
applied records.** A clean shutdown writes a completion watermark after the actor stops.
If the shutdown interrupts a stalled gate, the watermark gives only acknowledged ticks.
An epoch without a completion watermark (a crash, a failure to stop, or a failed watermark write)
is not complete and is **not replayable**. The audit for each session is separate and best effort (below).

**Replay** (the tool below, not the automatic recovery at restart) opens an epoch with `open_epoch`.
This function refuses an epoch without a watermark (`EpochStatus::Incomplete`, for example after a crash) or without
a snapshot (`Missing`). While it streams, it makes sure that the ticks are contiguous from the snapshot to
the watermark (`Gap`, `Truncated`).

**Audit.** `SessionAuditWriter::record_in / record_out` never block. Frames go to a bounded
buffer (16 Ki), and the writer drains it in pipelined batches. If the buffer is full or a frame is not acknowledged, the writer
drops the frame and counts it in `eventlog_audit_dropped_total` (alert `nf-audit-dropped`). Replay never
depends on these frames.

`infrastructure::eventlog::EventLogSessionAudit` implements the socket `SessionAudit` port
with that writer. The `CheckpointAudit` that exists wraps it. The composition root waits for
`JetStreamEventLog::connect` (this includes the `NF_SESSIONS` creation) before it starts a writer
or accepts sockets. Without `NATS_URL`, `DiscardSessionAudit` discards game frames with an
explicit warning, and does not start an audit worker. The audit key is the new socket UUID.
It is not the character/player UUID. The `.in`/`.out` subjects keep its compact (32 hex digit)
representation. The subjects and the protobuf records do not change: raw inbound bytes are field 7,
and raw outbound bytes are field 5.

The inbound receive time is sampled on the calling path. Sequences that the writer cannot decode use zero in the
uint64 field that exists. The zone/epoch/next-tick metadata goes with the
inbound frames. Outbound records use the tick that made them (the latest zone tick for a
rejection that is local to the session). Authentication stays at the HTTP upgrade/gRPC edge, and never goes into this game-frame
audit. The shutdown closes the sessions, and then flushes the buffered audit frames. If the broker does not complete the
acknowledgements in five seconds, the shutdown stops the worker with a warning. The remaining buffered
frames can be lost. `eventlog_audit_dropped_total` counts only drops from a full queue and
appends that are not acknowledged. It does not count frames that an aborted worker leaves. The `.checkpoint` subjects
and the JSON data do not change.

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

`nightfall-replay trace --file run.nfr [--session <entity-id>] [--fail-tick N] --out trace.html`
(E6.4) writes a self-contained offline page. The page uses the diagram palette and local font fallbacks. It has:

- A zone grid with the snapshot bounds.
- Sampled entity paths with tick dots.
- Glyphs for hit/miss/crit (not sampled), death, respawn, target, intention and rejection.
- Range controls and a tick scrubber.

The failure
tick stays pinned. A table of decoded commands, zone facts and observer outputs covers its ±20-tick
window (the first tick if the failure tick is not given). The table highlights the outputs of the selected session. Sampling
keeps the event ticks and the table window. The tests make sure that each of the two committed fixtures is less than 2 MB (and
thus less than the 5 MB fight-fixture ceiling). The current engine explicitly reconstructs the digest-only map facts.
Digest-only observer messages stay labelled as unavailable. The trace generation
validates that the recording is complete, but it is not a replay verification. `run-sim.sh` makes
`<recording>.trace.html` also with `--no-replay`. It adds a relative JUnit artifact reference and
pins a replay divergence when one is reported. For bot failures without a recorded tick, you can make the trace again
with `--fail-tick`.

| Exit | Meaning |
|---|---|
| 0 | All ticks matched. Prints ticks, players, bytes compared, time. |
| 1 | Divergence: tick, ordinals, session, decoded recorded output compared with produced output. |
| 2 | Usage, I/O, missing epoch, gap or truncated log. |
| 3 | The epoch is not complete (no watermark). `export` also refuses it. |

`--session` takes the entity id of the player (its character id), and selects the player whose outputs are
compared. The replay still applies all commands. `.nfr` = `NFREPLAY` magic, `u32` format version, and a zlib
protobuf of the snapshot, the records and the watermark (`recording.rs`). `moon run api:replay-check`
replays the two fixtures:

- `apps/api/fixtures/sessions/two-players-v4.nfr` (movement, rejection,
  `StopMove`).
- `two-players-fight-v2.nfr` (target/chase, seeded hit/miss/crit, social
  aggro/leash, kill/XP/level, corpse decay/NPC respawn, player death/delevel/protected respawn,
  `StopAttack`, mid-fight disconnect).

CI also runs these tests: epoch/mid-fight byte/digest replay, codec
round-trips, coefficient/RNG/AI/off-AOI mutations, and refusal of incomplete epochs and gaps. The historical
fixtures keep snapshot schema 4, record schema 3 and `.nfr` format 1. Current Phase 3
snapshots use schema 9 (record schema 7, binary V5) in the compressed `NFSNAP9\0` envelope.
They keep the checkpoint lanes (revision, cadence, dirty/fenced state, the exact pending requests
and events), the class states, the skill reuse, and the active effects. The replay can still read the historical snapshot schemas 4..8
under their historical rules (schema 4 with empty lanes; the replay refuses schemas 1–3).
The persistence recovery refuses legacy mid-fight snapshots that have players but no lanes.
The checkpoint metadata does not go into the simulation digest. The historical record schemas (3..6) and the fixtures
do not change.

E2.2 retired movement v1, E3.4 retired v2, and E2.4/E2.6 retired v3. The replay refuses older incompatible schemas. It does not migrate them.
The v4 movement fixture that remains is kept without changes. Record the fixtures again only for intended
behaviour changes:

- Movement: use `cargo run -p nightfall-api --example record_session`
  against a dev-token API. Then do a graceful shutdown and `nightfall-replay export`.
- Fight: use `bash apps/api/fixtures/sessions/record-fight.sh` with the compose services up.

The fight script
builds this clone and starts a new zone 6102 epoch on ports 3107/50107. It operates real gRPC/WS,
stops gracefully, exports, verifies and examines the coverage. If it is necessary to observe all three seeded attack outcomes, the script records more combat lives
(a maximum of 12 lives). It copies `test_zone.toml` and the Keltir template
to a temporary directory. It changes only the zone ID and the XP reward 28→68 (X[2]). Thus one kill
gives A a level, and the 29-XP death loss after it removes that level from A. The shipped balance data does not change.

**Simulation scripts (Phase 1a E3.8/E3.9).** From the repository root, build one time.
Then use these stable commands for each scenario. `SESSION_ENTITY` is the
character/entity UUID of the player (the own entity id of the failure bundle). It is **not** the WebSocket audit
connection UUID. You must give the zone and the epoch. For an isolated stack, `--latest` can replace `--epoch`.
Export validates that the selected entity occurs in the capture.
But it keeps the commands and outputs of all players, because a filter on the zone would break the replay.

```bash
cargo build -p nightfall-api --bin nightfall-replay
REPLAY=target/debug/nightfall-replay
"$REPLAY" export --zone "$ZONE_ID" --epoch "$EPOCH" --session "$SESSION_ENTITY" \
  --live --out "$ARTIFACTS/session.nfr"   # reads NATS_URL; or pass --nats URL
"$REPLAY" check --file "$ARTIFACTS/session.nfr" --out "$ARTIFACTS/divergence"
"$REPLAY" coverage --file "$ARTIFACTS/session.nfr" \
  --out "$ARTIFACTS/coverage.transitions.json"
```

`check` is an alias of `verify` (also the default command), with the exit codes above. `--file`
implies `--source file`. To compare all players, run the check without `--session`.
Use shell `set -e` or propagate its status, so that a divergence makes the sim job fail. `--live` is
only for export. It captures the finite durable prefix that is there when the log reader opens.
It examines the contiguous ticks and the zone/epoch identity, and writes a **local** watermark with
`reason: "capture"`. It never writes a completion watermark to JetStream, and never stops the
server. Before you capture, wait for the final acknowledgement of the scenario. Without `--live`,
export still must have a graceful epoch completion. If not, it exits 3. A live capture
proves only its recorded prefix. It does not prove that the scenario or the epoch is complete.

**Transition coverage JSON, schema version 1.** Coverage prints a table. The table includes:

- Each pair (25 NPC pairs, 9 player pairs).
- The reachable covered/total.
- The pairs that were never seen.
- The pairs that are not reachable under the current rules.

Coverage writes JSON to `--out`. The default is
`coverage.transitions.json` in the current directory. Zero coverage and replay divergence
do not make coverage fail. Input/usage that is not valid, or a file that it cannot read or write, gives exit 2.
The verification stays the separate divergence gate. The fixed top-level shape is:

```json
{
  "schema_version": 1,
  "npc_intentions": [{"from": "Idle", "to": "Active", "reachable": true, "count": 0}],
  "player_attack_states": [{"from": "idle", "to": "pending", "reachable": true, "count": 0}],
  "life_incarnations": {"npc:1->2": 0, "player:1->2": 0},
  "deaths": {"npc": 0, "player": 0},
  "respawns": {"npc": 0, "player": 0},
  "intent_rejected": {"OutOfBounds": 0}
}
```

The example shows only part of the two pair arrays. Each array has the full Cartesian product
in the state order `Idle, Active, Attack, ReturnHome, Dead` / `idle, pending, active`.
This includes self-pairs (not reachable: a state that does not change is not a transition). Counts are
unsigned 64-bit integers. NPC reachability follows `domain/zone/state_ai.rs`: Idle→Active,
Active→Idle, Idle/Active→Attack, Attack→ReturnHome, ReturnHome→Active,
Idle/Active/Attack→Dead and Dead→Idle. There is no coverage threshold.

NPC transitions and deaths/respawns count zone events one time. They do not count one time for each AOI observer.
Coverage monitors incarnations for each entity from the initial snapshot, across despawn/respawn. This includes decayed NPC
members. The first observation of an entity sets a baseline. Repeated spawn and
respawn facts for the same life count one time. The `life_incarnations` map aggregates the observed
changes as `<npc|player>:<old>-><new>`. The death and respawn maps always have the two kinds.
`intent_rejected` counts each recorded disposition one time, with the domain `RejectReason`
variant name (for example `OutOfBounds`). It does not count rejections in the transport only, because they are not in `.nfr`.
These two maps with variable keys have only the observed keys. An absent key means zero.

The player states show the **server-observable request lifecycle**:

- idle = auto-attack off.
- pending = an Attack request that waits for its recorded disposition.
- active = accepted auto-attack (this includes chase, swing and cooldown).

Coverage counts Idle→pending→active or idle→pending→idle
in the same tick. A repeated Attack while active has no effect. These items stop the active state: a successful movement,
StopMove, StopAttack, target changes and despawn. The replayed combat state at the end of the tick
gives the automatic stop on death/target loss. The initial snapshot
is a baseline, not a transition. This is not a measurement of the local pending
time of the client or of speculative UI transitions. For these, client contract coverage is necessary.

Coverage replays the commands to recover the attack state. Encoded records give the recorded
zone facts, also if the verification shows a divergence. Digest-only records give the replayed facts,
because the original facts cannot be recovered from SHA-256. Run `check` first, before you use
this coverage as evidence. Schema 1 describes this interpretation. If the semantics,
state names or field types change in a way that is not compatible, the version must increase. Sim mergers must make sure that the schema
versions and the pair metadata are equal. They must add the counts by `(from,to)` and map key, and calculate covered/total again
from the merged counts (never add covered/total). Coverage always covers the full recording.
Coverage refuses `--session`, so that it does not report a partial zone without a warning.

### 2.6 Sessions and zones

The [Device login → zone admission](../diagrams/login-zone-sequence.html) diagram follows the access
token, the single-use play ticket and the upgrade with the ticket in the header.
The [Session lifecycle](../diagrams/session-lifecycle.html) diagram shows HTTP refusals, admission,
recoverable rate limits and WebSocket close codes as separate items.

`application::session` runs one actor for each WebSocket (api-guidelines.md section 3b).
`interface::ws` gives the axum socket halves and the protobuf codec through the
`FrameSource`, `FrameSink` and `SessionCodec` traits. Thus the session logic has no transport or
wire dependency. `SessionRegistry` records which session controls each player entity. It puts
lifecycle commands in the queue under one lock. Thus the zone sees admissions and departures in the order that the
registry set. The registry key is the player `EntityId`: a newer generation replaces
the current socket of that entity. Ticket generations are for each account. This is not a live
eviction across all characters of an account. `ZoneBootstrap` starts the fixture zone.
`ZoneRegistry` keeps its handle, and `start_realtime` now binds sessions to
`zones.fixture()`. A restart opens a new epoch.

### 2.7 Stat engine

The [Stat derivation data flow](../diagrams/stat-derivation-data-flow.html) diagram follows one value from
the pinned L2J source to a damage roll. Phase 1 plan §3.1 is the contract.
[`packages/data/SOURCES.md`](../../packages/data/SOURCES.md) records the source revisions and errata.

- **Data.** `packages/data/scripts/gen_tables.py` (Python
  `decimal`) makes `packages/data/tables/{stat_bonus,formulas,experience,penalties,starter_weapon}.toml`
  and `classes/*.toml` from L2J High Five. Each decimal is stored as its literal source string and as an
  exact integer at `Q = 1_000_000`. Each formula carries its literal source lines.
- **Loading.** `infrastructure::rules_data` parses decimals exactly (TOML floats are type
  errors). It compares the literal with the scaled value, and examines the coverage, the monotone XP, the bounds and the cross-file
  references. Then `domain::zone::StatRules::new` examines the invariants and derives each class at
  each level. If one check fails, the startup stops and lists all errors. The result is
  `ResolvedRules { rules, config_hash }`, injected with `ZoneBootstrap::with_rules`. The hash
  is the SHA-256 of a canonical JSON of the parsed files. The rules are embedded by default. `RULES_DIR`
  overrides them.
- **Calculation.** `domain::zone` uses only integer mathematics: `Scaled` (`i64` at `Q`), checked
  `i128` products divided one time with explicit floor/ceil, and `isqrt`. `StatSheet::for_player`
  derives P.Atk/P.Def, HP/MP, accuracy/evasion, crit and attack speed.
  `StatSheet::from_final` takes the NPC template values without a change. `combat_math` covers hit/crit
  rolls (the caller draws), damage with K = 76, attack timing in ticks, hate and respawn.
  `progression` covers XP, level and death loss. There are no floats (a lint and a source scan make sure of this).

### 2.8 Progression checkpoints (Phase 1 §3.3)

`CheckpointService` reads the live ticks after the durable log gate, before the broadcast. It saves
the changed state of each player each 50 ticks. It saves immediately on death, level change or respawn, and
before it applies disconnect/replacement commands. If a lifecycle command comes after a different command
for that player, it waits for the next tick boundary. The serial lane keeps the exact request
through failures (100 ms–5 s backoff). It applies backpressure to the actor, and does not drop facts.
`checkpoint_lag_seconds` and `checkpoint_failures_total` report on this lane. During a database outage, this design intentionally
stalls the zone. The DB completion time never goes into the simulation rules.

Admission and departure use the same registry lock. Departure waits for the final save and the applied
Despawn before it releases the admission. A new SpawnPlayer carries the committed XP, level, HP, MP,
life, position and revision from `load_for_admission`. Replacements keep the live entity and
its one checkpoint lane. An old repository revision fences all more writes from that lane.
The loaded revision is an optional replay-command field. Old fixtures stay byte-identical.

One repository transaction does these steps:

- It writes the progression and increments its revision.
- It records the idempotency response.
- It stages each CharacterLeveled/CharacterDied fact.

Checkpoint keys hash the character,
zone, epoch and tick. Event IDs add the fact ordinal. The event sequence is the ordered pair
`(committed revision, ordinal)`. Thus it keeps more than one level-up, and the death delevel/XP loss.
Only the outbox relay that exists publishes. Save acknowledgements are separate session audit
records (`nightfall.session.<id>.checkpoint`, JSON). An audit failure cannot undo a committed save.

Before the startup opens the next epoch, it reads each unresolved epoch from `zone_epochs`. For each epoch, it
reconstructs and validates its durable prefix
against its snapshot, digest-only records included. Then it projects the same checkpoint requests.
Known requests replay their stored responses. Missing critical saves and outbox facts commit
one time. The applied log is the durable pending queue. Thus recovery does not have to have a save audit ack
or an epoch watermark. Usual progress after the last checkpoint can roll back. This path
is separate from the replay verification: `open_epoch` still refuses epochs that are not complete, and the
verifier has no persistence port. The actor answers runtime snapshots only after the pending saves
are complete, and these snapshots include the lanes.
Recovery restores those lanes before it replays the records after the baseline. These conditions log an error and refuse the
zone startup/admission: missing snapshots,
gaps, divergence, or a prefix that ends before the indexed attempted tick. If the result of an unacknowledged final publish is not certain, the recovery also fails closed.
Only a completed recovery, or a successful final save and snapshot, closes an index row. Closed
epochs do not have to have stream history for admission. The checkpoint service updates
`last_checkpointed_tick` after all critical saves through that tick are complete.

## 3. Request lifecycle: `CreateCharacter`

0. `AuthLayer` verifies the bearer token and puts the `AccountId` of the caller in the request
   extensions (api-guidelines.md section 3a). Without a token, no handler runs.
1. `interface::grpc` reads the caller from the extensions. It parses `idempotency_key` and
   `race` into typed values. Each failure is `INVALID_ARGUMENT`, and nothing else runs.
2. `CreateCharacter::execute` validates the name (domain), builds the aggregate, and calculates the
   idempotency fingerprint.
3. `PgCharacterRepository::create_idempotent` runs one transaction: it claims
   `(account, create_character, key)`, inserts the character, stages the outbox row, and commits. Unique
   violations map to typed errors.
4. After the commit, the outbox relay publishes `CharacterCreated` to JetStream.
5. The interface maps the result to `nightfall.v1.Character` or to a status code.

Each step has a test:

- The domain constructors (unit).
- The use case with in-memory ports (unit).
- The repository against Postgres (adapter).
- The endpoint through a real socket (integration).

## 3a. Request lifecycle: `MoveTo` over the WebSocket

0. **Admission.** Follow the [login sequence](../diagrams/login-zone-sequence.html).
   The [session state machine](../diagrams/session-lifecycle.html) covers refusal and closure.
1. **Frame in** (session actor). The session actor counts the binary frame (`ws_frames_total{in}`) and
   audits it (`SessionAudit::record_in`). Then it does these steps in this order:
   - Decode with prost (only if the frame is a maximum of
     4096 bytes).
   - `seq` must be more than the last value (if not, close 4400).
   - Token bucket (if not, `RATE_LIMITED`).
   - Size, binary and decode checks (if not, `INVALID`).
   - `zone_mapping` changes
     `MoveToRequest` into `ZoneCommand::MoveTo`. It rounds the float destination to milli-tiles
     (non-finite: `OUT_OF_BOUNDS`).

   Each refusal here is an `IntentRejected` that goes into the queue immediately.
2. **To the zone.** The session opens a root span `ws.frame{seq}`. Its `TraceCarrier` goes with
   `ZoneInput::session(entity, generation, seq, MoveTo)` through `ZoneHandle::send_traced`. A
   full queue is `OVERLOADED`.
3. **Tick** (zone actor, next 100 ms beat). The actor drafts the input with an ordinal (a maximum of 8 for each
   session for each tick). `ZoneState::run_tick` applies it:
   it authorises the input against the generation of the session, and examines it against the bounds and the 64-tile limit.
   Then the movement phase takes the first step, and the AOI diff runs. The output of the player
   for the tick is `Accepted{seq, tick}` (or `Rejected`), followed by its `EntityMove`s. The
   actor records `zone.apply` under the carried span. It waits for `TickGate::admit` with the
   finished record. After the ack, it broadcasts the `AppliedTick`.
4. **Frames out** (each session in range). Each session takes the output of its player from the
   tick and encodes it with `zone_mapping` (`server_time_ms = time_origin_ms + tick * 100`, never a
   clock read). It records `ws.deliver` under the trace of the intent, audits each frame
   (`record_out`) and puts it in the queue of its outbound `mpsc(256)`. If the queue is full, the session drops the frame,
   counts it, and closes 4429.
5. **Socket** (writer task). The writer task writes the frames in queue order and counts them
   (`ws_frames_total{out}`). Later ticks emit one `EntityMove` for each tick while the entity walks,
   and a final one on arrival (destination and speed zero).

Each step has a test: domain (`state_tests.rs`: acks and order), actor (`session_tests.rs`:
admission, 4429, `OVERLOADED`, replacement), wire (`ws_session.rs`, `ws_trace.rs`).

## 4. Adding a feature

1. Domain: add or extend the entity and its invariants. Add unit tests.
2. Proto: add the RPC and the messages. Give mutating RPCs an `idempotency_key`.
3. Application: if it is necessary, add a port method. Then add one use case. Add unit tests with memory adapters.
4. Infrastructure: implement the port in `memory` and `postgres`. Add adapter tests.
5. Interface: connect the RPC to the use case. Add integration tests as `api-guidelines.md` specifies.
6. If the feature emits an event, add a `DomainEvent` variant, and stage it in the outbox.
