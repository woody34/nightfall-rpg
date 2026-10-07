# Client Engine Comparison: Bevy vs Godot vs Unreal

**Date:** 2026-10-07. **Status:** recommendation, not yet decided.
**Question:** which engine should the real Nightfall client be built in, replacing the Phaser prototype?

Detailed per-engine matrices (32 rows each, with sources) are in
[engine-bevy.md](engine-bevy.md), [engine-godot.md](engine-godot.md), and
[engine-unreal.md](engine-unreal.md). This document is the synthesis.

## 1. Context that drives the decision

- The server is Rust and authoritative. The client is a renderer plus intent sender. It needs a
  WebSocket with protobuf frames and a gRPC-web client, and nothing more from networking.
- Solo developer. Time to a playable slice and the cost of the daily iteration loop matter more
  than peak rendering quality.
- Lineage 2's world is zone-based with level-banded hunting grounds. Seamless continent-scale
  streaming is nice, not required.
- Native desktop is required. Browser deployment is desirable. The Phaser prototype already
  runs in a browser.
- Versions researched: Bevy 0.19.1 (Jun 2026), Godot 4.7.2 (Aug 2026), Unreal 5.8 (Jun 2026).

## 2. Scorecard

Status key: ● native and stable, ◐ partial, plugin, or beta, ○ missing or abandoned.

| # | Capability | Bevy | Godot | Unreal |
|---|-----------|:----:|:-----:|:------:|
| 1 | Stable release cadence with LTS | ○ every minor breaks, no LTS | ◐ ~6-month minors, no LTS | ◐ 2 majors/yr, no LTS, UE6 looming |
| 2 | Reuse our Rust domain crate in the client | ● zero FFI | ◐ via gdext 0.5 (0.x) | ○ C-ABI shim by hand |
| 3 | Scene editor and inspector | ○ archived prototypes, MVP is a 2026-27 goal | ● | ● |
| 4 | Hot reload (code and assets) | ◐ assets yes, code experimental | ● GDScript; Rust editor-only | ◐ Live Coding, fragile in 5.8 |
| 5 | Licensing cost | ● MIT/Apache | ● MIT | ◐ 5% above $1M |
| 6 | Skeletal animation, state machines, IK, retargeting | ◐ graph native, state machine/IK via one-maintainer plugin | ● AnimationTree, new IK in 4.6 | ● best in class |
| 7 | 2D, tilemaps, isometric | ◐ bevy_ecs_tilemap | ● TileMapLayer with iso/Y-sort | ◐ Paper2D, weak |
| 8 | World streaming, LOD, HLOD, terrain | ◐ LOD and GPU culling; no streaming or terrain | ◐ LOD/occlusion; no partition/HLOD; Terrain3D plugin | ● World Partition, HLOD, Nanite |
| 9 | Hundreds of characters on screen | ◐ GPU-driven, beta | ◐ fine at hundreds, strains at thousands | ● Mass/Nanite skeletal |
| 10 | VFX | ◐ hanabi (WebGPU only on web) | ● GPUParticles | ● Niagara |
| 11 | Lighting and day/night | ● atmosphere, shadows | ● SDFGI | ● Lumen, MegaLights |
| 12 | Physics, character controller, navmesh | ◐ avian + tnua + vleue_navigator | ● Jolt built in, NavigationServer | ● Chaos, CMC |
| 13 | Data-heavy UI (inventory, trade, quest log, auction) | ○ no reactivity, no drag-drop, no rich text, no i18n | ● Control nodes, RichTextLabel, drag-drop, i18n | ● UMG, CommonUI, MVVM |
| 14 | Audio buses, crossfade, spatial | ◐ bevy_kira_audio | ● | ● MetaSounds |
| 15 | Input rebinding and click-to-move picking | ● bevy_picking | ● InputMap | ● Enhanced Input |
| 16 | Data-driven content and scripting | ◐ BSN experimental | ● Resources, GDScript | ◐ Blueprints; Verse not available |
| 17 | WebSocket, protobuf, gRPC-web clients | ● prost/tonic, same codegen as server | ◐ WebSocketPeer native; protobuf via Rust | ◐ WebSockets module; TurboLink, no gRPC-web |
| 18 | Built-in multiplayer stays out of the way | ● none in core | ● fully optional | ○ engine assumes a UE server |
| 19 | Prediction and interpolation helpers | ◐ lightyear (heavy) or hand-rolled | ◐ physics interpolation; netfox | ◐ built for UE server only |
| 20 | Headless mode for shared simulation | ● MinimalPlugins | ● --headless | ● dedicated server target |
| 21 | Desktop exports | ● | ● | ● |
| 22 | Web export | ◐ WebGPU, single-threaded, 5-10 MB | ◐ WebGL2 only; Rust path experimental; no C# | ○ none since UE 4.24 |
| 23 | Asset pipeline (glTF, FBX, compression) | ◐ glTF only, no FBX | ● glTF, FBX, .blend | ● Interchange |
| 24 | Iteration speed | ◐ 1-3 min clean, seconds incremental | ● no compile for GDScript | ○ worst of the three |
| 25 | Linux CI and automated tests | ● cargo test | ● headless templates, GUT | ◐ heavy containers |
| 26 | Community and learning resources | ◐ | ● | ● |
| 27 | Shipped MMO or large multiplayer title | ○ none | ○ none at MMO scale | ● Aion 2, Dune Awakening, Throne and Liberty |
| 30 | Integration effort with our Rust server | ● ~1 week | ◐ desktop low, web high | ○ 2-4 weeks plus ongoing FFI |
| 31 | Time to a desktop vertical slice (solo) | 8-12 wk 2D, 16-24 wk 3D | 4-8 wk part-time | months |

