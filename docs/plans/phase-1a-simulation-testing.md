# Phase 1a Plan: Headless Simulation Testing

**Status:** DRAFT 2026-10-08. Starts after Phase 1 closes (E5.4, E6.4). **Planning model:** Opus; implementation model and effort per story.

## 1. Goal

A headless Unreal client ("walker") that reuses every line of the real client, is driven by a
scenario script, runs against the real server stack from CI, and fails on any assertion, crash,
ensure or error log. Use it to (a) back-fill simulation scenarios for everything Phase 0b and
Phase 1 shipped, and (b) make "a simulation scenario" a standing deliverable of every client epic
from Phase 2 on.

What this is **not**: an extracted engine-free logic library, or a Rust bot. Option analysis in
§2 D1. A Rust load-test bot stays a candidate for the live-operations phase (§7).

## 2. Decisions

| # | Decision | Choice | Rejected | Why |
|---|----------|--------|----------|-----|
| D1 | Bot runtime | The shipped client binary, `-game -nullrhi -nosound -unattended`, every subsystem live, no renderer | (a) Extract `Net/`, `Auth/`, `Combat/` into an engine-free C++ library and write a separate thin client; (b) a Rust bot speaking `packages/proto` | The client is already thin: ~2,200 lines of protocol/state code have zero `UWorld`/actor references, but they are built on `FString`/`TMap`/delegates/`IWebSocket`/TurboLink. (a) rewrites all of that onto a second network stack plus a marshaling seam back into UE, roughly doubling the code to serve one consumer. (b) reuses the contract, not the client, so it cannot catch client projection bugs. (a) is 3–5 weeks; this is days, because `nf.Login`/`nf.EnterWorld`/`nf.ClickMove` and `FScopedTestGameInstance` already exist |
| D2 | Driver | A `UBotScenarioRunner` game-instance subsystem executes a scenario file passed as `-BotScenario=<path>`; steps are the existing `nf.*` console commands plus new `nf.Expect*`/`nf.WaitFor`/`nf.Within` steps; the process exits with a code and writes a JUnit report | Gauntlet `UGauntletTestController` as the only driver; Blueprint/Python scripting; Functional Test actors in a map | Console commands are the headless path the client already has (README "Headless"). Gauntlet needs UAT and a packaged build and is adopted later for multi-client orchestration (E4.4), not as the entry point. The Python plugin is editor-only. Functional Test maps need a world and a renderer-shaped lifecycle the bot does not |
| D3 | Scenario format | One `.nfs` text file per scenario under `apps/client-unreal/Scenarios/`: one step per line, `#` comments, `nf.*` command syntax, every wait with an explicit timeout | JSON/YAML, C++ per scenario | Readable in a diff, no parser beyond the console, and a scenario can be pasted into a running editor to reproduce by hand |
| D4 | Oracle | Assertions read the client's projections (`UCombatStateSubsystem`, `UNetClientSubsystem` cache, `UWorldProxySubsystem` proxies) and the typed events the server sent. Each run also records its session; `api:replay-check` must replay it byte-identically | Asserting server internals through a side channel; screenshot comparison | The server is authoritative, so the only client-side truth is "what the server said, projected correctly". Replaying the bot's own session ties the scenario to the deterministic core at no extra cost |
| D5 | Determinism and isolation | Each scenario runs against a fresh Compose stack (Postgres, NATS, Keycloak) and API with `AUTH_DEV_TOKENS=1`; bots use `test:<uuid>` accounts; the zone seed is pinned per scenario; NPC spawn tables are the Phase 1 fixtures | Shared long-lived server; real Keycloak logins per bot | Reproducible runs and parallel scenarios that cannot see each other |
| D6 | Pass criteria | Every `nf.Expect` true within its timeout; zero `Error`/`Fatal` lines in `LogNightfall`, `LogTurboLink` and `LogNet*`; zero ensures; clean exit; wall-clock under the scenario's budget | Assertions only | A hang or an ensure is a bug the assertions would not see |
| D7 | CI placement | A `sim` job on a **self-hosted Linux runner** with UE 5.8 installed (one owner step: provision it). Runs on `main` and nightly first, PR-gated once two weeks flake-free. Multi-client soak runs nightly only | GitHub-hosted runners; Epic's container images | A UE install plus DDC exceeds hosted runner disk; the container images need an Epic-linked GitHub org and a registry pull per run. A persistent runner keeps the engine, DDC and TurboLink libraries warm |

## 3. Architecture

```
apps/client-unreal/
  Scenarios/
    0b-login-enter-world.nfs          one scenario per story being covered (E2, E3)
    1-kill-one-monster.nfs
    ...
  Source/Nightfall/Bot/
    BotScenarioRunner.{h,cpp}         UGameInstanceSubsystem: reads -BotScenario, runs steps on tick,
                                      owns the JUnit writer and the log/ensure sentinel, exits the process
    BotSteps.{h,cpp}                  nf.WaitFor, nf.Expect*, nf.Within, nf.Sleep, nf.Target, nf.Attack,
                                      nf.StopAttack, nf.Respawn (console commands; also usable interactively)
    BotPredicates.{h,cpp}             named predicates over the projections (see below)
  Scripts/
    run-sim.sh                        one scenario or a glob; starts/attaches API; collects artifacts
    run-sim-multi.sh                  N processes, one scenario each, shared stack (E4.3)
.github/workflows/sim.yml             self-hosted job: compose up -> API -> run-sim -> replay-check -> JUnit
```

**Runner lifecycle.** `UBotScenarioRunner::Initialize` reads `-BotScenario`; if absent the
subsystem is inert, so the shipped game is unchanged. Steps execute sequentially on the game
thread tick; a waiting step polls its predicate each tick until true or timed out. The runner
fails fast on the first failed step, writes `Saved/Sim/<scenario>.xml` (JUnit, one testcase per
`nf.Expect`/`nf.WaitFor`) and `Saved/Sim/<scenario>.log`, then calls `FPlatformMisc::RequestExit`
with 0/1. An `FOutputDevice` sentinel counts `Error`/`Fatal` lines and ensures during the run;
any count above zero fails the scenario even if every step passed (D6).

