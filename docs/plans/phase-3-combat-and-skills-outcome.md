# Phase 3 combat and skills: verification outcome

**Status:** final acceptance documentation (closed root receipts). Local Phase 3 implementation — server-authoritative skill combat over the 39 populated class trees, SP learning at the Class Master, casting lifecycle and reuse timers, effect scheduling and abnormal stacking, HP/MP/CP regeneration, PvP flagging, karma gain and decay, CP damage absorption, Death Penalty, skill resurrection with offer expiry and the peace square, on Snapshot 9 / Record 7 / Binary V5 with legacy formats replaying byte-exact — is VERIFIED by offline UE automation (103/0), required-live UE automation (103/0/0), the full ordinary scenario suite (59 units / 59 PASS), Game package staging (84.99 s), the packaged casting soak (attempt 12: 8 clients × 17 iterations, 1,200 s requested, 12,246 ticks observed, combat tick p99 10.776 ms < 20 ms budget, no client errors, API and cleanup exit 0, on tmpfs-backed Postgres/NATS volumes — see §2), and the final Rust gate on the merged tree (fmt/clippy PASS; 1018 passed / 0 failed / 7 ignored; release zone_step: 1000 moving NPCs mean 146.7 µs p99 173.3 µs vs 2 ms budget, 1000 moving players with all observers mean 9.54 ms p99 19.42 ms vs 10 ms mean budget). Main and origin/main merged and pushed `995ffa9e37b065f540c12e6012a94509658f8035` on 2026-10-13. Post-merge root Editor and Game builds exit 0 (04:35:42 / 04:37:06 UTC); root offline automation without an API ran 102 passed / 1 failed (`CatalogueRpcFailures` needs a reachable API), and the rerun with the API up passed 103/103, exit 0 (05:58:28 UTC). Local Phase 3 is proven complete once this final reviewed docs commit lands. External self-hosted Linux runner remains unregistered, and the two-week reliability programme remains a separate operational follow-up (not a local acceptance gate or fresh 14-day wait).

See the [executable plan](phase-3-combat-and-skills.md), [reference design](../planning/03-combat-and-skills.md), [architecture decisions](phase-3-architecture-decisions.md), and [zone design](phase-3-zone-design.md).

The preserved public evidence root is `/home/matt-woodruff/.config/Claude/side-session-notes/phase3-codex-2026-10-10/integration-evidence/`. Preserved evidence receipts cite files within that root: `r11-00-rust.log`, `r12-00-rust.log`, `r13-01-ue-builds.log`, `r13-02-ue-offline.log`, `r13-03-ue-required-live.log`, `r13-04-sim-ci.log`, `r13-soak-attempt1/stage.log`, `r21-03-replay-check.log`, `r22-00-summary.log`, `r22-05b-checkpoint-audit-rerun.log`, `r22-05c-tests.log`, `r22-05d-benches-testmode.log`, `r22-05e-zone-step-release.log`, `r22-soak/report.json`, `r22-soak/summary.txt`, `r22-soak/storage.txt`, `r22-soak/replay-check.log`, `r22-soak/probe-failed/fsync-root.log`, `root-build-995ffa9.log`, `root-smoke-995ffa9.log`, and `root-smoke2-995ffa9.log` / `root-smoke2-995ffa9.summary`. Review receipts and progress logs sit one level up, in `/home/matt-woodruff/.config/Claude/side-session-notes/phase3-codex-2026-10-10/`.

---

## 1. Implemented contracts

