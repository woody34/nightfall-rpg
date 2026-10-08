# Phase 0 — Foundations

Architecture, networking, persistence, data pipeline, and accounts for Nightfall. Everything in later
phases assumes the decisions recorded here.

---

## 1. Purpose and scope

**Delivers**

- The client/server topology: an authoritative game server, a thin auth/HTTP surface, and a browser
  Phaser client that only renders and sends intent.
- The wire model: how request/response calls and the real-time world stream reach a browser that
  cannot speak raw gRPC.
- The simulation clock: tick rate, interest management, movement sync, and the (deliberately small)
  amount of client prediction a click-to-move MMO needs.
- Persistence: Postgres schema strategy, write-behind caching of character state, item ownership and
  trade atomicity, snapshot/rollback.
- The data pipeline: how designers author skills, items, monsters, class tables as data files that are
  validated at boot and hot-reloaded in development.
- Accounts and security: account vs character, Argon2id password hashing, session tokens, the
  login→game handoff ticket, character slots.
- A concrete crate list and module layout for `apps/api/src`.

**Excludes**

- Any gameplay formula (Phase 1), class/race content (Phase 2), combat (Phase 3).
- Multi-process world sharding. We design so that it is possible, we do not build it now.
- Anti-cheat beyond server authority and basic rate limiting (Phase 9).

---

## 2. Reference: how Lineage 2 does it

### 2.1 Login server / game server split

Lineage 2 runs two separate daemons. The **login server** (default port 2106) owns accounts, and the
**game server** (default port 7777) owns the world. They are connected by a private TCP link
(L2J `GameServerThread` on the login side, `LoginServerThread` on the game side). The client never
talks to both at once; it is handed from one to the other with a **session key** made of four 32-bit
integers:

```java
// L2J (Interlude branch) loginserver/SessionKey.java
public int playOkID1, playOkID2, loginOkID1, loginOkID2;
public boolean checkLoginPair(int loginOk1, int loginOk2) {
    return (loginOkID1 == loginOk1) && (loginOkID2 == loginOk2);
}
```

The full flow, with L2J packet names:

| Step | Direction | Packet | Payload of interest |
|------|-----------|--------|---------------------|
| 1 | LS → C | `Init` | Blowfish key for this connection, RSA public key for password |
| 2 | C → LS | `RequestAuthLogin` | RSA-encrypted username/password |
| 3 | LS → C | `LoginOk` | `loginOk1`, `loginOk2` (opcode 0x03) |
| 4 | C → LS | `RequestServerList` | — |
| 5 | LS → C | `ServerList` | realm list, per-realm population/status |
| 6 | C → LS | `RequestServerLogin` | chosen server id + `loginOk1/2` |
| 7 | LS → C | `PlayOk` | `playOk1`, `playOk2` (opcode 0x07) |
| 8 | C → GS | `ProtocolVersion`, GS replies `KeyPacket` | per-connection XOR key |
| 9 | C → GS | `AuthLogin` | account, `playOk1/2`, `loginOk1/2` |
| 10 | GS → LS | `PlayerAuthRequest` | account + all four ints |
| 11 | LS → GS | `PlayerAuthResponse` | authed yes/no; LS forgets the key |
| 12 | GS → C | `CharSelectionInfo` | character list (slots) |

Points worth copying: the game server never sees a password; the login server keeps the session key
only until it is consumed (`removeAuthedLoginClient` on success); the game server parks the client in
a `_waitingClients` list until the login server answers; a bad key closes the socket with
`LoginFail.SYSTEM_ERROR_LOGIN_LATER`. Login-server packets are Blowfish-ECB with a static key plus an
XOR checksum; the game-server stream is a rolling XOR cipher. Neither is cryptographically meaningful
today, which is why we use TLS instead.

### 2.2 World model: one seamless world per shard, grid regions

An L2 "server" (Bartz, Sieghardt, ...) is a **shard**: one JVM process owning one full copy of the
world. There is no zoning and no loading screen; the world is continuous. Inside the process the
world is a fixed grid (L2J `L2World`):

```java
public static final int SHIFT_BY = 12;                  // region = 4096 units
private static final int TILE_SIZE = 32768;             // map tile (one geodata file)
public static final int TILE_X_MIN = 11, TILE_X_MAX = 28;
public static final int TILE_Y_MIN = 10, TILE_Y_MAX = 26;
public L2WorldRegion getRegion(int x, int y) {
    return _worldRegions[(x >> SHIFT_BY) + OFFSET_X][(y >> SHIFT_BY) + OFFSET_Y];
}
```

So the live map is 18 × 17 tiles of 32768 units, each tile 8 × 8 regions of 4096 units: a
144 × 136 region grid. Each `L2WorldRegion` holds its objects and a precomputed list of its 8
`_surroundingRegions`. Visibility ("known objects") is the 3 × 3 block of regions around you, which is
12288 units square. Regions **deactivate** when neither they nor any neighbour contains a player, which
stops AI and regeneration tasks for empty parts of the map. This is the interest-management scheme we
copy.

### 2.3 Clock and packet model

L2J's `GameTimeController` runs at **10 ticks per second** (`TICKS_PER_SECOND = 10`,
`MILLIS_IN_TICK = 100`). Movement is integrated per tick: `distPassed = moveSpeed * (ticks since last
update) / TICKS_PER_SECOND`. In-game time runs 6 days per real day.

Movement is **click-to-move, server-planned**. The client sends `MoveBackwardToLocation(targetX,
targetY, targetZ, originX, originY, originZ)`. The server rejects a request if `dx² + dy² > 9900²`,
otherwise sets `AI_INTENTION_MOVE_TO`, computes the path, and broadcasts `MoveToLocation(objectId,
from, to)` to everyone who knows the object. Every client then dead-reckons the mover at its known
speed. Periodically the client sends `ValidatePosition(x, y, z, heading)`; the server compares to its
own position and, depending on `CoordSynchronize` config, either trusts the client's z, snaps the
client, or ignores. There is **no input prediction and no rollback**; the model works because
movement is a straight line at a known speed from a known start.

