# Rust Guidelines

The analogue of a Roslyn `.editorconfig` + analyzer ruleset. Section 1 is enforced by the
toolchain; sections 2 onward are what reviewers check.

## 1. Enforced as code

### 1.1 Formatting: `rustfmt.toml`

100-column lines, 4-space indent, Unix newlines, field-init and `?` shorthand, sorted imports and
modules. Only stable options, so it runs on the pinned stable toolchain. `cargo fmt --all` before
every commit; CI runs `cargo fmt --check` and fails on any diff. Never argue about formatting in
review: if rustfmt accepts it, it is formatted.

### 1.2 Lint levels: `[workspace.lints]` in the root `Cargo.toml`

Every crate opts in with `[lints] workspace = true`. The policy, in order of severity:

| Level | What | Why |
|-------|------|-----|
| `forbid` | `unsafe_code` | Nothing in the game server needs it. A crate that does gets its own, audited, with `SAFETY:` comments on every block. |
| `deny` | `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`, `print_stdout`, `print_stderr`, `float_cmp`, `mem_forget`, `undocumented_unsafe_blocks`, `unused_must_use` | A game server must not crash on bad input. Return `Result`. Log with `tracing`. |
| `warn` (fails CI via `-D warnings`) | `clippy::all`, `clippy::pedantic`, `missing_docs`, `unreachable_pub`, `indexing_slicing`, `arithmetic_side_effects`, `wildcard_enum_match_arm`, and the rest | Pedantic catches real bugs in math-heavy code; the three named ones force explicit overflow policy and exhaustive matching on our own enums. |
| `allow` | `module_name_repetitions`, `must_use_candidate`, `missing_errors_doc`, `missing_panics_doc`, `cast_precision_loss` | Each has a one-line reason in `Cargo.toml`. Add to this list only with a reason. |

Tests relax `unwrap_used`, `expect_used`, and `indexing_slicing` at module level:

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests { ... }
```

Anywhere else, suppress a lint with the narrowest `#[allow(...)]` on the item and a comment
saying why. Never widen the workspace list for one call site.

### 1.3 Thresholds and disallow-lists: `clippy.toml`

Cognitive complexity 20, 6 arguments, 120 lines per function. `std::sync::Mutex` is disallowed
in favour of `tokio::sync::Mutex` (async) or `parking_lot::Mutex` (short sync sections).
`std::env::set_var`, `std::process::exit`, and ad-hoc `spawn_blocking` are disallowed with
reasons in the file.

### 1.4 Supply chain: `deny.toml`

Permissive licenses only, no OpenSSL (rustls everywhere), no wildcard versions, no unknown
registries, RustSec advisories fail the build. `Cargo.lock` is committed. `rust-version` is
pinned in the workspace and matches `.prototools`.

## 2. Types

- **Newtype every identifier and unit.** `CharacterId(Uuid)`, `IdempotencyKey(Uuid)`,
  `CharacterName(String)`. A function taking two `Uuid`s can have them swapped; one taking a
  `CharacterId` and an `AccountId` cannot.
  UUID ids are declared with `uuid_id!` (`domain/ids.rs`), never hand-rolled, and parsed at the
  transport edge with `application::parse_id("field", raw)`, which yields the uniform
  `InvalidArgument("field must be a UUID")`. `SeaORM` conversions are added in
  `infrastructure/postgres/mod.rs` via `uuid_id_sea_orm!` so the domain stays persistence-free.
- **Validate in constructors.** `CharacterName::new` is the only way to get a `CharacterName`,
  so every instance is 3-16 ASCII letters. Code downstream never re-checks.
- **Enums over booleans and sentinels.** `Race` not `u8`; `CreateOutcome::{Created, Replayed,
  KeyReused}` not `(bool, Option<Character>)`.
- **`#[non_exhaustive]` on enums that will grow** (`DomainEvent`), so adding a variant is not a
  breaking change for downstream matches inside the crate.
- **Naming**: `into_x` consumes, `to_x` may allocate, `as_x` borrows. `parse` for fallible
  string constructors, `new` for infallible or validated ones.

## 3. Errors

- Each layer has one error type: `domain::character::NameError` (what), `application::AppError`
  (why, coarse), transport status (how to tell the client). Inner errors convert outward with
  `From`; infrastructure errors become `AppError::Infrastructure(anyhow::Error)` and are logged,
  never shown to clients.
- `thiserror` for typed errors, `anyhow` only at the infrastructure boundary and in `main`.
- `?` for propagation. If a retry or rollback policy applies, write it out; do not hide it in
  a `map_err`.
- Panics are for programmer defects only, and the lints deny the usual macros. If an invariant
  truly cannot be violated, encode it in a type rather than asserting it.

## 4. Ownership and concurrency

- Borrow by default. `&str` over `&String`, `&[T]` over `&Vec<T>`.
- Static dispatch in hot paths (the tick loop, stat math). `Arc<dyn Trait>` only at the
  composition root where adapters are chosen at runtime, which is what `Dependencies` does.
- Single owner, message passing. The world simulation owns its state and receives commands
  on a channel (Phase 0 §6). No `Arc<Mutex<World>>`.
- Bound every queue. `mpsc::channel(256)` not `unbounded_channel`. State the backpressure
  behaviour in a comment.
- `tokio::sync::Mutex` across awaits, `parking_lot::Mutex` for short critical sections,
  never `std::sync::Mutex` (disallowed).

## 5. Modules and visibility

