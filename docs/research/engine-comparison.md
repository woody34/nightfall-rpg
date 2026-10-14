# Client Engine Comparison: Bevy vs Godot vs Unreal

**Date:** 2026-10-07. **Status:** DECIDED 2026-10-07: Unreal Engine 5 (see §0).
**Question:** which engine do we use to build the real Nightfall client, as a replacement for the Phaser prototype?

The detailed matrices for each engine (32 rows each, with sources) are in
[engine-bevy.md](engine-bevy.md), [engine-godot.md](engine-godot.md), and
[engine-unreal.md](engine-unreal.md). This document is the synthesis.

## 0. Decision: Unreal Engine 5, thin client over the Rust server

We examined the recommendation in §5 (Godot) and did not accept it. There are two reasons. The
scorecard did not give sufficient weight to these two reasons:

1. **The networking objection does not apply to a thin client**. Replication, GAS, Iris, and the
   dedicated server are optional. A client that keeps a WebSocket to the Rust server and moves
   pawns from snapshots uses none of them. Games of this genre use this design. Lineage 2 was an
   Unreal Engine 2 client against the C++ server of NCSoft. Throne and Liberty and Aion 2 use
   the same design on UE4 and UE5.
2. **Visual quality comes mostly from the asset floor, and the floor of Unreal is much higher**.
   Unreal has Megascans, MetaHuman, the Game Animation Sample, Lumen without baking, and a
   marketplace of complete environments. These let a solo developer release a game that looks
   like a modern MMO.
   Godot and Bevy games look mediocre because their authors must build that floor themselves.

Accepted costs:
- C++ and Blueprints on the client. The Rust domain crate stays on the server side, and a thin
  client wants this. We generate protobuf for C++ from the same `.proto` files.
- A daily iteration loop that is slower than Rust.
- No browser client.
- A ~300 MB install floor.
- A UE6 migration at approximately 2028.

Consequences:
- We retire the Phaser client. The browser is not a target.
- The client is in `apps/client-unreal`. Its README has the architecture and the first slice.
- `docs/planning/08-client-presentation.md` is superseded where it assumes Phaser. Its
  decisions for the server side (snapshot cadence, interpolation window, SystemMessage ids) stay.

## 1. Context that drives the decision

- The server is Rust and authoritative. The client is a renderer and an intent sender. From
  networking, the client must have a WebSocket with protobuf frames and a gRPC-web client, and
  nothing more.
- There is one solo developer. The time to a playable slice and the cost of the daily iteration
  loop are more important than the peak quality of the rendering.
- The world of Lineage 2 uses zones with hunting grounds for bands of levels. Seamless streaming
  at the scale of a continent is good to have, but it is not necessary.
- Native desktop is necessary. Browser deployment is desirable. The Phaser prototype already
  runs in a browser.
- Versions that we researched: Bevy 0.19.1 (Jun 2026), Godot 4.7.2 (Aug 2026), Unreal 5.8 (Jun 2026).

## 2. Scorecard

Status key: ● native and stable, ◐ partial, plugin, or beta, ○ missing or abandoned.

