# Engine research: Bevy (Rust)

| | |
|---|---|
| Engine | Bevy (bevyengine/bevy), MIT OR Apache-2.0 |
| Latest stable release when we wrote this document | **0.19.1** (0.19.0 was released on 2026-06-19; 0.19.1 is the current patch on crates.io) |
| Previous releases | 0.18 (2026-01-13), 0.17 (2025-09-30), 0.16 (2025-04-24) |
| Research date | 2026-10-07 |
| Researcher | Claude (agent), for the Nightfall decision about the client engine |
| Matrix | 32 rows from `scratchpad/engine-matrix.md` |

Status legend: **Native** = in the engine; **Plugin** = third-party crate; **Partial** = available, but not complete for our use; **Missing** = nothing usable.
Maturity legend: stable / beta / experimental / abandoned (this applies to the specified item, not to Bevy as a whole).

A note on sources: many search engines show one blog post ("Bevy Rust game engine 2026 indie guide", strayspark.studio). It says that 0.18 was released in March 2026 with an "editor preview". This is not in agreement with bevy.org and with the archived `bevy_editor_prototypes` repo. On bevy.org, 0.18 was released on 2026-01-13, and the August 2026 birthday post still lists the editor as a *next-year priority*. Thus, I think that the post is not reliable, and I do not use it below.

## Feature matrix

