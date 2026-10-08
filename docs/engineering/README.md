# Engineering Guidelines

How we build Nightfall. These are binding for every contribution; the planning docs in
`../planning` say *what* to build, these say *how*.

| Document | Covers |
|----------|--------|
| [rust-guidelines.md](rust-guidelines.md) | The Rust standard: what is enforced as code (rustfmt, clippy, workspace lints, cargo-deny) and what is reviewed by hand. |
| [architecture.md](architecture.md) | Clean architecture layers, the event-bus core on NATS, command vs event, the transactional outbox, the zone actor and its replay log. |
| [api-guidelines.md](api-guidelines.md) | Endpoint rules: idempotency, error mapping, and the test every endpoint must ship with. |
| [telemetry](#telemetry) | Logs, traces, metrics, dashboards and alerts: what exists and the rules for adding to it. |
| [database-guidelines.md](database-guidelines.md) | Postgres rules: atomic transactions, schema conventions, pooling, migrations, maintenance. |

## The standard as code

Most of the standard is enforced by tooling, not by reviewers:

| Concern | Where it lives | Enforced by |
|---------|----------------|-------------|
| Formatting | [`rustfmt.toml`](../../rustfmt.toml) | `cargo fmt --check` (moon `api:format`) |
| Lint levels | `[workspace.lints]` in [`Cargo.toml`](../../Cargo.toml) | `cargo clippy --all-targets -- -D warnings` (moon `api:lint`) |
| Lint thresholds and disallow-lists | [`clippy.toml`](../../clippy.toml) | same |
| Supply chain: licenses, advisories, duplicate crates | [`deny.toml`](../../deny.toml) | `cargo deny check` (moon `api:deny`) |
| Tests | `#[cfg(test)]` modules and `apps/api/tests/` | `cargo test` (moon `api:test`) |

Run everything the CI runs with:

```bash
moon check --all
```

## Telemetry

The stack is OpenTelemetry end to end with Grafana LGTM (`grafana/otel-lgtm`, started by
`docker-compose.yml`) as the backend. Setup and verification commands are in the root
[README](../../README.md#observability). Rules:

- **Initialise once**, in `main`, with `infrastructure::telemetry::init`; keep the returned
  guard and `shutdown().await` it after the servers drain so buffered data is flushed. Tests use
  `Metrics::detached()` and never need a collector.
- **Metrics live in the catalogue** (`infrastructure/telemetry/metrics.rs`), created once and
  passed through `Dependencies`. Add an instrument there, a panel to
  `infra/grafana/dashboards/nightfall-api.json`, and, if it can page someone, a rule in
  `infra/grafana/alerts/`. Names are `nightfall_` + base unit; labels are low-cardinality
  (route templates, method names, status codes), never character, account or session ids.
- **Never record secrets.** Spans take the URL path, not the query or headers. Do not put
  tokens, tickets or passwords in span fields or log fields.
- **Correlate.** Every request has a `request_id` on its root span. Handlers that learn the
  account call `telemetry::record_account_id`. Work handed to another task through a channel
  carries a `TraceCarrier` so the trace stays whole.
- **Log with `tracing`**, structured fields over formatted strings. Logs go to stdout and, over
  OTLP, to Loki.

## Replay log

Binding rules for anything that changes zone state (architecture.md §2.5):

- **Logged before visible.** Nothing a zone tick produced is broadcast, and no later tick runs,
  until that tick's record is acknowledged by `JetStream`. Never bypass `DurableTickGate` in
  production; `OpenGate` is for tests and replay only.
- **Every change is a command in the applied log.** Starting content, joins, leaves and
  replacements are `ZoneCommand`s, never direct state edits, so the log is a complete input
  history. The log is never sampled.
- **Snapshot first, watermark last.** An epoch's snapshot precedes its first record (enforced
  by `EpochStarted`); replay refuses an epoch without a watermark.
- **Record formats are append-only.** Never renumber a protobuf tag or reject reason in
  `replay_log/codec.rs`; bump `SNAPSHOT_SCHEMA_VERSION` on any snapshot change.

## Sources

These guidelines distil the following references. Where they conflict, this document wins.

- "Rust Best Practices" gist (auser): newtypes, error handling, lints as code, allocation discipline.
  <https://gist.github.com/auser/c3161f55a8393faa8af5ddda68c6befa>
- Dezhic, "Reliable software engineering with Rust" (Globant, 2023).
  <https://medium.com/globant/reliable-software-engineering-with-rust-5bb4553b5d54>
- MSC29, clean-architecture-rust: adapters / application / domain layout, repository traits, fixture-driven integration tests.
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

Every agent session that writes code runs in its own git worktree under `.claude/worktrees/`
on its own branch, so two writers never touch the same files. Each worktree carries its own
Rust `target/` directory (3-10 GB after a full build), so worktrees are removed as soon as their
branch is merged:

```bash
git worktree remove --force .claude/worktrees/<name> && git worktree prune
```

Merged remote branches are deleted; local branches stay so a session can be resumed. Read-only
reviewers (Codex) run against the main checkout in a read-only sandbox and need no worktree.
Repo policy: no pull requests; the architect merges branches into `main` after lint and tests
pass on the merged tree, then removes the worktree.
