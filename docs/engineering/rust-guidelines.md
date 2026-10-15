# Rust Guidelines

This document has the same function as a Roslyn `.editorconfig` and analyzer ruleset. The
toolchain enforces section 1. Reviewers examine the rules in section 2 and the sections after it.

## 1. Enforced as code

### 1.1 Formatting: `rustfmt.toml`

The rules are: 100-column lines, 4-space indent, Unix newlines, field-init and `?` shorthand, and sorted imports and
modules. The file uses only stable options, thus it runs on the pinned stable toolchain. Run `cargo fmt --all` before
each commit. CI runs `cargo fmt --check` and fails on a diff. Never argue about formatting in
review: if rustfmt accepts the code, the code is formatted.

### 1.2 Lint levels: `[workspace.lints]` in the root `Cargo.toml`

Each crate opts in with `[lints] workspace = true`. The policy, in order of severity:

| Level | What | Why |
|-------|------|-----|
| `forbid` | `unsafe_code` | No part of the game server must have it. If a crate must have it, that crate gets its own audited rule, with `SAFETY:` comments on each block. |
| `deny` | `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`, `print_stdout`, `print_stderr`, `float_cmp`, `mem_forget`, `undocumented_unsafe_blocks`, `unused_must_use` | A game server must not crash on bad input. Return `Result`. Log with `tracing`. |
| `warn` (fails CI through `-D warnings`) | `clippy::all`, `clippy::pedantic`, `missing_docs`, `unreachable_pub`, `indexing_slicing`, `arithmetic_side_effects`, `wildcard_enum_match_arm`, and the other lints | Pedantic finds real bugs in code with a lot of math. The three named lints make an explicit overflow policy and exhaustive matching on our own enums necessary. |
| `allow` | `module_name_repetitions`, `must_use_candidate`, `missing_errors_doc`, `missing_panics_doc`, `cast_precision_loss` | Each has a one-line reason in `Cargo.toml`. Add to this list only with a reason. |

Tests relax `unwrap_used`, `expect_used` and `indexing_slicing` at module level:

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests { ... }
```

In all other locations, suppress a lint with the narrowest `#[allow(...)]` on the item. Add a comment
that gives the reason. Never make the workspace list wider for one call site.

### 1.3 Thresholds and disallow-lists: `clippy.toml`

The limits are: cognitive complexity 20, 6 arguments, 120 lines per function. `std::sync::Mutex` is disallowed.
Use `tokio::sync::Mutex` (async) or `parking_lot::Mutex` (short sync sections) for it.
`std::env::set_var`, `std::process::exit` and ad-hoc `spawn_blocking` are disallowed. The file gives the
reasons.

### 1.4 Supply chain: `deny.toml`

The rules are: permissive licenses only, no OpenSSL (rustls everywhere), no wildcard versions and no unknown
registries. RustSec advisories cause the build to fail. `Cargo.lock` is committed. `rust-version` is
pinned in the workspace and is the same as in `.prototools`.

## 2. Types

- **Newtype every identifier and unit.** `CharacterId(Uuid)`, `IdempotencyKey(Uuid)`,
  `CharacterName(String)`. A caller can swap the arguments of a function that takes two `Uuid`s. A caller cannot swap
  the arguments of a function that takes a `CharacterId` and an `AccountId`.
  Declare UUID ids with `uuid_id!` (`domain/ids.rs`). Never write them by hand. Parse them at the
  transport edge with `application::parse_id("field", raw)`. This gives the uniform error
  `InvalidArgument("field must be a UUID")`. Add the `SeaORM` conversions in
  `infrastructure/postgres/mod.rs` through `uuid_id_sea_orm!`. Thus the domain has no persistence code.
- **Validate in constructors.** `CharacterName::new` is the only way to get a `CharacterName`.
  Thus each instance has 3-16 ASCII letters. Code downstream never does the check again.