| # | Feature | Status | What / plugin | Maturity | Note |
|---|---------|--------|---------------|----------|------|
| 1 | Version, cadence, breaking changes, LTS | Native | Bevy 0.19.1; minor releases each ~quarter to 5 months (0.16 Apr 2025, 0.17 Sep 2025, 0.18 Jan 2026, 0.19 Jun 2026); 0.x semver, each minor release has breaking changes | beta | No LTS, and no promise of stability. The 0.18→0.19 migration guide has ~95 entries. These include "Resources as components", render graph → systems, cosmic-text → parley, and `bevy_scene` renamed `bevy_world_serialization`. In GH discussion #21911, the maintainers say that there is no timeline for a production-ready release, and API stabilization "may never" occur. The community tells you to pin one version for each project and to upgrade between milestones. |
| 2 | Gameplay language; use of our Rust domain crate again | Native | Rust only; you can use all crates of the workspace in the client | stable | This is the strongest argument for Bevy here. Our `domain` crate, the protobuf types that prost generates, and the shared sim code compile into the client with no changes. This is true for native and wasm32, if the dependencies are clean for wasm. ECS components can be newtype wrappers over the domain types. |
| 3 | Editor / inspector / asset browser / visual debugging | Partial | Official editor: **not released**. `bevy_editor_prototypes` was archived on 2026-04-16; "Bevy Editor MVP upstream" is the #1 stated priority for the 2026-27 year. Community: `jackdaw` (Bevy 0.19, "very early in dev", live inspector that uses BRP, .bsn scene saving), `bevy_granite`, `bevy-inspector-egui`/`bevy_editor_pls` (last update Apr 2025). In the engine: Feathers widgets, interactive transform gizmo, infinite grid, text gizmos, diagnostics overlay (0.19), FreeCamera/PanCamera, EasyScreenshot (0.18), frame-time graph (0.17) | experimental | In practice, you must build the world and zone tools yourself (BSN files + jackdaw + egui inspector). Or you make the levels in Blender/Tiled and import them. The parts for the editor arrive quickly, but there is no integrated scene editor now. |
| 4 | Hot reload (code + assets) | Native / Partial | Assets: `AssetPlugin { watch_for_changes_override }` with the `file_watcher` feature, stable since 0.12. Code: "hot patching systems" with Dioxus `subsecond` + `dx serve --hotpatch` was merged in 0.17 (experimental; it patches the bodies of systems, not the type layouts) | assets stable, code experimental | Code hot-patching is real, but it has limits. If you add components or resources, or change struct layouts, you must do a full rebuild. Use it together with the `dynamic_linking` feature for incremental builds of ~1-3 s. |
| 5 | Licensing / cost | Native | MIT OR Apache-2.0; no royalties, no store fees, full source; the Bevy Foundation is a 501(c)(3) that donations fund (2024 budget: one full-time maintainer at ~$150k) | stable | Zero cost and zero lock-in. The disadvantage is the opposite: there are very few paid persons, thus the roadmap is frequently late. |
| 6 | 3D: PBR, skeletal anim, blending, state machines, IK, retargeting | Native + Plugin | PBR `StandardMaterial` (validated with the white-furnace test in 0.19), GPU skinning, `AnimationGraph` DAG with blend + additive nodes, animation masks, animation events (0.14-0.15), `TryStableInterpolate` (0.18). **No** built-in state machine, blend space, IK or retargeting. `bevy_animation_graph` (mbrea-c, supports 0.19) adds state machines, nested graphs, IK nodes, ragdoll, event tracks and a visual graph editor | core stable; state machines/IK only as a plugin, beta | For MMO character animation (locomotion blend by speed and direction, attack layers for the upper body, hit reactions), you will depend on `bevy_animation_graph`. Or you write a state machine by hand over the `AnimationGraph` weights. You must do the retargeting yourself. 0.19 corrected the culling of skinned meshes: it uses the actual joint positions, thus limbs do not disappear during a swing. |
| 7 | 2D / 2.5D: sprites, tilemaps, isometric, sprite anim, 2D lighting | Native + Plugin | Sprites/atlases/Text2d native; built-in chunked tilemap renderer (0.17, foundation only); `bevy_ecs_tilemap` (compatible with 0.19, iso + hex, Tiled/LDtk loaders); `bevy_sparse_tilemap` for 15k×15k maps; 2D lighting with the `bevy_light_2d` / `bevy_2d_lighting` ecosystem | sprites stable; tilemap plugin stable; 2D lighting beta | This is sufficient to copy the look of the current Phaser prototype 1:1 if we stay 2D/iso. You must do the isometric depth sort yourself (a Y-sort system is a few lines). |
| 8 | Large open world: streaming, LOD, HLOD, impostors, occlusion, terrain | Partial | Native: `VisibilityRange` LOD + dithered crossfade (0.14), two-phase GPU occlusion culling (0.16), GPU-driven rendering + BVH culling (0.16/0.17), virtual geometry (meshlets, `bevy_pbr::experimental`), 0.19 "render big scenes faster" (2.6x on culled scenes; `bevy_city` 55k entities @60fps). Streaming: **none built in**; `big_space` (floating origin, i128 grids) for precision; you must do the chunk load/unload yourself with async asset loading + `AssetServer`. Terrain: `bevy_terrain` (UDLOD clipmaps, research quality), `bevy_mesh_terrain`; no HLOD, no impostors | LOD/culling beta; streaming/terrain experimental | A continuous world at the scale of Lineage 2 means that we must write our own zone/chunk streamer (spawn/despawn scenes for each grid cell). This is possible in ECS, but it is engine work, not game work. |
| 9 | Many characters: instancing, GPU skinning, crowds, modular customization | Native | Automatic instancing/batching of the same mesh+material, GPU skinning, GPU-driven rendering with bindless (partial, on Metal only with texture arrays in 0.19). Character customization = many skinned child meshes that share one `AnimationPlayer`/skeleton + material swaps (by hand) | beta | There is no "crowd" system, and no LOD for skinned meshes or for animation, in the default engine. Hundreds of skinned characters are not a problem. Thousands in one siege need custom animation culling/LOD. Modular armor swaps are a known pattern, but there are no asset tools for them. |
| 10 | VFX: GPU particles, trails, decals, post-processing | Native + Plugin | `bevy_hanabi` 0.19 (GPU particles, ribbons/trails, WASM only with WebGPU); decals native (forward + clustered, 0.16); post: bloom, TAA, SSAO, SSR (physically based 0.19), contact shadows, vignette, lens distortion, chromatic aberration, DLSS (0.17), motion blur | hanabi stable; post stable | Hanabi is the only serious particle system. It does not run on WebGL2. The many spell VFX of an MMO are possible on native and WebGPU targets. |
| 11 | Lighting: dynamic GI, shadows, day/night | Native | Cascaded + point/spot shadow maps, light probes/env maps (realtime-filtered 0.17), procedural atmosphere + sun disk + raymarched sky (0.16-0.18), **Solari** raytraced GI (experimental, RT hardware is necessary), rectangular area lights (0.19). No toolchain for baked lightmaps in the engine (lightmap *sampling* is available; the baking is external, for example the Blender/`bevy_lightmapper` ecosystem) | shadows/atmosphere stable; GI experimental | Our 4h day cycle is the same as a rotation of the `DirectionalLight` under the procedural atmosphere. There is no dynamic GI for hardware that is not RTX, thus plan to use SSAO + probes. |
| 12 | Physics, character controller, navmesh, LOS | Plugin | `avian` 0.7 (Bevy 0.19; 2D/3D, Parry colliders, spatial queries, CCD, joints, f64 option) or `bevy_rapier` (now maintained in the dimforge/rapier monorepo); `bevy_tnua` floating character controller; navmesh: `vleue_navigator` (Polyanya, active Mar 2026), `oxidized_navigation` (runtime generation of recast-style tiles, avian/rapier backends); LOS = raycast with avian spatial queries | avian beta-stable; tnua stable; navmesh beta | Bevy has none of these built in. A client that talks to an authoritative server mostly needs raycasts (click-to-move, LOS) and a kinematic controller for local prediction. These are all available. The physics of Avian is not the same as our server sim. This is not a problem, because the server is responsible for the truth. |
| 13 | UI toolkit | Partial | `bevy_ui` (flexbox/grid with taffy, scroll with sticky headers, gradients, borders for each side, `ViewportNode`); headless core widgets 0.17 (Button, Slider, Checkbox, Radio, Scrollbar); **Feathers** widgets 0.17-0.19 (buttons, sliders, dropdowns, popover, menu, color picker, text input, number input, list view, panes) ported to BSN; `EditableText` with IME/bidi/selection/clipboard (0.19); font families, variable fonts, letter spacing, OpenType features (0.18-0.19); directional gamepad nav (0.18). Alternatives: `bevy_egui` 0.41 (immediate-mode, Bevy 0.19), `bevy_lunex` (retained layout, 0.19). **No** data binding/reactivity, no built-in drag-and-drop, no rich-text markup, no localization | beta | This is the weakest area for an MMO. You can build inventory grids, action bars, chat with rich text, quest logs and auction UIs. But there is no reactivity, thus you must write the change-detection systems by hand. BSN (0.19) helps with composition, but not with binding. The style of Feathers is explicitly "editor tooling". Talin (the lead for bevy_ui) says that the current bevy_ui is like "writing a web page entirely in JavaScript". Many Bevy games use egui for panels with much data. Localization: `bevy_fluent` (0.13 for Bevy 0.17; it is late). |
| 14 | Audio: spatial, crossfade, buses, streaming | Native (basic) + Plugin | `bevy_audio` (rodio): play/sink/spatial pan, no mixer graph. `bevy_kira_audio` 0.26 (Bevy 0.19): channels, fades/tweens, limited spatial. `bevy_seedling` 0.8 (Firewheel graph backend, the planned first-party replacement; "Firewheel audio" is in the 0.17 what's-next list, but it is not upstream in 0.19) | bevy_audio stable but thin; kira stable; seedling beta | Zone-music crossfade and ambient buses operate with kira channels now. Jarl uses FMOD. This shows that studios do not use the built-in stack. |
| 15 | Input: rebinding, gamepad, click-to-move picking | Native + Plugin | Native gamepad/keyboard/mouse/touch, `ButtonInput<Key>` (0.17); **`bevy_picking`** native since 0.15 with mesh picking backend (0.16) and UI picking; rebinding with `bevy_enhanced_input` 0.26 (Bevy 0.19, contexts in the UE style) or `leafwing-input-manager` 0.20 (Bevy 0.18) | stable | Click-to-move on terrain = `MeshPickingPlugin` or an avian raycast from `Camera::viewport_to_world`. You must still build the widget for the rebinding UI. |
| 16 | Scripting / modding, data-driven content | Partial | Data: BSN `.bsn` scenes in code (0.19; the loader for asset files is the top item of the 0.20 roadmap), RON/serde assets with `Reflect`, with hot reload. Scripting: `bevy_mod_scripting` 0.19 (Bevy 0.18+; Lua 5.1-5.4/LuaJIT/Luau, Rhai; hot loading; **no WASM**) | BSN experimental; scripting beta | For our item, skill and quest data, the usual path is serde+RON/JSON assets that `AssetServer` loads. These get hot reload at no cost. Lua modding is possible natively, but not on the web target. |
| 17 | WebSocket client, protobuf, gRPC-web | Plugin (crates that are not Bevy crates) | `prost`/`tonic` native (same codegen as the server). Native: the `tonic` client with no changes; wasm: `tonic-web-wasm-client` 0.9.2 (Sep 2026, tonic ^0.14; unary + server-streaming over fetch). WebSocket native+wasm: `tokio-tungstenite-wasm`, `ewebsock`, `bevy_nfws` (Bevy wrapper), `bevy_simplenet`, `aeronet_websocket`. Async bridge: `bevy_tokio_tasks` 0.19 or the `IoTaskPool` of Bevy + channels (`crossbeam`/`async-channel`); 0.19 made web tasks cancellable | stable | All of this is plain Rust that we already use on the server side. The integration is a plugin of ~300 lines: spawn a task that controls the socket, decode the protobuf frames, push them into a `Messages<ServerEvent>` buffer, and drain them in `PreUpdate`. The engine does not work against us here. |
| 18 | Built-in multiplayer stack | Missing (by design) | None in core. Ecosystem: `lightyear` 0.30 (Bevy 0.19; full client-server, prediction/rollback, WebTransport/WebSocket/Steam/UDP with aeronet) and `bevy_replicon` 0.44 (Bevy 0.19; world replication, transport-agnostic, "server need not be Bevy") | n/a | The two assume an ECS world like Bevy on the server to replicate. In practice, lightyear is a full stack. Our server is axum/tonic/NATS and it is already authoritative, thus we would **not** use either of them. It is an advantage that there is no opinionated built-in stack. |
| 19 | Prediction / interpolation / snapshot buffers | Partial | No core helpers. `lightyear` has prediction, rollback, snapshot interpolation, input delay and lag compensation, but it is coupled to its own transport/replication. `bevy_rewind` (rollback), `bevy_transform_interpolation`, `bevy_tweening` 0.16 (Bevy 0.19) for cosmetic smoothing. `FixedUpdate` + `Time<Fixed>` with transform interpolation is native (avian has `TransformInterpolation`) | partial / beta | We expect to write our own snapshot ring buffer + entity interpolation + local-player prediction over `FixedUpdate`. Because we use the shared domain crate again, client-side reconciliation of the server state is possible with the same types. |
| 20 | Headless / server mode | Native | `MinimalPlugins` / `App` without window+render; 0.17 decoupled the cameras/lights/meshes/UI APIs from `bevy_render`; `no_std` support (0.16); you can use `bevy_ecs` alone | stable | We could run `bevy_ecs` in the axum server for the zone simulation (shared systems between client prediction and server). This is optional, but it is a real option that no other candidate engine gives. |
| 21 | Export targets | Native | Windows, macOS, Linux (first-class); Web with wasm32 (WebGPU and WebGL2 backends); Android/iOS "initial support" with examples and community CI templates (`bevy_game_template`, `bevy_ios_app_delegate`); consoles: **none** (wgpu has no NDA console backends) | desktop stable; web almost stable; mobile beta; console missing | The engine fully satisfies the requirement for native desktop. Tiny Glade was released on Win/Linux and macOS (May 2026). Consoles are not possible, and this agrees with the scope of Nightfall. |
| 22 | Web export quality | Partial | WebGPU (Chrome/Edge, Safari 26+, Firefox 141+) or WebGL2 fallback (no Hanabi, no compute, less post-fx); **single-threaded** on wasm (no `SharedArrayBuffer` threading in Bevy; systems run one after the other); binaries are frequently 10-30 MB raw, ~5-10 MB brotli with `wasm-opt`+`opt-level="z"`; `bevy_cli` (alpha, "web"/"web-release" profiles, `bevy build web --bundle`); audio must have a user gesture (browser rule); http(s) asset loading native since 0.17 (it includes `bevy_web_asset`, which is now archived) | beta | This is possible for a lightweight browser client. But a full 3D MMO client in wasm with a single-threaded ECS and no compression pipeline for streamed assets is difficult. A realistic plan: native desktop is the primary target, and web is a secondary target with less fidelity. |
| 23 | Asset pipeline | Partial | glTF 2.0 import (with extension hooks 0.18, forward-axis config 0.17); **no FBX**; KTX2/Basis/zstd textures load natively; an asset *processor* (`AssetProcessor`, `.meta` files) is available, but there are few released processors; `bevy_basisu_saver` community; `meshopt` crate for meshlet preprocessing; `bevy_asset_loader` (0.19) for loading states/collections; seekable readers (0.18); asset saving API (0.19); a rewrite "Assets as Entities" is planned | glTF stable; processing pipeline experimental | Blender → glTF is the only sensible path. Texture compression is a build step that you must do yourself (toktx/basisu), not an import setting of the engine. There is no format for asset bundles or packs, other than "ship the assets folder" or a custom `AssetReader`. |
| 24 | Build times / iteration | Partial | A clean debug build of a Bevy app: ~1-3 min on a modern desktop; incremental with `dynamic_linking` + lld/mold: ~1-5 s; the `cranelift` nightly backend is ~12-30% faster; hot-patching (0.17) for changes to system bodies | beta | The Rust compile time is the cost. With the recommended setup, iteration is good. The problem is each quarterly Bevy upgrade (hours to days of migration for a project of medium size). |
| 25 | CI friendliness | Native | `cargo test` on `bevy_ecs`/`MinimalPlugins` is pure Rust; headless rendering with `RenderPlugin` with `WgpuSettings` on lavapipe/llvmpipe in Linux CI (the CI of Bevy does screenshot tests this way); `bevy_cli` lint; `bevy_game_template` has GitHub Actions for all targets | stable | The toolchain is the same as for our server CI (cargo, clippy, nextest). You can do unit tests of gameplay logic in plain systems without a window. |
| 26 | Community, docs, learning, assets | Partial | 47.5k GitHub stars, 1,527 contributors, 23.4k Discord members, 6.66M crates.io downloads (Aug 2026). Official docs: Quick Start book + examples (the "New Bevy Book" still gets larger); the unofficial Bevy Cheat Book is partly out of date; the migration guides are excellent. Third-party asset stores: none that are specific to Bevy (use glTF from Sketchfab/Quaternius/Kenney and other sources) | beta | The community is very active. But the knowledge base becomes out of date quickly because of the breaking changes: most blog posts and LLM answers are for the 0.12-0.15 APIs. There are fewer ready-made systems than in Godot/Unity. |
| 27 | Released MMO / large multiplayer titles | Missing | **No released MMO.** Released: Tiny Glade (Pounce Light, Sep 2024, custom renderer on Bevy ECS; BAFTA Technical Achievement nominee; macOS May 2026), Toroban (Dec 2025), Wee Boats, Gob Johnson's Downhill Marmalade, Insanio, Court Wizard (from the Bevy Foundation). In development: Jarl (solo dev, changed Bevy + custom "Alloy" editor, FMOD audio, closed playtests), Exofactory (Replicon multiplayer, Q3 2026), MineWars (IyesGames) | n/a | Nobody released a game at the scale of Lineage 2 on Bevy. The two flagship titles both forked Bevy and wrote their own tools. This is the honest sign of what "production Bevy" is now. |
| 28 | Known gaps / pain points 2025-26 | — | (a) no editor (archived prototypes, MVP moved to 2026-27); (b) breaking changes in each release ("absolutely caustic" upgrade cycle, r/rust "Migrating away from Rust" thread Apr 2025; "Architect of Ruin" left Bevy); (c) UI ergonomics and no reactivity; (d) the docs are late and LLMs know less about Bevy than about Unity/Godot; (e) compile times; (f) the framing "engine for people who want to tinker"; (g) thin audio; (h) no asset pipeline/bundles | — | The maintainers agree that all of these are problems. The 2026-27 priorities (editor MVP, assets-as-entities, `.bsn` loader, reactivity convergence) agree exactly with them. |
| 29 | Trajectory for the next 12 months | — | Stated for 0.20+: `.bsn` asset loader, Bevy Editor MVP upstream, Assets as Entities, unified 2D/3D rendering internals, entity inspector, WESL shader language, reactivity ecosystem convergence, expanded Book, Firewheel audio. The cadence gives 0.20 at ~Nov 2026 and 0.21 at ~Mar/Apr 2027 | — | The momentum is strong (261 contributors / 1,185 PRs in 0.19). But the editor was late many times since the 2024 roadmap. Plan as if the editor arrives in alpha in 2027, not before. |
| 30 | Integration effort with our Rust server | Native | Shared workspace: the `domain` crate + the `prost` types + the tonic client compile into the client. WebSocket + protobuf framing plugin ~1 week. gRPC-web: tonic native / `tonic-web-wasm-client` on wasm behind a trait. NATS is internal to the server and is not related to the client side | stable | This is the lowest integration cost of all candidates: zero FFI, zero schema duplication, one toolchain, one CI. This is the primary reason to select Bevy. |
| 31 | Time to vertical slice for a team of one | Partial | Estimate: 2D/iso slice (login, zone load, click-to-move, combat, chat, inventory) ~8-12 weeks with `bevy_ecs_tilemap`+`bevy_egui`; 3D slice with skeletal characters, Avian raycasts, Hanabi VFX, bevy_ui HUD ~16-24 weeks, with ~2 more weeks for each Bevy minor upgrade | — | This is productive if you accept egui for panels with much data and do the polish of the UI later. It is slower than Godot for all work that an editor helps with (zone layout, UI layout, animation state machines). |
| 32 | Top 3 risks | — | 1) **UI**: inventory/trade/quest/auction UIs without reactivity or an editor use the most time, and they are the most probable cause of burnout for a solo dev. 2) **Breaking-change treadmill**: ~95 migration entries for each release, ecosystem crates are 1-8 weeks late, the knowledge base becomes out of date; you must pin and do upgrades in batches. 3) **Gap in large-world + animation tools**: no world streaming, HLOD, animation state machine or retargeting in core; the plugins are available, but individuals maintain them (bevy_animation_graph, big_space, vleue_navigator), and they can become dormant | — | Mitigations: egui for tools and complex panels, bevy_ui only for the HUD; pin Bevy for each milestone; design zone streaming as our own crate from the first day. |

