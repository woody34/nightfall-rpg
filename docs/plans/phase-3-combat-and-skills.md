# Phase 3 Plan: Combat and Skills

**Status:** final acceptance documentation (closed root receipts). The local Phase 3 implementation is VERIFIED. It includes these functions:

- server-authoritative skill combat over the 39 populated class trees
- SP learning at the Class Master
- the casting lifecycle and reuse timers
- effect scheduling and abnormal stacking
- HP/MP/CP regeneration
- PvP flags, karma gain and decay
- CP damage absorption
- Death Penalty
- skill resurrection with offer expiry, and the peace square.

It operates on Snapshot 9 / Record 7 / Binary V5, and the legacy formats replay byte-exact. This evidence verifies it:

- offline UE automation (103/0)
- required-live UE automation (103/0/0)
- the full ordinary scenario suite (59 units / 59 PASS)
- Game package staging (84.99 s)
- the packaged casting soak (attempt 12: 8 clients × 17 iterations, 1,200 s requested, 12,246 ticks observed, combat tick p99 10.776 ms < 20 ms budget, no client errors, API and cleanup exit 0, on tmpfs-backed Postgres/NATS volumes — see §2)
- the final Rust gate on the merged tree (fmt/clippy PASS; 1018 passed / 0 failed / 7 ignored; release zone_step: 1000 moving NPCs mean 146.7 µs p99 173.3 µs vs 2 ms budget, 1000 moving players with all observers mean 9.54 ms p99 19.42 ms vs 10 ms mean budget).

Main and origin/main merged and pushed `995ffa9e37b065f540c12e6012a94509658f8035` on 2026-10-13. After the merge, the root Editor and Game builds exit 0 (04:35:42 / 04:37:06 UTC). The root offline automation without an API ran 102 passed / 1 failed (`CatalogueRpcFailures` must have an API that it can reach). The second run with the API up passed 103/103, exit 0 (05:58:28 UTC). Local Phase 3 is proven complete when this final reviewed docs commit merges. The external self-hosted Linux runner is still not registered. The two-week reliability programme is a separate operational follow-up (it is not a local acceptance gate or a new 14-day wait). The outcome summary [`phase-3-combat-and-skills-outcome.md`](phase-3-combat-and-skills-outcome.md) reconciles this status. **Author:** agy gemini-3.8-flash-high; bounded revision Codex gpt-6.1-sol / high. Architect: Claude. Independent plan review: Codex gpt-6-astra (Z).

## 1. Goal and locked decisions

The goal is server-authoritative skill combat for 39 populated class trees. The scope is SP/learning, resource/reuse casting, effects/status/regen, skill/heal aggro, death penalty/resurrection, player CP/flags/karma and one peace zone. Unreal sends intents and renders authoritative messages. Historic epochs keep their rules and bytes.

**§2 is this plan's canonical contract section.** It refers to the binding design, and it does not define competing mechanics in stories. The latest architect corrections replace older text. Scope/authority:

| Decision | Authority | Plan reference |
|---|---|---|
| No runtime pets/servitors; reservation hooks only | Owner §0.1 | §8 |
| Players targetable, CP/PvP/karma/counters, one town peace zone | Owner §0.2 | §2.6 |
| Pinned H5 defaults with independent oracle; research tuning remains knobs | Owner §0.3 | §2/§2.7 |
| SP at Class Master (126,128), radius3, critical learning | Owner §0.4 | §2.5/§5.2 |
| 100-ms seeded deterministic actor, Q i64/checked i128, no float/wall clock | Architect A1 | §2.3 |
| Additive transport, legacy isolation, durable retained mutations | Architect A2–A4/D-R9 | §2.1/§2.2/§2.5 |
| Peace geometry/offer lifetime are configurable architect choices | Architect R7 | §2.6 |
| Unlimited width0, complete hydration, final resurrection vitals, private stats/u64 ticks | Architect §9 C1/C2/C2′/C4 | §2.1/§2.4 |
| Independent review/serialized ownership | Architect R15 | §3/§6 |

There are no day estimates. The soak duration is a test-design choice. Measurements are pending.

## 2. Source authority and canonical contracts

**Design** = [phase-3-zone-design.md](phase-3-zone-design.md). **Decisions** = [phase-3-architecture-decisions.md](phase-3-architecture-decisions.md), all sections, including the newest §9 C1–C5/C2′. The Design now has a binding **ARCHITECT ERRATA** header: architecture-decisions.md §9 C5 replaces its obsolete 1/99 text and the other defaults that conflict. **Design §3 schedule, §4 version/codec and §5 persistence/LearnSkill stay authoritative. The committed schema in §2.1 is authoritative for concrete wire field allocations.** The tables here are allocation/version summaries and one scenario matrix. Story/test acceptance refers to them, and it does not use alternative contracts.

Inputs (all are preserved evidence): side-session-notes/phase3-codex-2026-10-10/data-review-1.md, side-session-notes/phase3-codex-2026-10-10/plan-review-2.md, side-session-notes/phase3-codex-2026-10-10/design-review-2.md and side-session-notes/phase3-codex-2026-10-10/skill-inventory/report.md. The [Planning template](../planning/README.md), the [API guidelines](../engineering/api-guidelines.md) and the engineering standards control the implementation. [Combat research](../planning/03-combat-and-skills.md) gives context. It is not runtime authority.

The pinned revisions agree with [SOURCES.md](../../packages/data/SOURCES.md):

- Game **abfde0490ac52a2106e884b7242c0276c44cdf76** — formulas/functions/caller behavior/config.
- Datapack **3ca488dd2bd0bfaca43e378886a3c2e37968153a** — skill XML/handlers/templates/karma rows.

The generator/oracle provenance must examine the full hashes, the source paths/literal lines and the input/output/vector hashes. Cached clones are provenance. They are not runtime dependencies. Formula citations come from Design §6. Count reports do not override its source ledger.

**Canonical C5 (architecture-decisions.md §9 C5; Design ARCHITECT ERRATA):** The abnormal-success defaults are **10/90**. Explicit per-skill min/maxChance values override them. Keep the immunity/no-draw exceptions. **MaxMAtkSpeed=1999, MaxPAtkSpeed=1500, MaxEvasion=250**. Missing attributes keep **magicLvl=0, castRange/effectRange=-1** (unlimited/none, as the source-specific usage defines), and **abnormalLvl=0**. The generator and the independent oracle keep these values and semantics. The engine loads the clamps from **tables/formulas.toml**, and it never hard-codes them. §5.1 makes boundary/default regression vectors necessary.

### 2.1 Additive wire allocation and visibility

Implement **Design §2 + C2/C2′/C4**. The committed contract **241293f** on **codex/phase3-server-api**, [world.proto](../../packages/proto/nightfall/v1/world.proto) and [game.proto](../../packages/proto/nightfall/v1/game.proto), is authoritative for concrete field allocations. The preserved evidence side-session-notes/phase3-codex-2026-10-10/proto-review-1.md (**APPROVE WITH REQUIRED CHANGES**) keeps three handoff conditions:

1. Append V5 **TargetChanged.tick**. Document its relation to baseline_tick. Test the baseline acceptance, the stale-delta rejection and the subsequent selection, with the zero/default legacy encoding unchanged.
2. Test all three RPCs for unauthenticated **UNAUTHENTICATED** and authenticated rollout **UNAVAILABLE**. Test all three WS intents for exactly one sequence-preserving **NOT_YET_IMPLEMENTED** rejection without actor mutation (a refused learn request leaves SP/skills/receipts unchanged).
3. Document the catalogue **UNAVAILABLE** so that it agrees with the handler, including catalogue unavailability and the rollout stub.

Keep the old envelopes/tags/bytes.

| Tag space | Allocation summary | Audience / authority |
|---|---|---|
| ClientMessage | 16 CastSkill; 17 CancelCast; 18 AcceptResurrect | Design §2; LearnSkill is GameService RPC, no WS tag. |
| WorldEvent | 14 SkillCastStarted; 15 SkillCastResult; 16 SkillCastCancelled; 17 EffectApplied; 18 EffectRemoved | AOI-filtered lifecycle + owner; recursive reference filtering. |
| WorldEvent | 19 SkillReuse; 20 SkillLearned; 22 ResurrectOffer; 23 SpGained; 24 SkillStateSync | Owner-only. |
| WorldEvent | 21 PvpStateChanged; 25 VitalsChanged | Owner + observers knowing subject. |
| StatsChanged fields | 15 karma; 16 pk_count; 17 pvp_count; 18 flag_until_tick; 19 death_penalty_level; 20 effective_stats | **Entire message owner-only (C4)**; reuse SP11/CP8/maxCP9/tick14. |
| EntitySpawn fields | 22 flag_until_tick; 23 karma; 24 repeated ActiveEffect; 25 SkillCastStarted pending_cast | Identity then hydration; death penalty private. |
| AttackResult | Field8 cp_damage | V5 CP removed. |
| RejectReason | 13–24, Design §2 exact names | Pre-accept disposition; separate cancellation enum. |
| Replay commands/loads | PbCommand16–19; PbPlayerLoad.phase3=9; PbNpcCombat.phase3=6 | Design §4.4, separate numbering. |
| Replay events/rejects | Events20–29/private30/projection31; rejects from21 | Design §4.4 allowlists; public24/25 use projection payload. |

**All tick/deadline/remainder fields are uint64/Tick(u64)** with checked arithmetic. StatsChanged is WorldEvent **7**, and AttackStarted is **11**. StatsChanged **field11** is SP. Public PvP data goes only through PvpStateChanged/EntitySpawn. MP/CP totals, SP/XP and learning/offers stay private.

V5 AttackResult.damage is the HP removed. SkillCastResult gives separate rolled/CP/HP amounts. **Legacy rolled-overkill output bytes stay unchanged** (Design §3). VitalsChanged covers HP **or effective maxHP**, including changes to the max only. Clients set absolute values, and they never subtract two times.