| # | Capability | Bevy | Godot | Unreal |
|---|-----------|:----:|:-----:|:------:|
| 1 | Stable release cadence with LTS | ○ every minor breaks, no LTS | ◐ ~6-month minors, no LTS | ◐ 2 majors/yr, no LTS, UE6 comes soon |
| 2 | Use our Rust domain crate again in the client | ● zero FFI | ◐ with gdext 0.5 (0.x) | ○ C-ABI shim by hand |
| 3 | Scene editor and inspector | ○ archived prototypes, MVP is a 2026-27 goal | ● | ● |
| 4 | Hot reload (code and assets) | ◐ assets yes, code experimental | ● GDScript; Rust in the editor only | ◐ Live Coding, fragile in 5.8 |
| 5 | Licensing cost | ● MIT/Apache | ● MIT | ◐ 5% above $1M |
| 6 | Skeletal animation, state machines, IK, retargeting | ◐ graph native, state machine/IK with a plugin that has one maintainer | ● AnimationTree, new IK in 4.6 | ● best in class |
| 7 | 2D, tilemaps, isometric | ◐ bevy_ecs_tilemap | ● TileMapLayer with iso/Y-sort | ◐ Paper2D, weak |
| 8 | World streaming, LOD, HLOD, terrain | ◐ LOD and GPU culling; no streaming or terrain | ◐ LOD/occlusion; no partition/HLOD; Terrain3D plugin | ● World Partition, HLOD, Nanite |
| 9 | Hundreds of characters on screen | ◐ GPU-driven, beta | ◐ good at hundreds, strains at thousands | ● Mass/Nanite skeletal |
| 10 | VFX | ◐ hanabi (WebGPU only on web) | ● GPUParticles | ● Niagara |
| 11 | Lighting and day/night | ● atmosphere, shadows | ● SDFGI | ● Lumen, MegaLights |
| 12 | Physics, character controller, navmesh | ◐ avian + tnua + vleue_navigator | ● Jolt built in, NavigationServer | ● Chaos, CMC |
| 13 | UI with much data (inventory, trade, quest log, auction) | ○ no reactivity, no drag-drop, no rich text, no i18n | ● Control nodes, RichTextLabel, drag-drop, i18n | ● UMG, CommonUI, MVVM |
| 14 | Audio buses, crossfade, spatial | ◐ bevy_kira_audio | ● | ● MetaSounds |
| 15 | Input rebinding and click-to-move picking | ● bevy_picking | ● InputMap | ● Enhanced Input |
| 16 | Data-driven content and scripting | ◐ BSN experimental | ● Resources, GDScript | ◐ Blueprints; Verse not available |
| 17 | WebSocket, protobuf, gRPC-web clients | ● prost/tonic, same codegen as server | ◐ WebSocketPeer native; protobuf with Rust | ◐ WebSockets module; TurboLink, no gRPC-web |
| 18 | Built-in multiplayer does not interfere | ● none in core | ● fully optional | ○ engine assumes a UE server |
| 19 | Prediction and interpolation helpers | ◐ lightyear (heavy) or written by hand | ◐ physics interpolation; netfox | ◐ built for UE server only |
| 20 | Headless mode for shared simulation | ● MinimalPlugins | ● --headless | ● dedicated server target |
| 21 | Desktop exports | ● | ● | ● |
| 22 | Web export | ◐ WebGPU, single-threaded, 5-10 MB | ◐ WebGL2 only; Rust path experimental; no C# | ○ none since UE 4.24 |
| 23 | Asset pipeline (glTF, FBX, compression) | ◐ glTF only, no FBX | ● glTF, FBX, .blend | ● Interchange |
| 24 | Iteration speed | ◐ 1-3 min clean, seconds incremental | ● no compile for GDScript | ○ worst of the three |
| 25 | Linux CI and automated tests | ● cargo test | ● headless templates, GUT | ◐ heavy containers |
| 26 | Community and learning resources | ◐ | ● | ● |
| 27 | Released MMO or large multiplayer title | ○ none | ○ none at MMO scale | ● Aion 2, Dune Awakening, Throne and Liberty |
| 30 | Integration effort with our Rust server | ● ~1 week | ◐ desktop low, web high | ○ 2-4 weeks and continuous FFI work |
| 31 | Time to a desktop vertical slice (solo) | 8-12 wk 2D, 16-24 wk 3D | 4-8 wk part-time | months |

## 3. Where Bevy is short

Your instinct is correct. For the needs of an MMO client, the gaps of Bevy are in a small number
of areas, and they are on the critical path:

1. **No editor**. You must do the zone layout, the UI layout, and the animation state machines in
   code or in third-party tools. The official prototypes were archived in April 2026. The editor
   MVP is the stated top priority for the 2026-27 year. Thus, it is not available for the first
   year of this project.
2. **UI without reactivity**. An MMO client is mostly UI: inventory grids, drag and drop, rich
   text chat, quest logs, trade windows, and auction search. `bevy_ui` has no data binding,
   no drag and drop, no rich text, and no localization. Each important Bevy game that was
   released uses egui for all UI with much data, or it has its own UI code.
