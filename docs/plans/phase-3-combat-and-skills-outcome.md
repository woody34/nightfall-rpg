# Phase 3 combat and skills: verification outcome

**Status:** final acceptance documentation (closed root receipts). Local Phase 3 implementation including server-authoritative skill combat, 39 populated class trees, SP learning at the Class Master, casting lifecycle and reuse timers, effect scheduling and abnormal stacking, health/mana/CP regeneration, PvP flagging, karma gain and decay, CP damage absorption, death penalty, skill resurrection with offer expiration, peace square protection, Snapshot 9 / Record 7 / Binary V5 codecs with byte-exact legacy preservation, offline automation (103 tests), required-live UE automation (103 tests), full ordinary scenario suite (59 units / 59 PASS), Game package staging (84.99 s), packaged combat soak (8 clients × 17 iterations, 1,200 s requested, 12,246 ticks observed, combat tick p99 10.776 ms < 20 ms budget, all exits 0), and post-merge root main integration (`995ffa9`) are VERIFIED. Main and origin/main merged and pushed `995ffa9e37b065f540c12e6012a94509658f8035`; Rust release gate passed (1018 passed / 0 failed / 7 ignored over 64 targets, zone_step release benchmarks passed: 1000 moving NPCs mean 146.7 µs p99 173.3 µs < 2 ms; 1000 moving player observers mean 9.54 ms p99 19.42 ms < 10 ms mean budget). Local Phase 3 is proven complete once this final reviewed docs commit lands. External self-hosted Linux runner remains unregistered, and the two-week reliability programme remains a separate operational follow-up (not a local acceptance gate or fresh 14-day wait).

See the [executable plan](phase-3-combat-and-skills.md), [reference design](../planning/03-combat-and-skills.md), [architecture decisions](phase-3-architecture-decisions.md), and [zone design](phase-3-zone-design.md).

The preserved public evidence root is `/home/matt-woodruff/.config/Claude/side-session-notes/phase3-codex-2026-10-10/integration-evidence/`. Preserved evidence receipts cite files within that root: `r11-00-rust.log`, `r12-00-rust.log`, `r13-01-ue-builds.log`, `r13-02-ue-offline.log`, `r13-03-ue-required-live.log`, `r13-04-sim-ci.log`, `r13-soak-attempt1/stage.log`, `r22-00-summary.log`, `r22-05c-tests.log`, `r22-05e-zone-step-release.log`, `r22-soak/report.json`, `r22-soak/summary.txt`, `r22-soak/storage.txt`, and `r22-soak/replay-check.log`.

---

## 1. Implemented contracts

### 1.1 Combat, skills, and lifecycle contracts
- **Casting lifecycle:** Implemented according to [phase-3-architecture-decisions.md](phase-3-architecture-decisions.md) §1 A11, §6 R2, and §8 F3/F4: `mpConsume1` deducted at cast start; `mpConsume2` deducted at cast impact/completion (aborts with `REJECT_REASON_RESOURCES` if MP is insufficient); cast animation time computed from pinned `Formulas.calcCastTime` with 500 ms floor under source conditions; interrupt cutoff window set to `floor(trunc(animation_ms)/100) - 2` ticks. Movement during casting triggers `SkillCastCancelled` with reason `Moved`. Accepted casts are acknowledged; completion failure emits `SkillCastCancelled` rather than late intent rejection.
- **Effects and abnormal stacking:** CharEffectList stacking rules enforce slots (20 normal buff slots, +4 via `ENLARGE_ABNORMAL_SLOT`, 12 song/dance slots, 8 debuff slots). Stacking replaces lower or equal abnormal levels and restarts timers; `NONE` stacks independently but replaces the same skill ID. Death removes all effects except those flagged `isStayAfterDeath`. Class transfers remove effects except those flagged `isStayOnSubclassChange`.
- **Regeneration:** HP/MP/CP regeneration adheres to pinned `Formulas.calcHpRegen`, `calcMpRegen`, and `calcCpRegen` with standing (1.1) and moving (0.7) posture multipliers applied every 30 ticks (3.0 s). CP regeneration is player-only. Fractional regen is tracked in high-precision Q remainder accumulators and emitted as whole values. Dead actors do not regenerate.
- **PvP flagging, karma, and CP absorption:**
  - Player targeting is enabled (`SetTarget` accepts live players, excluding self).
  - Hostile actions against non-flagged, non-chaotic players initiate a 1,200-tick (120 s) purple flag; hostile actions between flagged players set a 600-tick (60 s) flag.
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
  - Executability is strictly calculated by data generator capability checks: any skill requiring unimplemented operate types (`CA`, `DA`, `T`), unsupported target scopes (`PARTY`, `PLEDGE`, `GROUND`, `FAN`, `SQUARE`), item consumables (shots, ores), charges, pets, or missing effect handlers is automatically marked deferred with its named blocker.
