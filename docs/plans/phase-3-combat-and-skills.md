# Phase 3 Plan: Combat and Skills

**Status:** Approved with bounded corrections applied (revision 5); integration underway

Architect: Claude. Original author: agy gemini-3.8-flash-high (D); bounded revision: Codex gpt-6.1-sol (C). Independent plan review: Codex gpt-6-astra (Z). This is an implementation plan, not an implementation or execution attestation.

## 1. Goal and locked decisions

Deliver server-authoritative skill combat for 39 populated class trees: SP/learning, resource/reuse casting, effects/status/regen, skill/heal aggro, death penalty/resurrection, player CP/flags/karma and one peace zone. Unreal submits intent and renders authoritative messages; historic epochs retain their rules and bytes.

**§2 is this plan's canonical contract section.** It points to the binding design instead of defining competing mechanics in stories. Latest architect corrections supersede older wording. Scope/authority:

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

No day estimates; soak duration is a test-design choice. Measurements pending.

## 2. Source authority and canonical contracts

**Design** = [phase-3-zone-design.md](phase-3-zone-design.md). **Decisions** = [phase-3-architecture-decisions.md](phase-3-architecture-decisions.md), all sections including newest §9 C1–C5/C2′. Design now carries a binding **ARCHITECT ERRATA** header: architecture-decisions.md §9 C5 supersedes its obsolete 1/99 text and other conflicting defaults. **Design §3 schedule, §4 version/codec and §5 persistence/LearnSkill remain authoritative; the committed schema in §2.1 is authoritative for concrete wire field allocations.** Tables here are allocation/version summaries and one scenario matrix; story/test acceptance references them without alternative contracts.

Inputs: preserved evidence: side-session-notes/phase3-codex-2026-10-10/data-review-1.md, preserved evidence: side-session-notes/phase3-codex-2026-10-10/plan-review-2.md, preserved evidence: side-session-notes/phase3-codex-2026-10-10/design-review-2.md, preserved evidence: side-session-notes/phase3-codex-2026-10-10/skill-inventory/report.md. [Planning template](../planning/README.md), [API guidelines](../engineering/api-guidelines.md) and engineering standards govern implementation. [Combat research](../planning/03-combat-and-skills.md) is context, not runtime authority.

Pinned revisions match [SOURCES.md](../../packages/data/SOURCES.md):

- Game **abfde0490ac52a2106e884b7242c0276c44cdf76** — formulas/functions/caller behavior/config.
- Datapack **3ca488dd2bd0bfaca43e378886a3c2e37968153a** — skill XML/handlers/templates/karma rows.

Generator/oracle provenance must verify full hashes, source paths/literal lines and input/output/vector hashes. Cached clones are provenance, not runtime dependencies. Formula citations come from Design §6; count reports do not override its source ledger.

**Canonical C5 (architecture-decisions.md §9 C5; Design ARCHITECT ERRATA):** Abnormal-success defaults are **10/90**, overridden by explicit per-skill min/maxChance; retain immunity/no-draw exceptions. **MaxMAtkSpeed=1999, MaxPAtkSpeed=1500, MaxEvasion=250**. Missing attributes retain **magicLvl=0, castRange/effectRange=-1** (unlimited/none according to source-specific usage), and **abnormalLvl=0**. Generator and independent oracle preserve these values and semantics; the engine loads clamps from **tables/formulas.toml**, never hard-codes them. §5.1 requires boundary/default regression vectors.

### 2.1 Additive wire allocation and visibility

Implement **Design §2 + C2/C2′/C4**. Committed contract **241293f** on **codex/phase3-server-api**, [world.proto](../../packages/proto/nightfall/v1/world.proto) and [game.proto](../../packages/proto/nightfall/v1/game.proto), is authoritative for concrete field allocations. Preserved evidence: side-session-notes/phase3-codex-2026-10-10/proto-review-1.md **APPROVE WITH REQUIRED CHANGES** retains three handoff conditions: append V5 **TargetChanged.tick**, document its relationship to baseline_tick and test baseline acceptance/stale-delta rejection/subsequent selection with unchanged zero/default legacy encoding; test all three RPCs for unauthenticated **UNAUTHENTICATED** and authenticated rollout **UNAVAILABLE**, and all three WS intents for exactly one sequence-preserving **NOT_YET_IMPLEMENTED** rejection without actor mutation (refused learning leaves SP/skills/receipts unchanged); document catalogue **UNAVAILABLE** consistently with the handler, including catalogue unavailability and the rollout stub. Preserve old envelopes/tags/bytes.

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

**All tick/deadline/remainder fields are uint64/Tick(u64)** with checked arithmetic. StatsChanged is WorldEvent **7**, AttackStarted **11**; StatsChanged **field11** is SP. Public PvP only through PvpStateChanged/EntitySpawn; MP/CP totals, SP/XP, learning/offers remain private.

V5 AttackResult.damage is HP removed; SkillCastResult distinguishes rolled/CP/HP amounts. **Legacy rolled-overkill output bytes stay unchanged** (Design §3). VitalsChanged covers HP **or effective maxHP**, including max-only changes; clients set absolutes, never subtract twice.

Binding hydration corrections:

- All V5 identities first, active_effects empty, pending_cast/pending_swing absent. Then baseline EffectApplied/**SkillCastStarted** hydration with stable IDs/incarnations/deadlines, plus **baseline AttackStarted for existing swings (C2)**. No separate PendingCast type. No animations/transition counts for baselines.
- Filter every nested endpoint recursively against known visible incarnation after the batch; hidden entity/incarnation absent together. Preserve visible subject summary without leaking full source attribution.
- Baseline exceptions include owner **StatsChanged/TargetChanged/SkillStateSync**, public **PvpStateChanged** bootstrap, effect/cast/swing hydration. Construct final observe values and accept at baseline tick, including idle reconnect; ordinary mutation suppression remains.
- Same-ID resurrection in existing AOI emits final **VitalsChanged after EntityRespawned** with baseline exception (C2′). Suppress vitals only if that observer received final HP/maxHP pair.

E2.4/E5.5 evidence covers mutual cast/swing newcomers, hidden endpoints, empty reset, idle replacement/reconnect and resurrection maxHP change.

### 2.2 Version matrix and legacy isolation

Use **Design §4.1–§4.5** without widening current validation. This is the sole compatibility table.

| Snapshot | Digest | Classes | Record | Policy |
|---|---|---|---|---|
| 4,5 | JsonV1 | false | 3 | Historic Phase1 |
| 6..8 | JsonV1/BinaryV2 | false | 3 JSON/4 V2 | Existing no-class matrix |
| 7 | BinaryV3 | true | 5 | Phase2 before tokens |
| 8 | BinaryV4 | true | 6 | Phase2 tokens |
| 9 | BinaryV5 | true + valid Phase3 block/rules/hash | 7 | Phase3 only |

Reject illegal snapshot/digest/record pairs and **recursive Phase3 presence even empty/default** under legacy nested tags/loads/projections/receipts/outbox. Validate raw protobuf/JSON before defaults discard presence; encoder and digest-only reproduced outputs share context. Legacy policy gains no new combat/regen/SP/PvP/peace/draw/baseline behavior. Preserve old serializer/digest bytes and retain record3–6 decoding; .nfr container1 unchanged.

Design §4.2/§4.3 governs complete embedded rules/canonical state/hash/pending lanes and rebuilt indexes. Snapshot restore retains short/exact timers, unlike logout admission. NFSNAP9 envelope is magic + LE expanded length + zlib, no separate envelope hash. **Before freeze**, measure full registry +1,000 full-effect entities against ~1MiB encoded/32MiB decoded, without truncation or external replay fetch. Larger limit/bundle needs architect disposition after measurement.

Design §4.5's **pre-run** byte admission bounds complete normalized loads/dispositions/recipients, FIFO prefix/suffix, oversize terminal failure, immutable gate retry and lost-ack recovery. Output digest fallback alone cannot bound retained commands (§5.6).

### 2.3 Engine state and scheduling

**Design §1** defines module/state/identity/handler/Func/fraction contracts; **§3 is the complete V5 schedule/edge table**, not copied here. Preserve legacy run_tick; eight-command budget is per session in actor. Verify staged command order, separate launch/impact, cast ordinal versus auto-attacker EntityId order, expiry/pulse/completion/impact edges, immediate death, ordered AI requests and coalesced output.

Design §1.2/§6.1 binds animation/continuous head start/launch/impact and **independent animation-derived interrupt deadline**; completion-minus-two is withdrawn. coolTime/hitCancelTime remain imported unused, Idle/Casting only. Design §1.3/§1.4 binds slots/stacking/status overlaps, player/NPC damage-break, periodic ideal-ms DIFF/PER and rebuild without one-time heals/RNG/cost. TickHp is nonlethal (HP1/CP); TickHpFatal deferred. Register every new zone file in SOURCES; production remains ordered/integer-only, no wall clock.

### 2.4 Generated data and capability manifest

**R13 + Design §1.1/§6**: generate every rank of 378 tree catalogue ids +5076/transitive closure, with source hashes/exact literals/computed status and blockers, reproduced by future gen_skills.py --check. Never hand-edit status.

E1.2's machine-readable **capability manifest** validates every effect **parameter**, individual stat/operation/priority, target/operate type, scope/**filter**, recursive and/or/not leaf, effect/self/start/end/channeling/pvp/pve section, item prerequisite and referenced dependency. Unknown defers whole skill with named blockers; handler-name/mul/add/sub recognition alone is insufficient. Runtime rejects manifest drift and enforces **using kind against actual equipped Squire's Sword** and canResurrect against state. Design §1.1's admitted set/types is authoritative; unsupported racial/mastery/trigger behavior cannot become partial execution. **C1: affectLimit width0 →0/no draw/unlimited**, bounded by population; positive width uses pinned sampling.

Inventory **241/379 is historical only, never a gate**. Data-branch generator report: **211 executable / 167 deferred of 378**, **389 including extras** (220/169 overall: 5076, nine referenced ids, sample-only 286) are **reported classifications from the rejected data revision**, pending fixes, regeneration and independent re-review under preserved evidence: side-session-notes/phase3-codex-2026-10-10/data-review-1.md. Passing generator **--check does not establish TOML validity, capability correctness or oracle correctness**. All eleven findings remain handoff blockers until reviewed closure: invalid generated TOML; lost stat conditions; zeroed per-level ranges; wrong short continuous-cast expectations; TickHp rounding; missing parameter/operation/filter admission; lost locked reuse semantics; wrong source defaults; false universal karma-equivalence claim; absent required adaptation coverage; wrong provenance symbols/lines. Regenerate exact totals/blocker histogram against reviewed capabilities and explain changes; counts are neither targets nor local validation.

Runtime [keltir.toml](../../packages/data/npcs/keltir.toml) **id keltir** gains **sp_reward=2**, provenance pinned Young Keltir **20545**; preserve tunable XP28/stats. Design §3/R6 governs living lethal-killer credit, actual saturation delta, no sharing/Human XP bonus on SP.

### 2.5 Persistence, recovery, admission and learning

**Design §5.1 is full persistence acceptance; §5.2 admission; §5.3 LearnSkill.** Extend memory/PostgreSQL DTOs/adapters/fingerprints/pending snapshots/recovery/outbox together; preserve old fingerprints. No new backend.

[000007](../../apps/api/src/infrastructure/postgres/migrations/m20261009_000007_character_classes.sql) already owns characters.sp/cp, character_class_slots.sp and learned rows. **000008 adds no SP column**; retain ClassState/active-slot projection equality, no duplicate skill ledger. New Design §5.1 scalar defaults zero, karma/counters u32-bounded bigint, penalty0..5, fractions0..Q−1, checked nonnegative timer/XP/death bounds; optional death fields form consistent record. New reuse/effect child rows have deterministic keys/FK cascade and **atomic replacement in fenced final state+receipt+ordered outbox transaction**.

Design §5.1 critical triggers include retained effect/reuse apply/refresh/remove/expiry, flag refresh/expiry, learning/SP/PvP/death/resurrection/class/passives and **new MP/CP/fraction mutations**. Freeze request/fingerprint/revision/remainders; one final character write/tick. Deadline countdown alone is not a per-tick write. Only old ordinary HP/XP cadence exception and existing outbox relay remain.

Retention is strict **50/51 reuse,100/101 effects**, flags without threshold, penalty untimed; preserve source/periodic ideal-ms phase/fractions/shared passive-effect relative order. Real logout drops short entries/offers but retains death loss; snapshot keeps exact timers; live replacement preserves cast/offer/deadlines without reload. Rebase at actual admission, no wall-time catch-up or hooks/RNG/cost replay.

Learning authenticates ownership/current generation for new work, with receipt lookup before eligibility and exact-rank/fingerprint/frozen result under API §2/Design §5.3. Matching historical success survives later progression/class/disconnect. Failure consumes no key/SP. Duplicate queued keys settle once; different-key same-rank loser cannot buy next rank. Log ack → fenced checkpoint ack precedes output/success. **API §4 endpoint matrix (§5.2)** and full parity/ack/crash/fence acceptance (§5.4) are mandatory.

### 2.6 Aggro, PvP, peace, death and resurrection

**Design §3** binds damage/credit/aggro/PvP truth table/death; **§1.2** geometry/offers; **§6** math. most_hated stays **current target when tied, else lowest EntityId**. Skill `\|effectPoint\|` once per applicable skill/target, actual-heal ledger fanout, AddHate changes hate; **GetAgro enqueues forced NPC intention at AI with no hate increment**, not TargetMe/taunt lock.

PvP uses pre-impact victim flag/karma and pre-increment killer PK, D-R4 counter precedence. **1200/600 ticks =120s/60s**; actor-only **max(old_deadline,now+duration)**, including neutral attacker hitting flagged victim. Hit never flags/refreshes victim. No healer/time/death karma decay; player DoT routes CP first. Truth table stays in Design §3.

Peace inclusive square center126,126/half-width10 tiles=10,000 milli-tiles/contains Class Master is **architect R7**, not owner/source. Both endpoints at acceptance/impact/AoE/pulse: **IntentRejected only pre-accept**, later cast/swing cancels or recipient/pulse blocks; suppressed pulses advance. NPC acquisition/leash/protection follow Design.

Skill resurrection 1016 uses explicit **PC_BODY CastSkill target**, separate from client live-player SetTarget. Verified D-R5′: HP raised to 65% effective max if lower with Q restoration, **MP/CP unchanged**, corpse/no town protection/new incarnation then XP/level/passive/final vitals; town respawn unchanged. Offer **expires_at = created_at + 600** checked ticks; first valid retained. Decline/expiry/town/accept/real disconnect invalidates, replacement preserves; accept-at-expiry loses. Reconnect retains actual death-loss record, no offer; new offer may use record. Pinned refund rounding consumes loss once crash-safely (§5.5). Penalty5076 is private/untimed/cap5 with Design eligibility/no-draw rules.

### 2.7 Enumerated formula/rounding ledger

**Design §6/§6.1/§6.2** binds independent vectors for each family (no unsupported fixed formula-count claim):

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

Fraction/Decimal oracle reads pinned literals, not Rust/generated Q expectations; manifest records revisions/path/line/input/roll/intermediate/rounding/branch/output/hash. Sampling agreement is evidence, not universal proof. Design §6.2 requires checked sqrt bounds/refinement or proven bounded equivalent at straddling boundaries; unresolved precision rejects registry. Production remains integer-only; §5.1 tests, scenarios compare received outcomes to external oracle constants.

### 2.8 Canonical scenario matrix

One canonical story → file → authoritative assertion matrix. Files are proposed under apps/client-unreal/Scenarios; first nonempty header is `# covers: ` plus exact middle-column payload. Every epic's simulation story refers here. **First ship 3-cast-skill: learn → cast → MP/reuse**. Regen covers only regen stories; E1.2 status and real damage/debuff have dedicated assertions. §5.3/§5.6 supply role/record/replay evidence; E5 pixels/assets require separate automation/cooking, with explicit rendering subscopes.

| Scenario file | Covered stories / exact covers payload | Authoritative assertions and fixture purpose |
|---|---|---|
| `3-cast-skill.nfs` | `3 E1.4, 3 E1.6, 3 E2.1, 3 E2.2, 3 E2.7, 3 E3.2, 3 E3.3, 3 E5.1, 3 E5.2, 3 E5.5, 3 E5.6, 3 E6.7` | Test rank initially unlearned: query/learn at trainer, receive receipt/SP/rank, leave peace, cast NPC, assert MP1/start reuse then MP2/result/ready boundary. Magic outcome/HP/None attribute matches oracle; full coefficient boundaries are oracle-only. UI via separate automation. |
| `3-cast-rejected.nfs` | `3 E1.2, 3 E1.6, 3 E2.1, 3 E2.7, 3 E6.7` | Catalogue deferred status/blocker and runtime refusal; unknown/unlearned/reuse/MP/status/range/real-weapon guards. Accepted completion failure cancels, never second disposition; cancel/move/damage retains MP1/reuse. |
| `3-learn-skill.nfs` | `3 E1.2, 3 E1.6, 3 E3.1, 3 E3.2, 3 E3.3, 3 E3.5, 3 E5.1, 3 E5.4, 3 E5.5, 3 E5.6, 3 E6.7` | Executable/deferred/blockers/costs match manifest. Runtime keltir earns one SP 2, exact-rank learn debits, same-key retry returns all frozen fields with no second event, including progression/reconnect. Competing/cap variants in PvP roles. |
| `3-learn-rejected.nfs` | `3 E1.2, 3 E1.6, 3 E3.2, 3 E3.3, 3 E3.5, 3 E6.7` | Status/unlearnable reasons and SP/level/lineage/rank/distance/item/engaged/dead/casting/generation/key failures; state unchanged/no event/key. Failed key later usable; ownership/concurrency separately §5.2. |
| `3-buff-expires.nfs` | `3 E2.2, 3 E2.3, 3 E2.7, 3 E5.3, 3 E5.6, 3 E6.7` | Stable IDs/deadlines, equal/higher replace/lower silent rejection, oldest overflow, removal-before-apply, expiry restores stats/maxHP/no phantom pulse. Include capped heal/dispel outcomes; HUD separate. |
| `3-debuff-status.nfs` | `3 E1.5, 3 E1.6, 3 E2.2, 3 E2.3, 3 E2.7, 3 E6.7` | Actual physical damage/debuff status/duration oracle match; seeded resist/immune plus engine traces. Stun-before-swing/root/overlap, player/NPC direct/DoT break and HP1/CP nonlethal pulses. No probability inference from few casts. |
| `3-regen.nfs` | `3 E1.3, 3 E1.6, 3 E6.7` | Natural regen only: 30-tick player HP/MP/CP standing/moving and NPC HP/MP oracle deltas, dead/full no-growth, multi-pulse fractions via whole outputs. Low-resource pre-admission seeds; no damage/debuff coverage. |
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

Follow clean architecture and Design §1.1's module/hook table: domain immutable drafts/facts, application durable coordination/ports, infrastructure memory/PostgreSQL/JetStream, interface reviewed DTOs; Unreal renders only.

- **Z: Codex gpt-6-astra/high** owns E2/E6.1/E6.2 and shared zone state.rs/state_combat.rs/command.rs/state_digest.rs, replay record/codec and snapshot codec. Publish typed damage/hostility/death/learn/stat/persistence hooks before C module integration; serialize shared-file edits.
- **C: Codex gpt-6.1-sol/high** owns first additive proto/receipt/RPC contract, E3 schema/adapters/state_learn, E4 isolated state_pvp/state_death against Z hooks, E5/E6 harness/soak/metrics. **D: agy gemini-3.8-flash-high** owns E1 data/oracle/formula handoff and E6.6 docs/dashboard; current bounded revision is C-authored. D formula patches to shared modules go through Z.
- Independent handoffs: C proto/E3/E4/E5/harness → Z; D data/oracle → Z/C; Z E2/runtime/admission and **Z-authored E6.1/E6.2 → C**; D docs/dashboard → Codex; this plan → Z. No sole self-review. Root integrates/merges/pushes later and serializes all Unreal activity.

Runtime restore review precedes separate admission integration: E3 consumes published types/hooks, **E2.6 consumes E3.4 adapters/E3.3 receipts**. Foundation/live gates are separate (§6), no circular dependency. Proto has one editor at a time.

## 4. Epics, stories, models and Done When

Status: **implementation is underway in sibling worktrees; no story is marked complete; this plan revision performed no implementation or execution gates; phase acceptance remains pending**. Tags belong only in client-observable story titles. Future completion uses `| ✅ N.M Title … |` in this plan's first cell after evidence/review. Current parser reads tags from whole line; prose carries none. E6.5 title-cell hardening is future work, not needed to parse this plan.

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
| 3.4 Migration 000008, checkpoint adapters and recovery [required] | C | Extend 000007 via 000008/ports/adapters/recovery under Design §5.1 (§2.5). **Done When:** No duplicate SP; bounded defaulted state/atomic child+receipt+outbox replacement and §5.4 parity/ack/crash/fence pass; publish adapters to E2.6. |
| 3.5 Simulation scenario(s) [client-visible] [required] | C | Author §2.8 learn-skill/rejected and SP assertions in pvp-flag-a/b. **Done When:** Catalogue/frozen retry/failure/competing-SP runs pass with §5.2/§5.4 race/crash evidence; no solo-kill overclaim. |

### Epic 4: Aggro, PvP, peace, death and resurrection (C; Z review)

| Story | Model | Work / Done When |
|---|---|---|
| 4.1 HateLedger integration and forced NPC intention [client-visible] [required] | C | Integrate Design §3/§6 hate/AI hooks: `\|effectPoint\|`, actual-heal/AddHate and GetAgro intention. **Done When:** §5.1/§2.8 aggro cases preserve current-target else lowest EntityId tie and no GetAgro hate; TargetMe deferred. |
| 4.2 PvP flags, counters, karma and CP [client-visible] [required] | C | Implement state_pvp using Design truth table/math and Z damage hooks (§2.6). **Done When:** §5.5 cross-product/expiry/cap plus §2.8 CP/actor-only max-deadline/counter/karma runs pass without duplicate persistent effects. |
| 4.3 Architect R7 peace geometry and hostile boundaries [client-visible] [required] | C | Ship **architect R7** square/hostile enforcement (§2.6). **Done When:** §5.5 inclusive/both-endpoint/AI vectors and pvp-flag-a show pre-accept reject, later cancel/block and suppressed pulses advancing. |
| 4.4 Death record, XP loss and untimed penalty [client-visible] [required] | C | Implement Design death/record/penalty via Z hooks. **Done When:** §5.5 eligibility/no-draw/cleanup/delevel and death-penalty scenario pass; actual loss/penalty reconnect, no double charge, town rule unchanged. |
| 4.5 Resurrection offers, consent and crash-safe one-time revival [client-visible] [required] | C | Implement Design offer/consent/refund via explicit corpse cast (§2.6). **Done When:** Every §5.5 lifecycle/crash and §2.8 role variant passes, preserving MP/CP and one XP refund/final maxHP. |
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

**Planned, not executed.** Use existing module/test conventions; new filenames below are proposals. §2.8 is the only scenario matrix. Actor/adapter tests cover precise races/crashes that bots cannot force; final digest alone cannot replace record-byte evidence.

### 5.1 Data, formula, cast/effect and schedule evidence

Implement **Design §8.1** and §2.7's enumerated ledger. E1 supplies independent expectations; Z integrates Rust/domain tests (proposed formula_oracle.rs, skill_classifier.rs, tick_ordering.rs and state_skill/effects/pvp tests):

- Classifier every rank/parameter/stat/priority/filter/condition/section/item/reference, unknown/transitive blockers and manifest drift. Runtime Squire's Sword guard, catalogue status/blockers and reported counts; no forced count.
- Oracle revisions/literal/input/vector hashes, exact intermediates, class/level/modifier orders, signed/Func/getter casts/caps, perfect/nonperfect sqrt/near-boundaries, all attribute coefficients, magic crit/both failure rolls/≥10 gap, binary32 conversion, exact karma compound truncation, actual-heal hate/zero/multiple ledgers, resurrection quirk/refund rounding/XP-floor loss. Precision unproved → registry rejection.
- C5 boundary/default regression vectors: abnormal-success default 10/90 and explicit per-skill min/maxChance overrides with immunity/no-draw exceptions; getter clamps below/at/above MaxMAtkSpeed1999/MaxPAtkSpeed1500/MaxEvasion250 loaded from tables/formulas.toml; missing magicLvl0, castRange/effectRange−1 source-specific unlimited/none semantics and abnormalLvl0 preserved through generator/oracle/runtime.
- RNG full Design §6 callsite/target/handler/bound/value/comparison/cursor ledger, including no-draw exits, continuous/duration/instant/SELF, zero-evasion/vengeance/reflection, attack-break pre/post-resource and out-of-window draw, stun/death order. Invalid/canceled adds no handler draws, prior selection draws retained.
- Cast authorization/generation/rank/captured target, staged Cancel/Move/Cast order, every rejection/cancel, MP split/start reuse/HP>cost, all timing vectors/independent cutoff t−1/t/t+1, after-launch cancellation and range/life/incarnation/protection/peace rechecks.
- Effects pools/shrink/stack/NONE/one-container/status overlap, dispel protected/newest/truncation, cold/cache/shared install order, live hooks versus rebuild. MaxHp100×1.5 heals50 once; priority-1 evasion before later funcs; immunity/basic-property/activate-rate exits, no universal min/max assertion.
- Periodic ticks1/3 cadence/binary32 amount/DIFF-PER/phase/expiry/suppressed advancement, HP1/CP nonlethal clamp, player/NPC direct/DoT/HP-consumption/zero/CP-only break/stun behavior. Natural player/NPC regen/fractions/dead/full boundaries separately.
- All Design §3 tick edges, shuffled UUID-versus-ordinal order, event+state comparison including kill SP/karma/level/tokens. Aggro current-target/EntityId ties, effectPoint once/AddHate/actual-heal fanout, ordered GetAgro→hate/hate→GetAgro/two/no-op/stale-dead requests and reverse-index restore.

Review oracle independently Z/C, preserve old sheet/outcome expectations and production SOURCES/integer-only checks. No Rust-authored expected answers or tolerance widening.

### 5.2 LearnSkill and read endpoint matrix

**API guidelines §4 + Design §5.3** bind ListLearnableSkills/catalogue and LearnSkill. Unit in-memory use cases, real TestApp wire suites (proposed grpc_learn_skill.rs) and DATABASE_URL-gated PostgreSQL adapters are required. C publishes RPC comments/typed mapping first: API §3 errors, documented state/prerequisite FAILED_PRECONDITION, oversize RESOURCE_EXHAUSTED, sanitized infrastructure INTERNAL. Trigger every reachable AppError and assert every documented tonic::Code.

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

24-hour receipts follow API §2, not bounded transfer history. Historical replay tests operate within retention; exact-rank guards prevent a cleaned-up key buying another rank. Every new method meets API §4 round-trip/constraints/atomicity/concurrency.

### 5.3 First harness task: predicate → authoritative message mapping

**Publish/implement before dependent scenarios** (E6.5). UNKNOWN until received valid evidence; reset connection/entity/generation scope and fence tick/incarnation/IDs, with §2.1 baseline exceptions. No client formula/animation guess or live zone mutation. Publish actual DSL grammar/source/reset/error handling; unsupported syntax fails.

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

Extend contract audit preserving old rules/counters; command acceptance differs from successful effects. Mapping UNKNOWN/reset/parser stub evidence precedes live work. **run-sim.sh/run-sim-multi.sh/run-sim-ci.py** orchestrate scenarios/record/replay/trace/contract/JUnit. **run-tests.sh --require-live-api** runs named Unreal HUD/keybind/reducer/live UI automation; cooking/staging separately proves assets, .nfs cannot verify pixels.

Guarded Phase2-derived packs own fresh PostgreSQL/JetStream, migrate/seed **before API/admission**, record normalized loads and exact role/account/class/level/SP/rank/resources/position/data-binary-seeder-vector hashes. Private tokens/owned ports/cleanup/quarantine/old fixtures remain intact. First cast rank unlearned; low-resource/dead/other skills/overlays explicit, peace crossing via real movement.

Exact paired units cast-observer-a/b, pvp-flag-a/b, resurrect-a/b use distinct accounts/matching headers; orphan/mismatch fails. Include competing-keltir role reversal/near-cap variants. Solo-named karma/aggro needs explicit cooperating role manifest with unique artifact namespaces: **new E6.5 orchestration deliverable**, current runner refuses duplicate same-name processes. Publish before use or add named companions/reconcile §2.8 during implementation. Every role is a real client with retained report. Timer/offer/replacement/seed variants recorded; precise crashes/races are linked actor/adapter evidence, never live SQL/test setters.

### 5.4 Persistence and recovery acceptance

All **Design §5.1/§5.2** fields/triggers/invariants required; memory/PostgreSQL parity, fake-ack actor/checkpoint and real adapter transaction evidence:

1. Complete SP/learned/progression equality, PvP/death/fractions, effect/reuse/source/periodic payload/shared install order; full fingerprint/frozen-request parity, old bytes preserved, invalid bounds rejected.
2. **50/51 reuse,100/101 buff/debuff**, zero/positive flags/untimed penalty/actual loss; apply/refresh/remove/expiry atomic child replacement. Threshold crossing at real save; retries never recompute remainders/thresholds.
3. Every retained/flag/MP/CP/fraction/learning/progression/death/kill/resurrection trigger detected; one final fenced character write/tick includes receipts/ordered outbox. Only ordinary existing HP/XP cadence exception.
4. Independently pause log ack and post-log checkpoint ack: **no output/RPC success/next draft/incompatible snapshot**, exact immutable record/RNG/request retry, no rerun.
5. Crash before append, lost log ack, after log before DB, DB lost ack and before terminal reply: reconcile acknowledged work before admission; critical detector cannot discard it. Exactly one SP award/spend/rank/PK/PvP/karma/refund and receipt/outbox set; disconnect cannot cancel logged learning.
6. Stale revision/generation/permanent lane/fingerprint-key conflict fences without output or silent logged-work rollback. Every failed child/receipt/outbox step leaves no partial rows; typed adapter errors/sole relay parity.
7. Reconstruct before clamp/validation with shared order/source/ideal-ms phase, no hooks/heals/instant/end/RNG/cost replay. Rebase at actual deferred admission; snapshot keeps exact short deadlines/fractions.
8. Disconnect drops offer/short timers but retains death record; replacement preserves live cast/offer/ordinal/deadlines without DB reload. Dead admission no cast/regen credit; idle owner bootstrap complete; new offer after reconnect refunds recorded loss once across recovery.
9. Competing/repeated lethal/corpse hits credit living killer once; dead/departed source none. Human XP bonus does not affect SP; zero/i64::MAX reward/spend/actual saturation preserve active-slot equality. No replay/reconnect reaward.

C adapters precede E2.6; Z reviews durability, C reviews Z restore/replay. Show withheld output, not eventual correct rows alone.

### 5.5 PvP, peace, death and resurrection evidence

Design §3 truth table/§8.1 full cross-product including chaotic independent flags, actor-only nonshortening deadline/victim unchanged/exact impact expiry; CP-only/HP/overkill/player-DoT versus NPC, protection/no healer-time-death decay/caps. Peace inclusive/corner/both-endpoint crossing/AoE/pulse advancement/AI-leash: pre-accept rejection differs from later cancel/block. Death eligibility/20% seeded outcomes/cap/no-draw/XP-floor/delevel/grants/cleanup/private untimed penalty and actual loss recorded.

Every resurrection case is mandatory, paired §2.8 runs plus precise actor/adapter evidence:

- Explicit PC_BODY, private frozen offer/id/created/expiry/loss/percent, hidden source absent; no corpse SetTarget.
- Competing offers retain first; post-invalidation new id; stale/wrong-incarnation/duplicate consent no refund.
- Decline/real disconnect invalidate; live replacement preserves; source disconnect leaves safe attribution; offline stores no offer.
- Accept t−1/exact-expiry, expiry wins; both consent/town command orders no double revival; later-created offer cannot be preaccepted.
- Reconnect death record/no offer → new offer; later death invalidates previous incarnation/consent.
- D-R5′ HP 65% if lower/MP-CP preserved/corpse/no protection/incarnation/progression/final clamp, town unchanged. Oracle >20/equal20/base0/100/90cap quirk and Math.round refund/reduced loss.
- Same-ID unchanged AOI/maxHP change → respawn then final VitalsChanged exception/owner bootstrap, no duplicates/private leak.
- Crash/lost acknowledgements around consume/refund → one atomic XP/incarnation/loss/outbox effect (§5.4).

### 5.6 Replay, coverage, baselines and performance

Implement **Design §8.2 coverage schema3**, preserving all prior NPC/attack/class-transfer/token/contract fields. Zone facts count once, baselines count no transitions. Idle→Casting accepted start; Casting→Idle bounded reason histogram; self-pairs unreachable. Effect stable-ID Absent→Active/Active→Removed; same-ID Removed→Active unreachable. Pulses/stack-reject separate; requests versus committed learns, offers/CP/PvP/karma/regen/revival counted. No Recovering/lethal TickHp. Merge equal schema/reachability, recompute totals; digest-only facts require verified replay first.

**New V5 .nfr two players/NPCs** covers every Design §8.2 branch plus §2.1 corrections, with seed/rules/vector hashes. Old .nfr/V1–V4 digest/WS/record/compression and coverage/contract baselines untouched. Legal/illegal pairs/raw nested old-tag presence/empty fields/malformed state reject; canonical-field mutation changes V5 hash, derived indexes rebuild same. Arbitrary mid-cast/pulse/fraction/pending-checkpoint suffix equals uninterrupted **record bytes**; new-event byte corruption fails even with same final digest.

Pre-run admission tests: command limit/B−1/B/B+1, worst disposition/recipient reserve,1024 loads/eight-session limits,FIFO suffix/oversize head/rebase/no-rerun retry/crash recovery. Post-run oversize fences; full-registry/full-effect snapshot measurement before freeze.

Each file/role/variant retains manifests/logs/nonvacuous JUnit/export/byte-replay/trace/contract/coverage3/pipeline-verdict/cleanup; missing evidence fails. Preserve error/quarantine gates, no invented percentage. Traceability is structural only and refuses vacuous completion.

A20/Design §7: 1k NPC **mean<2ms**,1k players/observers **mean<10ms**, eight packaged clients casting/long effects **p99<20ms**. Reference hardware/build/data and simulation/digest/encoding/actor/log/DB measured separately; synchronized pulses/regen,0/8/full effects/hydration/critical writes before soak. Duration10,000 ticks test-design choice; no measured result or durability exemption here.

### 5.7 Telemetry and real validator

C extends **infra/scripts/verify-observability.sh**, preserving trace/log/Prometheus-OTLP/scrape; D catalogue/dashboard reviewed independently. Proposed cast metric `zone_skill_casts_total{outcome}` uses bounded start/completed/rejected/cancelled, no Ack-as-impact. Effect apply/remove/pulse/stack-reject bounded kind/reason; active gauge rises in active phase/returns baseline including 0. Learn outcome versus committed counters: frozen retry no new commitment; PvP/SP committed once, no observer/recovery duplication. Histogram gains samples; log/checkpoint labels bounded phase/result/reason. **No skill/entity/account ids/UUIDs/arbitrary errors**, enumerate cardinality.

Validator controlled fixture workload records pre/active/post scrapes and correlated committed facts, expected cast/effect/learn-new-retry/PvP/SP deltas and gauge values. Nonzero after soak insufficient; hydration/retry/replay does not inflate counters.

## 6. Dependencies, order and integration gates

Baseline: Phase1a harness, Phase2 progression/tokens, existing actor/checkpoint/outbox/codecs. Artifact order, no calendar estimates:

1. **Foundation:** C committed proto/error/receipt contract **241293f** on **codex/phase3-server-api** ([world.proto/game.proto and proto-review-1.md](#21-additive-wire-allocation-and-visibility)) → Z, with required handoff closure for TargetChanged.tick + baseline/stale/subsequent-selection tests preserving legacy encoding, all three RPC/WS fail-closed endpoint tests, and handler-consistent catalogue UNAVAILABLE documentation (§2.1); D generator/manifest/oracle → Z/C, with all eleven rejected data-review findings blocking handoff (§2.4); C E6.5 predicate/DSL/parser/role foundation → Z. Parser/stub/provenance evidence only; do not gate on whole E1/E6.5 live completion.
2. **Runtime:** Reviewed definitions/proto → Z E2.1–E2.5/E6.1 → C. Publish hooks, measure snapshot/pre-run admission before freeze; no dependency on live E2.6.
3. **Adapters/gameplay:** C E3/E4 consumes published hooks → Z. **E3.4 adapters/E3.3 receipts precede E2.6**, separately reviewed by C; serialize shared edits through Z, no circular entire-epic gate.
4. **Client/harness:** Proto enables E5 skeleton; live requires engine/API/adapters. First 3-cast-skill then §2.8. C E5/harness/metrics → Z; D docs/dashboard → Codex; **Z E6.1/E6.2 → C**.
5. **Live:** Scenario wrappers plus separate live automation/cooking, V5/legacy replay/coverage3/contract/trace and endpoint/actor/adapter concurrency/crash evidence complete live stories; declarations alone never mark done.
6. **Close:** C reviews Z DB/log/CPU measurements before packaged soak/telemetry. Root serializes Unreal/integrates reviewed outputs; outcome cites evidence.

000008 extends existing SP schema; fixtures migrate/seed before admission. Publish commits/types/manifests/evidence at handoff. Failed privacy/compatibility/durability/roles block completion without baseline changes. This revision runs no implementation gates.

## 7. Risks and mitigations

| Risk | Consequence | Mitigation/evidence |
|---|---|---|
| Legacy semantic/codec leakage | Changed old bytes or accepted nested Phase3 fields | Isolated policy/recursive presence validation and preserved fixtures (§2.2/§5.6). |
| Arithmetic/RNG divergence | Incorrect outcomes or replay | Independent rounding/caller ledger, binary32/sqrt bounds and cursor traces (§2.7/§5.1). |
| Durability/snapshot cost | Missed budgets or unappendable work | Coalesced writes, immutable retries, pre-run byte admission and measured registry/resource/checkpoint cost (§5.4/§5.6). |
| Baseline/privacy gaps | Dangling references or leaked resources | Identity-first hydration, owner/bootstrap/respawn exceptions and hidden/mutual-reference cases (§2.1). |
| Overclaimed coverage/dependency deadlock | Green headers without behavior or circular work | One §2.8 matrix, mapped predicates/separate automation and split foundation/runtime/admission/live gates (§5.3/§6). |

## 8. Out of scope

Enforce exclusions through manifest/blockers and guards; unsupported portions cannot execute partially.

- Pets/servitors/summons: no runtime entities, feeding/collars/crystals/costs/XP sharing; reserve kinds/neutral penalty hook only. Summoner trees 14/28/41 excluded; Phase4 item dependency is owner-approved.
- Item-consuming skills/arrows/soul or spirit ore/shots, inventory/equipment systems, weapon attributes, shields/blocking and high-grade branches. Supported conditions still enforce real Squire's Sword; unsupported weapon/slot prerequisites defer. No chaotic item drops.
- Operate/target/scope capabilities outside Design §1.1: T/CA/DA/absent A4, PARTY/PLEDGE/PARTY_PLEDGE/DEAD_PLEDGE, GROUND/FAN/SQUARE, SUMMON/NPC_BODY/DOOR_TREASURE and unsupported scope filters. Proximity never substitutes for party membership.
- Unknown parameter/stat/recursive condition/section/reference, Charges/energy/race/slot/item requirements, unsupported mastery/trigger behavior. Imported triggered definitions provide closure/provenance, not triggering runtime support.
- **Fear is excluded** because it needs flee pathing; no phase assignment. TargetMe and TickHpFatal remain named blockers. Unsupported summon/cubic/transformation/lethal/fatal/blow/drain/spoil/sweep/escape/teleport/hate-delete/randomize/target-cancel/crafting handlers do not execute without later scope/manifest approval.
- Sitting, Mother Tree/estate/siege regen bonuses, guards, extra world content/zone types/peace zones. Preserve existing NPC/leash behavior; one architect-configured town square is delivered.
- Unpopulated trees, playable third transfers, Olympiad/Noblesse/live subclass-certification, party/clan/alliance/sieges and cap expansion. Preserve metadata hooks; cap 85, playable transfers through tier 2.
- Final combat animation/audio/art, prediction, replication/GAS, client-authoritative combat or UE server. Deliver thin-client placeholders/UI with cooking evidence.

## 9. Outcome and review disposition

**Approved with bounded corrections applied (revision 5): implementation is underway in sibling worktrees; no story is marked complete; this plan revision performed no implementation or execution gates; phase acceptance remains pending.** Only this plan edited; no builds/tests/scenarios/benchmarks/migrations/implementation run. §2.4 counts are reported classifications from the rejected data revision, pending fixes/regeneration/independent re-review; all eleven findings remain handoff blockers. These are document dispositions; no story has completion prefix.

| Finding | Section map |
|---|---|
| 1 Pins | §2 |
| 2 Wire/privacy/types/hydration | §2.1; E2.4/E5.5 |
| 3 SP/migration/backend | §2.5; E3.4/§5.4 |
| 4 Capability/counts | §2.4; E1.2/§5.1 |
| 5 Traceability/E6 ending | §4; E6.5/E6.7 |
| 6 Behavioral coverage | §2.8 |
| 7 Harness/coverage/V5/baselines | §5.3/§5.6; E6.1/E6.5 |
| 8 Endpoint matrix | §2.5/§5.2 |
| 9 Persistence/recovery/boundaries | §2.5/§5.4 |
| 10 Dependencies/review | §3/§6; E2.6/E6.1/E6.2 |
| 11 Aggro | §2.6; E4.1/§5.1/§2.8 |
| 12 Peace/corpse | §2.6; E5.1/§5.5 |
| 13 Formula/rounding/branches | §2.7/§5.1 |
| 14 Resurrection lifecycle | §2.6/§5.5 |
| 15 PvP timing/actor refresh | §2.6/§2.8/§5.5 |
| 16 Telemetry/validator | E6.4/§5.7 |
| 17 Runtime provenance/exclusions | §2.4/E3.1; §1/§2.6/§8 |
| 18 Duplication/status/syntax | §2/§4/§9; E6.3 |

Phase-close placeholder: reviewed commits and linked data/API/actor/client/replay/performance/observability evidence, limitations and dispositions. **Phase acceptance remains pending.**