Binding hydration corrections:

- First, all V5 identities, with active_effects empty and pending_cast/pending_swing absent. Then the baseline EffectApplied/**SkillCastStarted** hydration with stable IDs/incarnations/deadlines, plus **baseline AttackStarted for existing swings (C2)**. There is no separate PendingCast type. There are no animations/transition counts for baselines.
- After the batch, filter every nested endpoint recursively against the known visible incarnation. A hidden entity and its incarnation are absent together. Keep the visible subject summary, but do not leak the full source attribution.
- The baseline exceptions include owner **StatsChanged/TargetChanged/SkillStateSync**, the public **PvpStateChanged** bootstrap, and effect/cast/swing hydration. Make the final observe values and accept them at the baseline tick, including on an idle reconnect. The ordinary mutation suppression stays.
- A same-ID resurrection in the existing AOI emits the final **VitalsChanged after EntityRespawned** with a baseline exception (C2′). Suppress the vitals only if that observer received the final HP/maxHP pair.

The E2.4/E5.5 evidence covers mutual cast/swing newcomers, hidden endpoints, empty reset, idle replacement/reconnect and resurrection maxHP change.

### 2.2 Version matrix and legacy isolation

Use **Design §4.1–§4.5**, and do not make the current validation wider. This is the only compatibility table.

| Snapshot | Digest | Classes | Record | Policy |
|---|---|---|---|---|
| 4,5 | JsonV1 | false | 3 | Historic Phase1 |
| 6..8 | JsonV1/BinaryV2 | false | 3 JSON/4 V2 | Existing no-class matrix |
| 7 | BinaryV3 | true | 5 | Phase2 before tokens |
| 8 | BinaryV4 | true | 6 | Phase2 tokens |
| 9 | BinaryV5 | true + valid Phase3 block/rules/hash | 7 | Phase3 only |

Reject illegal snapshot/digest/record pairs. Also reject **recursive Phase3 presence even empty/default** under legacy nested tags/loads/projections/receipts/outbox. Validate the raw protobuf/JSON before the defaults discard the presence. The encoder and the digest-only reproduced outputs share context. The legacy policy gets no new combat/regen/SP/PvP/peace/draw/baseline behavior. Keep the old serializer/digest bytes, and keep record3–6 decoding. The .nfr container1 does not change.

Design §4.2/§4.3 controls the complete embedded rules, the canonical state/hash/pending lanes and the rebuilt indexes. A snapshot restore keeps short/exact timers, but a logout admission does not. The NFSNAP9 envelope is magic + LE expanded length + zlib, with no separate envelope hash. **Before freeze**, measure the full registry +1,000 full-effect entities against ~1MiB encoded/32MiB decoded, without truncation or external replay fetch. A larger limit/bundle must have an architect disposition after the measurement.

The **pre-run** byte admission of Design §4.5 sets limits on complete normalized loads/dispositions/recipients, FIFO prefix/suffix, oversize terminal failure, immutable gate retry and lost-ack recovery. The output digest fallback alone cannot set a limit on retained commands (§5.6).

### 2.3 Engine state and scheduling

**Design §1** defines the module/state/identity/handler/Func/fraction contracts. **§3 is the complete V5 schedule/edge table**, and this plan does not copy it. Keep the legacy run_tick. The eight-command budget is per session in the actor. Examine these items: staged command order, separate launch/impact, cast ordinal versus auto-attacker EntityId order, expiry/pulse/completion/impact edges, immediate death, ordered AI requests and coalesced output.

Design §1.2/§6.1 binds the animation/continuous head start/launch/impact and the **independent animation-derived interrupt deadline**. The completion-minus-two rule is withdrawn. coolTime/hitCancelTime stay imported but not used, Idle/Casting only. Design §1.3/§1.4 binds slots/stacking/status overlaps, player/NPC damage-break, periodic ideal-ms DIFF/PER, and rebuild without one-time heals/RNG/cost. TickHp is nonlethal (HP1/CP). TickHpFatal is deferred. Register every new zone file in SOURCES. Production stays ordered and integer-only, with no wall clock.

### 2.4 Generated data and capability manifest

**R13 + Design §1.1/§6**: generate every rank of the 378 tree catalogue ids +5076/transitive closure. Include the source hashes, the exact literals, the computed status and the blockers. A future gen_skills.py --check must reproduce them. Never edit the status manually.

The machine-readable **capability manifest** of E1.2 validates these items: each effect **parameter**, each individual stat/operation/priority, target/operate type, scope/**filter**, recursive and/or/not leaf, effect/self/start/end/channeling/pvp/pve section, item prerequisite and referenced dependency. An unknown item defers the whole skill with named blockers. Recognition of handler-name/mul/add/sub alone is not sufficient. The runtime rejects manifest drift. It applies **using kind against actual equipped Squire's Sword** and canResurrect against state. The admitted set/types of Design §1.1 is authoritative. Unsupported racial/mastery/trigger behavior cannot become partial execution. **C1: affectLimit width0 →0/no draw/unlimited**, and the population is the limit. A positive width uses pinned sampling.

Inventory **241/379 is historical only, never a gate**. Data-branch generator report: **211 executable / 167 deferred of 378**, **389 including extras** (220/169 overall: 5076, nine referenced ids, sample-only 286). These are **reported classifications from the rejected data revision**. They wait for fixes, regeneration and an independent new review under the preserved evidence side-session-notes/phase3-codex-2026-10-10/data-review-1.md. A pass of generator **--check does not establish TOML validity, capability correctness or oracle correctness**. All eleven findings stay handoff blockers until a review closes them:

- invalid generated TOML
- lost stat conditions
- zeroed per-level ranges
- wrong short continuous-cast expectations
- TickHp rounding
- missing parameter/operation/filter admission
- lost locked reuse semantics
- wrong source defaults
- false universal karma-equivalence claim
- absent required adaptation coverage
- wrong provenance symbols/lines.

Generate the exact totals/blocker histogram again against the reviewed capabilities, and explain the changes. The counts are not targets, and they are not local validation.

The runtime [keltir.toml](../../packages/data/npcs/keltir.toml) **id keltir** gets **sp_reward=2**, with the provenance pinned to Young Keltir **20545**. Keep the tunable XP28/stats. Design §3/R6 controls the credit rules. The living lethal killer gets the credit, and the credit is the actual saturation delta. There is no sharing and no Human XP bonus on SP.

### 2.5 Persistence, recovery, admission and learning

**Design §5.1 is full persistence acceptance; §5.2 admission; §5.3 LearnSkill.** Extend the memory/PostgreSQL DTOs/adapters/fingerprints/pending snapshots/recovery/outbox together. Keep the old fingerprints. Do not add a new backend.

[000007](../../apps/api/src/infrastructure/postgres/migrations/m20261009_000007_character_classes.sql) already controls characters.sp/cp, character_class_slots.sp and the learned rows. **000008 adds no SP column**. Keep the ClassState/active-slot projection equality, and do not add a duplicate skill ledger. The new Design §5.1 scalars default to zero. Karma/counters are u32-bounded bigint, the penalty is 0..5 and the fractions are 0..Q−1. The timer/XP/death bounds are checked and nonnegative. The optional death fields make a consistent record. The new reuse/effect child rows have deterministic keys/FK cascade and **atomic replacement in fenced final state+receipt+ordered outbox transaction**.

The critical triggers of Design §5.1 include retained effect/reuse apply/refresh/remove/expiry, flag refresh/expiry, learning/SP/PvP/death/resurrection/class/passives and **new MP/CP/fraction mutations**. Freeze the request/fingerprint/revision/remainders. There is one final character write per tick. A deadline countdown alone is not a per-tick write. Only the old ordinary HP/XP cadence exception and the existing outbox relay stay.

Retention is strict: **50/51 reuse,100/101 effects**, flags without threshold, and an untimed penalty. Keep the source/periodic ideal-ms phase/fractions and the relative order of shared passive effects. A real logout removes short entries/offers but keeps the death loss. A snapshot keeps exact timers. A live replacement keeps the cast/offer/deadlines without a reload. Rebase at the actual admission, with no wall-time catch-up and no hooks/RNG/cost replay.

For new work, the learn operation authenticates ownership and the current generation. It looks up the receipt before eligibility, and it uses the exact-rank/fingerprint/frozen result under API §2/Design §5.3. A matching historical success stays valid after later progression/class/disconnect. A failure uses no key/SP. Duplicate queued keys settle one time. A loser with a different key for the same rank cannot buy the next rank. The log ack → fenced checkpoint ack occurs before output/success. The **API §4 endpoint matrix (§5.2)** and the full parity/ack/crash/fence acceptance (§5.4) are mandatory.

### 2.6 Aggro, PvP, peace, death and resurrection

**Design §3** binds the damage/credit/aggro/PvP truth table/death. **§1.2** binds geometry/offers, and **§6** binds math. most_hated stays the **current target when tied, else lowest EntityId**. Skill `\|effectPoint\|` applies one time per applicable skill/target, with actual-heal ledger fanout. AddHate changes hate. **GetAgro enqueues forced NPC intention at AI with no hate increment**. It is not a TargetMe/taunt lock.

PvP uses the pre-impact victim flag/karma and the pre-increment killer PK, with D-R4 counter precedence. **1200/600 ticks =120s/60s**. The rule is actor-only **max(old_deadline,now+duration)**, also when a neutral attacker hits a flagged victim. A hit never flags or refreshes the victim. There is no healer/time/death karma decay. Player DoT goes to CP first. The truth table stays in Design §3.

The peace zone is an inclusive square with center126,126 and half-width10 tiles=10,000 milli-tiles, and it contains the Class Master. This is **architect R7**, not owner/source. The check uses both endpoints at acceptance/impact/AoE/pulse: **IntentRejected only pre-accept**. After acceptance, the cast/swing cancels or the recipient/pulse is blocked. Suppressed pulses advance. NPC acquisition/leash/protection follow the Design.

Skill resurrection 1016 uses an explicit **PC_BODY CastSkill target**, separate from the client live-player SetTarget. Verified D-R5′: if the HP is lower than 65% of the effective max, the HP goes up to 65% with Q restoration. **MP/CP unchanged**. The sequence is corpse, no town protection, new incarnation, then XP/level/passive/final vitals. Town respawn does not change. Offer **expires_at = created_at + 600** checked ticks. The first valid offer is kept. Decline/expiry/town/accept/real disconnect makes the offer invalid, but a replacement keeps it. An accept at expiry loses. A reconnect keeps the actual death-loss record, but no offer. A new offer can use the record. The pinned refund rounding uses the loss one time, and it is crash-safe (§5.5). Penalty5076 is private, untimed and cap5, with the Design eligibility/no-draw rules.

### 2.7 Enumerated formula/rounding ledger

**Design §6/§6.1/§6.2** binds independent vectors for each family (there is no unsupported fixed formula-count claim):

1. Cast timing/static/speed/floor, reuse/MP1/MP2 modifiers; **HP>cost, cannot self-kill**.
2. Physical-skill evasion/damage/actual skill-crit caller; old auto-attack frozen.
3. Magic damage/crit/both failure rolls/level-gap, success **binary32 then Math.round**.
4. Effect success/duration/traits/vulnerability, immunity/blocked/no-draw and second-ceil; min/max not universal.
5. Attack-break caller order/cutoff, zero-reflection/vengeance/SELF/out-of-window draws.
6. Exact attribute piecewise coefficients/boundaries/None/mismatch.
7. Ordered Func/intermediate casts/getter caps/magic-crit scaling/MaxHp lifecycle/install order.
8. Individual Heal/HpPerMax/Mp/Cp guard/static/crit/cap/signed branches and periodic DIFF/PER/binary32 amount.
9. Player HP/MP/CP versus **NPC HP/MP regen**, no player scaling/posture/CP on NPC; fractions/clamps.
10. Skill/AddHate/**actual-heal hate**, zero overheal/multiple ledgers.
11. Karma exact literals/compound truncation/pre-increment PK/pre-award level.
12. Death loss and resurrection quirk/**XP Math.round**, reduced actual loss, one-time refund/caps.

The Fraction/Decimal oracle reads pinned literals, not Rust/generated Q expectations. The manifest records revisions/path/line/input/roll/intermediate/rounding/branch/output/hash. Sampling agreement is evidence, not universal proof. At straddling boundaries, Design §6.2 makes checked sqrt bounds/refinement, or a proven bounded equivalent, necessary. Unresolved precision rejects the registry. Production stays integer-only. The §5.1 tests and the scenarios compare received outcomes to external oracle constants.

### 2.8 Canonical scenario matrix

This is one canonical story → file → authoritative assertion matrix. The files are proposed under apps/client-unreal/Scenarios. The first nonempty header is `# covers: ` plus the exact payload of the middle column. The simulation story of each epic refers to this section. **First ship 3-cast-skill: learn → cast → MP/reuse**. Regen covers only regen stories. The E1.2 status and the real damage/debuff have dedicated assertions. §5.3/§5.6 supply role/record/replay evidence. The E5 pixels/assets must have separate automation/cooking, with explicit rendering subscopes.

| Scenario file | Covered stories / exact covers payload | Authoritative assertions and fixture purpose |
|---|---|---|
| `3-cast-skill.nfs` | `3 E1.4, 3 E1.6, 3 E2.1, 3 E2.2, 3 E2.7, 3 E3.2, 3 E3.3, 3 E5.1, 3 E5.2, 3 E5.5, 3 E5.6, 3 E6.7` | Test rank initially unlearned: query/learn at trainer, receive receipt/SP/rank, leave peace, cast NPC, assert MP1/start reuse then MP2/result/ready boundary. Magic outcome/HP/None attribute matches oracle; full coefficient boundaries are oracle-only. Separate automation examines the UI. |
| `3-cast-rejected.nfs` | `3 E1.2, 3 E1.6, 3 E2.1, 3 E2.7, 3 E6.7` | Catalogue deferred status/blocker and runtime refusal; unknown/unlearned/reuse/MP/status/range/real-weapon guards. Accepted completion failure cancels, never second disposition; cancel/move/damage retains MP1/reuse. |
| `3-learn-skill.nfs` | `3 E1.2, 3 E1.6, 3 E3.1, 3 E3.2, 3 E3.3, 3 E3.5, 3 E5.1, 3 E5.4, 3 E5.5, 3 E5.6, 3 E6.7` | Executable/deferred/blockers/costs match manifest. Runtime keltir earns one SP 2, exact-rank learn debits, same-key retry returns all frozen fields with no second event, including progression/reconnect. Competing/cap variants in PvP roles. |
| `3-learn-rejected.nfs` | `3 E1.2, 3 E1.6, 3 E3.2, 3 E3.3, 3 E3.5, 3 E6.7` | Status/unlearnable reasons and SP/level/lineage/rank/distance/item/engaged/dead/casting/generation/key failures; state unchanged/no event/key. Failed key later usable; ownership/concurrency separately §5.2. |
| `3-buff-expires.nfs` | `3 E2.2, 3 E2.3, 3 E2.7, 3 E5.3, 3 E5.6, 3 E6.7` | Stable IDs/deadlines, equal/higher replace/lower silent rejection, oldest overflow, removal-before-apply, expiry restores stats/maxHP/no phantom pulse. Include capped heal/dispel outcomes; HUD separate. |
| `3-debuff-status.nfs` | `3 E1.5, 3 E1.6, 3 E2.2, 3 E2.3, 3 E2.7, 3 E6.7` | Actual physical damage/debuff status/duration oracle match; seeded resist/immune plus engine traces. Stun-before-swing/root/overlap, player/NPC direct/DoT break and HP1/CP nonlethal pulses. No probability inference from few casts. |
| `3-regen.nfs` | `3 E1.3, 3 E1.6, 3 E6.7` | Natural regen only: 30-tick player HP/MP/CP standing/moving and NPC HP/MP oracle deltas, dead/full no-growth, multi-pulse fractions through whole outputs. Low-resource pre-admission seeds; no damage/debuff coverage. |
| `3-cast-observer-a.nfs` | `3 E2.1, 3 E2.4, 3 E2.7, 3 E5.5, 3 E5.6, 3 E6.7` | Paired caster receives ordinary start/result/cancel and final/max-only vitals; AOI-filtered AoE. Mutual cast/swing newcomers participate; peer frames contain no owner stats. |
| `3-cast-observer-b.nfs` | `3 E2.4, 3 E2.7, 3 E5.3, 3 E5.5, 3 E5.6, 3 E6.7` | Paired mid-cast/effect/swing entry: all identities then baseline effects/casts/AttackStarted, exact IDs/incarnations/deadlines, hidden endpoints absent, no animations/double application. Empty reset, public maxHP/PvP bootstrap. |
| `3-cast-reconnect.nfs` | `3 E2.4, 3 E2.6, 3 E2.7, 3 E5.5, 3 E5.6, 3 E6.7` | 50/51 reuse,100/101 buff/debuff, flags/fractions/phase/install-order variants. Real logout restores only retained; replacement preserves live cast/offer. Idle stats/selection/sync/bootstrap accepted without later mutation/no repeated heal. |
| `3-death-penalty.nfs` | `3 E4.4, 3 E4.6, 3 E6.7` | Seeded eligible roll outcomes/cap/ineligible trace, actual XP loss/delevel/death record and private untimed penalty survive reconnect. Cast/swing/effect/fraction cleanup; town rule unchanged. |
| `3-resurrect-a.nfs` | `3 E2.4, 3 E4.5, 3 E4.6, 3 E5.4, 3 E5.5, 3 E5.6, 3 E6.7` | Paired caster explicit corpse target/first offer; competing offers retained. Public corpse revival and EntityRespawned→final VitalsChanged for same-ID unchanged-AOI/maxHP change; hidden source filtering. |
| `3-resurrect-b.nfs` | `3 E2.4, 3 E4.5, 3 E4.6, 3 E5.4, 3 E5.5, 3 E5.6, 3 E6.7` | Dead owner offer/id/relative expiry/loss; HP 65% if lower/MP-CP unchanged/one refund. Variants: decline/disconnect/replacement/competition/exact-expiry/duplicate/town race/reconnect new offer. Crash one-time proof linked to §5.4. |
| `3-pvp-flag-a.nfs` | `3 E3.1, 3 E3.5, 3 E4.2, 3 E4.3, 3 E4.6, 3 E5.3, 3 E5.6, 3 E6.7` | Paired actor 1200 ticks/victim unchanged/CP/privacy; pre-accept peace reject versus later block/cancel/both-endpoint/AoE/pulse. Competing keltir nonlethal actor gets 0 SP, role reversal/near-cap actual delta once. |
| `3-pvp-flag-b.nfs` | `3 E3.1, 3 E3.5, 3 E4.2, 3 E4.6, 3 E6.7` | Paired neutral→flagged actor 600 ticks/max(old,now+duration), no victim refresh; impact expiry/counter branches. Lethal keltir killer receives exactly one SP 2/event, later corpse/reconnect/replay no reaward; reverse roles/balances. |
| `3-pvp-karma-decay.nfs` | `3 E4.2, 3 E4.6, 3 E6.7` | Controlled peer role: neutral PK/pre-increment karma, flagged PvP, chaotic-unflagged neither; monster actual XP/pre-award level/exact multiplier decay. No healer/time/death decay; full cross-product engine suite. |
| `3-aggro-skill-hate.nfs` | `3 E4.1, 3 E4.6, 3 E6.7` | Multiple NPC ledgers/cooperating player: effectPoint once, AddHate/actual-heal/zero-overheal. Tie current else lowest EntityId; GetAgro forced intention/no hate and ordered request interactions; precise stale/no-op cases engine suite. |

## 3. Engineering architecture, ownership and review gates

Follow clean architecture and the module/hook table of Design §1.1. The domain has immutable drafts/facts, the application has durable coordination/ports, the infrastructure has memory/PostgreSQL/JetStream, and the interface has reviewed DTOs. Unreal only renders.

- **Z: Codex gpt-6-astra/high** is responsible for E2/E6.1/E6.2 and the shared zone state.rs/state_combat.rs/command.rs/state_digest.rs, the replay record/codec and the snapshot codec. Z publishes typed damage/hostility/death/learn/stat/persistence hooks before C module integration. Z serializes edits to shared files.
- **C: Codex gpt-6.1-sol/high** is responsible for the first additive proto/receipt/RPC contract, the E3 schema/adapters/state_learn, the E4 isolated state_pvp/state_death against Z hooks, and the E5/E6 harness/soak/metrics. **D: agy gemini-3.8-flash-high** is responsible for the E1 data/oracle/formula handoff and the E6.6 docs/dashboard. C wrote the current bounded revision. D formula patches to shared modules go through Z.
- Independent handoffs: C proto/E3/E4/E5/harness → Z; D data/oracle → Z/C; Z E2/runtime/admission and **Z-authored E6.1/E6.2 → C**; D docs/dashboard → Codex; this plan → Z. No author is the only reviewer of their own work. Later, root integrates/merges/pushes and serializes all Unreal activity.

The runtime restore review occurs before the separate admission integration. E3 uses the published types/hooks, and **E2.6 consumes E3.4 adapters/E3.3 receipts**. The foundation/live gates are separate (§6), and there is no circular dependency. Proto has one editor at a time.

## 4. Epics, stories, models and Done When

Status: **the implementation continues in sibling worktrees. No story is marked complete. This plan revision did not run implementation or execution gates. Phase acceptance is pending**. Tags are only in the titles of client-observable stories. After evidence/review, future completion uses `| ✅ N.M Title … |` in the first cell of this plan. The parser of this time reads tags from the whole line, thus the prose has no tags. E6.5 title-cell hardening is future work, and it is not necessary to parse this plan.

### Epic 1: Skill data and independent formula foundation (D; Z/C review)

| Story | Model | Work / Done When |
|---|---|---|
| 1.1 Independent formula oracle and provenance ledger [required] | D | Create independent Fraction/Decimal oracle/provenance for §2.7/Design §6. **Done When:** Reviewed source/vector hashes, rounding/precision and injected traces satisfy §5.1 without Rust-generated expectations. |
| 1.2 Skill XML generator and capability manifest [client-visible] [required] | D | Generate definitions/catalogue and R13 capability manifest (§2.4). **Done When:** Every rank/parameter/stat/filter/section/reference checked, named blockers/exact generated counts reported with provenance, runtime weapon/manifest validation and §2.8 status assertions pass. |
| 1.3 Effective-stat and regeneration formula foundation [client-visible] [required] | D | Supply pure stat/regen foundation against E1.1/Design §1.4/§6; Z integrates shared modules. **Done When:** Cold/cache and player/NPC branches match oracle, old sheets unchanged, §2.8 natural regen passes. |
| 1.4 Attribute and magical damage/success formulas [client-visible] [required] | D | Supply attribute/magic functions and bounded precision under §2.7. **Done When:** All oracle coefficient/crit/failure branches pass; 3-cast-skill asserts real magic damage/None attribute from results/vitals. |
| 1.5 Physical-skill, debuff and duration formulas [client-visible] [required] | D | Supply physical-skill/debuff/duration functions (§2.7). **Done When:** Source evasion/crit, immunity/no-roll/clamp and cursor vectors pass; 3-debuff-status verifies actual damage/status/duration. |
| 1.6 Simulation scenario(s) [client-visible] [required] | D | Author E1 assertions in §2.8 with C harness integration. **Done When:** Status/cast/debuff/regen evidence passes; 3-regen covers only E1.3/E1.6 plus harness simulation; foundation handoff precedes live completion. |

### Epic 2: Zone cast/effects engine and version boundary (Z; C review)

| Story | Model | Work / Done When |
|---|---|---|
| 2.1 Cast lifecycle, resource costs, reuse and interruption [client-visible] [required] | Z | Implement Design §1.2/§3/§6 staged casts, launch/impact/cancel. **Done When:** §5.1 timing/caller/MP/reuse/HP-safety vectors and §2.8 cast-skill/rejected pass; one pre-accept disposition, later cancel/result; first scenario learns first. |
| 2.2 Instant handlers and unified V5 damage/restoration [client-visible] [required] | Z | Implement Design §3/§6 instant handlers/shared damage/restoration, publish E4 hooks. **Done When:** §5.1 oracle/draw/conservation and §2.8 damage/heal/dispel pass, A2 distinction preserved, legacy bytes unchanged. |
| 2.3 Effect container, status, modifiers and periodic scheduling [client-visible] [required] | Z | Implement Design §1.3/§1.4 effects/status/periodic/regen caches. **Done When:** §5.1 pools/stack/Func/hooks/phase vectors and §2.8 buff/debuff pass; reconstruction repeats no heal/RNG/cost; regen has dedicated evidence. |
| 2.4 Observer-safe wire baselines and lifecycle projections [client-visible] [required] | Z | Produce §2.1 observe DTOs; C maps them. **Done When:** C2/C2′/C4 privacy/identity/bootstrap and observer/reconnect/resurrection cases pass, including swings/idle owner/same-ID maxHP; no observer StatsChanged. |
| 2.5 Snapshot/digest isolation and actor byte admission [required] | Z | Implement Design §4 runtime snapshot/digest/codec/pre-run byte admission (§2.2). **Done When:** §5.6 legal/illegal/nested/legacy/FIFO/retry checks and pre-freeze worst-size measurement pass. No dependency on E2.6 live admission. |
| 2.6 Character admission restore and paused offline timers [client-visible] [required] | Z | Restore per Design §5.2, explicitly dependent on **E3.4 memory/PostgreSQL adapters/E3.3 receipts**. **Done When:** §5.4 order/phase/fraction/50–51/100–101 and 3-cast-reconnect pass; replacement preserves live state. Separate from E2 runtime review. |
| 2.7 Simulation scenario(s) [client-visible] [required] | Z | Author engine assertions for §2.8 cast/effect/observer/reconnect files; C implements DSL/fixtures. **Done When:** Mapped scenarios/variants pass runner/record/replay/contract gates with independent C review. |

### Epic 3: SP, learning and critical persistence (C; Z review)

| Story | Model | Work / Done When |
|---|---|---|
| 3.1 Runtime keltir SP and exactly-once lethal credit [client-visible] [required] | C | Add reward to runtime keltir.toml **id keltir**, source 20545 (§2.4); use Z kill hooks. **Done When:** §5.4 cap/recovery and §2.8 learn/PvP role-reversal prove living lethal killer gets one actual SP delta, others zero; XP/stats retained. |
| 3.2 Skill catalogue and ListLearnableSkills RPC [client-visible] [required] | C | Implement authenticated catalogue/ListLearnableSkills (§2.5). **Done When:** §5.2 read matrix/no-write/ownership and §2.8 next-rank/cost/status/blocker assertions pass; eligibility is authoritative. |
| 3.3 LearnSkill RPC and actor receipt lifecycle [client-visible] [required] | C | Implement Design §5.3/API §2/§4 exact-rank actor/receipt lifecycle. **Done When:** Full §5.2 unit/wire/adapter matrix and live learning pass, queued duplicates/races spend once, failures consume nothing, historical frozen responses remain exact. |
| 3.4 Migration 000008, checkpoint adapters and recovery [required] | C | Extend 000007 through 000008/ports/adapters/recovery under Design §5.1 (§2.5). **Done When:** No duplicate SP; bounded defaulted state/atomic child+receipt+outbox replacement and §5.4 parity/ack/crash/fence pass; publish adapters to E2.6. |
| 3.5 Simulation scenario(s) [client-visible] [required] | C | Author §2.8 learn-skill/rejected and SP assertions in pvp-flag-a/b. **Done When:** Catalogue/frozen retry/failure/competing-SP runs pass with §5.2/§5.4 race/crash evidence; no solo-kill overclaim. |

### Epic 4: Aggro, PvP, peace, death and resurrection (C; Z review)

| Story | Model | Work / Done When |
|---|---|---|
| 4.1 HateLedger integration and forced NPC intention [client-visible] [required] | C | Integrate Design §3/§6 hate/AI hooks: `\|effectPoint\|`, actual-heal/AddHate and GetAgro intention. **Done When:** §5.1/§2.8 aggro cases preserve current-target else lowest EntityId tie and no GetAgro hate; TargetMe deferred. |
| 4.2 PvP flags, counters, karma and CP [client-visible] [required] | C | Implement state_pvp using Design truth table/math and Z damage hooks (§2.6). **Done When:** §5.5 cross-product/expiry/cap plus §2.8 CP/actor-only max-deadline/counter/karma runs pass without duplicate persistent effects. |
| 4.3 Architect R7 peace geometry and hostile boundaries [client-visible] [required] | C | Ship **architect R7** square/hostile enforcement (§2.6). **Done When:** §5.5 inclusive/both-endpoint/AI vectors and pvp-flag-a show pre-accept reject, later cancel/block and suppressed pulses advancing. |
| 4.4 Death record, XP loss and untimed penalty [client-visible] [required] | C | Implement Design death/record/penalty through Z hooks. **Done When:** §5.5 eligibility/no-draw/cleanup/delevel and death-penalty scenario pass; actual loss/penalty reconnect, no double charge, town rule unchanged. |
| 4.5 Resurrection offers, consent and crash-safe one-time revival [client-visible] [required] | C | Implement Design offer/consent/refund through an explicit corpse cast (§2.6). **Done When:** Every §5.5 lifecycle/crash and §2.8 role variant passes, preserving MP/CP and one XP refund/final maxHP. |
| 4.6 Simulation scenario(s) [client-visible] [required] | C | Author §2.8 aggro/PvP/karma/death/resurrection files including `\|effectPoint\|`/tick evidence. **Done When:** All mapped variants and §5.5 precise actor/adapter cases pass recordings/gates, reviewed by Z. |

### Epic 5: Unreal presentation and authoritative projections (C; Z review)

| Story | Model | Work / Done When |
|---|---|---|
| 5.1 Learned skills, hotbar, keybinds and cast targeting [client-visible] [required] | C | Build learned/hotbar/keybinds; **local corpse selection separate from SetTarget**, explicit CastSkill PC_BODY (§2.6). **Done When:** §2.8 cast/learn and named drag/drop/key/cooldown automation pass. |
| 5.2 Cast bar, cancellation feedback and placeholder FX [client-visible] [required] | C | Render authoritative cast progress/cancel/reason and placeholder FX. **Done When:** §2.8 protocol plus named bar/FX automation pass, baseline animations suppressed; Ack alone never implies impact. |
| 5.3 Effect strip, target vitals and PvP display [client-visible] [required] | C | Render authoritative effects/deadlines/vitals/PvP precedence, no invented stack count/private totals. **Done When:** §2.8 buff/observer/PvP protocol and named icon/countdown/color/max-only UI automation pass. |
| 5.4 Trainer dialog and death/resurrection consent screen [client-visible] [required] | C | Build exact-rank/key trainer and offer-id Accept/Decline/town controls. **Done When:** §2.8 learn/revive lifecycle and named UI/error/consent automation pass; absent caster honest, invalid prompt cleared. |
| 5.5 Client baseline, tick/incarnation/generation fences and cooking [client-visible] [required] | C | Implement §2.1 baseline/tick/incarnation/generation fences and asset cooking. **Done When:** Observer/idle reconnect/same-ID resurrection scenarios, reducer automation and zero-missing-reference cooking pass. |
| 5.6 Simulation scenario(s) [client-visible] [required] | C | Integrate §2.8 E5 protocol scenarios; separately run **run-tests.sh --require-live-api**/cooking. **Done When:** Named rendering-only subscope evidence links actual tests/source/scenarios, protocol replay passes; no pixel claim from .nfs. |

