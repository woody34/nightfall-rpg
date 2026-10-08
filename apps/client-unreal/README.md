# Nightfall Unreal Client

Unreal Engine 5 thin client for the Rust server. Decision and reasoning:
`docs/research/engine-comparison.md` §0.

## Principles

1. **The server owns the game.** The client renders state and sends intents. No UE replication,
   no GAS, no dedicated server, no `NetDriver`. `OnlineSubsystem` and `GameplayAbilities` are
   disabled in the `.uproject`, and `DefaultEngine.ini` clears the net driver definitions.
2. **One place touches bytes.** `UNetClientSubsystem` owns the WebSocket, `USessionClient` owns the
   gRPC channel, and all protobuf bytes are produced by generated code. Everything else uses typed
   structs and delegates.
3. **Interpolate, never extrapolate.** Remote entities render 150 ms behind server time from
   `FSnapshotBuffer`. The local pawn previews its own moves and is corrected by the server.
4. **Same contract as the server.** Client code is generated from `packages/proto` (`moon run
   client-unreal:gen-proto`). Change the proto first, regenerate, then both sides.
5. **Warnings are errors** (`bWarningsAsErrors` in `Nightfall.Build.cs`), as on the Rust side.

## Layout

```
Nightfall.uproject
Config/                      Engine, game, input settings ([/Script/Nightfall.NetSettings] = gRPC endpoint)
Plugins/TurboLink/           git submodule, thejinchao/turbolink @ v1.4.2 (gRPC for Unreal)
Source/Nightfall/
  Nightfall.{h,cpp}          Module + log category
  Net/ProtoCodec.{h,cpp}     WebSocket envelope structs (FNet*) <-> generated messages
  Net/NetSettings.h          UNetSettings: gRPC endpoint and call deadline
  Net/SessionClientSubsystem.* USessionClient: GameService over gRPC (Ping, GetCharacter, CreateCharacter)
  Net/SnapshotBuffer.{h,cpp} Per-entity position ring with time-delayed interpolation
  Net/NetClientSubsystem.*   GameInstance subsystem: WebSocket, reconnect, seq/ack, clock offset
  World/RemoteEntityActor.*  Visual proxy for a server entity; Blueprint subclass adds the mesh
  World/WorldProxySubsystem.* Spawns/destroys proxies from server events
  NightfallPlayerController.* Click-to-move: raycast, local preview, MoveTo intent
  Tests/                     Automation tests (Nightfall.Net.*)
  Generated/                 protoc + protoc-gen-turbolink output, committed. Compiled in TurboLinkGrpc (see below)
  GrpcBridge/                NightfallWire: binary encode/decode of world.proto envelopes, also compiled in TurboLinkGrpc
Scripts/
  setup-turbolink.sh         Submodule + prebuilt gRPC/protobuf libs + links into the plugin (moon: setup)
  gen-proto.sh               Regenerates Generated/ (moon: gen-proto)
  install-proto-tools.sh     Builds protoc 23.4, grpc_cpp_plugin 1.57, protoc-gen-turbolink for Linux
  run-tests.sh               Headless automation tests (moon: test-editor)
```

## Installing Unreal on Linux

Epic requires an account. Two routes:

- **Prebuilt binaries (fastest):** sign in at <https://www.unrealengine.com/linux>, download
  `Linux_Unreal_Engine_5.8.x.zip`, unzip to `~/UnrealEngine`, run
  `~/UnrealEngine/Engine/Binaries/Linux/UnrealEditor` once to let it finish setup.