Attack timing is also server-owned: time between swings is `470000 / pAtkSpd` ms in High Five (`500000`
in older branches); bows use `1500 * 345 / pAtkSpd`. Casting uses `skillTime * 333 / mAtkSpd`. These
constants belong to Phase 1/3 but they define the latency budget: a 300 atk-speed fighter swings every
1567 ms, so a 100 ms tick and 150–250 ms of interpolation delay are invisible.

### 2.4 Persistence

L2J stores everything in MySQL/MariaDB and caches it in memory. `Character.properties` has
`CharacterDataStoreInterval = 15` (minutes) for the periodic character store, with
`LazyItemsUpdate` and `UpdateItemsOnCharStore` controlling whether item rows are written lazily. Item
instances are rows in `items` keyed by `object_id` with `owner_id`, `loc` (INVENTORY, PAPERDOLL,
WAREHOUSE, ...), `loc_data` (slot), `count`, `enchant_level`. A trade (`TradeList`) is validated
in memory and then both inventories are mutated and written. This is the classic shape; it also
produced the classic dupe bugs whenever two code paths mutated inventory without a common lock, which
is why section 3.3 insists on a single ownership transaction.

### 2.5 Data authoring

L2J moved from SQL tables to **XML + XSD** for static data: `stats/statBonus.xml` (the six
stat→multiplier tables), `stats/experience.xml`, `stats/chars/baseStats/<Class>.xml` (base stats,
per-level HP/MP/CP), `stats/skills/*.xml`, `stats/items/*.xml`, `stats/npcs/*.xml`, plus
`config/*.properties` for tunables (`MaxRunSpeed = 300`, `MaxPAtkSpeed = 1500`, `MaxMAtkSpeed = 1999`,
`MaxPCritRate = 500`, `MaxMCritRate = 200`, `MaxEvasion = 250`, `PartyXpCutoffGaps = 0,9;10,14;15,99`,
`PartyXpCutoffGapPercent = 100;30;0`). Every XML has an XSD, loaded once at boot by a
`DocumentParser` subclass. There is no hot reload except for scripts.

---

## 3. Design decisions for Nightfall

### 3.1 Topology: one binary, two listeners, one shard

**Decision.** `nightfall-api` stays one Rust binary that runs:

- an **axum** HTTP/1.1+WebSocket listener on `:3000` for auth, account REST, the browser real-time
  channel, health;
- a **tonic** gRPC listener on `:50051` for native tools (GM console, load bots, future native
  client), with **tonic-web** enabled so the browser can call unary/server-streaming RPCs over
  grpc-web as well.

The "login server" and "game server" of L2 become two **modules in one process** (`auth` and
`world`) separated by the same ticket handoff L2 uses, so that they can be split into two deployables
later without a protocol change. One process owns one **realm** (shard). Multiple realms are multiple
processes with separate databases; a `realm_id` is baked into every character id so cross-realm
transfers are possible later.

**Alternatives considered.** (a) Separate login daemon from day one — correct long-term, but it
doubles deploy surface for a team of one; the ticket design keeps the door open. (b) Zoned world
(one process per zone, loading screens) — simpler scaling but worse feel, and L2's seamless world is
part of what we are copying. (c) Single-shard-for-everyone (EVE model) — requires distributed
simulation; out of scope.

### 3.2 Networking

> Revised by §10.D3 and §10.D7 (Phase 0b).

#### Why not plain gRPC to the browser

Browsers do not expose HTTP/2 frames, so they cannot speak native gRPC. The two browser-compatible
protocols are **grpc-web** and **Connect**; both support **unary and server-streaming only** in
browsers. The grpc-web project has formally declined client/bidi streaming (its roadmap points at
WebTransport as the eventual answer), and tonic-web documents the same limit: "unary and
server-streaming calls only". Connect's own protocol doc says bidi needs HTTP/2 end-to-end, which
`fetch()` does not give us. **WebTransport** (HTTP/3, QUIC streams) is now green in Chrome 97+,
Firefox 114+, Edge 98+ and Safari 26.4+ (caniuse 91.6% global), but needs UDP/443 reachability, a
QUIC-capable edge, and has no DevTools inspection yet. **WebSocket** is universal, TCP-only, one
ordered stream.

| Option | Browser bidi | Ordered | HoL blocking | Infra | Verdict |
|--------|--------------|---------|--------------|-------|---------|
| grpc-web (tonic-web) | server-stream only | yes | yes | HTTP/1.1 ok | request/response + server push only |
| Connect | server-stream only (bidi via connect-bidi-web add-on) | yes | yes | HTTP/1.1 ok | same, nicer curl ergonomics |
| WebSocket (axum) | yes | yes | yes | any LB | **real-time channel now** |
| WebTransport | yes, multi-stream | per stream | no | HTTP/3 edge, UDP | real-time channel later, feature-flagged |

**Decision.**

1. **Request/response** (login, character list, create/delete character, shop lists, mail, anything
   not latency-critical): gRPC services in `game.proto`, served by tonic and exposed to the browser
   through `tonic-web` (`GrpcWebLayer`, `accept_http1(true)`). The client uses `@connectrpc/connect-web`
   with the grpc-web protocol so one generated TypeScript client covers both.
2. **Real-time world channel**: a single **WebSocket** at `GET /ws?ticket=…` on the axum listener,
   carrying length-free **protobuf** frames (`ClientMessage` / `ServerMessage` envelopes, one message
   per binary WebSocket frame). The message types live in the same `.proto` package so the client
   decodes them with the same generated code. WebTransport is a planned second transport behind the
   same envelope (connect-bidi-web shows the shape: one QUIC stream per RPC, same flag+length framing).
3. Nothing in the world simulation knows which transport a session arrived on; `net::session` hands
   the world an `mpsc::Sender<ServerMessage>` and a `mpsc::Receiver<ClientMessage>`.

#### Clock, snapshots, interest management

- **Simulation tick: 100 ms (10 Hz)**, same as L2. Every world mutation happens on the tick thread;
  network tasks only enqueue intents. Rationale: click-to-move has no per-frame input; the slowest
  observable action (an attack swing) is ≥ 500 ms at the hard cap of 1500 atk speed.