### 1.1 Combat, skills, and lifecycle contracts
- **Casting lifecycle:** Implemented according to [phase-3-architecture-decisions.md](phase-3-architecture-decisions.md) §1 A11, §6 R2, and §8 F3/F4: `mpConsume1` deducted at cast start; `mpConsume2` deducted at cast impact/completion (if MP is insufficient the completion is cancelled with `SkillCastCancelled` reason `CAST_CANCEL_REASON_RESOURCES`); cast animation time computed from pinned `Formulas.calcCastTime` with 500 ms floor under source conditions; interrupt cutoff window set to `floor(trunc(animation_ms)/100) - 2` ticks. Movement during casting triggers `SkillCastCancelled` with reason `CAST_CANCEL_REASON_MOVED`. Accepted casts are acknowledged; completion failure emits `SkillCastCancelled` rather than late intent rejection.
- **Effects and abnormal stacking:** CharEffectList stacking rules enforce slots (20 normal buff slots, +4 via `ENLARGE_ABNORMAL_SLOT`, 12 song/dance slots, 8 debuff slots; oldest evicted first). An equal or higher abnormal level replaces the existing stack and restarts its timers, a lower level is ignored; `NONE` stacks independently but replaces the same skill ID. Death removes all effects except those flagged `isStayAfterDeath`. Class transfers remove effects except those flagged `isStayOnSubclassChange`.
- **Regeneration:** HP/MP/CP regeneration adheres to pinned `Formulas.calcHpRegen`, `calcMpRegen`, and `calcCpRegen` applied every 30 ticks (3.0 s); players use the standing (11/10) and moving (7/10) posture multipliers, NPCs use base NPC regen without posture multipliers ([zone design](phase-3-zone-design.md) `regen_amounts`). CP regeneration is player-only. Fractional regen is tracked in high-precision Q remainder accumulators and emitted as whole values. Dead actors do not regenerate.
- **PvP flagging, karma, and CP absorption:**
  - Player targeting is enabled (`SetTarget` accepts live players, excluding self).
  - A hostile act against a non-flagged player refreshes the attacker's flag to 1,200 ticks (120 s); a hostile act against a flagged player refreshes it to 600 ticks (60 s); the deadline is `max(old, new)` and the target never flags from being hit (decisions §7 D-R4; zone design PvP truth table).
  - Killing a non-flagged, non-chaotic player increments `pk_count` and increases karma via pinned `calculateKarmaGain(pkCount)`. Killing a flagged player increments `pvp_count`.
  - Monster XP awards decay karma according to pinned `calculateKarmaLost` using `pcKarmaIncrease.xml` multipliers.
  - CP absorbs player-sourced damage (normal attacks and skills) before HP; NPC-sourced damage bypasses CP.
- **Death and resurrection:**
  - Standard death XP loss table is preserved.
  - Death Penalty (skill 5076) rolls a 20% chance on death to NPCs or to players while chaotic, up to level 5 (not applied in peace zones).
  - Resurrection (skill 1016) casts on dead player corpses (`PC_BODY`), emitting a `ResurrectOffer` with offer ID, death incarnation, and a 600-tick (60 s) expiration window.
  - Target accepts via `AcceptResurrect`, reviving at the corpse with HP restored to `max(current, floor(maxHp * 65%))` per pinned `L2Character.doRevive()`, MP/CP preserved from death values, and one-time recoverable XP restored. Declines, timeouts, town respawns, or disconnects invalidate the offer.
- **SP and skill learning:**
  - Monster templates define `sp_reward` (Young Keltir NPC 20545 awards SP 2, matching its pinned datapack row). SP is credited on kill blow.
  - Learning occurs exclusively at the Class Master NPC (126,128) within radius 3 tiles.
  - `ListLearnableSkills` lists next-rank-only skills with SP costs and prerequisites. `LearnSkill` executes as a revision-fenced critical actor mutation with idempotency key and frozen response caching.

### 1.2 Data provenance and catalogue bounds
- **Pinned upstream source:**
  - Game formulas and mechanics: L2J Game server revision `abfde0490ac52a2106e884b7242c0276c44cdf76`.
  - Skills and templates: L2J Datapack revision `3ca488dd2bd0bfaca43e378886a3c2e37968153a`.
- **Catalogue classification:** Generated from the 39 populated class trees (378 skill references) plus Death Penalty 5076 and triggered references (389 parsed definitions total; sample-only 286 excluded; 388 definitions retained).
  - **180 executable / 198 deferred** within the 378 core catalogue.
  - **188 executable / 200 deferred** across all 388 retained skill definitions.
  - Executability is computed by the data generator's capability checks (`packages/data/scripts/gen_skills.py`): any skill needing the toggle operate type (`T`), an unsupported affect scope (`PARTY`, `PARTY_PLEDGE`, `DEAD_PLEDGE`, `FAN`) or target type, item consumption, charges, summons, an unsupported condition or stat, or a missing effect handler is marked deferred with its named blocker (`blockers = [...]` in `packages/data/skills/<id>.toml`).
