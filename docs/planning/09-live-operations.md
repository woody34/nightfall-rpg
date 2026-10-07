# Phase 9: Live Operations

Game master tooling, anti-cheat and bot detection, patching and deployment, metrics and
telemetry, and the event scheduler. Defines the `ops` module layout in `apps/api`, the GM gRPC
service, the metric catalogue, and the moon-based CI/CD pipeline.

---

## 1. Purpose and scope

**Delivers**

- A GM command surface (`AdminService` gRPC) with role-based access, confirmation for
  destructive commands, and an append-only audit log; a minimal web admin panel served by axum.
- Server-authoritative anti-cheat: per-tick movement validation, per-packet-type rate limits,
  bot heuristics (timing regularity, uptime, path repetition), a player bot-report system with
  L2-style thresholds, captcha-on-suspicion, and tiered responses (flag, jail, ban).
- Release engineering: client/server version handshake in `Ping`, protobuf compatibility rules
  enforced by `buf breaking`, `sqlx` migrations, graceful shutdown with character save, Docker
  images via `cargo-chef`, GitHub Actions running `moon ci`.
- Telemetry: `tracing` + OpenTelemetry + Prometheus, a defined metric list, Grafana dashboards
  provisioned from files, JSON logs with correlation ids, Sentry crash reporting.
- A data-driven event scheduler (cron triggers; actions: spawn NPCs, rate multipliers, enable
  drops, announce) modelled on L2J's `LongTimeEvent`.

**Excludes**

- Billing, account recovery, and customer support ticketing (external tools).
- Content of specific events (Phase 6 content lists); this phase delivers the engine.
- Kernel/host hardening and DDoS mitigation beyond application rate limiting.

---

## 2. Reference: how Lineage 2 does it

### 2.1 L2J admin commands and access levels

Commands are typed in chat as `//command args` (internally `admin_command`). `adminCommands.xml`
is a `<list>` of `<admin command="admin_spawn" accessLevel="100" [confirmDlg="true"]/>` entries
(600+ in L2J Mobius). `accessLevels.xml` defines the roles:

| Level | Name | Notes |
|---|---|---|
| -1 | Banned | |
| 0 | User | `allowTransaction=true`, `giveDamage=true`, `takeAggro=true`, `gainExp=true` |
| 10 | Chat Moderator | |
| 20 | Test GM | |
| 30 | General GM | |
| 40 | Support GM | |
| 50 | Event GM | |
| 60 | Head GM | |
| 70 | Admin | |
| 100 | Master | `isGM`, `allowPeaceAttack`, `allowFixedRes`, `allowAltg`, all flags true |

Each level carries `nameColor`, `titleColor`, `childAccess` (which levels it may administer),
`isGM`, `allowPeaceAttack`, `allowFixedRes`, `allowTransaction`, `allowAltg`, `giveDamage`,
`takeAggro`, `gainExp`. Note the shipped `adminCommands.xml` grants every command at level 100;
operators are expected to lower individual commands for their GM tiers. Representative commands:

| Area | Commands |
|---|---|
| Spawning | `//spawn npc_id [count] [respawn_delay]`, `//unspawnall`, `//respawnall`, `//delete` (target) |
| Movement | `//teleportto player`, `//recall player`, `//move_to x y z`, `//sendhome`, `//instant_move`, `//gmspeed 1-5` |
| Items | `//create_item id [count]`, `//create_coin`, `//enchant`, `//give_all` |
| Punishment | `//kick`, `//kick_non_gm`, `//ban_char`, `//ban_acc`, `//ban_chat`, `//jail player [minutes]`, `//unjail`, `//punishment` |
| Server | `//server_shutdown [seconds]`, `//server_restart [seconds]`, `//server_abort`, `//serverinfo`, `//setconfig key value` |
| Data | `//reload config|npc|quests|skills|items|multisell|buylist|teleport|htm|zone|...` |
| Effects/status | `//effects` (list target's effects), `//heal`, `//res`, `//invul`, `//invisible`, `//polymorph`, `//setkarma`, `//setfame` |
| Comms | `//announce text`, `//reload_announcements`, `//gmchat`, `//gmliston/off` |

`General.properties` adds `GMStartupInvulnerable=True`, `GMStartupInvisible=False`,
`GMStartupAutoList=False`, and jail behaviour (`JailIsPvp=False`, `JailDisableChat=True`,
`JailDisableTransaction=False`).

### 2.2 L2J flood protector (rate limiting)

`FloodProtector.properties` configures per-action minimum intervals in 100 ms game ticks, each
with `LogFlooding`, `PunishmentLimit`, `PunishmentType` (none/kick/ban/jail), `PunishmentTime`:

| Protector | Interval (ticks) | Protector | Interval |
|---|---|---|---|
| UseItem | 0 | DropItem | 10 |
| RollDice / Firework | 42 | ServerBypass | 5 |
| ItemPetSummon | 16 | MultiSell | 1 |
| HeroVoice / SendMail | 100 | Transaction | 10 |
| GlobalChat | 5 | Manufacture | 3 |
| Subclass | 20 | Manor | 30 |
| CharacterSelect | 30 | ItemAuction | 9 |

Punishment is off by default; operators enable it per protector.

### 2.3 L2J movement validation

See Phase 8 §2.1: the server rejects a `MoveBackwardToLocation` beyond 9,900 units
(`dx²+dy² > 98,010,000`) or while `isOutOfControl()`, simulates movement itself at the
character's run speed (capped by `MaxRunSpeed=300`), and on `ValidatePosition` resyncs the client
with `ValidateLocation` when the squared deviation exceeds 250,000 (500 units) or |dZ| > 200.
`CoordSynchronize=2` is the strict mode. A missing `moveMovement` field (BufferUnderflow) is
treated as an L2Walker signature.

### 2.4 L2J bot report ("Report bot" button)

`BotReportTable` implements the retail High Five system:

- Each player has 7 report points per day, reset at `BotReportPointsResetHour` (default 00:00).
- `BotReportDelay` (default 30 min) between reports by the same player; the same IP also
  cannot report twice inside the delay. `AllowReportsFromSameClanMembers=False`.
- Rejected when: target in a peace zone or battleground, in Olympiad, at clan war with the
  reporter, has gained no XP since login, reporter is themselves flagged, or already reported
  this target recently.
- Thresholds in `botreport_punishments.xml` apply debuff skills with a system message:

| Reports | Skill | SysMsg |
|---|---|---|
| 25 | 6038 (Report Status: XP/SP -) | 2473 |
| 75 | 6039 | 2474 |
| 100 | 6055 | 2477 |
| 125 | 6056 | 2478 |
| 150 / 175 | 6057 | 2480 |
| 150+ (range) | 6040 (harsher) | - |

Captcha challenges were a popular private-server addition layered on top of this (the suspect
must type an image code in N seconds or is jailed); retail relied on debuffs plus GM review.

### 2.5 L2J events

`LongTimeEvent` (extended by `L2Day`, `Christmas`, `Freya`, etc.) loads
`data/scripts/events/<Name>/config.xml`:

```xml
<event name="L2 Day" active="23 03 2006-28 03 2006" dropPeriod="23 03 2006-28 03 2006">
  <droplist>
    <add item="3875" min="1" max="1" chance="1%"/>  <!-- letters A..T, 3875-3887; 3888 = "II" -->
  </droplist>
  <spawnlist>
    <add npc="31854" x="-84128" y="243258" z="-3735" heading="0"/>  <!-- Talking Island -->
    <add npc="31855" x="45510"  y="48364"  z="-3065"/>              <!-- Elven Village -->
  </spawnlist>
  <messages>
    <add type="onEnter" text="L2 Day: collect letters to form NCSOFT, CHRONICLE, LINEAGE II"/>
    <add type="onEnd"   text="L2 Day: Event end!"/>
  </messages>
</event>
```

The constructor schedules `startEvent()` on the thread pool (immediately if inside the period,
delayed otherwise, skipped if expired). `startEvent` registers the droplist with
`EventDroplist.addGlobalDrop`, spawns NPCs with a despawn time equal to the event end, and
posts an `EventAnnouncement`; a `ScheduleEnd` runnable announces the end and optionally destroys
event items. Retail seasonal events: L2 Day (anniversary letter collection), Freya's Celebration
(winter buffs), Christmas (Santa Trainee / Saving Santa), Master of Enchanting, Golden Pig,
Halloween. All reduce to the same primitives: spawn, drop, rate multiplier, announce, exchange.

### 2.6 Bot detection literature

- Chen et al. (Ragnarok Online traffic) found bots distinguishable by **regularity of command
  release times**, **traffic burstiness across time scales**, and **insensitivity to network
  conditions**; ensemble classifiers reached 90% accuracy with zero false positives
  (conservative) or 95% with <1% false negatives (progressive).
- Thawonmas (DMIN 2010, Cabal Online logs) fed **action frequencies, types, and time intervals**
  to an SVM to separate bots from humans.
- Systematic reviews (IEEE Access) agree the strongest server-side signals are timing variance,
  session length, movement path repetition, and social-graph isolation.

---

## 3. Design decisions for Nightfall

### 3.1 GM tooling

- **Roles** (stored on the account, carried in the session JWT as `role` claim) mirror L2J
  levels so the mental model transfers: `USER=0, CHAT_MOD=10, EVENT_GM=50, GM=60, ADMIN=70,
  MASTER=100`. Each `AdminService` RPC declares a minimum level; `childAccess` semantics: a GM
  can only punish accounts with a lower level.
- **Commands are RPCs, not chat parsing.** The client's `//cmd` chat syntax is a thin
  translator in the HUD that calls the same RPCs the web panel calls. One code path, one audit.
- **Confirmation** for destructive ops (`ban`, `delete`, `shutdown`, `wipe_inventory`): the
  RPC must carry `confirm_token` obtained from a prior `Prepare` call (10 s validity) — the
  server-side equivalent of L2J's `confirmDlg="true"`.
- **Audit log** is append-only Postgres table `gm_audit` written inside the same transaction as
  the effect; every row has `actor_account_id, actor_role, command, args_json, target_ids,
  result, request_id, ip, at`. Audit writes are also emitted as `tracing` events at `INFO` with
  `audit=true` so they land in logs and can alert.
- **Web admin panel**: axum serves `/admin` (static Preact bundle built in `apps/admin`, or a
  single HTML page in v1) and `/admin/api/*` JSON routes that wrap the same service layer. Auth
  via the same JWT (role ≥ GM) in an `Authorization: Bearer` header; CSRF not needed for bearer
  tokens; CORS locked to the admin origin.
- **GM privileges in-world**: `invul`, `invisible`, `gmspeed`, `peace_attack` are per-session
  toggles that default on login to the L2J defaults (invulnerable on, invisible off, auto-list
  off). GM characters are excluded from leaderboards, XP metrics, and bot heuristics.

### 3.2 Anti-cheat

Server authority (Phase 0) already removes the classic cheats: damage, loot, inventory, and
cooldowns are computed server-side. What remains is input shaping and automation.

**Movement validation per tick (100 ms):**

```
allowed = run_speed_px_s * dt_s * 1.10        // 10% tolerance for tick jitter
if dist(prev_pos, new_pos) > allowed            → clamp to allowed, violation("speed", delta)
if !walkable(new_pos) && !gm                    → reject move, violation("clip")
if MoveTo.dest beyond 1,200 px from pos         → reject (mirrors L2J's 9,900-unit sanity cap)
if intent arrives while stunned/dead/casting-locked → ActionFailed, no violation (normal race)
```

Because movement is simulated server-side (client only sends destinations), speed hacks can
only manifest as *more frequent* destinations, which rate limiting handles, or as tampering
with the client's displayed position, which is cosmetic. The validation above matters for any
future client-reported positions (vehicles, knockbacks).

**Rate limits per intent type** (token bucket per session; L2J intervals converted):

| Intent | Sustained | Burst | On exceed |
|---|---|---|---|
| MoveTo | 10/s | 20 | drop silently, count |
| Attack / UseSkill | 8/s | 12 | drop, count |
| UseItem | 5/s | 10 | drop, count |
| Chat ALL/TRADE | 1 per 0.5 s | 3 | `SystemMessage(CHAT_FLOOD)` |
| Chat SHOUT | 1 per 10 s | 1 | message |
| Trade / Shop transaction | 1/s | 2 | drop |
| Mail | 1 per 10 s | 1 | drop |
| Any RPC (connection-level, `tower-governor`) | 50/s | 100 | HTTP 429 / `RESOURCE_EXHAUSTED` |
| Subscribe reconnects | 1 per 2 s | 3 | close |

Exceeding a bucket 20 times in a minute raises a `violation("flood", kind)`.

**Bot heuristics** (computed by an `anticheat::scorer` task every 5 min per online session
from an in-memory per-session action log of the last 500 actions):

| Signal | Computation | Points |
|---|---|---|
| Timing regularity | Coefficient of variation of inter-action intervals for the dominant action (attack/skill) over ≥200 samples; humans ≈ 0.3–0.6, bots < 0.08 | CV<0.08: +30, <0.15: +15 |
| Reaction time floor | Median delay between `EntitySpawn` of an attackable in range and first `Attack`; humans ≥ 250 ms | <150 ms median: +25 |
| Session length | Continuous activity (no gap >5 min) | >12 h: +15, >20 h: +30 |
| Path repetition | Hash of quantized (32 px) movement loops; same loop >50 times in an hour | +20 |
| Social isolation | No chat sent, no party, no trade in 6 h of activity | +10 |
| Camera/UI silence | Zero `SetTarget` by click (all by Tab) and zero window opens for 2 h | +10 |
| Prior flags | Each previous flag in 30 days | +10 |

Score ≥ 50 → **flag** (log, GM dashboard, enable fine-grained logging). Score ≥ 70 →
**captcha challenge**. Failed captcha or score ≥ 90 → **jail** 60 min + GM review queue.
Repeat jail within 7 days → **account ban** (temporary 7 d, then permanent on third). Every
tier is a GM-reviewable record; auto-bans require two independent signals (e.g. CV and
reaction floor) to limit false positives, per the literature's "conservative" scheme.

**Captcha-on-suspicion:** server generates a 5-character image (server-side rasterised with
`imageproc`/`ab_glyph`, noise lines) sent as `CaptchaChallenge{png_bytes, deadline_ms=60000}`
on the world stream. The HUD renders a modal; `AnswerCaptcha{text}` must arrive before the
deadline; three failures → jail. Rate: at most one captcha per hour per player regardless of
score, never during combat with players, never in Olympiad/siege.

**Player bot report**: port of L2J: 7 points/day reset 00:00 server time, 30-min delay per
reporter, same-IP delay, no same-clan reports, rejections identical to L2J. Reports add +5 to the
heuristic score (capped +25) rather than applying debuffs directly, so reports cannot be
weaponised without corroborating behaviour; 25+ reports always surface the player in the GM
queue.

### 3.3 Patching, versioning, maintenance

- **Version handshake** uses the existing `Ping(client_version)`. Server config holds
  `min_client_version` and `latest_client_version`. Response gains `compat` =
  `OK | UPDATE_AVAILABLE | UPDATE_REQUIRED`; the client shows a banner or blocks login. The
  client version is Vite's `__APP_VERSION__` from `package.json` + git short SHA. Server
  version is `CARGO_PKG_VERSION` (already returned).
- **Protobuf compatibility**: never change or reuse field numbers; remove fields by `reserved`
  number and name; add enum values only; `oneof` additions are safe, moves into an existing
  `oneof` are not; integer widenings are wire-compatible but avoid them. Enforced in CI by
  `buf breaking --against '.git#branch=main'` with the `WIRE_JSON` category (we also serialise
  JSON in the admin API). Services are versioned by package (`nightfall.v1`); a `v2` package
  is the only way to break.
- **Database migrations**: `sqlx::migrate!()` with `apps/api/migrations/<timestamp>_<name>.sql`
  (reversible pairs via `sqlx migrate add -r`), tracked in `_sqlx_migrations`. Chosen over
  refinery because the server already uses `sqlx` for queries (Phase 0), compile-time query
  checking shares the same offline data, and one tool is fewer. Rule: migrations are
  expand/contract — a deploy N adds columns/tables; deploy N+1 removes the old ones, so server
  N-1 and N can run against the same schema during a rollout.
- **Deploy model**: a stateful world server cannot be blue/green'd transparently (players are
  in memory). We use a **short maintenance window with graceful drain**, L2-style:
  `//server_shutdown 300` → countdown announcements at 300/180/60/30/10 s via `SystemMessage`
  → logins disabled at T-60 → at T: stop accepting intents, persist every online character and
  inventory in one transaction per character (parallel, bounded by a semaphore of 32), close
  streams with `Status::unavailable("maintenance")`, exit 0. The client auto-reconnects with
  backoff (Phase 8) and the new binary comes up behind the same address. True blue/green is
  used for the **stateless edge** (axum HTTP, admin panel, auth) which can run two versions
  behind the reverse proxy. If a second world shard ever exists, players are drained shard by
  shard.