### Epic 6: Harness, replay, performance, telemetry and evidence

| Story | Model | Work / Done When |
|---|---|---|
| 6.1 Replay schema 7, V5 fixture and coverage schema 3 [required] | Z | Implement Design §4.4/§8.2 replay, **coverage schema3** and V5 fixture; preserve baselines. **Done When:** §5.6 record/transition/reason/merge/byte-mutation checks pass and **C reviews Z output**. |
| 6.2 Combat/zone/observer and durability benchmarks [required] | Z | Measure Design §7 simulation/digest/encoding/log/DB/actor and snapshot costs. **Done When:** §5.6 reference-hardware budgets/critical-write measurements pass; **C reviews Z output before soak**. |
| 6.3 Eight-client packaged casting soak [required] | C | Run eight packaged clients casting/long effects/PvP; **10,000 ticks is a test-design choice**. **Done When:** Retained participation/replay/memory-trend/panic/latency evidence meets §5.6; root holds Unreal lock. |
| 6.4 Bounded metrics, catalogue and live observability validation [required] | C | Instrument bounded metrics and extend **infra/scripts/verify-observability.sh** (§5.7); D dashboards. **Done When:** Controlled counter/gauge/cardinality checks pass with existing trace/log/scrape checks preserved; no skill ids. |
| 6.5 Predicate mapping, commands, fixtures, runners and traceability [required] | C | **First harness task: predicate → authoritative message mapping** (§5.3), then DSL/roles/fixtures/run-sim*.sh/run-sim-ci.py and title-cell tag parser/Phase2 heading fix. **Done When:** Reviewed foundation then §5.3/§5.6 live evidence pass; completion only in accepted plan rows, no tag literal in prose. |
| 6.6 Architecture docs, sequence diagrams and dashboards [required] | D | Document implemented architecture/diagrams/runbooks/dashboard and evidence. **Done When:** Codex review approves D output and linked phase outcome; proposals never described as complete. |
| 6.7 Simulation scenario(s) [client-visible] [required] | C | Run every §2.8 file/role/variant with §5.3 fixtures and §5.6 evidence. **Done When:** Nonvacuous assertions/record/replay/coverage3/contract/traceability and UI subscopes pass; Z reviews C harness/client, C reviews Z replay/benchmarks. |