- **Snapshot rate: 10 Hz per client**, one `WorldDelta` per tick containing only entities in the
  client's area of interest and only fields that changed. Movement is not sent per tick: the server
  sends `MoveStart{entity, from, to, speed, server_tick}` once and `MoveStop`/`Teleport` on change,
  exactly like `MoveToLocation`; clients dead-reckon between.
- **Interpolation delay: 200 ms** for remote entities (2 ticks + ~0–30 ms jitter margin, following the
  "lose two packets and still have something to interpolate towards" rule of thumb). Position
  corrections are smoothed over 200 ms unless the error exceeds 2 cells, then snapped.
- **Area of interest**: uniform grid, cell = **512 world units** (16 tiles of 32 px). Each entity is
  registered in one cell; a client's interest set is the **3 × 3 block** around its cell (1536 units
  square, comfortably more than a 1280 × 720 viewport's 1468-unit diagonal). Entering/leaving the set
  produces `EntitySpawn` / `EntityDespawn`, which is L2's `CharInfo` / `DeleteObject`. Cells with no
  player in their 3 × 3 neighbourhood are **inactive**: monster AI, regen, and respawn timers do not run
  there, mirroring `L2WorldRegion.setActive`. Tile-based interest management is a well-studied cheap
  approximation of ideal visibility filtering (Boulanger et al.).
- **Priority**: if a client's outbound queue exceeds 64 KB the server drops non-essential updates for
  that client (chat first, then remote entity cosmetic updates) and keeps own-entity and combat
  results. A per-entity priority accumulator (Gaffer "state synchronization") is deferred until we
  have a measured problem.

#### Movement sync and prediction

Click-to-move needs **optimistic start, not rollback**:

1. Client clicks; client immediately starts its own avatar moving toward the point along a
   client-side path over the same tile map the server has (both ship the same collision grid).
2. Client sends `MoveTo{target, client_seq}`.
3. Server validates (distance ≤ 20 cells, not rooted/stunned/dead, walkable), plans the authoritative
   path, broadcasts `MoveStart` to interested clients including the mover, tagged with `client_seq`.
4. The mover compares the server path with its optimistic path; if the destination differs it blends
   over 200 ms; if the server rejected (`MoveRejected{client_seq, reason}`) it snaps back.

Lag compensation is limited to **range tolerance**: when validating an attack/skill the server
accepts `distance ≤ range + max(50, attacker_speed * rtt/2)` where `rtt` is the smoothed round trip
measured by `Ping` frames every 5 s. No rewind of other entities (there is no hitscan).

### 3.3 Persistence

> Revised by §10.D2 (Phase 0b).

**Postgres 16, sqlx 0.9** (`runtime-tokio`, `tls-rustls`, `postgres`, `uuid`, `time`, `migrate`),
compile-time checked queries against a dev database, migrations embedded with `sqlx::migrate!`.
SeaORM was considered and rejected: we want hand-written SQL for the few hot paths (inventory,
trade) and sqlx's `query!` gives type checking without an ORM layer.

**Two tiers of state.**

| Tier | Examples | Write policy |
|------|----------|--------------|
| Volatile character state | position, hp/mp/cp, xp, buffs, quest counters | **write-behind**: dirty flag, flushed every 30 s, on logout, on death, on realm transfer, on server shutdown |
| Ownership state | item rows, adena, warehouse, mail attachments, trade | **write-through, transactional**: the in-memory inventory is updated only after `COMMIT` |

The volatile flush is a single `UPDATE characters SET ... WHERE id = $1 AND version = $2` with a
`version` counter (optimistic lock) so that a stale server instance can never overwrite a newer save.
Thirty seconds is chosen against L2J's default of fifteen minutes: a crash loses at most 30 s of XP,
and the write (one row per online character every 30 s) is trivial load.

**Items and dupe prevention.** Every item instance is one row with a primary key the game never
reuses:

```sql
CREATE TABLE items (
  id            BIGINT PRIMARY KEY,                -- snowflake, realm-prefixed
  template_id   INT      NOT NULL,
  owner_id      BIGINT   REFERENCES characters(id),
  location      SMALLINT NOT NULL,                 -- 0 inventory, 1 equipped, 2 warehouse, 3 ground, 4 mail, 5 trade-escrow
  slot          INT,
  count         BIGINT   NOT NULL CHECK (count > 0),
  enchant       SMALLINT NOT NULL DEFAULT 0,
  version       INT      NOT NULL DEFAULT 0,
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ON items (owner_id, location);
CREATE TABLE item_ledger (                         -- append-only, never updated
  seq        BIGSERIAL PRIMARY KEY,
  at         TIMESTAMPTZ NOT NULL DEFAULT now(),
  item_id    BIGINT NOT NULL,
  template_id INT NOT NULL,
  delta      BIGINT NOT NULL,                      -- +count created / -count destroyed / 0 moved
  from_owner BIGINT, to_owner BIGINT,
  reason     SMALLINT NOT NULL,                    -- drop, pickup, trade, vendor, craft, gm, destroy...
  tx_id      UUID NOT NULL                          -- groups both halves of a trade
);
```

Rules:

1. All ownership mutations go through `persist::items` which opens one transaction, locks the affected
   rows with `SELECT ... FOR UPDATE` **ordered by `id`** (consistent lock order prevents deadlocks,
   per the Postgres explicit-locking docs), re-checks `owner_id`/`location`/`count` against what the
   request assumed, applies updates, inserts ledger rows, commits. Only then does the world thread
   mutate the in-memory inventory and send `InventoryUpdate`.
2. A player trade is **one transaction** touching both inventories: both parties' offered rows are
   locked, verified against the confirmed offer, moved; adena is a stackable item row so it uses the
   same path. Partial application is impossible; a crash before `COMMIT` leaves both inventories as
   they were.
3. Stackables never exceed `i64`; `count` has a `CHECK (count > 0)` and merging deletes the source
   row in the same transaction.
4. Transactions run at **Read Committed** with explicit row locks; we do not depend on Serializable,
   but the `persist` layer treats SQLSTATE `40001` and `40P01` (deadlock) as retryable up to 3 times.
5. Client requests carry a `client_seq`; the session rejects a duplicate seq, which kills "double
   click the vendor" dupes at the edge.

