# Phase 1a Plan: Headless Simulation Testing

**Status:** IMPLEMENTED 2026-10-09. The local full-suite verification passed, the projection fix is verified, and the replacement 20-minute soak passed a local verification (exit 0 pass). CI acceptance is pending (E4.1 registration, E4.5 two-week reliability, E6.3 CI-derived results). **Planning model:** Opus. The implementation model and effort are set per story.

## 1. Goal

The goal is a headless Unreal client ("walker") with these properties:

- It uses every line of the real client.
- A scenario script controls it.
- It runs against the real server stack from CI.
- It fails on an assertion, a crash, an ensure or an error log.

Use it for two purposes:

- (a) To back-fill simulation scenarios for all the functions that Phase 0b and Phase 1 released.
- (b) To make "a simulation scenario" a standing deliverable of every client epic from Phase 2 on.

This plan is **not** for an extracted logic library that is free of the engine, and it is not for a Rust bot.
Refer to §2 D1 for the analysis of the options. A Rust load-test bot stays a candidate for the live-operations phase (§7).

## 2. Decisions

| # | Decision | Choice | Rejected | Why |
|---|----------|--------|----------|-----|
| D1 | Bot runtime | The shipped client binary, `-game -nullrhi -nosound -unattended`, with all subsystems live and no renderer | (a) Extract `Net/`, `Auth/`, `Combat/` into an engine-free C++ library and write a separate thin client; (b) a Rust bot that uses `packages/proto` | The client is already thin: ~2,200 lines of protocol/state code have zero `UWorld`/actor references, but they use `FString`/`TMap`/delegates/`IWebSocket`/TurboLink as their base. (a) writes all of that again onto a second network stack, plus a marshaling seam back into UE. This approximately doubles the code for one consumer. (b) uses the contract again, but not the client, thus it cannot find client projection bugs. (a) takes 3–5 weeks. This option takes days, because `nf.Login`/`nf.EnterWorld`/`nf.ClickMove` and `FScopedTestGameInstance` already exist |
| D2 | Driver | A `UBotScenarioRunner` game-instance subsystem runs a scenario file that it gets as `-BotScenario=<path>`. The steps are the existing `nf.*` console commands plus new `nf.Expect*`/`nf.WaitFor`/`nf.Within` steps. The process exits with a code and writes a JUnit report | Gauntlet `UGauntletTestController` as the only driver; Blueprint/Python scripts; Functional Test actors in a map | Console commands are the headless path that the client already has (README "Headless"). Gauntlet must have UAT and a packaged build. The project adopts Gauntlet later for multi-client orchestration (E4.4), not as the entry point. The Python plugin operates only in the editor. Functional Test maps must have a world and a lifecycle with the shape of a renderer, and the bot does not have these |
| D3 | Scenario format | One `.nfs` text file per scenario in `apps/client-unreal/Scenarios/`: one step per line, `#` comments, `nf.*` command syntax, and an explicit timeout for each wait | JSON/YAML, C++ per scenario | It is easy to read in a diff, and it is not necessary to have a parser other than the console. You can paste a scenario into an editor that runs, to reproduce the scenario manually |
| D4 | Oracle | Assertions read the projections of the client (`UCombatStateSubsystem`, `UNetClientSubsystem` cache, `UWorldProxySubsystem` proxies) and the typed events that the server sent. Each run also records its session. `api:replay-check` must replay it byte-identically | Assertions on server internals through a side channel; screenshot comparison | The server is authoritative. Thus the only client-side truth is "what the server said, projected correctly". A replay of the bot session connects the scenario to the deterministic core at no extra cost |
| D5 | Determinism and isolation | Each scenario runs against a new Compose stack (Postgres, NATS, Keycloak) and API with `AUTH_DEV_TOKENS=1`. Bots use `test:<uuid>` accounts. The zone seed is pinned per scenario. The NPC spawn tables are the Phase 1 fixtures | A shared server with a long life; real Keycloak logins per bot | Runs that you can reproduce, and parallel scenarios that cannot see each other |
| D6 | Pass criteria | Every `nf.Expect` is true within its timeout; zero `Error`/`Fatal` lines in `LogNightfall`, `LogTurboLink` and `LogNet*`; zero ensures; a clean exit; wall-clock time less than the scenario budget | Only assertions | A hang or an ensure is a bug that the assertions do not see |
| D7 | CI placement | A `sim` job on a **self-hosted Linux runner** with UE 5.8 installed (one owner step: set up the runner). It runs on `main` and nightly first. It becomes a PR gate after two weeks without flakes. The multi-client soak runs only nightly | GitHub-hosted runners; Epic container images | A UE install plus DDC is larger than the disk of a hosted runner. The container images must have an Epic-linked GitHub org and a registry pull for each run. A persistent runner keeps the engine, DDC and TurboLink libraries warm |

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

**Runner lifecycle.** `UBotScenarioRunner::Initialize` reads `-BotScenario`. If the flag is not there, the
subsystem does nothing, thus the shipped game does not change. The steps run in sequence on the game
thread tick. A wait step polls its predicate each tick until the predicate is true or the timeout occurs.
The runner stops at the first failed step. It writes `Saved/Sim/<scenario>.xml` (JUnit, one testcase per
`nf.Expect`/`nf.WaitFor`) and `Saved/Sim/<scenario>.log`. Then it calls `FPlatformMisc::RequestExit`
with 0/1. An `FOutputDevice` sentinel counts `Error`/`Fatal` lines and ensures during the run.
If the count is more than zero, the scenario fails, also when every step passed (D6).

**Predicates** are named functions of the projections, for example `connected`, `in_world`,
`own_at <x> <y> <tol>`, `proxies >= <n>`, `target == <name|none>`, `target_hp < <n>`,
`own_hp < <n>`, `own_dead`, `own_level == <n>`, `xp_known`, `attack_state == <idle|pending|active>`,
`damage_numbers >= <n>`, `rejected == <reason>`, `last_ack_seq == <n>`. The list becomes longer with each phase.
Each predicate is one line of code over `UCombatStateSubsystem::BuildHudModel()` or the net cache.

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
`coverage.transitions.json` (E3.9). It prints the two files, with the traceability check (E5.1), in the job
summary. Here, coverage means contract coverage and state-transition coverage, not line coverage.
§7 lists a nightly clang source-based coverage build of the `Nightfall` module as optional.

**Multi-client scenarios.** `run-sim-multi.sh` starts N processes with per-process scenarios
and a shared `-SimGroup=<id>`. Coordination between clients uses the server (a client waits until
`proxies >= 2`), never IPC. For the nightly soak, Gauntlet (E4.4) replaces the script. Gauntlet
adds role assignment, per-process timeouts and log collection.

**Test hygiene that this plan includes (E1.4).** The existing automation tests move from `EngineFilter` to
`ProductFilter`. As a result, `Automation RunTests Nightfall` selects only the Nightfall tests. Live tests
(`*.EndToEnd`, `SessionClient.Ping`) get `-RequireLiveApi`. In CI, this flag changes their "API not reachable,
skipped" warning into a failure. Thus a server that is down can never show green.