- **Graceful shutdown**: `tokio::signal::ctrl_c()` + `SIGTERM` via `signal(SignalKind::terminate())`
  into a `CancellationToken`; axum `with_graceful_shutdown(token.cancelled())`, tonic
  `serve_with_shutdown(addr, svc, token.cancelled())`; the game loop observes the token, runs
  `persist_all()`, then drops the exporter guard (flush). Docker `stop_grace_period: 90s`.
- **Docker**: `cargo-chef` multi-stage (planner → cook deps → build → `debian:trixie-slim`
  runtime), one image `nightfall-api`, tagged with git SHA and semver. Client is static:
  `vite build` → `dist/` → an nginx or Caddy image, or object storage + CDN.
- **CI**: GitHub Actions runs `moon ci` which selects affected targets by comparing the PR head
  against base; `fetch-depth: 0` is required for the diff. Dev servers (`dev`, `preview`) are
  marked `runInCI: false`.

### 3.4 Metrics and telemetry

- **Library choice**: `tracing` for spans/logs (already in `main.rs`); `metrics` facade with
  `metrics-exporter-prometheus` for a `/metrics` endpoint on the axum port (simple, no
  collector needed, Grafana scrapes directly); `tracing-opentelemetry` + `opentelemetry-otlp`
  for traces to an OTel collector/Tempo/Jaeger, enabled by `OTEL_EXPORTER_OTLP_ENDPOINT`. We
  deliberately keep metrics on the `metrics` crate rather than OTel metrics: Prometheus
  exposition is the common denominator and `counter!/gauge!/histogram!` are cheap in the tick
  loop.
- **Naming** follows Prometheus conventions: `nightfall_` prefix, base units (`_seconds`,
  `_bytes`), `_total` for counters, low-cardinality labels only (zone ids, class ids, intent
  kinds — never character or account ids).
- **Histogram buckets** for tick duration: `[0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2,
  0.5, 1.0]` seconds (the tick budget is 0.1 s); DB latency: `[0.0005, 0.001, 0.0025, 0.005,
  0.01, 0.025, 0.05, 0.1, 0.25, 1.0]`.
- **Logs**: JSON (`tracing_subscriber::fmt().json()`) in production, pretty in dev
  (`LOG_FORMAT=pretty`). Every HTTP/gRPC request gets `request_id` (tower-http
  `SetRequestIdLayer` + `PropagateRequestIdLayer`, UUID v7) placed on the root span via
  `make_span_with`; game-loop spans carry `session_id`, `character_id`, `zone_id`. Audit and
  anticheat events carry `audit=true` / `anticheat=true` fields for log routing.
- **Crash reporting**: `sentry` + `sentry-tracing` layer: panics captured by default, `ERROR`
  events become Sentry events, `INFO+` become breadcrumbs. `sentry::init` must run before the
  Tokio runtime starts, so `main` becomes a plain `fn main` that builds the runtime after
  `init`. Release = `CARGO_PKG_VERSION+sha`. Client: `@sentry/browser` with the same release
  string so client/server errors correlate.
- **Dashboards** provisioned from `ops/grafana/dashboards/*.json` via a file provider:
  *Overview* (CCU, tick p50/p99, RPC error rate, DB p99), *Economy* (adena in circulation,
  created/destroyed per hour, item sinks), *World* (XP/h per zone, deaths per zone, monster
  kills), *Players* (class distribution, level histogram, session length), *Security* (flags,
  jails, bans, flood drops, captcha pass rate).