**Snapshots and rollback.** Nightly `pg_dump` plus continuous WAL archiving gives point-in-time
recovery. The `item_ledger` is the audit trail: a suspected dupe is investigated by replaying ledger
rows for the `item_id`, and a targeted rollback is `DELETE`/`UPDATE` on items driven by the ledger
rather than a full restore. Character volatile rows are additionally snapshotted into
`character_snapshots` (JSONB) at logout and kept for 7 days for GM restores.

### 3.4 Data pipeline

**Decision: TOML files in `packages/data/`, loaded at boot into immutable structs, validated,
hot-reloadable in dev.**

- **Format**: TOML (`toml` 0.8 via serde). Comments, no quoting noise, readable diffs, and every
  phase document (2 through 6, 9) already sketches its data in it. Tagged enums such as skill effects
  are written as arrays of tables with a `kind` field (`[[effects]] kind = "heal" power = 50`) and
  deserialised with `#[serde(tag = "kind")]`, which is the one place TOML is clumsier than RON. RON
  was considered for its native enum syntax and rejected to keep a single format designers and
  tooling (editors, Tiled exports, scripts) all understand. JSON was rejected for lacking comments.
  L2J's XML+XSD is the closest analogue; TOML plus serde-derived structs gives us the schema for free.
- **Layout**: one directory per entity kind (`classes/`, `skills/`, `items/`, `npcs/`, `zones/`,
  `tables/`), one file per entity or per table, filenames equal to the entity's `id`.
- **Loading**: `data::load(dir) -> Result<GameData, Vec<DataError>>` parses everything, then runs
  cross-reference validation (every `skill_id` referenced by a class exists, every drop template
  references a real item, XP table is strictly increasing, stat tables cover 1..=max_stat). Boot fails
  on any error and prints all of them.
- **Access**: `Arc<ArcSwap<GameData>>`. Systems read through a cheap `load()`; hot reload atomically
  swaps the pointer. Live entities hold ids, never references into `GameData`, so a swap cannot
  invalidate them.
- **Hot reload (dev only)**: `notify` 8 + `notify-debouncer-mini` watches `packages/data`, re-runs the
  full load, swaps on success, logs errors and keeps the old data on failure. A `POST /admin/reload`
  does the same in staging behind the admin token.
- **Build step**: a moon task `data:check` runs the loader in CI so a bad data commit fails fast.

Example class template (fields defined in Phase 1):

```toml
# packages/data/classes/human_fighter.toml
id = "human_fighter"
race = "human"
base = { str = 40, dex = 30, con = 43, int = 21, wit = 11, men = 25 }
hp = { base = 80.0, per_level = 11.765, accel = 0.065 }
mp = { base = 30.0, per_level = 5.430, accel = 0.030 }
cp_ratio = 0.4
base_p_atk = 4
base_m_atk = 6
base_p_def = 80
base_m_def = 41
base_p_atk_spd = 300
base_m_atk_spd = 333
base_crit = 44
run_speed = 115
walk_speed = 80
```

### 3.5 Accounts and security

> Revised by §10.D1 (Phase 0b).