## 4. Epics, stories, tasks

Models: **Sonnet medium** for mechanical work, **Opus high** for work with much design,
**Codex high/medium** for integration, review and docs. Each Done when includes tests.
Scenarios count as the **W** (real-socket) row of the
[API guidelines §4](../engineering/api-guidelines.md#4-required-tests-per-endpoint) matrix for
client-facing behaviour. They do not replace U/A on the server or the existing UE automation tests.

> [!NOTE]
> The `[client-visible]` tag identifies client-observable contract and state behavior. The simulation traceability checker examines the completed tagged stories against scenario coverage (`.nfs`). Parts that are only rendering keep automation and cook validation. But the tags stay on those stories, and the cases without coverage are explicitly reported. New plans must add tags independently before they write scenarios.

### E1: Bot runtime

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 1.1 Scenario runner | `UBotScenarioRunner`: parse of `-BotScenario`, line reader, step execution in sequence on tick, stop at the first failure, JUnit + log writers, `RequestExit` with code; does nothing without the flag | Opus high | An automation test gives a scenario string and asserts step order, timeout failure, exit code and JUnit shape. `-game -nullrhi` with no flag boots with no change |
| 1.2 Step and predicate commands | `nf.WaitFor`, `nf.Expect`, `nf.Within`, `nf.Sleep`; the predicates that §3 lists, over the Phase 0b/1 projections; `nf.Target <id|nearest_attackable|none>`, `nf.Attack`, `nf.StopAttack`, `nf.Respawn` | Sonnet medium | Automation tests operate each predicate from recorded events (use the Story 5.2 fixtures again) and assert true/false transitions. Bad predicate names cause the scenario to fail at parse time, not at runtime |
| 1.3 Log and ensure sentinel | `FOutputDevice` that counts `Error`/`Fatal` on the listed categories and ensures; one allow-list file per scenario for known noisy lines, each entry with a reason | Sonnet medium | A scenario that passes every step but logs one error exits 1 and names the line in JUnit. A line on the allow-list passes |
| 1.4 Test hygiene | `ProductFilter` on all Nightfall automation tests; `-RequireLiveApi` on live tests; `run-tests.sh` gives the flag in CI | Sonnet medium | If the API is down and the flag is set, `Nightfall.Login.EndToEnd` fails. Without the flag, it still skips with a warning |
| 1.5 `run-sim.sh` and moon task | Script: finds `UE_ROOT`, builds the game target if it is stale, starts or attaches to the API, runs one scenario or a glob, copies `Saved/Sim/*` and the session recording to an artifacts dir; `moon run client-unreal:sim -- <scenario>` | Codex medium | If you run the E2.1 scenario locally from a clean checkout with Compose up, it makes a JUnit file and exit 0. A scenario that is broken on purpose exits 1 |
| 1.6 Failure bundle | At the first failed step, write `Saved/Sim/<scenario>.failure.json` and put a summary in the JUnit `<failure>`. The summary has: the step, the predicate, its last evaluated value and the wait time; the last 50 typed events with server tick and arrival time; a projection snapshot (own HP/MP/XP/level/target/attack state; every proxy with position, HP, incarnation); the own-position history since the last `nf.ClickMove`; the server tick range of the scenario and the own entity id, so that `nightfall-replay --session <entity>` reproduces it from the recording | Sonnet medium | From the bundle of a seeded failure in `1-kill-one-monster`, a reviewer finds the failing tick without the UE log. The bundle schema has a version and a test |
| 1.7 Contract coverage counter | Count every `ClientMessage` intent sent and every `ServerMessage` event, `IntentRejected` reason and close code received. Enumerate them from the proto descriptors, so that new oneof cases automatically show as "never seen". Emit them as JUnit `<properties>`. `run-sim.sh` merges them across scenarios into `coverage.contract.json`. It fails when a message or reason that has coverage on `main` does not have coverage now | Sonnet medium | The report lists intents/events/reasons as `n/N` with the missing names. If you remove the only scenario that covers `Respawn`, the merge fails. The session-log count of the server for the same run agrees |

### E2: Back-fill scenarios for Phase 0b

Each scenario covers a Phase 0b story that was released. The names are files in `Scenarios/`.

| Story | Scenario(s) | Model | Done when |
|-------|-------------|-------|-----------|
| 2.1 Login and enter world | `0b-login-enter-world`: dev-token login, character list, make a character if the list is empty, play ticket, WS connect, own `EntitySpawn`, own pawn at spawn tile | Sonnet medium | Passes against the Compose stack. The `connected` and `in_world` predicates are the gate for each later scenario |
| 2.2 Click to move | `0b-click-move`: three `nf.ClickMove`, each one Acked, own position agrees within 1 tile; `0b-move-rejected`: a destination > 64 tiles gets `IntentRejected`, and the position does not change | Sonnet medium | Both pass. The rejection scenario asserts `rejected == <reason>` and no move |
| 2.3 Reconnect with a new ticket | `0b-reconnect`: stop the socket with `nf.DropSocket` (new, test-only command). Assert that the reconnect got a new ticket and is `in_world` again, and that the projections were built again from the spawn stream | Sonnet medium | Passes. It asserts that the ticket is different and that the entity id does not change |
| 2.4 Two clients see each other | `0b-two-clients-a/b` with `run-sim-multi.sh`: each client waits for `proxies >= 1`. A moves, and B sees the proxy of A within 2 tiles of the destination after the interpolation delay. A logs out, and B sees the despawn | Codex medium | Passes with two processes. The despawn assertion is the one that finds a leaked proxy |
| 2.5 Logout replacement | `0b-login-replaces`: a second login for the same character while the first is connected. The first is disconnected with the documented reason, and the second is `in_world` | Sonnet medium | Passes. The test asserts the disconnect reason of the first process, not only the drop |

### E3: Back-fill scenarios for Phase 1

| Story | Scenario(s) | Model | Done when |
|-------|-------------|-------|-----------|
| 3.1 Target and attack | `1-target-attack`: `nf.Target nearest_attackable` → `TargetChanged`, HUD target frame; `nf.Attack` → one Ack, `attack_state == active`; a repeated `nf.Attack` does not make a second Ack; `nf.StopAttack` → idle. `1-target-unknown`: the server rejects an unknown id with `UNKNOWN_ENTITY`, and the target stays none | Sonnet medium | Both pass. They replace the parts of `CombatEndToEndTest` that skip frequently |
| 3.2 Kill, XP, level | `1-kill-one-monster` (§3 example): damage numbers show one time per `AttackResult`, target HP becomes 0, the target clears, `XpGained` sets `xp_known`, and the level agrees with the XP table threshold for the fixture reward | Sonnet medium | Passes. The test calculates the asserted level from `experience.toml`. The level is not hard-coded |
| 3.3 Chase and leash | `1-chase-leash`: target an NPC and walk away inside aggro range. Assert that the NPC proxy follows (the distance decreases). Then stop beyond leash range and assert that the NPC goes back home (the proxy position is within tolerance of its spawn) | Opus high | Passes. The tolerances come from the NPC template speed and leash values in `packages/data` |
| 3.4 Player death and respawn | `1-die-respawn`: stand in the aggro of an NPC with no attack until `own_dead`; the dead overlay model is true; `nf.Respawn` → own HP = `floor(maxHP*65/100)`, MP 0, position at town spawn, target none, attack idle; `nf.Attack` stops the protection (the next NPC hit that lands shows this) | Sonnet medium | Passes. The HP assertion uses the E1 formula vector, not a literal |
| 3.5 Delevel | `1-delevel`: get to level 2 through 3.2, then die. Assert that the XP loss is equal to the HF table row and that the level goes back to 1 | Sonnet medium | Passes. A changed `penalties.toml` row causes it to fail |
| 3.6 Late AOI entry and stale facts | `1-late-entry-a/b`: A fights an NPC. B enters the world during the fight and asserts that the spawn has the current HP/life. A disconnects and reconnects during the fight. A asserts `XP --` until the next `XpGained`, and asserts that the re-sent spawn does not reset the HP of the NPC (stale incarnation rejection) | Opus high | Passes with two processes. The stale-spawn assertion is the one that finds a regression in `IsStaleSpawn` |
| 3.7 Social aggro | `1-social-aggro-a/b`: two NPC instances. A attacks one NPC. B stands in range of the other NPC and asserts that this NPC attacks B (hate from social aggro) | Sonnet medium | Passes. It must have the two-instance fixture from Phase 1 D1 |
| 3.8 Replay of a bot session | After the run, `api:replay-check` replays the session recording of every E2/E3 scenario | Codex medium | A divergence in a bot session causes the `sim` job to fail. The recording of one scenario is checked in as a fixture under the existing schema policy |
| 3.9 Transition coverage from recordings | `nightfall-replay coverage --file <nfr>`: counts every NPC intention transition (Idle/Active/Attack/ReturnHome/Dead), player attack-state transition and life-incarnation change in a recording, against the full transition table from `domain/zone`. `run-sim.sh` merges the per-scenario counts into `coverage.transitions.json`. The job summary prints covered/total and names the transitions that were never seen | Opus high | If you run it over the E3 set, it reports every transition that the Phase 1 state machine defines. A transition that the AI cannot reach shows in the table as unreachable, and it is not silently absent. A unit test on `two-players-fight-v2.nfr` pins its counts |

### E4: Pipeline

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 4.1 (owner) Self-hosted runner | **One owner action:** a Linux box or VM with UE 5.8.3, clang toolchain, Docker and the GitHub runner agent, with the label `unreal`; `UE_ROOT` set; TurboLink prebuilt libraries and DDC warmed one time | owner | `moon run client-unreal:build` and `client-unreal:test-editor` pass on the runner when a person runs them manually |
| 4.2 `sim` workflow | `.github/workflows/sim.yml` on `runs-on: [self-hosted, unreal]`: checkout with submodules, Compose services, API with `AUTH_DEV_TOKENS=1`, build of the game target, `run-sim.sh Scenarios/*.nfs`, `api:replay-check` on the recordings, JUnit upload, and logs and recordings as artifacts on failure. Triggers: push to `main`, nightly, `workflow_dispatch`, and PRs with the label `sim` | Codex high | Green on `main` for the E2/E3 set. A seeded failing scenario shows as a failed testcase with the step and predicate in the summary |
| 4.3 Multi-client script | `run-sim-multi.sh` as in §3; 2.4, 2.5, 3.6 and 3.7 use it | Sonnet medium | The four two-client scenarios run under it in CI |
| 4.4 Gauntlet nightly soak | `BuildCookRun` to a Linux staged build; a `NightfallSoak` Gauntlet test that starts N clients (default 8). Each client runs `1-kill-one-monster` in a loop for 20 minutes against one stack. The test collects per-client logs and the combat telemetry of the server. It fails on a crash, an ensure, an assertion or a tick p99 more than the Phase 1 budget | Opus high | Nightly passes with 8 clients. The soak report links the Grafana dashboard range |
| 4.5 Flake policy and PR gating | A quarantine list with owner and expiry. A scenario on the list runs, but it does not cause the job to fail. After two weeks with zero failures outside the quarantine, the `sim` job becomes necessary on PRs that change `apps/client-unreal/**`, `apps/api/**` or `packages/**` | Codex medium | Branch protection is updated. The list is empty, or every entry has an expiry |
| 4.6 Video on failure | After E4.4. When a scenario fails, `run-sim.sh --video` runs it again one time with rendering under Xvfb + Mesa lavapipe (`-RenderOffscreen -DumpMovie -ForceRes -ResX=960 -ResY=540`). It encodes the frames with ffmpeg and attaches the `.mp4` to the failure artifacts. It never does this on runs that pass. It never does this on the PR set until a measurement shows less than 2 minutes per scenario | Sonnet medium | A seeded failure in CI uploads a video of that scenario that can play. The JUnit of the second run is kept separate, thus the pass/fail verdict comes from the headless run |

### E5: Forward coverage for Phases 2–9

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 5.1 Standing rule and traceability check | Add to the [planning README](../planning/README.md) conventions and to the phase-plan template: every client epic has a story "Simulation scenario(s)" whose Done when names the `.nfs` files, and a phase cannot close while that story is open. Every `.nfs` starts with `# covers: <phase> <story>, ...`. `Scripts/sim-traceability.sh` parses those headers and the story tables of the plan files. It prints every ✅ story tagged client-visible that has no scenario that covers it. It runs in the `sim` job summary | Codex medium | The README and the E5 of the Phase 1 plan have the rule. The next phase plan follows it. After E2/E3 merge, the check lists zero 0b/1 stories without coverage. The check causes the job to fail for a newly completed story that has no scenario |
| 5.2 Scenario seeds per phase | A table in this document (§4.1 below) that names the first scenario that each later phase must release. Thus the predicates and `nf.*` commands that they need are known before that phase starts | Opus high | The table is examined against the client implications section of each planning doc |
| 5.3 Predicate backlog | For each seed, the predicates/commands that are not yet in E1.2 (for example `inventory_has`, `skill_ready`, `party_size`, `in_zone`), listed as the first task of the scenario story of that phase | Sonnet medium | The backlog cross-links each predicate to the proto message that it reads |

#### 4.1 Scenario seeds and predicate backlog per phase

| Phase | Document | First scenario(s) | New predicates / commands | Proto message / contract status |
|-------|----------|-------------------|---------------------------|---------------------------------|
| 2 Race and class | [02](../planning/02-race-and-class.md) | `2-create-each-race-class`: make one character per race/class, assert the starting stats from the class tables; `2-class-transfer`: get to the transfer level through an XP fixture, transfer, assert the stat delta | `own_stat <name> == <n>`, `nf.CreateCharacter <race> <class>`, `nf.ClassTransfer` | Base stats through [`Character.stats`](../../packages/proto/nightfall/v1/game.proto#L93) ([`BaseStats`](../../packages/proto/nightfall/v1/game.proto#L79)), race through [`CreateCharacterRequest.race`](../../packages/proto/nightfall/v1/game.proto#L67); class selection and transfer are **explicitly future contract** |
| 3 Combat and skills | [03](../planning/03-combat-and-skills.md) | `3-cast-skill`: learn, cast, assert cooldown and MP cost; `3-buff-expires`: apply a buff, let the duration elapse, the stat goes back; `3-pvp-flag`: two clients, attack flags, karma on kill | `skill_ready <id>`, `buff_active <id>`, `own_flag == <pvp|karma>`, `nf.Cast <id>` | **Explicitly future contract** (no skill, buff, or PvP/karma proto messages in `packages/proto`; the [`ClientMessage`](../../packages/proto/nightfall/v1/world.proto#L32) of this time supports only basic melee) |
| 4 Items and equipment | [04](../planning/04-items-and-equipment.md) | `4-equip-weapon`: take a fixture drop, equip, P.Atk changes; `4-enchant-fail`: enchant to failure, the item is destroyed, a replay asserts the ledger row | `inventory_has <template> >= <n>`, `equipped <slot> == <template>`, `nf.Equip`, `nf.Enchant` | **Explicitly future contract** (no inventory, item, or equipment messages exist in `packages/proto`) |
| 5 Economy and crafting | [05](../planning/05-economy-and-crafting.md) | `5-kill-loot`: kill, loot shows, take it; `5-vendor-roundtrip`: sell, buy, the adena total stays the same; `5-trade-a/b`: two clients trade, and the two inventories assert | `adena == <n>`, `trade_state`, `nf.Loot`, `nf.Trade` | **Explicitly future contract** (no currency, loot, vendor, or trade messages exist in `packages/proto`) |
| 6 World and content | [06](../planning/06-world-and-content.md) | `6-zone-travel`: walk to a zone edge, assert the zone change and the new spawn stream; `6-quest-chain`: accept, kill N, turn in; `6-instance-a/b`: a party enters an instance, isolated from a third client | `in_zone == <id>`, `quest_state <id>`, `nf.Travel`, `nf.AcceptQuest` | Movement through [`MoveToRequest`](../../packages/proto/nightfall/v1/world.proto#L46) and [`EntityMove`](../../packages/proto/nightfall/v1/world.proto#L169); zone transitions, instances, and quests are **explicitly future contract** |
| 7 Social systems | [07](../planning/07-social-systems.md) | `7-party-xp-a/b`: a party of two kills, and the XP splits per the rule; `7-clan-create`: make a clan, invite, roster; `7-siege-smoke`: N clients, the siege starts and stops on schedule | `party_size`, `clan == <name>`, `nf.Invite`, `nf.AcceptParty` | **Explicitly future contract** (no party or clan proto messages exist in `packages/proto`) |
| 8 Client presentation | [08](../planning/08-client-presentation.md) | Mostly out of headless scope; `8-localization`: every HUD string resolves in two locales (no `??` placeholders); `8-input-remap`: a remapped action still sends the intent | `hud_text <field> matches`, `nf.SetLocale` | Comes from the client projection of [`EntitySpawn.name`](../../packages/proto/nightfall/v1/world.proto#L149) or [`Character.name`](../../packages/proto/nightfall/v1/game.proto#L90); `nf.SetLocale` is only client-side (no server proto) |
| 9 Live operations | [09](../planning/09-live-operations.md) | `9-gm-teleport`: a GM command moves a bot; `9-rate-limit`: the server throttles an intent flood with the documented rejection; `9-patch-restart`: a server restart during a session, the bot reconnects and the state agrees; load: promote the Gauntlet soak to the target session count of the phase, or build the Rust bot (§7) if UE processes cannot get to that count | `nf.Gm <cmd>`, `rejected == RATE_LIMITED`, `nf.FloodMoveTo <n>` | Rate limits through [`IntentRejected`](../../packages/proto/nightfall/v1/world.proto#L100) with [`RejectReason::REJECT_REASON_RATE_LIMITED`](../../packages/proto/nightfall/v1/world.proto#L112) and [`MoveToRequest`](../../packages/proto/nightfall/v1/world.proto#L46); GM commands are **explicitly future contract** |

##### Predicate backlog (Story 5.3 detailed mapping)

The backlog below gives the wire contract for every predicate and console command that the phase seeds add. Note: at this time, `packages/proto` has only `game.proto`, `session.proto`, and `world.proto` (basic movement, session admission, and the Phase 1 melee combat core). There are no inventory, item, equipment, skill, buff, party, clan, trade, or quest proto messages yet. These are explicitly designated as future contracts, and each applicable phase will define them.

| Predicate / command | Target phase | Current proto mapping / wire contract | Status |
|---------------------|--------------|--------------------------------------|--------|
| `own_stat <name> == <n>` | Phase 2 | Base stats: [`Character.stats`](../../packages/proto/nightfall/v1/game.proto#L93) ([`BaseStats`](../../packages/proto/nightfall/v1/game.proto#L79)); resource/level state: [`StatsChanged`](../../packages/proto/nightfall/v1/world.proto#L249); the other derived combat stats must have a future owner-only contract | Existing base/resource proto; extended derived stats are future contract |
| `nf.CreateCharacter <race> <class>` | Phase 2 | Name and race: [`CreateCharacterRequest`](../../packages/proto/nightfall/v1/game.proto#L60) and enum [`Race`](../../packages/proto/nightfall/v1/game.proto#L70); class selection is not in proto | Existing for race; class is **explicitly future contract** |
| `nf.ClassTransfer` | Phase 2 | No class transfer RPC or intent in `packages/proto` | **Explicitly future contract** (Phase 2) |
| `skill_ready <id>` | Phase 3 | No skill messages exist in `packages/proto` | **Explicitly future contract** (Phase 3) |
| `buff_active <id>` | Phase 3 | No buff or effect messages exist in `packages/proto` | **Explicitly future contract** (Phase 3) |
| `own_flag == <pvp\|karma>` | Phase 3 | Not in [`EntitySpawn`](../../packages/proto/nightfall/v1/world.proto#L147) or in a different proto message | **Explicitly future contract** (Phase 3) |
| `nf.Cast <id>` | Phase 3 | [`ClientMessage`](../../packages/proto/nightfall/v1/world.proto#L32) has no skill cast intent (only melee [`AttackRequest`](../../packages/proto/nightfall/v1/world.proto#L68)) | **Explicitly future contract** (Phase 3) |
| `inventory_has <template> >= <n>` | Phase 4 | No inventory or item messages exist in `packages/proto` | **Explicitly future contract** (Phase 4) |
| `equipped <slot> == <template>` | Phase 4 | No equipment slot or item messages exist in `packages/proto` | **Explicitly future contract** (Phase 4) |
| `nf.Equip` | Phase 4 | No item equip intent or RPC exists in `packages/proto` | **Explicitly future contract** (Phase 4) |
| `nf.Enchant` | Phase 4 | No enchant intent or RPC exists in `packages/proto` | **Explicitly future contract** (Phase 4) |
| `adena == <n>` | Phase 5 | No currency or economy proto messages exist in `packages/proto` | **Explicitly future contract** (Phase 5) |
| `trade_state` | Phase 5 | No trade proto messages exist in `packages/proto` | **Explicitly future contract** (Phase 5) |
| `nf.Loot` | Phase 5 | No loot or pickup intent in [`ClientMessage`](../../packages/proto/nightfall/v1/world.proto#L32) | **Explicitly future contract** (Phase 5) |
| `nf.Trade` | Phase 5 | No player trade intent or RPC in `packages/proto` | **Explicitly future contract** (Phase 5) |
| `in_zone == <id>` | Phase 6 | Coordinates through [`Position`](../../packages/proto/nightfall/v1/game.proto#L97) and [`EntityMove`](../../packages/proto/nightfall/v1/world.proto#L169); zone boundary handoff / zone IDs are not in proto | **Explicitly future contract** for zone ID (Phase 6) |
| `quest_state <id>` | Phase 6 | No quest proto messages exist in `packages/proto` | **Explicitly future contract** (Phase 6) |
| `nf.Travel` | Phase 6 | Movement uses [`MoveToRequest`](../../packages/proto/nightfall/v1/world.proto#L46); zone transfer / teleport is not in proto | **Explicitly future contract** for travel (Phase 6) |
| `nf.AcceptQuest` | Phase 6 | No quest RPC or intent exists in `packages/proto` | **Explicitly future contract** (Phase 6) |
| `party_size` | Phase 7 | No party proto messages exist in `packages/proto` | **Explicitly future contract** (Phase 7) |
| `clan == <name>` | Phase 7 | No player clan proto messages exist in `packages/proto` | **Explicitly future contract** (Phase 7) |
| `nf.Invite` | Phase 7 | No party or clan invite RPC/intent in `packages/proto` | **Explicitly future contract** (Phase 7) |
| `nf.AcceptParty` | Phase 7 | No party accept RPC or intent in `packages/proto` | **Explicitly future contract** (Phase 7) |
| `hud_text <field> matches` | Phase 8 | Projection of [`EntitySpawn.name`](../../packages/proto/nightfall/v1/world.proto#L149) or [`Character.name`](../../packages/proto/nightfall/v1/game.proto#L90) with local client localization | Existing proto (for names); presentation check |
| `nf.SetLocale` | Phase 8 | A setting command that is only client-side | Client-side only (a server proto is not necessary) |
| `nf.Gm <cmd>` | Phase 9 | No GM/admin service or message exists in `packages/proto` | **Explicitly future contract** (Phase 9) |
| `rejected == RATE_LIMITED` | Phase 9 | [`IntentRejected`](../../packages/proto/nightfall/v1/world.proto#L100) with [`RejectReason::REJECT_REASON_RATE_LIMITED`](../../packages/proto/nightfall/v1/world.proto#L112) | Existing proto enum |
| `nf.FloodMoveTo <n>` | Phase 9 | Sends repeated [`MoveToRequest`](../../packages/proto/nightfall/v1/world.proto#L46) inside [`ClientMessage`](../../packages/proto/nightfall/v1/world.proto#L32) | Existing proto intent |

### E6: Docs

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 6.1 Client README | "Simulation" section: how to write a scenario, the predicate list, how to run one locally, how to read the JUnit, quarantine | Codex medium | A person can write a new scenario from the README alone (a reviewer tries this) |
| 6.2 Pipeline diagram | `docs/diagrams/simulation-pipeline.html`: PR/nightly lanes, Compose stack, bot processes, replay-check, artifacts; the diagram README links to it | agy medium | Docs-only validation per the diagram README |
| 6.3 Outcome | Add §8 Outcome with the scenario count, the runtime per scenario, the soak result and three lessons, as Phase 0b did | Codex medium | Numbers come from CI runs, not estimates |
| 6.4 Trace page | `nightfall-replay trace --file <nfr> [--session <entity>] [--fail-tick N] --out trace.html`: one self-contained HTML page in the style of [docs/diagrams](../diagrams/README.md). It shows the zone grid, the path of each entity over time, hit/miss/crit markers, deaths, respawns, target changes and intention changes, with a tick scrubber and the failing tick pinned. `run-sim.sh` makes one page per scenario, and the JUnit failure links it | Opus high | If you open the page for a failed `1-chase-leash` run, it shows the path of the NPC and the tick when it turned home. The page renders with no network access and is less than 2 MB for a 90 s scenario |

## 5. Order and estimate

1. E1.1–1.3 runner, steps, sentinel: **3–4 days**. Do E1.4 hygiene and the E1.5 script at the same time.
2. E2 Phase 0b scenarios: **2 days** (2.4/2.5 must have the E4.3 script. Write it early, because it is small).
3. E3 Phase 1 scenarios: **3–4 days**. 3.3 and 3.6 are the stories with the most design.
4. The E4.1 owner runner can start on day one. Start the E4.2 workflow when E2.1 passes locally: **2–3 days**;
   E4.4 Gauntlet: **2 days**; E4.5 is calendar time, not effort.
5. E5 and E6: **2 days**. Do E5.1 before the next phase plan is written.

6. Diagnostics and coverage (E1.6, E1.7, E3.9, E6.4): **3–4 days**. Do E1.6 immediately after E1.1, so that
   the bundle is available for every later scenario. Do E4.6 last.

**Total: 17–21 focused engineering days (approximately 4 weeks), 6 epics / 34 stories.** The runner
setup (E4.1) is the one owner step and the schedule risk. Everything in E1–E3 runs locally
without it.

## 6. Risks

| # | Risk | Mitigation |
|---|------|------------|
| R1 | Unstable (flaky) timing: 10 Hz ticks, 150 ms interpolation and reconnect backoff make wall-clock assertions unreliable | Every wait has a timeout that comes from the server constants (tick, interval, delay) with a 3× margin, never a bare sleep. Positions use tolerances in tiles. There is a quarantine list with expiry (E4.5) |
| R2 | Headless `-game` is different from the real client (no input, no pawn possession without a world) | `nf.EnterWorld` already loads `L_TestZone` headless and possesses the pawn (README build status). Scenarios assert on projections. Thus a regression that is only in the renderer is out of scope by design, and the existing UE automation tests cover it |
| R3 | The runner is a single point of failure and a maintenance cost | One documented setup script; the engine version is pinned to the `EngineAssociation` of the project; the `sim` job does not block until E4.5, thus a dead runner never blocks `main` |
| R4 | The scenario count increases faster than CI time | Scenarios are grouped by phase. They run in parallel processes against one stack when they do not share characters. The soak is only nightly. The target is less than 10 minutes for the PR set |
| R5 | Test-only commands (`nf.DropSocket`, `nf.FloodMoveTo`, `nf.Gm`) go into shipping builds by accident | Compiled under `#if !UE_BUILD_SHIPPING` and registered only when `-BotScenario` is present |
| R6 | Bot sessions add unwanted data to the replay fixtures or Grafana | Bots use `test:<uuid>` accounts on a temporary stack. Only one recording per phase becomes a checked-in fixture, and a person does this manually |

## 7. Out of scope for this phase

- An engine-free logic library or any `std::` rewrite of `Net/`, `Auth/`, `Combat/` (D1).
- A Rust bot. Examine this again in Phase 9 for load tests beyond what UE processes can get to
  (hundreds of concurrent sessions). Such a bot will share `packages/proto` and the types of the API crate,
  not client code.
- Screenshot, rendering or UI-layout tests; the Automation Driver; Functional Test maps.
- Line coverage as a gate. A nightly clang source-based coverage build of `Source/Nightfall`
  (`-fprofile-instr-generate -fcoverage-mapping`, `llvm-cov report`) is optional information. The
  numbers that the project records are the E1.7 contract coverage and the E3.9 transition coverage.
- Horde or BuildGraph. Use plain workflow jobs until the build graph has more than one platform.
- Windows/Mac runners. Linux only, the same as the CI of the API.

## 8. Outcome

### 8.1 Verification status and acceptance scope

All metrics, runtimes, and validation results below are **locally measured evidence** from the local Linux development machine. The persistent self-hosted Linux CI runner (`[self-hosted, unreal]`, Story 4.1) is not registered yet. No hosted or self-hosted GitHub Actions run occurred. As a result:

- **No registered CI runner**: The host runner is not registered in GitHub Actions yet.
- **No CI run, CI success, or 2-week reliability claimed**: The Story 4.2 pipeline run, the hosted/nightly CI runs, and the Story 4.5 two-week zero-flake PR gate are not measured. They wait until the runner is available.
- **Story 6.3 acceptance status**: The original E6.3 acceptance criterion ("Numbers come from CI runs, not estimates") stays formally **pending** until runner registration and a CI run. The local coordinator evidence gives the authoritative baseline.
- **Authoritative coordinator execution**: The final test coordinator ran all **15/15 logical units**, with **19 scenario files**. It had **0 quarantine, 0 assertion failures, and 0 infrastructure errors** in **1180.417 seconds wall time**. Each unit ran against a new, full Docker Compose stack (PostgreSQL, NATS JetStream, Keycloak, LGTM) and a real headless Unreal client (`-nullrhi -nosound -unattended -BotScenario=...`).
- **R4 runtime target remains unmet locally**: The serial fresh-stack coordinator took **19m40.417s**. This is more than the planned PR-set target of less than 10 minutes. Each unit includes a new PostgreSQL/NATS/Keycloak/LGTM startup. CI timing is not measured, and this document does not claim that the PR runtime budget passed.
- **Audited and diagnostic build status at 2e85d3f**: The final audited and diagnostic API and client were built at commit `2e85d3f`:
  - Cargo release profile API build (`nightfall-api v0.1.0`): **52.31 s** (optimized).
  - Unreal editor and staged standalone client through RunUAT BuildCookRun: **85.43 s** (stage command 2.02 s; the build passed).
  - The final root editor/game builds passed in 14.13 s / 29.19 s. Nightfall automation passed **54/54** with `-RequireLiveApi` and zero live skips (`LiveSkip`). Rust passed **341 library tests**, the integration and binary suites, and strict Clippy.
- **Audited soak status**: The first full-duration try failed after 574 seconds because of a missing damage cue. Fix `612acad` was examined and merged to `main` at `8147841`. The raw-decoder regression, eight combat-state tests, the focused kill/late-entry runs and the corrected eight-client smoke run all passed. The replacement 20-minute run (`release-8x1200-damage-fixed`) passed locally (overall exit 0) on all 8 clients. Its cumulative combat tick p99 was strictly less than the 20 ms budget. §8.4 gives the full run metrics. It also keeps the historical evidence of the failed run and the measured smoke results.

### 8.2 Environment isolation and infrastructure configurations

The simulation tests used three different infrastructure configurations:

1. **Initial historical in-memory iterations**: Early test iterations ran against new per-unit in-memory API repositories (`AUTH_DEV_TOKENS=1`), real gRPC and WebSocket endpoints, and a new NATS JetStream volume/journal per run. They used dedicated port allocations (API 3100, gRPC 50151, NATS 4226) for process isolation.
2. **Authoritative final simulation suite (15 logical execution units, 19 scenario files)**: The final coordinator runs each logical unit against its own dedicated, new, full Docker Compose stack. The stack has persistent backing services (PostgreSQL with baseline migrations, NATS JetStream, Keycloak identity provider, and LGTM telemetry stack) and a real headless Unreal client. For each measured unit, the coordinator resets the Compose volumes of the previous unit, starts and migrates the new stack, and runs the clients. Then it stops and examines the API, and it does the replay, trace, coverage, verdict and log collection. The last Compose stack was kept for the controlled audit. After the 1180.417s coordinator measurement, it was removed separately with success.
3. **Staged Gauntlet soak suite**: Long-duration load tests (`NightfallSoak`) run staged standalone Linux client builds against a complete, new Docker Compose stack. The stack has persistent backing services (PostgreSQL, NATS, Keycloak, LGTM). These tests measure multi-client concurrent combat performance and metric stability.

### 8.3 Scenario coverage and measured execution runtimes

The test suite has **19 `.nfs` scenario files** in **15 logical execution units** (11 single-client scenarios and 4 paired two-client scenarios). Traceability reports exactly **25 completed client-visible stories: 23 covered, 2 explicit exceptions, 0 missing**. The two exceptions are Phase 0b E1.5 (interactive device authorization and token storage) and E6.2 (browser approval and return-to-game UI). Headless dev-token admission does not test those browser/UI interactions. The manifest records the Phase 1a E3.6 combined live/adversarial evidence separately, outside those completed-story totals.

- **Authentication boundary**: Headless scenarios authenticate with dev tokens (`AUTH_DEV_TOKENS=1`, `nf.Login`). This tests session admission, play ticket issue, and WebSocket connection. The interactive browser OIDC device flow and the CommonUI widgets stay out of headless scope.
- **Combat state and late-entry assertions**: Paired late-entry live scenarios (`1-late-entry-a/b`) examine live reconnection and same-life HP continuity. Dedicated unit and projection automation tests validate the control of stale facts and the adversarial lower-incarnation rejections (`IsStaleSpawn`). The live server does not inject these cases.
- **Execution scope**: The measured runtimes come authoritatively from `/tmp/nightfall-phase1a/final-ci-suite/summary.json`. They show local coordinator runs, not CI runner runs. Each measured duration includes the volume reset of the previous unit, the new Compose startup and schema migration, and the headless client run. It also includes the API shutdown/verification, replay, HTML trace, contract/transition coverage, verdict and log collection. The final Compose teardown occurred separately after the audit of the last unit, and it exited 0. It is outside the 1180.417s coordinator total.

| Logical Unit | Scenario Files (`apps/client-unreal/Scenarios/`) | Topology | Measured Seconds | Status |
|---|---|---|---:|---|
| `0b-click-move` | `0b-click-move.nfs` | Single (1 bot) | 80.841 s | PASS |
| `0b-login-enter-world` | `0b-login-enter-world.nfs` | Single (1 bot) | 45.875 s | PASS |
| `0b-login-replaces` | `0b-login-replaces-a.nfs`, `0b-login-replaces-b.nfs` | Paired (2 bots) | 68.512 s | PASS |
| `0b-move-rejected` | `0b-move-rejected.nfs` | Single (1 bot) | 48.776 s | PASS |
| `0b-reconnect` | `0b-reconnect.nfs` | Single (1 bot) | 46.315 s | PASS |
| `0b-two-clients` | `0b-two-clients-a.nfs`, `0b-two-clients-b.nfs` | Paired (2 bots) | 56.387 s | PASS |
| `1-chase-leash` | `1-chase-leash.nfs` | Single (1 bot) | 92.580 s | PASS |
| `1-delevel` | `1-delevel.nfs` | Single (1 bot) | 145.475 s | PASS |
| `1-die-respawn` | `1-die-respawn.nfs` | Single (1 bot) | 112.578 s | PASS |
| `1-kill-one-monster` | `1-kill-one-monster.nfs` | Single (1 bot) | 76.917 s | PASS |
| `1-late-entry` | `1-late-entry-a.nfs`, `1-late-entry-b.nfs` | Paired (2 bots) | 91.141 s | PASS |
| `1-npc-respawn` | `1-npc-respawn.nfs` | Single (1 bot) | 114.064 s | PASS |
| `1-social-aggro` | `1-social-aggro-a.nfs`, `1-social-aggro-b.nfs` | Paired (2 bots) | 80.426 s | PASS |
| `1-target-attack` | `1-target-attack.nfs` | Single (1 bot) | 74.763 s | PASS |
| `1-target-unknown` | `1-target-unknown.nfs` | Single (1 bot) | 45.719 s | PASS |
| **Total (wall clock)** | **19 scenario files** | **15 logical units** | **1180.417 s** | **15/15 PASS** |

### 8.4 Gauntlet soak testing results

The Nightfall Gauntlet soak tests were validated locally with staged Linux builds against the full Compose stack. The performance validation strictly applies the Phase 1 E6.2 acceptance budget of **combat tick p99 < 20 ms** over all ticks in the isolated stack (`histogram_quantile(0.99, sum by (le) (nightfall_combat_tick_duration_seconds_bucket))`). The separate **50 ms** threshold is an operational alert threshold. It is not the acceptance budget.

The first debug runs examined the client assertions and the byte-identical replay. But the old reporter incorrectly used the 50ms operational alert threshold and `increase()`. Their performance verdicts are not valid now: debug 8×120 had a complete cumulative p99 of approximately 35.136ms, and it fails the actual <20ms budget. Then an abbreviated release run showed UUIDv7 character-name collisions in the same millisecond. The fix makes the names from random suffix bits. These failed/superseded runs stay as historical evidence, not acceptance.

The corrected reporter makes sure that every cumulative tick observation agrees with the completed recording. It also sends API or Compose teardown failures to its result. The intermediate release runs before the final audit/diagnostics integration passed. After them, the corrected post-fix smoke run passed:

| Run / artifact directory under `Saved/Soak` | Iterations | Client elapsed time | Gauntlet wall time | Combat p99 | Observed = replayed ticks | Teardown | Run type |
|---|---:|---|---:|---:|---:|---|---|
| `release-2x120` | 7 | client01:4 iterations/156.072s; client02:3/120.562s | 166s | 4.983ms | 1,668 | API 0, Compose 0 | Prior intermediate smoke |
| `release-8x120-fixed` | 30 | 120.926–158.977s across eight clients | 169s | 19.343ms | 1,697 | API 0, Compose 0 | Prior intermediate smoke |
| `release-8x120-damage-fixed` | 30 | 121.145–158.163s across eight clients | 169s | 11.28125ms | 1,695 | API 0, Compose 0 | Corrected damage-fixed smoke |
| `release-8x1200-damage-fixed` | 246 | 1201.004–1237.464s across eight clients | 1248s | 9.909452ms | 12,486 | API 0, Compose 0 | Locally verified full 20m soak pass |

All smoke recordings agreed byte-identically, with no digest-only records. The prior intermediate two-client recording had 7 players, 2,574 outputs and 161,652 bytes, and it replayed in 5.9 ms. The prior intermediate eight-client recording had 30 players, 11,456 outputs and 2,797,183 bytes, and it replayed in 31.5 ms. The corrected 8×120 damage-fixed smoke run (`release-8x120-damage-fixed`) passed all 30 iterations on eight clients (elapsed 121.145–158.163 s, Gauntlet wall time 169 s). Its combat tick p99 was 11.28125 ms (strict < 20 ms budget). Its 1,695 histogram ticks agreed with 1,695 recorded NFR ticks, and the teardown was clean (API 0 / Compose 0). Its session recording replayed byte-identically across 30 players, 11,364 outputs, and 2,754,186 bytes, with 0 digest-only records, in 31.4 ms. These are measured smoke results. They are not final integrated-source acceptance.

The final audited and diagnostic API and the staged client were first built at `2e85d3f` (API release in 52.31s, staged client BuildCookRun in 85.43s). The requested 8-client × 1200-second audited and diagnostic soak against the full Docker Compose stack (`Saved/Soak/release-8x1200-final`) failed functional acceptance (overall exit 1). Gauntlet stopped after 574 seconds (9m34s). At that time, iteration 14 of `client02` failed `nf.Expect damage_numbers_match_results` at line 28 (got 7 numbers / 8 distinct AttackResults; 13 completed iterations for `client02`). An exact replay proves this sequence at tick 5538: a new AOI spawn arrived with an NPC that was already dead (HP 0, incarnation 14). Immediately after the spawn came a valid, lethal, adjacent `AttackResult` in the same tick (damage 17, HP 0, incarnation 14). The dead guard of the client `CombatState` stopped the cue. Separately, the performance and replay pipeline evidence passed. The complete cumulative combat tick p99 was 10.562295 ms (strictly less than the 20 ms budget). 5,757 histogram ticks agreed with 5,757 recorded ticks. The session recording replayed byte-identically across 118 players, 43,638 outputs, and 9,734,800 bytes, with 0 digest-only records, in 120.1 ms. API exit and Compose cleanup both returned 0. The failed try does not prove 20-minute acceptance. The live combat duration includes durability and checkpoint work. The older OpenGate release microbenchmark measures a different workload, and it cannot replace this gate.

**Projection fix and verification on main (commit 8147841):**

Fix commit `612acad` corrected the bug in which the client projection stopped the cue. The fix was examined independently and merged to `main` at `8147841`. The change lets terminal lethal damage cues occur on snapshots that are already dead, and it does not make dead actors alive again. The verification was as follows:

- **Raw decoder regression test**: `Nightfall.Combat.State.DeadAoiSpawnLethalCue` was first RED on the original code and then GREEN on the fix. All 8 `Combat.State` automation tests passed.
- **Compilation times**: The incremental editor target compiled in 6.49 s. The game target compiled in 27.59 s.
- **Focused fresh-stack live scenarios**: Single-client `1-kill-one-monster` (78.079 s) and paired `1-late-entry` (93.668 s) passed against new Compose stacks on all 3 client instances. The assertions did not change, and the replay checks, the contract/transition coverage and the exit 0 cleanup passed.
- **Packaging and binary provenance**: The staged client was built again from source `8147841` through RunUAT BuildCookRun (the full command completed in 80.68 s). The release API and replay binaries from source `2e85d3f` had unchanged source files. They were used again, with verified SHA256 checksums.

**Locally verified replacement 20-minute soak execution (release-8x1200-damage-fixed):**

The replacement eight-client × 1200-second run against the full Compose stack (`Saved/Soak/release-8x1200-damage-fixed`) **passed with exit 0**. The native client was built from source `8147841`. The release API and replay binaries from `2e85d3f` were used again, after an examination of the unchanged source files and SHA-256 hashes. All eight clients exited with success and completed **246 iterations** (30–31 each) in 1201.004–1237.464 seconds per client. The Gauntlet wall time was 1248 seconds (20m48s). No assertions, ensures or crashes caused the run to fail.

The complete cumulative combat tick p99 was **9.909452 ms**, strictly less than the 20 ms budget. All **12,486 histogram observations agreed with the 12,486 ticks of the complete recording** for zone 1, epoch 1. The replay agreed byte-identically across 246 players, 94,729 outputs and 17,160,485 bytes, with zero digest-only records, in 249.5 ms. API exit and Compose cleanup both returned 0. The report has no errors. The per-client JUnit has 4,682 test cases with zero failures, errors or skips. The aggregate `soak.xml` has 4,683 cases, including the performance gate.

The report stores the [Grafana dashboard time range](http://localhost:13300/d/nightfall-api?from=1791591575000&to=1791592850000). The temporary local stack was removed. The saved telemetry and recording are the durable evidence. The local 20-minute acceptance passed. CI runner registration (E4.1), two-week reliability and PR gating (E4.5), and CI-derived results (E6.3) stay pending.

### 8.5 Diagnostics and coverage tracking status

This is the evidence from the local validation that `docs/engineering/sim-diagnostics.md` records, and from the authoritative final suite outputs:

- **E1.6 Failure bundle validation (real seeded failure actual values)**:
  - Seeded kill failure in `1-kill-one-monster`: the bot did the full walk, the target selection and the kill. Then it waited on purpose for `own_hp == 0` for 0.5 s.
  - The first failure kept line 33, observed HP 109, wait 0.50026 s, and elapsed scenario time 31.558 s.
  - The bundle had 50 typed server frames and 56 positions since the last click move. It also had the complete own projection (HP/MP/XP/level/target/attack state), and 4 proxy entries with position, HP, and life incarnation.
  - The observed ticks were 405 through 714. JUnit linked the bundle, and the trace was centered on tick 714. The replay agreed on 1,245 recorded ticks, 288 outputs, and 15,842 bytes. The process exited 1, as intended.
- **E1.7 Measured final-suite contract coverage**: The 19 clients sent **75 frames** and received **5,547 frames**. The compiled descriptor coverage is:

  | Category | Covered | Missing |
  |---|---:|---|
  | Intents | 5/6 | `stop_move` |
  | Payloads | 3/3 | None |
  | World events | 12/12 | None |
  | Reject reasons | 3/13 | Observed `INVALID`, `TOO_FAR`, `UNKNOWN_ENTITY`; the other 10 were not seen (full names in `coverage.contract.json`) |
  | Documented close codes | 1/4 | `4400`, `4408`, `4429`; the replacement close `4409` was observed |

- **Baseline floor and regression gate**: The complete suite passed the historical five-scenario floor. After an independent review, the measured full-suite floor was promoted on `main` at `534e0da`. This added `level_up`, `INVALID`, `UNKNOWN_ENTITY` and `4409`, and it kept the earlier positive observations. All 19 source reports pass the promoted floor. If you remove the actual death/respawn report, `intents.respawn` fails, as necessary.
- **Controlled raw server audit comparison**:
  - Initial audited run of `0b-move-rejected`: the runtime descriptor-backed counts and the `NF_SESSIONS` wire audit agreed exactly, with zero differences. The comparison used 1 socket session and 5 audited wire frames (1 inbound MoveTo, 4 outbound frames; 1 checkpoint JSON record not included).
  - Final audited run of `1-target-unknown`: this run was also examined against the `NF_SESSIONS` wire audit, with **0 diff** (`differences: []`). The comparison used 1 socket session and 5 audited wire frames. These were 1 inbound SetTarget and 4 outbound frames (2 spawn, 1 stats_changed, 1 rejected with `REJECT_REASON_UNKNOWN_ENTITY`). 1 checkpoint JSON record was not included.
- **State transition coverage (E3.9)**:
  - Transition coverage combines the recordings of all units. It removes duplicate multi-client recordings for paired groups. `group.nfr` is captured one time for each paired group, so that server transitions are not counted two times across client roles.
  - **NPC intentions**: **7/10** reachable pairs covered.
    - Never seen (3): `Idle -> Attack`, `Idle -> Dead`, `Active -> Dead`.
    - Unreachable (15): `Idle -> Idle`, `Idle -> ReturnHome`, `Active -> Active`, `Active -> ReturnHome`, `Attack -> Idle`, `Attack -> Active`, `Attack -> Attack`, `ReturnHome -> Idle`, `ReturnHome -> Attack`, `ReturnHome -> ReturnHome`, `ReturnHome -> Dead`, `Dead -> Active`, `Dead -> Attack`, `Dead -> ReturnHome`, `Dead -> Dead`.
  - **Player attack states**: **3/4** reachable pairs covered.
    - Never seen (1): `pending -> idle`.
    - Unreachable (5): `idle -> idle`, `idle -> active`, `pending -> pending`, `active -> pending`, `active -> active`.

### 8.6 Three lessons learned

1. **Wait for authoritative projections and examine the actual fixture mechanics.** A transport connection that succeeds does not prove that the actor is ready. Combat waits must let misses and tick phasing occur. The social fixture must make a distinction between clan help and ordinary proximity aggro.

2. **Logout does not unload the Unreal world.** Repeated scenarios load `L_Login` again and wait until the map is ready, to remove old proxies. The soak completes each final iteration, so that every assertion that started has a verdict. As a result, a nominal 120-second client ran for as long as 159.0 seconds. Also, concurrent bot account rotation showed that UUIDv7 names with a timestamp prefix caused collisions when clients registered in the same millisecond. The fix makes the character names from the last 52 random suffix bits (`Hex.Len() - 13`, `BotCharacterName::FromGuid`). This prevents deterministic timestamp collisions across parallel bots.

3. **A fixed seed and a faithful replay are different checks.** New Postgres/NATS storage pins the zone seed to `(1, 1)`. But client identities and arrival order can change between runs. A replay of the recorded commands makes sure that the run is byte-identical. A metrics flush before teardown separately examines its performance gate. Also, the original full soak showed a subtle arrival race. A new AOI spawn arrived with a monster that was already dead, immediately before the lethal hit of that tick. The regression test that came from this protects the client damage cue, and it does not make dead actors alive again.