- **Use enums, not booleans and sentinels.** `Race`, not `u8`. `CreateOutcome::{Created, Replayed,
  KeyReused}` not `(bool, Option<Character>)`.
- **Put `#[non_exhaustive]` on enums that will grow** (`DomainEvent`). Thus a new variant is not a
  breaking change for downstream matches inside the crate.
- **Naming**: `into_x` consumes, `to_x` can allocate, `as_x` borrows. Use `parse` for string constructors that can fail.
  Use `new` for constructors that cannot fail or that validate.

## 3. Errors

- Each layer has one error type: `domain::character::NameError` (what), `application::AppError`
  (why, coarse), and the transport status (what to tell the client). Inner errors convert outward with
  `From`. Infrastructure errors become `AppError::Infrastructure(anyhow::Error)`. The server logs them and
  never shows them to clients.
- Use `thiserror` for typed errors. Use `anyhow` only at the infrastructure boundary and in `main`.
- Use `?` for propagation. If a retry or rollback policy applies, write it in full. Do not hide it in
  a `map_err`.
- Panics are only for programmer defects, and the lints deny the frequently used macros. If an invariant
  truly cannot be violated, encode it in a type. Do not assert it.

## 4. Ownership and concurrency

- Borrow by default. Use `&str`, not `&String`. Use `&[T]`, not `&Vec<T>`.
- Use static dispatch in hot paths (the tick loop, stat math). Use `Arc<dyn Trait>` only at the
  composition root, where the code selects adapters at runtime. `Dependencies` does this.
- Use one owner and message passing. The world simulation controls its state and receives commands
  on a channel (Phase 0 §6). Do not use `Arc<Mutex<World>>`.
- Put a limit on each queue. Use `mpsc::channel(256)`, not `unbounded_channel`. Write the backpressure
  behaviour in a comment.
- Use `tokio::sync::Mutex` across awaits. Use `parking_lot::Mutex` for short critical sections.
  Never use `std::sync::Mutex` (disallowed).

## 5. Modules and visibility

- Layers are modules under `apps/api/src/`: `domain`, `application`, `infrastructure` and
  `interface`. Dependencies point only inward. See `architecture.md`.
- Start private. Use `pub(crate)` before `pub`. `unreachable_pub` gives a warning when the code cannot
  really reach a `pub` item.
- Put one use case in each file. Put one adapter in each file. `mod.rs` re-exports the public items.

## 6. Tests

- **Unit tests** are next to the code in `#[cfg(test)] mod tests`. They use in-memory adapters, and
  they never touch I/O. Each use case has them. Each domain constructor tests its limits
  (one below min, min, max, one above max).
- **Integration tests** are in `apps/api/tests/`. They boot the real axum and tonic servers on
  ephemeral ports through `common::TestApp`, and call each endpoint through a real socket.
  See `api-guidelines.md` for the mandatory matrix of tests for each endpoint.
- **Adapter tests** run against real Postgres and NATS only when `DATABASE_URL` / `NATS_URL` is set.
  If the services are not there, the tests skip (and pass). `docker compose up -d` gives the two services.
- Tests are deterministic: seeded RNG, fixed clocks (`Clock` port), and a new schema for each Postgres test.
- Write test names as sentences: `same_key_different_body_is_a_conflict`.

## 7. Determinism (`domain/zone`)

The zone simulation has the target of byte-for-byte replay (plan D7, §8 of
`docs/plans/phase-0b-connected-slice.md`). The same snapshot and the same applied-command log
must make the same `AppliedTick` records again. Rules for `domain/zone` and
`application/zone_actor`:

- **No floats.** Positions and lengths are `Fixed` (`i32`, 1/1000 tile). Distances compare
  exact squared integers (`u128`). Movement steps use integer math and a documented
  rounding rule (`Vec2Fixed::step_toward`), and the movement arrives exactly. The code converts wire floats one time,
  in `interface::zone_mapping`. It rounds to the nearest milli-tile after the finite/range checks.
