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

The typed `token_tier_1_count`/`token_tier_2_count` ledger proves consumption. Policy for
once-per-character transfer tokens at 20/40 including eligible existing backfill is implemented
in runtime foundation `1175b73` / `01b9f57`, reviewed production `692e989` with critical fix `ae0583d`
documented in [critical-fix-review.md](file:///home/matt-woodruff/.config/Claude/side-session-notes/phase2-codex-2026-10-09/token-bridge-2026-10-10/critical-fix-review.md)
(P1/P2 resolved), and test evidence review `a43e2f6`. Panels 26/27 expand the dashboard to seven Phase 2 panels total
(reviewed `a09d677`, imported `bf062e1`). Native full-suite, live automation, soak gates, and post-merge root Game and Editor builds with smoke suites are verified (root main merged and pushed `cf9225d`, Game exit 0 in 28.43 s, Editor exit 0 in 19.03 s, 9/9 root smoke pass across Class and TemplateTarget). Local Phase 2 is complete once this final docs commit lands; external runner registration and the two-week reliability programme remain separate operational follow-ups.

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
outbox records in Postgres while recording zero observational metric on `CheckpointOutcome::Replayed`;
this counter is observational telemetry, NOT durable ledger accounting. Generic
`TickTelemetry` explicitly ignores `ZoneEvent::TokensReconciled`.

All four final saved grant scrapes match expected counter samples:
- `2-token-milestone20`: `tier="1"`, `source="level_up"`, count 1
- `2-token-milestone40`: `tier="2"`, `source="level_up"`, count 1
- `2-token-jump19-40`: `tier="1"` count 1, `tier="2"` count 1, `source="level_up"`
- `2-token-backfill`: `tier="1"` count 1, `tier="2"` count 1, `source="admission"` after repeated reconnect/consumption.

Public saved sample excerpts are preserved at `native-evidence/full-suite-01/{unit}/{unit}/fixture/prometheus.token-grants.txt`. Independent bounded admission audit report is preserved at `token-bridge-2026-10-10/native-grant-metrics-final-backfill/report.json`. This counter is an observational metric, not durable accounting; no live Grafana query execution or rate calculation is claimed.

Initial diagnostic failures remain separated from the accepted final suite. In the initial lifecycle diagnostic run, milestone 20/40 assertions and byte-exact replay passed, but aggregate strict lifecycle coverage failed because stale reporter history compared incarnation 2 from a prior admission with the legitimate incarnation 1 of a new admission. Reviewed reporter fix `48c39f3` (integrated `a64423f`) resolved reporting only without modifying gameplay, consumer, or wire contracts: it clears only the global player despawn baseline while retaining NPC slot history, same-tick event order, AOI/session replacement semantics, and strict rejection of malformed backwards player steps (12 focused checks approved). Fresh single milestone 20 retry-02 passed (`e568fd2` + replay `48c39f3` wrapper PASS 27s, exit 0, exact 295/295 ticks, 45 outputs, 9,402 bytes, 0 digest-only, cleanup true). First private `StatsChanged`, claimed-zero post-consumption real re-cross, and frozen receipt assertions passed all final native flows. Required-live UE automation (65 tests), the full ordinary scenario suite (34 files / 29 units), Game package staging (48.35 s), packaged combat soak (8 clients × 1,200 s, p99 14.764 ms, all wrapper/API/cleanup exits 0), and post-merge root Game and Editor builds with Class (8/8) and TemplateTarget (1/1) smoke suites are fully verified (`receipt.json`). Local Phase 2 is complete upon landing this final reviewed documentation commit; external CI runner registration and the two-week reliability programme remain separate operational follow-ups.

Run `python3 scripts/validate_class_observability.py` from any directory for the focused JSON,
catalogue, diagram and link checks. Query-name validation uses the exporter contract; it is
separate from live Grafana query execution. Browser checks render the static diagrams offline.