## 5. Required acceptance evidence and fixtures

**Planned, not executed.** Use the existing module/test conventions. The new filenames below are proposals. §2.8 is the only scenario matrix. Actor/adapter tests cover the precise races/crashes that bots cannot force. A final digest alone cannot replace record-byte evidence.

### 5.1 Data, formula, cast/effect and schedule evidence

Implement **Design §8.1** and the enumerated ledger of §2.7. E1 supplies independent expectations. Z integrates the Rust/domain tests (proposed formula_oracle.rs, skill_classifier.rs, tick_ordering.rs and state_skill/effects/pvp tests):

- Classifier every rank/parameter/stat/priority/filter/condition/section/item/reference, unknown/transitive blockers and manifest drift. Runtime Squire's Sword guard, catalogue status/blockers and reported counts; no forced count.
- Oracle revisions/literal/input/vector hashes, exact intermediates, class/level/modifier orders, signed/Func/getter casts/caps, perfect/nonperfect sqrt/near-boundaries, all attribute coefficients, magic crit/both failure rolls/≥10 gap, binary32 conversion, exact karma compound truncation, actual-heal hate/zero/multiple ledgers, resurrection quirk/refund rounding/XP-floor loss. Precision unproved → registry rejection.
- C5 boundary/default regression vectors: abnormal-success default 10/90 and explicit per-skill min/maxChance overrides with immunity/no-draw exceptions; getter clamps below/at/above MaxMAtkSpeed1999/MaxPAtkSpeed1500/MaxEvasion250 loaded from tables/formulas.toml; missing magicLvl0, castRange/effectRange−1 source-specific unlimited/none semantics and abnormalLvl0 preserved through generator/oracle/runtime.
- RNG full Design §6 callsite/target/handler/bound/value/comparison/cursor ledger, including no-draw exits, continuous/duration/instant/SELF, zero-evasion/vengeance/reflection, attack-break pre/post-resource and out-of-window draw, stun/death order. Invalid/canceled adds no handler draws, prior selection draws retained.
- Cast authorization/generation/rank/captured target, staged Cancel/Move/Cast order, every rejection/cancel, MP split/start reuse/HP>cost, all timing vectors/independent cutoff t−1/t/t+1, after-launch cancellation and range/life/incarnation/protection/peace rechecks.
- Effects pools/shrink/stack/NONE/one-container/status overlap, dispel protected/newest/truncation, cold/cache/shared install order, live hooks versus rebuild. MaxHp100×1.5 heals50 once; priority-1 evasion before later funcs; immunity/basic-property/activate-rate exits, no universal min/max assertion.
- Periodic ticks1/3 cadence/binary32 amount/DIFF-PER/phase/expiry/suppressed advancement, HP1/CP nonlethal clamp, player/NPC direct/DoT/HP-consumption/zero/CP-only break/stun behavior. Natural player/NPC regen/fractions/dead/full boundaries separately.
- All Design §3 tick edges, shuffled UUID-versus-ordinal order, event+state comparison including kill SP/karma/level/tokens. Aggro current-target/EntityId ties, effectPoint once/AddHate/actual-heal fanout, ordered GetAgro→hate/hate→GetAgro/two/no-op/stale-dead requests and reverse-index restore.

