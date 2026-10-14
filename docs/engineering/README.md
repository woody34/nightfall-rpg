# Engineering Guidelines

This document tells how we build Nightfall. These rules are binding for all contributions. The planning documents in
`../planning` tell *what* to build. These guidelines tell *how* to build it.

| Document | Covers |
|----------|--------|
| [rust-guidelines.md](rust-guidelines.md) | The Rust standard: the rules that code enforces (rustfmt, clippy, workspace lints, cargo-deny) and the rules that persons examine manually. |
| [architecture.md](architecture.md) | Clean architecture layers, the event-bus core on NATS, command and event, the transactional outbox, the zone actor and its replay log. |
| [api-guidelines.md](api-guidelines.md) | Endpoint rules: idempotency, error mapping, and the test that each endpoint must have when it merges. |
| [telemetry](#telemetry) | Logs, traces, metrics, dashboards and alerts: what exists, and the rules for additions. |
| [database-guidelines.md](database-guidelines.md) | Postgres rules: atomic transactions, schema conventions, pooling, migrations, maintenance. |

## The standard as code

Tools enforce most of the standard. Reviewers do not:

| Concern | Where it lives | Enforced by |
|---------|----------------|-------------|
| Formatting | [`rustfmt.toml`](../../rustfmt.toml) | `cargo fmt --check` (moon `api:format`) |
| Lint levels | `[workspace.lints]` in [`Cargo.toml`](../../Cargo.toml) | `cargo clippy --all-targets -- -D warnings` (moon `api:lint`) |
| Lint thresholds and disallow-lists | [`clippy.toml`](../../clippy.toml) | The same |
| Supply chain: licenses, advisories, duplicate crates | [`deny.toml`](../../deny.toml) | `cargo deny check` (moon `api:deny`) |
| Tests | `#[cfg(test)]` modules and `apps/api/tests/` | `cargo test` (moon `api:test`) |

To run all the checks that the CI runs, use this command:

```bash
moon check --all
```

## Telemetry

The stack uses OpenTelemetry from end to end. The backend is Grafana LGTM (`grafana/otel-lgtm`, which
`docker-compose.yml` starts). The setup and verification commands are in the root
[README](../../README.md#observability). Rules:

- **Initialise one time**, in `main`, with `infrastructure::telemetry::init`. Keep the guard that it returns.
  After the servers drain, do `shutdown().await` on the guard, so that the buffered data is flushed. Tests use
  `Metrics::detached()`. A collector is never necessary for tests.
- **Metrics live in the catalogue** (`infrastructure/telemetry/metrics.rs`). The code makes them one time and
  passes them through `Dependencies`. To add a metric, add an instrument there and a panel to
  `infra/grafana/dashboards/nightfall-api.json`. If the metric can page a person, also add a rule in
  `infra/grafana/alerts/`. Names are `nightfall_` + base unit. Labels have low cardinality
  (route templates, method names, status codes). Labels are never character, account or session ids.
- **Combat telemetry** reads one admitted zone-wide batch for each live tick, before the
  next broadcast. Session AOI output and replay never increment it. The only labels are
  `outcome=miss|hit|crit`, `kind=player|npc`, and `to=idle|active|attack|return_home|dead`.
  `nightfall_combat_tick_duration_seconds` uses all `TickStats` samples (this includes idle ticks and
  admission waits). Its p99 alert fires above 50 ms for 1 minute. Admitted
  `NpcIntentionChanged` events count by `to`. The E4.2 checkpoint worker must still call
  its documented catalogue hooks.
  The checkpoint lag is the age of the oldest pending save. It goes back to zero when the queue is drained.
- **Never record secrets.** Spans take the URL path, not the query or the headers. Do not put
  tokens, tickets or passwords in span fields or log fields.
- **Correlate.** Each request has a `request_id` on its root span. Handlers that learn the
  account call `telemetry::record_account_id`. When work goes to a different task through a channel, it
  carries a `TraceCarrier`, so that the trace stays complete.
- **Log with `tracing`**. Use structured fields, not formatted strings. Logs go to stdout and, over
  OTLP, to Loki.

## Replay log

These rules are binding for all code that changes zone state (architecture.md §2.5):

- **Logged before visible.** The zone does not broadcast the output of a zone tick until `JetStream` acknowledges
  the record of that tick. Until then, the zone also does not run a later tick. Never go around `DurableTickGate` in
  production. `OpenGate` is only for tests and replay.
- **Each change is a command in the applied log.** Starting content, joins, leaves and
  replacements are `ZoneCommand`s, never direct state edits. Thus the log is a complete input
  history. The log is never sampled.
- **Snapshot first, watermark last.** The snapshot of an epoch comes before its first record
  (`EpochStarted` enforces this). Replay refuses an epoch without a watermark.
- **Record formats are append-only.** Never renumber a protobuf tag or reject reason in
  `replay_log/codec.rs`. Increase `SNAPSHOT_SCHEMA_VERSION` on each snapshot change.

## Sources

These guidelines come from the references that follow. If a reference and this document do not agree, this document is correct.

- "Rust Best Practices" gist (auser): newtypes, error handling, lints as code, allocation discipline.
  <https://gist.github.com/auser/c3161f55a8393faa8af5ddda68c6befa>
- Dezhic, "Reliable software engineering with Rust" (Globant, 2023).
  <https://medium.com/globant/reliable-software-engineering-with-rust-5bb4553b5d54>
- MSC29, clean-architecture-rust: adapters / application / domain layout, repository traits, integration tests that use fixtures.
  <https://github.com/MSC29/clean-architecture-rust>
- Kigawas, "A Rustacean's clean architecture approach": thin routers, thick persistence, slim models.
  <https://kigawas.me/posts/rustacean-clean-architecture-approach/>
- Rust Design Patterns (rust-unofficial): idioms, patterns, anti-patterns.
  <https://rust-unofficial.github.io/patterns/>
- jdno, "Designing an API for a video game": command bus and event bus around a simulation.
  <https://jdno.dev/designing-an-api-for-a-video-game/>
- Instaclustr, "Top 10 PostgreSQL best practices for 2025".
  <https://www.instaclustr.com/education/postgresql/top-10-postgresql-best-practices-for-2025/>

## Parallel sessions and worktrees

Each agent session that writes code runs in its own git worktree under `.claude/worktrees/`,
on its own branch. Thus two writers never change the same files. Each worktree has its own
Rust `target/` directory (3-10 GB after a full build). Thus we remove each worktree immediately after its
branch is merged:

```bash
git worktree remove --force .claude/worktrees/<name> && git worktree prune
```

We remove the remote branches after they merge. The local branches stay, so that a session can continue. Read-only
reviewers (Codex) run against the main checkout in a read-only sandbox, and do not use a worktree.
Repo policy: no pull requests. The architect merges branches into `main` when lint and tests
pass on the merged tree. Then the architect removes the worktree.