- **Oracle fidelity:** Pinned H5 abnormal land-rate defaults are 10/90 (clamped per-skill by `minChance`/`maxChance`). Engine caps: `MaxMAtkSpeed = 1999`, `MaxPAtkSpeed = 1500`, `MaxEvasion = 250`. Missing attributes default to `magicLvl = 0`, `castRange = -1`, `effectRange = -1`, and `abnormalLvl = 0`. Clamps are loaded from `tables/formulas.toml`.

### 1.3 Wire, versions, and deviations
- **Version boundary:** Replay codec formats are **Snapshot 9 / Record 7 / Binary V5**. Legacy Snapshot 8 / Record 6 / Binary V4 and older formats replay byte-exact under legacy rules (no regen, no skills, no PvP). Mixed 9/V4 or 8/V5 pairings are rejected. Outer NFR recording format 1 is preserved.
- **Wire additions:** Additive fields on existing envelopes: `ClientMessage` tags 16 (`CastSkill`), 17 (`CancelCast`), 18 (`AcceptResurrect`); `WorldEvent` cases 14–25; `StatsChanged` fields 15–23; `EntitySpawn` fields 22–28; `AttackResult.cp_damage = 8` (`packages/proto/nightfall/v1/world.proto`).
- **Privacy fences:** `StatsChanged` remains **owner-only**; public PvP visibility is projected solely via `PvpStateChanged` and `EntitySpawn`. Public HP adjustments without attack damage project via `VitalsChanged = 25`.
- **Architect deviations:**
  - **C17 retained-selection deviation:** Nightfall retains a player's selected target when the target leaves the AOI and re-selects it upon re-entry (in retail L2J, `CharKnownList.removeKnownObject` clears the target).
  - **C18 predicate semantics:** The scenario predicate `players_attacking_me` is defined strictly as the number of players with a currently received, unexpired swing or recovery against the actor; server pursuit without a new swing is unobservable on the wire.
  - **Offline timer pause deviation (R4):** Skill reuse remainders (> 5 s / > 50 ticks) and effect remainders (> 10 s / > 100 ticks) pause while characters are offline and are restored verbatim on admission, rather than advancing against wall-clock time.
  - **Resurrection offers:** Valid for 600 ticks (60 s), distinct from older 300-tick drafts.
  - **Peace zone geometry (R7):** Centered at town safe point (126,126) as an inclusive square of half-width 10 tiles (10,000 milli-tiles), enclosing the Class Master at (126,128). Hostile intents are rejected with `PEACE_ZONE`.

---

## 2. Verified evidence table

Receipts below are located in the preserved evidence root `/home/matt-woodruff/.config/Claude/side-session-notes/phase3-codex-2026-10-10/integration-evidence/`:

| Verification area | Status / evidence | Scope and source reference |
|---|---|---|
| Rust compilation & gates (r11) | **PASS (1000 passed, 0 failed, 3 ignored)** | Preserved log `r11-00-rust.log`. Tree `869eec7`. 1,000 tests passed, 0 failed, 3 ignored. `FMT_PASS`, `CLIPPY_PASS`, `ENTITIES_PASS`, `REPLAY_PASS`, `DATA_PASS`, `BUF_PASS`. Release binaries `nightfall-api` and `nightfall-replay` built in 59.96 s. |
| Rust compilation & gates (r12) | **PASS (1001 passed, 0 failed, 3 ignored)** | Preserved log `r12-00-rust.log`. Tree `fd7e734`. 1,001 tests passed, 0 failed, 3 ignored. `FMT_PASS`, `CLIPPY_PASS`, `REPLAY_PASS`. Release binaries built in 52.62 s. |
| Static analysis & data checks | **PASS (fmt/clippy/entities/replay/data/buf)** | `r11-00-rust.log`: `FMT_PASS`, `CLIPPY_PASS`, `ENTITIES_PASS`, `REPLAY_PASS`, `DATA_PASS`, `BUF_PASS`. Per-fixture replay detail in `r21-03-replay-check.log`: 4 fixtures byte-exact — legacy `two-players-v4.nfr` (467 ticks) and `two-players-fight-v2.nfr` (908 ticks), V5 `phase3-casts-v5.nfr` (131 ticks) and `phase3-gameplay-v5.nfr` (769 ticks). |
| Unreal client builds | **PASS (both builds exit 0)** | Preserved log `r13-01-ue-builds.log`. Tree `dfd16cf`. Editor build exit 0 in 23s 554ms; Game build exit 0 in 32s 271ms. |
| Offline UE automation | **PASS (103 passed, 0 failed, exit 0)** | Preserved log `r13-02-ue-offline.log`. 103 automation tests executed, 103 `Result={Success}`, 0 failures, exit code 0. |
| Required-live UE automation | **PASS (103 passed, 0 failed, 0 skipped, exit 0)** | Preserved log `r13-03-ue-required-live.log`. Tested against live API on owned port. 103 automation tests executed, 103 `Result={Success}`, 0 failures, 0 skips, exit code 0. |
| Full ordinary scenario suite | **PASS (59 units / 59 PASS, exit 0)** | Preserved log `r13-04-sim-ci.log` and directory `r13-sim-ci-final/`. 59 logical units (`r13-sim-ci-final/summary.json` `units: 59`; 6 Phase 0b + 9 Phase 1 + 14 Phase 2 + 30 Phase 3; several units run multi-role variant files). All 59 PASS, 0 FAIL. Contract coverage: intents 8/9 (missing `stop_move`), payloads 3/3, events 25/25, reasons 10/25, close codes 1/4. Reachable state transitions: NPC intentions 8/10, player attack 4/4, cast states 2/2, effect states 2/2, resurrection offers 4/4, PvP transitions 3/12. |
| Game package staging | **PASS (84.99 s, exit 0, Development Linux)** | Preserved log `r13-soak-attempt1/stage.log`. BuildCookRun stage completed in 84.99 s, 2,890 UFS items, exit code 0. Staged to `Saved/StagedBuilds/Linux`. |
| Final Rust gate on merged main | **PASS (1018 passed, 0 failed, 7 ignored)** | `r22-00-summary.log`: fmt exit 0 and clippy exit 0 on `7e1740a` (code-identical to `995ffa9`, which changes only `docs/plans/phase-3-architecture-decisions.md`); that first all-targets run aborted at `checkpoint_audit` (JetStream request timeout at host load 58–66) and the test passed alone 1/1 (`r22-05b-checkpoint-audit-rerun.log`). `r22-05c-tests.log` (HEAD `995ffa9`, `--no-fail-fast --test-threads=2`): 64 targets run; the 62 test-harness targets report 1,018 passed, 0 failed, 7 ignored; the 2 criterion benches (`phase3_effects`, `zone_step`) rejected the `--test-threads` argument (invocation artifact, cargo exit 101). `r22-05d-benches-testmode.log`: the debug-profile `zone_step` run panics on its budget (1000 moving NPCs mean 3.26 ms > 2 ms; the known unoptimized-profile artifact); the release run below is the budget gate. |
| Final release performance gate | **PASS (both budgets met)** | Preserved log `r22-05e-zone-step-release.log`. Built in release profile in 4m 04s:<br>• `1000 moving npcs`: mean **146.7 µs** (146.661 µs), p50 142.552 µs, p99 **173.3 µs** (173.313 µs) vs 2 ms budget.<br>• `1000 moving players all observers`: mean **9.54 ms** (9.539867 ms), p50 9.997 ms, p99 **19.42 ms** (19.417171 ms) vs 10 ms mean budget. |
| Packaged combat soak (attempt 12) | **PASS (8 clients × 17 iterations, p99 10.776 ms < 20 ms, exact replay)** | Preserved report `r22-soak/report.json` and `r22-soak/summary.txt`. 8 clients × 17 iterations, 1,200 s requested, 12,246 ticks observed and recorded. Combat tick p99 was **10.776 ms** (< 20 ms budget). Phase 3 acceptance evidence passed (`phase3_acceptance: true`, `phase3_evidence_passed: true`, `errors: []`). API exit 0, cleanup exit 0 (`r22-soak/api-exit-code.txt`, `r22-soak/cleanup-exit-code.txt`). Exact byte-exact replay verified in `r22-soak/replay-check.log`: zone 1 epoch 1 match across 12,246 replayed ticks, 8 players, 87,248 outputs, 11,802,765 bytes compared, 0 digest-only in 796.9 ms. |
| Soak storage isolation | **tmpfs-backed (SOAK_TMPFS_VOLUMES=8g)** | Preserved record `r22-soak/storage.txt`: `volumes=tmpfs:8g8g`. The soak stack's PostgreSQL and NATS JetStream volumes were mounted on tmpfs to isolate execution from host fsync stalls observed during attempts 9–11 on the shared encrypted NVMe root filesystem (perf round 4, architecture decisions §10 P6). The independent host fsync probe failed to start due to permission configuration (`r22-soak/probe-failed/fsync-root.log`: `PermissionError: [Errno 13]`), resulting in no concurrent host fsync sample for attempt 12. |
| Post-merge root builds | **PASS (Editor and Game exit 0)** | `root-build-995ffa9.log` (root checkout on `995ffa9`, start 04:31:26 UTC): Editor build exit 0 at 04:35:42 (3m 40s 511ms); Game build exit 0 at 04:37:06 (1m 23s 845ms). |
| Post-merge root offline automation | **PASS on rerun (103/103, exit 0)** | `root-smoke-995ffa9.log` (no API running): 102 `Result={Success}`, 1 `Result={Fail}` — `Nightfall.Skill.CatalogueRpcFailures` (`CallRpcError: FailedPrecondition: catalogue fixture precondition`; the test needs a reachable API), run-tests exit 255. Rerun with the API up, `root-smoke2-995ffa9.log` / `root-smoke2-995ffa9.summary`: 103 `Result={Success}`, 0 failures, `CatalogueRpcFailures` Success, run-tests exit 0 at 05:58:28 UTC. The summary's 60 s healthz probe line recorded no output. |