- **No hash-ordered collections.** Use only `BTreeMap` / `BTreeSet` or sorted `Vec`s. Thus each
  iteration order is defined. The code emits events, per-player outputs and AOI diffs in entity-id
  and ordinal order.
- **Time comes only through `Tick`.** The zone never reads a clock. `server_time_ms` is
  `time_origin_ms + tick * 100`. The only clock read of the actor is a monotonic `Instant` for
  `TickStats::duration_micros`. This value never goes into state or output.
- **Randomness comes only through the injected generator.** There is one `ChaCha12Rng` for each zone. Its key comes from
  `(zone_id, epoch)` with a fixed byte layout. The snapshot has its full state (key, stream, word position).
  Player entity ids are character ids. All other ids come from the generator.
- **Each state change is a command.** A command gets an ordinal when the code makes its draft. A refused command
  makes a `Disposition`, never a silent drop. `AppliedTick` is the unit of the replay log.

The enforcement applies only to `domain/zone`. It does not apply to the actor or the full crate:

- `mod.rs` denies the Clippy lints `float_arithmetic`, `float_cmp`, `float_cmp_const`,
  `lossy_float_literal`, `cast_precision_loss`, `imprecise_flops` and `suboptimal_flops`.
  `api:lint` rejects operations that those lints find. This is not a full ban on Rust float types.
- `zone_sources_use_no_nondeterministic_apis` scans embedded source lines, and ignores the text after
  the first `//`. It rejects these substrings: `f32`, `f64`, `HashMap`, `HashSet`, `Instant`,
  `SystemTime`, `thread_rng`, `OsRng`, `rand::random` and `from_entropy`. This is a text
  check. It is not name resolution, and it is not an exhaustive ban on all nondeterministic APIs.
- `every_zone_source_is_scanned` compares the embedded list with the `.rs` files directly in
  `domain/zone`. You must add each new file in that directory to the list. The test does not scan subdirectories.
- `tests/zone_determinism.rs` does a check of these items: identical streams across actors/runs, replay of recorded drafts,
  snapshot round-trips and exact movement. These tests do not prove equivalence
  across all machines or builds.

## 8. Documentation

- `missing_docs` is on. Each public item has a doc comment that tells its purpose. For
  functions, the comment also tells what the errors mean.
- Comments tell *why*. Put invariants, the reasons for an algorithm and L2 references in
  comments. Put what the code does in the code.
- Each `TODO` refers to an issue or to a section of a planning doc.

## 9. Dependencies

- Declare each dependency one time in `[workspace.dependencies]`. Use `default-features = false` where the crate
  has heavy defaults (`sqlx`, `chrono`, `reqwest`), and list the features explicitly.
- Do not add a new dependency for a helper of less than approximately fifty lines.
- Use the crates that are already in the tree: `tokio`, `tonic`, `axum`, `sqlx`, `async-nats`, `serde`,
  `thiserror`, `anyhow`, `tracing`, `uuid`, `chrono`, `parking_lot`.

## 10. Patterns we use, by name

From the Rust Design Patterns catalogue:

| Pattern | Where |
|---------|-------|
| Newtype | all ids, keys, names |
| Constructor with validation | `CharacterName::new`, `IdempotencyKey::parse` |
| Strategy through trait objects at the edge | `CharacterRepository`, `EventBus`, `Clock` ports |
| Command | `CreateCharacterInput` is the command, `DomainEvent` is the fact |
| RAII guard | `sqlx::Transaction` does a rollback on drop if it is not committed |
| `#[non_exhaustive]` | `DomainEvent` |
| Default | `InMemory*` adapters, `Position` |

Anti-patterns that we reject:

- A clone only to satisfy the borrow checker.
- `Deref` for inheritance.
- `#![deny(warnings)]` in source. The `-D warnings` flag is in CI, thus a new compiler does
  not break local builds.