**Predicates** are named functions of the projections, e.g. `connected`, `in_world`,
`own_at <x> <y> <tol>`, `proxies >= <n>`, `target == <name|none>`, `target_hp < <n>`,
`own_hp < <n>`, `own_dead`, `own_level == <n>`, `xp_known`, `attack_state == <idle|pending|active>`,
`damage_numbers >= <n>`, `rejected == <reason>`, `last_ack_seq == <n>`. The list grows with phases;
each predicate is a one-liner over `UCombatStateSubsystem::BuildHudModel()` or the net cache.

**Scenario syntax** (D3):

```
# 1-kill-one-monster.nfs: target the nearest attackable NPC, auto-attack to death, gain XP.
nf.Login
nf.WaitFor connected 15
nf.EnterWorld
nf.WaitFor in_world 15
nf.WaitFor proxies >= 1 10
nf.Target nearest_attackable
nf.WaitFor target != none 5
nf.Attack
nf.WaitFor attack_state == active 5
nf.WaitFor damage_numbers >= 1 10
nf.WaitFor target_hp == 0 60
nf.Expect target == none
nf.Expect xp_known
nf.Expect own_level >= 1
nf.Within 90                     # scenario wall-clock budget, checked at exit
```

**Outputs per run.** `Saved/Sim/<scenario>.xml` (JUnit), `.log` (UE), `.failure.json` (E1.6, on
failure), `trace.html` (E6.4), the `.nfr` session recording, and on failure a `.mp4` (E4.6).
`run-sim.sh` merges the per-scenario `coverage.contract.json` (E1.7) and
`coverage.transitions.json` (E3.9) and prints both, with the traceability check (E5.1), in the job
summary. Coverage here means contract and state-transition coverage, not line coverage: a nightly
clang source-based coverage build of the `Nightfall` module is listed under §7 as optional.

**Multi-client scenarios.** `run-sim-multi.sh` launches N processes with per-process scenarios
and a shared `-SimGroup=<id>`; cross-client coordination uses the server (a client waits until
`proxies >= 2`), never IPC. Gauntlet (E4.4) replaces the script for the nightly soak, where it
adds role assignment, per-process timeouts and log collection.

**Test hygiene folded in (E1.4).** Existing automation tests move from `EngineFilter` to
`ProductFilter` so `Automation RunTests Nightfall` selects only ours; live tests
(`*.EndToEnd`, `SessionClient.Ping`) gain `-RequireLiveApi`, which turns their "API not reachable,
skipped" warning into a failure in CI so a down server can never show green.

## 4. Epics, stories, tasks

