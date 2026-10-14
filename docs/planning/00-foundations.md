# Phase 0 — Foundations

This document gives the architecture, networking, persistence, data pipeline, and accounts for
Nightfall. All later phases use the decisions in this document.

---

## 1. Purpose and scope

**Delivers**

- The client/server topology: an authoritative game server, a thin auth/HTTP surface, and a browser
  Phaser client. The client only renders and sends intent.
- The wire model: how request/response calls and the real-time world stream get to a browser. A
  browser cannot use raw gRPC.
- The simulation clock: tick rate, interest management, movement sync, and the quantity of client
  prediction that a click-to-move MMO must have. (This quantity is small intentionally.)
- Persistence: the Postgres schema strategy, write-behind caching of character state, item ownership
  and trade atomicity, snapshot/rollback.
- The data pipeline: how designers write skills, items, monsters, and class tables as data files. The
  server validates these files at boot and hot-reloads them in development.
- Accounts and security: account vs character, Argon2id password hashing, session tokens, the
  login→game handoff ticket, character slots.
- A crate list and module layout for `apps/api/src`.

**Excludes**

- All gameplay formulas (Phase 1), class/race content (Phase 2), combat (Phase 3).
- Multi-process world sharding. Our design makes it possible, but we do not build it now.
- Anti-cheat other than server authority and basic rate limits (Phase 9).

---

## 2. Reference: how Lineage 2 does it

### 2.1 Login server / game server split

Lineage 2 runs two separate daemons. The **login server** (default port 2106) is responsible for
accounts. The **game server** (default port 7777) is responsible for the world. A private TCP link
connects them (L2J `GameServerThread` on the login side, `LoginServerThread` on the game side). The
client never communicates with both at the same time. One server gives the client to the other with a
**session key**. The session key is four 32-bit integers:

```java
// L2J (Interlude branch) loginserver/SessionKey.java
public int playOkID1, playOkID2, loginOkID1, loginOkID2;
public boolean checkLoginPair(int loginOk1, int loginOk2) {
    return (loginOkID1 == loginOk1) && (loginOkID2 == loginOk2);
}
```

The full sequence, with the L2J packet names:

| Step | Direction | Packet | Payload of interest |
|------|-----------|--------|---------------------|
| 1 | LS → C | `Init` | Blowfish key for this connection, RSA public key for password |
| 2 | C → LS | `RequestAuthLogin` | RSA-encrypted username/password |
| 3 | LS → C | `LoginOk` | `loginOk1`, `loginOk2` (opcode 0x03) |
| 4 | C → LS | `RequestServerList` | — |
| 5 | LS → C | `ServerList` | realm list, per-realm population/status |
| 6 | C → LS | `RequestServerLogin` | selected server id + `loginOk1/2` |
| 7 | LS → C | `PlayOk` | `playOk1`, `playOk2` (opcode 0x07) |
| 8 | C → GS | `ProtocolVersion`, GS replies `KeyPacket` | per-connection XOR key |
| 9 | C → GS | `AuthLogin` | account, `playOk1/2`, `loginOk1/2` |
| 10 | GS → LS | `PlayerAuthRequest` | account + all four ints |
| 11 | LS → GS | `PlayerAuthResponse` | authed yes/no; LS removes the key |
| 12 | GS → C | `CharSelectionInfo` | character list (slots) |

These points are good to copy:

- The game server never sees a password.
- The login server keeps the session key only until the key is used (`removeAuthedLoginClient` on
  success).
- The game server keeps the client in a `_waitingClients` list until the login server answers.
- A bad key closes the socket with `LoginFail.SYSTEM_ERROR_LOGIN_LATER`.

Login-server packets are Blowfish-ECB with a static key and an XOR checksum. The game-server stream is
a rolling XOR cipher. Today, neither cipher gives real cryptographic protection. This is the reason
that we use TLS in their place.

### 2.2 World model: one seamless world per shard, grid regions

An L2 "server" (Bartz, Sieghardt, ...) is a **shard**: one JVM process that holds one full copy of
the world. There are no zones and no loading screen. The world is continuous. In the process, the
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

Thus, the live map is 18 × 17 tiles of 32768 units. Each tile is 8 × 8 regions of 4096 units, so
the result is a 144 × 136 region grid. Each `L2WorldRegion` holds its objects and a precalculated list
of its 8 `_surroundingRegions`. Visibility ("known objects") is the 3 × 3 block of regions around you,
which is 12288 units square. If a region and all its neighbours have no player, the region
**deactivates**. This stops the AI and regeneration tasks for empty parts of the map. We copy this
interest-management scheme.

