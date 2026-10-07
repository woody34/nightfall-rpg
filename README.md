# Nightfall RPG

A Lineage 2-inspired MMORPG. Monorepo managed by [moon](https://moonrepo.dev).

## Layout

```
apps/
  api/        Rust game server: axum (HTTP) + tonic (gRPC)
  client/     Low-fidelity Phaser 3 + Vite + TypeScript client
packages/
  proto/      Shared Protobuf contracts (nightfall.v1)
.moon/        Workspace, toolchains, and inherited task config (moon 2.x)
```

## Prerequisites

Install [proto](https://moonrepo.dev/docs/proto/install), then from the repo root:

```bash
proto use
```

That installs the pinned versions of moon, Node, pnpm, and Rust from `.prototools`.
No system `protoc` is needed: the API compiles `.proto` files with `protox` at build time.

## Local infrastructure

```bash
docker compose up -d      # Postgres 16 on :5432, NATS 2.11 on :4222
cp .env.example .env      # DATABASE_URL / NATS_URL for the API
```

Without `DATABASE_URL` / `NATS_URL` the API runs with in-memory adapters and warns at startup.

## Running

```bash
# install JS deps
pnpm install

# API on http://localhost:3000 (REST) and localhost:50051 (gRPC)
moon run api:dev

# client on http://localhost:5173 (proxies /api -> :3000)
moon run client:dev
```

## Useful tasks

```bash
moon run api:check        # cargo check
moon run api:test         # cargo test
moon run api:lint         # clippy
moon run client:build     # vite build -> apps/client/dist
moon run client:typecheck # tsc --noEmit
moon check --all          # run every build/test/lint task in the workspace
```

## Proto workflow

Edit `packages/proto/nightfall/v1/*.proto`. The Rust server regenerates bindings on the next
`cargo build` (see `apps/api/build.rs`). Client bindings are not generated yet; the client currently
talks to the REST `/health` endpoint only.

## Documentation

- [docs/planning](docs/planning/README.md): what to build, per phase.
- [docs/engineering](docs/engineering/README.md): how to build it. Rust standard, architecture, API and database rules.

## Notes

- This repo uses **moon 2.x**. Config file names and fields differ from moon 1.x docs and examples
  (`.moon/toolchains.yml` is plural, `layer` replaces `type`, `preset: server` replaces `local: true`).
- The first `proto use` installs moon, Node, pnpm, and Rust under `~/.proto`. Nothing is installed system-wide.