## Narrative

### Where Bevy is in October 2026

Bevy 0.19 (June 2026) is the largest release of the engine until now. It has 261 contributors and 1,185 PRs. It added these items:
- The BSN scene system (`bsn!` macro), which was promised for a long time.
- Real text input.
- A large step in GPU-driven rendering (55k entities at 60 fps in the `bevy_city` example).
- A diagnostics/gizmo layer that finally looks like developer tools.

The three releases before it (0.16-0.18) added occlusion culling, GPU-driven rendering, procedural atmosphere, decals, ECS relationships, and the observer/event overhaul. They also added headless core widgets, the Feathers widget set, raytraced lighting (Solari), code hot-patching and first-party http asset loading. The project has 47.5k stars, 1,527 lifetime contributors and a Discord with 23k members. Technically, it is not a toy renderer now. For pure-Rust code, the ECS is the best in the industry, and the renderer is modern (wgpu, bindless, meshlets, DLSS).

The editor has *not* arrived. The `bevy_editor_prototypes` repository was archived in April 2026 with a pointer to the community `jackdaw` project. The August 2026 birthday post lists "Bevy Editor MVP upstream" as the top priority for the next year. In GitHub discussion #21911, a maintainer says that there is no timeline for a production-ready release. The maintainer also says that API stabilization in the Unity sense can possibly never occur. Each minor release has breaking changes. The 0.18→0.19 guide has approximately 95 entries, with fundamental changes: for example, resources became components and the render graph became systems. The ecosystem crates now follow the releases with discipline (avian, hanabi, kira, egui, tilemap, tweening, asset_loader, tokio-tasks, lightyear, replicon all released versions compatible with 0.19). But the delay and the total quantity of change are real.

