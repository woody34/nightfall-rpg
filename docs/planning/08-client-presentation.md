# Phase 8: Client Presentation

Camera and movement rendering, HUD and windows, animation and effects, audio, localization,
accessibility, and the concrete client architecture (scenes, state store, network layer, folder
layout) for the Phaser 3 client in `apps/client`.

---

> **Partially superseded (2026-10-07).** The client engine is now Unreal Engine 5, not Phaser
> (`docs/research/engine-comparison.md` §0). Sections on Phaser scenes, Preact HUD, Connect-ES,
> tilemaps, and the TypeScript layout no longer apply. What still binds the server: 10 Hz
> snapshots, 100-200 ms interpolation window, server-authoritative click-to-move with `MoveTo`
> intents and `seq` acks, `SystemMessage{id, params}` localization, and the `ClientMessage` /
> `ServerMessage` envelopes. The Unreal client design lives in `apps/client-unreal/README.md`.

## 1. Purpose and scope

**Delivers**

- A server-authoritative movement renderer: click-to-move intent, local prediction for the
  player, interpolation for everyone else, and a camera that follows with deadzone and zoom.
- A tilemap world drawn from Tiled JSON with correct depth sorting for a top-down 2.5D view.
- The full L2-style HUD (status bars, target frame, hotbar pages, system log, chat, inventory,
  skills, party, minimap) as an HTML/CSS overlay bound to a client-side state store.
- A sprite/animation pipeline (atlases, per-weapon animation sets, skill VFX, floating damage,
  screen shake) and an asset pipeline with license-safe placeholder art.
- Audio (audio sprites, zone music crossfade, positional attenuation), localization (string tables
  keyed by system-message IDs, server sends IDs + typed params, never prose), and accessibility
  baselines.
- The client's network layer (Connect-ES over gRPC-Web to tonic-web), state store, scene list,
  and `apps/client/src` folder layout.

**Excludes**

- Game rules of any kind: the server decides hit/miss, damage, aggro, pathing (Phases 1-7).
- Server-side networking internals, persistence, and the gRPC service implementations beyond
  the message shapes the client consumes (Phase 0 and Phase 9).
- Final art direction. Everything here assumes placeholder art swapped later behind stable
  atlas keys.

---

## 2. Reference: how Lineage 2 does it

### 2.1 Movement is a destination, not a velocity

L2 is click-to-move: the player left-clicks the ground and the character runs there; holding and
dragging steers continuously. The client sends a *destination*, not per-frame input. In L2J the
inbound packet `MoveBackwardToLocation` carries `targetX/Y/Z`, `originX/Y/Z`, and a
`moveMovement` flag (0 = cursor keys, 1 = mouse). The server:

1. Rejects the move if `dx² + dy² > 98,010,000` (a 9,900-unit radius) or the character
   `isOutOfControl()` (stunned, paralyzed, dead, in a cutscene).
2. Adjusts `targetZ` by the template collision height.
3. Sets the AI intention `AI_INTENTION_MOVE_TO`, which drives movement at the server's
   `runSpeed`, then broadcasts `MoveToLocation` (current position, destination, object id) to
   everyone in range. Failure sends `ActionFailed`.

