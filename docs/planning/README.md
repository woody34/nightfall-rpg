# Nightfall Planning Docs

This is the implementation reference for all systems in the game. The documents are in build-phase
sequence. Each phase uses the phases before it. Read Phase 1 first: the race, class, combat, and item
systems all come from the stat formulas in that document.

| Phase | Document | Scope | Status |
|-------|----------|-------|--------|
| 0 | [00-foundations.md](00-foundations.md) | Architecture, networking, persistence, data pipeline, accounts | Complete |
| 1 | [01-stat-formulas.md](01-stat-formulas.md) | Base stats, derived stats, XP curves | Complete |
| 2 | [02-race-and-class.md](02-race-and-class.md) | Races, class tree, transfers, subclasses | Complete |
| 3 | [03-combat-and-skills.md](03-combat-and-skills.md) | Combat core, skills, buffs, status, elements, aggro, death, PvP, pets | Complete |
| 4 | [04-items-and-equipment.md](04-items-and-equipment.md) | Weapons, armor, grades, consumables, enchant, augment, inventory | Planned |
| 5 | [05-economy-and-crafting.md](05-economy-and-crafting.md) | Drops, crafting, currency sinks, vendors, player trade | Planned |
| 6 | [06-world-and-content.md](06-world-and-content.md) | Zones, monsters and AI, travel, quests, raids, instances | Planned |
| 7 | [07-social-systems.md](07-social-systems.md) | Party, clan, alliance, sieges, Olympiad | Planned |
| 8 | [08-client-presentation.md](08-client-presentation.md) | Camera, UI, animation, audio, localization | Planned |
| 9 | [09-live-operations.md](09-live-operations.md) | GM tools, anti-cheat, patching, telemetry, events | Planned |

## Document structure

All phase documents have the same outline. Thus, you can use each document as a checklist during implementation:

1. **Purpose and scope** — what the phase gives, and what the phase does not include.
2. **Reference: how Lineage 2 does it** — the mechanics, formulas, and numbers of Lineage 2, with sources.
3. **Design decisions for Nightfall** — what we will do, the alternatives that we examined, and the reasons.
4. **Data model** — entities, fields, and proto message sketches for `packages/proto`.
5. **Interfaces** — gRPC services, server events, and the data that the client must have.
6. **Rust implementation notes** — crates, the module layout under `apps/api`, and concurrency problems.
7. **Client implications** — what the Phaser client must render or send.
8. **Open questions** — decisions that we will make at implementation time.
9. **Sources** — the links that we used for the research.

### Phase plan template

The phase plans under `docs/plans/` have this outline:

- **Goal**
- **Decisions table**
- **Architecture**
- **Epics with Story/Tasks/Model/Done-when tables incl. the simulation story**: during plan authoring, give the tag `[client-visible]` to each story that sets a client-observable contract or state behavior. Do this independently, before you write the scenarios.
- **Order and estimate**
- **Risks**
- **Out of scope**
- **Outcome**

## Conventions

- Stack: Rust + axum (HTTP) + tonic (gRPC) server, Unreal Engine 5 thin client (C++), protobuf contracts in `packages/proto`.
- The server is authoritative for all data. The client only renders and sends intent.
- Lineage 2 is the reference, not the target. If a document quotes a Lineage 2 number, that number is a start point for tuning. It is not a specification.
- Client-observable contract and state behavior: each story that sets gameplay or client contracts has the literal tag `[client-visible]` in its story title.
- A new phase plan must add the `[client-visible]` tags independently, before you write the scenarios.
- The simulation traceability checker (`apps/client-unreal/Scripts/sim-traceability.sh`) examines all completed (`✅`) tagged stories for scenario coverage. The coverage comes from the `# covers:` headers in the `.nfs` files under `apps/client-unreal/Scenarios/`.
- Rendering-only parts of a story keep the automation test and the asset-cooking validation. But these stories keep their tags. The checker reports all cases that have no coverage, and does not omit them.
- From Phase 2 on, each client-facing epic in each phase plan has a story 'Simulation scenario(s)'. The Done-when of that story names the `.nfs` files under `apps/client-unreal/Scenarios/`.
- [docs/plans/phase-1a-simulation-testing.md](../plans/phase-1a-simulation-testing.md) §4.1 lists the first scenario that each phase must release.

## Engineering standards

[docs/engineering](../engineering/README.md) sets the rules for how we write the code. These rules include:

- The clean architecture layers.
- The NATS event-bus core.
- Idempotent endpoints.
- Atomic transactions.
- The per-endpoint test matrix.
- The Rust standard, which rustfmt, clippy, workspace lints, and cargo-deny apply.

If a phase document and these documents do not agree about code structure, the engineering documents are correct.