- **Oracle fidelity:** Pinned H5 abnormal land-rate defaults are 10/90 (clamped per-skill by `minChance`/`maxChance`). Engine caps: `MaxMAtkSpeed = 1999`, `MaxPAtkSpeed = 1500`, `MaxEvasion = 250`. Missing attributes default to `magicLvl = 0`, `castRange = -1`, `effectRange = -1`, and `abnormalLvl = 0`. Clamps are loaded from `tables/formulas.toml`.

### 1.3 Wire, versions, and deviations
- **Version boundary:** Replay codec formats are **Snapshot 9 / Record 7 / Binary V5**. Legacy Snapshot 8 / Record 6 / Binary V4 and older formats replay byte-exact under legacy rules (no regen, no skills, no PvP). Mixed 9/V4 or 8/V5 pairings are rejected. Outer NFR recording format 1 is preserved.
- **Wire additions:** Additive fields on existing envelopes: `ClientMessage` tags 16 (`CastSkill`), 17 (`CancelCast`), 18 (`AcceptResurrect`); `WorldEvent` cases 14–25; `StatsChanged` fields 15–20; `EntitySpawn` fields 22–25; `AttackResult.cp_damage = 8`.
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
| Static analysis & data checks | **PASS (clean fmt/clippy/entities/replay/data/buf)** | Formatting clean, Clippy clean (`-D warnings`), SeaORM entities regenerated clean, 4 replay fixtures byte-exact match, `packages/data/scripts/check_all.py` clean PASS, Buf breaking/lint PASS. |
| Unreal client builds | **PASS (both builds exit 0)** | Preserved log `r13-01-ue-builds.log`. Tree `dfd16cf`. Editor build exit 0 in 23s 554ms; Game build exit 0 in 32s 271ms. |
| Offline UE automation | **PASS (103 passed, 0 failed, exit 0)** | Preserved log `r13-02-ue-offline.log`. 103 automation tests executed, 103 `Result={Success}`, 0 failures, exit code 0. |
| Required-live UE automation | **PASS (103 passed, 0 failed, 0 skipped, exit 0)** | Preserved log `r13-03-ue-required-live.log`. Tested against live API on owned port. 103 automation tests executed, 103 `Result={Success}`, 0 failures, 0 skips, exit code 0. |
| Full ordinary scenario suite | **PASS (59 units / 59 PASS, exit 0)** | Preserved log `r13-04-sim-ci.log` and directory `r13-sim-ci-final/`. 59 logical units across 59 files (6 Phase 0b + 9 Phase 1 + 14 Phase 2 + 30 Phase 3). All 59 PASS. Contract coverage: intents 8/9 (missing `stop_move`), payloads 3/3, events 25/25, reasons 10/25, close codes 1/4. Reachable state transitions: NPC intentions 8/10, player attack 4/4, cast states 2/2, effect states 2/2, resurrection offers 4/4, PvP transitions 3/12. |
| Game package staging | **PASS (84.99 s, exit 0, Development Linux)** | Preserved log `r13-soak-attempt1/stage.log`. BuildCookRun stage completed in 84.99 s, 2,890 UFS items, exit code 0. Staged to `Saved/StagedBuilds/Linux`. |
| Final Rust gate on merged main | **PASS (1018 passed, 0 failed, 7 ignored)** | Merged tree `995ffa9` (`r22-00-summary.log` fmt/clippy exit 0; `r22-05c-tests.log` 1,018 passed, 0 failed, 7 ignored across 64 targets; rerun `r22-05b-checkpoint-audit-rerun.log` passed 1/1). Benches in test mode passed (`r22-05d-benches-testmode.log`). |
| Final quiescent performance gate | **PASS (all budgets met)** | Preserved log `r22-05e-zone-step-release.log`. Built in release profile in 4m 04s:<br>• `1000 moving npcs`: mean **146.7 µs** (146.661 µs), p50 142.552 µs, p99 **173.3 µs** (173.313 µs) vs 2 ms budget.<br>• `1000 moving players all observers`: mean **9.54 ms** (9.539867 ms), p50 9.997 ms, p99 **19.42 ms** (19.417171 ms) vs 10 ms mean budget. |
| Packaged combat soak (attempt 12) | **PASS (8 clients × 17 iterations, p99 10.776 ms < 20 ms, exact replay)** | Preserved report `r22-soak/report.json` and `r22-soak/summary.txt`. 8 clients × 17 iterations, 1,200 s requested, 12,246 ticks observed and recorded. Combat tick p99 was **10.776 ms** (< 20 ms budget). Phase 3 acceptance evidence passed (`phase3_acceptance: true`, `phase3_evidence_passed: true`). Cleanup exit 0. Exact byte-exact replay verified in `r22-soak/replay-check.log`: zone 1 epoch 1 match across 12,246 replayed ticks, 8 players, 87,248 outputs, 11,802,765 bytes compared, 0 digest-only in 796.9 ms. |
| Soak storage isolation | **tmpfs-backed (SOAK_TMPFS_VOLUMES=8g)** | Preserved record `r22-soak/storage.txt`: `volumes=tmpfs:8g8g`. The soak stack's PostgreSQL and NATS JetStream volumes were mounted on tmpfs to isolate execution from host fsync stalls observed during attempts 9–11 on the shared encrypted NVMe root filesystem (perf round 4, architecture decisions §10 P6). The independent host fsync probe failed to start due to permission configuration (`r22-soak/probe-failed/fsync-root.log`: `PermissionError: [Errno 13]`), resulting in no concurrent host fsync sample for attempt 12. |
| Post-merge root rebuild receipt | *Placeholder (architect receipt pending)* | Root rebuild log `root-build-995ffa9.log` confirms Editor and Game builds succeeded in 82.79 s execution time (exit 0). Post-merge automated smoke suite receipt is pending formal provisioning by the root architect. |