### 2.1 Preserved review receipts
Review receipts (`<name>.md`) are preserved in `/home/matt-woodruff/.config/Claude/side-session-notes/phase3-codex-2026-10-10/`. Several record REJECT or REQUEST CHANGES verdicts that were closed by later fix rounds; the architect's closure of all code reviews at integration r13 (`dfd16cf`) is logged in `architecture-notes.md` (2026-10-11T23:22:39Z).
- **Plan reviews:** `plan-review-1`, `plan-review-2`, `plan-review-3`
- **Design reviews:** `design-review-1`, `design-review-2`
- **Data reviews:** `data-review-1`, `data-review-2`, `data-review-3`, `data-review-4`
- **Engine reviews:** `engine-review-1`, `engine-review-2`
- **API review:** `api-review-1`
- **Client reviews:** `client-review-1`, `client-review-2`
- **Harness review:** `harness-review-1`
- **Integration reviews:** `integration-review-1`, `integration-review-2`, `integration-review-3`
- **Triage reviews:** `triage-review-1`, `triage-review-2`, `triage-review-3`, `triage-review-4`, `triage-review-5`, `triage-review-6`, `triage-review-7`

---

## 3. Known limitations and deviations

- **Presentation and art assets:** Combat animations, spell-casting particle effects, sound effects, and character visual cues remain thin-client prototype placeholders. No custom 3D combat animations, projectile meshes, or specialized VFX are imported.
- **Deferred skills:** 198 of the 378 catalogue skills (and 200 of all 388 retained skills) are deferred with explicit named blockers. Blocker counts across the generated definitions (`packages/data/skills/*.toml`; one skill can carry several) include:
  - `scope:PARTY` (21 skills), `scope:PARTY_PLEDGE` (2), `scope:DEAD_PLEDGE` (1) — party/clan systems (Phase 7)
  - `item_consume` (28 skills) — Phase 4 items
  - `operate:T` toggles (13 skills)
  - `cond:player:Charges` (9 skills)
  - `effect:Summon` and `cond:player:canSummon` (7 skills each) — pets/servitors excluded (decisions §0.1)
  - `cond:target:race` (7 skills)
  - Unimplemented effect handlers such as Lethal, FatalBlow, Spoil, Sweeper, Escape, TeleportToTarget, DeleteHate, RandomizeHate, TargetCancel, HpDrain, EnergyAttack, Transformation and Fear.