- Layers are modules under `apps/api/src/`: `domain`, `application`, `infrastructure`,
  `interface`. Dependencies point inward only. See `architecture.md`.
- Start private. `pub(crate)` before `pub`. `unreachable_pub` warns when `pub` is
  not actually reachable.
- One use case per file. One adapter per file. `mod.rs` re-exports the public surface.

## 6. Tests

- **Unit tests** live next to the code in `#[cfg(test)] mod tests`, use in-memory adapters, and
  never touch I/O. Every use case has them. Every domain constructor tests its boundaries
  (one below min, min, max, one above max).
- **Integration tests** live in `apps/api/tests/`, boot the real axum and tonic servers on
  ephemeral ports via `common::TestApp`, and call every endpoint through a real socket.
  See `api-guidelines.md` for the mandatory per-endpoint matrix.
- **Adapter tests** against real Postgres and NATS are gated on `DATABASE_URL` / `NATS_URL`
  and skip (pass) when the services are absent. `docker compose up -d` provides both.
- Deterministic: seeded RNG, fixed clocks (`Clock` port), fresh schema per Postgres test.
- Name tests as sentences: `same_key_different_body_is_a_conflict`.

## 7. Determinism (`domain/zone`)

The zone simulation targets byte-for-byte replay (plan D7, §8 of
`docs/plans/phase-0b-connected-slice.md`): the same snapshot plus the same applied-command log
should reproduce the same `AppliedTick` records. Rules for `domain/zone` and
`application/zone_actor`:

- **No floats.** Positions and lengths are `Fixed` (`i32`, 1/1000 tile). Distances compare
  exact squared integers (`u128`); movement steps with integer math and a documented
  rounding rule (`Vec2Fixed::step_toward`) and arrives exactly. Wire floats are converted once,
  in `interface::zone_mapping`, rounding to the nearest milli-tile after finite/range checks.
- **No hash-ordered collections.** `BTreeMap` / `BTreeSet` or sorted `Vec`s only, so every
  iteration order is defined. Events, per-player outputs and AOI diffs are emitted in entity-id
  and ordinal order.
- **Time only via `Tick`.** The zone never reads a clock. `server_time_ms` is
  `time_origin_ms + tick * 100`. The actor's only clock read is a monotonic `Instant` feeding
  `TickStats::duration_micros`, which never reaches state or output.
- **Randomness only via the injected generator.** One `ChaCha12Rng` per zone, keyed from
  `(zone_id, epoch)` with a fixed byte layout. Its full state (key, stream, word position) is in
  the snapshot. Player entity ids are character ids; every other id comes from the generator.
- **Every state change is a command.** Commands get an ordinal when drafted; refused commands
  produce a `Disposition`, never a silent drop. `AppliedTick` is the replay log's unit.

Enforcement is scoped to `domain/zone`, not the actor or the whole crate:

- `mod.rs` denies Clippy's `float_arithmetic`, `float_cmp`, `float_cmp_const`,
  `lossy_float_literal`, `cast_precision_loss`, `imprecise_flops` and `suboptimal_flops`.
  `api:lint` rejects operations those lints detect; this is not a blanket Rust float-type ban.
- `zone_sources_use_no_nondeterministic_apis` scans embedded source lines, discards text from
  the first `//`, and rejects substrings `f32`, `f64`, `HashMap`, `HashSet`, `Instant`,
  `SystemTime`, `thread_rng`, `OsRng`, `rand::random`, and `from_entropy`. This is a text
  check, not name resolution or an exhaustive ban on all nondeterministic APIs.
- `every_zone_source_is_scanned` compares the embedded list with `.rs` files directly in
  `domain/zone`; a new file there must be listed. It does not recursively scan subdirectories.
- `tests/zone_determinism.rs` checks identical streams across actors/runs, recorded-draft
  replay, snapshot round-trips and exact movement. These tests do not prove equivalence
  across all machines or builds.

## 8. Documentation

- `missing_docs` is on. Every public item has a doc comment that says what it is for, and
  for functions, what errors mean.
- Comments explain *why*. Invariants, algorithmic rationale, and L2 references belong in
  comments; what the code does belongs in the code.
- Every `TODO` references an issue or a planning-doc section.

## 9. Dependencies

- Declared once in `[workspace.dependencies]` with `default-features = false` where the crate
  has heavy defaults (`sqlx`, `chrono`, `reqwest`), and features listed explicitly.
- No new dependency for a helper under about fifty lines.
- Prefer crates already in the tree: `tokio`, `tonic`, `axum`, `sqlx`, `async-nats`, `serde`,
  `thiserror`, `anyhow`, `tracing`, `uuid`, `chrono`, `parking_lot`.

## 10. Patterns we use, by name

From the Rust Design Patterns catalogue:

| Pattern | Where |
|---------|-------|
| Newtype | all ids, keys, names |
| Constructor with validation | `CharacterName::new`, `IdempotencyKey::parse` |
| Strategy via trait objects at the edge | `CharacterRepository`, `EventBus`, `Clock` ports |
| Command | `CreateCharacterInput` is the command, `DomainEvent` the fact |
| RAII guard | `sqlx::Transaction` rolls back on drop unless committed |
| `#[non_exhaustive]` | `DomainEvent` |
| Default | `InMemory*` adapters, `Position` |

Anti-patterns we reject: clone to satisfy the borrow checker; `Deref` for inheritance;
`#![deny(warnings)]` in source (the `-D warnings` flag lives in CI, so a new compiler does
not break local builds).
