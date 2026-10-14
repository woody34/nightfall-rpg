# Phase 8: Client Presentation

This document is about these items for the Phaser 3 client in `apps/client`:

- Camera and movement rendering.
- HUD and windows.
- Animation and effects.
- Audio.
- Localization.
- Accessibility.
- The concrete client architecture (scenes, state store, network layer, folder layout).

---

> **Partially superseded (2026-10-07).** The client engine is now Unreal Engine 5, not Phaser
> (`docs/research/engine-comparison.md` §0). The sections about Phaser scenes, Preact HUD,
> Connect-ES, tilemaps, and the TypeScript layout do not apply now. These items still apply to
> the server:
>
> - 10 Hz snapshots.
> - A 100-200 ms interpolation window.
> - Server-authoritative click-to-move with `MoveTo` intents and `seq` acks.
> - `SystemMessage{id, params}` localization.
> - The `ClientMessage` / `ServerMessage` envelopes.
>
> The design of the Unreal client is in `apps/client-unreal/README.md`.

## 1. Purpose and scope

**Delivers**

- A server-authoritative movement renderer. It has click-to-move intent, local prediction for
  the player, and interpolation for all other entities. It also has a camera that follows the
  player with a deadzone and zoom.
- A tilemap world that we draw from Tiled JSON, with correct depth sorting for a top-down 2.5D
  view.
- The full HUD in the style of L2 (status bars, target frame, hotbar pages, system log, chat,
  inventory, skills, party, minimap). It is an HTML/CSS overlay that is bound to a client-side
  state store.
- A sprite/animation pipeline (atlases, animation sets for each weapon, skill VFX, floating
  damage, screen shake). Also an asset pipeline with placeholder art that has a safe license.
- Audio (audio sprites, zone music crossfade, positional attenuation).
- Localization (string tables keyed by system-message IDs). The server sends IDs + typed params,
  and it never sends prose.
- Accessibility baselines.
- The network layer of the client (Connect-ES over gRPC-Web to tonic-web), the state store, the
  scene list, and the `apps/client/src` folder layout.

**Excludes**

- All game rules: the server decides hit/miss, damage, aggro, and pathing (Phases 1-7).
- The internal parts of server-side networking, persistence, and the gRPC service
  implementations. This document includes only the message shapes that the client uses (Phase 0
  and Phase 9).
- The final art direction. All of this document assumes placeholder art. We replace it later
  behind stable atlas keys.

---

## 2. Reference: how Lineage 2 does it

### 2.1 Movement is a destination, not a velocity

L2 uses click-to-move. The player left-clicks the ground and the character runs there. If the
player holds the button and drags, the character steers continuously. The client sends a
*destination*, not input for each frame. In L2J, the inbound packet `MoveBackwardToLocation` has
`targetX/Y/Z`, `originX/Y/Z`, and a `moveMovement` flag (0 = cursor keys, 1 = mouse). The server
does these steps:

1. It rejects the move if `dx² + dy² > 98,010,000` (a 9,900-unit radius). It also rejects the
   move if the character is `isOutOfControl()` (stunned, paralyzed, dead, in a cutscene).
2. It changes `targetZ` by the template collision height.
3. It sets the AI intention `AI_INTENTION_MOVE_TO`, which moves the character at the `runSpeed`
   of the server. Then it broadcasts `MoveToLocation` (current position, destination, object id)
   to all players in range. If the move fails, the server sends `ActionFailed`.

At intervals, the client sends `ValidatePosition` (its own x, y, z, heading). The server
compares this with its simulated position. The squared difference can be more than `250,000`
(500 units), or the Z difference can be more than 200. In these conditions, the server sends
`ValidateLocation` back, which moves the client immediately to the coordinates of the server.
`GeoData.properties` gives `CoordSynchronize` with three modes:

- `-1`: sync only Z client→server. This is the default without geodata.
- `1`: client→server only. Without geodata, it is harder to go around obstacles.
- `2`: geodata mode ("server sends validation packet if client goes too far from server
  calculated coordinates").

`Character.properties` sets the limit of `MaxRunSpeed` to 300 and of `MaxPAtkSpeed` to 1500. The
game runs on a 100 ms tick (L2J `GameTimeController`). Thus, position snapshots arrive at 10 Hz,
and the client interpolates visually between them.

Lessons for Nightfall: an intent that is a destination has these advantages:

- It is cheap on the wire (one message for each click, not for each frame).
- It is easy to validate (one distance check and one speed check for each tick).
- It naturally supports "render where the server says you are, with smoothing."

### 2.2 The L2 HUD

The classic interface (Interlude through High Five) has these elements:

| Element | Contents | Notes |
|---|---|---|
| Status window | Name, level, CP / HP / MP bars with numeric overlays, XP bar, weight gauge, buff icon strip | CP (Combat Points) absorbs PvP damage before HP; it changes with the class/level |
| Target window | Target name, level, HP %, title/clan, buff/debuff icons, raid flag | Target HP is a percentage; the exact values are hidden for players who are not in the party |
| Shortcut panel | 12 slots bound to F1-F12; 10 pages that you change with Alt+F1..F10; by default, the Tutorial Book is in F12 | Players use one page for attacks, one for buffs, and one for consumables |
| Chat window | Tabs for All, Trade, Party, Clan, Alliance, and whisper; prefixes `+` trade, `#` party, `@` clan, `$` alliance, `"name` whisper | Each tab has its own scrollback |
| System message window | Combat log ("You hit for 123 damage"), gains, errors; `SystemMessage` packets with an ID and typed params supply the messages | The client sets the color for each message class through `systemmsg-e.dat` |
| Inventory | Grid of 80 slots at base (Dwarf/skills can make it larger), tabs for Equip, Supplies, Quest items; paperdoll with 12+ equipment slots | The footer shows the weight and the adena |
| Skill window (Alt+K) | Tabs Active / Passive / Toggle, drag to hotbar | |
| Party window | Up to 9 members, HP/MP/CP bars, class icon, leader crown, grey when the member is out of range | |
| Radar / minimap | Circular radar at the top right, party dots, quest markers; Alt+M opens the world map | |
| Menu bar (Alt+X) | Options, Character (Alt+T), Inventory (Alt+V), Quest (Alt+U), Actions (Alt+C), Clan, Board (Alt+B) | Alt+H hides all of the UI |

### 2.3 System messages are IDs, not text

The `SystemMessage` packet of L2J writes opcode `0x62`, the message id, and a parameter count.
Then it writes each parameter as a type byte and a value. The types in `AbstractMessagePacket`
are:

```
TYPE_TEXT=0  TYPE_INT_NUMBER=1  TYPE_NPC_NAME=2  TYPE_ITEM_NAME=3  TYPE_SKILL_NAME=4
TYPE_CASTLE_NAME=5  TYPE_LONG_NUMBER=6  TYPE_ZONE_NAME=7  TYPE_ELEMENT_NAME=9
TYPE_INSTANCE_NAME=10  TYPE_DOOR_NAME=11  TYPE_PLAYER_NAME=12  TYPE_SYSTEM_STRING=13
TYPE_CLASS_ID=15  TYPE_POPUP_ID=16
```

Builders such as `addItemName(id)`, `addSkillName(id, lvl)`, `addPcName(pc)`, and `addInt(n)`
fill the array. `SystemMessageId.getParamCount()` sets the size of the array. The client has the
localized template ("You have earned $s1 experience and $s2 SP."). It replaces `$s1`, `$s2`,
`$c1` with the typed params: it finds the item/skill/NPC names in its own data tables. We will
copy this model. The server never formats prose that the user sees. Thus, localization is a
concern of the client only, and the wire stays small.

### 2.4 Animation per weapon type

L2 characters have an animation set for each weapon type. Sword/blunt, dual, bow, pole, dagger,
fist, and "none" each have attack, attack-wait, run, walk, cast, death, and hit-recoil clips. On
the server side, `pAtkSpd` controls the attack timing (the hit occurs at a fixed fraction of the
swing). The client plays the clip, scaled to that duration. A cast plays a loop for `castTime`,
then a "shot" clip when it is complete. An interrupt plays the hurt clip.

---

## 3. Design decisions for Nightfall

### 3.1 Perspective: top-down orthogonal 2.5D with y-sorting

**Decision:** Orthogonal top-down view with 32 px tiles. Characters are 64 px tall, and we draw
them with their feet on the tile. We sort the depth by the y of the feet. The view is not
isometric.

Alternatives that we examined:

| Option | Pros | Cons |
|---|---|---|
| Orthogonal top-down (selected) | The support in Tiled and Phaser is first-class; LPC and Kenney assets are top-down; collision, pathing, and `worldToTileXY` are easy; the y-sort gives a 2.5D look | Less similar to L2 than isometric |
| Isometric (Tiled `isometric` orientation) | Phaser 3.50+ renders isometric tilemaps natively; it is similar to classic MMOs | Few free isometric character sets; diagonal collision math; the depth sort needs `x + y`, and it frequently has errors with objects of many tiles; approximately 2x asset cost |
| Staggered/hex | Native Phaser support | No reason to use it for an RPG |

The world of the server uses 2D float coordinates (`Position{x, y}` in `game.proto`). The tile
size is a rendering detail. 1 world unit = 1 px at zoom 1.

### 3.2 Input: click-to-move primary, WASD as a convenience that emits the same intent

**Decision:** The two inputs make a `MoveTo{dest}` intent. A left-click on the ground sends the
world point of the click. While the player holds the WASD/arrow keys, they send a destination
`pos + dir * 96` each 200 ms. Thus, the server gets a stream of short legs. When the player
releases the keys, they send a `StopMove`. Thus, we keep one code path on the server and one
validation rule. The wire cost for keyboard users is a maximum of 5 moves/s.

| | Click-to-move | WASD (velocity) |
|---|---|---|
| Messages/s during movement | ~1 for each click | 10-20 (for each input change or for each tick) |
| Server validation | One speed check for each tick against a path | Speed/acceleration checks for each tick; input sequence replay is necessary |
| Cheating surface | Small: destination in range, speed limit | Larger: input-sequence reconciliation is necessary |
| Feel | Deliberate, similar to L2; automatic path around obstacles | Fast and nervous, better for action games |

With click-to-move, the server can also find the path (the Phase 6 AI already needs this). The
client only renders the path that the server sends.

### 3.3 Rendering authoritative positions

At each tick (100 ms, 10 Hz), the server sends a position snapshot for each entity in view. It
also sends an immediate `EntityMoved` for each new destination. There are two rendering
policies:

- **Remote entities: interpolation 100 ms behind.** Keep a ring buffer of the last 4
  snapshots `(server_tick, x, y)`. The render time is `t_render = t_server_now - 100 ms - jitter`.
  The jitter starts at 50 ms and changes to the 95th percentile of the measured gaps between
  arrivals. Find the two snapshots before and after `t_render`, and lerp. If the buffer becomes
  empty, extrapolate along the last known destination at the known speed of the entity. Do this
  for a maximum of 250 ms, then hold the position.
- **Local player: prediction with destination replay.** On a click, immediately start to move
  toward `dest` along a straight line. Use the known `run_speed` of the character (from the last
  stat update). The client does not find the path; the server returns the path. The
  `EntityMoved{path, speed, start_tick}` of the server arrives, frequently 40-120 ms later. Then
  replace the straight line with the server path. The predicted position can be more than 48 px
  from the position that the server gives for the player at `now`. If so, blend the error out
  over 150 ms; do not snap. If the server rejects the move (`MoveRejected`), stop at the server
  position.

This is the split of Gambetta. The player sees the local character in the present, and sees all
other entities 100 ms in the past. Combat does not need lag compensation, because the server
resolves hits with its own positions and ranges. The client only shows the result.

### 3.4 Camera

Use `this.cameras.main.startFollow(player, true, 0.12, 0.12)` with these settings:

- `setDeadzone(160, 90)` (one sixth of the 960x540 base viewport).
- `setBounds` to the pixel size of the map.
- Integer `setZoom` values of 1, 2, or 3 (pixel art needs an integer zoom; `roundPixels` on).

The mouse wheel changes the zoom with `zoomTo(target, 150, 'Sine.easeOut')`. Use
`shake(120, 0.004)` when the player gets a critical hit, `flash(80, 255, 255, 255)` on a level
up, and `fade` on zone transitions.

### 3.5 HUD: HTML/CSS overlay over the canvas, not Phaser DOM elements, not rexUI

**Decision:** A sibling `<div id="hud">` over the canvas. We render it with Preact +
`@preact/signals` (approximately 4 KB + 2 KB gzipped), and the client state store supplies its
data. Phaser keeps only the in-world UI (nameplates, floating damage, target ring, cast bar above
the head) as `BitmapText`/`Graphics`.

| Option | Verdict |
|---|---|
| Plain HTML/CSS overlay (selected) | Real text layout, scrolling, drag-and-drop, focus control, screen readers, CSS variables for themes/text scale, DevTools inspection. Zero rendering cost to the game loop. |
| Phaser `DOMElement` (`dom: { createContainer: true }`) | The same HTML, but it is tied to the scale manager and scene lifecycle of Phaser. It cannot interleave with sprites, it "cannot be enabled for input" through Phaser, and the limit of nesting is one level. It is useful only if the HUD must scroll with the camera. Our HUD does not. |
| rexUI (`phaser3-rex-plugins`) | 46+ canvas widgets (sizer, grid table, dialog, text box). Good when the UI must be in the canvas (for example, a console port). It costs draw calls, canvas text is blurry at scales that are not integers, and there is no accessibility tree. Keep it as a fallback for a mode in the canvas. |
| Phaser Graphics UI written by hand | Rejected: we would write the layout again. |

Input routing: when an HTML input (chat box, search) has focus, the keyboard of Phaser must not
take the keys. We set `input.keyboard.enabled = false` on `focusin` of the HUD, and we enable it
again on `focusout`. Phaser captures `F1-F12` (`addCapture`) only while the canvas has focus.

### 3.6 Animation and effects

- Characters: 64x64 frames that are compatible with LPC, 4 directions (rows: up, left, down,
  right). The clips are **spellcast** (7 frames), **thrust** (8), **walk** (9), **slash** (6),
  **shoot** (13), **hurt** (6). The weapon types map to these clips:
  - sword/blunt/dual/fist to `slash`
  - dagger/pole to `thrust`
  - bow to `shoot`
  - a cast to `spellcast`
  - death to `hurt`, held on the last frame.

  `idle` is walk frame 0.
- Attack clips play with `duration = attack_interval_ms` from the `CombatStarted` event of the
  server. Thus, the swing speed is the same as `pAtkSpd`. A constant table gives the hit frame
  index for each clip (`slash: 3, thrust: 4, shoot: 9`). We use it to time the hit flash and the
  damage number.
- Skill VFX: use `this.add.particles(x, y, 'fx', {...})` for bursts (heal sparkle, fire burst).
  Use short sprite-sheet effects for all effects with a shape (sword trail, holy circle). Sort
  their depth with the caster.
- Floating damage: pooled `BitmapText` objects (pool of 64). Tween `y -= 40` and alpha to 0 over
  800 ms. The color shows the class (white normal, yellow critical, cyan heal, grey miss, red
  damage to self). The scale is 1.4 for crits.
- Screen shake occurs only when the player gets a crit and on raid roars. A toggle in the
  settings disables it.

### 3.7 Audio

We use Web Audio through the `SoundManager` of Phaser (the fallback is HTML5 Audio). We pack the
SFX into audio sprites. Each category has one `.ogg` + `.m4a` pair with a JSON marker file. The
categories are `ui`, `combat`, `skills`, `ambient`. We stream the zone music for each zone as
separate files. On a zone change, we tween the volume of the old track to 0 over 2000 ms. At the
same time, we tween the new track from 0 to the music setting (crossfade). Positional SFX use
`sound.setListenerPosition(player.x, player.y)` each frame, and sources with
`refDistance: 160, maxDistance: 960, rolloffFactor: 1.2`. The audio unlocks on the first pointer
down (Phaser controls `sound.locked`/`unlocked`). Until then, the Boot scene shows "Click to
start".

### 3.8 Localization

The server sends `SystemMessage{id, params[]}` with typed params (the same as L2). The client
has `locales/<lang>/system.json`, keyed by numeric id, with `{0}`-style placeholders. It does
plurals similar to ICU with `Intl.PluralRules`. The UI strings are in `locales/<lang>/ui.json`.
We write a `t()` of 60 lines and do not use i18next. We need only interpolation, plurals and
locale changes. We do not need namespaces, backends, or React bindings. If the custom loader
becomes larger than a few hundred lines, i18next is the fallback.

### 3.9 Accessibility baseline

- Play with the keyboard only: Tab/Shift-Tab to cycle the target, `Enter` to interact, F-keys
  for the hotbar, Alt+letter to open and close windows. You can remap all of these keys in
  Settings, and we keep them in `localStorage`.
- Bars that are safe for colorblind players: HP `#D55E00` (vermillion), MP `#0072B2` (blue), CP
  `#E69F00` (orange), XP `#009E73` (green) from the Okabe-Ito palette. The bars also have
  different positions and icons. Thus, no information is shown only by color.
- Text scale: a CSS variable `--ui-scale` (0.9 to 1.5) scales the HUD. By default, the minimum
  body text is 14 px at 1080p, and 16 px for the chat/system log.
- Contrast: HUD text and bar fills have a minimum of 3:1 against adjacent colors (WCAG 1.4.11).
- Motion: toggles for screen shake and camera flash. Reduced motion obeys
  `prefers-reduced-motion`.

---

## 4. Data model

This section gives the client-side store entities (TypeScript) and the proto messages that
supply their data. The proto sketches extend `packages/proto/nightfall/v1/game.proto`. They use
its naming rules (snake_case fields, `RACE_`-style enum prefixes, `Position{x, y}` floats).

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

The shape of the client store (a vanilla store in the zustand style; see 6/7):

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
| **Connect-ES (`@connectrpc/connect-web`) with `createGrpcWebTransport` (selected for request/response)** | TS generated from our `.proto` with `protoc-gen-es` v2 (messages and service descriptors in one `*_pb.ts`); `createClient(GameService, transport)` gives unary calls that return promises; small bundle; `tonic-web` 0.14 serves gRPC-Web natively with `GrpcWebLayer` + `accept_http1(true)` + CORS. The same proto files are the source for Rust and TS. We use it for auth, character, inventory, shop, mail, social: all that is not the world tick. |
| Connect protocol (`createConnectTransport`) | Better (you can debug plain HTTP/JSON), but there is no maintained Connect protocol server for tonic; a proxy would be necessary. Examine it again if one becomes available. |
| `grpc-web` (Google's `grpc-web` npm) | Older, callback API, larger generated code, closure style. |
| **WebSocket + protobuf (selected for the real-time channel)** | Decided in Phase 0: one binary frame for each `ClientMessage`/`ServerMessage`, encoded with the same `protoc-gen-es` classes (`toBinary`/`fromBinary` from `@bufbuild/protobuf`). Thus, there is no framing written by hand. It is truly bidirectional, there is no HTTP request for each intent, and it operates through all load balancers. We are responsible for reconnect and `seq` matching, which is approximately 60 lines (sketch below). |

Browser constraint: gRPC-Web has no client streaming or bidi. For this reason, intents do not
go over gRPC. Reconnect: on socket close, back off 0.5 s, 1 s, 2 s, 4 s (max 10 s). Then open
`/ws` again with a new play ticket (Phase 0 §3.5). The server replays a full `EntitySpawn` set
for the view. Thus, the client clears `entities` on reconnect.

The `tonic-web` server side (sketch for `apps/api/src/grpc.rs`):

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

A moon task `proto:gen-ts` runs `buf generate`. `client:build` depends on it.

### 5.3 Proto messages the client consumes

| Message | Consumer |
|---|---|
| `PingResponse` | clock offset, version gate (Phase 9) |
| `Character`, `BaseStats`, `StatsUpdate` | character sheet, run speed for the prediction |
| `EntitySpawn/Despawn/Moved`, `PositionBatch` | entity renderer, minimap |
| `VitalsUpdate` | status window, target frame, party frames, nameplate bars |
| `CombatEvent` | attack clip timing, hit flash, floating numbers, system log line |
| `CastStarted/CastEnded` | cast bar, spellcast loop |
| `EffectApplied/Removed` | buff strips (self, target, party), toggle highlights |
| `SystemMessage` | system log (localized on the client side) |
| `ChatMessage` | chat tabs |
| `MoveRejected` | prediction rollback |
| `InventoryDelta` | inventory grid, weight gauge, hotbar item counts |
| `ZoneChanged` | tilemap load, music crossfade, camera bounds |
| `PartyUpdate`, `TargetChanged` | party window, target frame |

---

## 6. Rust implementation notes

This phase needs this server work in `apps/api` for the client (Phase 0/3/6 has the remaining
work):

- `src/grpc.rs` gets `tonic_web::GrpcWebLayer`, `accept_http1(true)`, and a CORS layer
  (`tower-http` already has the `cors` feature). Add `tonic-web = "0.14"` to the workspace
  dependencies.
- The axum `/ws` handler (`net::session`, Phase 0) gives the world an
  `mpsc::Sender<ServerMessage>` with a capacity of 256 for each session. If a slow client fills
  the channel, the server disconnects it (log `client_backpressure_disconnect`, Phase 9 metric).
- A `broadcast` module batches the positions for each tick. For each session, it collects the
  entities in the interest radius (1,200 px, approximately 1.25 viewports at zoom 1) into one
  `PositionBatch`. It includes only the entities with a position change since the last batch.
- The `SystemMessage` ids are in `packages/proto/nightfall/v1/system_messages.proto` as an
  enum. Thus, Rust and TS share the id space. The `system.json` of the client is keyed by the
  enum number.
- Clock: `WorldEvent.server_time_ms` is on each event. With it, the client calculates the offset
  again with an EMA (alpha 0.1). This keeps the interpolation stable without a separate time
  sync RPC.

---

## 7. Client implications

### 7.1 Scene list

| Scene | Key | Role |
|---|---|---|
| `BootScene` | `Boot` | Health check (exists), load fonts/UI atlas, show "Click to start" to unlock the audio |
| `PreloadScene` | `Preload` | Load the atlases that are not specific to a zone, audio sprites, locale JSON, bitmap fonts; progress bar |
| `LoginScene` | `Login` | Account/character selection (HTML forms in the overlay); makes the session token |
| `WorldScene` | `World` | Tilemap, entities, camera, input to intents, in-world UI (nameplates, damage numbers) |
| `HudBridgeScene` | `Hud` | Starts in parallel (`scene.launch('Hud')`); is responsible for the Preact root and subscribes it to the store; controls the handoff of the keyboard capture |
| `TransitionScene` | `Transition` | Fade in/out and zone map change on `ZoneChanged` |

### 7.2 State store

A vanilla TypeScript store (the `createStore` of zustand from `zustand/vanilla`, 1 KB) keeps the
`WorldState`. Reducers in `state/reducers.ts` apply `WorldEvent`s. Phaser objects subscribe with
`store.subscribe(selector, cb)`. Preact components use `useStore`. For interpolation, the
renderer reads `entities` directly each frame (no events).

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

Depth rule:

- ground 0
- detail 1
- dynamic entities and props `depth = footY` (range 2 .. `heightInPixels`)
- overhead layer above all
- in-world UI `heightInPixels + 1000`.

Entities set `setDepth(y)` each frame after interpolation. We use the Tiled layer properties
`collides=true` only to validate the click target (do not send a MoveTo into a wall). The server
is still the authority.

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

The HUD has these components:

- `StatusWindow` (name, level, CP/HP/MP bars, XP bar, weight, buff strip).
- `TargetFrame` (name, level, HP %, debuffs, cast bar).
- `Hotbar` (12 slots, page selector, cooldown sweep, drag from `SkillWindow`/`Inventory`).
- `SystemLog` (ring of 500 lines, filter chips).
- `ChatPanel` (one tab for each channel, input that parses the prefixes `+ # @ $ "`).
- `Inventory` (grid 10x8, paperdoll, tabs).
- `SkillWindow` (Active/Passive/Toggle).
- `PartyFrames` (up to 9).
- `Minimap` (a canvas element that draws entity dots from the store; 1 px = 8 world px).
- `Settings` (keybinds, audio sliders, UI scale, colorblind toggles, screen shake).
- `Tooltip`, `ContextMenu`, `DeathDialog`, `LoadingOverlay`.

### 7.6 Asset pipeline

- Source art is in `apps/client/assets-src/` (we do not release it). Packed atlases + JSON are
  in `apps/client/public/atlas/`.
- Packer: **Free Texture Packer** (open source, exports Phaser 3 JSON hash), or TexturePacker if
  we have a license. Phaser loads with `this.load.atlas(key, png, json)`. We make the animations
  with `anims.generateFrameNames(key, { prefix: 'human-slash-down-', start: 0, end: 5 })`.
  Sheets that Aseprite exports load with `this.load.aseprite` + `anims.createFromAseprite`.
- Placeholder art and licenses:
  - **Kenney** (CC0, no attribution necessary; UI, icons, tiles).
  - **LPC base assets on OpenGameArt** (dual CC-BY-SA 3.0 / GPL 3.0). These need attribution
    through the CREDITS.txt of the bundle, and share-alike on derivatives. Some contributors
    also permit CC-BY 3.0.
  - Other OpenGameArt submissions, one at a time (examine each: CC0, CC-BY, CC-BY-SA, GPL).

  Keep an `apps/client/public/CREDITS.md` that we generate from a `credits.json` manifest. If
  an atlas source has no license entry, CI fails.
- Bitmap font: generate it from a CC0/OFL font with BMFont or `msdf-bmfont-xml`. Load it with
  `this.load.bitmapFont('ui-12', 'fonts/ui-12.png', 'fonts/ui-12.xml')`. We use it only for
  in-world text (names, damage). The HUD uses web fonts.
- Audio: `.ogg` is primary and `.m4a` is the fallback (Safari). Both are in `load.audioSprite(key,
  'sfx.json', ['sfx.ogg', 'sfx.m4a'])`. Music is 96 kbps OGG, with loop points set in the JSON
  config. Sources: Kenney audio (CC0), OpenGameArt (license for each track), freesound (CC0
  filter).

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

Add these dependencies: `@connectrpc/connect`, `@connectrpc/connect-web`, `@bufbuild/protobuf`,
`preact`, `@preact/signals`, `zustand`; dev: `@bufbuild/buf`, `@bufbuild/protoc-gen-es`.

---

## 8. Open questions

1. **Interest radius vs. zoom.** At zoom 1, a 1,200 px radius covers the viewport. At zoom 0.5
   (if we permit it), entities would appear suddenly at the edges. Decide if we permit zoom-out.
   Alternatively, decide if the server scales the radius for each session.
2. **Position batch frequency.** 10 Hz is the same as the tick of L2J. At 20 Hz, the
   interpolation delay is half, and the bandwidth is 2x. Measure with 200 entities in view
   before you make the decision.
3. **Latency of unary intents.** The measured click-to-move latency can be more than ~150 ms on
   a usual connection because of the HTTP/1.1 request setup. If so, move the intents to a
   WebSocket side channel (the server already runs axum).
4. **HUD framework.** We recommend Preact. If the team wants no framework, the component list
   and the store binding stay the same with vanilla custom elements.
5. **Isometric later?** The store, network, and HUD do not depend on the perspective. Only
   `TilemapLoader`, the `EntityRenderer` depth rules, and the art change. Confirm that the
   orthogonal view is final before we order art.
6. **System message id registry.** Who is responsible for the allocation (proto enum vs. a data
   file)? How do content authors add messages without a proto change?
7. **Text rendering in-world.** Bitmap fonts limit the glyph set. For CJK locales, nameplates
   can need canvas `Text` or an HTML nameplate layer. Decide when we know the target locales.

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
