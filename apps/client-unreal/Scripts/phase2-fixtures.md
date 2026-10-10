Phase2 scenario fixtures use the ordinary simulation wrappers:

```bash
Scripts/run-sim.sh --api start --artifacts Saved/TransferAttempt Scenarios/transfer.nfs
Scripts/run-sim-multi.sh --api start --artifacts Saved/ObserverAttempt \
  Scenarios/observer-a.nfs Scenarios/observer-b.nfs
python3 Scripts/run-sim-ci.py --fresh-stack --artifacts Saved/SimCIAttempt
```

Select a new, empty artifact directory for each attempt. Headers must be exact:

| Header | Unit | Seed contract |
|---|---|---|
| `# fixture: phase2-transfer` | One scenario, including reconnect | Sole owner: female Human base class 0, level 40, tokens 1/1, milestone mask 3 |
| `# fixture: phase2-transfer-observer` | Exactly matching `-a` / `-b` roles | A is the transfer owner; B has a distinct account and sole male Human base class 0 character at level 1 |
| `# fixture: phase2-transfer-missing-token` | One scenario | Sole owner: female Human base class 0, level 20, tokens 0/0, milestone mask 1 |

All fixture characters start at (126,126). Owner character ID is
`01970000-0000-7000-8000-000000000020`; observer ID is
`01970000-0000-7000-8000-000000000040`. Ordinary creation, rejected-transfer,
and racial scenarios have no fixture header. Independent creation matrix names
use `-01`, `-02`, `-03`; CI always treats `-a` / `-b` as paired roles and rejects
orphans or mismatched headers.

Phase2 requires `--api start` (CI requires `--fresh-stack`). The wrapper generates
a unique Compose project with fresh Postgres and JetStream volumes, reserves
four available loopback ports excluding defaults, runs `nightfall-migrate up`,
and calls the published seeder with the exact pack and account UUIDs before
starting the API. The seeder manifest is validated before startup. Every unit
gets a new `nf_phase2_fixture_*` database. A mixed single-wrapper sequence tears
down each Phase2 unit after recording export and gates, then provisions the
next unit afresh. Each role receives a mode-0600 token file in the private fixture directory.
The bot argument array carries `-DevTokenFile=<path>` and the supported
`-ini:Game:[/Script/Nightfall.NetSettings]:GrpcEndpoint=<owned host:port>` override.
No complete developer token appears in bot arguments or command-line logs.

The helper uses a minimal generated Compose file and an empty explicit env file.
It never resolves development Compose configuration or sources root `.env`.
Inherited database, NATS, zone, auth and Compose settings are excluded from the
fixture API/migrator/seeder environment. Supplying a custom `SIM_API_URL` or
conflicting `DevToken`, `DevTokenFile` or `GrpcEndpoint` assignments anywhere
in extra bot arguments (including bare and mixed-case forms) is refused.
The obsolete `NfGrpc` override is also refused.
Both API listeners must be free and then owned by the launched process. A port
race fails the unit; it cannot attach to an existing API.

`SIM_API_BIN` and `SIM_MIGRATE_BIN` can select absolute executable paths from a
coordinator-approved candidate. Otherwise the helper builds the current
repository's API and migration binaries before starting infrastructure.
`SIM_PHASE2_SEEDER` can select an explicit published seeder path; its default is
`infra/scripts/seed-phase2-transfer-fixture.py`, requiring the pack interface
published in commit `1ee4338`. No external fixture environment file is sourced.
Host `psql` is unnecessary: a private shim validates the fixture database URI
and runs `psql` with stdin inside the owned Postgres service. It forwards decoded libpq components (`PGHOST`, `PGPORT`, database name in
`PGDATABASE`, `PGUSER`, `PGPASSWORD`) in environment variables, never credential
arguments. `PGDATABASE` is never forwarded as a URI to container psql.

Each unit's first scenario directory contains `fixture/manifest.json`, binary
and seeder SHA-256 hashes, `seed-manifest.json`, migration/seed/API/Compose logs,
and `cleanup-status.json`. Generated credentials/configuration are private
files inside a mode-0700 fixture directory. EXIT and signal traps stop the API,
retain logs and remove only the owned project's volumes; teardown failure is
blocking. CI retries owned cleanup if a process-group timeout interrupts the
wrapper. Phase2 CI never invokes root `docker compose down` or root log collection.
Legacy fixture routing and its existing CI project guard are unchanged.

Replay, trace, transition coverage, contract coverage and quarantine gates still
apply. Transfer RPCs are not counted as WebSocket intent coverage. Failure-only
video is diagnostic and does not alter the primary fixture evidence.

Run script regressions without Unreal, Docker or a live API:

```bash
python3 Scripts/test/phase2-fixture.test.py
bash Scripts/test/sim.test.sh
```

The Phase2 tests use command stubs, including a stub listener ownership probe.
They validate orchestration and cannot establish native gameplay acceptance.