### Strengths for Nightfall

The most important strength is that our stack is already Rust. The server is axum + tonic + NATS with a domain crate and protobuf that prost generates. In Bevy, each of those types compiles into the client with no changes, on native and wasm32. The WebSocket-plus-protobuf transport is a small plugin around `tokio-tungstenite-wasm` (or `ewebsock`). The gRPC-web client is tonic natively and `tonic-web-wasm-client` 0.9 in the browser. `bevy_tokio_tasks` or the task pools of Bevy connect async code to the ECS. There is no FFI boundary, no second language, no schema duplication, and there is one CI pipeline. If we want to share actual simulation systems (movement validation, combat formulas), `bevy_ecs` runs headless in the server process. The 0.17 decoupling from `bevy_render` and the 0.16 `no_std` work make this clean. No other candidate engine gives this.

Second, the rendering core is better than what an MMO needs, from isometric to 3D of medium fidelity. It has PBR, GPU skinning, instancing, `VisibilityRange` LOD, and two-phase occlusion culling. It also has procedural atmosphere for our 4-hour day cycle, decals, bloom/SSAO/SSR, and `bevy_hanabi` for GPU spell VFX. These parts are sufficiently mature for the client-side needs of a game with an authoritative server:
- Physics (avian 0.7).
- Character control (tnua).
- Navmesh (vleue_navigator / oxidized_navigation).
- Picking (native since 0.15).
- Input contexts (bevy_enhanced_input).

