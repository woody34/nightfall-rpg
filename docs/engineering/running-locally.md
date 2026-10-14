# Running the server and client locally

This guide starts the full stack on one Linux machine and plays the current build by hand:
log in, create a character, walk, fight keltirs, die, respawn, and change class at the Class Master.

It brings together steps from the root [README](../../README.md) and the
[client README](../../apps/client-unreal/README.md). Those two files have the full details.

## 0. One-time prerequisites

| What | Check | If missing |
|---|---|---|
| moon + Rust | `moon --version` | `proto use` from the repo root |
| Docker | `docker compose version` | install Docker Engine |
| Unreal 5.8.3 + Linux toolchain | `echo $UE_ROOT $LINUX_MULTIARCH_ROOT` | client README, "Installing Unreal on Linux" |
| `.env` | `ls .env` | `cp .env.example .env` |

`UE_ROOT` and `LINUX_MULTIARCH_ROOT` are exported from `~/.bashrc`. Run every command below in an
interactive shell; non-interactive shells such as scripts and IDE tasks don't load `.bashrc`.

Build the client once, and again after any C++ or `.proto` change:

```bash
git submodule update --init
moon run client-unreal:setup          # one-off ~900 MB TurboLink gRPC/protobuf libs, idempotent
moon run client-unreal:build-editor
```

Close any other Unreal Editor first. If one is open, the link fails with
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
| Keycloak | 8080 | admin console `admin`/`admin`; player `testplayer`/`testplayer` |
| Grafana LGTM | 3300 | `admin`/`admin`, traces/logs/metrics |

If you use an older Postgres volume and Keycloak fails to start, create its database once:
`docker compose exec postgres psql -U nightfall -c 'CREATE DATABASE keycloak'`.

## 2. Server (API)

There are two ways to run it. Pick one.

### A. Real login (Keycloak). Use this for testing by hand.

```bash
moon run api:dev
```

This loads `.env`: Postgres, NATS, OTLP export, and the Keycloak issuer/audience. Migrations run at
startup. When the API is ready it listens on:

- HTTP / WebSocket: `http://localhost:3000` (`/health`, `/metrics`, `/ws`)
- gRPC: `localhost:50051`

Smoke check:

```bash
curl -s localhost:3000/health
```

### B. Dev tokens (no Keycloak). Use this for scripted or multi-client tests.

Uncomment `AUTH_DEV_TOKENS=1` in `.env` and restart `moon run api:dev`. The server then accepts
`test:<any-uuid>` bearer tokens, and each UUID is a separate account. Leave it commented out
for the real login flow.

### C. Items and equipment (Phase 4, V6 zone)

Uncomment `PHASE4_ENABLED=1` in `.env` (with `PHASE3_ENABLED` unset or `1`) and restart. Every new
zone epoch then boots V6 with the embedded Phase 4 item and skill bundle (D27); the log line
"Phase3 bootstrap bundle validated with the Phase4 (V6) item extension" names its hashes. Unset or
`0` keeps V5 epochs. With `AUTH_DEV_TOKENS=1` as well, the dev `GrantItem` RPC grants items to an
offline or online character; without V6 it answers `UNAVAILABLE` "durable V6 zone unavailable".

## 3. Client

### Option 1: in the editor

```bash
moon run client-unreal:editor
```

The editor opens `L_Login`. Press **Play**; Standalone is closest to the real game.

### Option 2: as a standalone game window

```bash
cd $UE_ROOT/Engine/Binaries/Linux && ./UnrealEditor ~/repos/nightfall-rpg/apps/client-unreal/Nightfall.uproject -game -windowed -ResX=1280 -ResY=720
```

Start it from inside the engine binaries directory. If you start it from anywhere else, the editor
crashes on ICU data.

## 4. What to test

### Log in (Keycloak device flow)

1. Click **Login**. A code like `ABCD-EFGH` appears.
2. Click **Open in browser**, sign in as `testplayer` / `testplayer`, and approve.
3. Within about 5 s the screen switches to your character list.

The client stores the login. To force the device flow again, run `nf.Logout` in the console (`~`)
or delete `apps/client-unreal/Saved/SaveGames/NightfallAuth.sav`.

### Create a character

Enter a name (3–16 letters), then pick a race and base class. The stat bars come from the server's
`ListClasses` catalogue. Click **Create**. Each account holds up to 7 characters.

### Enter the world and move

Click the character. Status goes "Requesting a play ticket..." → "Connecting..." → `L_TestZone`
loads. **Left click** to move. The server rejects any destination more than 64 tiles away;
the reason shows on the status line.

### Fight

The zone is 256 × 256 tiles. On screen, 1 tile = 1 m.

| Place | Tile |
|---|---|
| New characters spawn at | (0, 0) |
| Keltir meadow (3 keltirs) | around (100, 100) and (104, 102) |
| Town respawn / safe point | (126, 126) |
| Class Master NPC | (126, 128), talk radius 3 tiles |

Walk to the meadow in hops of 64 tiles or less. Click a keltir to target and auto-attack it. Clicking the ground stops the attack and moves you. Watch the
HP bars, damage numbers, XP gain, and a level-up after about 3 kills. If you let one kill you, you get the
"You have died" overlay. **Respawn** puts you at (126, 126).