Z/C review the oracle independently. Keep the old sheet/outcome expectations and the production SOURCES/integer-only checks. Do not use Rust-authored expected answers, and do not make tolerances wider.

### 5.2 LearnSkill and read endpoint matrix

**API guidelines §4 + Design §5.3** bind ListLearnableSkills/catalogue and LearnSkill. These are necessary: unit in-memory use cases, real TestApp wire suites (proposed grpc_learn_skill.rs) and DATABASE_URL-gated PostgreSQL adapters. C first publishes the RPC comments and the typed mapping: API §3 errors, documented state/prerequisite FAILED_PRECONDITION, oversize RESOURCE_EXHAUSTED and sanitized infrastructure INTERNAL. Trigger every reachable AppError and assert every documented tonic::Code.

| Case / layers | Required assertion |
|---|---|
| GetSkillCatalog success — unit/wire | Authenticated immutable catalogue: identity/version/runtime status/deferred blockers; empty request, no character ownership or progression dependency; no write/outbox. |
| ListLearnableSkills success — unit/wire | Owned character current state: every next-rank/cost/level/learned/affordable/trainer/eligibility/unmet field; fresh query after progression; no write/outbox. |
| Learn success — unit/wire/adapter | Exact rank/SP/passive/stats/tick/owner events equal frozen response and durable state/receipt/outbox; read-after-write/reconnect agree. |
| Auth/ownership — unit/wire | All three RPCs: missing token UNAUTHENTICATED before use case; identity from auth context. Character-scoped ListLearnableSkills/LearnSkill: other account PERMISSION_DENIED, missing character NOT_FOUND. Unknown skill INVALID_ARGUMENT per 241293f, as are malformed UUID/key/rank. GetSkillCatalog has no character ownership check. No key/write/event or receipt ownership bypass. |
| Availability/rollout — wire | GetSkillCatalog: documented UNAVAILABLE for unavailable catalogue, including authenticated rollout stub. ListLearnableSkills/LearnSkill: documented UNAVAILABLE for zone unavailability, including authenticated rollout refusals. Test all three RPCs’ unauthenticated/authenticated rollout refusals and all three WS intents’ single sequence-preserving NOT_YET_IMPLEMENTED rejection without actor mutation; refused learning leaves SP/skills/receipts unchanged (§2.1). |
| New work fencing — actor/wire | Current account/character/generation/online/lane required, stale work fails documented state code without mutation. Authenticated matching receipt replays before current eligibility guards. |
| Each eligibility/error — unit/wire | Dead, incoming-hate/swings/readiness/hostile-effects engagement, cast, trainer distance, lineage/level/SP/non-next-rank, learning item, unsupported capability separately; documented reason, no key/SP/state/event/outbox consumed. |
| Validation — unit/wire | Before transactional port, no writes/events; failed key usable by later valid request; malformed admission never runs tick. |
| Same key/body — all | Every frozen field (rank/spent/remaining/stats/original tick) exact, no second spend/write/event/outbox, including later learning/class/progression/disconnect. |
| Key conflict — all | Different character/skill/rank under same account/operation/key FAILED_PRECONDITION; account/operation isolation, versioned semantic fingerprint. |
| Queued duplicate keys — actor/concurrent wire/adapter | One draft/receipt/spend/outbox, all callers same frozen result, queued body conflict rejects; DB uniqueness alone insufficient. |
| Different keys/same rank — actor/concurrent wire/adapter | One learn/transfer/character/tick with lifecycle barriers: one spends, loser buys no next rank/consumes no successful key; disconnect/transfer cannot bypass. |
| New repository methods — adapter | Round-trip, typed bounds/FK/uniqueness, concurrent retries exactly once, injected child/receipt/outbox failure atomic; memory/PostgreSQL parity. |
| Durable failures — actor/wire | Paused log/checkpoint ack withholds success/output, immutable retry/fence and sanitized errors; §5.4 lost-ack/crash cases. |

