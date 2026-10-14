# Running the server and client locally

This guide starts the full stack on one Linux machine. Then you play the current build manually:
log in, make a character, walk, fight keltirs, die, respawn, and change class at the Class Master.

This guide puts together steps from the root [README](../../README.md) and the
[client README](../../apps/client-unreal/README.md). These two files have all the details.

## 0. One-time prerequisites

| What | Check | If missing |
|---|---|---|
| moon + Rust | `moon --version` | `proto use` from the repo root |
| Docker | `docker compose version` | Install Docker Engine. |
| Unreal 5.8.3 + Linux toolchain | `echo $UE_ROOT $LINUX_MULTIARCH_ROOT` | Client README, "Installing Unreal on Linux" |
| `.env` | `ls .env` | `cp .env.example .env` |

`~/.bashrc` exports `UE_ROOT` and `LINUX_MULTIARCH_ROOT`. Run all the commands below in an
interactive shell. Shells that are not interactive, for example scripts and IDE tasks, do not load `.bashrc`.

Build the client one time. Build it again after each C++ or `.proto` change:

```bash
git submodule update --init
moon run client-unreal:setup          # one-off ~900 MB TurboLink gRPC/protobuf libs, idempotent
moon run client-unreal:build-editor
```

CAUTION: Close all other Unreal Editor windows first. If an editor is open, the link fails with
`unable to find library -lUnrealEditor-TurboLinkGrpc`.

## 1. Infrastructure (Docker)

```bash
docker compose up -d
docker compose ps                     # all four should be "healthy"
```

| Service | Port | Notes |
|---|---|---|
| Postgres 16 | 5432 | `nightfall` / `nightfall` |
| NATS | 4222 | |
| Keycloak | 8080 | Admin console `admin`/`admin`. Player `testplayer`/`testplayer`. |
| Grafana LGTM | 3300 | `admin`/`admin`, traces/logs/metrics |

If you use an older Postgres volume and Keycloak does not start, make its database one time:
`docker compose exec postgres psql -U nightfall -c 'CREATE DATABASE keycloak'`.

## 2. Server (API)

You can run the server in two ways. Select one.

### A. Real login (Keycloak). Use this for testing by hand.

```bash
moon run api:dev
```

This command loads `.env`: Postgres, NATS, OTLP export, and the Keycloak issuer/audience. The migrations run when
the API starts. When the API is ready, it listens on:

- HTTP / WebSocket: `http://localhost:3000` (`/health`, `/metrics`, `/ws`)
- gRPC: `localhost:50051`

Smoke check:

```bash
curl -s localhost:3000/health
```

### B. Dev tokens (no Keycloak). Use this for scripted or multi-client tests.

1. In `.env`, remove the comment mark from `AUTH_DEV_TOKENS=1`.
2. Stop and start `moon run api:dev` again.

The server then accepts `test:<any-uuid>` bearer tokens. Each UUID is a different account.
For the real login flow, keep the comment mark on this line.

### C. Items and equipment (Phase 4, V6 zone)

1. In `.env`, remove the comment mark from `PHASE4_ENABLED=1`. Keep `PHASE3_ENABLED` unset or set it to `1`.
2. Restart `moon run api:dev`.

Each new zone epoch then starts V6 with the embedded Phase 4 item and skill bundle (D27).
The log line "Phase3 bootstrap bundle validated with the Phase4 (V6) item extension" gives the hashes of the bundle.
When `PHASE4_ENABLED` is unset or `0`, the epochs stay V5.

When `AUTH_DEV_TOKENS=1` is also set, the dev `GrantItem` RPC grants items to an offline or online character.
Without V6, the RPC answers `UNAVAILABLE` "durable V6 zone unavailable".

## 3. Client

### Option 1: in the editor

```bash
moon run client-unreal:editor
```

The editor opens `L_Login`. Push **Play**. Standalone mode is the most similar to the real game.

### Option 2: as a standalone game window

```bash
cd $UE_ROOT/Engine/Binaries/Linux && ./UnrealEditor ~/repos/nightfall-rpg/apps/client-unreal/Nightfall.uproject -game -windowed -ResX=1280 -ResY=720
```

CAUTION: Start the game from the engine binaries directory. If you start it from a different directory, the editor
crashes on ICU data.

## 4. What to test

### Log in (Keycloak device flow)

1. Click **Login**. A code, for example `ABCD-EFGH`, comes into view.
2. Click **Open in browser**.
3. Sign in as `testplayer` / `testplayer`, and approve.
4. In approximately 5 s, the screen changes to your character list.

The client keeps the login. To do the device flow again, run `nf.Logout` in the console (`~`).
As an alternative, remove `apps/client-unreal/Saved/SaveGames/NightfallAuth.sav`.

### Create a character

1. Type a name (3–16 letters).
2. Select a race and a base class. The stat bars come from the server `ListClasses` catalogue.
3. Click **Create**.