### Class path and transfer (Phase 2)

The **Class path / Master** HUD button opens the class tree. You can also click the Class Master
next to the respawn point. A fresh character is too low level to transfer, so you'll see the
unmet requirements, which is expected. To try a real transfer you need a seeded level-20/40
character. The seeded profiles go through the sim harness rather than manual play; see
[phase2-fixtures.md](../../apps/client-unreal/Scripts/phase2-fixtures.md).

### Two players at once

Start a second client (Option 2 above) alongside the first. Two Keycloak logins on one machine
share the stored refresh token. The simplest way to get a second player is dev tokens (server
mode B) and the scripted playtest:

```bash
GRPC=localhost:50051 moon run client-unreal:playtest
```

It runs a "hunter" that kills 3 keltirs and a "victim" that dies and respawns. Pass means both
clients log `playtest: RESULT PASS`.

## 5. Useful console commands (non-shipping builds)

Open the console with `~` in the game window:

| Command | Does |
|---|---|
| `nf.Login` / `nf.Logout` | Log in using the stored login or dev token / forget the stored login |
| `nf.Characters` | List your characters |
| `nf.CreateCharacter <Name> [race 1-5]` | Create a character |
| `nf.EnterWorld [Name]` | Enter the world (first character by default) |
| `nf.ClickMove <x> <y>` | Move to a tile without the mouse |
| `nf.Status` / `nf.Respawn` | Print own state / press respawn |
| `nf.ClassCatalogue`, `nf.TransferOptions`, `nf.MoveMaster`, `nf.ChangeClass <id>` | Class flow |

For more client logging, add `-LogCmds="LogNightfall Verbose"` to the launch command.

## 6. Watching the server

- API stdout is human-readable (`TELEMETRY_PRETTY=1`).
- Grafana at <http://localhost:3300>: Explore → Tempo `{resource.service.name="nightfall-api"}`
  for traces, Loki `{service_name="nightfall-api"}` for logs, and the **Nightfall API** dashboard.
- `curl -s localhost:3000/metrics | grep nightfall_`

## 7. Troubleshooting

| Symptom | Likely cause |
|---|---|
| "Could not reach the login server" | Keycloak not up: `docker compose ps` |
| Login works, then every call is `UNAUTHENTICATED` | API started without `OIDC_ISSUER` (no `.env`), or with `AUTH_DEV_TOKENS=1` while you use Keycloak |
| Status loops "Reconnecting..." | API not running, or `/ws` unreachable at `WS_PUBLIC_URL` |
| API warns "in-memory adapters" | `DATABASE_URL` / `NATS_URL` not set: `.env` is missing |
| `moon run client-unreal:*` fails with an empty `UE_ROOT` path | the shell didn't load `~/.bashrc` |
| Build link error `-lUnrealEditor-TurboLinkGrpc` | another Unreal Editor is open |
| API exits: `zone admission refused: unresolved checkpoint recovery history ... cannot decode replay-log record` | an older build left unclosed epochs whose event format has since changed. Dev data only: `docker compose exec postgres psql -U nightfall -c "UPDATE zone_epochs SET closed_at = now() WHERE closed_at IS NULL"` |
| `Blocking waiting for file lock on build directory` | another `cargo` (rust-analyzer, tests, a second API) holds `target/`; wait or stop it |
| Port 3000/50051 in use | an old `nightfall-api` is still running: `pkill nightfall-api` |
| Login closes at once with code 4423 `character_lane_quarantined` | the checkpoint lane of that character is quarantined. Do the "Quarantined lanes" procedure below |

### Quarantined lanes

**Why it occurs.** At startup, the API recovers each zone epoch that did not close. If the
checkpoint of one character fails permanently during this recovery, the API quarantines that
character. The zone then starts for all other characters. The API does not release the dropped
facts of that character, so the database keeps its last committed checkpoint.

**What you see.**

- At startup, the API writes one log line for each zone. "no quarantined lanes" tells you that
  the zone has no quarantined characters. The warning "quarantined lanes" gives the number and
  the owners (`character@epoch:tick`).
- At login, the API writes the warning "login refused: character checkpoint lane quarantined"
  with the character and the reason. The client gets close code 4423.
- The dev `GrantItem` RPC for that character gives `FAILED_PRECONDITION` `character_lane_quarantined`.

**To list the quarantined characters:**

```bash
docker compose exec postgres psql -U nightfall -c \
  "SELECT character_id, zone_id, epoch, tick, reason, created_at FROM character_lane_quarantines ORDER BY created_at"
```

**To clear a quarantined character:**

1. Read the `reason` column. It names the check that failed.
2. Find the cause and correct it. Do not clear the row before the correction is in the build.
3. Stop the API.
4. Delete the row of that character:

   ```bash
   docker compose exec postgres psql -U nightfall -c \
     "DELETE FROM character_lane_quarantines WHERE character_id = '<character-id>'"
   ```

5. Start the API. Make sure that the startup log does not show the character in "quarantined lanes".
6. Log in with the character. The character starts from its last committed checkpoint.

If the cause is not corrected, the next failure of the lane fences the zone again.

## Shutting down

`Ctrl+C` the API. Close the client. `docker compose stop` keeps the data;
`docker compose down -v` wipes the Postgres and Keycloak volumes as well.