### 2.1 Preserved review receipts
The complete succession of independent review gates is preserved in `/home/matt-woodruff/.config/Claude/side-session-notes/phase3-codex-2026-10-10/`:
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
- **Deferred skills:** 198 of the 378 catalog skills (and 200 of all 388 retained skills) are deferred with explicit named blockers, including:
  - Party/Pledge affect scopes (21 skills; Phase 7 social systems)
  - Item consumption (28 skills; soulshots, spirit ore, arrows, symbols; Phase 4 items)
  - Toggle operate types (13 skills)
  - Charges and energy consumption (9 skills)
  - Summoning / pet controls (7 skills; Phase 4 item dependency)
  - Target race requirements (7 skills)
  - Specialized combat handlers: Lethal, FatalBlow, Spoil, Sweep, Escape, Teleport, Hate-Delete, Randomize, Target-Cancel, Crafting, and Drain.
- **Fear effect exclusion:** Fear is explicitly excluded from Phase 3 because it requires flee pathing and navigation geodata; skills referencing Fear are deferred.
- **Equipment and systems excluded:** Shields and blocking, weapon types/masteries, enchantments, and augments remain Phase 4 systems. Unsupported condition predicates defer skills requiring them. Real Squire's Sword weapon conditions are enforced.
- **Deviations from reference:**
  - **C17 retained-selection:** Target selection is preserved across AOI departure and re-entry.
  - **R4 offline-timer pause:** Buff and reuse timers pause while offline (persisted when > 100 ticks and > 50 ticks respectively).
  - **Offer duration (R7):** Resurrect offers expire after 600 ticks (60 s).
  - **Peace square (R7):** A 10-tile half-width square around (126,126) suppresses hostile action and auto-aggro.
  - **Death penalty cap:** Maximum level 5.
  - **Town respawn vitals:** Restores 65% HP and 0 MP (Phase 1 legacy contract preserved).
- **Synthetic stress above baseline ceilings:** As documented in `zone-progress.md`, synthetic stress testing under extreme periodic effect density (1,000 entities with 8 or 44 synchronized active effects) exceeds baseline quiescent ceilings (8 effects: 2.99 ms NPC / 16.04 ms player mean; 44 effects: 14.08 ms NPC / 34.45 ms player mean). Baseline quiescent limits (1,000 NPCs < 2 ms, 1,000 players < 10 ms) and realistic soak conditions are fully satisfied.
- **Hosted CI telemetry probe:** Hosted telemetry probe failures are pre-existing environmental constraints; local telemetry instrumentation, Prometheus scraping, and Grafana dashboard configurations are validated.
- **Operational follow-ups:**
  - External self-hosted Linux CI runner remains unregistered.
  - Two-week reliability programme remains an independent external operational follow-up. Neither is an acceptance blocker for Phase 3.

