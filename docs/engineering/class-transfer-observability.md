# Class transfer observability

The [Nightfall API dashboard](../../infra/grafana/dashboards/nightfall-api.json), UID
`nightfall-api`, adds five Phase 2 panels. See the offline [class tree](../diagrams/phase-2-class-tree.html)
and [transfer lifecycle](../diagrams/phase-2-class-transfer.html) for the catalogue and commit order.

| Panels | Prometheus series | Meaning |
|---|---|---|
| 21–22: response rate/count | `nightfall_class_transfer_seconds_count{outcome}` | Completed `ChangeClass::execute` calls, split into `success` and `error`. |
| 23: latency p50/p99 | `nightfall_class_transfer_seconds_bucket{outcome,le}` | Use-case wall time in seconds; quantiles retain `le` and `outcome`. |
| 24–25: committed transfer rate/count | `nightfall_class_transfers_total{from,to}` | One admitted internal `ClassTransfer` fact per new transfer, after log acknowledgement and atomic checkpoint. |

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

Labels are limited to the two response outcomes and catalogue profession IDs `from`/`to`.
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

The typed `token_tier_1_count`/`token_tier_2_count` ledger proves consumption. Required token
supply/backfill policy is still undecided; there is no production grant mechanism or grant metric.
These panels do not resolve that product decision or establish full Phase 2 acceptance.

Run `python3 scripts/validate_class_observability.py` from any directory for the focused JSON,
catalogue, diagram and link checks. Query-name validation uses the exporter contract; it is
separate from live Grafana query execution. Browser checks render the static diagrams offline.
