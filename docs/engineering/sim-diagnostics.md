# Simulation diagnostics

Phase 1a E1.6 and E1.7 use `FBotDiagnostics` in the bot runner. Normal client runs do not instantiate the collector. The protobuf bridge runs inside TurboLinkGrpc, which owns the generated protobuf runtime.

## Failure bundle, schema 1

A failed bot run writes `<scenario>.failure.json` beside its JUnit report. The executor retains the first failed step's final evaluated value; writing artifacts never evaluates the predicate again.

| Field | Meaning |
| --- | --- |
| `schema_version` | `1`; consumers must reject unsupported versions. |
| `scenario`, `own_entity_id`, `replay_session` | Scenario name and player entity selector used by replay. |
| `failure` | Source line, step text, predicate, last observed value, actual wait and scenario elapsed seconds. |
| `tick_first`, `tick_last` | First and highest authoritative tick observed by this client; empty before any ticked message. These describe observations, rather than a fabricated server clock at failure. |
| `events` | Last 50 server frames: protobuf message type, decoded fields, arrival monotonic time, actual wire tick when present and nearest preceding observed tick. |
| `projection.own` | HP/maxHP, MP/maxMP, XP, level, death, target, attack state and position. Unknown resources/position are null, with HP/MP/XP known flags. |
| `projection.proxies` | Every cached remote entity, sorted by entity ID, with position, HP and incarnation. |
| `own_position_history` | Positions observed since the last `nf.ClickMove`, with observed tick and arrival time. |

Ticks, XP and counters use decimal strings to preserve uint64 values beyond JavaScript's exact integer range. Messages without a `tick` field have `server_tick: null`; `nearest_preceding_tick` provides explicit correlation and does not claim an exact wire timestamp.

The JUnit failure body names the bundle, entity, last observed tick, failed predicate/value and wait. Properties `failure_bundle` and `failure_tick` let `sim-trace.py` center the trace around the last observed tick. Replay export/check still verifies the complete recording.

Decoded fields come only from `ClientMessage`/`ServerMessage` gameplay protobuf envelopes. Authentication metadata, WebSocket upgrade headers, bearer tokens and play tickets are outside these envelopes and are not captured. No raw authentication traffic is intercepted. Future protocol changes must preserve this separation.

## Runtime contract coverage

Each process writes `<scenario>.coverage.contract.json`, including zero-count entries discovered from compiled protobuf descriptors:

- `intents`: every `ClientMessage.intent` oneof field.
- `payloads`: every `ServerMessage.payload` field.
- `events`: every `WorldEvent.event` field, including events without a client projection callback.
- `reasons`: every `RejectReason` enum value.
- `close_codes`: documented application codes 4400, 4408, 4409 and 4429, plus other codes actually received. Close codes have no protobuf enum.

Keep-alive frames contain no intent, so they contribute to frame totals and their INVALID replies contribute to rejection totals. They do not invent a `keep_alive` proto case. A frame is counted once at the raw socket boundary, before projection conversion or keep-alive filtering. Coverage persists across soak iterations; failure history resets with each iteration.

JUnit properties `contract.<group>.<case>`, `.covered` and `.missing` expose counts and unseen names. Schema 1 JSON includes decimal-string counts and `wire_numbers` from the same compiled descriptors for the server audit decoder.

Aggregate only current headless process reports, excluding aggregate copies and video retries:

```sh
python3 Scripts/sim-contract.py merge --out Saved/Sim/coverage.contract.json \
  --baseline Scenarios/coverage.contract.baseline.json \
  Saved/Sim/example.coverage.contract.json
```

The full-suite gate fails when a case observed on the historical main baseline becomes unseen. Counts may decrease without failing when the case remains present. New descriptor cases appear automatically as unseen. The per-process wrapper validates coverage artifacts without comparing a single scenario against the full-suite baseline.

Explicit baseline promotion follows review of a complete measured suite:

```sh
python3 Scripts/sim-contract.py promote --candidate Saved/Sim/coverage.contract.json \
  --out Scenarios/coverage.contract.baseline.json
```

Compare before promoting. Promotion preserves all historical positive cases; it cannot erase earlier coverage merely because a later candidate omits a scenario. Removing the only Respawn scenario is a regression, tested by `test-sim-contract.py`.

## Server session audit comparison

`sim-contract-audit.py` compares runtime descriptor-backed counts with NF_SESSIONS `.in`/`.out` records. It requires a dedicated fresh stack containing only the measured clients. `--fresh-stack` acknowledges that precondition; the helper cannot prove arbitrary external stack ownership. Checkpoint JSON records are ignored. Missing wire audit records fail clearly.

```sh
python3 Scripts/sim-contract-audit.py --fresh-stack --nats nats://127.0.0.1:4222 \
  --coverage Saved/Sim/coverage.contract.json --out Saved/Sim/contract.audit.json
```

The audit uses append-only `SessionInRecord` and `SessionOutRecord` codecs and compiled descriptor field numbers exported by the client. Close codes are explicitly excluded because the server wire audit has no close record. Outbound audit records describe queue admission before socket delivery: queued frames can outnumber client receipts during disconnect. The report retains those differences and exits nonzero rather than claiming agreement. Reports contain counts and session totals, never raw frames or credentials.

`Nightfall.Bot.Diagnostics` automation covers schema values, the 50-event bound, projection/incarnation values, final predicate evaluation and raw contract counting. `test-sim-contract.py` covers presence regression and historical promotion; `test-sim-contract-audit.py` covers audit decoding, agreement and undelivered tails.