Models: **Sonnet medium** for mechanical work, **Opus high** for design-heavy work,
**Codex high/medium** for integration, review and docs. Every Done when includes tests.
Scenarios count as the **W** (real-socket) row of the
[API guidelines §4](../engineering/api-guidelines.md#4-required-tests-per-endpoint) matrix for
client-facing behaviour; they do not replace U/A on the server or the existing UE automation tests.

> [!NOTE]
> The `[client-visible]` tag denotes client-observable contract and state behavior. The simulation traceability checker verifies completed tagged stories against scenario coverage (`.nfs`). Rendering-only portions retain automation and cook validation, but tags remain on those stories and uncovered cases are explicitly reported. New plans should add tags independently before writing scenarios.

### E1: Bot runtime

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 1.1 Scenario runner | `UBotScenarioRunner`: `-BotScenario` parsing, line reader, sequential step execution on tick, first-failure stop, JUnit + log writers, `RequestExit` with code; inert without the flag | Opus high | Automation test feeds a scenario string and asserts step order, timeout failure, exit code and JUnit shape; `-game -nullrhi` with no flag boots unchanged |
| 1.2 Step and predicate commands | `nf.WaitFor`, `nf.Expect`, `nf.Within`, `nf.Sleep`; predicates listed in §3 over the Phase 0b/1 projections; `nf.Target <id|nearest_attackable|none>`, `nf.Attack`, `nf.StopAttack`, `nf.Respawn` | Sonnet medium | Automation tests drive each predicate from recorded events (reuse Story 5.2 fixtures) and assert true/false transitions; bad predicate names fail the scenario at parse time, not at runtime |
| 1.3 Log and ensure sentinel | `FOutputDevice` counting `Error`/`Fatal` on the listed categories and ensures; allow-list file per scenario for known noisy lines, each entry with a reason | Sonnet medium | A scenario that passes every step but logs one error exits 1 and names the line in JUnit; allow-listed line passes |
| 1.4 Test hygiene | `ProductFilter` on all Nightfall automation tests; `-RequireLiveApi` on live tests; `run-tests.sh` passes it in CI | Sonnet medium | With the API down and the flag set, `Nightfall.Login.EndToEnd` fails; without the flag it still skips with a warning |
| 1.5 `run-sim.sh` and moon task | Script: resolves `UE_ROOT`, builds the game target if stale, starts or attaches to the API, runs one scenario or a glob, copies `Saved/Sim/*` and the session recording to an artifacts dir; `moon run client-unreal:sim -- <scenario>` | Codex medium | Running the E2.1 scenario locally from a clean checkout with Compose up produces a JUnit file and exit 0; a deliberately broken scenario exits 1 |
| 1.6 Failure bundle | On the first failed step write `Saved/Sim/<scenario>.failure.json` and embed a summary in the JUnit `<failure>`: the step, predicate, its last evaluated value and wait time; the last 50 typed events with server tick and arrival time; a projection snapshot (own HP/MP/XP/level/target/attack state; every proxy with position, HP, incarnation); own-position history since the last `nf.ClickMove`; the scenario's server tick range and own entity id, so `nightfall-replay --session <entity>` reproduces it from the recording | Sonnet medium | A seeded failure in `1-kill-one-monster` yields a bundle from which a reviewer identifies the failing tick without the UE log; bundle schema has a version and a test |
| 1.7 Contract coverage counter | Tally every `ClientMessage` intent sent and every `ServerMessage` event, `IntentRejected` reason and close code received, enumerated from the proto descriptors so new oneof cases appear as "never seen" automatically; emit as JUnit `<properties>`; `run-sim.sh` merges across scenarios into `coverage.contract.json` and fails when a message or reason covered on `main` is no longer covered | Sonnet medium | Report lists intents/events/reasons as `n/N` with the missing names; removing a scenario that was the only cover for `Respawn` fails the merge; the server's session-log tally for the same run agrees |

### E2: Back-fill scenarios for Phase 0b

Each scenario covers a shipped Phase 0b story. Names are files under `Scenarios/`.

| Story | Scenario(s) | Model | Done when |
|-------|-------------|-------|-----------|
| 2.1 Login and enter world | `0b-login-enter-world`: dev-token login, character list, create if empty, play ticket, WS connect, own `EntitySpawn`, own pawn at spawn tile | Sonnet medium | Passes against Compose stack; `connected` and `in_world` predicates are the gate for every later scenario |
| 2.2 Click to move | `0b-click-move`: three `nf.ClickMove`, each Acked, own position reconciles within 1 tile; `0b-move-rejected`: destination > 64 tiles is `IntentRejected`, position unchanged | Sonnet medium | Both pass; the rejection scenario asserts `rejected == <reason>` and no move |
| 2.3 Reconnect with a new ticket | `0b-reconnect`: kill the socket via `nf.DropSocket` (new, test-only command), assert reconnect fetched a fresh ticket, `in_world` again, projections rebuilt from the spawn stream | Sonnet medium | Passes; asserts the ticket differs and the entity id is unchanged |
| 2.4 Two clients see each other | `0b-two-clients-a/b` via `run-sim-multi.sh`: each waits for `proxies >= 1`, A moves, B observes A's proxy within 2 tiles of the destination after interpolation delay; A logs out, B sees despawn | Codex medium | Passes with two processes; the despawn assertion is what catches a leaked proxy |
| 2.5 Logout replacement | `0b-login-replaces`: second login for the same character while the first is connected; first is disconnected with the documented reason, second is `in_world` | Sonnet medium | Passes; the first process's disconnect reason is asserted, not just the drop |

### E3: Back-fill scenarios for Phase 1

| Story | Scenario(s) | Model | Done when |
|-------|-------------|-------|-----------|
| 3.1 Target and attack | `1-target-attack`: `nf.Target nearest_attackable` → `TargetChanged`, HUD target frame; `nf.Attack` → one Ack, `attack_state == active`; repeated `nf.Attack` does not produce a second Ack; `nf.StopAttack` → idle. `1-target-unknown`: unknown id rejected with `UNKNOWN_ENTITY`, target stays none | Sonnet medium | Both pass; replaces the skip-prone parts of `CombatEndToEndTest` |
| 3.2 Kill, XP, level | `1-kill-one-monster` (§3 example): damage numbers appear once per `AttackResult`, target HP reaches 0, target clears, `XpGained` sets `xp_known`, level matches the XP table threshold for the fixture reward | Sonnet medium | Passes; the asserted level is computed from `experience.toml`, not hard-coded |
| 3.3 Chase and leash | `1-chase-leash`: target an NPC, walk away inside aggro range, assert the NPC proxy follows (distance shrinks) then stop beyond leash range and assert it returns home (proxy position within tolerance of its spawn) | Opus high | Passes; tolerances are derived from the NPC template speed and leash values in `packages/data` |
| 3.4 Player death and respawn | `1-die-respawn`: stand in an NPC's aggro with no attack until `own_dead`; dead overlay model true; `nf.Respawn` → own HP = `floor(maxHP*65/100)`, MP 0, position at town spawn, target none, attack idle; `nf.Attack` ends protection (asserted through the next NPC hit landing) | Sonnet medium | Passes; HP assertion uses the E1 formula vector, not a literal |
| 3.5 Delevel | `1-delevel`: reach level 2 via 3.2, then die; assert XP loss equals the HF table row and level returns to 1 | Sonnet medium | Passes; a changed `penalties.toml` row fails it |
| 3.6 Late AOI entry and stale facts | `1-late-entry-a/b`: A fights an NPC; B enters world mid-fight and asserts the spawn carries current HP/life; A disconnects and reconnects mid-fight, asserts `XP --` until the next `XpGained` and that the NPC's HP is not reset by the re-sent spawn (stale incarnation rejection) | Opus high | Passes with two processes; the stale-spawn assertion is the one that would catch a regression in `IsStaleSpawn` |
| 3.7 Social aggro | `1-social-aggro-a/b`: two NPC instances; A attacks one, B stands in range of the other and asserts it engages B (hate from social aggro) | Sonnet medium | Passes; needs the two-instance fixture from Phase 1 D1 |
| 3.8 Replay of a bot session | Every E2/E3 scenario's session recording is replayed by `api:replay-check` after the run | Codex medium | Divergence in any bot session fails the `sim` job; one scenario's recording is checked in as a fixture under the existing schema policy |
| 3.9 Transition coverage from recordings | `nightfall-replay coverage --file <nfr>`: counts every NPC intention transition (Idle/Active/Attack/ReturnHome/Dead), player attack-state transition and life-incarnation change in a recording, against the full transition table from `domain/zone`; `run-sim.sh` merges the per-scenario counts into `coverage.transitions.json` and the job summary prints covered/total with the never-seen transitions named | Opus high | Running it over the E3 set reports every transition the Phase 1 state machine defines; a transition the AI cannot reach is listed as such in the table, not silently absent; unit test on `two-players-fight-v2.nfr` pins its counts |

### E4: Pipeline

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 4.1 (owner) Self-hosted runner | **One owner action:** a Linux box or VM with UE 5.8.3, clang toolchain, Docker, the GitHub runner agent, labelled `unreal`; `UE_ROOT` set; TurboLink prebuilt libraries and DDC warmed once | owner | `moon run client-unreal:build` and `client-unreal:test-editor` pass on the runner by hand |
| 4.2 `sim` workflow | `.github/workflows/sim.yml` on `runs-on: [self-hosted, unreal]`: checkout with submodules, Compose services, API with `AUTH_DEV_TOKENS=1`, build game target, `run-sim.sh Scenarios/*.nfs`, `api:replay-check` on the recordings, JUnit upload, logs and recordings as artifacts on failure. Triggers: push to `main`, nightly, `workflow_dispatch`, and PRs labelled `sim` | Codex high | Green on `main` for the E2/E3 set; a seeded failing scenario shows as a failed testcase with the step and predicate in the summary |
| 4.3 Multi-client script | `run-sim-multi.sh` as in §3; used by 2.4, 2.5, 3.6, 3.7 | Sonnet medium | The four two-client scenarios run under it in CI |
| 4.4 Gauntlet nightly soak | `BuildCookRun` to a Linux staged build; a `NightfallSoak` Gauntlet test launching N clients (default 8) each looping `1-kill-one-monster` for 20 minutes against one stack; collects per-client logs and the server's combat telemetry; fails on any crash, ensure, assertion or tick p99 over the Phase 1 budget | Opus high | Nightly passes with 8 clients; the soak report links the Grafana dashboard range |
| 4.5 Flake policy and PR gating | Quarantine list with owner and expiry; a scenario on the list runs but does not fail the job; after two weeks with zero unquarantined failures the `sim` job becomes required on PRs touching `apps/client-unreal/**`, `apps/api/**` or `packages/**` | Codex medium | Branch protection updated; the list is empty or every entry has an expiry |
| 4.6 Video on failure | After E4.4. When a scenario fails, `run-sim.sh --video` re-runs it once with rendering under Xvfb + Mesa lavapipe (`-RenderOffscreen -DumpMovie -ForceRes -ResX=960 -ResY=540`), encodes the frames with ffmpeg and attaches the `.mp4` to the failure artifacts; never on passing runs, never on the PR set until it is measured under 2 minutes per scenario | Sonnet medium | A seeded failure in CI uploads a playable video of that scenario; the re-run's own JUnit is kept separate so the pass/fail verdict comes from the headless run |

### E5: Forward coverage for Phases 2–9

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 5.1 Standing rule and traceability check | Add to [planning README](../planning/README.md) conventions and the phase-plan template: every client epic has a story "Simulation scenario(s)" whose Done when names the `.nfs` files, and a phase cannot close with that story open. Every `.nfs` starts with `# covers: <phase> <story>, ...`; `Scripts/sim-traceability.sh` parses those headers and the plan files' story tables and prints every ✅ story tagged client-visible with no covering scenario; runs in the `sim` job summary | Codex medium | README and the Phase 1 plan's E5 carry the rule; the next phase plan drafted follows it; the check lists zero uncovered 0b/1 stories once E2/E3 land and fails the job for a newly completed story without a scenario |
| 5.2 Scenario seeds per phase | A table in this document (§4.1 below) naming the first scenario each later phase must ship, so the predicates and `nf.*` commands they need are known before that phase starts | Opus high | Table reviewed against each planning doc's client implications section |
| 5.3 Predicate backlog | For each seed, the predicates/commands not yet in E1.2 (e.g. `inventory_has`, `skill_ready`, `party_size`, `in_zone`) listed as the first task of that phase's scenario story | Sonnet medium | Backlog cross-links each predicate to the proto message it reads |

#### 4.1 Scenario seeds and predicate backlog per phase

| Phase | Document | First scenario(s) | New predicates / commands | Proto message / contract status |
|-------|----------|-------------------|---------------------------|---------------------------------|
| 2 Race and class | [02](../planning/02-race-and-class.md) | `2-create-each-race-class`: create one character per race/class, assert starting stats from the class tables; `2-class-transfer`: reach the transfer level via XP fixture, transfer, assert stat delta | `own_stat <name> == <n>`, `nf.CreateCharacter <race> <class>`, `nf.ClassTransfer` | Base stats via [`Character.stats`](../../packages/proto/nightfall/v1/game.proto#L93) ([`BaseStats`](../../packages/proto/nightfall/v1/game.proto#L79)), race via [`CreateCharacterRequest.race`](../../packages/proto/nightfall/v1/game.proto#L67); class selection and transfer are **explicitly future contract** |
| 3 Combat and skills | [03](../planning/03-combat-and-skills.md) | `3-cast-skill`: learn, cast, assert cooldown and MP cost; `3-buff-expires`: buff applied, duration elapses, stat returns; `3-pvp-flag`: two clients, attack flags, karma on kill | `skill_ready <id>`, `buff_active <id>`, `own_flag == <pvp|karma>`, `nf.Cast <id>` | **Explicitly future contract** (no skill, buff, or PvP/karma proto messages in `packages/proto`; current [`ClientMessage`](../../packages/proto/nightfall/v1/world.proto#L32) only supports basic melee) |
| 4 Items and equipment | [04](../planning/04-items-and-equipment.md) | `4-equip-weapon`: pick up fixture drop, equip, P.Atk changes; `4-enchant-fail`: enchant to failure, item destroyed, ledger row asserted via replay | `inventory_has <template> >= <n>`, `equipped <slot> == <template>`, `nf.Equip`, `nf.Enchant` | **Explicitly future contract** (no inventory, item, or equipment messages exist in `packages/proto`) |
| 5 Economy and crafting | [05](../planning/05-economy-and-crafting.md) | `5-kill-loot`: kill, loot appears, pick up; `5-vendor-roundtrip`: sell, buy, adena conserved; `5-trade-a/b`: two clients trade, both inventories assert | `adena == <n>`, `trade_state`, `nf.Loot`, `nf.Trade` | **Explicitly future contract** (no currency, loot, vendor, or trade messages exist in `packages/proto`) |
| 6 World and content | [06](../planning/06-world-and-content.md) | `6-zone-travel`: walk to a zone edge, assert zone change and new spawn stream; `6-quest-chain`: accept, kill N, turn in; `6-instance-a/b`: party enters an instance, isolated from a third client | `in_zone == <id>`, `quest_state <id>`, `nf.Travel`, `nf.AcceptQuest` | Movement via [`MoveToRequest`](../../packages/proto/nightfall/v1/world.proto#L46) and [`EntityMove`](../../packages/proto/nightfall/v1/world.proto#L169); zone transitions, instances, and quests are **explicitly future contract** |
| 7 Social systems | [07](../planning/07-social-systems.md) | `7-party-xp-a/b`: party of two kills, XP split per rule; `7-clan-create`: create, invite, roster; `7-siege-smoke`: N clients, siege starts and ends on schedule | `party_size`, `clan == <name>`, `nf.Invite`, `nf.AcceptParty` | **Explicitly future contract** (no party or clan proto messages exist in `packages/proto`) |
| 8 Client presentation | [08](../planning/08-client-presentation.md) | Mostly out of headless scope; `8-localization`: every HUD string resolves in two locales (no `??` placeholders); `8-input-remap`: remapped action still sends the intent | `hud_text <field> matches`, `nf.SetLocale` | Sourced from client projection of [`EntitySpawn.name`](../../packages/proto/nightfall/v1/world.proto#L149) or [`Character.name`](../../packages/proto/nightfall/v1/game.proto#L90); `nf.SetLocale` is client-side only (no server proto) |
| 9 Live operations | [09](../planning/09-live-operations.md) | `9-gm-teleport`: GM command moves a bot; `9-rate-limit`: intent flood is throttled with the documented rejection; `9-patch-restart`: server restart mid-session, bot reconnects and state matches; load: promote the Gauntlet soak to the phase's target session count, or build the Rust bot (§7) if UE processes cannot reach it | `nf.Gm <cmd>`, `rejected == RATE_LIMITED`, `nf.FloodMoveTo <n>` | Rate limiting via [`IntentRejected`](../../packages/proto/nightfall/v1/world.proto#L100) with [`RejectReason::REJECT_REASON_RATE_LIMITED`](../../packages/proto/nightfall/v1/world.proto#L112) and [`MoveToRequest`](../../packages/proto/nightfall/v1/world.proto#L46); GM commands are **explicitly future contract** |

##### Predicate backlog (Story 5.3 detailed mapping)

The backlog below details the wire contract for every predicate and console command introduced across the phase seeds. Note that `packages/proto` currently contains only `game.proto`, `session.proto`, and `world.proto` (basic movement, session admission, and Phase 1 melee combat core). No inventory, item, equipment, skill, buff, party, clan, trade, or quest proto messages exist yet; these are explicitly designated as future contracts to be defined in their respective phases.

| Predicate / command | Target phase | Current proto mapping / wire contract | Status |
|---------------------|--------------|--------------------------------------|--------|
| `own_stat <name> == <n>` | Phase 2 | Base stats: [`Character.stats`](../../packages/proto/nightfall/v1/game.proto#L93) ([`BaseStats`](../../packages/proto/nightfall/v1/game.proto#L79)); resource/level state: [`StatsChanged`](../../packages/proto/nightfall/v1/world.proto#L249); other derived combat stats need a future owner-only contract | Existing base/resource proto; extended derived stats are future contract |
| `nf.CreateCharacter <race> <class>` | Phase 2 | Name and race: [`CreateCharacterRequest`](../../packages/proto/nightfall/v1/game.proto#L60) and enum [`Race`](../../packages/proto/nightfall/v1/game.proto#L70); class selection not in proto | Existing for race; class is **explicitly future contract** |
| `nf.ClassTransfer` | Phase 2 | No class transfer RPC or intent in `packages/proto` | **Explicitly future contract** (Phase 2) |
| `skill_ready <id>` | Phase 3 | No skill messages exist in `packages/proto` | **Explicitly future contract** (Phase 3) |
| `buff_active <id>` | Phase 3 | No buff or effect messages exist in `packages/proto` | **Explicitly future contract** (Phase 3) |
| `own_flag == <pvp\|karma>` | Phase 3 | Not present in [`EntitySpawn`](../../packages/proto/nightfall/v1/world.proto#L147) or any proto message | **Explicitly future contract** (Phase 3) |
| `nf.Cast <id>` | Phase 3 | [`ClientMessage`](../../packages/proto/nightfall/v1/world.proto#L32) has no skill cast intent (only melee [`AttackRequest`](../../packages/proto/nightfall/v1/world.proto#L68)) | **Explicitly future contract** (Phase 3) |
| `inventory_has <template> >= <n>` | Phase 4 | No inventory or item messages exist in `packages/proto` | **Explicitly future contract** (Phase 4) |
| `equipped <slot> == <template>` | Phase 4 | No equipment slot or item messages exist in `packages/proto` | **Explicitly future contract** (Phase 4) |
| `nf.Equip` | Phase 4 | No item equip intent or RPC exists in `packages/proto` | **Explicitly future contract** (Phase 4) |
| `nf.Enchant` | Phase 4 | No enchant intent or RPC exists in `packages/proto` | **Explicitly future contract** (Phase 4) |
| `adena == <n>` | Phase 5 | No currency or economy proto messages exist in `packages/proto` | **Explicitly future contract** (Phase 5) |
| `trade_state` | Phase 5 | No trade proto messages exist in `packages/proto` | **Explicitly future contract** (Phase 5) |
| `nf.Loot` | Phase 5 | No loot or pickup intent in [`ClientMessage`](../../packages/proto/nightfall/v1/world.proto#L32) | **Explicitly future contract** (Phase 5) |
| `nf.Trade` | Phase 5 | No player trade intent or RPC in `packages/proto` | **Explicitly future contract** (Phase 5) |
| `in_zone == <id>` | Phase 6 | Coordinates via [`Position`](../../packages/proto/nightfall/v1/game.proto#L97) and [`EntityMove`](../../packages/proto/nightfall/v1/world.proto#L169); zone boundary handoff / zone IDs not in proto | **Explicitly future contract** for zone ID (Phase 6) |
| `quest_state <id>` | Phase 6 | No quest proto messages exist in `packages/proto` | **Explicitly future contract** (Phase 6) |
| `nf.Travel` | Phase 6 | Movement uses [`MoveToRequest`](../../packages/proto/nightfall/v1/world.proto#L46); zone transfer / teleport not in proto | **Explicitly future contract** for travel (Phase 6) |
| `nf.AcceptQuest` | Phase 6 | No quest RPC or intent exists in `packages/proto` | **Explicitly future contract** (Phase 6) |
| `party_size` | Phase 7 | No party proto messages exist in `packages/proto` | **Explicitly future contract** (Phase 7) |
| `clan == <name>` | Phase 7 | No player clan proto messages exist in `packages/proto` | **Explicitly future contract** (Phase 7) |
| `nf.Invite` | Phase 7 | No party or clan invite RPC/intent in `packages/proto` | **Explicitly future contract** (Phase 7) |
| `nf.AcceptParty` | Phase 7 | No party accept RPC or intent in `packages/proto` | **Explicitly future contract** (Phase 7) |
| `hud_text <field> matches` | Phase 8 | Projection of [`EntitySpawn.name`](../../packages/proto/nightfall/v1/world.proto#L149) or [`Character.name`](../../packages/proto/nightfall/v1/game.proto#L90) with local client localization | Existing proto (for names); presentation check |
| `nf.SetLocale` | Phase 8 | Purely client-side setting command | Client-side only (no server proto needed) |
| `nf.Gm <cmd>` | Phase 9 | No GM/admin service or message exists in `packages/proto` | **Explicitly future contract** (Phase 9) |
| `rejected == RATE_LIMITED` | Phase 9 | [`IntentRejected`](../../packages/proto/nightfall/v1/world.proto#L100) with [`RejectReason::REJECT_REASON_RATE_LIMITED`](../../packages/proto/nightfall/v1/world.proto#L112) | Existing proto enum |
| `nf.FloodMoveTo <n>` | Phase 9 | Sends repeated [`MoveToRequest`](../../packages/proto/nightfall/v1/world.proto#L46) inside [`ClientMessage`](../../packages/proto/nightfall/v1/world.proto#L32) | Existing proto intent |

### E6: Docs

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 6.1 Client README | "Simulation" section: how to write a scenario, the predicate list, running one locally, reading the JUnit, quarantine | Codex medium | A new scenario can be written from the README alone (reviewer tries) |
| 6.2 Pipeline diagram | `docs/diagrams/simulation-pipeline.html`: PR/nightly lanes, Compose stack, bot processes, replay-check, artifacts; linked from the diagram README | agy medium | Docs-only validation per the diagram README |
| 6.3 Outcome | Append §8 Outcome with scenario count, runtime per scenario, soak result and three things learned, as Phase 0b did | Codex medium | Numbers come from CI runs, not estimates |
| 6.4 Trace page | `nightfall-replay trace --file <nfr> [--session <entity>] [--fail-tick N] --out trace.html`: one self-contained HTML in the style of [docs/diagrams](../diagrams/README.md): the zone grid, each entity's path over time, hit/miss/crit markers, deaths, respawns, target changes and intention changes, with a tick scrubber and the failing tick pinned; `run-sim.sh` generates one per scenario and the JUnit failure links it | Opus high | Opening the page for a failed `1-chase-leash` run shows the NPC's path and the tick it turned home; the page renders with no network access and under 2 MB for a 90 s scenario |

## 5. Order and estimate

1. E1.1–1.3 runner, steps, sentinel: **3–4 days**. E1.4 hygiene and E1.5 script alongside.
2. E2 Phase 0b scenarios: **2 days** (2.4/2.5 need E4.3's script; write it early, it is small).
3. E3 Phase 1 scenarios: **3–4 days**; 3.3 and 3.6 are the design-heavy ones.
4. E4.1 owner runner can start on day one; E4.2 workflow once E2.1 passes locally: **2–3 days**;
   E4.4 Gauntlet: **2 days**; E4.5 is calendar time, not effort.
5. E5 and E6: **2 days**, E5.1 before the next phase plan is drafted.

6. Diagnostics and coverage (E1.6, E1.7, E3.9, E6.4): **3–4 days**, E1.6 right after E1.1 so every
   later scenario is written with the bundle available; E4.6 last.

**Total: 17–21 focused engineering days (about 4 weeks), 6 epics / 34 stories.** The runner
provisioning (E4.1) is the one owner step and the schedule risk; everything in E1–E3 runs locally
without it.

## 6. Risks

| # | Risk | Mitigation |
|---|------|------------|
| R1 | Flaky timing: 10 Hz ticks, 150 ms interpolation, reconnect backoff make wall-clock assertions brittle | Every wait has a timeout derived from the server constants (tick, interval, delay) with a 3× margin, never a bare sleep; positions use tolerances in tiles; quarantine list with expiry (E4.5) |
| R2 | Headless `-game` differs from the real client (no input, no pawn possession without a world) | `nf.EnterWorld` already loads `L_TestZone` headless and possesses the pawn (README build status); scenarios assert on projections, so a renderer-only regression is out of scope by design and is covered by the existing UE automation tests |
| R3 | Runner is a single point of failure and a maintenance cost | One documented provisioning script; engine version pinned to the project's `EngineAssociation`; the `sim` job is non-blocking until E4.5 so a dead runner never blocks `main` |
| R4 | Scenario count grows faster than CI time | Scenarios are grouped by phase and run in parallel processes against one stack where they do not share characters; the soak is nightly only; target under 10 minutes for the PR set |
| R5 | Test-only commands (`nf.DropSocket`, `nf.FloodMoveTo`, `nf.Gm`) leak into shipping builds | Compiled under `#if !UE_BUILD_SHIPPING` and registered only when `-BotScenario` is present |
| R6 | Bot sessions pollute the replay fixtures or Grafana | Bots use `test:<uuid>` accounts on a throwaway stack; only one recording per phase is promoted to a checked-in fixture, by hand |

## 7. Out of scope for this phase

- An engine-free logic library or any `std::` rewrite of `Net/`, `Auth/`, `Combat/` (D1).
- A Rust bot. Revisit in Phase 9 for load testing beyond what UE processes can reach
  (hundreds of concurrent sessions); it would share `packages/proto` and the API crate's types,
  not client code.
- Screenshot, rendering or UI-layout tests; the Automation Driver; Functional Test maps.
- Line coverage as a gate. A nightly clang source-based coverage build of `Source/Nightfall`
  (`-fprofile-instr-generate -fcoverage-mapping`, `llvm-cov report`) is optional information; the
  tracked numbers are E1.7's contract coverage and E3.9's transition coverage.
- Horde or BuildGraph. Plain workflow jobs until the build graph has more than one platform.
- Windows/Mac runners. Linux only, matching the API's CI.

## 8. Outcome

### 8.1 Verification status and acceptance scope

All metrics, runtimes, and validation results below represent **locally measured evidence** gathered on the local Linux development machine. The persistent self-hosted Linux CI runner (`[self-hosted, unreal]`, Story 4.1) is not yet registered, and no hosted or self-hosted GitHub Actions execution has been performed. Consequently:

- **No registered CI runner**: The host runner is not yet registered in GitHub Actions.
- **No CI run or nightly success is claimed**: Story 4.2 pipeline execution and Story 4.5 two-week zero-flake PR gating remain unmeasured and pending runner availability.
- **Story 6.3 acceptance status**: The original E6.3 acceptance criterion ("Numbers come from CI runs, not estimates") remains formally **pending** until the runner is online and executes the suites; local measured evidence documents the verified baseline.
- **Audited and diagnostic build status at 2e85d3f**: Final audited and diagnostic API and client were built at commit `2e85d3f`:
  - Cargo release profile API build (`nightfall-api v0.1.0`): **52.31 s** (optimized).
  - Unreal editor and staged standalone client via RunUAT BuildCookRun: **85.43 s** (stage command 2.02 s; build succeeded).
  - Earlier, pre-diagnostics regression passed 47/47 Nightfall automation tests on a live stack and 338 Rust library tests plus 2 replay binary tests. Final integrated-source regression is tracked separately below.
  - The full 8-client × 1200-second (20-minute) soak on final integrated source remains pending the serial full-suite execution.

### 8.2 Environment isolation and infrastructure configurations

To avoid conflation between scenario execution and soak testing, two distinct infrastructure configurations were used:

1. **Scenario suite (15 logical execution units)**: Executed against fresh per-unit in-memory API repositories (`AUTH_DEV_TOKENS=1`), real gRPC and WebSocket endpoints, and a fresh NATS JetStream volume/journal per run. To isolate the final corrected scenario pass from concurrent soak and diagnostics runs, dedicated ports were allocated (API 3100, gRPC 50151, NATS 4226).
2. **Gauntlet soak suite**: Executed against a complete, fresh Docker Compose stack with persistent backing services (PostgreSQL with baseline migrations, NATS JetStream, Keycloak identity provider, and LGTM telemetry stack).

### 8.3 Scenario coverage and measured execution runtimes

The test suite contains **19 `.nfs` scenario files** representing **15 logical execution units** (11 single-client scenarios and 4 paired two-client scenarios). The traceability checker reports **25 completed client-visible stories: 23 covered, 2 explicit exceptions, 0 missing**. The exceptions are Phase 0b E1.5 (interactive device authorization and token storage) and E6.2 (browser approval and return-to-game UI); headless dev-token admission does not exercise those interactions. The manifest separately records Phase 1a E3.6 combined live/adversarial evidence outside those completed-story totals.

- **Authentication boundary**: Headless scenarios authenticate via dev tokens (`AUTH_DEV_TOKENS=1`, `nf.Login`); this exercises session admission, play ticket issuance, and WebSocket connection, but does not exercise the interactive browser OIDC device flow or CommonUI login widgets.
- **Combat state and late-entry assertions**: Paired late-entry live scenarios (`1-late-entry-a/b`) verify live reconnection and same-life HP continuity. Stale-fact handling and adversarial lower-incarnation rejections (`IsStaleSpawn`) are validated via dedicated unit and projection automation tests rather than injected by the live server.
- **Coverage status**: Final 19-scenario cross-scenario contract coverage remains pending serial full-suite coordinator execution.

Measured runtimes reflect the latest relevant execution runs from local integration passes (`/tmp/nightfall-phase1a/results.txt`, `/tmp/nightfall-phase1a/retest/results.txt`, and `/tmp/nightfall-phase1a/final/results.txt`). No aggregate CI runtime estimate is provided, as execution depends on runner parallelism and topology.

| Logical Unit | Scenario Files (`apps/client-unreal/Scenarios/`) | Topology | Measurement Stage | Measured Wall Time | Status |
|---|---|---|---|---|---|
| login | `0b-login-enter-world.nfs` | Single (1 bot) | Retest (actor assertions) | 18 s | PASS |
| click | `0b-click-move.nfs` | Single (1 bot) | Retest (reconcile / exactness) | 23 s | PASS |
| move-rejected | `0b-move-rejected.nfs` | Single (1 bot) | Initial | 13 s | PASS |
| reconnect | `0b-reconnect.nfs` | Single (1 bot) | Initial | 10 s | PASS |
| two-clients | `0b-two-clients-a.nfs`, `0b-two-clients-b.nfs` | Paired (2 bots) | Retest | 26 s | PASS |
| login-replaces | `0b-login-replaces-a.nfs`, `0b-login-replaces-b.nfs` | Paired (2 bots) | Initial | 36 s | PASS |
| target-attack | `1-target-attack.nfs` | Single (1 bot) | Initial | 39 s | PASS |
| target-unknown | `1-target-unknown.nfs` | Single (1 bot) | Initial | 10 s | PASS |
| kill | `1-kill-one-monster.nfs` | Single (1 bot) | Retest (reward exactness) | 55 s | PASS |
| chase | `1-chase-leash.nfs` | Single (1 bot) | Final (first-hit wait fix) | 61 s | PASS |
| npc-respawn | `1-npc-respawn.nfs` | Single (1 bot) | Final (corpse/despawn/new-life) | 85 s | PASS |
| die-respawn | `1-die-respawn.nfs` | Single (1 bot) | Initial | 73 s | PASS |
| delevel | `1-delevel.nfs` | Single (1 bot) | Initial | 82 s | PASS |
| late-entry | `1-late-entry-a.nfs`, `1-late-entry-b.nfs` | Paired (2 bots) | Final | 61 s | PASS |
| social | `1-social-aggro-a.nfs`, `1-social-aggro-b.nfs` | Paired (2 bots) | Final (deterministic fixture) | 47 s | PASS |

### 8.4 Gauntlet soak testing results

Nightfall Gauntlet soak testing was validated locally using staged Linux builds against the full Compose stack. Performance validation strictly enforces the Phase 1 E6.2 acceptance budget of **combat tick p99 < 20 ms** over all ticks in the isolated stack (`histogram_quantile(0.99, sum by (le) (nightfall_combat_tick_duration_seconds_bucket))`). The separate **50 ms** threshold is an operational alert threshold, not the acceptance budget.

The first debug runs verified client assertions and byte-identical replay, but the old reporter incorrectly used the 50ms operational alert threshold and `increase()`. Their performance verdicts are superseded: debug 8×120 had a complete cumulative p99 of approximately 35.136ms and fails the actual <20ms budget. An abbreviated release run then exposed same-millisecond UUIDv7 character-name collisions, now fixed with a random suffix. These failed/superseded runs remain historical evidence, not acceptance.

The corrected reporter requires every cumulative tick observation to match the completed recording and propagates API or Compose teardown failures. Intermediate release runs before final audit/diagnostics integration passed:

| Run / artifact directory under `Saved/Soak` | Iterations | Client elapsed time | Gauntlet wall time | Combat p99 | Observed = replayed ticks | Teardown |
|---|---:|---|---:|---:|---:|---|
| `release-2x120` | 7 | client01:4 iterations/156.072s; client02:3/120.562s | 166s | 4.983ms | 1,668 | API 0, Compose 0 |
| `release-8x120-fixed` | 30 | 120.926–158.977s across eight clients | 169s | 19.343ms | 1,697 | API 0, Compose 0 |

Both recordings matched byte-identically with no digest-only records. The two-client recording contained 7 players, 2,574 outputs and 161,652 bytes and replayed in 5.9ms; the eight-client recording contained 30 players, 11,456 outputs and 2,797,183 bytes and replayed in 31.5ms. These are measured intermediate smoke results, not final integrated-source acceptance.

Final audited/diagnostic API and client were built at `2e85d3f` (API release in 52.31s, staged client BuildCookRun in 85.43s). The full eight-client × 1200-second soak remains pending the serial full-suite run. Live combat duration includes durability and checkpoint work; the older OpenGate release microbenchmark measures a different workload and does not substitute for this gate.

### 8.5 Diagnostics and coverage tracking status

Evidence from initial local validation documented in `docs/engineering/sim-diagnostics.md`:

- **E1.6 Failure bundle validation (real seeded failure actual values)**:
  - Seeded kill failure in `1-kill-one-monster`: executed full walk, targeting, and kill, then deliberately waited for `own_hp == 0` for 0.5 s.
  - First failure retained line 33, observed HP 109, wait 0.50026 s, and elapsed scenario time 31.558 s.
  - Bundle contained 50 typed server frames, 56 positions since the last click move, complete own projection (HP/MP/XP/level/target/attack state), and 4 proxy entries with position, HP, and life incarnation.
  - Observed ticks were 405 through 714; JUnit linked the bundle and the trace centered on tick 714. Replay matched 1,245 recorded ticks, 288 outputs, and 15,842 bytes. Process exited 1 as intended.
- **E1.7 Contract coverage baseline floor and regression gate**:
  - Initial historical presence floor established from 5 passing scenarios: death/respawn (`1-die-respawn`, 77 s), target/attack (`1-target-attack`, 44 s), click movement (`0b-click-move`, 47 s), rejected movement (`0b-move-rejected`, 17 s), and kill (`1-kill-one-monster`, 45 s), each with replay and trace checks.
  - Combined observations covered 5/6 intents, 3/3 payloads, 11/12 events, 1/13 rejection reasons, and 0/4 documented close codes.
  - Removal regression verification: removing the only death/respawn report from those measurements caused the gate to exit 1 for `intents.respawn` and `events.entity_respawned`. Synthetic regression tests separately verified that reducing a still-positive count does not fail.
- **Controlled raw server audit comparison**:
  - Fresh dedicated NATS/API run of `0b-move-rejected` passed in 13 s with replay and trace.
  - Runtime descriptor-backed counts and `NF_SESSIONS` wire audit agreed exactly across 1 socket session and 5 audited wire frames: 1 inbound MoveTo and 4 outbound frames (1 checkpoint JSON record excluded).
  - All intent, payload, event, and rejection counters matched with zero differences. Close codes were excluded by documented audit limitation.
- **State transition coverage (E3.9)**: Acceptance aggregation selects the latest passing recording for each of the 19 current scenario files (`summary.json`). NPC intentions cover 7/10 reachable edges (`Idle->Attack`, `Idle->Dead`, and `Active->Dead` unseen). Player attack state covers 3/4 reachable edges (`pending->idle` unseen). 15 NPC and 5 player edges are labeled unreachable.
- **Pending final coverage**: Full 19-scenario cross-scenario contract coverage across the complete suite remains pending serial full-suite coordinator execution.

### 8.6 Three lessons learned

1. **Wait for authoritative projections and check actual fixture mechanics.** A successful transport connection does not prove actor readiness. Combat waits must allow misses and tick phasing; the social fixture must distinguish clan help from ordinary proximity aggro.

2. **Logout does not unload the Unreal world.** Repeated scenarios reload `L_Login` and await map readiness to remove old proxies. The soak completes each final iteration so every started assertion has a verdict; a nominal 120-second client therefore ran as long as 159.0 seconds. Additionally, concurrent bot account rotation revealed that timestamp-prefixed UUIDv7 names caused collisions when clients registered in the same millisecond; resolving this required deriving character names from the trailing 52 random suffix bits (`Hex.Len() - 13`, `BotCharacterName::FromGuid`) to avoid timestamp-prefix collisions across parallel bots.

3. **A fixed seed and a faithful replay are different checks.** Fresh Postgres/NATS storage pins the zone seed to `(1, 1)`, while client identities and arrival order can vary between runs. Replaying the recorded commands verifies that run byte-identically; flushing metrics before teardown separately verifies its performance gate.
