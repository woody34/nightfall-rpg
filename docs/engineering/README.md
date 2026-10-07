# Engineering Guidelines

How we build Nightfall. These are binding for every contribution; the planning docs in
`../planning` say *what* to build, these say *how*.

| Document | Covers |
|----------|--------|
| [rust-guidelines.md](rust-guidelines.md) | The Rust standard: what is enforced as code (rustfmt, clippy, workspace lints, cargo-deny) and what is reviewed by hand. |
| [architecture.md](architecture.md) | Clean architecture layers, the event-bus core on NATS, command vs event, the transactional outbox. |
| [api-guidelines.md](api-guidelines.md) | Endpoint rules: idempotency, error mapping, and the test every endpoint must ship with. |
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