See [docs/plans/](../plans/) for the phase plans. These phase plans are complete: Phase 0b, Phase 1 ([phase-1-kill-a-monster.md](../plans/phase-1-kill-a-monster.md)), Phase 2 ([phase-2-race-and-class.md](../plans/phase-2-race-and-class.md)), and Phase 3 ([phase-3-combat-and-skills.md](../plans/phase-3-combat-and-skills.md)). Phase 1a ([phase-1a-simulation-testing.md](../plans/phase-1a-simulation-testing.md)) is in progress. Phase 1a builds a headless Unreal bot that runs scripted gameplay scenarios in CI. It also adds these scenarios to Phases 0b–1 after the work. And it makes a simulation scenario a necessary story of each subsequent client epic (see E5.1 in that plan).
See [docs/decisions/phase-1-combat.md](../decisions/phase-1-combat.md) for the Phase 1 combat decision record.

## Cross-cutting decisions

We made each of these decisions in one phase, and they apply to all the other phases. If a phase document does not agree, the owner in this table is correct. You must then correct the other document.

| Decision                     | Owner                                | Summary |
|------------------------------|--------------------------------------|---------|
| Client engine                | research/engine-comparison.md §0     | Unreal Engine 5 thin client. No replication, no GAS, no UE server: server snapshots move the pawns through the Phase 0 WebSocket. Phaser retired 2026-10-07. |
| Real-time transport          | Phase 0 §3.2                         | One WebSocket at `GET /ws?ticket=…` on the axum listener, one protobuf `ClientMessage` / `ServerMessage` for each binary frame. gRPC (tonic) is only for request/response. The Unreal client calls gRPC natively through TurboLink. |
| Tick rate                    | Phase 0 §3.2                         | 100 ms server tick (10 Hz), 10 Hz world deltas to clients. The client interpolates remote entities over 100-200 ms. |
| Game data format             | Phase 0 §3.4                         | TOML in `packages/data/`. The server loads it at boot into immutable structs behind `ArcSwap`, validates it with cross-reference checks, and hot-reloads it in dev. |
| Persistence                  | Phase 0 §3.3                         | Postgres through `sqlx`. Write-behind for volatile character state. Write-through transactions for all changes that move items or currency. Each item mutation is appended to `item_ledger`. |
| Randomness                   | Phase 3 §3.3, Phase 5 §3             | Per-zone seeded `ChaCha12` streams. Thus, you can replay combat and loot rolls from a logged seed and get the same results. |
| Stat formulas                | Phase 1 §3                           | High Five formulas and constants, if a phase doc does not set a different value. All numbers quoted from Lineage 2 are start points for tuning. |
| Event bus                    | engineering/architecture.md §2       | NATS. Commands on `nightfall.cmd.<aggregate>.<command>`, events on `nightfall.<aggregate>.<event>`. Events go into a transactional `outbox` table in the same transaction as the state change. |
| Idempotency                  | engineering/api-guidelines.md §2     | Each mutating RPC has a client UUID `idempotency_key`. The server stores the key, a request fingerprint, and the result atomically with the write. |
| Code layout                  | engineering/architecture.md §1       | `domain` / `application` / `infrastructure` / `interface` under `apps/api/src`, with dependencies only inward. This replaces the module tree in Phase 0 §6. |
| Proto layout                 | Phase 0 §5                           | `packages/proto/nightfall/v1/`: `game.proto` (shared messages, Ping, Character), and one file for each domain when a phase adds it: `world.proto`, `combat.proto`, `items.proto`, `economy.proto`, `social.proto`, `admin.proto`. |
| Identity                     | plans/phase-0b-connected-slice.md §2 | Keycloak self-hosted in Compose, with the realm as code. The client authenticates through the OAuth 2.0 Device Authorization Grant. The server validates JWTs and does not store credentials. |
| ORM                          | plans/phase-0b-connected-slice.md §2 | SeaORM on the sqlx pool that exists, with `sea-orm-migration` and CI-generated entities. The transactional repository ports stay the same. |
| Session event log and replay | plans/phase-0b-connected-slice.md §2, §8 | NATS JetStream. The zone actor is the only writer of the replay log (`nightfall.zone.<zone>.<epoch>.applied`, one acknowledged record for each tick). The per-session `.in`/`.out` logs are for audit. Replay runs the zone actor again from a tick-boundary snapshot and makes sure that the output is byte-identical. |
| Telemetry backend            | plans/phase-0b-connected-slice.md §2 | Grafana LGTM all-in-one in Compose. It ingests the OTLP traces, metrics, and structured logs that the server exports. |
| Determinism                  | plans/phase-0b-connected-slice.md §2 | Fixed 100 ms tick with `i32` fixed-point coordinates (1/1000 tile), tick-stamped commands, and per-zone seeded `ChaCha12` RNG. These make sure that replay is bit-exact. |

## Research caveats

The research agents could not get access to some primary sources. The official L2J repositories moved,
and the wikis block automated fetches. Some numbers come from memory and not from a fetched file.
For these numbers, the Open questions section has a "verify against repo" note. When the
applicable phase starts, examine those numbers first.