- **Account vs character.** `accounts` holds login identity; `characters` holds up to **7**
  characters per account per realm (L2's default slot count). Characters are soft-deleted with a
  **7-day** grace (`deleted_at`), after which a reaper hard-deletes rows and moves items to the
  ledger as destroyed.
- **Passwords**: Argon2id (`argon2` 0.6, PHC string output `$argon2id$v=19$...`) with OWASP's
  recommended `m=19456 KiB, t=2, p=1`. Verification reads params from the stored PHC string, so
  parameters can be raised later and rehashed on next login. Hashing runs on
  `tokio::task::spawn_blocking`.
- **Session token** (`POST /auth/login` → 32 random bytes, base64url). Stored as SHA-256 in
  `sessions(token_hash, account_id, expires_at, created_ip)`, 7-day expiry, sent as `Authorization:
  Bearer` on gRPC/HTTP calls. One account may hold several sessions (multiple devices).
- **Play ticket** — our `playOk`. `EnterWorld(character_id)` over gRPC returns a **single-use ticket
  valid 60 s**, stored in memory in the `auth` module keyed by ticket, bound to `(account_id,
  character_id, client_ip)`. The WebSocket upgrade must present it; the `world` module asks `auth`
  to redeem it (the in-process equivalent of `PlayerAuthRequest`/`PlayerAuthResponse`) and then
  spawns the character. An account may have only one character in world per realm; redeeming a
  ticket for an account already in world kicks the old session (L2's duplicate-login handling).
- **Transport security**: TLS terminated at the reverse proxy (Caddy/nginx) for both listeners; the
  binary itself speaks plaintext on loopback. gRPC and WebSocket both ride HTTPS/WSS in production.
- **Rate limits**: `tower_governor` on `/auth/*` (5/min/IP), WebSocket inbound capped at 30
  messages/s per session with disconnection on sustained overflow.
- Never log passwords, tokens, or tickets; `tracing` fields are allow-listed.

---

## 4. Data model

### 4.1 Tables (Postgres)

```sql
CREATE TABLE accounts (
  id            BIGINT PRIMARY KEY,
  email         CITEXT UNIQUE NOT NULL,
  password_phc  TEXT NOT NULL,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  banned_until  TIMESTAMPTZ
);
CREATE TABLE sessions (
  token_hash    BYTEA PRIMARY KEY,
  account_id    BIGINT NOT NULL REFERENCES accounts(id),
  expires_at    TIMESTAMPTZ NOT NULL,
  created_ip    INET
);
CREATE TABLE characters (
  id            BIGINT PRIMARY KEY,            -- realm-prefixed snowflake
  account_id    BIGINT NOT NULL REFERENCES accounts(id),
  realm_id      SMALLINT NOT NULL,
  slot          SMALLINT NOT NULL,
  name          CITEXT NOT NULL,
  race          SMALLINT NOT NULL,
  class_id      TEXT NOT NULL,                 -- data id, e.g. 'human_fighter'
  level         SMALLINT NOT NULL DEFAULT 1,
  xp            BIGINT NOT NULL DEFAULT 0,
  sp            BIGINT NOT NULL DEFAULT 0,
  hp            INT NOT NULL, mp INT NOT NULL, cp INT NOT NULL,
  x             REAL NOT NULL, y REAL NOT NULL, zone_id TEXT NOT NULL,
  karma         INT NOT NULL DEFAULT 0, pk_count INT NOT NULL DEFAULT 0, pvp_count INT NOT NULL DEFAULT 0,
  version       INT NOT NULL DEFAULT 0,        -- optimistic lock for write-behind
  deleted_at    TIMESTAMPTZ,
  last_saved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (realm_id, name),
  UNIQUE (account_id, realm_id, slot)
);
CREATE TABLE character_snapshots (character_id BIGINT, at TIMESTAMPTZ, body JSONB);
-- items, item_ledger: see §3.3
```

### 4.2 Proto additions (`packages/proto/nightfall/v1/`)

```proto
// auth.proto
service AuthService {
  rpc Register(RegisterRequest) returns (AuthResponse);
  rpc Login(LoginRequest) returns (AuthResponse);          // returns session token
  rpc Logout(LogoutRequest) returns (google.protobuf.Empty);
}
message AuthResponse { string session_token = 1; int64 expires_at_ms = 2; }

// character.proto
service CharacterService {
  rpc ListCharacters(ListCharactersRequest) returns (CharacterList);
  rpc CreateCharacter(CreateCharacterRequest) returns (Character);
  rpc DeleteCharacter(DeleteCharacterRequest) returns (google.protobuf.Empty);  // soft delete
  rpc RestoreCharacter(RestoreCharacterRequest) returns (Character);
  rpc EnterWorld(EnterWorldRequest) returns (EnterWorldResponse);               // issues play ticket
}
message EnterWorldResponse { string ticket = 1; string ws_url = 2; int32 tick_ms = 3; }

// world.proto — frames carried over the WebSocket, one per binary frame
message ClientMessage {
  uint32 client_seq = 1;
  oneof body {
    Ping ping = 2;
    MoveTo move_to = 3;
    StopMove stop_move = 4;
    Interact interact = 5;         // target entity, action (attack/talk/pickup)
    CastSkill cast_skill = 6;
    Chat chat = 7;
  }
}
message ServerMessage {
  uint32 server_tick = 1;
  oneof body {
    Pong pong = 2;
    WorldDelta delta = 3;
    EntitySpawn spawn = 4;
    EntityDespawn despawn = 5;
    MoveStart move_start = 6;      // entity_id, from, to, speed, client_seq (0 if not ours)
    MoveStop move_stop = 7;
    MoveRejected move_rejected = 8;
    Teleport teleport = 9;
    SystemMessage system = 10;
    Chat chat = 11;
  }
}
message MoveTo { Position target = 1; }
message MoveStart { uint64 entity_id = 1; Position from = 2; Position to = 3; float speed = 4; uint32 client_seq = 5; }
message WorldDelta { repeated EntityDelta entities = 1; }
message EntityDelta { uint64 entity_id = 1; optional uint32 hp = 2; optional uint32 max_hp = 3; /* sparse */ }
```

`Position` stays `{float x, float y}` in world units (1 unit = 1 px at 1× zoom).

---

## 5. Interfaces

| Surface | Transport | Auth | Who calls |
|---------|-----------|------|-----------|
| `AuthService`, `CharacterService` | gRPC on :50051; grpc-web via tonic-web; also reachable via proxy on :3000 `/grpc/*` | Bearer session token (metadata) | browser client, tools |
| `GameService.Ping/GetCharacter` (existing) | same | none / Bearer | client boot |
| `GET /ws?ticket=` | WebSocket, binary protobuf frames | play ticket | browser client |
| `GET /health`, `GET /metrics` | HTTP | none / internal | infra |
| `POST /admin/reload`, `/admin/kick` | HTTP | admin token | ops |

Server-side events (internal `tokio::broadcast` topics, not wire): `EntityMoved`, `EntityDamaged`,
`EntityDied`, `ItemOwnershipChanged`, `PlayerEnteredWorld`, `PlayerLeftWorld`. Persistence,
interest management, and metrics subscribe to these; gameplay code publishes them.

---

## 6. Rust implementation notes

> Revised by §10.D4, §10.D5, and §10.D6 (Phase 0b).

> **Superseded in part.** The module layout below predates the clean-architecture restructure.
> The authoritative layout is `docs/engineering/architecture.md` §1 (`domain`, `application`,
> `infrastructure`, `interface`), the event bus is NATS per `architecture.md` §2, and every
> mutating endpoint follows `api-guidelines.md` §2 (idempotency) and `database-guidelines.md`
> §1 (one atomic transaction per repository write). The concurrency model (single world thread,
> bounded channels, 100 ms tick) and the crate choices below still stand.

### 6.1 Crates

| Crate | Version (Oct 2026) | Use |
|-------|--------------------|-----|
| `tokio` (full) | 1.x | runtime, timers, channels |
| `tonic`, `tonic-prost`, `tonic-prost-build`, `protox` | 0.14.x | gRPC, codegen (already in workspace) |
| `tonic-web` | 0.14.x | grpc-web for the browser |
| `axum` | 0.8.x | HTTP + WebSocket (`axum::extract::ws`, tokio-tungstenite 0.29 underneath) |
| `tower`, `tower-http` | current | CORS, tracing, compression, `tower_governor` for rate limits |
| `prost`, `prost-types` | 0.14.x | protobuf for WS frames |
| `sqlx` (postgres, runtime-tokio, tls-rustls, uuid, time, migrate) | 0.9.x | persistence |
| `serde`, `serde_json`, `toml` | 1.x / 0.8 | data files, snapshots |
| `arc-swap` | 1.x | hot-swappable `GameData` |
| `notify`, `notify-debouncer-mini` | 8.x | dev hot reload |
| `argon2`, `password-hash`, `rand`, `sha2`, `base64` | 0.6 / current | credentials, tokens |
| `uuid` | 1.x | tx ids |
| `dashmap` or `parking_lot` | current | session registry |
| `tracing`, `tracing-subscriber`, `metrics`, `metrics-exporter-prometheus` | current | observability |
| `anyhow`, `thiserror` | 1.x | errors |
| `glam` (or hand-rolled `Vec2`) | 0.29 | positions |
| `pathfinding` | 4.x | A* over the tile grid |

### 6.2 Module layout (`apps/api/src`)

```
main.rs                 bootstrap: config, data load, db pool, spawn world, serve http + grpc
config.rs               env/CLI → Config (addrs, db url, data dir, realm id, tick ms)
http.rs                 axum Router: /health, /metrics, /ws, /admin/*, grpc-web proxy mount
grpc.rs                 tonic Server: AuthService, CharacterService, GameService
pb/                     generated code re-exports (tonic::include_proto!)
auth/
  mod.rs                AccountService: register/login/logout, session tokens
  password.rs           argon2id hash/verify (spawn_blocking)
  ticket.rs             play-ticket issue/redeem, single-use, 60 s TTL
net/
  session.rs            per-connection task: decode ClientMessage, seq check, rate limit, forward
  ws.rs                 WebSocket upgrade handler, frame <-> prost
  codec.rs              envelope encode/decode shared by ws and future webtransport
world/
  mod.rs                World: tick loop (100 ms), entity store, system ordering
  entity.rs             EntityId, Entity (player/npc/item-on-ground), component structs
  grid.rs               512-unit cells, 3x3 interest sets, active-cell tracking
  movement.rs           MoveTo validation, A* path, MoveStart/MoveStop broadcast
  replication.rs        per-client known set, spawn/despawn, WorldDelta assembly
  commands.rs           ClientMessage -> world intent dispatch
  events.rs             broadcast topics (EntityMoved, EntityDied, ...)
data/
  mod.rs                GameData, load(), validate(), ArcSwap handle, hot reload watcher
  schema/*.rs           serde structs: ClassTemplate, SkillDef, ItemDef, NpcDef, Tables
persist/
  mod.rs                PgPool, migrations, retry helper (40001/40P01)
  characters.rs         load_for_login, write_behind flush (version check), snapshots
  items.rs              ownership transactions, ledger, trade
  accounts.rs           accounts/sessions SQL
stats/                  Phase 1: bonus tables, derived stats (see 01-stat-formulas.md)
```

### 6.3 Concurrency model

- **One world thread.** `World` runs on a dedicated `std::thread` with a `tokio::sync::mpsc` inbox
  (`Command` = intent + session handle) and a 100 ms deadline loop (`Instant`-based, no drift).
  Gameplay code is single-threaded: no locks on entity state, deterministic ordering, easy replay.
- **Network tasks** (one tokio task per connection) never touch world state; they translate frames to
  `Command`s and drain an outbound `mpsc::Sender<Bytes>` the world fills during replication.
- **Persistence** is a separate tokio task pool: the world emits `PersistJob`s (flush character,
  ownership tx) and awaits only on the ownership path (which returns a oneshot the world polls next
  tick; the request is parked as `Pending` until then so the inventory cannot be double-spent while
  the DB round trip is in flight).
- Tick budget alarm: if a tick exceeds 60 ms, log `tracing::warn!` with system timings; if 10
  consecutive ticks exceed 100 ms, shed load by halving replication rate for far entities.

---

## 7. Client implications

- Two generated TypeScript artifacts from `packages/proto`: a Connect/grpc-web client for
  `AuthService`/`CharacterService`/`GameService`, and plain `prost`-equivalent message classes
  (`@bufbuild/protobuf`) for `ClientMessage`/`ServerMessage`.
- `net/Socket.ts`: opens `wss://host/ws?ticket=…`, encodes one `ClientMessage` per binary frame,
  decodes `ServerMessage`, keeps a `client_seq` counter, measures RTT from `Ping/Pong` every 5 s.
- `world/Replica.ts`: entity map keyed by `entity_id`; applies `EntitySpawn/Despawn`, `WorldDelta`;
  for `MoveStart` stores `(from, to, speed, server_tick)` and dead-reckons in `update(dt)` with the
  200 ms interpolation offset; snaps if error > 1024 units.
- Local player: on pointer click, start optimistic movement along the shared tile grid and send
  `MoveTo`; reconcile on `MoveStart` (own `client_seq`) or `MoveRejected`.
- Phaser scene boundaries do not map to server cells; the client simply culls to camera. The server
  guarantees nothing arrives outside the 3 × 3 interest block.
- Reconnect: a dropped socket re-requests a ticket via `EnterWorld`; the server re-attaches to the
  still-live character (kept in world for 60 s after disconnect, standing still, attackable — the L2
  "logout timer" behaviour).

---

## 8. Open questions

1. **WebTransport timing.** Add as a second transport once Safari 26.4+ is the installed majority on
   iOS, or earlier if WebSocket head-of-line blocking shows up in metrics.
2. **Snowflake layout** for ids: 10 bits realm, 41 bits ms timestamp, 12 bits sequence is the
   working assumption; confirm against expected item churn.
3. **Serializable vs row locks** for trades: row locks are chosen; revisit if cross-table invariants
   (clan warehouse, auction house) make lock ordering fragile.
4. **Write-behind interval**: 30 s is a guess; measure DB load at 1k concurrent and adjust.
5. **Tile map sharing**: the collision grid must be byte-identical on client and server. Decide
   between shipping the Tiled JSON to both or having the server export a compact bitset.
6. **Multiple characters per account online** (L2 allows one per account per server): keep L2's
   rule for now; a "box" allowance is a product decision.
7. **Email verification / OAuth**: not in Phase 0; the `accounts` table has room for it.

---

## 9. Sources

- L2J (High Five era, com.l2jserver) source, mirrored at
  <https://github.com/andridgitalbox/l2j-mobius>: `GameTimeController.java` (10 ticks/s),
  `L2World.java` / `L2WorldRegion.java` (SHIFT_BY 12, TILE_SIZE 32768, surrounding regions,
  region activation), `LoginServerThread.java` (waiting clients, `PlayerAuthResponse` handling),
  `network/clientpackets/AuthLogin.java`, `MoveBackwardToLocation.java` (9900-unit limit),
  `ValidatePosition.java`, `model/actor/L2Character.java` (`calculateTimeBetweenAttacks`,
  `updatePosition`), `model/TradeList.java`, `dist/game/config/Character.properties`.
- L2J Interlude (C6) fork, <https://github.com/Hl4p3x/L2JServer_C6_Interlude>:
  `loginserver/SessionKey.java`, `loginserver/GameServerThread.java`
  (`onReceivePlayerAuthRequest`), `loginserver/network/serverpackets/LoginOk.java`, `PlayOk.java`,
  `gameserver/network/gameserverpackets/PlayerAuthRequest.java`.
- L2 login protocol write-ups: <https://github-wiki-see.page/m/kukfa/mmoserver/wiki/Login-Server-Protocol>,
  <https://bitbucket.org/l2emu-unique/netpro/wiki/protocol/Auth.md>.
- grpc-web streaming roadmap: <https://github.com/grpc/grpc-web/blob/master/doc/streaming-roadmap.md>.
- tonic-web docs (unary + server-streaming only): <https://docs.rs/tonic-web/latest/tonic_web/>.
- tonic docs: <https://docs.rs/tonic/latest/tonic/>.
- Connect protocol reference: <https://connectrpc.com/docs/protocol/>.
- connect-bidi-web (WebSocket/WebTransport transports for Connect): <https://github.com/sudorandom/connect-bidi-web>.
- gRPC over WebTransport analysis: <https://kmcd.dev/posts/grpc-webtransport/>.
- WebTransport browser support: <https://caniuse.com/webtransport>.
- axum WebSocket docs: <https://docs.rs/axum/latest/axum/extract/ws/index.html>.
- Gaffer on Games: *What Every Programmer Needs To Know About Game Networking*
  <https://gafferongames.com/post/what_every_programmer_needs_to_know_about_game_networking/>;
  *Snapshot Interpolation* <https://gafferongames.com/post/snapshot_interpolation/>;
  *State Synchronization* <https://gafferongames.com/post/state_synchronization/>.
- Gabriel Gambetta, *Fast-Paced Multiplayer* series: <https://www.gabrielgambetta.com/client-server-game-architecture.html>.
- Boulanger, Kienzle, Verbrugge, *Comparing Interest Management Algorithms for Massively Multiplayer
  Games* (NetGames 2006): <https://www.sable.mcgill.ca/~clump/papers/boulanger-06-comparing.pdf>.
- CCP Games, *The Server Technology of EVE* (GDC China 2010): <https://gdcvault.com/play/1014168/The-Server-Technology-of-EVE>.
- Photon Engine, *Building an MMO Backend: 7 Years, 10 Iterations, One Seamless World*:
  <https://blog.photonengine.com/?p=7372>.
- PostgreSQL docs: explicit locking <https://www.postgresql.org/docs/current/explicit-locking.html>;
  transaction isolation <https://www.postgresql.org/docs/current/transaction-iso.html>.
- sqlx docs: <https://docs.rs/sqlx/latest/sqlx/>.
- argon2 crate docs: <https://docs.rs/argon2/latest/argon2/>.
- OWASP Password Storage Cheat Sheet (Argon2id parameters):
  <https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html>.
- notify crate docs: <https://docs.rs/notify/latest/notify/>.
- toml crate docs: <https://docs.rs/toml/latest/toml/>.
- Write-behind pattern overview: <https://oneuptime.com/blog/post/2026-01-30-write-behind-pattern/markdown>.

---

## 10. Revisions (Phase 0b, 2026-10-07)

### 10.D1 Identity provider

- **Status:** Accepted
- **Context:** §3.5 originally specified custom account management with Argon2id password hashing
  stored directly in Postgres, short-lived session tokens generated by `POST /auth/login`, and an
  in-memory play ticket store. Corresponding database schemas (`accounts` with `password_phc` and
  `sessions` with `token_hash`) were defined in §4.1, along with a custom `AuthService` in
  `auth.proto` (§4.2) and the interface catalogue (§5). This design required the game server to
  manage, hash, and persist user credentials directly.
- **Decision:** **Keycloak**, self-hosted in Compose, realm config as code, device authorization
  grant for the client (rejected: Zitadel, Auth0/Clerk, Epic Online Services). Most mature OIDC
  server; device flow suits a game client (no embedded browser; CEF is broken on Linux/Wayland);
  federates Epic/Steam/Google later. The server never sees a password.
- **Consequences:** The §3.5 Argon2 password design is withdrawn; the server stores no credentials.
  In §4.1, the `sessions` table is removed, and `accounts` is reduced to identity mapping (`id = IdP
  sub`, `created_at`, `last_login`) provisioned on first login via an idempotent `EnsureAccount` use
  case. The custom `AuthService` RPCs in `auth.proto` (§4.2) and §5 are retired. Inbound
  authentication is verified via Keycloak JWKS public keys cached in
  `infrastructure/auth/keycloak.rs`, with `AccountId` injected by an async tower layer (Revision 1 #15). The client
  authenticates via OAuth 2.0 Device Authorization Grant. The play ticket mechanism in §3.5 remains,
  issued through `SessionService.IssuePlayTicket` (Story 1.4) to authorize the WebSocket upgrade.

### 10.D2 ORM

- **Status:** Accepted
- **Context:** §3.3 originally selected raw `sqlx` 0.9 with compile-time query verification
  (`query!`) against a dev database, explicitly rejecting SeaORM to retain hand-written SQL for
  hot-path tables like inventory and trade. In §6.1, `sqlx` was specified as the primary persistence
  dependency.
- **Decision:** **SeaORM** on the existing sqlx pool; `sea-orm-migration` for migrations; entities
  generated in CI (rejected: Diesel, raw sqlx). Async, same pool, transactions with the same
  atomicity rules. Repository ports and their tests do not change.
- **Consequences:** The §3.3 rejection of SeaORM is reversed; SeaORM is adopted across the
  persistence layer while continuing to utilize the existing `sqlx` Postgres connection pool. Schema
  migrations transition from embedded `sqlx::migrate!` to `sea-orm-migration`. Strongly typed entity
  definitions in `infrastructure/postgres/entities/` are automatically generated by `sea-orm-cli`
  with CI validation, superseding manual SQL mapping structs. The strict transaction semantics in
  §3.3 (single ownership transactions, ordered `SELECT ... FOR UPDATE`, and retryable deadlock
  handling) are preserved and implemented using SeaORM's `TransactionTrait`. Repository ports and
  their tests remain unchanged.

### 10.D3 Client request/response

- **Status:** Accepted
- **Context:** §3.2 and §5 originally established that request/response RPCs were served over gRPC on
  port 50051 using tonic, with `tonic-web` (`GrpcWebLayer`, `accept_http1(true)`) mounted to support
  browser clients over grpc-web via `@connectrpc/connect-web`. This was required because web browsers
  could not emit HTTP/2 frames.
- **Decision:** **gRPC via TurboLink** in Unreal, tonic on the server (no tonic-web needed)
  (rejected: HTTP/JSON). One protocol everywhere; TurboLink bundles protoc, so its generated classes
  also replace the hand-written WebSocket codec. Cost: first-build fights on Linux (see R1).
- **Consequences:** `tonic-web` is removed from crate dependencies (§6.1), server startup (§3.1),
  and interface bindings (§3.2, §5). The Unreal Engine client communicates directly with tonic over
  standard HTTP/2 gRPC through the TurboLink plugin, eliminating grpc-web translation, proxy
  mappings on `:3000 /grpc/*`, and browser networking shims. TurboLink generated C++ classes also
  replace hand-written binary codecs for WebSocket frames, sharing protobuf contracts directly with
  the server. The real-time world channel remains a single WebSocket at `GET /ws?ticket=` on axum.

### 10.D4 Replay scope

- **Status:** Accepted
- **Context:** §6.3 defined a single-threaded world simulation loop to achieve deterministic ordering
  and "easy replay," but left replay verification and recording scope undefined. Earlier sections did
  not specify how session traffic would be captured or how deterministic execution would be validated
  against regressions.
- **Decision:** **Server-side**: log inbound and outbound frames per session; headless replay asserts
  identical outbound bytes (rejected: Client-side frame recording too). Enough to reproduce any
  server bug; client recording is only useful once client prediction exists.
- **Consequences:** Replay scope is formally established as server-side only; client-side frame
  recording is deferred until client prediction is introduced. A headless verification utility
  (`apps/api/src/bin/nightfall-replay.rs`) loads zone initial state snapshots and inbound session
  streams, steps the zone actor using injected tick clocks, and asserts byte-identical output against
  recorded outbound frames. Any discrepancy in sequence, tick, or message bytes halts replay to pinpoint
  non-deterministic regressions.

### 10.D5 Session event log

- **Status:** Accepted
- **Context:** §5 identified server-side events as in-process `tokio::broadcast` topics, while §6 did
  not provide a mechanism for persistent session recording or frame history. Without an append-only
  frame log, headless verification and offline debugging were impossible without imposing write load
  on the primary relational database.
- **Decision:** **NATS JetStream**, one stream `NF_SESSIONS`, subjects `nightfall.session.<id>.in` /
  `.out`, retention 7 days (rejected: Postgres table, both). Already running; append-only with replay
  by sequence; keeps write load off the game DB. Archival to object storage is a later story.
- **Consequences:** Session frame recording is implemented on NATS JetStream under stream
  `NF_SESSIONS`, capturing `nightfall.session.<id>.in` and `nightfall.session.<id>.out` subjects with a
  7-day retention window. Network session tasks log inbound and outbound frames asynchronously via
  bounded channels with drop-counter metrics, ensuring frame logging cannot block the tick loop. This
  replaces transient in-memory broadcasting (§5, §6) with durable, sequential event logs for replay,
  keeping high-frequency session writes off Postgres.

### 10.D6 Telemetry backend

- **Status:** Accepted
- **Context:** §5 and §6.1 specified observability using `tracing`, `tracing-subscriber`, `metrics`,
  and `metrics-exporter-prometheus` exposed on a `/metrics` scrape endpoint. Distributed trace
  storage, centralized log search, and correlated tick diagnostics were not integrated into the local
  development environment.
- **Decision:** **Grafana LGTM** all-in-one in Compose, OpenTelemetry OTLP from the server (rejected:
  Datadog, Honeycomb). Free, local, OTel-native; forward to a SaaS later by changing one endpoint.
- **Consequences:** The server telemetry stack adopts OpenTelemetry (`tracing-opentelemetry` with an
  OTLP exporter) targeting a self-hosted Grafana LGTM container (Loki, Grafana, Tempo, Mimir) in
  Docker Compose. Structured JSON logs emitted by the server incorporate `request_id`, `session_id`,
  and `tick` fields, linked directly to distributed traces. In §6.1, standalone Prometheus scraping is
  superseded by unified OTLP export, and dashboard JSON definitions are maintained in `infra/grafana/`.

### 10.D7 Determinism

- **Status:** Accepted
- **Context:** §3.2 and §4.2 specified entity positions, movement interpolation, and delta messages
  using floating-point numbers (`Position` with `float x, float y` and `float speed`), mirrored in
  §4.1 by `REAL` columns in the `characters` table. While §3.2 established a 100 ms (10 Hz) simulation
  clock, floating-point math across different hardware architectures and compiler targets compromises
  deterministic simulation replay.
- **Decision:** Fixed 100 ms tick; positions as `i32` fixed-point (1/1000 tile); `ChaCha12` RNG
  seeded per zone per session; every command stamped with the tick it is applied on (rejected: f32
  positions). "Perfect replay" is impossible otherwise. Floats drift across builds and CPUs.
- **Consequences:** All spatial coordinates, speeds, and movement calculations in `domain/zone` are
  refactored to integer fixed-point math (`Fixed(i32)` representing 1/1000th of a tile). In §4.1 and
  §4.2, floating-point representations (`REAL` in SQL, `float` in proto `Position`) are replaced with
  fixed-point integers. Float arithmetic is prohibited in simulation logic and enforced in CI via
  `clippy::float_arithmetic` deny. Zone simulation randomness is driven by `ChaCha12` seeded
  deterministically from `(zone_id, session_epoch)`, and all incoming client commands are stamped with
  their applied `Tick(u64)` to guarantee exact replay reproducibility.

