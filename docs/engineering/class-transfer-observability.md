# Class transfer observability

The [Nightfall API dashboard](../../infra/grafana/dashboards/nightfall-api.json), UID
`nightfall-api`, adds seven Phase 2 panels. For the catalogue and the commit order, refer to the offline [class tree](../diagrams/phase-2-class-tree.html)
and the [transfer lifecycle](../diagrams/phase-2-class-transfer.html).

| Panels | Prometheus series | Meaning |
|---|---|---|
| 21–22: response rate/count | `nightfall_class_transfer_seconds_count{outcome}` | Completed `ChangeClass::execute` calls, divided into `success` and `error`. |
| 23: latency p50/p99 | `nightfall_class_transfer_seconds_bucket{outcome,le}` | The wall time of the use case, in seconds. The quantiles keep `le` and `outcome`. |
| 24–25: committed transfer rate/count | `nightfall_class_transfers_total{from,to}` | One admitted internal `ClassTransfer` fact for each new transfer, after the log acknowledgement and the atomic checkpoint. |
| 26–27: observed token grant rate/count | `nightfall_class_transfer_token_grants_total{tier,source}` | New tokens that an applied checkpoint confirms. Replay acknowledgements and mark-only passes are not included. |

The histogram also exports `nightfall_class_transfer_seconds_sum{outcome}`. Its finite bucket
limits come from `IO_BUCKETS` in [metrics.rs](../../apps/api/src/infrastructure/telemetry/metrics.rs).

The [handler](../../apps/api/src/interface/grpc/mod.rs) starts the timer after authentication and
after it parses the **idempotency-key** successfully. The use case parses the character UUID.
Thus the histogram includes these syntax errors. The histogram does not include authentication failures and
idempotency keys that are not correctly formed. The general gRPC request panels show the transport status.
Receipt retries count as successful responses. This includes an offline historical receipt that the API returns
after a later transfer. Receipt retries do not increment the committed-transfer counter, and do not move the live state back.
The public `ClassChanged` observer fan-out also does not increment this counter. The
[admitted-event consumer](../../apps/api/src/infrastructure/telemetry/combat.rs) and its
[focused test](../../apps/api/src/infrastructure/telemetry/combat_tests.rs) define that limit.

The application labels are only these:

- The two response outcomes.
- The catalogue profession IDs `from`/`to`.
- The token grant `tier` (`"1"` or `"2"`) and `source` (`"admission"` or `"level_up"`).

The Prometheus exporter also adds the fixed `otel_scope_name="nightfall-api"` label. The panel
aggregations keep only the documented application labels (and `le` for buckets).
The labels include no account, character, name or mutation key. Thus a count of successful responses
is different from the number of new transfers.

To find the cause of stalls, use these panels: the checkpoint lag/failure panels (19–20), the event-log failure panel and the paused-zone
panel. These panels are **shared all-zone** persistence signals. They are not counts for transfers only.
Critical transfers hold later drafts until the checkpoint resolves. Permanent failures
fence the output. Zero checkpoint lag means that no pending checkpoint has an age. It does not prove that a
transfer was successful. Refer to [checkpoint.rs](../../apps/api/src/application/checkpoint.rs) and
[zone_actor.rs](../../apps/api/src/application/zone_actor.rs).

The API makes a new label set when it first observes it. Before that, the class-transfer queries can
show **No data**. Rates also must have a sufficient number of scrape samples. An empty series is not a measured
zero. The API initializes the checkpoint lag/failure and paused-zone instruments when it starts. Thus
these instruments can report zero before a transfer occurs. No query puts a synthetic zero in place of a missing series.

The typed `token_tier_1_count`/`token_tier_2_count` ledger proves consumption. The policy gives transfer tokens
one time for each character at 20/40, and includes the backfill for eligible characters that exist. These commits implement the policy:

