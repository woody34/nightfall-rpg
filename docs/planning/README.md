# Nightfall Planning Docs

Implementation reference for every system in the game, organised by build phase. Each phase
depends on the ones before it. Read Phase 1 before anything else: race, class, combat, and items
all derive from the stat formulas defined there.

| Phase | Document | Scope |
|-------|----------|-------|
| 0 | [00-foundations.md](00-foundations.md) | Architecture, networking, persistence, data pipeline, accounts |
| 1 | [01-stat-formulas.md](01-stat-formulas.md) | Base stats, derived stats, XP curves |
| 2 | [02-race-and-class.md](02-race-and-class.md) | Races, class tree, transfers, subclasses |
| 3 | [03-combat-and-skills.md](03-combat-and-skills.md) | Combat core, skills, buffs, status, elements, aggro, death, PvP, pets |
| 4 | [04-items-and-equipment.md](04-items-and-equipment.md) | Weapons, armor, grades, consumables, enchant, augment, inventory |
| 5 | [05-economy-and-crafting.md](05-economy-and-crafting.md) | Drops, crafting, currency sinks, vendors, player trade |
| 6 | [06-world-and-content.md](06-world-and-content.md) | Zones, monsters and AI, travel, quests, raids, instances |
| 7 | [07-social-systems.md](07-social-systems.md) | Party, clan, alliance, sieges, Olympiad |
| 8 | [08-client-presentation.md](08-client-presentation.md) | Camera, UI, animation, audio, localization |
| 9 | [09-live-operations.md](09-live-operations.md) | GM tools, anti-cheat, patching, telemetry, events |

## Document structure

Every phase document follows the same outline so it can be used as a checklist during implementation:

1. **Purpose and scope** — what the phase delivers and what it explicitly excludes.
2. **Reference: how Lineage 2 does it** — concrete mechanics, formulas, and numbers, with sources.
3. **Design decisions for Nightfall** — what we will do, alternatives considered, and why.
4. **Data model** — entities, fields, and proto message sketches for `packages/proto`.
5. **Interfaces** — gRPC services, server events, and what the client needs.
6. **Rust implementation notes** — crates, module layout under `apps/api`, concurrency concerns.
7. **Client implications** — what the Phaser client must render or send.
8. **Open questions** — decisions deferred to implementation time.
9. **Sources** — links used for the research.

## Conventions

- Stack: Rust + axum (HTTP) + tonic (gRPC) server, Unreal Engine 5 thin client (C++), protobuf contracts in `packages/proto`.
- Server is authoritative for everything. The client renders and sends intent only.
- Lineage 2 is the reference, not the target. Where its numbers are quoted they are a starting point for tuning, not a spec.

## Engineering standards

How the code is written is governed by [docs/engineering](../engineering/README.md): clean
architecture layers, the NATS event-bus core, idempotent endpoints, atomic transactions, the
per-endpoint test matrix, and the Rust standard enforced by rustfmt, clippy, workspace lints,
and cargo-deny. Those documents win over anything in a phase document about code structure.

## Cross-cutting decisions

These were made in one phase and bind the others. If a phase document disagrees, the owner listed
here wins and the other document is the one to fix.

| Decision                     | Owner                                | Summary |
|------------------------------|--------------------------------------|---------|
| Client engine                | research/engine-comparison.md §0     | Unreal Engine 5 thin client. No replication, no GAS, no UE server: pawns are driven from server snapshots over the Phase 0 WebSocket. Phaser retired 2026-10-07. |
| Real-time transport          | Phase 0 §3.2                         | One WebSocket at `GET /ws?ticket=…` on the axum listener, one protobuf `ClientMessage` / `ServerMessage` per binary frame. gRPC (tonic) is for request/response only; the Unreal client calls it natively via TurboLink. |
| Tick rate                    | Phase 0 §3.2                         | 100 ms server tick (10 Hz), 10 Hz world deltas to clients, client interpolates remote entities over 100-200 ms. |
| Game data format             | Phase 0 §3.4                         | TOML in `packages/data/`, loaded at boot into immutable structs behind `ArcSwap`, validated with cross-reference checks, hot-reloaded in dev. |
| Persistence                  | Phase 0 §3.3                         | Postgres via `sqlx`. Write-behind for volatile character state, write-through transactions for anything that moves items or currency. Every item mutation is appended to `item_ledger`. |
| Randomness                   | Phase 3 §3.3, Phase 5 §3             | Per-zone seeded `ChaCha12` streams so combat and loot rolls are reproducible from a logged seed. |
| Stat formulas                | Phase 1 §3                           | High Five formulas and constants unless a phase doc says otherwise. All numbers quoted from Lineage 2 are tuning starting points. |
| Event bus                    | engineering/architecture.md §2       | NATS. Commands on `nightfall.cmd.<aggregate>.<command>`, events on `nightfall.<aggregate>.<event>`. Events are staged in a transactional `outbox` table in the same transaction as the state change. |
| Idempotency                  | engineering/api-guidelines.md §2     | Every mutating RPC carries a client UUID `idempotency_key`; the key, a request fingerprint, and the result are stored atomically with the write. |
| Code layout                  | engineering/architecture.md §1       | `domain` / `application` / `infrastructure` / `interface` under `apps/api/src`, dependencies inward only. Supersedes the module tree sketched in Phase 0 §6. |
| Proto layout                 | Phase 0 §5                           | `packages/proto/nightfall/v1/`: `game.proto` (shared messages, Ping, Character), plus one file per domain as the phases introduce them: `world.proto`, `combat.proto`, `items.proto`, `economy.proto`, `social.proto`, `admin.proto`. |
| Identity                     | plans/phase-0b-connected-slice.md §2 | Keycloak self-hosted in Compose with realm as code; client authenticates via OAuth 2.0 Device Authorization Grant and the server validates JWTs without storing credentials. |
| ORM                          | plans/phase-0b-connected-slice.md §2 | SeaORM on the existing sqlx pool with `sea-orm-migration` and CI-generated entities, preserving transactional repository ports. |
| Session event log and replay | plans/phase-0b-connected-slice.md §2, §8 | NATS JetStream. The zone actor is the single writer of the replay log (`nightfall.zone.<zone>.<epoch>.applied`, one acknowledged record per tick); per-session `.in`/`.out` logs are for audit. Replay re-runs the zone actor from a tick-boundary snapshot and asserts byte-identical output. |
| Telemetry backend            | plans/phase-0b-connected-slice.md §2 | Grafana LGTM all-in-one in Compose, ingesting OTLP traces, metrics, and structured logs exported from the server. |
| Determinism                  | plans/phase-0b-connected-slice.md §2 | Fixed 100 ms tick with `i32` fixed-point coordinates (1/1000 tile), tick-stamped commands, and per-zone seeded `ChaCha12` RNG to guarantee bit-exact replay. |

## Research caveats

The research agents could not reach several primary sources (the official L2J repositories moved and
the wikis block automated fetches). Where a number comes from memory rather than a fetched file,
the document says so in its Open questions section with a "verify against repo" note. Treat those
as the first things to check when the relevant phase starts.