3. **Breaking changes in each release**. There are approximately 95 migration entries from 0.18 to
   0.19, and the ecosystem crates are some weeks late. The maintainers do not give a timeline for
   a production-ready release.
4. **No world streaming, HLOD, terrain, or animation state machine in core**. There is a plugin
   for each of these, but one person maintains each plugin.
5. **No released MMO**. Tiny Glade and Jarl both forked the engine and replaced parts of it.

Bevy has one advantage that the other engines do not have. The domain crate, the prost messages,
and all shared simulation code compile into the client with no changes. That is a real
advantage, but it saves approximately one week of integration. The gaps above cost months.

## 4. Why Unreal is the wrong tool here

The rendering, animation, and world tools of Unreal are the best available, and teams released
real MMOs with it. But for this project, it loses on three points that no function can offset:

- **Its multiplayer stack assumes a UE dedicated server**. Replication, Iris, GAS prediction,
  and the Character Movement Component all work against a design where Rust is authoritative.
  The client would use none of them and must work around their assumptions.
- **The iteration loop for a solo developer is the worst of the three**. The causes are C++
  compiles, fragile Live Coding in 5.8, and a 12-16 core and 32 GB baseline. GPU device-removed
  crashes also occur again and again.
- **There is no browser path**. Also, UE6 with Verse will make the Actor and Blueprint code of
  today legacy by 2028. Rust can be used again only through FFI; `unreal-rust` is dead.

## 5. Original recommendation (superseded by §0): Godot 4 with Rust through gdext

Godot covers 24 of the 32 rows natively and stably. These include the two rows where Bevy is
weakest (editor, UI with much data). The specific design:

- **Godot 4.7 (pin one version for each milestone), Jolt physics, Forward+ renderer on desktop**.
- **One Rust `cdylib` crate built with gdext 0.5**. It depends on a new `nightfall-domain`
  crate that we extract from `apps/api/src/domain`. It is responsible for the WebSocket, the
  protobuf decoding, the snapshot buffer, and the interpolation math. It gives a `NetClient` node
  and typed signals. GDScript never touches protobuf.
- **GDScript for scenes, UI, and animation**. In this area, the editor of Godot gives the most
  value. In this area, Bevy would also cost the most time.
- **Web as a stretch target, not a plan**. The web export of Godot is WebGL2-only, and the Rust
  path must have nightly Rust and emscripten. If a browser client is important, keep the Phaser
  prototype as the thin browser client against the same server. You already have this
  arrangement.
- **A world design that uses zones**. Lineage 2 already uses zones. Each hunting zone is a scene
  that is loaded with `load_threaded_request`. Do not try to stream at the scale of a continent in
  Godot.

### Risks and how to retire them early

| Risk | Test to run in the first two weeks |
|------|------------------------------------|
| Character count at scale | Spawn 300 animated `Skeleton3D` characters with LOD in one zone. Measure the frame time on the target hardware. |
| gdext churn | Pin `godot` 0.5.x and Godot 4.7.x together. Upgrade the two only at milestone boundaries. |
| gdext on web | Decide now that web is a stretch target. Do not spend time on the emscripten build until the desktop client is released. |
| No released MMO on Godot | The server is responsible for the simulation and the client is a renderer, and this decreases the risk. Keep the client thin. |

### When Bevy would be the correct decision

- If the client stays **2D or isometric** for a long time: `bevy_ecs_tilemap` and egui get to a
  slice in 8-12 weeks. The UI gap is less of a problem, and the advantage of the shared crate has
  no cost.
- If it becomes central to share **simulation code** between client and server (for example,
  client-side prediction of the combat math). Godot can also do this with gdext, with one FFI
  boundary.
- If the editor MVP is released and `bevy_ui` gets reactivity. Examine this again in 12 months.

### Decision needed

Confirm Godot and gdext for the desktop client. Also decide if we keep Phaser as the browser
client. After the confirmation, do these steps:
1. Extract `nightfall-domain` into `packages/domain`.
2. Scaffold `apps/client-godot` with the gdext crate.
3. Run the two-week risk tests above.
