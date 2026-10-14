# Database Guidelines

We use Postgres 16 through `SeaORM` 2.x on a `sqlx` 0.9 pool. The `sea-orm-migration`
migrations are in the binary (see §3a).
Reference: the top-10 practices from Instaclustr, changed for a game server.

## 1. Transactions are atomic, always

- Each repository method that writes is exactly one transaction: `db.begin()`
  (`TransactionTrait`), all statements, then `commit()`. If a method must have two transactions, make two methods.
- The transaction includes all data that must stay consistent: the state change, the
  idempotency record and the outbox row. See `PgCharacterRepository::create_idempotent`.
- `DatabaseTransaction` does a rollback when the value is dropped. Thus an early `return Err(...)` is safe. Use an explicit
  `rollback()` only to make the code easier to read when you expect the error (for example, the name is taken).
- Lock order: some transactions touch many rows that other transactions also touch (trades, Phase 5).
  In these transactions, lock the rows `FOR UPDATE` in ascending id order. This prevents deadlocks.
- If a serialization failure (`40001`) or a deadlock (`40P01`) occurs, try again with jittered backoff.
  The limit is 3 tries. The code does not automatically try other errors again.
- Read Committed is the default. It is sufficient when constraints make sure that values are unique.
  Thus idempotency uses `INSERT ... ON CONFLICT DO NOTHING` and not check-then-insert.

## 2. Schema conventions

| Rule | Why |
|------|-----|
| `uuid` v7 primary keys for entities | The keys are in time order, thus B-tree inserts add to the end and do not fragment. They are globally unique for sharding at a later time. Exception: `accounts.id` is the `sub` from the identity provider. |
| `bigint GENERATED ALWAYS AS IDENTITY` for logs and outbox | This is the cheapest sequential key for append-only tables. |
| `timestamptz` everywhere, never `timestamp` | Each value is one exact instant. |
| `smallint` for stats, `integer` for level and counts, `bigint` for currency | Each type has the correct size. Currency as an integer (adena units) prevents float precision problems. |
| `text` with `CHECK` constraints, not `varchar(n)` | The storage is the same, the intent is easier to understand, and the constraint holds the rule. |
| `jsonb` only for data that truly has no schema (outbox payload, snapshots) | Typed columns are better for indexes and constraints. |
| Named constraints (`characters_name_normalized_key`) | The application matches the name to map violations to typed errors. |
| `created_at` / `updated_at` on each entity table | For debugging and retention jobs. |
| Normalized by default; denormalize only for a measured reason on the read path | Consistency comes first. The hot path of the game is in memory in all conditions. |

The migrations are in `apps/api/src/infrastructure/postgres/migrations/` as `sea-orm-migration`
modules (`mYYYYMMDD_NNNNNN_name.rs`). Each module runs plain SQL that is kept next to it and that people examined.
The migrations run at startup through `Migrator::up`. The first migration is a baseline: if the tables
are already there, it does nothing. Thus databases that the old sqlx migrator made upgrade in place.
For live changes, use expand/contract. Add the new column, then deploy code that
writes both columns. Then backfill, deploy code that reads the new column, and remove the old column. Never rename in place.

## 2a. Idempotency records

One table is for all operations that change data (api-guidelines.md section 2):

```
idempotency_keys(account_id uuid, operation text, key uuid, fingerprint text,
                 response jsonb, created_at timestamptz,
                 PRIMARY KEY (account_id, operation, key))
```

- `operation` is the RPC name in snake case (`create_character`, `issue_play_ticket`). The
  constants are in `infrastructure/postgres/idempotency.rs`. The keys have an account scope. Thus a client
  cannot probe the keys of a different account or collide with them.
- `response` is the value that a retry returns. The code writes it in the same transaction as the state change.
  Store the minimum data that makes the reply again exactly. Store an id if the code can read the entity again
  (`{"character_id": ...}`). Store the full reply if it cannot (the play ticket).
  The response shape is a private serde struct in the repository. Thus a change to it is a code change that people examine, never an ad-hoc JSON edit.
- Use `idempotency::claim(&tx, account, operation, key, fingerprint, response)` as the first
  statement of the transaction. It is `INSERT ... ON CONFLICT DO NOTHING`. If a conflict occurs, it reads
  the stored row, and the repository compares the fingerprints (`Replayed` or `KeyReused`). The
  primary key puts concurrent retries in series. The retry that loses waits for the commit of the retry that wins.
- A response can hold a secret only if the secret has a short life and is for one use only (the 60 s play
  ticket). The code stores a hash of a secret with a longer life, and you cannot replay that operation.

## 2b. Progression checkpoints

After creation, `CharacterRepository::checkpoint` is the only writer of the progression columns of `characters` (level,
xp, hp, mp, alive, position). It is one transaction. First, it claims the idempotency key
(operation `save_checkpoint`; response `{"revision": n}`). Thus a retry replays
when the revision has changed. Then it runs `UPDATE ... WHERE id = $1 AND revision = $revision_seen`
with `revision = revision + 1`. Zero rows means `Stale`. Then the full transaction (with the key) does a
rollback, thus a stale writer never uses a key. Last, the code inserts the domain events into `outbox`
in the same transaction. The key fingerprint includes the full body and the events. The same key
with a different body is `KeyReused`. Check violations (`23514`) show as the typed
`CheckpointError::Constraint(name)`. If `hp`/`mp` is NULL, the value is full, because the stat engine controls the maximum values.