The 24-hour receipts follow API §2, not the bounded transfer history. Historical replay tests operate within retention. Exact-rank guards prevent a key that was cleaned up from buying another rank. Every new method meets the API §4 round-trip/constraints/atomicity/concurrency rules.

### 5.3 First harness task: predicate → authoritative message mapping

**Publish/implement before dependent scenarios** (E6.5). A predicate is UNKNOWN until it receives valid evidence. Reset the connection/entity/generation scope, and fence the tick/incarnation/IDs, with the §2.1 baseline exceptions. Do not use client formulas, animation guesses or live zone mutations. Publish the actual DSL grammar/source/reset/error handling. Unsupported syntax fails.

| Command/predicate | Authoritative source |
|---|---|
| Catalogue identity/version/status/blockers | Authenticated immutable GetSkillCatalog RPC; no character scope or ownership dependency (E1.2). |
| nf.LearnableSkills; next-rank/cost/eligibility | ListLearnableSkills RPC scoped to owned character/current progression/fresh query; learned/affordable/trainer/unmet fields (E1.2/§5.2). |
| nf.Learn | Exact rank/optional explicit UUID key; frozen RPC plus SkillLearned/StatsChanged/SkillStateSync; retries same request. |
| nf.Cast/nf.CancelCast | Session WS seq; Ack/reject then Started/Result/Cancelled. Explicit corpse target, no client rank/power/cost/time. |
| nf.AcceptResurrect | Offer id/accept bool (false declines); owner offer+lifecycle/vitals. |
| skill_ready/reuse_remaining | SkillReuse/SkillStateSync ready_tick + received current tick, skill-id keyed. |
| cast_state | Started/Result/Cancelled + identity reset/baseline hydration, exact cast/incarnation; Ack alone insufficient. |
| buff_active/debuff_active/effect_count | EffectApplied/Removed/stable-ID hydration; no public system penalty or synthetic transition. |
| own_flag/karma/pk/pvp | Owner PvP/stats/sync; peers public PvP/spawn only. |
| own_sp; delta/event counts | SpGained/stats/sync/frozen response, initial received balance and tick/id scope. |
| own_mp_cost_matches/HP-MP-CP | Owner StatsChanged + independent fixture expectations/start-completion ticks; account for received regen, no client arithmetic. |
| stat_matches_oracle | effective_stats versus external hash-pinned vectors, never client formulas. |
| Public vitals/damage/heal/CP | VitalsChanged/results/respawn known-incarnation absolute values, no double subtraction/private total leak. |
| Offer/death/revival | Owner offer/sync/stats+lifecycle, exact expiry/id/incarnation/loss/refund or frozen fixture evidence. |
| NPC target/hate outcome | Received NPC target/move/attack or verified zone facts; numeric ledger/RNG remains engine oracle, no setter. |

Extend the contract audit, and keep the old rules/counters. Command acceptance is different from successful effects. The UNKNOWN mapping, reset and parser stub evidence must exist before live work. **run-sim.sh/run-sim-multi.sh/run-sim-ci.py** orchestrate scenarios/record/replay/trace/contract/JUnit. **run-tests.sh --require-live-api** runs the named Unreal HUD/keybind/reducer/live UI automation. Cooking/staging separately proves the assets, because .nfs cannot examine pixels.

Guarded Phase2-derived packs control a new PostgreSQL/JetStream. They migrate/seed **before API/admission**. They record normalized loads and the exact role/account/class/level/SP/rank/resources/position/data-binary-seeder-vector hashes. Private tokens/owned ports/cleanup/quarantine/old fixtures stay intact. The first cast rank is unlearned. Low-resource/dead/other skills/overlays are explicit. The peace crossing uses real movement.

The exact paired units cast-observer-a/b, pvp-flag-a/b and resurrect-a/b use different accounts and matching headers. An orphan or a mismatch fails. Include the competing-keltir role reversal/near-cap variants. Solo-named karma/aggro must have an explicit cooperating role manifest with unique artifact namespaces. This is a **new E6.5 orchestration deliverable**, because the runner of this time refuses duplicate processes with the same name. Publish it before use, or add named companions and reconcile §2.8 during implementation. Every role is a real client with a retained report. Timer/offer/replacement/seed variants are recorded. Precise crashes/races are linked actor/adapter evidence, never live SQL/test setters.

### 5.4 Persistence and recovery acceptance

All the fields/triggers/invariants of **Design §5.1/§5.2** are necessary, with memory/PostgreSQL parity, fake-ack actor/checkpoint evidence and real adapter transaction evidence:

1. Complete SP/learned/progression equality, PvP/death/fractions, effect/reuse/source/periodic payload/shared install order; full fingerprint/frozen-request parity, old bytes preserved, invalid bounds rejected.
2. **50/51 reuse,100/101 buff/debuff**, zero/positive flags/untimed penalty/actual loss; apply/refresh/remove/expiry atomic child replacement. Threshold crossing at real save; retries never recompute remainders/thresholds.
3. Every retained/flag/MP/CP/fraction/learning/progression/death/kill/resurrection trigger detected; one final fenced character write/tick includes receipts/ordered outbox. Only ordinary existing HP/XP cadence exception.
4. Independently pause log ack and post-log checkpoint ack: **no output/RPC success/next draft/incompatible snapshot**, exact immutable record/RNG/request retry, no rerun.
5. Crash before append, lost log ack, after log before DB, DB lost ack and before terminal reply: reconcile acknowledged work before admission; critical detector cannot discard it. Exactly one SP award/spend/rank/PK/PvP/karma/refund and receipt/outbox set; disconnect cannot cancel logged learning.
6. Stale revision/generation/permanent lane/fingerprint-key conflict fences without output or silent logged-work rollback. Every failed child/receipt/outbox step leaves no partial rows; typed adapter errors/sole relay parity.
7. Reconstruct before clamp/validation with shared order/source/ideal-ms phase, no hooks/heals/instant/end/RNG/cost replay. Rebase at actual deferred admission; snapshot keeps exact short deadlines/fractions.
8. Disconnect drops offer/short timers but retains death record; replacement preserves live cast/offer/ordinal/deadlines without DB reload. Dead admission no cast/regen credit; idle owner bootstrap complete; new offer after reconnect refunds recorded loss once across recovery.
9. Competing/repeated lethal/corpse hits credit living killer once; dead/departed source none. Human XP bonus does not affect SP; zero/i64::MAX reward/spend/actual saturation preserve active-slot equality. No replay/reconnect reaward.

C adapters come before E2.6. Z examines durability, and C examines the Z restore/replay. Show the withheld output, not only the correct rows at the end.

### 5.5 PvP, peace, death and resurrection evidence

The evidence includes the Design §3 truth table and the §8.1 full cross-product. This includes chaotic independent flags, the actor-only nonshortening deadline, victim unchanged and exact impact expiry. It also includes CP-only/HP/overkill/player-DoT versus NPC, protection, no healer-time-death decay and caps. The peace evidence covers inclusive/corner/both-endpoint crossing, AoE, pulse advancement and AI-leash. A pre-accept rejection is different from a later cancel/block. The death eligibility, the 20% seeded outcomes, the cap, no-draw, the XP floor, delevel, grants, cleanup, the private untimed penalty and the actual loss are recorded.

Every resurrection case is mandatory. Each case has paired §2.8 runs plus precise actor/adapter evidence:

- Explicit PC_BODY, private frozen offer/id/created/expiry/loss/percent, hidden source absent; no corpse SetTarget.
- Competing offers retain first; post-invalidation new id; stale/wrong-incarnation/duplicate consent no refund.
- Decline/real disconnect invalidate; live replacement preserves; source disconnect leaves safe attribution; offline stores no offer.
- Accept t−1/exact-expiry, expiry wins; both consent/town command orders no double revival; later-created offer cannot be preaccepted.
- Reconnect death record/no offer → new offer; later death invalidates previous incarnation/consent.
- D-R5′ HP 65% if lower/MP-CP preserved/corpse/no protection/incarnation/progression/final clamp, town unchanged. Oracle >20/equal20/base0/100/90cap quirk and Math.round refund/reduced loss.
- Same-ID unchanged AOI/maxHP change → respawn then final VitalsChanged exception/owner bootstrap, no duplicates/private leak.
- Crash/lost acknowledgements around consume/refund → one atomic XP/incarnation/loss/outbox effect (§5.4).