### 3.5 Events

Data-driven definitions in `apps/api/events/*.toml`, hot-reloadable with `//reload events`:

```toml
[event]
id = "winter_festival_2026"
name_msg_id = 7001            # SystemMessage id for announcements (localised client-side)
active = { start = "2026-12-15T04:00:00Z", end = "2027-01-05T04:00:00Z" }

[[trigger]]                   # optional sub-schedules inside the active window (6-field cron, UTC)
cron = "0 0 */2 * * *"        # every 2 hours: spawn the raid
action = "spawn"
npc_id = 90012
positions = [{ zone = 3, x = 1024, y = 2200 }]
despawn_after_s = 3600

[[action]]                    # applied at start, reverted at end
kind = "rate_multiplier"
xp = 1.5
sp = 1.5
drop = 1.25

[[action]]
kind = "enable_drops"
drops = [{ item_id = 3875, min = 1, max = 1, chance_bp = 100, zones = "all" }]  # 1% as basis points

[[action]]
kind = "spawn"
npc_id = 31854
positions = [{ zone = 1, x = 400, y = 380 }, { zone = 2, x = 900, y = 120 }]

[[action]]
kind = "announce"
on = "enter"                  # enter | end | login
msg_id = 7002
```

The `EventScheduler` uses `tokio-cron-scheduler` (6-field, seconds-first expressions; `0 */5 *
* * *` = every five minutes) for triggers and a `start`/`end` one-shot per event. Actions are
reversible: each `apply()` returns an `Undo` that `end()` runs in reverse order. Rate multipliers
compose multiplicatively with the global `Rates` config. Event state (which events are active,
what was spawned) is persisted in `event_state` so a restart mid-event re-applies without
double-spawning.

---

## 4. Data model

Postgres tables (migrations in `apps/api/migrations/`):

```sql
CREATE TABLE gm_audit (
  id            BIGSERIAL PRIMARY KEY,
  at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  request_id    UUID NOT NULL,
  actor_account BIGINT NOT NULL,
  actor_role    SMALLINT NOT NULL,
  actor_ip      INET,
  command       TEXT NOT NULL,                  -- "spawn", "give_item", "ban_account" ...
  args          JSONB NOT NULL,
  targets       BIGINT[] NOT NULL DEFAULT '{}',
  result        TEXT NOT NULL,                  -- "ok" | error code
  duration_ms   INT
);
CREATE INDEX gm_audit_actor_at ON gm_audit (actor_account, at DESC);
CREATE INDEX gm_audit_targets ON gm_audit USING GIN (targets);

CREATE TABLE punishments (
  id            BIGSERIAL PRIMARY KEY,
  account_id    BIGINT NOT NULL,
  character_id  BIGINT,
  kind          SMALLINT NOT NULL,               -- 1 chat_ban, 2 jail, 3 account_ban, 4 ip_ban
  reason        TEXT NOT NULL,
  issued_by     BIGINT,                          -- NULL = automatic
  issued_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at    TIMESTAMPTZ,                     -- NULL = permanent
  lifted_at     TIMESTAMPTZ, lifted_by BIGINT,
  evidence      JSONB                            -- heuristic scores, violation list, report count
);

CREATE TABLE anticheat_flags (
  id BIGSERIAL PRIMARY KEY, character_id BIGINT NOT NULL, at TIMESTAMPTZ NOT NULL DEFAULT now(),
  score SMALLINT NOT NULL, signals JSONB NOT NULL, tier SMALLINT NOT NULL,   -- 1 flag 2 captcha 3 jail 4 ban
  reviewed_by BIGINT, verdict SMALLINT                                        -- NULL pending, 1 bot, 2 clean
);

CREATE TABLE bot_reports (
  reporter_character BIGINT NOT NULL, reported_character BIGINT NOT NULL,
  reporter_ip INET, at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (reporter_character, reported_character, at)
);

CREATE TABLE event_state (
  event_id TEXT PRIMARY KEY, active BOOLEAN NOT NULL, started_at TIMESTAMPTZ,
  spawned_entity_ids BIGINT[] NOT NULL DEFAULT '{}', undo JSONB NOT NULL DEFAULT '[]'
);

CREATE TABLE server_meta (key TEXT PRIMARY KEY, value JSONB NOT NULL);  -- min_client_version, maintenance flag
```

Economy counters that feed metrics are not tables: `adena_in_circulation` is a gauge
recomputed every 60 s by `SELECT sum(count) FROM items WHERE item_id = ADENA`; item
create/destroy counters increment inline in the inventory service.

### 4.1 Proto additions

