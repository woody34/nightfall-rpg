# Database Guidelines

Postgres 16, accessed through `SeaORM` 2.x on top of a `sqlx` 0.9 pool, with `sea-orm-migration`
migrations embedded in the binary (see §3a).
Reference: Instaclustr's top-10 practices, adapted to a game server.

## 1. Transactions are atomic, always

- Every repository method that writes is exactly one transaction: `db.begin()`
  (`TransactionTrait`), all statements, `commit()`. A method that needs two transactions is two methods.
- The transaction includes everything that must be consistent: the state change, the
  idempotency record, and the outbox row. See `PgCharacterRepository::create_idempotent`.
- `DatabaseTransaction` rolls back on drop. Early `return Err(...)` is therefore safe; explicit
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
| `uuid` v7 primary keys for entities | Time-ordered, so B-tree inserts append and don't fragment; globally unique for sharding later. Exception: `accounts.id` is the identity provider's `sub`. |
| `bigint GENERATED ALWAYS AS IDENTITY` for logs and outbox | Cheapest sequential key for append-only tables. |
| `timestamptz` everywhere, never `timestamp` | Unambiguous instants. |
| `smallint` for stats, `integer` for level and counts, `bigint` for currency | Right-sized; currency as integer (adena units) avoids float precision problems. |
| `text` with `CHECK` constraints, not `varchar(n)` | Same storage, clearer intent, constraint carries the rule. |
| `jsonb` only for genuinely schemaless data (outbox payload, snapshots) | Typed columns index and constrain better. |
| Named constraints (`characters_name_normalized_key`) | The application matches on the name to map violations to typed errors. |
| `created_at` / `updated_at` on every entity table | Debugging and retention jobs. |
| Normalized by default; denormalize only with a measured read-path reason | Consistency first; the game's hot path is in-memory anyway. |

Migrations live in `apps/api/src/infrastructure/postgres/migrations/` as `sea-orm-migration`
modules (`mYYYYMMDD_NNNNNN_name.rs`) that execute reviewed plain SQL kept beside them, and run
at startup via `Migrator::up`. The first migration is a baseline: it no-ops when the tables
already exist, so databases created by the old sqlx migrator upgrade in place. Expand/contract for live changes: add the new column, deploy code that
writes both, backfill, deploy code that reads the new one, drop the old. Never rename in place.

## 2a. Idempotency records

One table for every mutating operation (api-guidelines.md section 2):

```
idempotency_keys(account_id uuid, operation text, key uuid, fingerprint text,
                 response jsonb, created_at timestamptz,
                 PRIMARY KEY (account_id, operation, key))
```

- `operation` is the snake-case RPC name (`create_character`, `issue_play_ticket`); the
  constants live in `infrastructure/postgres/idempotency.rs`. Scoping by account means a client
  cannot probe or collide with another account's keys.
- `response` is what a retry returns, written in the same transaction as the state change.
  Store the minimum that rebuilds the reply exactly: an id when the entity can be re-read
  (`{"character_id": ...}`), the full reply when it cannot (the play ticket). The response
  shape is a private serde struct in the repository, so changing it is a code change with a
  review, never an ad-hoc JSON edit.
- Use `idempotency::claim(&tx, account, operation, key, fingerprint, response)` as the first
  statement of the transaction. It is `INSERT ... ON CONFLICT DO NOTHING`; on conflict it reads
  the stored row, and the repository compares fingerprints (`Replayed` or `KeyReused`). The
  primary key serializes concurrent retries; the loser waits for the winner's commit.
- A response may hold a secret only if the secret is short-lived and single-use (the 60 s play
  ticket). Anything longer-lived is stored hashed and the operation is not replayable.

## 2b. Progression checkpoints

`CharacterRepository::checkpoint` is the only writer of `characters` progression columns (level,
xp, hp, mp, alive, position) after creation. One transaction: claim the idempotency key
(operation `save_checkpoint`; response `{"revision": n}`) before anything else, so a retry replays
even though the revision has moved on; then `UPDATE ... WHERE id = $1 AND revision = $revision_seen`
with `revision = revision + 1` - zero rows means `Stale` and the whole transaction (key included)
rolls back, so a stale writer never consumes a key. Domain events are inserted into `outbox` last,
in the same transaction. The key fingerprint covers the whole body including the events; same key
with a different body is `KeyReused`. Check violations (`23514`) surface as typed
`CheckpointError::Constraint(name)`. `hp`/`mp` NULL means full, because the stat engine owns maxima.

## 2c. Zone recovery epochs

`zone_epochs` is durable recovery evidence, retained independently of JetStream messages and
snapshot cleanup. Baseline writes update `zone_snapshots` and insert the epoch row in one
transaction. `started_at` and `closed_at` are its lifecycle timestamps; `first_seq` remains the
epoch's first applied sequence as recovery snapshots advance. `last_recorded_tick` is a
conservative upper bound recorded before log admission; `last_checkpointed_tick` advances
only after all critical saves through that tick complete. Unclosed rows require recovery before
admission. Missing history (including an uncertain final publish) fails closed and is logged;
only completed recovery or a successful clean-shutdown save and baseline closes the row.

## 3. Queries

- No `SELECT *`. Name the columns; the row mapper depends on them.
- Static SQL strings only. `sqlx` 0.9 rejects dynamically built strings at compile time, which
  is a feature: it makes injection impossible by construction. Parameters are always `$n` binds.
- `EXPLAIN (ANALYZE, BUFFERS)` any query that will run per tick or per player action before
  merging. Keep the plan in the PR description.
- Fetch what you need: `fetch_optional` for by-id, `fetch_one` only when absence is a bug.

## 3a. ORM (SeaORM)

- Entities in `infrastructure/postgres/entities/` are **generated** from the migrated schema by
  `moon run api:entities` (`sea-orm-cli generate entity`, run against a throwaway schema). They
  are never hand-edited; change the migration and regenerate. CI runs `api:entities-check`,
  which fails if regeneration produces a diff.
- Run migrations through `infrastructure::postgres::migrate`: it holds the session-scoped
  Postgres advisory lock `0x4e49_4748_5446_414c` while `Migrator::up` runs. The dedicated
  lock connection is closed afterwards, releasing the lock; concurrent starters serialize.
- Entities are infrastructure types. They never cross into `application` or `domain`; the
  repository maps `Model` to domain types.
- Transactions go through `TransactionTrait` (`db.begin()` / `commit()`); pass `&tx` to every
  statement. One repository write method is one transaction.
- Idempotent inserts use `Entity::insert(..).on_conflict(OnConflict::column(..).do_nothing()
  .to_owned()).try_insert()` and branch on `TryInsertResult`.
- Constraint violations are matched by constraint *name* through the underlying sqlx
  `DatabaseError`; the entity API does not expose it.
- Raw SQL is allowed only through `Statement` with bound parameters (`Statement::from_sql_and_values`).
  Never interpolate values into SQL text.
- The `DatabaseConnection` is built from the existing pool
  (`SqlxPostgresConnector::from_sqlx_postgres_pool`); there is one pool per process.

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