## 2c. Zone recovery epochs

`zone_epochs` is durable recovery evidence. It stays independently of JetStream messages and
snapshot cleanup. Baseline writes update `zone_snapshots` and insert the epoch row in one
transaction. `started_at` and `closed_at` are the lifecycle timestamps of the row. `first_seq` stays the
first applied sequence of the epoch while the recovery snapshots go forward. `last_recorded_tick` is a
conservative upper limit that the code records before log admission. `last_checkpointed_tick` goes forward
only after all critical saves through that tick are complete. Rows that are not closed must have recovery before
admission. If history is missing (this includes an uncertain final publish), the zone fails closed and logs it.
Only a completed recovery, or a successful clean-shutdown save and baseline, closes the row.

## 3. Queries

- Do not use `SELECT *`. Name the columns, because the row mapper uses them.
- Use only static SQL strings. `sqlx` 0.9 rejects dynamically built strings at compile time. This
  is good: it makes injection impossible by design. Parameters are always `$n` binds.
- Before you merge, run `EXPLAIN (ANALYZE, BUFFERS)` on each query that runs per tick or per player action.
  Keep the plan in the PR description.
- Get only the data that is necessary. Use `fetch_optional` to get by id. Use `fetch_one` only when absence is a bug.

## 3a. ORM (SeaORM)

- The entities in `infrastructure/postgres/entities/` are **generated** from the migrated schema by
  `moon run api:entities` (`sea-orm-cli generate entity`, which runs against a temporary schema). Never
  edit them by hand. Change the migration and make the entities again. CI runs `api:entities-check`.
  This check fails if the new generation makes a diff.
- Run migrations through `infrastructure::postgres::migrate`. It holds the session-scoped
  Postgres advisory lock `0x4e49_4748_5446_414c` while `Migrator::up` runs. After that, the code closes the dedicated
  lock connection, and this releases the lock. Concurrent starters run in series.
- Entities are infrastructure types. They never go into `application` or `domain`. The
  repository maps `Model` to domain types.
- Transactions go through `TransactionTrait` (`db.begin()` / `commit()`). Give `&tx` to each
  statement. One repository write method is one transaction.
- Idempotent inserts use `Entity::insert(..).on_conflict(OnConflict::column(..).do_nothing()
  .to_owned()).try_insert()` and branch on `TryInsertResult`.
- The code matches constraint violations by constraint *name* through the underlying sqlx
  `DatabaseError`. The entity API does not give access to it.
- You can use raw SQL only through `Statement` with bound parameters (`Statement::from_sql_and_values`).
  Never interpolate values into SQL text.
- The code builds the `DatabaseConnection` from the pool that is already there
  (`SqlxPostgresConnector::from_sqlx_postgres_pool`). There is one pool per process.

## 4. Indexes

- Index each foreign key and each column in a `WHERE` that the hot path uses
  (`characters_account_id_idx`).
- Use partial indexes for queue tables (`outbox_pending_idx ... WHERE published_at IS NULL`). Thus the
  index stays very small while the table grows.
- Examine `pg_stat_user_indexes` each month (Phase 9 dashboard). Remove indexes with zero scans.
  Each index adds cost on write.

## 5. Connections

- Use one `PgPool` per process, `max_connections = 10`. Postgres operates best with few connections. The game
  server has one world thread and a small number of request handlers.
- Set `acquire_timeout = 5s`. Thus a full pool fails fast and does not wait in a queue forever.
- If there are many server processes, put PgBouncer in transaction mode in front of Postgres. Keep
  the pool of each process small.

## 6. Maintenance and configuration

These values are set in `docker-compose.yml` for dev. The production config has the same values:

| Setting | Dev value | Why |
|---------|-----------|-----|
| `shared_buffers` | 256MB (25% of RAM in prod) | Page cache. |
| `work_mem` | 16MB | Sorts and hashes in memory. Keep it small, because it applies to each operation. |
| `autovacuum_vacuum_scale_factor` | 0.05 | Game tables change a lot. Vacuum sooner than the 20% default. |
| `log_min_duration_statement` | 250ms | The slow-query log is the first dashboard. |
| `log_lock_waits` | on | You can see deadlocks and lock pile-ups. |

- Keep autovacuum on. Never disable it to "speed things up".
- Run `ANALYZE` after bulk loads (data imports, migrations that backfill).

## 7. Backups and recovery

- Use WAL archiving and a nightly `pg_basebackup`. Run `pg_dump` of the reference data on each deploy.
- Each month, do a practice restore into a temporary database and record the time.
- The code also writes item and currency changes to `item_ledger` (Phase 4). Thus you can do a rollback of a
  dupe incident per item, and you do not restore the full database.

## 8. Security

- The application role controls only its schema. It does not have superuser or `CREATEDB`.
- Credentials come from the environment, never from files in the repo. Git ignores `.env`.
- Use TLS to the database in production (`sslmode=verify-full`). The dev compose file uses plaintext
  on loopback.

## 9. Monitoring

Phase 9 connects these to Grafana. Until then, examine these items when an operation is slow:

- `pg_stat_statements` (top queries by total time)
- `pg_stat_activity` (waits)
- `pg_locks`
- `pg_stat_user_tables` (dead tuples, last autovacuum)
- The pool wait time from the `tracing` spans of the application.