Third, Bevy has no opinionated multiplayer stack in core, and `lightyear` and `bevy_replicon` are both optional. This is exactly correct for a custom authoritative server: nothing works against us.

### Weaknesses for Nightfall

UI is the largest problem. `bevy_ui` has layout, scrolling, gradients, text input (0.19), variable fonts and gamepad navigation. Feathers gives a widget set in a tooling style. But there is no data binding or reactivity. Each inventory slot, buff icon, quest tracker and chat line is a change-detection system that you write by hand. Talin, the lead of the UI work, compares the current bevy_ui to writing a web page entirely in JavaScript. Reactivity "convergence" is still a roadmap item. Most Bevy games with UIs that show much data use `bevy_egui`. That operates, but it looks like a developer tool, and it is a second UI paradigm that you must maintain. Localization is a community crate that is late (`bevy_fluent` follows 0.17). Rich text is spans with a style for each span, with no markup.

Big worlds are a second gap. Culling and LOD are good, but core has no world/zone streaming, no HLOD, no impostors, and no terrain system. One person maintains each of `big_space` and `bevy_terrain`. A seamless continent like Lineage 2 means that we must write a chunk streamer ourselves. Animation is similar. The core `AnimationGraph` does blends and masks, but state machines, blend spaces, IK and retargeting are in `bevy_animation_graph`, a project with one maintainer.