```proto
// packages/proto/nightfall/v1/admin.proto
service AdminService {
  rpc Prepare(PrepareRequest) returns (PrepareResponse);          // confirm_token for destructive ops
  rpc Spawn(SpawnRequest) returns (SpawnResponse);                // EVENT_GM+
  rpc Despawn(DespawnRequest) returns (Ack);                      // EVENT_GM+
  rpc Teleport(TeleportRequest) returns (Ack);                    // GM+   (self, player-to-self, self-to-player, to coords)
  rpc GiveItem(GiveItemRequest) returns (Ack);                    // ADMIN+
  rpc SetStat(SetStatRequest) returns (Ack);                      // ADMIN+ (level, xp, karma, fame)
  rpc Kick(KickRequest) returns (Ack);                            // GM+
  rpc Punish(PunishRequest) returns (PunishResponse);             // chat_ban: CHAT_MOD+, jail: GM+, account_ban: ADMIN+ (needs confirm_token)
  rpc LiftPunishment(LiftRequest) returns (Ack);                  // same tiers
  rpc Announce(AnnounceRequest) returns (Ack);                    // EVENT_GM+ (msg_id or raw text flagged as GM text)
  rpc Reload(ReloadRequest) returns (ReloadResponse);             // ADMIN+  (config|npcs|items|skills|drops|events|zones)
  rpc Effects(EffectsRequest) returns (EffectsResponse);          // GM+   list/add/remove effects on target
  rpc Toggle(ToggleRequest) returns (Ack);                        // GM+   invul|invisible|gmspeed|peace_attack
  rpc ServerInfo(ServerInfoRequest) returns (ServerInfoResponse); // GM+
  rpc Shutdown(ShutdownRequest) returns (Ack);                    // MASTER (needs confirm_token); seconds, restart bool, abort bool
  rpc QueryAudit(QueryAuditRequest) returns (QueryAuditResponse); // ADMIN+
  rpc ReviewFlag(ReviewFlagRequest) returns (Ack);                // GM+   verdict on anticheat_flags
  rpc TriggerEvent(TriggerEventRequest) returns (Ack);            // EVENT_GM+ start/stop by event_id
}

enum GmRole { GM_ROLE_USER = 0; GM_ROLE_CHAT_MOD = 10; GM_ROLE_EVENT_GM = 50; GM_ROLE_GM = 60; GM_ROLE_ADMIN = 70; GM_ROLE_MASTER = 100; }

message PrepareRequest  { string command = 1; bytes args_hash = 2; }
message PrepareResponse { string confirm_token = 1; int64 expires_at_ms = 2; string summary = 3; }
message SpawnRequest    { uint32 npc_id = 1; uint32 count = 2; uint32 respawn_delay_s = 3; uint32 zone_id = 4; Position pos = 5; }
message SpawnResponse   { repeated uint64 entity_ids = 1; }
message TeleportRequest { oneof mode { uint64 self_to_player = 1; uint64 player_to_self = 2; Target to_coords = 3; } }
message Target          { uint32 zone_id = 1; Position pos = 2; }
message GiveItemRequest { uint64 character_id = 1; uint32 item_id = 2; uint64 count = 3; uint32 enchant = 4; }
message PunishRequest   { uint64 account_id = 1; uint64 character_id = 2; PunishKind kind = 3; uint32 minutes = 4; string reason = 5; string confirm_token = 6; }
enum PunishKind { PUNISH_UNSPECIFIED = 0; CHAT_BAN = 1; JAIL = 2; ACCOUNT_BAN = 3; IP_BAN = 4; }
message AnnounceRequest { uint32 msg_id = 1; repeated MessageParam params = 2; string gm_text = 3; }
message ShutdownRequest { uint32 seconds = 1; bool restart = 2; bool abort = 3; string confirm_token = 4; }
message ServerInfoResponse { uint32 ccu = 1; uint64 uptime_s = 2; string version = 3; double tick_p99_ms = 4; uint32 zones_loaded = 5; repeated string active_events = 6; }

// Additions to game.proto
message PingResponse {
  string server_version = 1;
  int64 server_time_ms = 2;
  VersionCompat compat = 3;          // new
  string latest_client_version = 4;  // new
  bool maintenance = 5;              // new: logins disabled
}
enum VersionCompat { VERSION_COMPAT_UNSPECIFIED = 0; OK = 1; UPDATE_AVAILABLE = 2; UPDATE_REQUIRED = 3; }

// Additions to world.proto (WorldEvent oneof)
message CaptchaChallenge { bytes png = 1; uint32 deadline_ms = 2; string challenge_id = 3; }
message ShutdownNotice   { uint32 seconds_remaining = 1; bool restart = 2; }
message EventStateChanged { string event_id = 1; bool active = 2; uint32 msg_id = 3; }
// New unary intents on WorldService
// rpc AnswerCaptcha(AnswerCaptchaRequest) returns (Ack);
// rpc ReportBot(ReportBotRequest) returns (ReportBotResponse);
```

---

## 5. Interfaces

### 5.1 Auth interceptor

A tonic interceptor validates the JWT (`jsonwebtoken`, HS256 for v1, key from env), extracts
`account_id`, `role`, `session_id` into request extensions, and rejects `AdminService` calls
whose role is below the method's minimum (a `static` table `METHOD_MIN_ROLE: &[(&str, GmRole)]`
keyed by `grpc path`). The same JWT is accepted by the axum admin routes via an extractor.

### 5.2 Admin HTTP routes (axum, port 3000)

| Route | Purpose |
|---|---|
| `GET /health` | exists |
| `GET /metrics` | Prometheus exposition (internal network only; firewall or basic auth) |
| `GET /admin/` | static panel |
| `GET /admin/api/server` | `ServerInfoResponse` as JSON |
| `GET /admin/api/players?online=1&q=` | online list with zone, level, score |
| `POST /admin/api/announce`, `/punish`, `/spawn`, `/teleport`, `/reload`, `/shutdown` | wrap `AdminService` |
| `GET /admin/api/audit?actor=&target=&since=` | audit query |
| `GET /admin/api/flags?pending=1`, `POST /admin/api/flags/:id/review` | anticheat queue |
| `GET /admin/api/events`, `POST /admin/api/events/:id/start|stop` | event control |

### 5.3 Server events to the client

`ShutdownNotice`, `CaptchaChallenge`, `EventStateChanged`, plus `SystemMessage`s for all
punishments (jail entry/exit, chat ban, flood warnings) so the client needs no new text.

### 5.4 Observability endpoints and signals

- `/metrics` scraped every 15 s by Prometheus.
- OTLP traces exported over gRPC to `OTEL_EXPORTER_OTLP_ENDPOINT` (sampling: 100% of
  admin/auth RPCs, 1% of world intents, 100% of errors via tail sampling in the collector).
- Logs to stdout as JSON; container runtime ships them (Loki/Vector).
- Sentry DSN from `SENTRY_DSN`.

---

## 6. Rust implementation notes

### 6.1 Module layout (`apps/api/src`)

```
src/
  main.rs                 # sentry::init -> runtime -> telemetry::init -> servers -> shutdown
  http.rs grpc.rs         # existing; grpc adds auth interceptor, GrpcWebLayer, AdminService
  ops/
    mod.rs
    telemetry.rs          # tracing registry: fmt(json|pretty) + EnvFilter + OpenTelemetryLayer + sentry layer
    metrics.rs            # PrometheusBuilder::install_recorder(), bucket config, metric name consts, describe_*()
    request_id.rs         # tower-http SetRequestId/PropagateRequestId, make_span_with
    shutdown.rs           # CancellationToken, SIGTERM/SIGINT, countdown announcer, persist_all()
    version.rs            # semver compare for Ping compat
    admin/
      mod.rs service.rs   # AdminService impl: role check -> handler -> audit (same txn)
      roles.rs            # GmRole, METHOD_MIN_ROLE, child_access()
      audit.rs            # AuditWriter (sqlx), tracing event emission
      confirm.rs          # Prepare tokens (HMAC(command, args_hash, exp))
      http.rs             # axum /admin/api routes + static files
    anticheat/
      mod.rs
      movement.rs         # per-tick speed/walkable validation, violation() sink
      ratelimit.rs        # per-session token buckets per IntentKind; tower-governor for connection level
      heuristics.rs       # ActionLog ring (500), scorer: cv, reaction floor, uptime, path hash, isolation
      captcha.rs          # image gen, challenge store (DashMap<challenge_id, (answer, deadline)>)
      reports.rs          # bot report points, delays, rejections (L2J port)
      response.rs         # tiering: flag -> captcha -> jail -> ban; writes punishments/anticheat_flags
    events/
      mod.rs loader.rs    # TOML parse + validate (npc/item ids exist)
      scheduler.rs        # tokio-cron-scheduler jobs, start/end one-shots
      actions.rs          # Spawn/RateMultiplier/EnableDrops/Announce with Undo
      state.rs            # event_state persistence, idempotent re-apply on boot
```

### 6.2 Telemetry init sketch

