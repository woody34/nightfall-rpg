# Class transfer observability

The [Nightfall API dashboard](../../infra/grafana/dashboards/nightfall-api.json), UID
`nightfall-api`, adds seven Phase 2 panels. See the offline [class tree](../diagrams/phase-2-class-tree.html)
and [transfer lifecycle](../diagrams/phase-2-class-transfer.html) for the catalogue and commit order.

| Panels | Prometheus series | Meaning |
|---|---|---|
| 21–22: response rate/count | `nightfall_class_transfer_seconds_count{outcome}` | Completed `ChangeClass::execute` calls, split into `success` and `error`. |
| 23: latency p50/p99 | `nightfall_class_transfer_seconds_bucket{outcome,le}` | Use-case wall time in seconds; quantiles retain `le` and `outcome`. |
| 24–25: committed transfer rate/count | `nightfall_class_transfers_total{from,to}` | One admitted internal `ClassTransfer` fact per new transfer, after log acknowledgement and atomic checkpoint. |
| 26–27: observed token grant rate/count | `nightfall_class_transfer_token_grants_total{tier,source}` | New tokens confirmed by applied checkpoint; replay acknowledgements and mark-only passes excluded. |

The histogram also exports `nightfall_class_transfer_seconds_sum{outcome}`. Its finite bucket
boundaries come from `IO_BUCKETS` in [metrics.rs](../../apps/api/src/infrastructure/telemetry/metrics.rs).

The [handler](../../apps/api/src/interface/grpc/mod.rs) starts timing after authentication and
successful **idempotency-key** parsing. Character UUID parsing happens inside the use case;
those syntax errors are included. Authentication failures and malformed idempotency keys
are excluded from this histogram; the general gRPC request panels cover transport status.
Receipt retries count as successful responses, including an offline historical receipt returned
after a later transfer. They do not increment the committed-transfer counter or rewind live state.
Public `ClassChanged` observer fan-out also does not increment it. The
[admitted-event consumer](../../apps/api/src/infrastructure/telemetry/combat.rs) and its
[focused test](../../apps/api/src/infrastructure/telemetry/combat_tests.rs) define that boundary.

Application labels are limited to the two response outcomes, catalogue profession IDs `from`/`to`,
and token grant `tier` (`"1"` or `"2"`) and `source` (`"admission"` or `"level_up"`).
The Prometheus exporter also adds the fixed `otel_scope_name="nightfall-api"` label; panel
aggregations retain only the documented application labels (and `le` for buckets).
They contain no account, character, name or mutation key. A successful-response count is
therefore different from the number of new transfers.

Use the existing checkpoint lag/failure panels (19–20), event-log failure panel and paused-zone
panel to investigate stalls. These are **shared all-zone** persistence signals, not transfer-only
counts. Critical transfers hold later drafts until the checkpoint resolves; permanent failures
fence output. Zero checkpoint lag means no pending checkpoint age, not proof of a successful
transfer. See [checkpoint.rs](../../apps/api/src/application/checkpoint.rs) and
[zone_actor.rs](../../apps/api/src/application/zone_actor.rs).

New label sets are created on their first observation. Before that, class-transfer queries can
show **No data**. Rates also need enough scrape samples; an empty series is not a measured
zero. Checkpoint lag/failure and paused-zone instruments are initialized at startup, so they
can report zero before a transfer occurs. No query replaces missing series with synthetic zero.

The typed `token_tier_1_count`/`token_tier_2_count` ledger proves consumption. Policy is accepted
for once-per-character transfer tokens at 20/40 including eligible existing backfill (2026-10-10);
runtime foundation `1175b73` exists and critical followup `692e989` (integrated at `ae0583d`)
is now independently approved (coordinator receipt in shared token `critical-fix-review.md`);
actual native and exporter validation remain pending. These panels do not establish overall Phase 2
completion or native PASS.

The actual OpenTelemetry instrument is `nightfall_class_transfer_token_grants` (`u64_counter` in
[metrics.rs](../../apps/api/src/infrastructure/telemetry/metrics.rs)), exported to Prometheus as
`nightfall_class_transfer_token_grants_total`. Labels are strictly bounded to `tier="1"|"2"` and
`source="admission"|"level_up"`. The fixed exporter label `otel_scope_name="nightfall-api"` is discarded
by dashboard aggregations (`sum by (tier, source)`).

Only real `DomainEvent::CharacterTokenGranted` events confirmed after `CheckpointOutcome::Applied`
increment the counter via `metrics.token_granted` in
[checkpoint.rs](../../apps/api/src/application/checkpoint.rs) and
[combat.rs](../../apps/api/src/infrastructure/telemetry/combat.rs). Mark-only/noop passes, replay
verifier checks, duplicate/reconnect without new grants, receipt retry, and
`CheckpointOutcome::Replayed` do not count. A lost commit ACK may yield durable exactly-once
outbox records in Postgres while recording zero observational metric if the zone retries or
replays; this counter is observational telemetry, NOT durable ledger accounting. Generic
`TickTelemetry` explicitly ignores `ZoneEvent::TokensReconciled`.

Run `python3 scripts/validate_class_observability.py` from any directory for the focused JSON,
catalogue, diagram and link checks. Query-name validation uses the exporter contract; it is
separate from live Grafana query execution. Browser checks render the static diagrams offline.