- **Fear effect exclusion:** Fear is explicitly excluded from Phase 3 because it requires flee pathing (decisions §1 A8); skills referencing Fear are deferred.
- **Equipment and systems excluded:** Shields and blocking, weapon types/masteries, enchantments, and augments remain Phase 4 systems. Unsupported condition predicates defer skills requiring them. Real Squire's Sword weapon conditions are enforced.
- **Deviations from reference:**
  - **C17 retained-selection:** Target selection is preserved across AOI departure and re-entry.
  - **R4 offline-timer pause:** Buff and reuse timers pause while offline (persisted when > 100 ticks and > 50 ticks respectively).
  - **Offer duration (R7):** Resurrect offers expire after 600 ticks (60 s).
  - **Peace square (R7):** A 10-tile half-width square around (126,126) suppresses hostile action and auto-aggro.
  - **Death penalty cap:** Maximum level 5.
  - **Town respawn vitals:** Restores 65% HP and 0 MP (Phase 1 legacy contract preserved).
- **Synthetic stress above baseline ceilings:** As documented in the preserved `inbox/zone-progress.md` (zone engine worker report), synthetic stress testing under extreme periodic effect density (1,000 moving entities each carrying 8 or 44 periodic effects) exceeds the baseline ceilings (8 effects: 2.99 ms NPC / 16.04 ms player mean, player p99 96.41 ms; 44 effects: 14.08 ms NPC / 34.45 ms player mean, player p99 342.11 ms). This is a reported stress result, not a gate. Baseline quiescent limits (1,000 NPCs < 2 ms, 1,000 players < 10 ms) and realistic soak conditions are fully satisfied.
- **Soak storage backing:** The passing soak (attempt 12) ran with the soak stack's PostgreSQL and NATS volumes on tmpfs (`SOAK_TMPFS_VOLUMES=8g`). On the shared encrypted root filesystem (ext4 on LVM/dm-crypt, DRAM-less NVMe) attempts 9–11 failed: perf round 4 matched every fence stall ≥ 500 ms (59/59) to a host fsync stall, with zero `zone_epochs` lock waits. No compliant code change exists because A3 requires the per-tick durable fence commit; the owner accepted the tmpfs-backed soak as the gate with no second-drive provisioning (decisions §10 P6). The attempt-12 fsync probe failed to start, so that run has no concurrent host-fsync record.
- **Hosted CI telemetry probe:** On the hosted runner the `ci.yml` step "Verify telemetry in LGTM" (`infra/scripts/verify-observability.sh`) fails at its Tempo trace probe while `moon ci` passes; the failure predates Phase 3 (failing since at least `8fb1a57`, Oct 9) and is out of Phase 3 scope (`architecture-notes.md`).
- **Operational follow-ups:**
  - External self-hosted Linux CI runner remains unregistered.
  - Two-week reliability programme remains an independent external operational follow-up. Neither is an acceptance blocker for Phase 3.

---

## 4. Historical diagnostics and triage

### 4.1 Triage rounds summary
Five integration triage rounds, each independently reviewed (triage reviews 1–7), closed the live-suite failures (`inbox/triage-progress.md`, `architecture-notes.md`):
- **Triage round 1 (r6 `033e717`):** All 31 failing Phase 2/3 units from r5 suite run 1 were diagnosed; the suspected engine defects were refuted (the debuff miss was a legitimate resisted roll, all regen amounts matched the independent oracle). Fixes: actual-input revive/karma oracle (`fa4f7d4`), replay-only corpse/revive and XP/karma credit facts (`a357087`), warning-level optional catalogue and client correlations (`a5727fb`), and C16 V5 fixture packs, complete NPC overlays, TCP database readiness and narrow `.allow` declarations for expected RPC rejections (`033e717`).
- **Triage round 2 (r8 `93e150b`):** Durable keltir templates lacked Phase 3 profiles in legacy fixture packs (`1-late-entry`, `1-social-aggro`); token-milestone second-death waits timed out under V5 (source-correct miss-driven acquisition delay). Fixes touched the harness and also Rust (`skill_data_profiles.rs`, `zone_runtime.rs`), so suite run 2 on the stale r7 binary was aborted and restarted.
- **Host reboot and suite run 2:** The owner rebooted the host at 15:30 UTC on 2026-10-11 during suite run 2, which was lost. On r9 `7e76292`, 5 NATS-dependent Rust tests failed in the post-reboot all-targets run and passed in isolation (JetStream recovery after unclean shutdown). Suite run 3 finished 57 PASS / 2 FAIL (`1-late-entry`, `3-pvp-flag`; `r9-sim-ci-run3`).
- **Triage round 3 (r10 `12a3f90`):** `1-late-entry` role B was diagnosed as scenario timing, not V5 hydration; received-state coordination plus a replay-verified HP continuity gate (`59d6a81`). The r10 final suite was still 57/59 (same two units).
- **Triage round 4 (r11 `869eec7`):** Fixed a real zone engine defect — restore the retained selection when the target re-enters the AOI (`5855886`, recorded as deviation C17) — made the late-entry hydration gate mandatory (`6bf0fa0`), and bounded the client attacker audit by received swing recovery (`869eec7`); review #5 recorded C17 and C18. The r11 suite was still 57/59.
- **Triage round 5 (r12 `fd7e734`):** Retained received regen during the peace-cancellation audit for `3-pvp-flag` (`1306bb8`) and coordinated late entry within a received NPC windup (`fd7e734`). r12 suite 59/59 PASS; automation 101/2 on the two test expectations raised by review #6.
- **Review #6 follow-up (r13 `dfd16cf`):** Client fixes for damage masked by regen and the two test expectations; triage review #7 APPROVE. r13 reached 103/0 offline, 103/0/0 required-live, 59/59 scenarios PASS, and Game package staging (84.99 s).