```rust
// ops/telemetry.rs
pub fn init(cfg: &OpsConfig) -> anyhow::Result<TelemetryGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=debug".into());
    let fmt = if cfg.log_json {
        tracing_subscriber::fmt::layer().json().with_current_span(true).with_span_list(false).boxed()
    } else { tracing_subscriber::fmt::layer().pretty().boxed() };

    let otel = cfg.otlp_endpoint.as_ref().map(|ep| {
        let exporter = opentelemetry_otlp::SpanExporter::builder().with_tonic().with_endpoint(ep).build()?;
        let provider = SdkTracerProvider::builder().with_batch_exporter(exporter)
            .with_resource(Resource::builder().with_service_name("nightfall-api").build()).build();
        anyhow::Ok(tracing_opentelemetry::layer().with_tracer(provider.tracer("nightfall-api")))
    }).transpose()?;

    tracing_subscriber::registry().with(filter).with(fmt).with(otel).with(sentry_tracing::layer()).init();

    let prom = PrometheusBuilder::new()
        .set_buckets_for_metric(Matcher::Full("nightfall_tick_duration_seconds".into()),
            &[0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0])?
        .set_buckets_for_metric(Matcher::Suffix("db_query_duration_seconds".into()),
            &[0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 1.0])?
        .install_recorder()?;                    // returns PrometheusHandle; axum GET /metrics -> handle.render()
    Ok(TelemetryGuard { prom })
}
```

`main.rs` shape: `fn main() { let _sentry = sentry::init((dsn, ClientOptions { release:
sentry::release_name!(), ..})); tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(run()) }`.

### 6.3 Tick instrumentation

```rust
let start = Instant::now();
world.tick(dt);
histogram!("nightfall_tick_duration_seconds").record(start.elapsed().as_secs_f64());
gauge!("nightfall_ccu").set(sessions.len() as f64);
if start.elapsed() > Duration::from_millis(100) { counter!("nightfall_tick_overrun_total").increment(1); }
```

Game-loop spans are created per tick (`info_span!("tick", tick = n)`) but **not** per entity;
per-entity work records metrics only, to keep overhead under 1% of the tick budget.

### 6.4 Graceful shutdown

```rust
let token = CancellationToken::new();
tokio::spawn(shutdown::listen(token.clone()));          // SIGINT/SIGTERM -> token.cancel()
let http = axum::serve(listener, router).with_graceful_shutdown(token.clone().cancelled_owned());
let grpc = Server::builder()./*...*/.serve_with_shutdown(grpc_addr, token.clone().cancelled_owned());
let game = game_loop::run(world.clone(), token.clone());  // observes token; on cancel: persist_all().await
tokio::try_join!(http, grpc, game)?;
```

