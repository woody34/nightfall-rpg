# Nightfall RPG

An MMORPG that comes from the ideas of Lineage 2. This monorepo uses [moon](https://moonrepo.dev).

## Status

The prototype has a complete combat loop (login, zone, click-to-move, auto-attack keltirs, death/respawn, XP/level) with animated monsters (status on 2026-10-09). Players can do these steps:

- Authenticate through the Keycloak device authorization.
- Make a character.
- Get a play ticket that they can use one time.
- Join a zone over WebSocket.
- Move with click-to-move and server reconciliation.
- Fight keltirs with deterministic replay logging.

These documents give the architecture, the verification details, and the combat decisions:

- The [Phase 1 plan](docs/plans/phase-1-kill-a-monster.md).
- The [Phase 1 combat decision record](docs/decisions/phase-1-combat.md).
- The [Phase 0b plan](docs/plans/phase-0b-connected-slice.md).
- [docs/diagrams/README.md](docs/diagrams/README.md).

## Layout

```
apps/
  api/        Rust game server: axum (HTTP) + tonic (gRPC)
  client-unreal/  Unreal Engine 5 thin client (C++), see its README
packages/
  proto/      Shared Protobuf contracts (nightfall.v1)
.moon/        Workspace, toolchains, and inherited task config (moon 2.x)
```

## Prerequisites

Install [proto](https://moonrepo.dev/docs/proto/install). Then, from the repo root, run:

```bash
proto use
```

This command installs the pinned versions of moon and Rust from `.prototools`. You install Unreal Engine separately. Refer to `apps/client-unreal/README.md`.
A system `protoc` is not necessary: the API compiles `.proto` files with `protox` when it builds.

## Local infrastructure

```bash
docker compose up -d      # Postgres 16 on :5432, NATS 2.11 on :4222, Grafana LGTM on :3300, Keycloak on :8080
cp .env.example .env      # DATABASE_URL / NATS_URL for the API
```

Keycloak (OIDC, device flow login) imports the `nightfall` realm when it starts. The admin console is at http://localhost:8080 (admin/admin). The dev user is `testplayer`/`testplayer`. If you use a Postgres volume that exists, make the Keycloak database one time: `docker compose exec postgres psql -U nightfall -c 'CREATE DATABASE keycloak'`. Refer to `infra/keycloak/README.md`.

If `DATABASE_URL` / `NATS_URL` are not set, the API runs with in-memory adapters and gives a warning when it starts.

## Running

```bash
# API on http://localhost:3000 (REST) and localhost:50051 (gRPC)
moon run api:dev
```

To open the Unreal client, open `apps/client-unreal/Nightfall.uproject` in Unreal Editor.

## Observability

`docker compose up -d` also starts `grafana/otel-lgtm` (decision D6). This container has Grafana, Tempo, Loki, Prometheus and an
OpenTelemetry collector. Grafana is at <http://localhost:3300>
(`admin` / `admin`). OTLP gRPC is on `:4317`, and OTLP HTTP is on `:4318`.

When `OTEL_EXPORTER_OTLP_ENDPOINT` is set, the API exports traces, logs and metrics over OTLP
(refer to `.env.example`). If it is not set, no data goes out of the process. Logs still go to stdout (JSON, or
a format that is easy for persons to read with `TELEMETRY_PRETTY=1`). `GET /metrics` still serves Prometheus text.

| Signal | Where |
|--------|-------|
| Traces | Grafana -> Explore -> Tempo, `{resource.service.name="nightfall-api"}`. One root span for each HTTP request (`http.request`) or RPC (`grpc.request` with `rpc.service`, `rpc.method`, `account_id`). All root spans have `request_id` (uuid v7, also the `x-request-id` response header). |
| Logs | Explore -> Loki, `{service_name="nightfall-api"}`. The OpenTelemetry logs bridge sends them over OTLP. |
| Metrics | Explore -> Prometheus, or scrape `http://localhost:3000/metrics`. The catalogue is in `infrastructure/telemetry/metrics.rs`. All names start with `nightfall_`. |
| Dashboard | "Nightfall API", provisioned from `infra/grafana/dashboards/`. |
| Alerts | Provisioned from `infra/grafana/alerts/`: tick p99 > 50 ms, a dropped WebSocket frame in 5 m, outbox lag > 30 s. |

Some metrics do not have producers yet (tick loop, sessions, WebSocket frames, event log,
outbox relay). These metrics are defined, and they show "No data" until those stories merge. The outbox gauges read
from the `OutboxStatsSource` hook. The relay registers itself with `Metrics::set_outbox_source`.
To move a trace across a queue (socket -> zone actor -> broadcast), put a `TraceCarrier` in the
message. Refer to `infrastructure/telemetry/carrier.rs`.

Requests record **only the URL path**. They never record the query string or the headers, because tickets and tokens must
not go into telemetry. `tests/span_secrets.rs` enforces this rule.

To verify the pipeline without a browser, start the API with the OTLP endpoint set. Then run:

```bash
docker compose up -d lgtm
infra/scripts/verify-observability.sh      # trace, Loki line, metric sample, /metrics scrape
```

The script runs these commands against the Grafana datasource proxy:

```bash
curl -s -u admin:admin -G localhost:3300/api/datasources/proxy/uid/tempo/api/search \
  --data-urlencode 'q={resource.service.name="nightfall-api"}'
curl -s -u admin:admin -G localhost:3300/api/datasources/proxy/uid/loki/loki/api/v1/query_range \
  --data-urlencode 'query={service_name="nightfall-api"}' --data-urlencode limit=1
curl -s -u admin:admin -G localhost:3300/api/datasources/proxy/uid/prometheus/api/v1/query \
  --data-urlencode 'query=nightfall_http_requests_total{route="/health"}'
```

To see an alert fire, push a synthetic slow tick histogram and poll the rule state. The rule state is pending for
approximately a minute, then it changes to firing:

```bash
infra/scripts/push-synthetic-tick.sh 300 &
curl -s -u admin:admin localhost:3300/api/prometheus/grafana/api/v1/rules | grep -o '"state":"[a-z]*"'
```

## Useful tasks

```bash
moon run api:check        # cargo check
moon run api:test         # cargo test
moon run api:lint         # clippy
moon check --all          # run every build/test/lint task in the workspace
```

## Proto workflow

Change `packages/proto/nightfall/v1/*.proto`. The Rust server makes the bindings again on the next
`cargo build` (refer to `apps/api/build.rs`). `apps/client-unreal/Scripts/gen-proto.sh` makes the C++ bindings for the Unreal client.
This script must have `protoc`.

## Documentation

- [docs/planning](docs/planning/README.md): what to build, for each phase.
- [docs/engineering](docs/engineering/README.md): how to build it. The Rust standard, the architecture, and the API and database rules.
- [docs/research](docs/research/README.md): decision research. Now, this is the comparison of client engines.

## Notes

- This repo uses **moon 2.x**. The config file names and fields are different from the moon 1.x docs and examples
  (`.moon/toolchains.yml` is plural, `layer` replaces `type`, `preset: server` replaces `local: true`).
- The first `proto use` installs moon, Node, pnpm, and Rust under `~/.proto`. It installs nothing for the full system.