### 2.3 Clock and packet model

The L2J `GameTimeController` runs at **10 ticks per second** (`TICKS_PER_SECOND = 10`,
`MILLIS_IN_TICK = 100`). Movement is integrated for each tick: `distPassed = moveSpeed * (ticks since last
update) / TICKS_PER_SECOND`. In-game time runs 6 days for each real day.

Movement is **click-to-move, server-planned**. The client sends `MoveBackwardToLocation(targetX,
targetY, targetZ, originX, originY, originZ)`. The server rejects a request if `dx² + dy² > 9900²`,
and if not, it sets `AI_INTENTION_MOVE_TO` and calculates the path. Then it sends `MoveToLocation(objectId,
from, to)` to all clients that know the object. Each client then dead-reckons the mover at its known speed.
At intervals, the client sends `ValidatePosition(x, y, z, heading)`. The server compares this to its
own position. Then, as the `CoordSynchronize` config sets, it trusts the z of the client, snaps the
client, or ignores the message. There is **no input prediction and no rollback**. The model works
because movement is a straight line at a known speed from a known start.

The server also controls attack timing. In High Five, the time between swings is `470000 / pAtkSpd`
ms (`500000` in older branches). Bows use `1500 * 345 / pAtkSpd`. Casting uses
`skillTime * 333 / mAtkSpd`. These constants are part of Phase 1/3, but they set the latency budget. A
300 atk-speed fighter swings each 1567 ms. Thus, a 100 ms tick and 150–250 ms of interpolation delay
are not visible.

### 2.4 Persistence

L2J stores all data in MySQL/MariaDB and caches it in memory. `Character.properties` has
`CharacterDataStoreInterval = 15` (minutes) for the periodic character store.
`LazyItemsUpdate` and `UpdateItemsOnCharStore` control whether the server writes item rows lazily.
Item instances are rows in `items`, keyed by `object_id`, with `owner_id`, `loc` (INVENTORY, PAPERDOLL,
WAREHOUSE, ...), `loc_data` (slot), `count`, `enchant_level`. The server validates a trade
(`TradeList`) in memory, and then changes and writes both inventories. This is the classic shape. It
also caused the classic dupe bugs: two code paths changed an inventory without a common lock. This is
the reason that section 3.3 makes one ownership transaction necessary.

### 2.5 Data authoring

L2J moved from SQL tables to **XML + XSD** for static data. The files are:

- `stats/statBonus.xml` (the six stat→multiplier tables).
- `stats/experience.xml`.
- `stats/chars/baseStats/<Class>.xml` (base stats, per-level HP/MP/CP).
- `stats/skills/*.xml`, `stats/items/*.xml`, `stats/npcs/*.xml`.
- `config/*.properties` for tunables (`MaxRunSpeed = 300`, `MaxPAtkSpeed = 1500`, `MaxMAtkSpeed = 1999`,
  `MaxPCritRate = 500`, `MaxMCritRate = 200`, `MaxEvasion = 250`, `PartyXpCutoffGaps = 0,9;10,14;15,99`,
  `PartyXpCutoffGapPercent = 100;30;0`).

Each XML has an XSD. A `DocumentParser` subclass loads it one time at boot. Only scripts have hot
reload.

---

## 3. Design decisions for Nightfall

### 3.1 Topology: one binary, two listeners, one shard

**Decision.** `nightfall-api` stays one Rust binary. It runs:

- an **axum** HTTP/1.1+WebSocket listener on `:3000`. It is for auth, account REST, the browser
  real-time channel, and health.
- a **tonic** gRPC listener on `:50051`. It is for native tools (GM console, load bots, future native
  client). **tonic-web** is on, so the browser can also call unary/server-streaming RPCs over
  grpc-web.

The L2 "login server" and "game server" become two **modules in one process** (`auth` and
`world`). The same ticket handoff that L2 uses separates them. Thus, we can later split them into two
deployables without a protocol change. One process controls one **realm** (shard). Each realm is a
separate process with a separate database. Each character id includes a `realm_id`, so cross-realm
transfers are possible later.

**Alternatives considered.**

- (a) A separate login daemon from day one. This is correct for the long term, but it doubles the
  deploy surface for a team of one person. The ticket design keeps this option open.
- (b) A zoned world (one process for each zone, loading screens). Scaling is simpler, but the feel is
  worse. Also, the seamless world of L2 is a part of what we copy.
- (c) One shard for all players (EVE model). This must have a distributed simulation. It is out of
  scope.

### 3.2 Networking