`AdminService::Shutdown{seconds}` spawns the countdown announcer which cancels the token at T;
`abort=true` cancels the announcer. `persist_all` uses `futures::stream::iter(chars).for_each_concurrent(32, save)`
with a 60 s overall deadline, logging any failures at `ERROR` with character ids so they can be
recovered from the last periodic save (every 15 min, matching L2J's `CharacterDataStoreInterval`).

### 6.5 Concurrency notes

- Rate-limit buckets live in the per-session struct (single-writer per session task); no locks.
- The heuristics scorer snapshots `ActionLog`s via `Arc<Mutex<_>>` with `try_lock` and skips a
  session rather than blocking the game loop.
- Audit writes are awaited inside the admin RPC (not fire-and-forget) so a failed audit fails
  the command.
- Event actions mutate world state through the same command channel the game loop drains, so
  they apply at tick boundaries.

### 6.6 Metric catalogue

All names carry the `nightfall_` prefix. Labels are low-cardinality only.

| Metric | Type | Labels | Source / notes |
|---|---|---|---|
| `ccu` | gauge | `shard` | sessions with an active world stream |
| `sessions_total` | counter | `result` (ok, rejected_version, rejected_banned, maintenance) | login attempts |
| `tick_duration_seconds` | histogram | - | game loop, buckets 1 ms .. 1 s; alert p99 > 80 ms |
| `tick_overrun_total` | counter | - | ticks > 100 ms |
| `entities_active` | gauge | `zone_id`, `kind` | per tick |
| `intents_total` | counter | `kind`, `result` (ok, rejected, rate_limited) | every WorldService unary |
| `rpc_duration_seconds` | histogram | `service`, `method`, `code` | tonic layer |
| `stream_backpressure_disconnects_total` | counter | - | channel full |
| `db_query_duration_seconds` | histogram | `query` (static name) | sqlx wrapper |
| `db_pool_connections` | gauge | `state` (idle, busy) | sqlx pool |
| `db_errors_total` | counter | `query` | |
| `character_saves_total` | counter | `trigger` (periodic, logout, shutdown), `result` | |
| `adena_in_circulation` | gauge | - | SUM over inventories every 60 s |
| `adena_flow_total` | counter | `direction` (created, destroyed), `source` (drop, quest, vendor_sell, vendor_buy, tax, repair, enchant, trade_fee) | economy sinks/faucets |
| `items_total` | counter | `direction` (created, destroyed), `source`, `grade` | never per item id |
| `xp_awarded_total` | counter | `zone_id` | XP/hour per zone = `rate(...[1h])` |
| `monster_kills_total` | counter | `zone_id`, `level_band` (10-wide) | |
| `player_deaths_total` | counter | `zone_id`, `cause` (monster, pvp, raid, fall) | deaths per zone |
| `players_by_class` | gauge | `class_id` | recomputed every 60 s from online set |
| `players_by_level_band` | gauge | `band` | |
| `session_length_seconds` | histogram | - | on logout; buckets 1m .. 24h |
| `anticheat_violations_total` | counter | `kind` (speed, clip, flood, captcha_fail) | |
| `anticheat_flags_total` | counter | `tier` (flag, captcha, jail, ban) | |
| `anticheat_score` | histogram | - | score distribution per scorer pass |
| `bot_reports_total` | counter | `result` (accepted, rejected_reason) | |
| `captcha_challenges_total` | counter | `result` (pass, fail, timeout) | |
| `gm_commands_total` | counter | `command`, `role`, `result` | mirrors audit |
| `events_active` | gauge | `event_id` | bounded by configured events |
| `event_actions_total` | counter | `event_id`, `action` | |
| `rate_multiplier` | gauge | `kind` (xp, sp, drop, adena) | effective global multiplier |
| `build_info` | gauge (=1) | `version`, `git_sha`, `rustc` | standard `_info` pattern |
| `client_fps_p5` | histogram | `zoom` | from client beacons (sampled) |
| `client_rtt_seconds` | histogram | - | from client beacons |

Grafana alert rules shipped with the dashboards: tick p99 > 80 ms for 5 min; `db_errors_total`
rate > 1/s; `ccu` drops > 50% in 5 min (crash or network); `adena_flow_total{direction="created"}`
hourly rate > 3x its 7-day median (dupe/exploit tripwire); `anticheat_flags_total{tier="ban"}`
> 10/h (false-positive tripwire).

### 6.7 CI/CD pipeline

`.github/workflows/ci.yml`:

```yaml
name: ci
on:
  push: { branches: [main] }
  pull_request:
jobs:
  ci:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0, filter: 'blob:none' }      # full history for affected detection
      - uses: moonrepo/setup-toolchain@v0
        with: { auto-install: true }
      - uses: bufbuild/buf-action@v1
        with:
          breaking_against: 'https://github.com/${{ github.repository }}.git#branch=main,subdir=packages/proto'
          input: packages/proto
          push: false
      - run: moon ci                                        # affected :lint :typecheck :test :build (+deps/dependents)
      - uses: moonrepo/run-report-action@v1
        if: success() || failure()
        with: { access-token: '${{ secrets.GITHUB_TOKEN }}' }

  image:
    needs: ci
    if: github.ref == 'refs/heads/main'
    runs-on: ubuntu-latest
    permissions: { contents: read, packages: write }
    steps:
      - uses: actions/checkout@v4
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with: { registry: ghcr.io, username: '${{ github.actor }}', password: '${{ secrets.GITHUB_TOKEN }}' }
      - uses: docker/build-push-action@v6
        with:
          file: apps/api/Dockerfile
          push: true
          tags: ghcr.io/${{ github.repository_owner }}/nightfall-api:${{ github.sha }}
          cache-from: type=gha
          cache-to: type=gha,mode=max

  deploy:
    needs: image
    if: github.ref == 'refs/heads/main'
    environment: production          # manual approval gate in GitHub Environments
    runs-on: ubuntu-latest
    steps:
      - name: announce + drain
        run: ./ops/scripts/gm.sh shutdown --seconds 300 --restart     # AdminService.Shutdown via grpcurl with a MASTER token
      - name: migrate
        run: docker run --rm -e DATABASE_URL ghcr.io/.../nightfall-api:${{ github.sha }} nightfall-api migrate
      - name: roll
        run: ssh deploy@host 'IMAGE=...:${{ github.sha }} docker compose up -d api && docker compose logs --since 1m api'
      - name: smoke
        run: curl -fsS https://api.nightfall.example/health && ./ops/scripts/grpc-ping.sh
```

Task wiring in moon: `api` project gets `lint: cargo clippy -- -D warnings`, `test: cargo test`,
`build: cargo build --release` (outputs `target/release/nightfall-api`); `client` already has
`build`; add `typecheck: tsc --noEmit` and `lint: eslint .`; `proto` project gets
`lint: buf lint` and `gen-ts: buf generate` (inputs `nightfall/**/*.proto`, outputs
`../../apps/client/src/gen`), and `client:build` depends on `proto:gen-ts`. `dev` and
`preview` tasks are `runInCI: false`. Rust CI uses `Swatinem/rust-cache@v2` in addition to
moon's own cache for `target/`.

`apps/api/Dockerfile`:

```dockerfile
FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app
RUN apt-get update && apt-get install -y protobuf-compiler && rm -rf /var/lib/apt/lists/*

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin nightfall-api

FROM debian:trixie-slim AS runtime
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/* \
 && useradd -r -u 10001 nightfall
USER nightfall
WORKDIR /app
COPY --from=builder /app/target/release/nightfall-api /usr/local/bin/nightfall-api
COPY apps/api/events /app/events
ENV HTTP_ADDR=0.0.0.0:3000 GRPC_ADDR=0.0.0.0:50051 LOG_FORMAT=json
EXPOSE 3000 50051
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/nightfall-api"]
```

The binary accepts a `migrate` subcommand (runs `sqlx::migrate!()` and exits) so migrations run
as a one-off container before the server rolls. `docker-compose.yml` for production sets
`stop_grace_period: 90s` so the drain in §3.3 completes; `ops/compose/observability.yml` runs
Prometheus (scrape `api:3000/metrics` every 15 s), Grafana with file provisioning
(`/etc/grafana/provisioning/{datasources,dashboards}` mounted from `ops/grafana/`), and an
OTel collector.

### 6.8 Crates to add

`metrics`, `metrics-exporter-prometheus`, `tracing-opentelemetry`, `opentelemetry`,
`opentelemetry_sdk`, `opentelemetry-otlp`, `sentry` (features `tracing`, `tower`),
`sentry-tracing`, `tower-governor` (feature `tonic`), `tokio-util` (`CancellationToken`),
`tokio-cron-scheduler`, `sqlx` (`postgres`, `runtime-tokio`, `migrate`, `uuid`, `chrono`),
`jsonwebtoken`, `semver`, `toml`, `serde`, `dashmap`, `imageproc` + `ab_glyph` (captcha),
`uuid` (v7), `tonic-web`.

---

## 7. Client implications

- **Version gate** in `BootScene`: on `UPDATE_REQUIRED` show a blocking dialog with the latest
  version and a reload button (the client is a web bundle, so "update" is a hard refresh with a
  cache-busted `index.html`); on `UPDATE_AVAILABLE` show a dismissible banner; on `maintenance`
  show the countdown screen and poll `Ping` every 10 s.
- **Shutdown countdown**: `ShutdownNotice` renders a top-center banner with the remaining
  seconds; the client disables intent sending at 0 and shows "Reconnecting" with backoff.
- **Captcha modal**: `CaptchaChallenge` opens a modal with the PNG, an input, and a countdown;
  input is sent via `AnswerCaptcha`. The modal cannot be dismissed; failing is handled
  server-side.
- **GM chat syntax**: HUD translates `//spawn 30001 3` into `AdminService.Spawn` for accounts
  whose session role ≥ EVENT_GM; unknown commands show a help list. The GM also gets a small
  "GM" panel (toggles, teleport-to-click, target info with entity id) only when role ≥ GM.
- **Bot report**: a "Report bot" context-menu entry on players; shows remaining points (7/day)
  and the 30-minute cooldown; results arrive as `SystemMessage`s.
- **Event banners**: `EventStateChanged` triggers a toast using the localized `msg_id`; the
  world map shows event NPC markers from `EntitySpawn.kind == NPC` with an `event_id` tag.
- **Client telemetry**: `@sentry/browser` with `release` = client version; a `performance`
  beacon every 60 s posting FPS p5, entity count, RTT to `POST /telemetry/client` (sampled
  10%), which the server turns into `nightfall_client_fps_p5` histograms.

---

## 8. Open questions

1. **Shard model.** The drain-and-restart deploy assumes one world process. If zones become
   separate processes (Phase 6 instancing), per-zone rolling restarts become possible and the
   maintenance window shrinks to seconds per zone; decide before building the orchestrator.
2. **Captcha accessibility.** Image captchas fail screen-reader users. Options: audio
   alternative, or replace captcha with a "GM whisper" check for flagged accounts. Decide with
   the accessibility baseline from Phase 8.
3. **Native histograms.** Prometheus recommends native histograms; `metrics-exporter-prometheus`
   exposes classic buckets. Revisit when the exporter supports native histograms.
4. **Where the admin panel lives.** A single HTML page served by axum is enough for v1; if it
   grows, promote to `apps/admin` (Vite + Preact) as a moon project with its own deploy.
5. **Automatic bans.** The thresholds in §3.2 are initial guesses; calibrate by running the
   scorer in shadow mode (flag only) for the first month and reviewing false-positive rates
   before enabling jail/ban tiers.
6. **Economy gauges cost.** `SUM(count)` over all items every 60 s is fine at 10k characters;
   at 100k use an incremental counter updated by the inventory service instead.
7. **Client log shipping.** Whether to ship client console logs to the server at all (privacy,
   volume) or rely solely on Sentry breadcrumbs.

---

## 9. Sources

L2J / Lineage 2 reference
- L2J `accessLevels.xml` (levels -1..100 and attributes) — https://github.com/andridgitalbox/l2j-mobius/blob/master/dist/game/config/accessLevels.xml
- L2J `adminCommands.xml` (command/accessLevel/confirmDlg entries) — https://github.com/andridgitalbox/l2j-mobius/blob/master/dist/game/config/adminCommands.xml
- L2J Mobius admin command reference (//spawn, //teleportto, //ban_char, //jail, //server_shutdown, //reload, //setconfig) — https://mintlify.wiki/fermanzolido/L2C4/admin/commands
- L2J `FloodProtector.properties` — https://github.com/andridgitalbox/l2j-mobius/blob/master/dist/game/config/FloodProtector.properties
- L2J `General.properties` (bot report settings, jail, GM startup, CharacterDataStoreInterval) — https://github.com/andridgitalbox/l2j-mobius/blob/master/dist/game/config/General.properties
- L2J `GeoData.properties` (CoordSynchronize) — https://github.com/andridgitalbox/l2j-mobius/blob/master/dist/game/config/GeoData.properties
- L2J `Character.properties` (MaxRunSpeed and caps) — https://github.com/andridgitalbox/l2j-mobius/blob/master/dist/game/config/Character.properties
- L2J `MoveBackwardToLocation.java` — https://github.com/andridgitalbox/l2j-mobius/blob/master/java/com/l2jserver/gameserver/network/clientpackets/MoveBackwardToLocation.java
- L2J `ValidatePosition.java` — https://github.com/andridgitalbox/l2j-mobius/blob/master/java/com/l2jserver/gameserver/network/clientpackets/ValidatePosition.java
- L2J `BotReportTable.java` — https://github.com/andridgitalbox/l2j-mobius/blob/master/java/com/l2jserver/gameserver/datatables/BotReportTable.java
- L2J `botreport_punishments.xml` — https://github.com/andridgitalbox/l2j-mobius/blob/master/dist/game/config/botreport_punishments.xml
- L2J `LongTimeEvent.java` — https://github.com/rukhavi/l2j-mobius/blob/master/java/com/l2jserver/gameserver/model/event/LongTimeEvent.java
- L2Day event script and config.xml (letters 3875-3888, event NPCs) — https://gist.github.com/Pandragon/9915099
- Lineage II anniversary letter-collection events — https://www.engadget.com/2011-04-20-lineage-iis-7th-anniversary-kicks-off-with-three-weeks-of-event.html
- Saving Santa holiday event — https://www.engadget.com/2008-12-15-celebrating-the-holidays-with-lineage-iis-saving-santa-event.html

Anti-cheat and bot detection
- Chen et al., "Identifying MMORPG Bots: A Traffic Analysis Approach" — https://homepage.iis.sinica.edu.tw/~swc/pub/bot_identification.html
- Thawonmas, "Detection of MMORPG Misconducts Based on Action Frequencies, Types and Time-Intervals", DMIN 2010 — https://www.ice.ci.ritsumei.ac.jp/~ruck/PAP/dmin10.pdf
- "Cheating and Detection Method in MMORPG: Systematic Literature Review" (IEEE Access) — https://scholar.korea.ac.kr/handle/2021.sw.korea/142114
- "Multimodal Game Bot Detection using User Behavioral Characteristics" — https://arxiv.org/html/1606.01426
- tower-governor (GCRA rate limiting for axum/tonic) — https://docs.rs/tower-governor/latest/tower_governor/

Release engineering
- Protocol Buffers: updating a message type — https://protobuf.dev/programming-guides/proto3/#updating
- buf breaking change detection — https://buf.build/docs/breaking/ and https://buf.build/docs/breaking/usage/
- sqlx `migrate!` macro — https://docs.rs/sqlx/latest/sqlx/macro.migrate.html
- refinery (alternative considered) — https://docs.rs/refinery/latest/refinery/
- tonic `Server::serve_with_shutdown`, `accept_http1` — https://docs.rs/tonic/latest/tonic/transport/server/struct.Server.html
- axum `Serve::with_graceful_shutdown` — https://docs.rs/axum/latest/axum/serve/struct.Serve.html
- cargo-chef multi-stage Dockerfile — https://github.com/lukemathwalker/cargo-chef
- moon CI guide (`moon ci`, affected detection, `--job/--job-total`, run-report-action, `runInCI`) — https://moonrepo.dev/docs/guides/ci
- Blue/green deployment and stateful session considerations — https://www.redhat.com/en/topics/devops/what-is-blue-green-deployment and https://igaming.createit.com/news/blue-green-deployment-strategy-for-online-casinos/

Telemetry
- tracing-opentelemetry 0.34 — https://docs.rs/tracing-opentelemetry/latest/tracing_opentelemetry/
- metrics-exporter-prometheus 0.18 — https://docs.rs/metrics-exporter-prometheus/latest/metrics_exporter_prometheus/
- OpenTelemetry Rust — https://github.com/open-telemetry/opentelemetry-rust
- Instrumenting Rust/axum with OpenTelemetry — https://oneuptime.com/blog/post/2026-02-06-instrument-rust-axum-opentelemetry/view
- Monitoring game server tick rate with OpenTelemetry — https://oneuptime.com/blog/post/2026-02-06-monitor-game-server-tick-rate-opentelemetry/view
- Prometheus metric naming — https://prometheus.io/docs/practices/naming/
- Prometheus histograms and summaries — https://prometheus.io/docs/practices/histograms/
- Grafana provisioning (dashboards and datasources from files) — https://grafana.com/docs/grafana/latest/administration/provisioning/
- tower-http request_id (SetRequestId, PropagateRequestId, TraceLayer ordering) — https://docs.rs/tower-http/latest/tower_http/request_id/index.html
- sentry-tracing — https://docs.rs/crate/sentry-tracing/latest
- Sentry Rust SDK (init before runtime, panic integration) — https://docs.sentry.io/platforms/rust/

Events
- tokio-cron-scheduler (6-field cron, Job::new_async, persistence) — https://github.com/mvniekerk/tokio-cron-scheduler
- tokio-cron-scheduler cron format guide — https://www.cronuru.com/guides/tokio-cron-scheduler
