# Simulation diagnostics

Phase 1a E1.6 and E1.7 use `FBotDiagnostics` in the bot runner. Usual client runs do not make an instance of the collector. The protobuf bridge runs in TurboLinkGrpc. TurboLinkGrpc is responsible for the generated protobuf runtime.

## Failure bundle, schema 1

When a bot run fails, it writes `<scenario>.failure.json` next to its JUnit report. The executor keeps the final evaluated value of the first failed step. When the executor writes the artifacts, it never evaluates the predicate again.

| Field | Meaning |
| --- | --- |
| `schema_version` | `1`. Consumers must refuse versions that they do not support. |
| `scenario`, `own_entity_id`, `replay_session` | The scenario name and the player entity selector that replay uses. |
| `failure` | The source line, the step text, the predicate, the last observed value, the real wait and the elapsed seconds of the scenario. |
| `tick_first`, `tick_last` | The first and the highest authoritative tick that this client observed. These are empty before a message with a tick arrives. They show observations. They are not a server clock at the failure that the client makes up. |
| `events` | The last 50 server frames: the protobuf message type, the decoded fields, the monotonic arrival time, the real wire tick when it is there, and the nearest observed tick before the frame. |
| `projection.own` | HP/maxHP, MP/maxMP, XP, level, death, target, attack state and position. Unknown resources/position are null, with known flags for HP/MP/XP. |
| `projection.proxies` | All cached remote entities, sorted by entity ID, with position, HP and incarnation. |
| `own_position_history` | The positions observed since the last `nf.ClickMove`, with the observed tick and the arrival time. |

Ticks, XP and counters use decimal strings. Thus they keep uint64 values that are larger than the exact integer range of JavaScript. Messages without a `tick` field have `server_tick: null`. `nearest_preceding_tick` gives an explicit correlation, and does not claim an exact wire timestamp.

The JUnit failure body gives the names of the bundle and the entity, the last observed tick, the failed predicate/value and the wait. The properties `failure_bundle` and `failure_tick` let `sim-trace.py` put the center of the trace at the last observed tick. Replay export/check still verifies the complete recording.

The decoded fields come only from the `ClientMessage`/`ServerMessage` gameplay protobuf envelopes. Authentication metadata, WebSocket upgrade headers, bearer tokens and play tickets are outside these envelopes. The collector does not capture them. The collector does not intercept raw authentication traffic. Future protocol changes must keep this separation.

## Runtime contract coverage

Each process writes `<scenario>.coverage.contract.json`. This file includes zero-count entries that the process finds from the compiled protobuf descriptors:

- `intents`: each `ClientMessage.intent` oneof field.
- `payloads`: each `ServerMessage.payload` field.
- `events`: each `WorldEvent.event` field. This includes events that do not have a client projection callback.
- `reasons`: each `RejectReason` enum value.
- `close_codes`: the documented application codes 4400, 4408, 4409 and 4429, and the other codes that the client really received. Close codes do not have a protobuf enum.

Keep-alive frames have no intent. Thus they add to the frame totals, and their INVALID replies add to the rejection totals. They do not make up a `keep_alive` proto case. The collector counts a frame one time at the raw socket boundary, before the projection conversion or the keep-alive filter. Coverage continues across soak iterations. The failure history goes back to the start at each iteration.

The JUnit properties `contract.<group>.<case>`, `.covered` and `.missing` show the counts and the names that the client did not see. The Schema 1 JSON includes decimal-string counts and `wire_numbers` from the same compiled descriptors, for the server audit decoder.

Aggregate only the reports of the current headless processes. Do not include aggregate copies and video retries:

```sh
python3 Scripts/sim-contract.py merge --out Saved/Sim/coverage.contract.json \
  --baseline Scenarios/coverage.contract.baseline.json \
  Saved/Sim/example.coverage.contract.json
```

The full-suite gate fails when a case that the historical main baseline observed is not seen. Counts can decrease, and the gate does not fail if the case is still there. New descriptor cases are automatically shown as unseen. The per-process wrapper validates coverage artifacts. It does not compare one scenario against the full-suite baseline.

An explicit baseline promotion comes after a review of a complete measured suite:

```sh
python3 Scripts/sim-contract.py promote --candidate Saved/Sim/coverage.contract.json \
  --out Scenarios/coverage.contract.baseline.json
```

Compare before you promote. The promotion keeps all historical positive cases. It cannot remove earlier coverage only because a later candidate does not include a scenario. If you remove the only Respawn scenario, this is a regression. `test-sim-contract.py` tests for this regression.

## Server session audit comparison

`sim-contract-audit.py` compares the runtime counts from the descriptors with the NF_SESSIONS `.in`/`.out` records. It must have a dedicated new stack that has only the measured clients. `--fresh-stack` acknowledges that precondition. The helper cannot prove who controls a random external stack. The audit ignores checkpoint JSON records. If wire audit records are missing, the audit fails with a clear message.

```sh
python3 Scripts/sim-contract-audit.py --fresh-stack --nats nats://127.0.0.1:4222 \
  --coverage Saved/Sim/coverage.contract.json --out Saved/Sim/contract.audit.json
```