The client periodically sends `ValidatePosition` (its own x, y, z, heading). The server compares
against its simulated position and, when the squared difference exceeds `250,000` (500 units) or
the Z difference exceeds 200, sends `ValidateLocation` back, which snaps the client to the
server's coordinates. `GeoData.properties` exposes `CoordSynchronize` with three modes: `-1`
(sync only Z client→server, the default without geodata), `1` (client→server only, harder to
bypass obstacles without geodata), `2` (geodata mode: "server sends validation packet if client
goes too far from server calculated coordinates"). `Character.properties` caps `MaxRunSpeed` at
300 and `MaxPAtkSpeed` at 1500. The game runs on a 100 ms tick (L2J `GameTimeController`),
so position snapshots arrive at 10 Hz and the client interpolates visually between them.

Takeaways for Nightfall: destination-based intent is cheap on the wire (one message per click,
not per frame), trivially validated (one distance check plus a speed check per tick), and
naturally supports "render where the server says you are, with smoothing."

### 2.2 The L2 HUD

The classic (Interlude through High Five) interface consists of:

| Element | Contents | Notes |
|---|---|---|
| Status window | Name, level, CP / HP / MP bars with numeric overlays, XP bar, weight gauge, buff icon strip | CP (Combat Points) absorbs PvP damage before HP, class/level dependent |
| Target window | Target name, level, HP %, title/clan, buff/debuff icons, raid flag | Target HP is a percentage; exact values are hidden for non-party members |
| Shortcut panel | 12 slots bound to F1-F12; 10 pages switched with Alt+F1..F10; the Tutorial Book sits in F12 by default | Players use one page for attacks, one for buffs, one for consumables |
| Chat window | Tabs for All, Trade, Party, Clan, Alliance, plus whisper; prefixes `+` trade, `#` party, `@` clan, `$` alliance, `"name` whisper | Separate scrollback per tab |
| System message window | Combat log ("You hit for 123 damage"), gains, errors; driven by `SystemMessage` packets carrying an ID and typed params | Colored per message class via `systemmsg-e.dat` on the client |
| Inventory | Grid of 80 slots at base (expandable by Dwarf/skills), tabs for Equip, Supplies, Quest items; paperdoll with 12+ equipment slots | Weight and adena shown in footer |
| Skill window (Alt+K) | Tabs Active / Passive / Toggle, drag to hotbar | |
| Party window | Up to 9 members, HP/MP/CP bars, class icon, leader crown, distance-greyed when out of range | |
| Radar / minimap | Circular radar top-right, party dots, quest markers; Alt+M opens the world map | |
| Menu bar (Alt+X) | Options, Character (Alt+T), Inventory (Alt+V), Quest (Alt+U), Actions (Alt+C), Clan, Board (Alt+B) | Alt+H hides the whole UI |

### 2.3 System messages are IDs, not text

L2J's `SystemMessage` packet writes opcode `0x62`, the message id, a parameter count, then
each parameter as a type byte plus value. The types in `AbstractMessagePacket` are:

```
TYPE_TEXT=0  TYPE_INT_NUMBER=1  TYPE_NPC_NAME=2  TYPE_ITEM_NAME=3  TYPE_SKILL_NAME=4
TYPE_CASTLE_NAME=5  TYPE_LONG_NUMBER=6  TYPE_ZONE_NAME=7  TYPE_ELEMENT_NAME=9
TYPE_INSTANCE_NAME=10  TYPE_DOOR_NAME=11  TYPE_PLAYER_NAME=12  TYPE_SYSTEM_STRING=13
TYPE_CLASS_ID=15  TYPE_POPUP_ID=16
```

Builders like `addItemName(id)`, `addSkillName(id, lvl)`, `addPcName(pc)`, `addInt(n)` fill the
array sized by `SystemMessageId.getParamCount()`. The client owns the localized template
("You have earned $s1 experience and $s2 SP.") and substitutes `$s1`, `$s2`, `$c1` from the
typed params by looking up item/skill/NPC names in its own data tables. This is the model to
copy: the server never formats user-visible prose, so localization is a client-only concern and
the wire stays small.

### 2.4 Animation per weapon type

L2 characters carry a weapon-type-specific animation set: sword/blunt, dual, bow, pole, dagger,
fist, and "none" each have attack, attack-wait, run, walk, cast, death, and hit-recoil clips.
Attack timing is driven server-side by `pAtkSpd` (the hit lands at a fixed fraction of the swing)
and the client plays the clip scaled to that duration. Casting plays a loop for `castTime` then a
"shot" clip on completion; interrupt plays the hurt clip.

---

## 3. Design decisions for Nightfall

### 3.1 Perspective: top-down orthogonal 2.5D with y-sorting

**Decision:** Orthogonal top-down, 32 px tiles, characters 64 px tall drawn with their feet on the
tile, depth-sorted by foot y. Not isometric.

Alternatives considered:

| Option | Pros | Cons |
|---|---|---|
| Orthogonal top-down (chosen) | Tiled and Phaser support are first-class; LPC and Kenney assets are top-down; collision, pathing, and `worldToTileXY` are trivial; y-sorting gives a 2.5D feel | Less "L2-like" than isometric |
| Isometric (Tiled `isometric` orientation) | Phaser 3.50+ renders isometric tilemaps natively; evocative of classic MMOs | Few free isometric character sets; diagonal collision math; depth sorting needs `x + y` and is error-prone with multi-tile objects; roughly 2x asset cost |
| Staggered/hex | Native Phaser support | No reason for an RPG |

The server's world is 2D float coordinates (`Position{x, y}` in `game.proto`), tile size is a
rendering detail; 1 world unit = 1 px at zoom 1.

### 3.2 Input: click-to-move primary, WASD as a convenience that emits the same intent

**Decision:** Both inputs produce a `MoveTo{dest}` intent. Left-click on ground sends the
clicked world point. WASD/arrow keys, while held, send a destination `pos + dir * 96` every
200 ms (so the server sees a stream of short legs), and send a `StopMove` on release. This keeps
one server code path, one validation rule, and a wire cost of at most 5 moves/s for keyboard
users.

| | Click-to-move | WASD (velocity) |
|---|---|---|
| Messages/s while moving | ~1 per click | 10-20 (per input change or per tick) |
| Server validation | One speed check per tick against a path | Per-tick speed/acceleration checks, needs input sequence replay |
| Cheating surface | Small: destination within range, speed cap | Larger: needs input-sequence reconciliation |
| Feel | Deliberate, L2-like; auto-path around obstacles | Twitchy, better for action games |

Click-to-move also lets the server pathfind (Phase 6 AI already needs it), while the client only
renders the resulting path.

### 3.3 Rendering authoritative positions

Server sends a position snapshot for every entity in view every tick (100 ms, 10 Hz), plus an
immediate `EntityMoved` on each new destination. Two rendering policies:

- **Remote entities: interpolation 100 ms behind.** Keep a ring buffer of the last 4
  snapshots `(server_tick, x, y)`. Render time `t_render = t_server_now - 100 ms - jitter`
  (jitter starts at 50 ms, adapts to the 95th percentile of observed inter-arrival gaps). Find
  the two snapshots bracketing `t_render` and lerp. If the buffer runs dry, extrapolate along
  the last known destination at the entity's known speed for at most 250 ms, then hold.
- **Local player: prediction with destination replay.** On click, immediately start moving at
  the character's known `run_speed` (from the last stat update) toward `dest` along a straight
  line (the client does not pathfind; the server returns the path). When the server's
  `EntityMoved{path, speed, start_tick}` arrives (typically 40-120 ms later), swap the straight
  line for the server path and, if the predicted position is more than 48 px from where the
  server says the player is at `now`, blend the error out over 150 ms rather than snapping.
  If the server rejects the move (`MoveRejected`), stop at the server position.

This is Gambetta's split: the player sees themselves in the present, everyone else 100 ms in the
past. Combat does not need lag compensation because the server resolves hits by its own
positions and ranges; the client only shows the result.

### 3.4 Camera

`this.cameras.main.startFollow(player, true, 0.12, 0.12)` with `setDeadzone(160, 90)` (one
sixth of the 960x540 base viewport), `setBounds` to the map pixel size, and integer `setZoom`
values of 1, 2, or 3 (pixel art needs integer zoom; `roundPixels` on). Mouse wheel changes zoom
with `zoomTo(target, 150, 'Sine.easeOut')`. `shake(120, 0.004)` on taking a critical hit;
`flash(80, 255, 255, 255)` on level up; `fade` on zone transitions.

### 3.5 HUD: HTML/CSS overlay over the canvas, not Phaser DOM elements, not rexUI

**Decision:** A sibling `<div id="hud">` positioned over the canvas, rendered with Preact +
`@preact/signals` (about 4 KB + 2 KB gzipped), fed by the client state store. Phaser keeps only
in-world UI (nameplates, floating damage, target ring, cast bar above the head) as
`BitmapText`/`Graphics`.

| Option | Verdict |
|---|---|
| Plain HTML/CSS overlay (chosen) | Real text layout, scrolling, drag-and-drop, focus management, screen readers, CSS variables for theming/text scaling, DevTools inspection. Zero rendering cost to the game loop. |
| Phaser `DOMElement` (`dom: { createContainer: true }`) | Same HTML but tied to Phaser's scale manager and scene lifecycle; cannot interleave with sprites, "cannot be enabled for input" through Phaser, and nesting is limited to one level. Useful only if HUD needed to scroll with the camera, which ours does not. |
| rexUI (`phaser3-rex-plugins`) | 46+ canvas widgets (sizer, grid table, dialog, text box). Good when the UI must live inside the canvas (e.g. a console port). Costs draw calls, canvas text is blurry at non-integer scales, no accessibility tree. Keep as a fallback for an in-canvas mode. |
| Hand-rolled Phaser Graphics UI | Rejected: reinventing layout. |

Input routing: when an HTML input (chat box, search) has focus, Phaser's keyboard must not
swallow keys. We set `input.keyboard.enabled = false` on `focusin` of the HUD and re-enable on
`focusout`; `F1-F12` are captured by Phaser (`addCapture`) only while the canvas has focus.

### 3.6 Animation and effects

- Characters: LPC-compatible 64x64 frames, 4 directions (rows: up, left, down, right), clips
  **spellcast** (7 frames), **thrust** (8), **walk** (9), **slash** (6), **shoot** (13), **hurt**
  (6). Weapon types map onto these: sword/blunt/dual/fist to `slash`, dagger/pole to `thrust`,
  bow to `shoot`, casting to `spellcast`, death to `hurt` held on last frame. `idle` is walk
  frame 0.
- Attack clips are played with `duration = attack_interval_ms` from the server's
  `CombatStarted` event so swing speed matches `pAtkSpd`; the hit frame index per clip is a
  constant table (`slash: 3, thrust: 4, shoot: 9`) used to time the hit flash and damage number.
- Skill VFX: `this.add.particles(x, y, 'fx', {...})` for bursts (heal sparkle, fire burst) and
  short sprite-sheet effects for anything with shape (sword trail, holy circle), depth sorted
  with the caster.
- Floating damage: pooled `BitmapText` objects (pool of 64), tween `y -= 40` and alpha to 0 over
  800 ms, color by class (white normal, yellow critical, cyan heal, grey miss, red damage to
  self), scale 1.4 for crits.
- Screen shake only on crits taken and raid roars; a settings toggle disables it.

### 3.7 Audio

Web Audio via Phaser's `SoundManager` (falls back to HTML5 Audio). SFX packed into audio
sprites (one `.ogg` + `.m4a` pair with a JSON marker file per category: `ui`, `combat`,
`skills`, `ambient`). Zone music is streamed per zone as separate files; on zone change we tween
the old track's volume to 0 over 2000 ms while the new one tweens from 0 to the music setting
(crossfade). Positional SFX use `sound.setListenerPosition(player.x, player.y)` each frame and
sources with `refDistance: 160, maxDistance: 960, rolloffFactor: 1.2`. Audio is unlocked on the
first pointer down (Phaser handles `sound.locked`/`unlocked`); the Boot scene shows "Click to
start" until then.

### 3.8 Localization

Server sends `SystemMessage{id, params[]}` with typed params (mirroring L2). The client holds
`locales/<lang>/system.json` keyed by numeric id with `{0}`-style placeholders and ICU-ish
plural via `Intl.PluralRules`. UI strings live in `locales/<lang>/ui.json`. We write a 60-line
`t()` rather than pull i18next: our needs are interpolation, plurals and locale switching; we do
not need namespaces, backends, or React bindings. i18next remains the fallback if the custom
loader grows past a few hundred lines.

### 3.9 Accessibility baseline

- Keyboard-only play: Tab/Shift-Tab target cycling, `Enter` to interact, F-keys for hotbar,
  Alt+letter window toggles, all remappable in Settings and persisted to `localStorage`.
- Colorblind-safe bars: HP `#D55E00` (vermillion), MP `#0072B2` (blue), CP `#E69F00` (orange),
  XP `#009E73` (green) from the Okabe-Ito palette; bars also differ by position and icon so no
  information is color-only.
- Text scaling: a CSS variable `--ui-scale` (0.9 to 1.5) scales the HUD; minimum body text 14 px
  at 1080p, 16 px for chat/system log by default.
- Contrast: HUD text and bar fills meet 3:1 against adjacent colors (WCAG 1.4.11).
- Motion: screen shake and camera flash toggles. Reduced motion respects
  `prefers-reduced-motion`.

---

## 4. Data model

Client-side store entities (TypeScript) and the proto messages they are hydrated from. Proto
sketches extend `packages/proto/nightfall/v1/game.proto` and follow its naming (snake_case
fields, `RACE_`-style enum prefixes, `Position{x, y}` floats).

```proto
// ---- world streaming (new file: packages/proto/nightfall/v1/world.proto) ----
// Real-time traffic is NOT a gRPC service. Per 00-foundations.md §3.2 it rides one WebSocket at
// GET /ws?ticket=… with one protobuf message per binary frame. These are the two envelopes:
message ClientMessage {
  uint32 seq = 1;                                   // echoed back in Ack
  oneof intent {
    MoveToRequest move_to = 10;
    StopMoveRequest stop_move = 11;
    SetTargetRequest set_target = 12;
    AttackRequest attack = 13;
    UseSkillRequest use_skill = 14;
    UseItemRequest use_item = 15;
    ChatRequest chat = 16;
  }
}
message ServerMessage {
  oneof payload {
    Ack ack = 1;
    WorldEvent event = 2;                           // everything the client renders arrives here
  }
}
message Ack { uint32 seq = 1; }                      // echoes the client's intent seq

message MoveToRequest { uint32 seq = 1; Position dest = 2; bool keyboard = 3; }
message StopMoveRequest { uint32 seq = 1; }
message SetTargetRequest { uint32 seq = 1; uint64 entity_id = 2; }
message AttackRequest { uint32 seq = 1; uint64 target_id = 2; bool force = 3; } // force = Ctrl
message UseSkillRequest { uint32 seq = 1; uint32 skill_id = 2; uint64 target_id = 3; bool force = 4; }
message UseItemRequest { uint32 seq = 1; uint64 item_uid = 2; }
message ChatRequest { uint32 seq = 1; ChatChannel channel = 2; string text = 3; string whisper_to = 4; }

enum ChatChannel { CHAT_CHANNEL_UNSPECIFIED = 0; ALL = 1; TRADE = 2; PARTY = 3; CLAN = 4; ALLIANCE = 5; WHISPER = 6; SHOUT = 7; }

message WorldEvent {
  uint64 server_tick = 1;          // 100 ms ticks since server start
  int64 server_time_ms = 2;        // for clock sync with PingResponse.server_time_ms
  oneof event {
    EntitySpawn spawn = 10;
    EntityDespawn despawn = 11;
    EntityMoved moved = 12;        // new destination / path
    PositionBatch positions = 13;  // 10 Hz snapshot of nearby entities
    VitalsUpdate vitals = 14;      // hp/mp/cp for self, party, target
    CombatEvent combat = 15;
    CastStarted cast_started = 16;
    CastEnded cast_ended = 17;
    EffectApplied effect_applied = 18;
    EffectRemoved effect_removed = 19;
    SystemMessage system_message = 20;
    ChatMessage chat = 21;
    MoveRejected move_rejected = 22;
    InventoryDelta inventory = 23;
    StatsUpdate stats = 24;
    ZoneChanged zone = 25;
    PartyUpdate party = 26;
    TargetChanged target = 27;
  }
}

enum EntityKind { ENTITY_KIND_UNSPECIFIED = 0; PLAYER = 1; NPC = 2; MONSTER = 3; PET = 4; DROP = 5; }

message EntitySpawn {
  uint64 entity_id = 1; EntityKind kind = 2; string name = 3; string title = 4;
  uint32 level = 5; Position pos = 6; float heading = 7;
  uint32 appearance_id = 8;        // atlas/animation set key
  uint32 weapon_type = 9;          // selects attack clip
  float run_speed = 10;            // px per second, for extrapolation/prediction
  bool attackable = 11; bool pvp_flag = 12; uint32 clan_id = 13;
  uint32 hp_percent = 14;          // non-party entities only expose %
}
message EntityDespawn { uint64 entity_id = 1; }
message EntityMoved  { uint64 entity_id = 1; Position from = 2; repeated Position path = 3; float speed = 4; }
message PositionBatch { repeated EntityPos entries = 1; }
message EntityPos { uint64 entity_id = 1; Position pos = 2; float heading = 3; bool moving = 4; }
message MoveRejected { uint32 seq = 1; Position server_pos = 2; uint32 reason_msg_id = 3; }

message VitalsUpdate { uint64 entity_id = 1; uint32 hp = 2; uint32 max_hp = 3; uint32 mp = 4; uint32 max_mp = 5; uint32 cp = 6; uint32 max_cp = 7; }
message CombatEvent {
  uint64 attacker_id = 1; uint64 target_id = 2; uint32 damage = 3;
  bool critical = 4; bool miss = 5; bool blocked = 6; bool heal = 7; bool killed = 8;
  uint32 skill_id = 9; uint32 attack_interval_ms = 10;   // client scales swing clip to this
}
message CastStarted { uint64 caster_id = 1; uint32 skill_id = 2; uint32 cast_time_ms = 3; uint64 target_id = 4; }
message CastEnded { uint64 caster_id = 1; bool interrupted = 2; }
message EffectApplied { uint64 entity_id = 1; uint32 effect_id = 2; uint32 duration_ms = 3; uint32 stacks = 4; }
message EffectRemoved { uint64 entity_id = 1; uint32 effect_id = 2; }

// L2-style: an id plus typed params; the client owns the text.
message SystemMessage {
  uint32 message_id = 1;
  repeated MessageParam params = 2;
}
message MessageParam {
  oneof value {
    string text = 1; int64 number = 2; uint32 item_id = 3; uint32 skill_id = 4;
    uint32 npc_id = 5; string player_name = 6; uint32 zone_id = 7; uint32 class_id = 8;
  }
}
message ChatMessage { ChatChannel channel = 1; string sender = 2; uint64 sender_id = 3; string text = 4; int64 time_ms = 5; }
message ZoneChanged { uint32 zone_id = 1; string map_key = 2; uint32 music_id = 3; Position spawn = 4; }
```

Client store shape (zustand-style vanilla store; see 6/7):

```ts
interface WorldState {
  self: { id: bigint; stats: Stats; vitals: Vitals; xp: Xp; position: Vec2; target?: bigint };
  entities: Map<bigint, EntityView>;     // all visible entities, including self
  party: PartyMember[];
  inventory: InventoryItem[]; equipment: Partial<Record<Slot, InventoryItem>>;
  skills: SkillView[]; hotbar: HotbarSlot[][];   // 10 pages x 12 slots
  effects: Map<bigint, EffectView[]>;
  systemLog: RingBuffer<RenderedMessage>;        // 500 entries
  chat: Record<ChatChannel, RingBuffer<ChatLine>>;
  zone: { id: number; mapKey: string; musicId: number };
  net: { connected: boolean; rttMs: number; clockOffsetMs: number; serverTick: bigint };
}
interface EntityView {
  id: bigint; kind: EntityKind; name: string; title: string; level: number;
  appearanceId: number; weaponType: number; runSpeed: number;
  snapshots: RingBuffer<{ t: number; x: number; y: number; heading: number }>; // 4 deep
  path?: Vec2[]; pathStartMs?: number; speed?: number;
  hpPercent: number; vitals?: Vitals; pvp: boolean; attackable: boolean;
}
```

---

## 5. Interfaces

### 5.1 Transport: Connect-ES over gRPC-Web to tonic-web

| Option | Verdict |
|---|---|
| **Connect-ES (`@connectrpc/connect-web`) with `createGrpcWebTransport` (chosen for request/response)** | Generated TS from our `.proto` via `protoc-gen-es` v2 (messages and service descriptors in one `*_pb.ts`); `createClient(GameService, transport)` gives promise-based unary calls; small bundle; `tonic-web` 0.14 serves gRPC-Web natively with `GrpcWebLayer` + `accept_http1(true)` + CORS. Same proto files drive Rust and TS. Used for auth, character, inventory, shop, mail, social: everything that is not the world tick. |
| Connect protocol (`createConnectTransport`) | Nicer (plain HTTP/JSON debuggable) but there is no maintained Connect protocol server for tonic; would need a proxy. Revisit if one appears. |
| `grpc-web` (Google's `grpc-web` npm) | Older, callback API, bigger generated code, closure-style. |
| **WebSocket + protobuf (chosen for the real-time channel)** | Decided in Phase 0: one binary frame per `ClientMessage`/`ServerMessage`, encoded with the same `protoc-gen-es` classes (`toBinary`/`fromBinary` from `@bufbuild/protobuf`), so there is no hand-rolled framing. Truly bidirectional, no per-intent HTTP request, and it works through any load balancer. We own reconnect and `seq` matching, which is about 60 lines (sketch below). |

Browser constraint: gRPC-Web has no client streaming or bidi, which is why intents do not go over
gRPC. Reconnect: on socket close, back off 0.5 s, 1 s, 2 s, 4 s (max 10 s), reopen `/ws` with a
fresh play ticket (Phase 0 §3.5), and the server replays a full `EntitySpawn` set for the view (so
the client clears `entities` on reconnect).

`tonic-web` server side (sketch for `apps/api/src/grpc.rs`):

```rust
use tonic_web::GrpcWebLayer;
use tower_http::cors::{Any, CorsLayer};

Server::builder()
    .accept_http1(true)
    .layer(CorsLayer::new().allow_origin(Any).allow_headers(Any).expose_headers(Any))
    .layer(GrpcWebLayer::new())
    .add_service(GameServiceServer::new(GameServiceImpl))
    .add_service(WorldServiceServer::new(world))
    .serve(addr).await?;
```

### 5.2 TypeScript client sketch

```ts
// src/net/transport.ts
import { createClient } from '@connectrpc/connect';
import { createGrpcWebTransport } from '@connectrpc/connect-web';
import { fromBinary, toBinary } from '@bufbuild/protobuf';
import { GameService } from '../gen/nightfall/v1/game_pb';
import { ClientMessageSchema, ServerMessageSchema } from '../gen/nightfall/v1/world_pb';

// Request/response: gRPC-Web to tonic-web.
const transport = createGrpcWebTransport({
  baseUrl: import.meta.env.VITE_GRPC_BASE ?? 'http://localhost:50051',
  useBinaryFormat: true,
});
export const game = createClient(GameService, transport);

// src/net/session.ts — real-time channel: WebSocket at /ws (Phase 0 §3.2), protobuf per frame.
export class Session {
  private seq = 0;
  private ws?: WebSocket;
  constructor(private store: WorldStore, private ticket: string) {}

  async start() {
    const ping = await game.ping({ clientVersion: __APP_VERSION__ });
    this.store.net.clockOffsetMs = Number(ping.serverTimeMs) - Date.now();
    this.open();
  }
  private open(attempt = 0) {
    const base = import.meta.env.VITE_WS_BASE ?? 'ws://localhost:3000';
    const ws = new WebSocket(`${base}/ws?ticket=${encodeURIComponent(this.ticket)}`);
    ws.binaryType = 'arraybuffer';
    ws.onopen = () => { this.store.net.connected = true; attempt = 0; };
    ws.onmessage = (m) => {
      const msg = fromBinary(ServerMessageSchema, new Uint8Array(m.data as ArrayBuffer));
      if (msg.payload.case === 'event') applyEvent(this.store, msg.payload.value); // src/state/reducers.ts
      else if (msg.payload.case === 'ack') this.store.net.lastAckedSeq = msg.payload.value.seq;
    };
    ws.onclose = () => {
      this.store.net.connected = false;
      const delay = Math.min(10_000, 500 * 2 ** attempt);
      setTimeout(() => this.open(attempt + 1), delay);  // ticket refresh via game.refreshTicket() if expired
    };
    this.ws = ws;
  }
  private send(intent: ClientMessage['intent']) {
    const seq = ++this.seq;
    this.ws?.send(toBinary(ClientMessageSchema, { seq, intent } as ClientMessage));
    return seq;
  }
  moveTo(dest: Vec2, keyboard = false) {
    const seq = ++this.seq;
    this.store.predictMove(seq, dest);
    world.moveTo({ seq, dest, keyboard }).catch(() => this.store.cancelPrediction(seq));
    return seq;
  }
  useSkill(skillId: number, targetId: bigint, force = false) {
    return world.useSkill({ seq: ++this.seq, skillId, targetId, force });
  }
}
```

`buf.gen.yaml` at `packages/proto/`:

```yaml
version: v2
clean: true
plugins:
  - local: protoc-gen-es
    out: ../../apps/client/src/gen
    include_imports: true
    opt: target=ts
```

A moon task `proto:gen-ts` runs `buf generate`; `client:build` depends on it.

### 5.3 Proto messages the client consumes

| Message | Consumer |
|---|---|
| `PingResponse` | clock offset, version gate (Phase 9) |
| `Character`, `BaseStats`, `StatsUpdate` | character sheet, run speed for prediction |
| `EntitySpawn/Despawn/Moved`, `PositionBatch` | entity renderer, minimap |
| `VitalsUpdate` | status window, target frame, party frames, nameplate bars |
| `CombatEvent` | attack clip timing, hit flash, floating numbers, system log line |
| `CastStarted/CastEnded` | cast bar, spellcast loop |
| `EffectApplied/Removed` | buff strips (self, target, party), toggle highlights |
| `SystemMessage` | system log (localized client side) |
| `ChatMessage` | chat tabs |
| `MoveRejected` | prediction rollback |
| `InventoryDelta` | inventory grid, weight gauge, hotbar item counts |
| `ZoneChanged` | tilemap load, music crossfade, camera bounds |
| `PartyUpdate`, `TargetChanged` | party window, target frame |

---

## 6. Rust implementation notes

Client-facing server work that this phase requires of `apps/api` (the rest is Phase 0/3/6):

- `src/grpc.rs` gains `tonic_web::GrpcWebLayer`, `accept_http1(true)`, and a CORS layer
  (`tower-http` already has the `cors` feature). Add `tonic-web = "0.14"` to the workspace
  dependencies.
- The axum `/ws` handler (`net::session`, Phase 0) hands the world an `mpsc::Sender<ServerMessage>`
  with capacity 256 per session; a slow client that fills the channel is disconnected (log
  `client_backpressure_disconnect`, Phase 9 metric).
- A `broadcast` module batches positions per tick: for each session, collect entities within
  the interest radius (1,200 px, about 1.25 viewports at zoom 1) into one `PositionBatch`.
  Only entities whose position changed since the last batch are included.
- `SystemMessage` ids live in `packages/proto/nightfall/v1/system_messages.proto` as an enum
  so Rust and TS share the id space; the client's `system.json` is keyed by the enum number.
- Clock: `WorldEvent.server_time_ms` on every event lets the client re-estimate offset with an
  EMA (alpha 0.1), which keeps interpolation stable without a separate time sync RPC.

---

## 7. Client implications

### 7.1 Scene list

| Scene | Key | Role |
|---|---|---|
| `BootScene` | `Boot` | Health check (exists), load fonts/UI atlas, show "Click to start" to unlock audio |
| `PreloadScene` | `Preload` | Load zone-independent atlases, audio sprites, locale JSON, bitmap fonts; progress bar |
| `LoginScene` | `Login` | Account/character select (HTML forms in overlay); produces session token |
| `WorldScene` | `World` | Tilemap, entities, camera, input to intents, in-world UI (nameplates, damage numbers) |
| `HudBridgeScene` | `Hud` | Launched in parallel (`scene.launch('Hud')`); owns the Preact root and subscribes it to the store; handles keyboard capture handoff |
| `TransitionScene` | `Transition` | Fade in/out and zone map swap on `ZoneChanged` |

### 7.2 State store

A vanilla TypeScript store (zustand's `createStore` from `zustand/vanilla`, 1 KB) holding
`WorldState`. Reducers in `state/reducers.ts` apply `WorldEvent`s. Phaser objects subscribe via
`store.subscribe(selector, cb)`; Preact components use `useStore`. The renderer reads
`entities` every frame directly (no events) for interpolation.

### 7.3 Tilemap rendering

```ts
const map = this.make.tilemap({ key: zone.mapKey });                 // Tiled JSON
const tiles = map.addTilesetImage('terrain', 'tiles-terrain');       // 32x32
const ground = map.createLayer('ground', tiles).setDepth(0);
const detail = map.createLayer('detail', tiles).setDepth(1);
const over   = map.createLayer('overhead', tiles).setDepth(10_000);  // tree tops, roofs
// "props" is an object layer: each object becomes a sprite depth-sorted with characters
map.getObjectLayer('props')!.objects.forEach(o => {
  const s = this.add.image(o.x!, o.y!, 'props', o.name!).setOrigin(0, 1);
  s.setDepth(s.y);   // feet y
});
this.cameras.main.setBounds(0, 0, map.widthInPixels, map.heightInPixels);
```

Depth rule: ground 0, detail 1, dynamic entities and props `depth = footY` (range 2 ..
`heightInPixels`), overhead layer above all, in-world UI `heightInPixels + 1000`. Entities set
`setDepth(y)` each frame after interpolation. Tiled layer properties `collides=true` are only
used for click-target validation (do not send a MoveTo into a wall); the server is still the
authority.

### 7.4 Interpolation loop (per frame)

```ts
update(_time: number, dt: number) {
  const renderT = Date.now() + net.clockOffsetMs - INTERP_DELAY_MS - net.jitterMs;
  for (const e of store.entities.values()) {
    const view = views.get(e.id)!;
    if (e.id === store.self.id) { view.setPosition(prediction.x, prediction.y); }
    else { const p = sampleSnapshots(e.snapshots, renderT, e); view.setPosition(p.x, p.y); }
    view.setDepth(view.y);
    view.playIfNotPlaying(e.moving ? `${e.appearanceId}-walk-${dir(e.heading)}` : `${e.appearanceId}-idle-${dir(e.heading)}`);
  }
  prediction.step(dt);   // advance local player along path at runSpeed; blend correction
}
```

### 7.5 HUD component list (Preact)

`StatusWindow` (name, level, CP/HP/MP bars, XP bar, weight, buff strip), `TargetFrame` (name,
level, HP %, debuffs, cast bar), `Hotbar` (12 slots, page selector, cooldown sweep, drag from
`SkillWindow`/`Inventory`), `SystemLog` (500-line ring, filter chips), `ChatPanel` (tabs per
channel, input with prefix parsing `+ # @ $ "`), `Inventory` (grid 10x8, paperdoll, tabs),
`SkillWindow` (Active/Passive/Toggle), `PartyFrames` (up to 9), `Minimap` (canvas element
drawing entity dots from the store; 1 px = 8 world px), `Settings` (keybinds, audio sliders, UI
scale, colorblind toggles, screen shake), `Tooltip`, `ContextMenu`, `DeathDialog`,
`LoadingOverlay`.

### 7.6 Asset pipeline

- Source art under `apps/client/assets-src/` (not shipped); packed atlases + JSON under
  `apps/client/public/atlas/`.
- Packer: **Free Texture Packer** (open source, exports Phaser 3 JSON hash) or TexturePacker if
  licensed. Phaser loads with `this.load.atlas(key, png, json)`; animations are created with
  `anims.generateFrameNames(key, { prefix: 'human-slash-down-', start: 0, end: 5 })`.
  Aseprite-exported sheets load via `this.load.aseprite` + `anims.createFromAseprite`.
- Placeholder art and licenses: **Kenney** (CC0, no attribution required; UI, icons, tiles),
  **LPC base assets on OpenGameArt** (dual CC-BY-SA 3.0 / GPL 3.0; requires attribution via the
  bundle's CREDITS.txt and share-alike on derivatives; some contributors also allow CC-BY 3.0),
  other OpenGameArt submissions individually (check each: CC0, CC-BY, CC-BY-SA, GPL). Keep a
  `apps/client/public/CREDITS.md` generated from a `credits.json` manifest; CI fails if an atlas
  source lacks a license entry.
- Bitmap font: generate from a CC0/OFL font with BMFont or `msdf-bmfont-xml`; load with
  `this.load.bitmapFont('ui-12', 'fonts/ui-12.png', 'fonts/ui-12.xml')`. Used only for
  in-world text (names, damage); HUD uses web fonts.
- Audio: `.ogg` primary, `.m4a` fallback (Safari), both listed in `load.audioSprite(key,
  'sfx.json', ['sfx.ogg', 'sfx.m4a'])`. Music 96 kbps OGG, loop points set in the JSON config.
  Sources: Kenney audio (CC0), OpenGameArt (per-track), freesound (CC0 filter).

### 7.7 Folder layout (`apps/client/src`)

```
src/
  main.ts                 # Phaser.Game config, mounts HUD root
  api.ts                  # REST helpers (exists)
  gen/nightfall/v1/       # protoc-gen-es output (gitignored, built by proto:gen-ts)
  net/
    transport.ts          # Connect transport + clients
    session.ts            # subscribe loop, reconnect, intent senders with seq
    clock.ts              # server time offset EMA, jitter estimate
  state/
    store.ts              # zustand vanilla store, WorldState
    reducers.ts           # applyEvent(store, WorldEvent)
    selectors.ts
    prediction.ts         # local player path prediction + reconciliation
    ringbuffer.ts
  scenes/
    BootScene.ts PreloadScene.ts LoginScene.ts WorldScene.ts HudBridgeScene.ts TransitionScene.ts
  world/
    EntityView.ts         # sprite + nameplate + bars + cast bar
    EntityRenderer.ts     # entity map <-> views, interpolation, depth sort
    TilemapLoader.ts      # Tiled JSON -> layers, props, collision mask
    InputController.ts    # click/WASD/Tab -> intents, key capture handoff
    CameraRig.ts          # follow/deadzone/zoom/shake
    fx/ DamageNumbers.ts SkillVfx.ts HitFlash.ts
    anim/ AnimationSets.ts (clip tables per appearance/weapon) 
  hud/
    index.tsx             # Preact root, store binding
    components/ StatusWindow.tsx TargetFrame.tsx Hotbar.tsx SystemLog.tsx ChatPanel.tsx
                Inventory.tsx SkillWindow.tsx PartyFrames.tsx Minimap.tsx Settings.tsx ...
    styles/ tokens.css (palette, --ui-scale) hud.css
  audio/
    AudioManager.ts       # sprites, music crossfade, positional listener
  i18n/
    t.ts                  # loader, interpolation, Intl.PluralRules
    systemMessages.ts     # id -> template lookup, typed param formatting (item/skill/npc names)
  settings/
    keybinds.ts settings.ts (localStorage persistence)
  assets/
    manifest.ts           # atlas/audio keys and paths; credits.json validation
```

Dependencies to add: `@connectrpc/connect`, `@connectrpc/connect-web`, `@bufbuild/protobuf`,
`preact`, `@preact/signals`, `zustand`; dev: `@bufbuild/buf`, `@bufbuild/protoc-gen-es`.

---

## 8. Open questions

1. **Interest radius vs. zoom.** At zoom 1 a 1,200 px radius covers the viewport; at zoom 0.5
   (if ever allowed) entities would pop at the edges. Decide whether zoom-out is allowed at all
   or whether the server scales the radius per session.
2. **Position batch frequency.** 10 Hz matches L2J's tick; moving to 20 Hz halves interpolation
   delay at 2x bandwidth. Measure with 200 entities in view before choosing.
3. **Unary intents latency.** If the measured click-to-move latency exceeds ~150 ms on a
   typical connection because of HTTP/1.1 request setup, move intents to a WebSocket side
   channel (server already runs axum).
4. **HUD framework.** Preact is the recommendation; if the team prefers no framework, the
   component list and store binding stay the same with vanilla custom elements.
5. **Isometric later?** The store, network, and HUD are perspective-agnostic; only
   `TilemapLoader`, `EntityRenderer` depth rules, and the art change. Confirm orthogonal is
   final before commissioning art.
6. **System message id registry.** Who owns allocation (proto enum vs. a data file) and how
   do content authors add messages without a proto change.
7. **Text rendering in-world.** Bitmap fonts limit the glyph set; for CJK locales nameplates may
   need canvas `Text` or an HTML nameplate layer. Decide once target locales are known.

---

## 9. Sources

Lineage 2 / L2J reference
- L2J (Mobius lineage) `MoveBackwardToLocation.java` — https://github.com/andridgitalbox/l2j-mobius/blob/master/java/com/l2jserver/gameserver/network/clientpackets/MoveBackwardToLocation.java
- L2J `ValidatePosition.java` — https://github.com/andridgitalbox/l2j-mobius/blob/master/java/com/l2jserver/gameserver/network/clientpackets/ValidatePosition.java
- L2J `GeoData.properties` (CoordSynchronize modes) — https://github.com/andridgitalbox/l2j-mobius/blob/master/dist/game/config/GeoData.properties
- L2J `Character.properties` (MaxRunSpeed, MaxPAtkSpeed) — https://github.com/andridgitalbox/l2j-mobius/blob/master/dist/game/config/Character.properties
- L2J `AbstractMessagePacket.java` (SystemMessage param types) — https://github.com/andridgitalbox/l2j-mobius/blob/master/java/com/l2jserver/gameserver/network/serverpackets/AbstractMessagePacket.java
- L2J `SystemMessage.java` — https://github.com/andridgitalbox/l2j-mobius/blob/master/java/com/l2jserver/gameserver/network/serverpackets/SystemMessage.java
- Lineage 2 shortcut keys (F1-F12, Alt+F1..F10, Alt+letter windows) — https://www.onlinegamecommands.com/lineage-2-shortcut-hot-keys/
- Lineage 2 controls and UI basics (click-to-move, hotbar pages, chat) — https://l2calendar.com/blog/lineage-2-controls-ui-basics
- Lineage 2 Classic Encyclopaedia, Interface — https://l2wiki.com/classic/Interface
- CP / HP / MP mechanics — https://l2revolution.gamepedia.com/Mechanics
- `systemmsg-e.dat` placeholder discussion — https://www.elitepvpers.com/forum/lin2-exploits-hacks-bots-tools-macros/90100-client-modifications.html

Networking model
- Gambetta, Fast-Paced Multiplayer Part II (prediction, reconciliation) — https://gabrielgambetta.com/client-side-prediction-server-reconciliation.html
- Gambetta, Part III (entity interpolation) — https://gabrielgambetta.com/entity-interpolation.html
- Connect-ES getting started (web) — https://connectrpc.com/docs/web/getting-started/
- Connect-ES v2 migration (createClient, protoc-gen-es, buf.gen.yaml) — https://github.com/connectrpc/connect-es/blob/main/MIGRATING.md
- tonic-web 0.14 docs (GrpcWebLayer, accept_http1, CORS, no client streaming) — https://docs.rs/tonic-web/latest/tonic_web/
- tonic `Server` (accept_http1, layer, serve_with_shutdown) — https://docs.rs/tonic/latest/tonic/transport/server/struct.Server.html
- Using gRPC in React: from grpc-web to Connect — https://dev.to/arichy/using-grpc-in-react-the-modern-way-from-grpc-web-to-connect-41lc

Phaser 3
- Camera API (startFollow, setDeadzone, setLerp, setZoom, zoomTo, shake, flash, fade, setBounds) — https://docs.phaser.io/api-documentation/class/cameras-scene2d-camera
- Tilemap API (orientations, createLayer, addTilesetImage, getObjectLayer, renderOrder) — https://docs.phaser.io/api-documentation/class/tilemaps-tilemap
- Depth sorting by y in top-down games — https://phaser.discourse.group/t/change-depth-z-index-of-sprites-based-on-their-position/11268 and https://phaser.io/devlogs/110
- DOMElement (dom.createContainer, limitations) — https://docs.phaser.io/api-documentation/3.88.2/class/gameobjects-domelement
- Scenes (launch vs start, parallel UI scene) — https://docs.phaser.io/phaser/concepts/scenes
- Scale Manager (FIT, RESIZE, autoCenter, resize event) — https://docs.phaser.io/phaser/concepts/scale-manager
- Animations (anims.create, generateFrameNames, chain, createFromAseprite) — https://docs.phaser.io/phaser/concepts/animations
- Audio (Web Audio vs HTML5, audio sprites, locked/unlocked, spatial config) — https://docs.phaser.io/phaser/concepts/audio
- BaseSoundManager API — https://newdocs.phaser.io/docs/3.80.0/Phaser.Sound.BaseSoundManager
- Bitmap Text — https://docs.phaser.io/phaser/concepts/gameobjects/bitmap-text and https://github.com/samme/phaser3-faq/wiki/Bitmap-text
- Aseprite loader — https://docs.phaser.io/api-documentation/class/loader-filetypes-asepritefile
- rexUI overview (components, install) — https://rexrainbow.github.io/phaser3-rex-notes/docs/site/ui-overview/
- Free Texture Packer — https://phaser.io/news/2020/02/free-texture-packer
- TexturePacker + Phaser 3 tutorial — https://phaser.io/news/2018/03/texturepacker-and-phaser-3-tutorial

Assets and licensing
- Kenney license (CC0) — https://kenney.nl/support
- LPC base assets (CC-BY-SA 3.0 / GPL 3.0, CREDITS.txt) — https://opengameart.org/content/liberated-pixel-cup-lpc-base-assets-sprites-map-tiles

Localization and accessibility
- i18next getting started — https://www.i18next.com/overview/getting-started
- Okabe-Ito / Wong (2011) colorblind-safe palette hex codes — https://conceptviz.app/blog/okabe-ito-palette-hex-codes-complete-reference
- WCAG 2.1 SC 1.4.11 Non-text Contrast — https://www.boia.org/WCAG2/CP/1.4.11
- Game Accessibility Guidelines (basic) — https://gameaccessibilityguidelines.com/basic/
- Game Accessibility Guidelines (colour, text size, remapping) — https://gameaccessibilityguidelines.com/?p=12