The breaking-change treadmill makes all of these problems worse. A project of medium size loses days for each release. Blog posts and LLM suggestions are for old APIs. A plugin that you depend on can skip a release. The community consensus (pin one version for each milestone, upgrade between phases) is possible, but it means that you intentionally stay one release behind.

Finally, nobody released an MMO on Bevy. The flagship titles, Tiny Glade and Jarl, both forked the engine and built their own editors (the "Alloy" editor of Jarl) and audio (FMOD). That is the realistic form of "production Bevy" in 2026: an excellent ECS and renderer kernel that a team puts inside its own tools.

### Fit verdict

Bevy is the best technical fit to share code with our Rust server. It is also the best engine for a programmer who wants to be responsible for each system. It is a poor fit for the parts of an MMO that need tools: UI with much data, world authoring, and animation state authoring. For a team of one, those parts decide if a vertical slice is released. Nightfall can stay 2D/isometric for its first playable version (with the look of the Phaser prototype). In that condition, Bevy with `bevy_ecs_tilemap` and `bevy_egui` is a defensible decision, with approximately a quarter to a slice and a very cheap network layer. The goal can be a 3D client in the next year, with skeletal characters, a continuous world and a polished inventory/auction UI. In that condition, the gaps of Bevy are exactly on our critical path. In that condition, an engine with an editor at its center will get to a playable slice faster, although it has an integration cost. Examples are Godot with Rust GDExtension, or Unity/Unreal with a thin gRPC-web/WebSocket bridge. If we select Bevy, do these steps:
- Pin 0.19.x for all of the first milestone.
- Use egui for each panel that is not part of the HUD.
- Make zone streaming our own crate from the first day.
- Plan for one Bevy upgrade each quarter.