Each account can have a maximum of 7 characters.

### Enter the world and move

1. Click the character. The status changes from "Requesting a play ticket..." to "Connecting...". Then `L_TestZone`
   loads.
2. To move, use the **Left click**.

The server refuses each destination that is more than 64 tiles away. The status line shows the reason.

### Fight

The zone is 256 × 256 tiles. On the screen, 1 tile = 1 m.

| Place | Tile |
|---|---|
| New characters spawn at | (0, 0) |
| Keltir meadow (3 keltirs) | Approximately (100, 100) and (104, 102) |
| Town respawn / safe point | (126, 126) |
| Class Master NPC | (126, 128), talk radius 3 tiles |

1. Walk to the meadow in steps of 64 tiles or less.
2. Click a keltir to target it and auto-attack it. A click on the ground stops the attack and moves you.
3. Monitor the HP bars, the damage numbers, the XP gain, and a level-up after approximately 3 kills.

If a keltir kills you, you get the "You have died" overlay. **Respawn** puts you at (126, 126).

### Class path and transfer (Phase 2)

The **Class path / Master** HUD button opens the class tree. You can also click the Class Master
near the respawn point. A new character has a level that is too low for a transfer. Thus you see the
requirements that the character does not meet. This result is correct. For a real transfer, you must have a seeded level-20/40
character. The seeded profiles go through the sim harness, not through manual play. Refer to
[phase2-fixtures.md](../../apps/client-unreal/Scripts/phase2-fixtures.md).

### Two players at once

Start a second client (Option 2 above) together with the first client. Two Keycloak logins on one machine
use the same stored refresh token. The easiest procedure for a second player is to use dev tokens (server
mode B) and the scripted playtest:

```bash
GRPC=localhost:50051 moon run client-unreal:playtest
```

The playtest runs a "hunter" that kills 3 keltirs and a "victim" that dies and respawns. The test passes when the two
clients log `playtest: RESULT PASS`.

## 5. Useful console commands (non-shipping builds)

To open the console, push `~` in the game window:

| Command | Does |
|---|---|
| `nf.Login` / `nf.Logout` | Log in with the stored login or dev token / forget the stored login |
| `nf.Characters` | Show a list of your characters |
| `nf.CreateCharacter <Name> [race 1-5]` | Make a character |
| `nf.EnterWorld [Name]` | Enter the world (the first character is the default) |
| `nf.ClickMove <x> <y>` | Move to a tile without the mouse |
| `nf.Status` / `nf.Respawn` | Print your state / push respawn |
| `nf.ClassCatalogue`, `nf.TransferOptions`, `nf.MoveMaster`, `nf.ChangeClass <id>` | Class flow |

For more client log data, add `-LogCmds="LogNightfall Verbose"` to the launch command.

## 6. Watching the server

- The API stdout is easy for persons to read (`TELEMETRY_PRETTY=1`).
- Grafana at <http://localhost:3300>: use Explore → Tempo `{resource.service.name="nightfall-api"}`
  for traces and Loki `{service_name="nightfall-api"}` for logs. Also use the **Nightfall API** dashboard.
- `curl -s localhost:3000/metrics | grep nightfall_`

## 7. Troubleshooting

| Symptom | Likely cause |
|---|---|
| "Could not reach the login server" | Keycloak does not operate: `docker compose ps` |
| Login works, then each call is `UNAUTHENTICATED` | The API started without `OIDC_ISSUER` (no `.env`), or with `AUTH_DEV_TOKENS=1` while you use Keycloak. |
| Status loops "Reconnecting..." | The API does not run, or the API cannot get access to `/ws` at `WS_PUBLIC_URL`. |
| API warns "in-memory adapters" | `DATABASE_URL` / `NATS_URL` is not set: `.env` is missing. |
| `moon run client-unreal:*` fails with an empty `UE_ROOT` path | The shell did not load `~/.bashrc`. |
| Build link error `-lUnrealEditor-TurboLinkGrpc` | A different Unreal Editor is open. |
| API exits: `zone admission refused: unresolved checkpoint recovery history ... cannot decode replay-log record` | An older build did not close some epochs, and their event format changed after that build. Dev data only: `docker compose exec postgres psql -U nightfall -c "UPDATE zone_epochs SET closed_at = now() WHERE closed_at IS NULL"` |
| `Blocking waiting for file lock on build directory` | A different `cargo` (rust-analyzer, tests, a second API) holds `target/`. Wait, or stop it. |
| Port 3000/50051 in use | An old `nightfall-api` continues to run: `pkill nightfall-api` |

## Shutting down

1. Push `Ctrl+C` in the API terminal.
2. Close the client.
3. To stop the containers and keep the data, run `docker compose stop`.

CAUTION: `docker compose down -v` also removes the Postgres and Keycloak volumes.
