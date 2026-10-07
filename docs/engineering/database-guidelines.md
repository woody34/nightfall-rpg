# Database Guidelines

Postgres 16, accessed through `sqlx` 0.9 with runtime-checked queries and embedded migrations.
Reference: Instaclustr's top-10 practices, adapted to a game server.

## 1. Transactions are atomic, always

- Every repository method that writes is exactly one transaction: `pool.begin()`, all
  statements, `commit()`. A method that needs two transactions is two methods.
- The transaction includes everything that must be consistent: the state change, the
  idempotency record, and the outbox row. See `PgCharacterRepository::create_idempotent`.
- `sqlx::Transaction` rolls back on drop. Early `return Err(...)` is therefore safe; explicit
  `rollback()` is only for readability when the error is expected (name taken).
- Lock ordering: when a transaction touches multiple rows that other transactions also touch
  (trades, Phase 5), lock them `FOR UPDATE` in ascending id order to prevent deadlocks.
- Retry on serialization failure (`40001`) and deadlock (`40P01`) with jittered backoff,
  bounded to 3 attempts. Nothing else is retried automatically.
- Read Committed is the default and is sufficient when uniqueness is enforced by constraints,
  which is why idempotency uses `INSERT ... ON CONFLICT DO NOTHING` rather than check-then-insert.

## 2. Schema conventions

| Rule | Why |
|------|-----|
| `uuid` v7 primary keys for entities | Time-ordered, so B-tree inserts append and don't fragment; globally unique for sharding later. |
| `bigint GENERATED ALWAYS AS IDENTITY` for logs and outbox | Cheapest sequential key for append-only tables. |
| `timestamptz` everywhere, never `timestamp` | Unambiguous instants. |
| `smallint` for stats, `integer` for level and counts, `bigint` for currency | Right-sized; currency as integer (adena units) avoids float precision problems. |
| `text` with `CHECK` constraints, not `varchar(n)` | Same storage, clearer intent, constraint carries the rule. |
| `jsonb` only for genuinely schemaless data (outbox payload, snapshots) | Typed columns index and constrain better. |
| Named constraints (`characters_name_normalized_key`) | The application matches on the name to map violations to typed errors. |
| `created_at` / `updated_at` on every entity table | Debugging and retention jobs. |
| Normalized by default; denormalize only with a measured read-path reason | Consistency first; the game's hot path is in-memory anyway. |

Migrations are plain SQL in `apps/api/migrations/`, numbered, embedded with `sqlx::migrate!`,
and run at startup. Expand/contract for live changes: add the new column, deploy code that
writes both, backfill, deploy code that reads the new one, drop the old. Never rename in place.

## 3. Queries

- No `SELECT *`. Name the columns; the row mapper depends on them.
- Static SQL strings only. `sqlx` 0.9 rejects dynamically built strings at compile time, which
  is a feature: it makes injection impossible by construction. Parameters are always `$n` binds.
- `EXPLAIN (ANALYZE, BUFFERS)` any query that will run per tick or per player action before
  merging. Keep the plan in the PR description.
- Fetch what you need: `fetch_optional` for by-id, `fetch_one` only when absence is a bug.

## 4. Indexes

- Index every foreign key and every column in a `WHERE` the hot path uses
  (`characters_account_id_idx`).
- Partial indexes for queue tables (`outbox_pending_idx ... WHERE published_at IS NULL`) so the
  index stays tiny as the table grows.
- Review `pg_stat_user_indexes` monthly (Phase 9 dashboard); drop indexes with zero scans.
  Every index costs on write.

## 5. Connections

- One `PgPool` per process, `max_connections = 10`. Postgres wants few connections; the game
  server has one world thread and a handful of request handlers.
- `acquire_timeout = 5s` so a saturated pool fails fast instead of queueing forever.
- When there are multiple server processes, put PgBouncer in transaction mode in front and keep
  per-process pools small.

## 6. Maintenance and configuration

Set in `docker-compose.yml` for dev; mirrored in production config:

| Setting | Dev value | Why |
|---------|-----------|-----|
| `shared_buffers` | 256MB (25% of RAM in prod) | Page cache. |
| `work_mem` | 16MB | Sorts and hashes in memory; keep modest because it is per-operation. |
| `autovacuum_vacuum_scale_factor` | 0.05 | Game tables churn; vacuum sooner than the 20% default. |
| `log_min_duration_statement` | 250ms | Slow-query log is the first dashboard. |
| `log_lock_waits` | on | Deadlocks and lock pile-ups are visible. |

- Autovacuum stays on. Never disable it to "speed things up".
- `ANALYZE` after bulk loads (data imports, migrations that backfill).

## 7. Backups and recovery

- WAL archiving plus a nightly `pg_basebackup`; `pg_dump` of reference data on each deploy.
- Restore is rehearsed monthly into a scratch database and the time is recorded.
- Item and currency changes are additionally written to `item_ledger` (Phase 4), which lets a
  dupe incident be rolled back per item without restoring the whole database.

## 8. Security

- The application role owns only its schema and has no superuser or `CREATEDB`.
- Credentials come from the environment, never from files in the repo. `.env` is ignored.
- TLS to the database in production (`sslmode=verify-full`); the dev compose file is plaintext
  on loopback.

## 9. Monitoring

Phase 9 wires these to Grafana; until then they are what to look at when something is slow:

`pg_stat_statements` (top queries by total time), `pg_stat_activity` (waits), `pg_locks`,
`pg_stat_user_tables` (dead tuples, last autovacuum), pool wait time from the application's
`tracing` spans.