## Sources

Official Bevy
- https://bevy.org/news/ (release index; dates for 0.16-0.19 and birthday posts)
- https://bevy.org/news/bevy-0-19/ (0.19 release notes, 2026-06-19)
- https://bevy.org/news/bevy-0-18/ (0.18 release notes, 2026-01-13)
- https://bevy.org/news/bevy-0-17/ (0.17 release notes, 2025-09-30)
- https://bevy.org/news/bevy-0-16/ (0.16 release notes, 2025-04-24)
- https://bevy.org/news/bevys-sixth-birthday/ (2026-08-10; stats, shipped games, next-year priorities)
- https://bevy.org/news/community-reflection-on-bevys-fifth-year/
- https://bevy.org/learn/migration-guides/0-18-to-0-19/ (breaking-change count)
- https://bevy.org/learn/migration-guides/0-14-to-0-15/ (animation masks / additive blending)
- https://bevy.org/foundation/budget/ (Foundation budget and staffing)
- https://bevy.org/examples/animation/animation-graph/
- https://bevy.org/examples/ui-user-interface/feathers-gallery/
- https://bevy.org/examples/scene/bsn/
- https://bevy.org/news/bevy-webgpu/
- https://bevy.org/learn/quick-start/getting-started/setup/ (dynamic_linking, fast compiles)
- https://dev-docs.bevyengine.org/bevy/pbr/experimental/meshlet/struct.MeshletPlugin.html
- https://github.com/bevyengine/bevy_editor_prototypes (archived 2026-04-16)
- https://github.com/bevyengine/bevy/discussions/21911 (production-readiness discussion)
- https://github.com/bevyengine/bevy/issues/318 (wasm threading)
- https://github.com/bevyengine/bevy/pull/20158 (BSN)

