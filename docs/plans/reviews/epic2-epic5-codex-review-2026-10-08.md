**Verdict: follow-up fixes required.**

- **P1 — Credentials exported to Loki:** [telemetry/mod.rs:202](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/infrastructure/telemetry/mod.rs:202) exports the existing [NATS connection log:18](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/infrastructure/nats/mod.rs:18), which records the complete URL. Authenticated URLs expose passwords/tokens. Remove the URL field or log only sanitized host/port; add a sentinel-credential capture test.

- **P2 — Outbox gauges always report zero:** [main.rs:118](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/main.rs:118) starts the relay without registering its stats. `RelayStats` does not implement `OutboxStatsSource`; only tests call `set_outbox_source`. Implement the adapter and register it on the exported `Metrics`. Test through production dependency wiring.

- **P2 — Lag cannot detect a stalled relay:** [relay.rs:130](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/infrastructure/outbox/relay.rs:130) records the last successful publication’s age. During broker failure it stays unchanged; after draining it can remain high indefinitely. Compute the oldest pending row’s age on every poll, including failed publishes, and reset to zero when empty.

- **P2 — Concurrent startup migrations race:** [postgres/mod.rs:34](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/infrastructure/postgres/mod.rs:34) replaces sqlx’s migration locking with an unguarded `Migrator::up`. Concurrent instances can both select the baseline as pending and collide on DDL or migration bookkeeping, aborting startup. Serialize migration discovery and execution with a database advisory lock; test concurrent fresh installs and legacy upgrades.

- **P2 — Development startup is broken:** [apps/api/Cargo.toml:1](/home/matt-woodruff/repos/nightfall-rpg/apps/api/Cargo.toml:1) has no `default-run` after adding `nightfall-migrate`. Reproduced: the command used by `api:dev`, `cargo run -p nightfall-api`, exits because two binaries exist. Set `default-run = "nightfall-api"`.

- **P2 — Arbitrary gRPC paths become metric labels:** [layers.rs:167](/home/matt-woodruff/repos/nightfall-rpg/apps/api/src/infrastructure/telemetry/layers.rs:167) derives `service` and `method` directly from client-controlled paths. Repeated unknown paths create distinct metric series and exhaust useful cardinality. Map known RPC paths to fixed labels and bucket everything else as `unknown`; test multiple invalid paths.

- **P2 — Entity verification lacks its CI prerequisite:** [moon.yml:28](/home/matt-woodruff/repos/nightfall-rpg/apps/api/moon.yml:28) enables `entities-check`, but CI never installs `sea-orm-cli`; [entities.sh:14](/home/matt-woodruff/repos/nightfall-rpg/apps/api/entities.sh:14) therefore exits immediately on a clean runner. Install a pinned CLI version before `moon ci`.

- **P2 — CreateCharacter test matrix remains incomplete:** [grpc_create_character.rs:35](/home/matt-woodruff/repos/nightfall-rpg/apps/api/tests/grpc_create_character.rs:35) checks only name/race explicitly; create/get equality cannot detect shared mapping errors. Unit/wire tests omit infrastructure failure, and [postgres_character_repository.rs:125](/home/matt-woodruff/repos/nightfall-rpg/apps/api/tests/postgres_character_repository.rs:125) never forces failure after character insertion. Add explicit response-field assertions, injected `Infrastructure` → sanitized `INTERNAL` tests, and an outbox-insert failure asserting rollback of character, key, and event (§4 items 2, 5, 6, 9).

- **P3 — Unexplained broad lint suppression:** [tests/metrics.rs:2](/home/matt-woodruff/repos/nightfall-rpg/apps/api/tests/metrics.rs:2) suppresses `deprecated` across the entire integration crate without a reason. Scope it to the deprecated request construction and document the Story 1.6 dependency.

Static review; startup failure reproduced. Full tests were not run in the read-only workspace.