### 5.6 Replay, coverage, baselines and performance

Implement **Design §8.2 coverage schema3**, and keep all prior NPC/attack/class-transfer/token/contract fields. Zone facts count one time, and baselines count no transitions. Idle→Casting is an accepted start. Casting→Idle has a bounded reason histogram. Self-pairs are unreachable. Effect stable-ID Absent→Active/Active→Removed; same-ID Removed→Active is unreachable. Pulses and stack-reject are separate. Requests are counted separately from committed learns. Offers/CP/PvP/karma/regen/revival are counted. There is no Recovering/lethal TickHp. Merge equal schema/reachability, and calculate the totals again. Digest-only facts must have a verified replay first.

**New V5 .nfr two players/NPCs** covers every Design §8.2 branch plus the §2.1 corrections, with seed/rules/vector hashes. Do not change the old .nfr/V1–V4 digest/WS/record/compression and coverage/contract baselines. Tests cover legal/illegal pairs. Raw nested old-tag presence, empty fields and malformed state are rejected. A canonical-field mutation changes the V5 hash, and the derived indexes rebuild the same. An arbitrary mid-cast/pulse/fraction/pending-checkpoint suffix is equal to the uninterrupted **record bytes**. A new-event byte corruption fails, also with the same final digest.

Pre-run admission tests: command limit/B−1/B/B+1, worst disposition/recipient reserve,1024 loads/eight-session limits,FIFO suffix/oversize head/rebase/no-rerun retry/crash recovery. Post-run oversize fences; full-registry/full-effect snapshot measurement before freeze.

Each file/role/variant keeps manifests/logs/nonvacuous JUnit/export/byte-replay/trace/contract/coverage3/pipeline-verdict/cleanup. Missing evidence fails. Keep the error/quarantine gates, and do not invent a percentage. Traceability is only structural, and it refuses vacuous completion.

A20/Design §7: 1k NPC **mean<2ms**,1k players/observers **mean<10ms**, eight packaged clients casting/long effects **p99<20ms**. Measure the reference hardware/build/data and the simulation/digest/encoding/actor/log/DB costs separately. Before the soak, measure the synchronized pulses/regen, 0/8/full effects, hydration and critical writes. The duration of 10,000 ticks is a test-design choice. This section gives no measured result and no durability exemption.

### 5.7 Telemetry and real validator

C extends **infra/scripts/verify-observability.sh** and keeps the trace/log/Prometheus-OTLP/scrape checks. An independent reviewer examines the D catalogue/dashboard. The proposed cast metric `zone_skill_casts_total{outcome}` uses the bounded values start/completed/rejected/cancelled, and it does not count an Ack as an impact. Effect apply/remove/pulse/stack-reject use a bounded kind/reason. The active gauge increases in the active phase and goes back to the baseline, including 0. Learn outcome counters are separate from committed counters: a frozen retry makes no new commitment. PvP/SP are committed one time, with no observer/recovery duplication. The histogram gets samples. Log/checkpoint labels use a bounded phase/result/reason. **No skill/entity/account ids/UUIDs/arbitrary errors**, and enumerate the cardinality.

For a controlled fixture workload, the validator records pre/active/post scrapes and correlated committed facts. It also records the expected cast/effect/learn-new-retry/PvP/SP deltas and gauge values. A nonzero value after the soak is not sufficient. Hydration/retry/replay does not make the counters larger.

## 6. Dependencies, order and integration gates

Baseline: the Phase1a harness, the Phase2 progression/tokens, and the existing actor/checkpoint/outbox/codecs. The order is by artifact, with no calendar estimates:

1. **Foundation:** The C committed proto/error/receipt contract **241293f** on **codex/phase3-server-api** ([world.proto/game.proto and proto-review-1.md](#21-additive-wire-allocation-and-visibility)) goes to Z. The necessary handoffs must close first (§2.1). They are TargetChanged.tick + baseline/stale/subsequent-selection tests that keep the legacy encoding, and all three RPC/WS fail-closed endpoint tests. They also include catalogue UNAVAILABLE documentation that agrees with the handler. The D generator/manifest/oracle goes to Z/C, and all eleven rejected data-review findings block that handoff (§2.4). The C E6.5 predicate/DSL/parser/role foundation goes to Z. Use only parser/stub/provenance evidence. Do not wait for the whole E1/E6.5 live completion.
2. **Runtime:** Reviewed definitions/proto → Z E2.1–E2.5/E6.1 → C. Publish hooks, and measure the snapshot/pre-run admission before freeze. There is no dependency on live E2.6.
3. **Adapters/gameplay:** C E3/E4 uses the published hooks → Z. **E3.4 adapters/E3.3 receipts precede E2.6**, and C reviews them separately. Serialize shared edits through Z. There is no circular gate for an entire epic.
4. **Client/harness:** Proto makes the E5 skeleton possible. Live work must have the engine/API/adapters. First 3-cast-skill, then §2.8. C E5/harness/metrics → Z; D docs/dashboard → Codex; **Z E6.1/E6.2 → C**.
5. **Live:** These items complete the live stories: scenario wrappers plus separate live automation/cooking, V5/legacy replay/coverage3/contract/trace and endpoint/actor/adapter concurrency/crash evidence. Declarations alone never mark a story as done.
6. **Close:** C examines the Z DB/log/CPU measurements before the packaged soak/telemetry. Root serializes Unreal and integrates the reviewed outputs. The outcome cites the evidence.

000008 extends the existing SP schema. Fixtures migrate/seed before admission. Publish the commits/types/manifests/evidence at handoff. Failed privacy/compatibility/durability/roles block completion, without baseline changes. This revision runs no implementation gates.

## 7. Risks and mitigations

| Risk | Consequence | Mitigation/evidence |
|---|---|---|
| Legacy semantic/codec leakage | Changed old bytes or accepted nested Phase3 fields | Isolated policy/recursive presence validation and preserved fixtures (§2.2/§5.6). |
| Arithmetic/RNG divergence | Incorrect outcomes or replay | Independent rounding/caller ledger, binary32/sqrt bounds and cursor traces (§2.7/§5.1). |
| Durability/snapshot cost | Missed budgets or unappendable work | Coalesced writes, immutable retries, pre-run byte admission and measured registry/resource/checkpoint cost (§5.4/§5.6). |
| Baseline/privacy gaps | Dangling references or leaked resources | Identity-first hydration, owner/bootstrap/respawn exceptions and hidden/mutual-reference cases (§2.1). |
| Overclaimed coverage/dependency deadlock | Green headers without behavior or circular work | One §2.8 matrix, mapped predicates/separate automation and split foundation/runtime/admission/live gates (§5.3/§6). |

## 8. Out of scope

Apply the exclusions through the manifest/blockers and guards. Unsupported parts cannot run partially.

- Pets/servitors/summons: no runtime entities, feeding/collars/crystals/costs/XP sharing. Only reserve kinds and a neutral penalty hook. Summoner trees 14/28/41 are excluded. The owner approved the Phase4 item dependency.
- Item-consuming skills/arrows/soul or spirit ore/shots, inventory/equipment systems, weapon attributes, shields/blocking and high-grade branches. Supported conditions still apply the real Squire's Sword check. Unsupported weapon/slot prerequisites defer. No chaotic item drops.
- Operate/target/scope capabilities outside Design §1.1: T/CA/DA/absent A4, PARTY/PLEDGE/PARTY_PLEDGE/DEAD_PLEDGE, GROUND/FAN/SQUARE, SUMMON/NPC_BODY/DOOR_TREASURE and unsupported scope filters. Proximity never replaces party membership.
- Unknown parameter/stat/recursive condition/section/reference, Charges/energy/race/slot/item requirements, unsupported mastery/trigger behavior. Imported triggered definitions give closure/provenance, not runtime support for triggers.
- **Fear is excluded** because it must have flee pathing. It has no phase assignment. TargetMe and TickHpFatal stay named blockers. Unsupported summon/cubic/transformation/lethal/fatal/blow/drain/spoil/sweep/escape/teleport/hate-delete/randomize/target-cancel/crafting handlers do not run without a later scope/manifest approval.
- Sitting, Mother Tree/estate/siege regen bonuses, guards, extra world content/zone types/peace zones. Keep the existing NPC/leash behavior. The plan delivers one town square that the architect configures.
- Unpopulated trees, playable third transfers, Olympiad/Noblesse/live subclass-certification, party/clan/alliance/sieges and cap expansion. Keep the metadata hooks. The cap is 85, and playable transfers go through tier 2.
- Final combat animation/audio/art, prediction, replication/GAS, client-authoritative combat or UE server. Deliver thin-client placeholders/UI with cooking evidence.

## 9. Outcome and review disposition

**Final acceptance documentation (closed root receipts): Phase 3 combat and skills implementation is complete and verified on final main SHA `995ffa9`.** All eighteen initial plan review findings and eleven handoff blockers are resolved and closed. Independent reviews and five integration triage rounds closed them. The closure of all code reviews at integration r13 `dfd16cf` is logged in the preserved `architecture-notes.md`. The review receipts are `plan-review-1..3`, `design-review-1..2`, `data-review-1..4`, `engine-review-1..2`, `api-review-1`, `client-review-1..2`, `harness-review-1`, `integration-review-1..3` and `triage-review-1..7`.

| Finding | Section map | Resolution / evidence |
|---|---|---|
| 1 Pins | §2 | Resolved. L2J Game `abfde0490ac52a2106e884b7242c0276c44cdf76` and Datapack `3ca488dd2bd0bfaca43e378886a3c2e37968153a` pinned; data generator `check_all.py` PASS. |
| 2 Wire/privacy/types/hydration | §2.1; E2.4/E5.5 | Resolved. Additive wire allocations committed; StatsChanged owner-only; V5 two-phase identity and hydration ordering verified. |
| 3 SP/migration/backend | §2.5; E3.4/§5.4 | Resolved. Migration 000008 adds karma/PK/PvP/flag/death-penalty/recoverable-XP and regen-fraction columns plus `character_skill_reuse` and `character_effects`; Young Keltir awards 2 SP; SeaORM critical checkpoints verified. |
| 4 Capability/counts | §2.4; E1.2/§5.1 | Resolved. 180 executable / 198 deferred of 378 core catalogue (188 / 200 of 388 retained definitions); exact capability checks computed. |
| 5 Traceability/E6 ending | §4; E6.5/E6.7 | Resolved. `# covers:` traceability annotations in scenarios verified by simulation tooling; all scenarios pass. |
| 6 Behavioral coverage | §2.8 | Resolved. 59 logical units verified (59 PASS, 0 failures; `quarantine.json` empty). |
| 7 Harness/coverage/V5/baselines | §5.3/§5.6; E6.1/E6.5 | Resolved. Snapshot 9 / Record 7 / Binary V5 verified; replay check byte-exact on 4 fixtures (2 legacy, 2 V5). |
| 8 Endpoint matrix | §2.5/§5.2 | Resolved. GameService gRPC RPCs (ListLearnableSkills, LearnSkill) and WebSocket intents verified against error and auth matrix. |
| 9 Persistence/recovery/boundaries | §2.5/§5.4 | Resolved. Critical mutations follow draft -> durable log ack -> revision-fenced checkpoint -> output/RPC success; fail-closed recovery tested. |
| 10 Dependencies/review | §3/§6; E2.6/E6.1/E6.2 | Resolved. Independent review separation strictly maintained across Codex and agy workers. |
| 11 Aggro | §2.6; E4.1/§5.1/§2.8 | Resolved. HateLedger extended with skill effectPoint, heal hate, and Aggression hate; verified in `3-aggro-skill-hate`. |
| 12 Peace/corpse | §2.6; E5.1/§5.5 | Resolved. Peace square (10-tile half-width around 126,126) rejects hostile intents with PEACE_ZONE; corpse targeting for PC_BODY verified. |
| 13 Formula/rounding/branches | §2.7/§5.1 | Resolved. Independent Fraction-based Python oracle (`packages/data/scripts/skill_oracle.py`) provides the formula vectors; Q-scaled fixed point with checked arithmetic verified. |
| 14 Resurrection lifecycle | §2.6/§5.5 | Resolved. 600-tick ResurrectOffer, corpse revive at 65% HP, preserved MP/CP, and one-time recoverable XP refund verified. |
| 15 PvP timing/actor refresh | §2.6/§2.8/§5.5 | Resolved. 1200-tick (vs non-flagged target) / 600-tick (vs flagged target) PvP flags, PK/PvP counters, karma decay on monster XP, and CP absorption verified. |
| 16 Telemetry/validator | E6.4/§5.7 | Resolved. OpenTelemetry metrics catalogue, Prometheus counters, and Grafana dashboard panels verified. |
| 17 Runtime provenance/exclusions | §2.4/E3.1; §1/§2.6/§8 | Resolved. Fear, pets/servitors, enchant, shields, and party scopes explicitly excluded and guarded. |
| 18 Duplication/status/syntax | §2/§4/§9; E6.3 | Resolved. Plan and design errata aligned to binding architecture decisions record. |

### Outcome reconciliation checklist

- [x] **Catalogue provenance & capability counts:** Pinned L2J Datapack `3ca488dd2bd0bfaca43e378886a3c2e37968153a` and Game `abfde0490ac52a2106e884b7242c0276c44cdf76` are verified. The catalogue gives exactly 180 executable / 198 deferred of 378 core skills (188 executable / 200 deferred across all 388 retained skills; 286 sample-only excluded). Deferred skills apply named blockers. `packages/data/scripts/check_all.py` PASS.
- [x] **Wire allocation, versioning & legacy preservation:** Additive wire tags on ClientMessage (16–18), WorldEvent (14–25), StatsChanged (15–23), EntitySpawn (22–28), and AttackResult.cp_damage (8) are verified. Snapshot 9 / Record 7 / Binary V5 are verified. Snapshot 8 / Record 6 / Binary V4 and older replay under legacy rules. The replay check is byte-exact on 4 fixtures: legacy `two-players-v4.nfr` (467 ticks) and `two-players-fight-v2.nfr` (908 ticks), V5 `phase3-casts-v5.nfr` (131 ticks) and `phase3-gameplay-v5.nfr` (769 ticks) (`r21-03-replay-check.log`).
- [x] **Casting lifecycle & effect scheduling:** mpConsume1 at start, mpConsume2 at impact. The interrupt cutoff is `floor(trunc(animation_ms)/100) - 2` ticks. MoveTo cancels the cast. The stacking rules apply 20 buff / +4 extra / 12 song-dance / 8 debuff slots. The death and class transfer removal policies are verified.
- [x] **Regeneration:** The HP/MP/CP regeneration formulas are verified every 30 ticks (3.0 s), with standing (1.1) and moving (0.7) multipliers. CP is player-only. Fractional Q remainders are recorded and persisted.
- [x] **PvP, karma, CP absorption & peace zone:** These items are verified: PvP flags (1,200 ticks after a hostile act against a non-flagged player / 600 ticks against a flagged player), PK/PvP counters, karma calculation and monster-XP decay. CP absorbs player damage before HP. NPC damage bypasses CP. The peace zone square (half-width 10 tiles around 126,126) rejects hostile intents with `PEACE_ZONE`, and it stops NPC auto-aggro.
- [x] **Death penalty & resurrection:** There is a 20% death penalty chance, up to level 5. Resurrection 1016 on `PC_BODY` emits a 600-tick `ResurrectOffer`. It revives at the corpse with 65% HP, preserved MP/CP, and a one-time recoverable XP refund.
- [x] **SP & skill learning:** Young Keltir gives 2 SP on the kill blow. LearnSkill runs as a revision-fenced critical actor mutation, with a frozen idempotency receipt, at the Class Master (126,128) radius 3.
- [x] **Documented deviations:** These deviations are documented:
  - C17 retained-selection across AOI departure/re-entry
  - C18 `players_attacking_me` received-evidence predicate semantics
  - R4 offline buff/reuse timer pause
  - 600-tick resurrect offer lifetime
  - peace square geometry R7
  - maximum level 5 death penalty
  - Phase 1 town respawn vitals (65% HP, 0 MP).
- [x] **Server gates & static analysis:** The Rust gates passed at r11 (1000 passed / 0 failed / 3 ignored, `r11-00-rust.log`) and r12 (1001 passed / 0 failed / 3 ignored, `r12-00-rust.log`). Clean fmt, clippy, entities, replay, data check, and Buf breaking.
- [x] **Unreal client builds & automation:** The Editor and Game builds exit 0 (`r13-01-ue-builds.log`). Offline automation: 103 passed / 0 failed (`r13-02-ue-offline.log`). Required-live automation: 103 passed / 0 failed / 0 skipped (`r13-03-ue-required-live.log`).
- [x] **Full ordinary scenario suite:** 59 logical units are verified (59 PASS, 0 FAIL, exit 0; `r13-04-sim-ci.log`, `r13-sim-ci-final/`).
- [x] **Game package staging:** The BuildCookRun stage completed in 84.99 s, exit 0, Linux Development (`r13-soak-attempt1/stage.log`).
- [x] **Final Rust gate on merged main 995ffa9:** fmt/clippy exit 0 (`r22-00-summary.log`, run on code-identical `7e1740a`). The tests had 1018 passed / 0 failed / 7 ignored across the 62 test-harness targets of 64 run (`r22-05c-tests.log`). The 2 criterion bench targets rejected the `--test-threads` argument. The debug-profile `zone_step` budget panic in `r22-05d-benches-testmode.log` is the known artifact of the unoptimized profile. The release zone_step benchmark passed: 1000 NPCs mean 146.7 µs p99 173.3 µs < 2 ms budget; 1000 players all observers mean 9.54 ms p99 19.42 ms < 10 ms mean budget (`r22-05e-zone-step-release.log`).
- [x] **Packaged combat soak (attempt 12):** 8 clients × 17 iterations, 1,200 s requested, 12,246 ticks observed, combat tick p99 10.776 ms < 20 ms budget. The Phase 3 acceptance evidence passed, there were no client errors, and the API and cleanup exit 0 (`r22-soak/report.json`, `r22-soak/summary.txt`). The replay agreed byte-exact across 12,246 ticks in 796.9 ms (`r22-soak/replay-check.log`). The stack storage was isolated on tmpfs (`SOAK_TMPFS_VOLUMES=8g`, `r22-soak/storage.txt`). The reason is that attempts 9–11 failed on host fsync stalls of the shared encrypted root filesystem (perf round 4; architecture decisions §10 P6). The attempt-12 fsync probe did not start, thus no concurrent host-fsync record exists.
- [x] **Root main integration:** Merged and pushed to main / origin/main (`995ffa9e37b065f540c12e6012a94509658f8035`). After the merge, the root Editor and Game builds exit 0 at 04:35:42 / 04:37:06 UTC (`root-build-995ffa9.log`). The root offline automation was 102/1 without an API (`CatalogueRpcFailures` must have an API that it can reach; `root-smoke-995ffa9.log`). The second run with the API up was 103/103, exit 0 (`root-smoke2-995ffa9.log`, `root-smoke2-995ffa9.summary`).
- [ ] **External operational follow-ups:** The external self-hosted Linux CI runner is still not registered. The two-week reliability programme is pending as an independent operational follow-up (it is not a local acceptance gate).

The outcome document [`phase-3-combat-and-skills-outcome.md`](phase-3-combat-and-skills-outcome.md) gives the detailed verification metrics, evidence receipts and closed gates.