Editor / tooling
- https://github.com/jbuehler23/jackdaw
- https://gittrend.io/repo/BlakeDarrow/bevy_granite
- https://gittrend.io/repo/jakobhellermann/bevy_editor_pls
- https://thebevyflock.github.io/bevy_cli/

Ecosystem crates
- https://github.com/cBournhonesque/lightyear
- https://github.com/projectharmonia/bevy_replicon
- https://github.com/avianphysics/avian
- https://github.com/dimforge/bevy_rapier (moved into dimforge/rapier)
- https://github.com/djeedai/bevy_hanabi
- https://github.com/NiklasEi/bevy_kira_audio
- https://github.com/StarArawn/bevy_ecs_tilemap
- https://github.com/bytestring-net/bevy_lunex
- https://github.com/NiklasEi/bevy_asset_loader
- https://github.com/johanhelsing/bevy_web_asset (archived; merged into Bevy 0.17)
- https://github.com/mbrea-c/bevy_animation_graph
- https://github.com/EkardNT/bevy-tokio-tasks
- https://github.com/djeedai/bevy_tweening
- https://github.com/makspll/bevy_mod_scripting
- https://docs.rs/crate/bevy_egui/0.41.1/source/README.md
- https://docs.rs/crate/bevy_enhanced_input/0.26.0/source/README.md
- https://docs.rs/leafwing-input-manager/latest
- https://docs.rs/bevy_fluent
- https://github.com/vleue/vleue_navigator/
- https://docs.rs/oxidized_navigation
- https://gittrend.io/repo/idanarye/bevy-tnua
- https://lib.rs/crates/big_space
- https://github.com/kurtkuehnert/bevy_terrain/blob/main/docs/development.md
- https://github.com/NoahShomette/bevy_sparse_tilemap
- https://docs.rs/bevy_basisu_saver
- https://docs.rs/tonic-web-wasm-client/latest/tonic_web_wasm_client/
- https://rust-digger.code-maven.com/crates/bevy_nfws
- https://www.lib.rs/crates/bevy_simplenet
- https://docs.rs/ewebsock
- https://docs.rs/crate/bevy_tween/latest
- https://gittrend.io/repo/NiklasEi/bevy_game_template (mobile/web CI template)
- https://github.com/rustunit/bevy_ios_app_delegate

Released games and community
- https://en.wikipedia.org/wiki/Tiny_Glade
- https://jarl-game.com/
- https://gg.deals/steam/app/1961850/ (Toroban)
- https://www.dekudeals.com/items/exofactory
- https://hackmd.io/@dreamertalin/ryE0polcxx ("When Bevy Was Five", Talin, on bevy_ui limits)
- https://redlib.hackliberty.org/r/rust/comments/1ka4g28/migrating_away_from_rust/ (Apr 2025 thread; "Architect of Ruin" leaving Bevy)
- https://redlib.belloworld.it/r/godot/comments/19889xi/dev_downspiral (Bevy → Godot migration anecdote)
- https://github.com/bevyengine/bevy/issues/254 (early UI complaints)
- https://hackmd.io/@bevy/HkjcMkJFC (Bevy UI design notes)
- https://www.strayspark.studio/blog/bevy-rust-game-engine-2026-indie-guide (consulted; dates contradict bevy.org, treated as unreliable)