### 4.2 Soak attempts 1–11 diagnostics
The packaged combat soak required twelve total attempts to isolate harness issues and host hardware contention:
- **Attempt 1:** Harness failed prior to client launch due to missing host `psql` binary in `seed-phase3-soak.py`.
- **Attempt 2:** (deferred first while a foreign OCR workload kept host load ~10–12) All 8 clients failed at `EnterWorld`: the seeder's character names (`Skillrole01`, …) violated the letters-only name rule; fixed in `28a9301`.
- **Attempt 3:** Aborted because the orchestrator exported a wrong `UE_ROOT` (not a code defect).
- **Attempt 4:** Clients failed on BotLoop iteration 2 because the casting lane return leg (~118 tiles) exceeded `MAX_MOVE_DISTANCE_TILES` (64).
- **Attempt 5:** Client 05 encountered `PEACE_ZONE` rejection because radial casting lanes passed within proximity of the town peace square.
- **Attempt 6:** All 8 clients ran 17 clean iterations for 1,200 s; client-02 then failed at iteration 18 (`buff_active 1068 absent`): each Might recast was cancelled by the immediate move because `buff_active` passed on iteration 1's buff until it expired at 1,200 s (harness race).
- **Attempt 7:** Functional pass (8 clients × 17 iterations, 12,230 ticks, exact replay), but p99 reached 53.6 ms due to zone actor log/checkpoint fan-out overhead under heavy host contention. Prompted perf rounds 1–3.
- **Attempt 8:** Stack storage collapsed at tick 54 (COMMIT 1.3 s, `UPDATE zone_epochs` 8.5 s) under host load 19–21 from concurrent compilation and OCR tasks.
- **Attempts 9–11 (Host fsync stalls):**
  - In attempt 9 (owner's OCR workload stopped; three compiling workers demoted to `SCHED_IDLE`), the tick body improved but tail storage stalls persisted (66% of ticks 2–5 ms; 32 `UPDATE zone_epochs` statements > 1 s; p99 190 ms).
  - Attempt 10 failed during startup connection pooling during post-teardown load spikes.
  - Attempt 11 on a quiet host with frozen workers exhibited tick rate halving and 11 slow pre-log fence updates (2–21 s each).
  - Diagnostic perf round 4 proved that the stalls were caused by host fsync latency on the shared encrypted NVMe root filesystem (ext4 on LVM/dm-crypt on DRAM-less hardware; 59 of 59 stalls ≥ 500 ms coincided with host fsync probe stalls; zero `zone_epochs` lock waits); receipts in `perf-runtime-r4/`.
- **Attempt 12 (PASS):** In accordance with architecture decisions §10 P6, the PostgreSQL and NATS volumes were placed on tmpfs (`SOAK_TMPFS_VOLUMES=8g`, `r22-soak/storage.txt`); its fsync probe failed to start (`r22-soak/probe-failed/fsync-root.log`), so there is no concurrent host-fsync record for this attempt. Attempt 12 passed: 8 clients × 17 iterations, 1,200 s requested, 12,246 ticks observed, combat tick p99 of **10.776 ms**, and byte-exact replay verification.
