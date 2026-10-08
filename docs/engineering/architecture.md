# Architecture

## 1. Layers

Clean architecture, four layers, dependencies point inward only:

```
apps/api/src/
  domain/          Entities, value objects, domain events, domain errors.
                   No framework, DB, transport, or runtime imports.
  application/     Use cases (one per endpoint) and ports (traits the use cases need).
                   Depends on domain only.
  infrastructure/  Adapters implementing ports: memory, postgres (sqlx), nats.
                   Depends on application and domain.
  interface/       Inbound adapters: http (axum), grpc (tonic). Parse, call one use case,
                   map result/error to the wire. Depends on application and domain.
  lib.rs           `Dependencies` (ports bound to adapters) and server builders.
  main.rs          Composition root: config -> adapters -> servers.
```

The rule, from Kigawas: routers thin, use cases thick, models slim. A handler that contains an
`if` about business state is in the wrong layer.

### 1.1 What goes where

| Question | Layer |
|----------|-------|
| Is a name valid? | domain (`CharacterName::new`) |
| What happens when a character is created? | application (`CreateCharacter::execute`) |
| How is a character stored? | infrastructure (`PgCharacterRepository`) |
| What gRPC code does "name taken" map to? | interface (`to_status`) |
| What stats does a Dwarf start with? | domain, from data files once Phase 1 lands |

### 1.2 Ports

`application::ports` defines:

- `CharacterRepository`: `get`, `create_idempotent`. Each method is one atomic unit of work.
- `EventBus`: `publish(&DomainEvent)`.
- `Clock`: `now()`.

Adapters are chosen at the composition root and injected as `Arc<dyn Port>` via
`Dependencies`. Tests build `Dependencies` with in-memory adapters; `main` builds it from
`DATABASE_URL` and `NATS_URL`, falling back to in-memory with a warning when either is unset.

## 2. The event-bus core

Following jdno's design: the API does not mutate the world directly. Two buses connect three
independent systems.

```
  client ──gRPC/WS──▶ API (interface + application)
                        │ commands                     ▲ events
                        ▼                              │
                    command bus ──▶ world simulation ──▶ event bus ──▶ API, projections,
                    (NATS)            (tick loop)         (NATS)        chat, telemetry
```

- **Command**: an intent that may be rejected. `MoveTo`, `UseSkill`, `CreateCharacter`.
  Validated by a use case, then either applied in a transaction (account/character data) or
  forwarded to the simulation (world state).
- **Event**: a fact that already happened. `CharacterCreated`, `EntityMoved`, `SkillCast`.
  Published after the state change is durable. Consumers are idempotent because delivery is
  at-least-once.

### 2.1 Why NATS

- Core NATS gives fan-out pub/sub with subject wildcards, which is exactly the shape of
  "every system that cares about character events subscribes to `nightfall.character.>`".
- JetStream adds persistence and replay when a consumer needs it (projections, audit), without
  changing publishers.
- One binary, trivial to run locally (`docker compose up -d`), Rust client is first-party
  (`async-nats`).

### 2.2 Subjects

`nightfall.<aggregate>.<event>` for events, `nightfall.cmd.<aggregate>.<command>` for commands.
Snake case. The subject is a method on the event (`DomainEvent::subject`), so it cannot drift
from the type. Payload is JSON with a `type` tag today; switch to protobuf on the bus when a
non-Rust consumer appears.

### 2.3 Transactional outbox

A process can die between committing a transaction and publishing its event. So:

1. The repository inserts the event into `outbox` in the same transaction as the state change.
2. The use case does not publish. The relay (`infrastructure/outbox/relay.rs`) is the only
   publisher of domain events, so there is a single path and no duplicate-publish race.
3. The relay polls `outbox WHERE published_at IS NULL ORDER BY id LIMIT 100 FOR UPDATE SKIP
   LOCKED` in a transaction (250 ms poll, exponential backoff to 5 s on failure), publishes each
   row to `JetStream` stream `NF_EVENTS` (`nightfall.>`) and waits for the ack, sending
   `Nats-Msg-Id = outbox.id` so the broker drops a retry after a crash between publish and
   mark. `published_at` is set only after the ack. Several relays can run side by side.
   ``RelayStats` exposes `outbox_pending` and the last publish lag for telemetry.

Delivery is at-least-once with broker-side dedupe inside the stream's duplicate window;
consumers still dedupe on the event's aggregate id and a sequence.

### 2.4 The world simulation

Lives in-process for now (Phase 0 §6: single world thread, bounded `mpsc`, 100 ms tick). It is
a consumer of commands and a producer of events like everything else, which is what lets it
move to its own process later without changing the API.

## 3. Request lifecycle: `CreateCharacter`

1. `interface::grpc` parses `idempotency_key`, `account_id`, `race` into typed values. Any
   failure is `INVALID_ARGUMENT` and nothing else runs.
2. `CreateCharacter::execute` validates the name (domain), builds the aggregate, computes the
   idempotency fingerprint.
3. `PgCharacterRepository::create_idempotent` runs one transaction: claim key, insert
   character, stage outbox row, commit. Unique violations map to typed errors.
4. On `Created`, the use case publishes `CharacterCreated` on NATS. A publish failure is logged,
   not returned, because the outbox will deliver it.
5. The interface maps the result to `nightfall.v1.Character` or a status code.

Every step has a test: domain constructors (unit), use case with in-memory ports (unit),
repository against Postgres (adapter), the endpoint through a real socket (integration).

## 4. Adding a feature

1. Domain: add or extend the entity and its invariants. Unit tests.
2. Proto: add the RPC and messages. Mutating RPCs get `idempotency_key`.
3. Application: add a port method if needed, then one use case. Unit tests with memory adapters.
4. Infrastructure: implement the port in `memory` and `postgres`. Adapter tests.
5. Interface: wire the RPC to the use case. Integration tests per `api-guidelines.md`.
6. If the feature emits an event: add a `DomainEvent` variant, stage it in the outbox.