- **From source:** link your GitHub account to Epic
  (<https://www.unrealengine.com/ue-on-github>), clone `EpicGames/UnrealEngine` at the `5.8`
  branch, then `./Setup.sh && ./GenerateProjectFiles.sh && make`. Budget 1-3 hours and ~200 GB.

The prebuilt zip does **not** include Epic's Linux compiler toolchain, and Unreal Build Tool
will refuse to compile without it ("Platform Linux is not a valid platform to build. SDK
validation failed"). Download the native toolchain the engine asks for (5.8 wants
`v26_clang-20.1.8-rockylinux8`, 1.5 GB) and point `LINUX_MULTIARCH_ROOT` at it:

```bash
mkdir -p ~/UnrealToolchains
curl -L https://cdn.unrealengine.com/Toolchain_Linux/native-linux-v26_clang-20.1.8-rockylinux8.tar.gz | tar -xz -C ~/UnrealToolchains
```

Then in `~/.bashrc`:

```bash
export UE_ROOT="$HOME/Linux_Unreal_Engine_5.8.3"
export LINUX_MULTIARCH_ROOT="$HOME/UnrealToolchains/v26_clang-20.1.8-rockylinux8"
```

Launch the bare editor from inside its binaries directory the first time; this build resolves
`Engine/Content` relative to the working directory and crashes on ICU data otherwise:

```bash
cd $UE_ROOT/Engine/Binaries/Linux && ./UnrealEditor
```

## First build

```bash
git submodule update --init                 # Plugins/TurboLink (setup does this too)
moon run client-unreal:setup                # one-off ~900 MB download of TurboLink's gRPC/protobuf libs
moon run client-unreal:build-editor         # compiles TurboLink, the generated code and Nightfall
moon run client-unreal:editor               # opens the project
```

`setup` is idempotent and `build-editor` depends on it. It never modifies tracked files in the
submodule: the libraries and links it adds are untracked, and `.gitmodules` sets
`ignore = untracked` so the submodule does not show as dirty.

If another Unreal Editor using the same engine is running (even on a different checkout), UBT
switches to hot-reload naming (`libUnrealEditor-*-0001.so`) and the link fails with
`unable to find library -lUnrealEditor-TurboLinkGrpc`. Close it, or add `-NoHotReloadFromIDE` to
the Build.sh command.

## gRPC (TurboLink)

The client calls the API's tonic `GameService` with [TurboLink](https://github.com/thejinchao/turbolink)
(plan decision D3), which wraps gRPC C++ 1.57 and protobuf 23.4 in UObjects.

```cpp
USessionClient* Session = GetGameInstance()->GetSubsystem<USessionClient>();
Session->Ping([](const FNetResult& Result, const FGrpcNightfallV1PingResponse& Response)
{
	if (Result.IsOk()) { UE_LOG(LogNightfall, Log, TEXT("server %s"), *Response.ServerVersion); }
	else { /* Result.Error is an ENetError (canonical gRPC code), Result.Message the server's text */ }
});
Session->SetBearerToken(AccessToken);   // Story 1.5; sent as `authorization: Bearer ...`
```

Blueprints get the same calls as `Ping`, `Get Character` and `Create Character` nodes with typed
delegates. Every call carries the deadline from `[/Script/Nightfall.NetSettings]`
(`CallTimeoutSeconds`, default 5 s); `GrpcEndpoint` defaults to `localhost:50051`. Callbacks run on
the game thread from TurboLink's manager tick.

### Where generated code lives, and why

TurboLink's generated classes have to compile **inside its `TurboLinkGrpc` module**: they include
its private headers, use its private protobuf/gRPC include paths, and hard-code reflection paths
such as `/Script/TurboLinkGrpc.…`. The protobuf and gRPC static libraries must also exist exactly
once in the process; if the Nightfall module linked them too, it would get its own copy of gRPC's
global state.

So the code is committed under `Source/Nightfall/Generated` (and the hand-written
`Source/Nightfall/GrpcBridge`), each with a `.ubtignore` so UBT does not compile it as part of
Nightfall, and `Scripts/setup-turbolink.sh` symlinks their entries into
`Plugins/TurboLink/Source/TurboLinkGrpc/{Public,Private}`. Nightfall depends on `TurboLinkGrpc`
and only uses the generated USTRUCTs (`FGrpcNightfallV1*`) and UObjects (`UGameService`,
`UGameServiceClient`).

The WebSocket envelopes (`ClientMessage`/`ServerMessage`) are not gRPC calls but use the same
generated classes: `ProtoCodec` maps the client-facing `FNet*` structs to `FGrpcNightfallV1*`, and
`NightfallWire` (in GrpcBridge, exported from TurboLinkGrpc) turns those into bytes with
protoc's C++ code.

### Regenerating after a .proto change

```bash
moon run client-unreal:gen-proto
```

TurboLink only ships Windows `.exe` generators. On first use, `Scripts/install-proto-tools.sh`
builds Linux ones into `~/.cache/nightfall/proto-tools` (about 10 minutes, once):

- `protoc` 23.4 and `grpc_cpp_plugin` from grpc v1.57.0, the exact versions TurboLink's prebuilt
  libraries were compiled from. Protobuf's generated code refuses to compile against any other
  runtime version, so do **not** use a distro `protoc` (`apt install protobuf-compiler` is 3.x).
  Needs `git`, a C++ compiler and `python3-venv` (`sudo apt install build-essential git
  python3-venv`); CMake and Ninja come from PyPI into a private venv, pinned below CMake 4,
  which rejects grpc 1.57's build files.
- `protoc-gen-turbolink` v2.7.0 (the one shipped in TurboLink v1.4.2), built from source with the
  .NET SDK bundled in Unreal. Upstream is a Visual Studio .NET Framework project with T4
  templates; the script preprocesses them with `dotnet-t4` and builds an SDK-style project.

Output is deterministic: regenerating unchanged protos produces no diff.

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
4. **Login over gRPC.** Done for `GameService` (Story 6.1, `USessionClient`). The device-flow
   login and `SessionService.IssuePlayTicket` follow in Story 1.5; until then the ticket is a dev
   constant.
5. **HUD.** CommonUI + MVVM: target frame, HP/MP/CP bars, chat log fed by `SystemMessage` ids.
6. ~~Replace the hand-written codec~~ Done in Story 6.1: the WebSocket codec uses the generated
   classes.

## Tests

```bash
moon run api:dev                         # in another terminal; the Ping test needs the real API
moon run client-unreal:test-editor       # or: bash Scripts/run-tests.sh [filter]
```

| Test | Needs server | Checks |
|---|---|---|
| `Nightfall.Net.ProtoCodec.Encode` | no | `ClientMessage` bytes match hand-assembled protobuf |
| `Nightfall.Net.ProtoCodec.Decode` | no | `Ack`, `IntentRejected`, `EntitySpawn`, empty and truncated frames |
| `Nightfall.Net.SessionClient.Ping` | yes | Ping round trip; `server_version` equals the Cargo workspace version |

## Build status

2026-10-08, UE 5.8.3 Linux, clang 20.1.8 (`v26_clang-20.1.8-rockylinux8`), TurboLink v1.4.2:

- `NightfallEditor Linux Development` builds clean from scratch; the `Nightfall Linux Development`
  game target builds as well. The Nightfall module keeps
  `bWarningsAsErrors`; no relaxation was needed because TurboLink's headers are warning-free
  under clang 20 and generated code does not compile in the Nightfall module.
- Warnings that remain are all in plugin-compiled code, where warnings are not errors:
  `TurboLinkEditorModule.cpp` uses `FCoreDelegates::OnPostEngineInit` (deprecated in 5.8;
  breaks on the next engine upgrade, see the spike log) and the generated marshaling touches
  the `[deprecated = true]` `CreateCharacterRequest.account_id`.
- `Automation RunTests Nightfall` against `nightfall-api` with in-memory adapters: 3/3 pass;
  Ping returned `server_version=0.1.0` in ~50 ms. With the API stopped the Ping test fails with
  `ENetError::Unavailable` (connection refused), as it should.
- WebSocket runtime behaviour is still unverified until the server's `/ws` endpoint exists.

## Spike log: TurboLink on UE 5.8.3 / Linux (Story 6.1, risk R1)

Time-box was one working day; the plugin built within the first hour. Outcome: **TurboLink works on UE
5.8.3 Linux with clang 20. Fallbacks B (vendored grpc++) and C (HTTP/JSON) are not needed.**

| # | Step | Problem | Fix |
|---|---|---|---|
| 1 | Add submodule at `v1.4.2` (`32cc00b`), enable in `.uproject` | The git repo has no third-party libraries (`Source/ThirdParty` holds only `.gitkeep`); TurboLink's README points at turbolink-libraries v1.3.1, which predates the v1.4.2 sources | Take `Source/ThirdParty` from the v1.4.2 release asset `TurboLink.UE56.zip` (912 MB, sha256 `9ad1ecd9…eaf39`): gRPC 1.57.0, protobuf 23.4, abseil, re2 with prebuilt `lib/linux/{Debug,Release}` static libs. Scripted in `setup-turbolink.sh`, pinned by sha256 |
| 2 | First `Build.sh NightfallEditor` | All TurboLink sources compiled with clang 20 on the first try. Link failed: `ld.lld: error: unable to find library -lUnrealEditor-TurboLinkGrpc` | Not a TurboLink bug: a user's editor (same engine, other checkout) was running, so UBT used hot-reload names (`-0001.so`) and a new module has no un-suffixed library to link against. `-NoHotReloadFromIDE` → **build succeeded** |
| 3 | Warnings in the plugin | `TurboLinkEditorModule.cpp:33` `FCoreDelegates::OnPostEngineInit` is deprecated in 5.8; TurboLink's Build.cs warns that `Private/pb` does not exist; UBT notes `no UE_OpenSSL.ver ... linking bundled OpenSSL without symbol isolation` | Left alone (plugin is pinned upstream and its warnings are not errors). `Private/pb` exists once code is generated. The OnPostEngineInit API is removed in the next engine release: patch or update TurboLink before moving past 5.8 |
| 4 | Code generation on Linux | TurboLink ships `protoc.exe`, `grpc_cpp_plugin.exe`, `protoc-gen-turbolink.exe` and a `.cmd` script only | `install-proto-tools.sh`: build protoc + grpc_cpp_plugin from grpc v1.57.0 (matches the libraries); build protoc-gen-turbolink from source with UE's bundled .NET 10 SDK |
| 5 | grpc 1.57 CMake configure | CMake 4.x: `Compatibility with CMake < 3.5 has been removed` | Pin `cmake<4` from PyPI |
| 6 | protoc-gen-turbolink build | Upstream csproj is .NET Framework 4.6.2 and its T4-preprocessed `.cs` files are not committed. First attempt: `CS0103: The name 's' does not exist` | Preprocess with `dotnet-t4` using class names `protoc_gen_turbolink.Template.<Name>` and drop `public` so they merge with the internal halves in `TemplateModel.cs`; SDK-style csproj targeting the bundled net10.0 |
| 7 | Where generated code compiles | TurboLink expects generated files copied into its own module; this repo wants them committed in our tree, and the submodule must stay pristine | Commit under `Source/Nightfall/{Generated,GrpcBridge}` with `.ubtignore`; symlink into `TurboLinkGrpc`. UBT and UHT follow the directory symlinks without complaint |
| 8 | Name clash after Story 0.1 | `session.proto` makes TurboLink generate `SNightfallV1/SessionClient.h` (`USessionServiceClient`), whose `SessionClient.generated.h` would be on Nightfall's include path next to ours | Our class stays `USessionClient`; its files are `Net/SessionClientSubsystem.{h,cpp}` |
| 9 | Oneofs in TurboLink's generated structs | No "not set" case: the case enum defaults to the first member, and `TURBOLINK_TO_GRPC` dereferences the active member's `TSharedPtr` unchecked | `NightfallWire` only marshals an intent whose pointer is valid; `ProtoCodec` checks the pointer, not just the case, when decoding |
| 10 | Editor automation tests | No game world ticks a test's game instance, so TurboLink never drains its completion queue | The Ping test pumps `UTurboLinkGrpcManager::Tick` itself while waiting |

Other notes: TurboLink's lambda API (`UGameService::CallPing`) keeps its callback in an
unrooted `NewObject` that the garbage collector may collect mid-call; `USessionClient` uses the
client-object API with its own handle-to-callback map instead.