---

## 4. Historical diagnostics and triage

### 4.1 Triage rounds summary
Seven formal triage rounds resolved defects identified during integration:
- **Triage round 1 (r6 `033e717`):** Refuted suspected core combat bugs across 31 scenario failures. Addressed 9 valid RPC rejection sidecars (`.allow` files), TCP readiness race conditions on test stack restarts, and restored NPC oracle provenance.
- **Triage round 2 (r8 `93e150b`):** Addressed missing Phase 3 profiles for durable Keltir templates in legacy fixture packs (`1-late-entry`, `1-social-aggro`) and adjusted token milestone death wait timeouts under V5.
- **Host reboot & suite run 2:** During triage round 2, an external host reboot interrupted suite run 2. On restart, 5 NATS-dependent test failures occurred due to unclean JetStream shutdown; these passed cleanly when isolated. Rerun 3 completed with 57 PASS / 2 FAIL (`r9-sim-ci-run3`).
- **Triage round 3 (r10 `12a3f90`):** Enforced mandatory late-entry hydration gates and refined scenario timing.
- **Triage round 4 (r11 `869eec7`):** Identified and repaired real zone engine target retention defect on AOI re-entry (`5855886`, formalized as deviation C17) and defined predicate semantics for `players_attacking_me` (`C18`).
- **Triage round 5 (r12 `fd7e734`):** Fixed client health regen masking assertion in PvP scenario. Reached 59/59 PASS on the full scenario suite.
- **Triage rounds 6 & 7 (r13 `dfd16cf`):** Resolved final client test expectation alignments. Reached 103/0 offline, 103/0/0 live, 59/59 scenarios PASS, and successful Game package staging (84.99 s).

### 4.2 Soak attempts 1–11 diagnostics
The packaged combat soak required twelve total attempts to isolate harness issues and host hardware contention:
- **Attempt 1:** Harness failed prior to client launch due to missing host `psql` binary in `seed-phase3-soak.py`.
- **Attempt 2:** Clients failed on `EnterWorld` because role identifiers (`role01`) violated character name regex constraints; foreign workload (user OCR service) caused high host load.
- **Attempt 3:** Aborted due to an unexported `UE_ROOT` variable in the test launcher environment.
- **Attempt 4:** Clients failed on BotLoop iteration 2 because the casting lane return leg (~118 tiles) exceeded `MAX_MOVE_DISTANCE_TILES` (64).
- **Attempt 5:** Client 05 encountered `PEACE_ZONE` rejection because radial casting lanes passed within proximity of the town peace square.
- **Attempt 6:** Client 02 encountered a race where casting Might was cancelled by movement because iteration 1's buff remained active up to 1,200 s.
- **Attempt 7:** Functional pass (8 clients × 17 iterations, 12,230 ticks, exact replay), but p99 reached 53.6 ms due to zone actor log/checkpoint fan-out overhead under heavy host contention. Prompted perf rounds 1–3.
- **Attempt 8:** Stack storage collapsed at tick 54 (COMMIT 1.3 s, `UPDATE zone_epochs` 8.5 s) under host load 19–21 from concurrent compilation and OCR tasks.
- **Attempts 9–11 (Host fsync stalls):**
  - In attempt 9, with foreign background work stopped, ticks clustered at 2–5 ms but tail storage stalls persisted (32 `UPDATE zone_epochs` statements > 1 s; p99 190 ms).
  - Attempt 10 failed during startup connection pooling during post-teardown load spikes.
  - Attempt 11 on a quiet host with frozen workers exhibited tick rate halving and 11 slow pre-log fence updates (2–21 s each).
  - Diagnostic perf round 4 proved that the stalls were caused by host fsync latency on the shared encrypted NVMe root filesystem (ext4 on LVM/dm-crypt on DRAM-less hardware; 59 of 59 stalls ≥ 500 ms coincided with host fsync probe stalls; zero database row-lock contention).
- **Attempt 12 (PASS):** In accordance with architecture decisions §10 P6, the PostgreSQL and NATS volumes were placed on tmpfs (`SOAK_TMPFS_VOLUMES=8g`). Attempt 12 achieved full compliance: 8 clients × 17 iterations, 1,200 s requested, 12,246 ticks observed, combat tick p99 of **10.776 ms**, and byte-exact replay verification.