> Revised by §10.D3 and §10.D7 (Phase 0b).

#### Why not plain gRPC to the browser

Browsers do not give access to HTTP/2 frames. Thus, they cannot use native gRPC. The two
browser-compatible protocols are **grpc-web** and **Connect**. In browsers, both support **unary and
server-streaming only**. The grpc-web project formally declined client/bidi streaming (its roadmap
points to WebTransport as the answer in the future). tonic-web documents the same limit: "unary and
server-streaming calls only". The protocol doc of Connect says that bidi must have HTTP/2 end-to-end,
and `fetch()` does not give us that. **WebTransport** (HTTP/3, QUIC streams) now works in Chrome 97+,
Firefox 114+, Edge 98+ and Safari 26.4+ (caniuse 91.6% global). But it must have UDP/443
reachability and a QUIC-capable edge, and it has no DevTools inspection now. **WebSocket** works in
all browsers. It is TCP-only, with one ordered stream.

| Option | Browser bidi | Ordered | HoL blocking | Infra | Verdict |
|--------|--------------|---------|--------------|-------|---------|
| grpc-web (tonic-web) | server-stream only | yes | yes | HTTP/1.1 ok | request/response + server push only |
| Connect | server-stream only (bidi through connect-bidi-web add-on) | yes | yes | HTTP/1.1 ok | same, better curl ergonomics |
| WebSocket (axum) | yes | yes | yes | any LB | **real-time channel now** |
| WebTransport | yes, multi-stream | per stream | no | HTTP/3 edge, UDP | real-time channel later, feature-flagged |

**Decision.**

1. **Request/response** (login, character list, create/delete character, shop lists, mail, and all
   other calls that are not latency-critical): gRPC services in `game.proto`. tonic serves them, and
   `tonic-web` (`GrpcWebLayer`, `accept_http1(true)`) gives the browser access to them. The client
   uses `@connectrpc/connect-web` with the grpc-web protocol. Thus, one generated TypeScript client
   is sufficient for both.
2. **Real-time world channel**: one **WebSocket** at `GET /ws?ticket=…` on the axum listener. It
   carries length-free **protobuf** frames (`ClientMessage` / `ServerMessage` envelopes, one message
   for each binary WebSocket frame). The message types are in the same `.proto` package, so the client
   decodes them with the same generated code. WebTransport is a planned second transport behind the
   same envelope. (connect-bidi-web shows the shape: one QUIC stream for each RPC, the same
   flag+length framing.)
3. No part of the world simulation knows on which transport a session arrived. `net::session` gives
   the world an `mpsc::Sender<ServerMessage>` and a `mpsc::Receiver<ClientMessage>`.

#### Clock, snapshots, interest management

- **Simulation tick: 100 ms (10 Hz)**, the same as L2. All world mutations occur on the tick thread.
  Network tasks only enqueue intents. The reason: click-to-move has no per-frame input. The slowest
  observable action (an attack swing) is ≥ 500 ms at the hard cap of 1500 atk speed.
- **Snapshot rate: 10 Hz per client**. Each tick sends one `WorldDelta`. It has only the entities in
  the area of interest of the client, and only the fields that changed. The server does not send
  movement for each tick. It sends `MoveStart{entity, from, to, speed, server_tick}` one time, and
  `MoveStop`/`Teleport` when a change occurs, exactly as `MoveToLocation` does. Between these
  messages, the clients dead-reckon.
- **Interpolation delay: 200 ms** for remote entities (2 ticks + ~0–30 ms jitter margin). This
  follows the rule of thumb "lose two packets and still have something to interpolate towards". The
  client smooths position corrections over 200 ms. If the error is more than 2 cells, the client
  snaps the position.
- **Area of interest**: a uniform grid, cell = **512 world units** (16 tiles of 32 px). Each entity
  is registered in one cell. The interest set of a client is the **3 × 3 block** around its cell
  (1536 units square). This is sufficiently larger than the 1468-unit diagonal of a 1280 × 720
  viewport. When an entity enters or leaves the set, the server sends `EntitySpawn` /
  `EntityDespawn`. These are the L2 `CharInfo` / `DeleteObject`. If a cell has no player in its
  3 × 3 neighbourhood, the cell is **inactive**. Monster AI, regen, and respawn timers do not run in
  that cell. This is the same as `L2WorldRegion.setActive`. Tile-based interest management is a
  well-studied, cheap approximation of ideal visibility filtering (Boulanger et al.).