## 3. Where Bevy is actually short

Your instinct is right. Against an MMO client's needs, Bevy's gaps are concentrated, not
scattered, and they sit on the critical path:

1. **No editor.** Zone layout, UI layout, and animation state machines are done in code or in
   third-party tools. The official prototypes were archived in April 2026 and the editor MVP is
   the stated top priority for the 2026-27 year, which means it is not available for this
   project's first year.
2. **UI without reactivity.** An MMO client is mostly UI: inventory grids, drag and drop, rich
   text chat, quest logs, trade windows, auction search. `bevy_ui` has none of data binding,
   drag and drop, rich text, or localization. Every shipped Bevy game of note either uses egui
   for anything data-heavy or wrote its own.
3. **Breaking changes every release.** Roughly 95 migration entries from 0.18 to 0.19, and
   ecosystem crates lag by weeks. The maintainers do not give a production-ready timeline.
4. **No world streaming, HLOD, terrain, or animation state machine in core.** Plugins cover each,
   but each is maintained by one person.
5. **No shipped MMO.** Tiny Glade and Jarl both forked the engine and replaced parts of it.

What Bevy has that nothing else does: the domain crate, prost messages, and any shared
simulation code compile into the client unchanged. That is a real advantage, but it saves
about a week of integration, while the gaps above cost months.

## 4. Why Unreal is the wrong tool here

Unreal's rendering, animation, and world tooling are the best available and it has shipped
real MMOs. It still loses for this project on three counts that no feature can offset:

- **Its multiplayer stack assumes a UE dedicated server.** Replication, Iris, GAS prediction,
  and the Character Movement Component all fight a Rust-authoritative design. The client would
  use none of them and work around their assumptions.
- **The solo-developer iteration loop is the worst of the three.** C++ compiles, fragile Live
  Coding in 5.8, a 12-16 core and 32 GB baseline, and recurring GPU device-removed crashes.
- **No browser path at all**, and UE6 with Verse will make today's Actor and Blueprint code
  legacy by 2028. Rust reuse is FFI-only; `unreal-rust` is dead.

## 5. Recommendation: Godot 4 with Rust via gdext for the desktop client

Godot covers 24 of the 32 rows natively and stably, including the two where Bevy is weakest
(editor, data-heavy UI). The specific shape:

- **Godot 4.7 (pin per milestone), Jolt physics, Forward+ renderer on desktop.**
- **One Rust `cdylib` crate built with gdext 0.5** that depends on a new `nightfall-domain`
  crate extracted from `apps/api/src/domain`. It owns the WebSocket, protobuf decoding, the
  snapshot buffer and interpolation math, and exposes a `NetClient` node plus typed signals.
  Protobuf never touches GDScript.
- **GDScript for scenes, UI, and animation.** This is where Godot's editor pays for itself and
  where Bevy would cost the most time.
- **Web as a stretch target, not a plan.** Godot's web export is WebGL2-only and the Rust path
  needs nightly Rust and emscripten. If a browser client matters, keep the Phaser prototype as
  the thin browser client against the same server, which is the arrangement you already have.
- **Zone-based world design**, which Lineage 2 already is. Each hunting zone is a scene loaded
  with `load_threaded_request`. Do not attempt continent-scale streaming in Godot.

### Risks and how to retire them early

| Risk | Test to run in the first two weeks |
|------|------------------------------------|
| Character count at scale | Spawn 300 animated `Skeleton3D` characters with LOD in one zone; measure frame time on target hardware. |
| gdext churn | Pin `godot` 0.5.x and Godot 4.7.x together; upgrade both only at milestone boundaries. |
| gdext on web | Decide now that web is stretch. Do not spend time on the emscripten build until desktop ships. |
| No shipped MMO on Godot | Mitigated by the server owning the simulation; the client is a renderer. Keep it thin. |

### When Bevy would be the right call instead

- If the client stays **2D or isometric** for a long time: `bevy_ecs_tilemap` plus egui gets to a
  slice in 8-12 weeks, the UI gap hurts less, and the shared-crate advantage is pure upside.
- If sharing **simulation code** between client and server becomes central (client-side
  prediction of combat math, for example). Godot via gdext can still do this, with one FFI
  boundary.
- If the editor MVP ships and `bevy_ui` gains reactivity. Revisit in 12 months.

### Decision needed

Confirm Godot plus gdext for the desktop client, and whether to keep Phaser as the browser
client. Once confirmed: extract `nightfall-domain` into `packages/domain`, scaffold
`apps/client-godot` with the gdext crate, and run the two-week risk tests above.