The audit uses the append-only `SessionInRecord` and `SessionOutRecord` codecs, and the compiled descriptor field numbers that the client exports. The audit explicitly does not include close codes, because the server wire audit has no close record. Outbound audit records show the queue admission before the socket delivery. Thus, during a disconnect, there can be more queued frames than client receipts. The report keeps these differences and exits nonzero. It does not claim agreement. Reports have counts and session totals. They never have raw frames or credentials.

The `Nightfall.Bot.Diagnostics` automation covers the schema values, the 50-event limit, the projection/incarnation values, the final predicate evaluation and the raw contract count. `test-sim-contract.py` covers the presence regression and the historical promotion. `test-sim-contract-audit.py` covers audit decoding, agreement and undelivered tails.

## Initial local validation

The seeded kill failure did the real walk, targeting and kill. Then it intentionally waited for `own_hp == 0` for 0.5 seconds. The first failure kept line 33, observed HP 109, wait 0.50026 seconds and elapsed scenario time 31.558 seconds. Its bundle had these items:

- 50 typed server frames.
- 56 positions since the last click move.
- The own HP/MP/XP/level/target/attack state.
- Four proxy positions/HP/incarnations.

The observed ticks were 405 through 714. JUnit linked the bundle, and the trace had its center on tick 714. Replay matched 1,245 recorded ticks, 288 outputs and 15,842 bytes. The process exited 1, as intended.

The initial historical presence floor comes from five scenarios that passed: death/respawn (77 seconds), target/attack (44), click movement (47), rejected movement (17) and kill (45). Each scenario had replay and trace checks. Together, their observations covered 5/6 intents, 3/3 payloads, 11/12 events, 1/13 rejection reasons and 0/4 documented closes. These are measured observations of a subset. They do not claim that the tests exercised the unseen cases. The baseline promotion records the scenario names and keeps the historical positives. After comparison and review, a later complete suite that passes can increase this floor.

If you remove the only death/respawn report from these real measurements, the gate exits 1 for `intents.respawn` and `events.entity_respawned`. The synthetic regression tests separately verify that a decrease of a count that stays positive does not cause a failure.

Then we connected the server wire audit adapter. After that, a separate run of `0b-move-rejected` on a new dedicated NATS/API passed in 13 seconds with replay and trace. The runtime and NF_SESSIONS agreed exactly across one socket session and five audited wire frames: one inbound MoveTo and four outbound frames. The stream also had one checkpoint JSON record, which the audit did not include. All the intent, payload, event and rejection counters matched with zero differences. The documented audit limit excluded the close codes. This controlled run shows agreement. It does not remove possible differences between queue and delivery during intentional disconnect tests.

## Final complete-suite validation

The measured local run in `/tmp/nightfall-phase1a/final-ci-suite` passed all 15/15 logical units on new owned stacks and 19 scenario client files. The coordinator exit was 0. There were no quarantine entries and no infrastructure errors. The total of the unit run times was 1180.369s. Each role has raw coverage, recording, replay, trace, and transition coverage.

The independent historical baseline comparison passed before the explicit promotion. It kept all previous positive cases and added presence for:

- `events.level_up`
- `reasons.REJECT_REASON_INVALID`
- `reasons.REJECT_REASON_UNKNOWN_ENTITY`
- `close_codes.4409`

The raw runtime totals across the suite were 75 client frames and 5547 server frames. The contract coverage across all compiled categories is:

- Intents: 5/6 covered. `stop_move` is missing.
- Payloads: 3/3 covered. None are missing.
- Events: 12/12 covered. None are missing.
- Reasons: 3/13 observed (`REJECT_REASON_INVALID`, `REJECT_REASON_TOO_FAR`, `REJECT_REASON_UNKNOWN_ENTITY`). Missing: `REJECT_REASON_DEAD_ACTOR`, `REJECT_REASON_NON_ATTACKABLE_TARGET`, `REJECT_REASON_NOT_YET_IMPLEMENTED`, `REJECT_REASON_OUT_OF_BOUNDS`, `REJECT_REASON_OUT_OF_RANGE`, `REJECT_REASON_OVERLOADED`, `REJECT_REASON_PROTECTED`, `REJECT_REASON_RATE_LIMITED`, `REJECT_REASON_TARGET_NOT_IN_AOI`, and `REJECT_REASON_UNSPECIFIED`.
- Documented close codes: 1/4 observed (`4409`). Missing: `4400`, `4408`, and `4429`.

A final exact session audit on the kept new final stack of the last unit (`1-target-unknown`) showed zero differences. The audit had 1 socket session, 5 audited wire frames, and 6 stream records. The stream records included 1 ignored checkpoint JSON record. The counts from the descriptors agreed across these items:

- 1 inbound `SetTarget`.
- 4 outbound frames (payloads: 3 `event`, 1 `rejected`).
- Events (2 `spawn`, 1 `stats_changed`).
- 1 rejection reason `REJECT_REASON_UNKNOWN_ENTITY`.

The audit design excluded the close codes, and the difference between queue and delivery stays.

The CI runner registration and the two-week gating stability window are not complete.