- **Priority**: if the outbound queue of a client is more than 64 KB, the server drops the
  non-essential updates for that client. (It drops chat first, then remote entity cosmetic updates.) It keeps the own-entity
  and combat results. We will not add a per-entity priority accumulator (Gaffer "state
  synchronization") until we measure a problem.

#### Movement sync and prediction

Click-to-move must have an **optimistic start, not rollback**:

1. The player clicks. The client immediately starts to move its own avatar to the point. It moves
   along a client-side path over the same tile map that the server has (both use the same collision
   grid).
2. The client sends `MoveTo{target, client_seq}`.
3. The server validates the request (distance ≤ 20 cells, not rooted/stunned/dead, walkable). It
   plans the authoritative path. Then it sends `MoveStart`, tagged with `client_seq`, to the
   interested clients, which include the mover.
4. The mover compares the server path with its optimistic path. If the destination is different, it
   blends over 200 ms. If the server rejected the move (`MoveRejected{client_seq, reason}`), it snaps
   back.

Lag compensation is only **range tolerance**. When the server validates an attack/skill, it accepts
`distance ≤ range + max(50, attacker_speed * rtt/2)`. `rtt` is the smoothed round trip, measured by
`Ping` frames each 5 s. The server does not rewind other entities (there is no hitscan).

### 3.3 Persistence

> Revised by §10.D2 (Phase 0b).

**Postgres 16, sqlx 0.9** (`runtime-tokio`, `tls-rustls`, `postgres`, `uuid`, `time`, `migrate`),
with compile-time checked queries against a dev database, and migrations embedded with
`sqlx::migrate!`. We examined SeaORM and rejected it. We want hand-written SQL for the few hot paths
(inventory, trade), and the sqlx `query!` gives type checking without an ORM layer.

**Two tiers of state.**

| Tier | Examples | Write policy |
|------|----------|--------------|
| Volatile character state | position, hp/mp/cp, xp, buffs, quest counters | **write-behind**: dirty flag, flushed each 30 s, on logout, on death, on realm transfer, on server shutdown |
| Ownership state | item rows, adena, warehouse, mail attachments, trade | **write-through, transactional**: the in-memory inventory changes only after `COMMIT` |

The volatile flush is one `UPDATE characters SET ... WHERE id = $1 AND version = $2` with a
`version` counter (optimistic lock). Thus, a stale server instance can never overwrite a newer save.
We selected thirty seconds, in contrast to the L2J default of fifteen minutes. A crash loses a
maximum of 30 s of XP. The write (one row for each online character each 30 s) is a small load.

**Items and dupe prevention.** Each item instance is one row, with a primary key that the game
never uses again:

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

1. All ownership mutations go through `persist::items`. This module does these steps:
   - It opens one transaction.
   - It locks the applicable rows with `SELECT ... FOR UPDATE` **ordered by `id`**. (The Postgres
     explicit-locking docs say that a consistent lock order prevents deadlocks.)
   - It examines `owner_id`/`location`/`count` again, against the values that the request expected.
   - It applies the updates, inserts the ledger rows, and commits.

   Only after these steps does the world thread change the in-memory inventory and send
   `InventoryUpdate`.
2. A player trade is **one transaction** for both inventories. The offered rows of both parties are
   locked, validated against the confirmed offer, and moved. Adena is a stackable item row, so it uses
   the same path. Partial application is not possible. If a crash occurs before `COMMIT`, both
   inventories stay as they were.
3. Stackables never go above `i64`. `count` has a `CHECK (count > 0)`, and a merge removes the
   source row in the same transaction.
4. Transactions run at **Read Committed** with explicit row locks. We do not depend on Serializable.
   But the `persist` layer treats SQLSTATE `40001` and `40P01` (deadlock) as retryable, a maximum of
   3 times.
5. Client requests have a `client_seq`. The session rejects a duplicate seq. This stops "double
   click the vendor" dupes at the edge.

**Snapshots and rollback.** A nightly `pg_dump` and continuous WAL archiving give point-in-time
recovery. The `item_ledger` is the audit trail. To investigate a possible dupe, replay the ledger
rows for the `item_id`. A targeted rollback is a `DELETE`/`UPDATE` on items that uses the ledger data,
not a full restore. Also, the server copies the volatile character rows into
`character_snapshots` (JSONB) at logout, and keeps them for 7 days for GM restores.

### 3.4 Data pipeline

**Decision: TOML files in `packages/data/`, loaded at boot into immutable structs, validated,
hot-reloadable in dev.**

- **Format**: TOML (`toml` 0.8 through serde). It has comments, no quoting noise, and diffs that are
  easy to read. Also, all phase documents (2 through 6, 9) already sketch their data in TOML.
  - Tagged enums such as skill effects are arrays of tables with a `kind` field
    (`[[effects]] kind = "heal" power = 50`). serde deserialises them with `#[serde(tag = "kind")]`.
    This is the one place where TOML is less easy to use than RON.
  - We examined RON for its native enum syntax, and rejected it. We want one format that designers
    and tooling (editors, Tiled exports, scripts) all understand.
  - We rejected JSON because it has no comments.
  - The L2J XML+XSD is the nearest analogue. TOML plus serde-derived structs gives us the schema at
    no cost.
- **Layout**: one directory for each entity kind (`classes/`, `skills/`, `items/`, `npcs/`, `zones/`,
  `tables/`). One file for each entity or table. The filename is the same as the `id` of the entity.
- **Loading**: `data::load(dir) -> Result<GameData, Vec<DataError>>` parses all files. Then it runs
  cross-reference validation:
  - Each `skill_id` that a class refers to exists.
  - Each drop template refers to a real item.
  - The XP table is strictly increasing.
  - The stat tables cover 1..=max_stat.

  If there is an error, boot fails and prints all the errors.
- **Access**: `Arc<ArcSwap<GameData>>`. Systems read through a cheap `load()`. Hot reload swaps the
  pointer atomically. Live entities hold ids, never references into `GameData`. Thus, a swap cannot
  make them invalid.
- **Hot reload (dev only)**: `notify` 8 + `notify-debouncer-mini` monitors `packages/data`, and runs
  the full load again. If the load is successful, it swaps the data. If the load fails, it logs the
  errors and keeps the old data. A `POST /admin/reload` does the same in staging, behind the admin
  token.
- **Build step**: a moon task `data:check` runs the loader in CI. Thus, a bad data commit fails
  quickly.

Example class template (Phase 1 defines the fields):

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

- **Account vs character.** `accounts` holds the login identity. `characters` holds a maximum of
  **7** characters for each account for each realm (the default slot count of L2). Characters are
  soft-deleted with a **7-day** grace (`deleted_at`). After the grace, a reaper hard-deletes the rows
  and moves the items to the ledger as destroyed.
- **Passwords**: Argon2id (`argon2` 0.6, PHC string output `$argon2id$v=19$...`) with the OWASP
  recommended `m=19456 KiB, t=2, p=1`. Verification reads the params from the stored PHC string.
  Thus, we can increase the parameters later and rehash at the next login. Hashing runs on
  `tokio::task::spawn_blocking`.
- **Session token** (`POST /auth/login` → 32 random bytes, base64url). The server stores it as SHA-256 in
  `sessions(token_hash, account_id, expires_at, created_ip)`, with 7-day expiry. The client sends it as `Authorization:
  Bearer` on gRPC/HTTP calls. One account can hold many sessions (multiple devices).
- **Play ticket** — our `playOk`. `EnterWorld(character_id)` over gRPC returns a **single-use ticket
  valid 60 s**. The `auth` module stores it in memory, keyed by ticket, and bound to `(account_id,
  character_id, client_ip)`. The WebSocket upgrade must present it; the `world` module asks `auth`
  to redeem it (the in-process equivalent of `PlayerAuthRequest`/`PlayerAuthResponse`). Then it
  spawns the character. An account can have only one character in the world for each realm. If the
  server redeems a ticket for an account that is already in the world, it kicks the old session (the
  L2 duplicate-login handling).
- **Transport security**: the reverse proxy (Caddy/nginx) terminates TLS for both listeners. The
  binary itself uses plaintext on loopback. In production, gRPC and WebSocket both go over HTTPS/WSS.
- **Rate limits**: `tower_governor` on `/auth/*` (5/min/IP). WebSocket inbound is a maximum of 30
  messages/s for each session. If the overflow continues, the server disconnects the session.
- Never log passwords, tokens, or tickets. `tracing` fields are allow-listed.

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
| `AuthService`, `CharacterService` | gRPC on :50051; grpc-web through tonic-web; also available through proxy on :3000 `/grpc/*` | Bearer session token (metadata) | browser client, tools |
| `GameService.Ping/GetCharacter` (existing) | same | none / Bearer | client boot |
| `GET /ws?ticket=` | WebSocket, binary protobuf frames | play ticket | browser client |
| `GET /health`, `GET /metrics` | HTTP | none / internal | infra |
| `POST /admin/reload`, `/admin/kick` | HTTP | admin token | ops |

Server-side events (internal `tokio::broadcast` topics, not wire): `EntityMoved`, `EntityDamaged`,
`EntityDied`, `ItemOwnershipChanged`, `PlayerEnteredWorld`, `PlayerLeftWorld`. Persistence,
interest management, and metrics subscribe to these events. Gameplay code publishes them.

---

## 6. Rust implementation notes

> Revised by §10.D4, §10.D5, and §10.D6 (Phase 0b).

> **Superseded in part.** The module layout below is older than the clean-architecture restructure.
> The authoritative layout is `docs/engineering/architecture.md` §1 (`domain`, `application`,
> `infrastructure`, `interface`). The event bus is NATS, as in `architecture.md` §2. Each
> mutating endpoint obeys `api-guidelines.md` §2 (idempotency) and `database-guidelines.md`
> §1 (one atomic transaction for each repository write). The concurrency model (one world thread,
> bounded channels, 100 ms tick) and the crate choices below are still correct.

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

- **One world thread.** `World` runs on a dedicated `std::thread`. It has a `tokio::sync::mpsc` inbox
  (`Command` = intent + session handle) and a 100 ms deadline loop (`Instant`-based, no drift).
  Gameplay code is single-threaded. Thus, there are no locks on entity state, the sequence is
  deterministic, and replay is easy.
- **Network tasks** (one tokio task for each connection) never touch world state. They change frames
  into `Command`s, and drain an outbound `mpsc::Sender<Bytes>` that the world fills during
  replication.
- **Persistence** is a separate tokio task pool. The world emits `PersistJob`s (flush character,
  ownership tx). It waits only on the ownership path. That path returns a oneshot that the world
  polls at the next tick. Until then, the request stays as `Pending`. Thus, the inventory cannot be
  double-spent while the DB round trip is in progress.
- Tick budget alarm: if a tick is more than 60 ms, log `tracing::warn!` with the system timings. If 10
  consecutive ticks are more than 100 ms, decrease the load: halve the replication rate for far
  entities.

---

## 7. Client implications

- Two generated TypeScript artifacts from `packages/proto`: a Connect/grpc-web client for
  `AuthService`/`CharacterService`/`GameService`, and plain `prost`-equivalent message classes
  (`@bufbuild/protobuf`) for `ClientMessage`/`ServerMessage`.
- `net/Socket.ts`: opens `wss://host/ws?ticket=…`, encodes one `ClientMessage` for each binary frame,
  decodes `ServerMessage`, keeps a `client_seq` counter, and measures RTT from `Ping/Pong` each 5 s.
- `world/Replica.ts`: an entity map keyed by `entity_id`. It applies `EntitySpawn/Despawn` and
  `WorldDelta`. For `MoveStart`, it stores `(from, to, speed, server_tick)` and dead-reckons in
  `update(dt)` with the 200 ms interpolation offset. If the error is more than 1024 units, it snaps.
- Local player: on a pointer click, start optimistic movement along the shared tile grid and send
  `MoveTo`. Reconcile on `MoveStart` (own `client_seq`) or `MoveRejected`.
- Phaser scene boundaries are not related to server cells. The client only culls to the camera. The
  server makes sure that nothing arrives outside the 3 × 3 interest block.
- Reconnect: if the socket drops, the client requests a new ticket through `EnterWorld`. The server
  attaches the client again to the character, which is still live. The server keeps the character in
  the world for 60 s after disconnect. The character stands still, and other players can attack it.
  This is the L2 "logout timer" behaviour.

---

## 8. Open questions

1. **WebTransport timing.** Add it as a second transport when Safari 26.4+ is the installed majority
   on iOS. Add it earlier if metrics show WebSocket head-of-line blocking.
2. **Snowflake layout** for ids: our working assumption is 10 bits realm, 41 bits ms timestamp,
   12 bits sequence. Make sure that this agrees with the expected item churn.
3. **Serializable vs row locks** for trades: we selected row locks. Examine this again if
   cross-table invariants (clan warehouse, auction house) make the lock order fragile.
4. **Write-behind interval**: 30 s is an estimate. Measure the DB load at 1k concurrent, and change
   the interval as necessary.
5. **Tile map sharing**: the collision grid must be byte-identical on client and server. Select one
   method: send the Tiled JSON to both, or let the server export a compact bitset.
6. **Multiple characters per account online** (L2 lets one character be online for each account for
   each server): keep the L2 rule for now. A "box" allowance is a product decision.
7. **Email verification / OAuth**: not in Phase 0. The `accounts` table has space for it.

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
- **Context:** At first, §3.5 specified custom account management. It used Argon2id password hashing
  stored directly in Postgres, short-lived session tokens from `POST /auth/login`, and an
  in-memory play ticket store. §4.1 defined the related database schemas (`accounts` with
  `password_phc` and `sessions` with `token_hash`). §4.2 defined a custom `AuthService` in
  `auth.proto`, and §5 had the interface catalogue. With this design, the game server had to
  control, hash, and store user credentials directly.
- **Decision:** **Keycloak**, self-hosted in Compose, with the realm config as code. The client uses
  the device authorization grant (rejected: Zitadel, Auth0/Clerk, Epic Online Services). It is
  the most mature OIDC server. The device flow is applicable to a game client (no embedded browser;
  CEF is broken on Linux/Wayland). Later, it can federate Epic/Steam/Google. The server never sees a
  password.
- **Consequences:** We withdraw the §3.5 Argon2 password design. The server stores no credentials.
  In §4.1, the `sessions` table is removed, and `accounts` is reduced to identity mapping (`id = IdP
  sub`, `created_at`, `last_login`) provisioned on first login via an idempotent `EnsureAccount` use
  case. The custom `AuthService` RPCs in `auth.proto` (§4.2) and §5 are retired. Inbound
  authentication uses the Keycloak JWKS public keys, cached in `infrastructure/auth/keycloak.rs`. An
  async tower layer injects the `AccountId` (Revision 1 #15). The client authenticates through the
  OAuth 2.0 Device Authorization Grant. The play ticket mechanism in §3.5 stays.
  `SessionService.IssuePlayTicket` (Story 1.4) issues the ticket to authorize the WebSocket upgrade.

### 10.D2 ORM

- **Status:** Accepted
- **Context:** At first, §3.3 selected raw `sqlx` 0.9 with compile-time query verification
  (`query!`) against a dev database. It explicitly rejected SeaORM, to keep hand-written SQL for
  hot-path tables such as inventory and trade. §6.1 specified `sqlx` as the primary persistence
  dependency.
- **Decision:** **SeaORM** on the sqlx pool that exists; `sea-orm-migration` for migrations;
  entities generated in CI (rejected: Diesel, raw sqlx). It is async, it uses the same pool, and it
  has transactions with the same atomicity rules. Repository ports and their tests do not change.
- **Consequences:** We reverse the §3.3 rejection of SeaORM. All of the persistence layer uses
  SeaORM, and it continues to use the `sqlx` Postgres connection pool that exists. Schema migrations
  move from embedded `sqlx::migrate!` to `sea-orm-migration`. `sea-orm-cli` automatically generates
  the strongly typed entity definitions in `infrastructure/postgres/entities/`, with CI validation.
  These definitions replace the manual SQL mapping structs. The strict transaction semantics in §3.3
  (one ownership transaction, ordered `SELECT ... FOR UPDATE`, and retryable deadlock handling) stay
  the same. The SeaORM `TransactionTrait` implements them. Repository ports and their tests do not
  change.

### 10.D3 Client request/response

- **Status:** Accepted
- **Context:** At first, §3.2 and §5 set that gRPC on port 50051, through tonic, served the
  request/response RPCs. `tonic-web` (`GrpcWebLayer`, `accept_http1(true)`) was mounted to support
  browser clients over grpc-web through `@connectrpc/connect-web`. This was necessary because web
  browsers could not emit HTTP/2 frames.
- **Decision:** **gRPC through TurboLink** in Unreal, and tonic on the server (tonic-web is not
  necessary) (rejected: HTTP/JSON). We use one protocol in all locations. TurboLink bundles protoc,
  so its generated classes also replace the hand-written WebSocket codec. Cost: problems with the
  first build on Linux (see R1).
- **Consequences:** We remove `tonic-web` from the crate dependencies (§6.1), server startup (§3.1),
  and interface bindings (§3.2, §5). The Unreal Engine client communicates directly with tonic over
  standard HTTP/2 gRPC through the TurboLink plugin. This removes the grpc-web translation, the proxy
  mappings on `:3000 /grpc/*`, and the browser networking shims. The TurboLink generated C++ classes
  also replace the hand-written binary codecs for WebSocket frames, and share the protobuf contracts
  directly with the server. The real-time world channel stays one WebSocket at `GET /ws?ticket=` on
  axum.

### 10.D4 Replay scope

- **Status:** Accepted
- **Context:** §6.3 defined a single-threaded world simulation loop to get deterministic ordering
  and "easy replay". But it did not define the replay verification and recording scope. Earlier
  sections did not specify how to capture session traffic, or how to validate deterministic execution
  against regressions.
- **Decision:** **Server-side**: log the inbound and outbound frames for each session. A headless
  replay makes sure that the outbound bytes are identical (rejected: Client-side frame recording
  too). This is sufficient to reproduce all server bugs. Client recording is only useful when client
  prediction exists.
- **Consequences:** Replay scope is formally server-side only. We will not do client-side frame
  recording until client prediction exists. A headless verification utility
  (`apps/api/src/bin/nightfall-replay.rs`) loads the initial state snapshots of the zone and the
  inbound session streams. It steps the zone actor with injected tick clocks. It makes sure that the
  output is byte-identical to the recorded outbound frames. If there is a difference in sequence,
  tick, or message bytes, the replay stops. This shows the exact location of non-deterministic
  regressions.

### 10.D5 Session event log

- **Status:** Accepted
- **Context:** §5 identified the server-side events as in-process `tokio::broadcast` topics. §6 did
  not give a mechanism for persistent session recording or frame history. Without an append-only
  frame log, headless verification and offline debugging were not possible, unless we put write load
  on the primary relational database.
- **Decision:** **NATS JetStream**, one stream `NF_SESSIONS`, subjects `nightfall.session.<id>.in` /
  `.out`, retention 7 days (rejected: Postgres table, both). It already runs. It is append-only, with
  replay by sequence. It keeps the write load off the game DB. Archival to object storage is a later
  story.
- **Consequences:** Session frame recording uses NATS JetStream, under stream `NF_SESSIONS`. It
  records the `nightfall.session.<id>.in` and `nightfall.session.<id>.out` subjects with a 7-day
  retention window. Network session tasks log the inbound and outbound frames asynchronously, through
  bounded channels with drop-counter metrics. Thus, frame logs cannot block the tick loop. This
  replaces the transient in-memory broadcasts (§5, §6) with durable, sequential event logs for
  replay. It also keeps the high-frequency session writes off Postgres.

### 10.D6 Telemetry backend

- **Status:** Accepted
- **Context:** §5 and §6.1 specified observability with `tracing`, `tracing-subscriber`, `metrics`,
  and `metrics-exporter-prometheus`, shown on a `/metrics` scrape endpoint. Distributed trace
  storage, centralized log search, and correlated tick diagnostics were not part of the local
  development environment.
- **Decision:** **Grafana LGTM** all-in-one in Compose, with OpenTelemetry OTLP from the server
  (rejected: Datadog, Honeycomb). It is free, local, and OTel-native. Later, if we change one
  endpoint, we can forward to a SaaS.
- **Consequences:** The server telemetry stack uses OpenTelemetry (`tracing-opentelemetry` with an
  OTLP exporter). Its target is a self-hosted Grafana LGTM container (Loki, Grafana, Tempo, Mimir) in
  Docker Compose. The structured JSON logs that the server emits include the `request_id`,
  `session_id`, and `tick` fields, and link directly to distributed traces. In §6.1, unified OTLP
  export replaces standalone Prometheus scraping. The dashboard JSON definitions are kept in
  `infra/grafana/`.

### 10.D7 Determinism

- **Status:** Accepted
- **Context:** §3.2 and §4.2 specified entity positions, movement interpolation, and delta messages
  with floating-point numbers (`Position` with `float x, float y` and `float speed`). §4.1 had the
  same, with `REAL` columns in the `characters` table. §3.2 set a 100 ms (10 Hz) simulation clock.
  But floating-point math on different hardware architectures and compiler targets makes
  deterministic simulation replay unreliable.
- **Decision:** These rules apply (rejected: f32 positions):
  - Fixed 100 ms tick.
  - Positions as `i32` fixed-point (1/1000 tile).
  - `ChaCha12` RNG seeded for each zone for each session.
  - Each command is stamped with the tick on which it applies.

  Without this, "Perfect replay" is impossible. Floats drift across builds and CPUs.
- **Consequences:** We refactor all spatial coordinates, speeds, and movement calculations in
  `domain/zone` to integer fixed-point math (`Fixed(i32)`, which is 1/1000th of a tile). In §4.1 and
  §4.2, fixed-point integers replace the floating-point representations (`REAL` in SQL, `float` in
  proto `Position`). Float arithmetic is prohibited in simulation logic. CI enforces this with the
  `clippy::float_arithmetic` deny. `ChaCha12`, seeded deterministically from
  `(zone_id, session_epoch)`, controls the zone simulation randomness. All incoming client commands
  are stamped with their applied `Tick(u64)`. These rules make sure that replay is exactly
  reproducible.

