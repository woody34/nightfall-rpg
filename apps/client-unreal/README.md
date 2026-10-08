# Nightfall Unreal Client

Unreal Engine 5 thin client for the Rust server. Decision and reasoning:
`docs/research/engine-comparison.md` §0.

## Principles

1. **The server owns the game.** The client renders state and sends intents. No UE replication,
   no GAS, no dedicated server, no `NetDriver`. `OnlineSubsystem` and `GameplayAbilities` are
   disabled in the `.uproject`, and `DefaultEngine.ini` clears the net driver definitions.
2. **One place touches bytes.** `UNetClientSubsystem` owns the WebSocket and the protobuf codec.
   Everything else uses typed structs and delegates.
3. **Interpolate, never extrapolate.** Remote entities render 150 ms behind server time from
   `FSnapshotBuffer`. The local pawn previews its own moves and is corrected by the server.
4. **Same contract as the server.** Field numbers in `Net/ProtoCodec.h` are the ones in
   `packages/proto/nightfall/v1/world.proto`. Change the proto first, then both sides.
5. **Warnings are errors** (`bWarningsAsErrors` in `Nightfall.Build.cs`), as on the Rust side.

## Layout

```
Nightfall.uproject
Config/                      Engine, game, input settings
Source/Nightfall/
  Nightfall.{h,cpp}          Module + log category
  Net/ProtoCodec.{h,cpp}     Wire structs + protobuf encode/decode (hand-written minimal codec for now)
  Net/SnapshotBuffer.{h,cpp} Per-entity position ring with time-delayed interpolation
  Net/NetClientSubsystem.*   GameInstance subsystem: WebSocket, reconnect, seq/ack, clock offset
  World/RemoteEntityActor.*  Visual proxy for a server entity; Blueprint subclass adds the mesh
  World/WorldProxySubsystem.* Spawns/destroys proxies from server events
  NightfallPlayerController.* Click-to-move: raycast, local preview, MoveTo intent
  Generated/                 protoc C++ output (Scripts/gen-proto.sh), unused until protobuf-lite is vendored
Scripts/gen-proto.sh
```

## Installing Unreal on Linux

Epic requires an account. Two routes:

- **Prebuilt binaries (fastest):** sign in at <https://www.unrealengine.com/linux>, download
  `Linux_Unreal_Engine_5.8.x.zip`, unzip to `~/UnrealEngine`, run
  `~/UnrealEngine/Engine/Binaries/Linux/UnrealEditor` once to let it finish setup.
- **From source:** link your GitHub account to Epic
  (<https://www.unrealengine.com/ue-on-github>), clone `EpicGames/UnrealEngine` at the `5.8`
  branch, then `./Setup.sh && ./GenerateProjectFiles.sh && make`. Budget 1-3 hours and ~200 GB.

Then:

```bash
export UE_ROOT=~/UnrealEngine
sudo apt install -y clang lld            # UBT uses the engine's bundled clang, but the system one helps tooling
```

## First build

```bash
cd apps/client-unreal
moon run client-unreal:build-editor      # compiles the Nightfall module
moon run client-unreal:editor            # opens the project
```

In the editor, create a Blueprint subclass of `RemoteEntityActor` with a skeletal mesh, set it as
`EntityClass` on the world proxy subsystem (a config actor is the next step), create an Input
Mapping Context with a `ClickMove` action bound to left mouse, and assign both on a Blueprint
subclass of `NightfallPlayerController`.

## First vertical slice (in order)

1. **Connect.** Start the API with `moon run api:dev`, open the editor, call `Connect` on the net
   subsystem with `ws://localhost:3000` and a ticket. Requires the server's `/ws` endpoint
   (Phase 0 §3.2), which is the next server task.
2. **See others move.** Server emits `EntitySpawn`/`EntityMove`; `WorldProxySubsystem` spawns
   proxies that interpolate.
3. **Click to move.** `NightfallPlayerController` sends `MoveTo`; the server echoes our own
   entity's `EntityMove`.
4. **Login over gRPC.** Add a gRPC module (TurboLink or raw grpc++) for `GameService` and the
   auth ticket. Until then, the ticket is a dev constant.
5. **HUD.** CommonUI + MVVM: target frame, HP/MP/CP bars, chat log fed by `SystemMessage` ids.
6. **Replace the hand-written codec** with protoc output plus vendored protobuf-lite once a
   second message family (combat) arrives. Field numbers do not change.

## Not verified yet

This scaffold was written without an Unreal install on the authoring machine. Expect a handful
of compile fixes on first build (include paths, API renames in 5.8). The codec, buffer, and
subsystem logic are straightforward and were written against the documented 5.x APIs.