- Runtime foundation `1175b73` / `01b9f57`.
- Reviewed production `692e989`, with the critical fix `ae0583d`. [critical-fix-review.md](file:///home/matt-woodruff/.config/Claude/side-session-notes/phase2-codex-2026-10-09/token-bridge-2026-10-10/critical-fix-review.md)
  documents this fix (P1/P2 resolved).
- Test evidence review `a43e2f6`.

Panels 26/27 increase the dashboard to seven Phase 2 panels total
(reviewed `a09d677`, imported `bf062e1`). These items are verified:

- The native full suite, the live automation and the soak gates.
- The post-merge root Game and Editor builds with smoke suites. The root main was merged and pushed as `cf9225d`.
  Game exit 0 in 28.43 s. Editor exit 0 in 19.03 s. 9/9 root smoke pass across Class and TemplateTarget.

Local Phase 2 is complete when this final docs commit merges. The external runner registration and the two-week reliability programme are
separate operational follow-ups.

The real OpenTelemetry instrument is `nightfall_class_transfer_token_grants` (`u64_counter` in
[metrics.rs](../../apps/api/src/infrastructure/telemetry/metrics.rs)). It is exported to Prometheus as
`nightfall_class_transfer_token_grants_total`. The labels are strictly limited to `tier="1"|"2"` and
`source="admission"|"level_up"`. The dashboard aggregations (`sum by (tier, source)`) discard the fixed exporter
label `otel_scope_name="nightfall-api"`.

Only real `DomainEvent::CharacterTokenGranted` events that are confirmed after `CheckpointOutcome::Applied`
increment the counter, through `metrics.token_granted` in
[checkpoint.rs](../../apps/api/src/application/checkpoint.rs) and
[combat.rs](../../apps/api/src/infrastructure/telemetry/combat.rs). These items do not count:

- Mark-only/noop passes.
- Replay verifier checks.
- Duplicate/reconnect without new grants.
- Receipt retry.
- `CheckpointOutcome::Replayed`.

If a commit ACK is lost, Postgres can have durable exactly-once outbox records, while the metric records zero on
`CheckpointOutcome::Replayed`. This counter is observational telemetry, NOT durable ledger accounting. The generic
`TickTelemetry` explicitly ignores `ZoneEvent::TokensReconciled`.

All four final saved grant scrapes agree with the expected counter samples:

- `2-token-milestone20`: `tier="1"`, `source="level_up"`, count 1
- `2-token-milestone40`: `tier="2"`, `source="level_up"`, count 1
- `2-token-jump19-40`: `tier="1"` count 1, `tier="2"` count 1, `source="level_up"`
- `2-token-backfill`: `tier="1"` count 1, `tier="2"` count 1, `source="admission"` after repeated reconnect/consumption.

The public saved sample excerpts are kept at `native-evidence/full-suite-01/{unit}/{unit}/fixture/prometheus.token-grants.txt`.
The independent bounded admission audit report is kept at `token-bridge-2026-10-10/native-grant-metrics-final-backfill/report.json`.
This counter is an observational metric, not durable accounting. We do not claim a live Grafana query run or a rate calculation.

The initial diagnostic failures stay separate from the accepted final suite. In the initial lifecycle diagnostic run,
the milestone 20/40 assertions and the byte-exact replay passed. But the aggregate strict lifecycle coverage failed.
The cause was old reporter history: it compared incarnation 2 from a prior admission with the legitimate incarnation 1 of a new admission.

The reviewed reporter fix `48c39f3` (integrated `a64423f`) repaired only the report. It did not change the gameplay,
the consumer or the wire contracts. It clears only the global player despawn baseline. It keeps these items:

- The NPC slot history.
- The same-tick event order.
- The AOI/session replacement semantics.
- The strict rejection of player steps that go backwards and are not correctly formed (12 focused checks approved).

A new single milestone 20 retry-02 passed (`e568fd2` + replay `48c39f3` wrapper PASS 27s, exit 0, exact 295/295 ticks,
45 outputs, 9,402 bytes, 0 digest-only, cleanup true). All final native flows passed these assertions: the first private `StatsChanged`,
the claimed-zero real re-cross after consumption, and the frozen receipt. These items are fully verified (`receipt.json`):

- The required-live UE automation (65 tests).
- The full ordinary scenario suite (34 files / 29 units).
- The Game package staging (48.35 s).
- The packaged combat soak (8 clients × 1,200 s, p99 14.764 ms, all wrapper/API/cleanup exits 0).
- The post-merge root Game and Editor builds with Class (8/8) and TemplateTarget (1/1) smoke suites.

Local Phase 2 is complete when this final reviewed documentation commit merges. The external CI runner registration and the two-week
reliability programme are separate operational follow-ups.

For the focused JSON, catalogue, diagram and link checks, run `python3 scripts/validate_class_observability.py` from any directory.
The query-name validation uses the exporter contract. It is separate from a live Grafana query run. The browser checks render the static diagrams offline.
