> **BINDING RECORD (as of integration):** This document is the binding Phase 3 zone design record as of integration.

> **ARCHITECT ERRATA (binding, 2026-10-10):** (1) architecture-decisions §9 C5 replaces each "1/99" default in this document. The pinned abnormal-success clamp is 10/90 (character.properties:277-278), unless the skill supplies min/maxChance. MaxMAtkSpeed is 1999. The Skill.java defaults for a missing attribute are: magicLvl 0, ranges −1, abnormalLvl 0. (2) The design-review-2.md findings 1–3 are accepted requirements (§9 C2/C2'). These findings are: pending_swing hydration after the identity batch; owner/bootstrap baseline exceptions; a final public VitalsChanged after an incarnation change. (3) §9 C1: affectLimit width 0 = unlimited.

# Phase 3 zone engine: implementation design v3

This is the design review artifact v3, 2026-10-10. It is a design only. We did not run builds or tests, and we did not change tracked files. If architecture decisions §5–§8 and earlier text do not agree, §5–§8 apply. §8 F1–F16 gives a resolution for all findings in design-review-1.md. Below, the references to R/D-R/F refer to those binding dispositions.

## v3 changes

| Finding | Resolution |
|---|---|
| F1 — RNG caller order | §6 ledger: continuous success/duration before instant handlers, separate SELF evaluations, a positive-damage reflection draw also at zero; §8.1 injected traces. |
| F2 — Unappendable inputs | §4.5 actor byte-budget admission, bounded normalized loads, ordered deferral, terminal rejection and recovery. |
| F3 — Cast staging | §1.2 and §6.1: animation, continuous −300 ms, launch −400 ms, impact +400 ms; timing vectors. |
| F4 — Interrupt deadline | §1.2/§6.1: an independent floor(trunc(animation_ms)/100)−2 cutoff; the earlier completion−2 ruling is cancelled. |
| F5 — TickHp nonlethal | §1.3/§3: HP−1 clamp before CP; §8 removes the lethal-DoT fixtures; TickHpFatal is deferred. |
| F6 — Periodic units | §1.3: 666 ms ratio, binary32 amount multiplier, DIFF/PER, exact phase/cadence and persistence. |
| F7 — Damage/status removal | §1.3/§3/§6: player/NPC removal and removal for each cause, ordered stun roll, paralyze separate. |
| F8 — affectLimit | §6/§7: offset+width, zero/no-draw branch and corrected maxima/vectors. |
| F9 — Intermediate Func casts | §1.4/§6.2: priority-1 evasion truncation before later modifiers; V5 vectors, legacy unchanged. |
| F10 — Handler lifecycle | §1.3/§1.4/§5.2: onStart before generic Funcs, MaxHp capture/install/heal, reconstruction without one-time hooks. |
| F11 — GetAgro | §3: ordered forced-intention requests at the AI phase, no invented hate increment. |
| F12 — Restore formulas | §6.1: explicit Heal, HpPerMax, Mp and Cp branches, caps, crit and resource conversion. |
| F13 — Spawn references | §2: observer-filtered identity spawns, then hydration, explicit absent references and baseline semantics. |
| F14 — Public max HP | §1.4/§2: VitalsChanged on a change of HP or effective max HP, coalescing and lifecycle order. |
| F15 — Legacy nested injection | §4.1/§4.4: recursive validation that knows the version, with old tags; §8.1 negative fixtures. |
| F16 — Citation corrections | §2 keeps StatsChanged.tick=14; §4.2 describes magic/LE length/zlib with no envelope hash. |

Open architect decisions: **none**. The snapshot-size and performance measurements stay as implementation gates (§9). They are not open design decisions.

## Source convention and reconciliation

File citations use these exact directory aliases: **Z/** = `apps/api/src/domain/zone/`; **A/** = `apps/api/src/application/`; **R/** = `apps/api/src/application/replay_log/`; **I/** = `apps/api/src/interface/`; **W** = `packages/proto/nightfall/v1/world.proto`. Citations refer to files that we examined. **New** symbols are proposals. They are not in the implementation now.

For v3, we read all of `design-review-1.md` and the binding `architecture-decisions.md` §8 in `/home/matt-woodruff/.config/Claude/side-session-notes/phase3-codex-2026-10-10/`. We examined the changed source claims again against the two pinned clones. The earlier §5–§7 dispositions stay, except where §8 replaces them. The historical context stays in `codebase-state.md`, `reference-reconciliation.md` and the replaced sketches in `docs/planning/03-combat-and-skills.md:22`, `:283`, `:461`.

Pinned sources: `/home/matt-woodruff/.cache/nightfall/l2j/l2j-game` (HEAD `abfde0490ac52a2106e884b7242c0276c44cdf76`) and `.../l2j-dp` (pinned `3ca488dd2bd0bfaca43e378886a3c2e37968153a`). Below, these aliases apply:

- **G/** is `src/main/java/com/l2jserver/gameserver/` of that game revision.
- **D/** is the root of that datapack revision.
- **E/** is `D/src/main/java/com/l2jserver/datapack/handlers/effecthandlers/`.
- **game/** is the root of the game repository.

These sources are for provenance only. They are not runtime dependencies. `packages/data/SOURCES.md:10` and `packages/data/scripts/gen_tables.py:34` also record them.

Keep these items: the single-owner `ZoneState`, the ordered entities, movement/AOI, the receive-order drafts and the durable gate (`Z/state.rs:402`, `:732`, `:1259`; `A/zone_actor.rs:577`; `docs/engineering/architecture.md:101`). You must correct the mismatches that follow on purpose:

| Existing implementation | Phase 3 action |
|---|---|
| `run_tick` runs AI before movement and impacts (`Z/state.rs:757`). | Dispatch a separate V5 tick policy. Keep the legacy body unchanged. A5 moves AI after the new starts. |
| `set_target` excludes players and dead entities (`Z/state.rs:1131`). `may_attack` lets only player↔NPC attack (`Z/state_combat.rs:981`). | Use V5 target/hostility validators. Corpse targets are explicit cast targets. They do not relax all auto-attack rules. |
| `land` subtracts HP directly (`Z/state_combat.rs:409`, `:459`). | Use a V5 damage resolver that auto-attacks, skills and DoTs share. It includes CP, interruption, death and credit. |
| `validate_class_entity` compares against a profession sheet with no effects (`Z/state_class.rs:9`, `:47`). | Before the comparison, V5 builds the effective sheet again from base data, learned passives and effects. |
| `reconcile_player_tokens` runs only for BinaryV4 (`Z/state_class.rs:97`, `:104`). | Explicitly let V4 and V5 run it. Do not disable the Phase 2 grants by accident. V3 stays unchanged. |
| `ClassState` already controls SP, learned skills and CP (`apps/api/src/domain/character_progression.rs:88`). | Extend the persistence projections. Do not add a second SP or CP ledger for spending. |
| `ClassStatsView` already projects SP/CP (`Z/phase2.rs:35`, `:54`). The wire StatsChanged has SP at 11 (`W:257`). | Use those fields again. Append the PvP/death/effective-stat fields. |
| The architecture documentation still describes snapshot 5/binary 2 and coverage schema 1 (`docs/engineering/architecture.md:136`, `:300`, `:447`). | The code is the authority: snapshot 8/V4, record 6. Coverage is already schema 2 (`Z/state.rs:45`; `R/codec.rs:606`; `apps/api/src/bin/replay/coverage.rs:66`). |

## 1. State, definitions, effects and effective stats

### 1.1 Module boundaries and ownership

Extend the `Z/mod.rs:27` exports and its deterministic `SOURCES` list at `:91`. Add these **new** files:

- `skill.rs` (typed definitions/registry)
- `skill_math.rs` (pure formulas)
- `effects.rs` (value types/stacking)
- `phase3.rs` (policy/state/load types)
- `state_skill.rs` (cast execution)
- `state_effects.rs` (application/expiry/pulses)
- `state_regen.rs`, `state_pvp.rs`, `state_death.rs` and `state_learn.rs`.

Z is responsible for `state.rs`, `state_combat.rs`, `command.rs`, `state_digest.rs`, the replay record/codec and the snapshot codec. C is responsible for API/persistence/schema. C adds `state_pvp.rs`, `state_death.rs`, `state_learn.rs` after Z publishes the hooks below. Edits to shared files occur one at a time (R15). Each new file must stay in the no-float/no-wall-clock source scan (`Z/mod.rs:17`, `:91`).

Add `ZoneState.phase3: Option<Phase3Zone>` and a matching optional `Phase3Snapshot` block. The block has only serialized canonical fields. These fields are different from the runtime indexes that the code builds again. Do not put new fields with default values into the JSON of each old entity. `None` is mandatory for legacy policies. **New** value types:

```rust
struct SkillId(u32);                 // same numeric id as SkillLearnDef.skill_id
struct SkillRef { id: SkillId, rank: u32 }
struct EntityOrdinal(u64);           // allocation order, not UUID order or command ordinal
struct CastId(u64);                  // zone monotone allocation, no RNG draw
struct EffectId(u64);                // one continuous skill application, not each modifier
struct OfferId(u64);
type ReuseTimers = BTreeMap<SkillId, Tick>;

struct Phase3Zone {
    rules: Arc<Phase3Rules>,          // immutable, canonical snapshot data + hash
    next_entity_ordinal: u64,
    next_cast_id: u64,
    next_effect_id: u64,
    next_offer_id: u64,
    next_install_sequence: u64,       // shared order across passive and timed Func sources
    entities: BTreeMap<EntityId, Phase3Entity>,
}
struct Phase3Entity {
    ordinal: EntityOrdinal,
    cast: CastState,
    reuse: ReuseTimers,
    effects: EffectList,
    passive_install_order: BTreeMap<SkillId, u64>,
    regen: RegenSchedule,
    fractions: ResourceFractions,    // Q fractions of HP/MP/CP, each 0 <= x < Q
    player: Option<Phase3Player>,
}
enum CastState {
    Idle,
    Casting(CastInFlight),
}
struct Phase3Player {
    pvp: PvpState,
    death_penalty_level: u8,          // 0..=5, authoritative scalar
    last_death: Option<DeathRecord>,  // actual XP lost, incarnation, pre-death XP
    resurrect_offer: Option<ResurrectOffer>,
}
```

Keep `Entity`, `CombatState`, `CombatRole`, `PlayerProgression` as the original owners of identity/position, HP/MP/life/swing, XP and class progression (`Z/entity.rs:63`; `Z/combat.rs:79`, `:102`; `Z/phase2.rs:12`). The phase3 sidecar is mandatory for each V5 combatant, with NPCs included. It is forbidden for unknown entities. Allocate entity ordinals for all V5 spawns in the actual spawn order, with slot members included. The same slot entity gets a new ordinal on its next life. `ReplaceSession` keeps the ordinal, cast, effects and deadlines. Despawn removes them. Existing NPC IDs still come from `random_entity_id` and the zone RNG (`Z/state.rs:1207`; `Z/state_ai.rs:98`). Ordinal allocation must not use that RNG. Each counter uses a checked increment. If a counter is exhausted, the tick fails before publication.

`Phase3Rules` has these items:

- The registry definitions and the executable-feature manifest.
- The magical stat/regen templates.
- The karma decimal ratios and the caps.
- The timing policy, the peace geometry and the policy revision.

This block is separate from the existing serialized `StatRulesParts`. Thus the new regen does not change old hashes (`Z/stat_rules.rs:272`, `:305`; `Z/state.rs:1764`). Hash the skills, class linkage, formula constants, admission timer policy and geometry together. The existing class/rules hashes stay as separate hashes.

The registry includes all 378 tree references, 5076, and the transitive skill references. After D-R11, Tier B is: `PhysicalAttack`, `MagicalAttack`, `MagicalAttackRange`, `Stun`, `Sleep`, `Root`, `Buff`, `Debuff`, `TickHp`, `TickMp`, `Heal`, `HpPerMax`, `MaxHp`, `MaxMp`, `Mp`, `Cp`, `Resurrection`, `GetAgro`, `AddHate`, `DispelBySlot`, `DispelByCategory`, `DefenceTrait`, `AttackTrait`, `Passive`. `TickHpFatal` and `TargetMe` are deferred, with the blockers `effect:TickHpFatal` and `effect:TargetMe`. We do not promise a forced-selection implementation. Only these values are supported:

- Operate types: A1/A2/A3/P.
- Targets: SELF/TARGET/ENEMY/ENEMY_ONLY/PC_BODY.
- Scopes: SINGLE/POINT_BLANK/RANGE/unset.

Enforce `using kind` against the equipped Squire's Sword. Enforce `player canResurrect` against the actual state. The capability checks include these items:

- Each parameter/stat child (`mul`/`add`/`sub`).
- Each recursive and/or/not condition.
- The scope filter.
- Each effect section (with self/start/end/channeling/pvp/pve).
- Item consumption.
- The closure of referenced skills.

If a requirement is unknown, the full skill is deferred with named blockers. Fear, party, toggles, summons and unsupported mastery/trigger behavior cannot run partially. Generate exact counts. The value 241/379 is obsolete, and it is not a gate. Reserve only the pet/servitor kinds and a neutral owner-XP-penalty hook. No pet entity, pet cost or XP sharing runs (existing kinds at `Z/entity.rs:54`, `W:141`).

The concrete new entry points that follow make it possible to examine the module ownership. All of them are methods on `ZoneState`, except the pure/value helpers:

| New file / symbol | Existing integration point to extend |
|---|---|
| `state.rs::run_tick_v5`, `run_tick_legacy` | Move the current `run_tick` body out with no semantic edits. Dispatch at `Z/state.rs:757`. |
| `state_skill.rs::{stage_skill_intent, begin_cast, cancel_cast, complete_casts, validate_skill_target, collect_affect_targets}` | `apply` at `Z/state.rs:906`; authorization at `:865`; the command phase and the new-start/completion phases. |
| `state_effects.rs::{apply_effect, remove_effect, expire_effects, pulse_effects, rebuild_effective_stats}` | `land` at `Z/state_combat.rs:409`, effect phase barriers, class/level mutations. |
| `state_combat.rs::apply_damage_v5` | Keep the legacy `land` arithmetic. Call the new resolver only from the V5 branch at `Z/state_combat.rs:459`. |
| `state_regen.rs::regen_phase`, `skill_math.rs::regen_amounts` | A new phase before completion; `CombatState` resources at `Z/combat.rs:102`. |
| `state_pvp.rs::{hostility_allowed, note_hostile_act, expire_flags, apply_player_kill, decay_karma}` | V5 alternatives to `may_attack` at `Z/state_combat.rs:981`, `attack` at `:45`, and the kill/award hooks at `:560`, `:696`. |
| `state_death.rs::{phase3_death, make_resurrect_offer, accept_resurrect, expire_offers}` | The existing death guard/XP charge at `Z/state_combat.rs:560`, `:650`; new intent admission. |
| `state_learn.rs::{learn_skill, learnable_skills}` | Copy the `change_class` guards/commit structure at `Z/state_class.rs:415`, but not its in-character receipt retention. |
| `phase3.rs::{Phase3Rules, Phase3Load, Phase3Snapshot, Phase3Checkpoint, restore_phase3_player}` | `PlayerLoad` at `Z/combat.rs:182`, the snapshot at `Z/state.rs:212`, the repository DTO at `A/ports.rs:157`. |
| `state_digest.rs::phase3_state_digest` | The existing digest dispatcher at `Z/state.rs:804`. |

### 1.2 Cast, timers, PvP, geometry and resurrection values

`CastInFlight` stores these values:

- The cast id, the caster incarnation, the skill id/rank and the original command ordinal.
- The start/launch/complete ticks and the launch-completed bit.
- The sampled affect limit and the selected/launch-filtered target bindings.
- The original animation milliseconds and the independent damage-interrupt deadline.
- The effective timing/cost inputs at the start time.
- `TargetBinding { entity, incarnation }` where applicable.

Include the start position only for rules that actually use it. SINGLE binds one target. RANGE binds its center target. POINT_BLANK binds the caster. If the selection changes, never silently change the target of an accepted cast. At completion, validate again the current existence/incarnation, life, range, protection, peace and status. The damage/effect strength uses the current effective stats at completion. The timing does not become longer when haste expires. The periodic DIFF strength is frozen. PER reads the current resource at each pulse (§1.3).

Execution deadlines are absolute `Tick(u64)` values (the periodic ideal-ms phase is more canonical data, §1.3). They are half-open: active while `now < until`, due when `until <= now`. Use checked arithmetic, and use `ceil_ms_to_ticks` after the truncation of the source milliseconds. Cast completion is at start+1 or later. `TICK_MS=100` and `Tick` exist (`Z/entity.rs:15`, `:23`). The key for reuse is the skill id across ranks. Reuse starts at the accepted cast start, never at completion. A cancellation gives back neither the reuse nor MP1 (`G/model/actor/L2Character.java:1514`, `:1533`, `:1542`). §6 pins the static/magic/physical reuse and the cast-time branches.

F3/F4 replace the completion-based cutoff of D-R2. These are the rules:

- Let A be the nonnegative `calcCastTime` animation in source milliseconds, and m=trunc(A).
- For A>0, the launch delay is L=max(0,trunc(A−(continuous?300:0))−400). The impact delay is I=L+(m==0?0:400).
- For A=0, L=I=0.
- Convert each nonnegative absolute offset one time: launch_at=start+ceil(L/100), complete_at=start+max(1,ceil(I/100)).
- A zero-delay launch occurs within the accepted start, with no recursive impact.
- Store interrupt_until=start+max(0,floor(m/100)−2). The window is empty when the signed offset is <=0. The max only encodes an interval that is already empty, without u64 underflow.
- The window is `start <= t < interrupt_until`. It is independent of the continuous head start and of the tick-ceiled impact.

Sources: `G/model/actor/L2Character.java:1495`, `:1508`, `:1614`, `:1625`, `:1643`, `:1645`, `:4577`. The source scheduler forwards nonpositive delays (`G/ThreadPoolManager.java:132`). `hitCancelTime` stays imported but is not used. The code also parses `coolTime`, but in the pinned game no caller uses its getter. Finalization occurs immediately after effect execution. Thus we do not invent a post-cast recovery lock (`G/model/skills/Skill.java:633`; `G/model/actor/L2Character.java:4666`, `:4690`). As a result, CastState has only Idle/Casting.

`PvpState { flag_until: Option<Tick>, karma: u32, pk_count: u32, pvp_count: u32 }`. "Flagged" means deadline > now. "Chaotic" means karma > 0 (red has precedence on the display). Saturation must be explicit and must agree with the DB bounds. Do not wrap. Only the scalar fields persist. The derived color does not persist. `RegenSchedule { next_tick: Tick }` uses 30-tick periods. Its first value is the spawn/admission/resurrection tick + 30, and each period adds 30. A restored replay keeps the exact original deadline. A new admission starts a new schedule. All NPC HP/MP and player HP/MP/CP get regen. Dead entities do not get regen, and they do not collect fractional credit. For regen/restoration, add Q amounts, move whole units to the vitals and keep the remainder. Keep the fractions across ordinary whole damage. Clear them on death, on a full-resource clamp or on an explicit reset. Persist and hash the fractions, thus a reconnect does not reset the rounding. Damage stays whole (D-R12).

`PeaceGeometry` is the configurable inclusive square with its center at (126,126) and a half-width of **10 tiles = 10_000 milli-tiles**: (116,116)..(136,136). It is in ZoneBounds, and it includes the Class Master (126,128). Release it in `packages/data/zones/test_zone.toml` (R7/D-R11). The two hostile endpoints must be outside the square at intent, at impact, at AoE recipient selection and at each pulse. Chaotic players have no exception. Suppressed pulses still go forward. Suppress NPC auto-acquisition inside peace. NPCs that are already engaged obey the existing leash/return rules. They cannot land hostile impacts through the boundary. There is no town flag clear. Membership comes from the position (`ZoneBounds` at `Z/state.rs:125`; safe point at `:255`).

`ResurrectOffer { offer_id, caster, caster_incarnation, target_incarnation, skill, created_at, expires_at, restore_percent_q, recoverable_xp }` has a configurable **600-tick** lifetime. Freeze the pinned WIT percent and the persisted `recoverable_death_xp_loss` of this death. The first valid offer stays until it is declined, expired or invalidated (D-R5 replaces the replacement wording of R12). Later offers cannot silently replace it. AcceptResurrect names the offer_id and an accept boolean. It succeeds only while the target is dead, for the same death incarnation, before expiry. These events invalidate the offer: decline, expiry, town respawn, acceptance and disconnect. A later offer has a new id. A duplicate or stale acceptance gets the rejection NO_RESURRECT_OFFER. The checkpoint does not include offers for offline admission. Socket replacement keeps the live offer. Consumption clears the recoverable XP one time, increments the incarnation and revives the player at the corpse without town protection.

The pinned skill revival is `L2PcInstance.doRevive(power)` → `doRevive()` → base vitals, then XP restoration (`G/model/actor/instance/L2PcInstance.java:7951`, `:7984`). For each resource, raise it to `max * restore_fraction` only when that configured fraction is >0 and the current value is lower (`G/model/actor/L2Character.java:2106`). The pinned defaults are CP=0, HP=0.65, MP=0 (`game/src/main/resources/config/character.properties:40`). Thus raise HP to 65% of the effective max with the §6 Q restoration fractions. Keep the current MP/CP. Do not set them to zero. Make the restored XP/level/passives consistent, clamp to the final maxima, start regen again at +30, and emit respawn and owner stats. Town respawn is unchanged in all versions: the existing HP/MP behavior, CP not changed, and 6000-tick protection (`Z/state_combat.rs:806`, `:841`).

### 1.3 EffectList and ordering

`EffectList` has `BTreeMap<EffectId, ActiveEffect>` as canonical state. The derived indexes are `BTreeMap<StackKey, EffectId>` and `BTreeSet<(Pool, applied_tick, EffectId)>`. After a restore, build them again from the canonical entries and validate them. An `ActiveEffect` stores these values:

- The skill/rank, the source id/incarnation/source kind and the original target incarnation.
- The applied/expiry ticks (the expiry is optional for explicit permanent effects).
- The pool and the abnormal type/level.
- The ordered handler instances, with their next ideal pulse millisecond offset, the pulse index and the typed DIFF/PER payload fields.

Store the effects of one skill as one entry that uses one slot. `Buff { mul pDef, add evasion }` is not two buffs. Many periodic handlers keep their source XML ordinal.

`Pool = Buff | SongDance | Debuff | System`. System is only for slot-exempt effects that the data confirms, for example reconstructed passives/death penalty. The normal capacities are 20+the clamped Enlarge contribution (0..4), 12 and 8. Passives are in modifier sources that come from learned skills, not in the finite timed pools. Make the death penalty a system modifier source for skill 5076 at the scalar rank. It has stay-after-death semantics and no expiration. Do not copy its scalar into a separate buff that has its own restore and rank.

The stack key is `Abnormal(abnormal_type)`, except for NONE, which is `Skill(skill_id)`. For an existing stack, the code ignores a lower abnormal level. It does not restart the expiry, and it does not evict. An equal or higher level replaces the full old application. It emits the removal before the application, restarts the duration/pulse schedule and gets a new EffectId. The source identity is not part of the stack key. If there is no conflicting stack, enforce the capacity: remove the oldest `(applied_tick, EffectId)` first. If the slot cap becomes smaller, immediately evict the oldest excess buffs in that same order. In dispels, obey the supported irreplaceability/category flags. If the code cannot correctly show a protected-slot overflow case, mark that skill as deferred or make an explicit decision for it. Never loop forever to find an eviction candidate.

All expiry removals for a tick are in the order `(expires_at, target_entity_ordinal, target_id, effect_id)`. Pulses are in the order `(due_tick, target_entity_ordinal, target_id, effect_id, handler_ordinal)`. Container admission validates the replacement and reserves the slot. Then it removes the old entry and starts the new handlers in source scope/XML order. For each continuous handler, do these steps in sequence (`G/model/skills/BuffInfo.java:260`, `:266`, `:272`, `:278`):

1. Run onStart.
2. Register its pulse schedule.
3. Install its generic Funcs.

Build the stats again before each later stat read. Do not install all modifiers before the hooks. Funcs that a hook controls have a separate ownership, and the code installs them exactly one time. Handler-local function ordinals put them before the generic Funcs of that handler at equal priority. Instant handlers run while the code builds BuffInfo, before the continuous container is installed (§6). Their successful effects stay after a later stack rejection. The instant part of A2 can succeed when its continuous part is resisted or a stronger stack rejects it. The result entries show the two parts separately. A duration expiry at t prevents a pulse at t.

`StatusFlags` is a typed integer mask that comes from all live effects. It has: stun, sleep, root, magic mute, physical mute, paralyze, and the resurrection/debuff blocks that the data supports. Never clear a flag only because you remove one effect. A different effect can still supply the flag. These are the results of each flag:

- Stun/sleep/paralyze stop movement, cancel the cast and swing in flight, and suppress new attacks/casts.
- Root stops movement/chase, but lets the entity do actions in range.
- Magic/physical mute cancels or rejects only the matching casts.

Damage-breakable removal uses the skill flags and `awake`. It is not one sleep rule for all entities:

- Player status calls it for all damage that is not HP consumption, DoT included.
- Generic/NPC status calls it only for damage that is not DoT and not HP consumption.
- Non-toggle TickHp supplies awake=true (`G/model/actor/L2Character.java:5220`; `G/model/CharEffectList.java:847`, `:872`).

After that removal, if the target is still stunned and the damage is not DoT and not HP consumption, draw `Rnd.get(10)`. Remove the stun only on zero, before the CP/HP subtraction (`G/model/actor/status/PcStatus.java:82`, `:93`, `:185`; `G/model/actor/status/CharStatus.java:142`, `:149`). These status hooks do not have a gate on positive absorbed damage: a valid reduceHp call with a zero value still gets to them. Paralyze has no stun roll of this type. Only its own removal flags/lifecycle apply. Hard-disable cancellation does not use the damage-interrupt window or a random roll. `TargetMe` stays deferred. GetAgro puts a forced intention in the queue and does not change the hate. AddHate changes the hate (§3). Keep `HateLedger::most_hated` unchanged (`Z/combat.rs:344`).

**Periodic contract (F5/F6).** The XML `ticks` is an interval factor. It is never a pulse count. I_ms=ticks×666, with positive bounded ticks and a checked multiplication. The value 666 is pinned at `game/src/main/resources/config/character.properties:145`. These rules apply:

- The first pulse is application+ceil(I_ms/100).
- Each later pulse n is application+ceil(n×I_ms/100). It is not the previous rounded tick+ceil(I_ms/100).
- Persist the ideal next-pulse milliseconds/pulse index, and also its derived tick. This keeps the sub-tick phase across snapshots and paused admission (store the remaining ideal milliseconds relative to the checkpoint tick).
- Also let suppressed peace-zone pulses go forward.
- At an equal due tick, the expiry wins. There is no pulse at or after expiry, and no wall-time catch-up.
- For ticks=1, the first three offsets are 7,14,20 ticks. For ticks=3, they are 20,40,60.

The source schedules the first and repeated work at I_ms (`G/model/skills/BuffInfo.java:269`). For non-toggle skills, a false return of the handler does not stop the effect (`:295`, `:303`).

The amount multiplier M is the Java binary32 `float(I_ms)/1000f` widened to double. It is not an integer division, and it is not the rounded schedule interval (`G/model/effects/AbstractEffect.java:148`). Make its exact binary rational with the integer-only binary32 adapter (§6.2), and keep the precision through the multiplication. DIFF freezes the coefficient inputs for power×M. PER stores the power and the immutable multiplier reference. At each pulse, PER reads the current HP/MP (whole+restoration fraction): current×power×M, **no /100** (`E/ticks/TickHp.java:61`, `:64`, `:68`; `E/ticks/TickMp.java:53`, `:56`, `:60`).

The negative TickHp magnitude has a cap of currentHP−1. The cap applies before the whole-damage conversion and before the player-sourced CP absorption (`E/ticks/TickHp.java:73`, `:79`). Example: HP10/CP20 and computed damage50 → capped9 → CP11/HP10. HP1 → damage0, also with CP remaining. Thus TickHp never kills. TickHpFatal is deferred. Positive TickHp/MP uses the recoverable maxima. It keeps resources that are already above the cap, through the early exit of the source. Negative TickMp drains MP to zero (`E/ticks/TickHp.java:82`, `E/ticks/TickMp.java:65`; `G/model/actor/status/CharStatus.java:166`). Restoration keeps Q fractions. Drains use whole truncation, as D-R12 says.

Death removes all effects that are not stay-after-death, in EffectId order. It cancels the cast/swing and invalidates the offers and fractional resources. Then it applies a new death penalty rank, if there is one. Class transfer filters the effects by stay-on-subclass-change before the new effective stats and resource ratios. Despawn invalidates the target bindings. Effects on other entities can keep a frozen source attribution, but they do not keep a pointer to a departed entity. A departed or dead source gets no new live kill credit. This agrees with the existing living-killer rule (`Z/state_combat.rs:637`).

### 1.4 StatSheet integration, exact Func order, cache invalidation

Extend the stat derivation in `Z/stat_sheet.rs:20`, `:52`, `:91` with the **new** `derive_effective_stats(base_context, modifier_sources, phase3_rules) -> EffectiveStats`. Keep the legacy sheet for the old policy. Store the V5 physical projection in `CombatState.sheet`. Keep these Q values in a derived Phase3 cache: magical attack/defence, regen rates, cast speed, traits, consume/reuse modifiers and the effective resource maxima. Do not apply buffs to a `profession_stats` result that is already rounded. Do these steps:

1. Make again the class/template/weapon bases that are not rounded.
2. Run the ordered functions. Keep the intermediate casts of each built-in Func before the later modifiers.
3. Do the rounding of each getter.

The existing profession growth, the Elf adjustments and the Human/Dark Elf hooks stay, one time each (`Z/phase2.rs:67`; `Z/state_combat.rs:455`, `:714`). Do not change deferred racial metadata into a second, duplicate passive.

The pinned decimal priorities are: SET0; built-in1; MUL/DIV20; ADD/SUB/SHARE30; ENCHANTHP40. `G/enums/StatFunction.java:24`, `G/model/stats/functions/FuncTemplate.java:48` and `G/model/stats/functions/formulas/FuncPAtkMod.java:38` set them. An explicit XML `order` overrides the default. `Calculator.addFunc` inserts equal-priority functions after the existing ones (`G/model/stats/Calculator.java:110`, `:117`). The canonical key is `(stat, priority, source_install_sequence, effect_xml_ordinal, function_xml_ordinal)`. The builtins install first. The initial learned passives use the `(SkillId,rank)` order. Passive and timed sources share next_install_sequence. Learning and refresh allocate new sequences. Rebuild/restore keeps them, because EffectId alone cannot put passives in order against effects.

At the default priorities, the shape is `((base after order 0/1) * mul1 * mul2 ...) + add1 - sub1 ...`. It is not `(base+adds)*product(muls)`. Put the explicit priorities in their exact positions among the default priorities. Special functions that a handler installs override the generic priorities: the pinned `E/pump/MaxHp.java:77`, `:84` installs the MaxHP add/mul at order 1. Generate their actual priority, not the generic MUL20.

For a live MaxHp, do these steps in sequence:

1. Capture currentHP and (PER only) the maxHP before installation.
2. Install its own order-1 add or multiplier.
3. Build that maximum again.
4. If heal=true, DIFF sets capturedHP+power, and PER sets capturedHP+(multiplier−1)×oldMax. The resource setters clamp the result.
5. Only then install the generic Funcs of that handler.

Old max100 at ×1.5 heals50, never75 (`E/pump/MaxHp.java:50`, `:68`, `:77`, `:83`, `:87`). A snapshot/cache rebuild and an admission reconstruction install the declarative hook-owned and generic Func descriptors in the kept order. They do not call onStart/heal/instant/RNG again. A removal removes the two ownership sets (`:98`). Calculate signed add/sub with checked i128. Never use unsigned subtraction before the final clamp. At registry load, make sure that each division is positive and that the extrema are attainable. This extends the method of `StatRules::new`/`prove_every_sheet` (`Z/stat_rules.rs:315`, `:414`).

V5 folds use Q ADD/SUB and checked-i128 MUL/DIV with an ordered round_div to Q. Quantization is different from the Java whole casts. If the oracle does not agree, use a wider local/rational calculation. Never change the expectations. Whole-number casts inside Funcs stay explicit operations: `FuncAtkEvasion` priority1 truncates its full value toward zero before MUL/ADD. Then the getter does Math.round/clamps (`G/model/stats/functions/formulas/FuncAtkEvasion.java:37`, `:46`, `:61`; `G/model/actor/stat/CharStat.java:171`). With init0/DEX20/level1: built-in27, not28. ×1.1 then +0.4 gives30.1→30. Record the two intermediate values in the provenance. The legacy `Z/stat_sheet.rs:246` stays unchanged.

The cache key is a deterministic revision tuple `(base_revision, passive_revision, effect_revision)`. The code can make the cache/index contents and the revision counters again, and they are not a separate authority. These events make an entity dirty: apply/remove/replace/expiry/dispel, learn/auto-grant, a class/level change or a death-penalty rank change. For each of these events, do these steps:

1. Mark the affected entity as dirty.
2. At the next same-phase barrier, fold the flags and modifiers one time.
3. Clamp HP/MP/CP to the reduced maxima, without proportional healing.

An increase of a maximum does not heal, except an explicit handler with a heal flag. Class transfer keeps its deliberate resource-ratio rule (`Z/state_class.rs:459`, `:467`; `Z/phase2.rs:121`). Calculate again before the next operation that reads stats in the same tick, not only at the end of the tick. Haste changes the scheduling of future swings/casts, but never existing deadlines. A speed change emits the movement state also when the destination does not change.

Each change of the effective max HP or of HP marks the public vitals as dirty. This includes max-only changes from effects, learning/auto-grants, class/level changes and the death penalty. §2 defines coalescing. A restore builds all of this again before `validate_class_entity`. Reject cached physical projections that are not consistent (`Z/state_class.rs:9`).

## 2. Commands, events, visibility and additive wire contract

Extend `ZoneCommand` and its `entity()` match (`Z/command.rs:88`, `:191`):

| New command | Payload and authoritative behavior | Transport |
|---|---|---|
| `CastSkill` | The entity that the session owns, the skill id, an optional explicit target id. The actor finds the learned rank and the target incarnation. There is no client power/rank/timing/cost. If the target is missing, the code uses the selection captured at the ordinal of this command. SELF ignores the selection. | ClientMessage `cast_skill=16` |
| `CancelCast` | The entity, and an optional cast id for protection against a stale cancel. It is idempotent when idle. It cancels Casting and does not give back MP1 or reuse. | `cancel_cast=17` |
| `AcceptResurrect` | The entity, the offer id and the `accept` boolean. Dead actors can send it. A stale id, a dead incarnation or an expired offer is rejected. | `accept_resurrect=18` |
| `LearnSkill` | The entity, the verified account, the request key, and the skill_id and exact skill_level that the request supplies and the record keeps. The reply handle stays outside the command. | additive unary RPC → actor; no ClientMessage tag |

LearnSkill names the character, `skill_id`, the exact `skill_level` and the UUID `idempotency_key`. Make a fingerprint of all semantic fields in the account/operation scope. If a key is used again with a different character/skill/rank, the result is FAILED_PRECONDITION (§5.3). `SetTarget` keeps tag 12/payload (`W:32`; `Z/command.rs:149`). V5 lets a player select other live players in AOI, with or without a flag, and eligible NPCs. A selection is not hostility. Resurrection supplies an explicit PC_BODY target. Self-attack stays invalid. All playable levels stay 1..85. Class transfers stop at tier 2.

Extend the authorization and the dead-actor exceptions (`Z/state.rs:865`, `:1035`). Learning is work for an authenticated session/account actor. It is never an untrusted System bypass. Offers are the only new intent for a dead actor. `command_from_pb` injects the session entity (`I/zone_mapping.rs:149`). Zone authorization fences the CommandSource session generation (`Z/state.rs:865`). Keep the `Ack` semantics: the code never rejects an accepted intent at a later time (`W:92`). Thus the deferred start validation makes its disposition only at the start phase. Completion failures use cancellation/result events. 

Extend `ZoneEvent`, `tick()`, `entity()` and the observer routing (`Z/command.rs:324`, `:556`, `:581`; `Z/state.rs:1322`, `:1384`) as follows. Each public lifecycle event has the tick, the involved incarnation(s) and the stable cast/effect/offer id where applicable.

| WorldEvent tag | New event | Minimum content / visibility |
|---|---|---|
| 14 | `SkillCastStarted` | caster, bound target if there is one, skill/rank, cast id, start/complete deadlines; AOI observers that can see the caster + owner. |
| 15 | `SkillCastResult` | cast/effect id, resolution kind, ordered target outcomes, miss/resist/half/crit, rolled damage, cp_damage (CP absorbed), hp_removed, heal, public HP after, applied effect ids; a projection that is safe for observers. |
| 16 | `SkillCastCancelled` | caster, cast id, reason, tick; the same audience as the start. |
| 17 | `EffectApplied` | target/optional visible source, effect id, skill/rank, pool, start/end, abnormal level, public status; observers of the target + owner. |
| 18 | `EffectRemoved` | target, effect id, reason (expiry/replaced/overflow/dispel/damage/death/transfer), tick; the same target audience. |
| 19 | `SkillReuse` | owner, skill id, ready tick; incremental zero/ready update; the admission baseline uses SkillStateSync. Owner only. |
| 20 | `SkillLearned` | owner, skill/rank, SP cost/remaining, request key if the RPC reconciliation must have it; owner only. |
| 21 | `PvpStateChanged` | player, flag deadline, karma, PK/PvP counters, tick; owner + observers that know the player, as A16 requires. |
| 22 | `ResurrectOffer` | owner target, offer id, optional visible caster, percent, recoverable XP, expiry and state (offered/declined/expired/accepted); owner only. |
| 23 | `SpGained` | owner, actual credited amount and total; owner only. |
| 24 | `SkillStateSync` | admission/reconnect baseline: learned ranks, reuse ready_tick values, SP, karma/PK/PvP, flag deadline and death-penalty level; owner only. |
| 25 | `VitalsChanged` | entity, hp, max_hp, tick, incarnation; public to the owner and AOI observers each time HP or the effective max_hp changes, with max-only modifier/progression changes included. |

The `SkillCastResult` target rows have `resolution_kind`, the rolled damage, `cp_damage` (the `cp_absorbed` domain value), `hp_removed`, the target incarnation and the tick. The public VitalsChanged=25 supplies the authoritative HP/maxHP changes from all causes, with max-only transitions included. The MP/CP totals stay in the private StatsChanged. Results can animate skill damage. But when a vitals event arrives in the same tick, clients set the absolute HP and do not subtract the damage again. Filter the AoE target rows for the AOI of each observer, in canonical order.

Owner-only events arrive also when the source is off the AOI. On each observer/owner wire projection, omit the entity/incarnation of that source, unless it is already visible/known. The full attribution stays in the server state and in private replay facts. It does not leak on the wire. The internal facts `SkillLearnCommitted`, `Phase3PersistentDelta`, `CastTransition`, `EffectTransition`, `DamageApplied` map to no public frame (`I/zone_mapping.rs:414`; `Z/command.rs:694`).

Use the requested wire reasons exactly: 13 SKILL_UNKNOWN, 14 SKILL_NOT_LEARNED, 15 SKILL_COOLDOWN, 16 INSUFFICIENT_MP, 17 INSUFFICIENT_HP, 18 ALREADY_CASTING, 19 INCAPACITATED, 20 SILENCED, 21 INVALID_TARGET_TYPE, 22 PEACE_ZONE, 23 WEAPON_CONDITION, 24 NO_RESURRECT_OFFER. The existing INVALID/DEAD_ACTOR/OUT_OF_RANGE/PROTECTED stay where applicable. A data-deferred skill uses the existing NOT_YET_IMPLEMENTED. Append separate cast-cancellation reasons. Do not give a new meaning to the existing swing reason values (`W:106`, `:229`; `I/zone_mapping.rs:487`). Learning uses typed RPC failures, as the transfer does, and a recorded domain disposition. It does not use more WS management intents.

Append `StatsChanged.karma=15`, `pk_count=16`, `pvp_count=17`, `flag_until_tick=18`. The owner fields `death_penalty_level=19` and `effective_stats=20` support authoritative predicates. Use SP=11, CP=8/maxCP=9 and tick=14 (`W:271`) again, and do not rename tick. These tags are new: **EntitySpawn.flag_until_tick=22, karma=23, active_effects=24, pending_cast=25**. These rules apply to them:

- ActiveEffect summaries include skill_id, skill_level, remaining_ticks, is_debuff, an optional caster and the stable effect identity/incarnations.
- pending_cast is SkillCastStarted.
- These tags stay additive, but identity spawns omit the fields that hold references. The hydration below delivers their baseline semantics.
- Keep the system Death Penalty modifiers out of the public effect summaries/events. death_penalty_level is only in the owner state.
- Counters/bootstrap use PvpStateChanged/SkillStateSync.

**AttackResult.cp_damage=8** reports the CP removed. In V5, `damage` reports the HP removed, as its existing wire comment specifies (`W:205`). Keep the old-policy bytes, with the legacy rolled-overkill behavior (§3). C freezes the remaining nested tags in the first additive proto commit, and Z examines it. Extend CombatView/spawn with a Phase3 projection that is absent on legacy (`Z/combat.rs:139`, `:160`; `Z/state.rs:1579`, `:1672`; `I/zone_mapping.rs:185`, `:284`, `:432`).

AOI entry uses **two phases**:

1. All identity spawns/respawns, with the final nonreferential combat/PvP values, empty active_effects and absent pending_cast.
2. Hydration through EffectApplied/SkillCastStarted with the additive `baseline=true, baseline_tick`, in subject ordinal/id order and then effect-id order.

Identity resets the effect/cast state. Hydration upserts the original IDs/deadlines without animations or transition counts. An empty/idle subject has no hydration. Thus two newcomers that cast on each other both exist before each target reference. The EntitySpawn tags24/25 keep their declared types, but they have no cross-entity references in this identity phase.

Build observer-filtered DTOs in observe, and give them to `I/zone_mapping.rs:185`. Now that function does not have the AOI/known-set context. Each nested caster/target reference must match a **visible and known incarnation after the full spawn batch**. If not, omit the optional entity and incarnation together (the legacy string is empty). Never show sentinel/dangling IDs or hidden incarnations. Keep the summary of the visible subject when its other endpoint is absent. Apply this recursively to results, offers and owner baselines. The full attribution stays private to the server. A later source visibility cannot change the effect identity.

Delivery order: response → despawn → **all identity spawns** → hydration → moves/facts (`Z/state.rs:1259`, `:1304`). Ordinary facts that change state go only to subjects that are known at the start of the tick in the same incarnation. Clients ignore ordinary deltas at or before baseline_tick. But they accept the hydration that is explicitly marked at that tick. Combat animations cannot write stale HP over a new baseline.

Mark the public vitals as dirty on a change of **HP or effective maxHP** at each stat/resource barrier. After the result/effect/progression facts of the same tick, emit one final `(hp,max_hp,tick,incarnation)` for each known subject. Do this also if the intermediate changes went back to the starting pair. New or respawned identities already have that final pair: suppress a duplicate VitalsChanged. Changes of only MP/CP stay private. Clients set the absolute HP and never subtract the damage two times. This includes the max-only apply/remove/learn/auto-grant/class/level/death-penalty changes (§1.4), and MaxHp removal without an HP clamp.

## 3. Exact V5 tick execution and edge cases

`run_tick` stays the common wrapper for draft validation/epoch (`Z/state.rs:757`, `:837`). Dispatch by the validated snapshot policy. Never dispatch on the condition “a skills registry is there by chance”. Legacy execution stays exactly as it is now: commands → spawn → AI → chase → step → impacts → note_hits → start_swings → flush_progression → observe → digest. V5 runs this schedule:

| Order | Phase | Exact work / relationship to existing code |
|---|---|---|
| 0 | Commands in receive ordinal | Authorize. Apply spawn/despawn/replacement, target/stop/move/cancel, learning/class transfer/town respawn. Stage new CastSkill/Attack and resurrection acceptance. Movement validation still comes before cancellation (`Z/state.rs:1172`, `:1193`). Capture the selection at the command ordinal. A later stop/move/cancel/target change in the same tick can cancel an earlier staged start. Keep the command order. Do not start the cancelled action again at a later time. |
| 1 | `spawn_phase` | The existing corpse decay/member respawn scheduling (`Z/state_ai.rs:71`). It initializes the sidecars on new lives. |
| 2 | Expiries | Expire the effects first, and build the stats/flags again. Prune the reuse. Expire the PvP flags and resurrection offers. Removal/expiry events come before later effects. Death-penalty passives do not expire. |
| 3 | Periodic resources | Make a snapshot of the due DoT/HoT/MP effect handler keys. Process them in the total order of §1.3, and examine again that each entry/life is still valid. Apply the nonlethal TickHp clamp and the status/resource results immediately. Then process the due regen in entity-ordinal/id order: skip dead entities, HP→MP→CP, move the deadline forward. TickHp cannot cause death. Earlier direct deaths still suppress regen. There is no backlog that uses wall time. |
| 4 | Chase and movement | The existing chase/step, with hard movement disables and peace limits (`Z/state_combat.rs:326`; `Z/state.rs:1218`). Regen uses the movement state at phase 3: an entity that still walks before this step gets moving regen, also if it arrives now. Cast range checks use the positions after the step. There is no AI target acquisition yet. |
| 5 | Cast launch/completion | First, process the due launches in `(launch_at,caster_ordinal,caster_id,cast_id)` order. Validate/filter the stored targets without a new sample of the limits, and set launched. Then process the due impacts in `(complete_at, caster_ordinal, caster_id, cast_id)` order. Validate again, pay the completion MP/HP costs, resolve the ordered targets and handlers, and apply effects/damage/death immediately. After the source finalization, finish the cast as Idle. An earlier completion can disable or kill a later caster. Update the dirty caches before the next resolution. |
| 6 | Auto-attack impacts | The existing impact order by attacker EntityId stays in this phase (`Z/state_combat.rs:392`). Examine again alive/status/target incarnation/range/peace/protection. Then roll and run the V5 damage pipeline. A cancellation uses no damage RNG. |
| 7 | Hit bookkeeping | Extend `note_hits` to use the unified landed-damage fact, not only AttackResult (`Z/state_ai.rs:195`). Skill/DoT hits reset the NPC attack timeout correctly. The public result does not cause a second count. |
| 8 | New intent starts | Resolve the staged intents in the original ordinal order against the current state. Make the final accepted/rejected dispositions. Begin the accepted casts, charge MP1, set the reuse and emit the start. Apply the accepted resurrection offers here, with a new validation. Then run `start_swings` in its existing attacker-id order, with a gate against casting/disables (`Z/state_combat.rs:928`). An accepted Attack before a Cast on the same actor can turn on the auto intent, but the cast suspends the swing. A Cast before an Attack rejects the action while the actor casts. |
| 9 | AI | Drain the ordered hate-reselection/forced-intention requests (§3). Then run `ai_phase` (`Z/state_ai.rs:174`). Suppress acquisition in peace and while actions are disabled. The existing 10-tick think stagger stays. The attack/movement that AI turns on has its physical effect in the next tick. There is no second swing-start pass or movement pass. |
| 10 | Progression/output | Flush the existing progression and the new persistent deltas (`Z/state_combat.rs:896`). Coalesce the final owner stats one time for each affected player where it is safe. Observe, move next_tick forward and calculate the digest. The actor logs and makes checkpoints before any data is visible (`Z/state.rs:784`; `A/zone_actor.rs:613`, `:624`, `:633`). |

Now, `add_hate` selects again immediately, and `ai_target_changed` can start an attack and call clan help synchronously (`Z/state_combat.rs:223`, `:247`, `:282`; `Z/state_ai.rs:216`). For V5, those hooks put `(phase, resolution_sequence, npc, incarnation, request)` in a queue. They do not start an early AI pass. These rules apply:

- Hate mutations stay immediate. Put `Reselect{target_at_mutation}` in the queue, with the unchanged most_hated.
- GetAgro applies when an attackable NPC has AI and caster != current most_hated. Then GetAgro puts `ForceAttack{caster,incarnation}` in the queue, and it does not add hate (`E/instant/GetAgro.java:49`).
- At phase9, drain the requests in emission order. Validate again the living incarnations/peace/status, and apply the intention/clan-help hooks there.
- The last valid request wins. A later hate reselection can replace GetAgro, and a later GetAgro can replace a reselection.
- Do not reduce the queue to a set of dirty ids. Do not calculate each earlier request again from the final ledger.
- A forced request wins over the ordinary think reselection in that same tick. If the last request of an NPC is ForceAttack, skip the target-selection step of that NPC.
- A later normal think or hate can select again. There is no persistent taunt lock.
- Death cleanup stays immediate, and it invalidates stale requests. 

Command staging is a scratch structure for one tick. It is not durable state. Each admitted command is in the draft/record, also if its start is rejected at a later time. Stage a logical sequence of actions for each actor. Thus `Cast,Cancel` does not become `Cancel idle,Cast`. Cancellation marks the staged cast as canceled before a cost/start. But `Cancel,Cast` starts the second action. Do not do early gameplay checks on a staged Cast that expiry/regen can make invalid. Commands with immediate guards keep the command-phase semantics. An example is a Move that the code rejects while stunned, before expiry. This asymmetry is documented and tested. Learning/transfer use their command-phase guard. A kill later in the same tick cannot make them affordable. D-R1 accepts these different command/start guards. The budget of eight commands is per session per tick in the actor. It is not a limit for all of run_tick.

A zero or small base hitTime does not mean completion before the start phase: schedule `complete_at >= start+1`. There is no recursive completion queue: “stun first” applies to casts that are already due at phase5. A new instant stun at phase 8 cannot cancel an impact that the code resolved at phase 6. A completion at t lets a later new start occur at t, if the reuse and the other guards let it. CancelCast while Idle does nothing. Define damage interruption as `start <= t < interrupt_until`. Hard disable, explicit cancel, movement, death and target invalidation do not use this interval. Use the independent cutoff from the animation in §1.2. hitCancelTime never changes it.

**Unified V5 damage contract.** `apply_damage_v5(DamageRequest) -> DamageOutcome` receives the source id/incarnation/kind, the target incarnation, the skill/effect/cast attribution, the whole rolled damage and the cause. After validation and the handler-specific TickHp clamp, do these steps in sequence:

1. Capture the PvP state of the victim.
2. Project the actual CP+HP absorption for the attack-break input.
3. For magic, run the attack-break/reflection draws that come before the reduction.
4. Go into the reduceHp status hooks of the recipient (damage-break removal, then the eligible stun roll) before the resource subtraction.
5. Build any changed modifiers again.
6. Remove CP first for Player→Player (DoTs included), then HP. Keep the rolled damage, the actual cp_absorbed/hp_removed and the overkill separately.
7. Add hate.
8. Do the first lethal transition immediately.
9. Run the post-reduction draws of the caller, if there are any (§6).

The resource projection only reads. It is not an early mutation. Only source callers that call attack-break can draw it. Thus TickHp itself has no cast-break roll. The status removal depends on the target kind/cause/awake (§1.3). It does not depend on the actual absorbed damage. HP consumption does not go through these status hooks. Paralyze never shares the stun roll. Dead targets/actors cannot act or receive more damage in the same tick. In V5, AttackResult.damage is hp_removed and cp_damage=8 is cp_absorbed (R8). Skills show all three amounts. Now, the legacy `land` emits the rolled damage also on overkill (`Z/state_combat.rs:459`, `:470`), but `W:205` says HP removed. Do not change that legacy encoding. Enforce the binding HP-removed contract only in V5. There is no late IntentRejected after an accepted cast. A resource failure emits SkillCastCancelled{RESOURCES}.

Extend `kill`, `charge_death`, `award_xp`, `relevel` through V5 policy hooks, and keep the old paths (`Z/state_combat.rs:560`, `:650`, `:696`, `:757`). Keep the guard for the first death per incarnation and the current last-living-killer attribution (`:571`, `:637`). The NPC XP/SP goes to that killer. SP is independent of the Human XP bonus. SP saturates at i64::MAX with the actual credited delta. There is no damage sharing. The source has separate XP/SP bonus paths (`G/model/actor/instance/L2PcInstance.java:4753`). This is not an instruction to apply the Human XP hook to SP. The pinned Young Keltir 20545 has SP **2** (`D/src/main/resources/data/stats/npcs/20500-20599.xml:3648`, `:3657`). Keep the tunable Nightfall XP 28 unchanged (A17′). Emit SP with XP, but do not calculate SP by a division of XP.

Healing hate uses the actual HP restored (zero for overheal): `floor(heal*100/(mob_level+7))`. It applies to each NPC ledger that has the healed target. Use a derived reverse index that the code builds again on restore. The skill effectPoint adds its absolute value one time for each applicable skill/target, not one time for each child modifier. Aggression also adds its defined flat amount (A14; `Z/combat.rs:301`, `:307`, `:344`).

PvP uses the victim state at impact time for the kill counters, before any mutation. Starts, eligible pulses and impacts are hostile acts, also if a later attack misses or is resisted. Hostile skills cancel the spawn protection of the attacker, as Attack does. The flag refresh is `max(old_deadline, now + duration)`. A hit never flags the target. There is no healer flagging, no time karma decay and no death karma decay. Monster XP decays karma. It uses the actual credited XP and the level before the award. D-R4 explicitly resolves the conflicting chaotic-victim sentence of R11: **chaotic unflagged victims give neither PK nor PvP; a flagged victim gives PvP even if chaotic**.

| Attacker before act | Neutral victim (karma=0, unflagged) | Flagged victim (karma=0) | Chaotic victim (karma>0) |
|---|---|---|---|
| Neutral | Refresh 1200; on kill PK+1 and karma gain | Refresh 600; on kill PvP+1 | Unflagged: no refresh/counter; flagged: refresh 600, PvP+1 |
| Flagged | Refresh 1200; on kill PK+1 and karma gain | Refresh 600; on kill PvP+1 | Unflagged: keep own deadline, no counter; flagged: refresh 600, PvP+1 |
| Chaotic (with or without flag) | Refresh 1200; on kill PK+1 and karma gain | Refresh 600; on kill PvP+1 | Unflagged: keep own deadline, no counter; flagged: refresh 600, PvP+1 |

Each kill uses exactly one counter branch: if the victim is flagged → PvP; else if the victim has karma=0 → PK/karma; else neither. The karma gain uses the **pk_count before the increment** of the killer. The chaotic display has precedence over a flag that runs independently. The peace and protection validation comes before this table. Damage to CP only is also hostility.

| Same-tick edge | Deterministic result |
|---|---|
| Stun expiry and cast/swing due | Expiry removes the stun first. The due action can run if it is valid in other respects. A Move refused in phase 0 is not tried again automatically. |
| Stun completion versus swing impact | The completion applies the stun and cancels the swing before impact. There are no hit/crit/spread draws for that swing. |
| Two lethal completions or two lethal swings | The serial order wins. The first death cancels the action of the later actor. There is no simultaneous direct-damage trade. |
| TickHp from a dead/departed caster | A kept effect can pulse on a living target, with frozen attribution and the HP−1 clamp. It cannot award a kill or cause a mutual DoT death. |
| TickHp at HP1, with/without CP | Capped damage0; no CP drain, no death. The player damage-break hooks still run. An NPC DoT does not remove sleep. Regen can heal later in this phase. |
| Direct lethal completion then heal/resurrection | Death is immediate. An ordinary heal cannot revive. A cast bound to a living target cannot become PC_BODY. Stale offers cannot revive a later death. |
| Offer expiry t and Accept at t | Validate again in phase 8. The expiry won, thus reject with NO_RESURRECT_OFFER. A command cannot accept in advance an offer that the code makes later in the tick. |
| Existing dead target Accept and town Respawn in one draft | An earlier valid immediate town respawn invalidates the staged offer. A staged acceptance cannot resurrect the now-living target a second time. Put lifecycle/critical commands in series, and accept a maximum of one learn mutation per character per tick (§5). |
| PvP flag deadline equals hit tick | The expiry occurs first. The victim is white at impact. The killer can get PK/karma. Classify before the refresh of the attacker. |
| Move crosses peace boundary during cast | The completion/impact uses the geometry after the step. The hostile resolution is canceled/blocked if one endpoint is protected. An existing DoT moves its schedule forward but does no hostile pulse inside peace. |
| Effect expires and is recast at same tick | The expiry event comes first. Then a new application comes later, with a new EffectId. There is no phantom final pulse. |
| MaxHP buff expires while HP exceeds new max | Clamp to max without damage/hate/death. There is no calcAtkBreak and no sleep break. |

The code finds the death penalty eligibility against the karma and position before death. The conditions are: an NPC killer, or a player killer with a chaotic victim, outside peace. Then draw `roll_below(100) < 20`, with a cap of 5. There is no draw at the cap or on an ineligible death. Deterministic seeds must include the two eligible results and these no-draw branches. Use the existing XP loss table again, unchanged. The binding rule does not include the Lucky/level<10 exemption of the source. Do not silently add one. New or leveled penalty modifiers are visible after death, also if the player stays dead. Releveling can grant existing autoget skills and tokens. Thus the full final progression/learned/effect/PvP mutation is part of the critical checkpoint.

## 4. Snapshot 9 / Record 7 / Binary V5 and old replay isolation

### 4.1 Version gate

Add `StateDigestVersion::BinaryV5`, an explicit Phase3 constructor and `phase3_enabled()`. This function comes from the validated `(schema,digest)` pair (`Z/state.rs:49`, `:380`; existing constructors `:448`, `Z/state_class.rs:78`, `:88`). Keep the legacy constructor metadata (`SnapshotMeta::default` selects V2 at `Z/state.rs:194`). New 9/V5 epochs must have the full skills/rules/class bundle.

| Snapshot / digest | Record | Accepted semantics |
|---|---|---|
| Existing valid 4/5 JSON; 6..8 JSON or V2 without classes | 3 for JSON, 4 for V2 | Only the existing validation matrix. Do not make it wider. |
| 7 / V3 with classes | 5 | Phase 2 before the token policy, unchanged. |
| 8 / V4 with classes | 6 | The current Phase 2/token behavior, unchanged. |
| 9 / V5 with classes + valid Phase3 block/hash | 7 | Only the new engine. |

Before you run a command, reject these items: 9/V4, 8/V5, 9/V2, missing Phase3 rules, unexpected nonempty sidecars in legacy snapshots, and snapshot-record mismatches. Reject Phase3 content recursively in records <7 and snapshots <=8, with nested extensions under old tags included. Field **presence** is forbidden, also for an empty/default Phase3 object.

Give the validated schema/digest context to these decoders: command, disposition, load/profile, combat/progression projection, observer output and event. Validate the raw protobuf field presence and the versioned JSON keys before permissive decoding/defaults can discard them. Keep the historical fields that the historical schema allows. This includes:

- Spawn/ReplaceSession PlayerLoad.phase3, NpcCombat.phase3 and slot profiles.
- Old Spawn/StatsChanged nested projections.
- Pending checkpoint loads/receipts/outbox.
- Nested rejection enums.

An allowlist of top-level variants is not sufficient (`R/codec.rs:1344`, `:1354`, `:1374`). Apply the same recursive checks to the encoder inputs and to restored snapshots. No hidden Phase3 data can get to a legacy serializer or digest. Do not promote an old snapshot in `from_snapshot`. Replay uses its embedded rules, not the skill/regen/PvP configuration that is loaded now (`Z/state.rs:505`, `:588`).

The legacy path does **none** of these:

- Passive execution, regen, SP awards/spend.
- New resource fractions, CP absorption.
- PvP targeting/flags/karma, peace checks.
- Cast/effects/reuse/offer scheduling, new RNG draws.
- New stats/spawn fields, a changed AI phase or new owner-baseline events.

It is not sufficient to extend shared data types with serde defaults, because the legacy JSON digest serializes entities (`Z/state.rs:813`). In legacy paths, each optional extension must be absent and skipped on serialization. V5 code must not call a changed helper that changes the old `may_attack`, stat validation, NPC selection or event projection.

### 4.2 Snapshot fields and validation

`ZoneSnapshot` gets an optional `phase3` block (`Z/state.rs:212`). It is present only in schema 9. It has:

1. The complete immutable normalized `Phase3Rules` (skills, handler/modifier definitions and order, magical/regen profiles, exact karma ratios, policies, peace geometry), the policy revision and the hashes. Examine them against the class references and the existing rules. Include the data that offline replay must have, not a network registry URL.
2. All five next-id/install counters, the entity ordinals, the casts with timing/target bindings, and the reuse maps.
3. For each active effect, the complete canonical header and the handler runtime fields: IDs, source incarnation/kind, target incarnation, skill/rank, stack/pool, applied/expires, periodic ideal-ms phase/index/typed DIFF-PER payload. Also the passive install-order metadata and the modifier installation sequence of each effect. The code builds the derived indexes again.
4. The next regen deadline and all three fractional resource remainders. The player PvP fields, the death penalty level, the death record and the resurrection offer.
5. The new NPC source profile/SP reward data, with the spawn-slot profiles. Extend `NpcCombat` with an optional Phase3 profile that is absent on legacy (`Z/combat.rs:228`; slot snapshot at `Z/state.rs:250`).
6. The pending checkpoint request additions, the exact mutation receipt/outbox payload and the stored timer remainders in checkpoint lane snapshots (`Z/state.rs:260`, `:279`; `A/checkpoint.rs:175`, `:204`). These are durability state. They are not inputs to the simulation digest.

Restore validates these items:

- Cardinality bounds.
- IDs that are unique and in increasing order, and a next-id that is greater than the existing IDs.
- Valid ranks and executable handlers.
- Counters/resources and half-open deadlines.
- Slot/stack uniqueness and one sidecar for each combatant.
- Target incarnation rules and class/learned-SP consistency.

Dead entities cannot restore Casting, an active auto intent or an offer for a living target. Work that is due at the boundary is allowed only with the documented snapshot next-tick convention, and then it runs one time. Reject impossible negative/overflow deadlines. Do not silently reset them. Calculate the flags/stats and the derived timer/reverse-hate indexes again. Compare the effective physical sheet/CP max/speed to the serialized projections. Snapshot restore is not character admission. When you replay a snapshot, do not remove short effects/reuse and do not restart regen.

Extend the compressed snapshot encoding with `NFSNAP9` next to `NFSNAP8`/`NFSNAP7`, in the existing bounded envelope (`R/record.rs:202`, `:227`, `:259`, `:267`). The envelope is:

1. Eight-byte magic (`NFSNAP9\0`).
2. Four-byte little-endian expanded JSON length.
3. The zlib payload.

There is no separate envelope hash. The zlib integrity check and the embedded rules hashes are different items. Use schema9/V5 as the only valid pair in that envelope. Keep the old magic recognition and the exact old compressed/uncompressed behavior. Validate the full registry snapshot size against the existing limits of 32 MiB decoded / approximately 1 MiB encoded. Do not truncate skill data to make it fit. A larger limit/bundle must have a separate approval. Replay never gets missing rules from a different location.

### 4.3 Digest layout

Add the **new** `phase3_state_digest` in `state_digest.rs`. Dispatch in `ZoneState::state_digest` (`Z/state.rs:804`). The recommended implementation hashes these items:

- A new domain separator (`nightfall-state-v5`).
- The unchanged V4 digest of the legacy/progression fields.
- The Phase3 rules hash, the counters and the canonical sidecar entries.

Now, V4 wraps V3 and V2, and it already hashes SP/CP/learned skills/token receipts (`Z/state_digest.rs:19`, `:26`, `:44`, `:104`). The existing `Writer` uses explicit little-endian scalars and UUID bytes (`Z/state_digest.rs:89`, `:212`, `:248`). Extend it with explicit discriminants and length prefixes. Never use the Rust memory layout or the Debug output.

Hash each mutable canonical field in §4.2 items 2–4. Also hash the NPC profile/SP attribution through the rules/profile hashes, and each per-entity override. Hash the installation sequence of permanent passives, because the order changes the results. Include these items: absent/present bits, source/target incarnations, all periodic counters, resource fractions, last-death XP, offer percent/expiry and the flag deadline. Hash the geometry and all immutable policies through their canonical rules hash.

Do not hash these items:

- Checkpoint revisions/request keys that are not related to the simulation.
- Observer known caches and reverse hate/timer indexes.
- Compiled modifier arrays and dirty sets.
- Scratch pending starts and wall-clock telemetry.

The end-of-tick staging must be empty. The code builds and validates the caches again. It does not hash them as independent authorities. A mutation to a canonical field for future behavior must cause digest divergence, also if no public event is emitted in this tick.

### 4.4 Record codec: append-only, separate tag spaces

`R/codec.rs` uses prost types that people wrote by hand. Its tags are not WorldEvent tags (`PbCommand` at `R/codec.rs:138`, `PbOutput` at `:415`). Keep all old encoders byte-exact. Set the new record schema=7 only for V5 (`R/codec.rs:593`, `:606`). Keep the decoding of 3/4/5/6 explicitly. The existing accepted-version list at `R/codec.rs:1325` includes the latest constant. Thus, if you change only that constant, you remove version 6 by accident.

Append the `PbCommand` cases 16 CastSkill, 17 CancelCast, 18 AcceptResurrect, 19 LearnSkill (the existing ChangeClass=15). New typed payloads use integer/UUID bytes or the current UUID conventions, with strict rank/boolean/enum validation. Append `PbPlayerLoad.phase3=9` after progression JSON=8/checkpoint revision=7 (`R/codec.rs:189`). Append `PbNpcCombat.phase3=6`, which has SP and the magical/regen profile, after XP=5 (`:233`). Do not renumber the existing final physical stat fields (`:209`).

Append the replay event cases 20..29 in the same logical order as the public events 14..23. These are the other cases:

- Case 30 is for a strictly versioned Phase3-private fact payload.
- Case 31 is for an allowlisted Phase3 projection payload, with spawn/stats, SkillStateSync and VitalsChanged.
- Public 24/25 do not use replay 30/31 as direct one-to-one cases (D-R13).
- Cases 18/19 stay the existing Phase2/token JSON payloads.

Strictly allowlist the variant kinds in the private payload, as the current Phase2 JSON decoder does (`R/codec.rs:1057`, `:1596`, `:1608`, `:1775`). Reject new kinds under old tags. Give the record-version context to `encode_outputs`/`decode_events` and to each nested decoder. The code validates the encoded output/event bytes recursively before replay.

The code cannot examine digest-only outputs at decode. Thus do these steps:

1. Make the output again under the validated legacy policy.
2. Before the hash, validate the output/events that you made again recursively.
3. Compare all digests.

Never read a digest as encoded fields. Observers and zone-wide events must both stay through this path. Internal transition facts cannot disappear only because the WS mapper emits nothing. Append the replay reject reason numbers from 21 upward. They map independently to the public reasons 13..24 (`R/codec.rs:767`). Do not mix the two enum numberings.

Keep the record output bounding and the digest-only comparisons (`R/record.rs:96`, `:122`, `:138`, `:151`). §4.5 makes sure that the digest-form record fits before simulation. The output fallback alone cannot put a limit on the kept commands/dispositions. The `.nfr` container stays format 1. Never make old fixtures again to make a compatibility test pass. The authoritative replay assertion stays the full bytes of the reproduced record, not only the final state digest (`docs/engineering/architecture.md:289`).

### 4.5 Deterministic actor byte-budget admission (F2)

E2 adds **actor admission before draft construction/run_tick**, after normalization/preflight, at `A/zone_actor.rs:793`. The Snapshot9 metadata records `phase3_admission_policy { version, record_budget B, max_command_bytes, max_observers }`. It is absent on legacy. The default of B is 1 MiB minus the transport reserve, and B cannot be more than the gate ceiling. Each complete normalized command has a limit of **256 KiB canonical encoded command bytes**. This includes PlayerLoad/progression/effects, NPC profiles and strings. Enforce the collection bounds that come from the registry, and bounded raw decoding. The 1,024 queued-input limit and the eight/session limit do not prove a byte bound. Do not truncate loads.

A pure **record-codec sizing helper** calculates the sum of these items:

- The exact candidate command bytes/tags/prefixes.
- The worst legal disposition for each command.
- The record framing/scalars/state/event digests.
- One entity-id/32-byte output-digest entry for each possible output recipient.

Reserve space for the current observers, the candidate spawns/replacements and any more command-response recipients. Never subtract removals in the draft. Use maximum-width varints for unknown ordinals/ticks, all nested length prefixes and bounded disposition fields (the error text stays outside them). At epoch creation, this sum must fit B: the empty framing + the configured maximum observer reserve + one maximum legal command/disposition + its possible response recipients. Enforce the configured observer cardinality during admission. Reject an invalid configuration before simulation. Full outputs can fit, but the digest form is the fallback that is sure to fit (`R/record.rs:122`, `:138`; framing `R/codec.rs:95`). No RNG, clock or simulation goes into the sizing.

Walk the receive order. Keep the session/generation and learning/lifecycle barriers. V5 stops at the first deferral for the session budget, a barrier or the byte budget. It admits the largest eligible **prefix** that fits. For later ticks, keep the suffix that it did not touch, the normalized payloads, the original receive ordinals and the reply handles. Rebase the paused admission timers only at the actual admission.

If an input is more than its own bound, or it cannot fit **alone** with the framing/disposition/observer reserve, it is the terminal `ADMISSION_TOO_LARGE`. For this input, do these steps:

1. Remove it one time.
2. Log a bounded structured failure (correlation, canonical hash, length, policy version, reason; no giant body).
3. Reject the RPC/admission with RESOURCE_EXHAUSTED, and fail that attempted session admission.
4. Also tell the system producers.

No spawn, receipt, RNG or simulation command occurs. It is an ingress failure. It is not a late IntentRejected or a permanent head-of-line deferral. Process the next head on a later selection pass.

Freeze the selected draft/record across gate retries. Never select again or run the simulation again. If the digest form is too large after the run, it is an invariant failure. Then fence, release nothing, log diagnostics and recover from the last durable boundary. Never try an impossible append again without end (`A/zone_actor.rs:613`). Recovery does these steps first:

1. It compares lost append acks against the durable log.
2. It replays the acknowledged drafts without a new partition.
3. It recovers the pending checkpoint writes before new admission.

The code can discard only the drafts that are confirmed as not logged and the volatile suffixes. Reconnect/reload and receipt-based RPC retries recover the callers. Terminal oversized requests have no success receipt, and they fail again until someone corrects them. §8 specifies the boundary, FIFO and recovery fixtures. The Snapshot9 sizing stays a separate gate.

## 5. Checkpoints, admission and LearnSkill critical mutation

### 5.1 Persistence contract

Extend `ProgressionState` and `CharacterCheckpoint` (`A/ports.rs:130`, `:157`) with an optional versioned `Phase3Checkpoint` block. The block keeps compatibility with the old serialization. Show the logical fields that follow in the repository/admission contracts:

| Field | Source / serialization rule |
|---|---|
| `sp: u64`, bounded 0..=i64::MAX | The projection of `ClassState.sp`. The checkpoint rejects a duplicated top-level value that is not equal to the nested class state. The existing learned skills stay in the class state. There is no second learned ledger. |
| `karma`, `pk_count`, `pvp_count` | The final player PvP scalars. |
| `flag_remaining_ticks` | Persist each positive remainder, with no threshold. Zero means none. Do not store the absolute tick of a different zone. |
| `reuse_remainders` | Sorted `(SkillId, remaining_ticks)`, only `remaining > 50`, not >=50. |
| `effect_remainders` | Only restorable timed effects with `remaining > 100`, not >=100. It has: source/skill/rank, pool/stack, relative install order, the periodic remaining ideal milliseconds/index/typed payload and the persistence flags. The permanent death penalty is restored from its scalar. |
| `death_penalty_level` | 0..5. Make the skill5076 modifiers again from it. |
| `last_death` | The actual loss/pre-death XP that resurrection after a reconnect must have. Do not calculate it from the loss formula of the current level. |
| `resource_fractions` | The persisted HP/MP/CP Q fractions for regen/restoration. They are clamped/cleared with the resources. |
| `modifier_install_order` | The relative installation order for learned passive and kept timed sources together. A new admission gives new global sequence numbers in that order. |
| `timer_policy_version` | An explicit paused-remainder policy. There is no elapsed-time/saved_at input to the simulation. |

Freeze the request, the timer remainders, the fingerprint, the revision and the outbox at construction. Retries never calculate the thresholds again (`A/checkpoint.rs:624`, `:638`). Add the Phase3 fields to `CharacterCheckpoint::fingerprint` only for V5 payloads. Old requests keep their exact old canonical JSON fingerprint (`A/ports.rs:184`, `:197`). Extend these items together: the bounds/invariant checks (`:212`), the checkpoint snapshot projections (`A/checkpoint.rs:187`, `:217`) and the memory/Postgres adapters.

R10 fixes **migration 000008**: the progression stays on `characters`. There is no character_progression table. The existing `characters.sp/cp` and `character_class_slots.sp` stay (migration 000007 at `apps/api/src/infrastructure/postgres/migrations/m20261009_000007_character_classes.sql:11`, `:69`). The learned ranks stay in `character_learned_skills` (`:104`). Add `characters.{karma,pk_count,pvp_count,flag_remaining_ticks,death_penalty_level,recoverable_death_xp_loss}` with zero defaults and bounds. Also add the persisted death-incarnation/pre-death-XP and the HP/MP/CP Q remainders that this contract must have.

Add `character_skill_reuse(character_id,skill_id,remaining_ticks)` and `character_effects(character_id,retained_ordinal,skill_id,level,remaining_ticks,caster_kind,...)`. The effect rows include the caster id/incarnation, the abnormal type/level, the pool, the relative modifier order, the periodic ideal-ms phase/index/typed values and the retention flags. Child rows have a character FK/cascade and deterministic keys. Replace them in the same fenced checkpoint transaction, never independently. The Death Penalty is untimed scalar state. It is not an effects row that expires.

R13 sets the SP cap at **i64::MAX**. This agrees with the signed bigint and the existing validation (`A/ports.rs:223`). Keep the uint64 wire/domain compatibility, but reject admissions that are more than the cap. A reward uses a checked wide addition, then saturates to the cap and emits the actual delta. A spend examines the balance and uses a checked subtraction. Keep the characters/active class slot projections equal. Do not widen to numeric(20,0). Store the u32 karma/counters in a nonnegative bigint with u32 bounds, and saturate at that cap. These items change together: memory/Postgres adapters, admission DTOs, fingerprints, recovery and outbox facts.

The critical triggers are:

- Death, kill, resurrection, disconnect and the 50-tick checkpoint.
- A successful learning and an SP award.
- Karma/PK/PvP/flag changes and expiry.
- Death-penalty changes and class/autoget changes.
- Changes to kept cooldowns/effects.

D-R9 requires that all newly persisted Phase3 mutations are log-acked and fenced-checkpointed before visible output. Only the existing ordinary HP/XP cadence exception stays. Do not make one exemption for all new MP/CP/fraction mutations. Changes to kept effects/reuse make the related tick critical. Coalesce all changes to one character into one final checkpoint transaction for each admitted tick. Long-reuse/buff casts are critical. Benchmark DB+log+CPU (§7).

The countdown itself is not a semantic mutation, and it does not cause a new critical write each tick. Compare the absolute deadlines in the live state. Project the remainders only when you make an actual save. Mark application/removal/refresh/expiry as mutations. When a remainder goes across a retention threshold, the next cadence/lifecycle save shows this. An actual expiration removes the stored entry through the critical removal path.

Use the **new** `Phase3PersistentDelta`/`SkillLearnCommitted` facts to let `CheckpointService::admitted` find the critical writes (`A/checkpoint.rs:285`, `:325`, `:396`, `:429`, `:469`). Include all mutation receipts and the ordered outbox facts in the same revision-fenced transaction as the final character state. Only the existing outbox relay publishes. Never publish from the domain or from the RPC (`docs/engineering/architecture.md:78`, `:558`). Stable checkpoint/outbox IDs continue to use zone/epoch/tick/ordinal (`A/checkpoint.rs:734`). Extend the critical-work detection of recovery (`A/checkpoint.rs:549`, `:606`). Thus recovery cannot discard an acked learning/karma/cooldown tick as ordinary post-checkpoint progress.

### 5.2 Admission restore order

1. Resolve the unfinished epochs before admission. `load_for_admission` supplies the committed state/fence revision (`A/ports.rs:106`; `docs/engineering/architecture.md:565`). No RPC changes the live state directly.
2. Validate the class lineage and the learned ranks against the immutable rules. Keep the `LearnedSkill { key: String, level: u32 }` persistence. Make a unique key↔SkillId index/BTreeMap from it (`apps/api/src/domain/subclass.rs:29`).
3. Build the base life/combat without the clamp that has no effects. Branch before the early clamp of `loaded_player` (`Z/state_class.rs:149`).
4. Load SP/CP one time from ClassState, the PvP/death metadata and the kept effects. Do a checked rebase of the reuse/effect/flag deadlines and the periodic **ideal-ms phase** to the actual admission tick. Allocate new EffectIds/global installation sequences in the persisted relative order. The source identity is attribution. It is not a necessary pointer.
5. Make the declarative generic/hook-owned Funcs again in installation order. Never replay onStart heals, instant handlers, success/duration RNG or costs. Find the effective stats/flags, then clamp HP/MP/CP/fractions. A dead admission has HP0, no cast/regen credit and only stay-after-death effects. Make the DeathRecord again.
6. The cast is Idle, and there is no offer. Living regen starts at admission+30. Dead schedules stay out of the queue until revival+30. Install the checkpoint lane/revision before output.
7. Emit SkillStateSync=24 and the stats/detail through the identity/hydration phases of §2 (`Z/state.rs:988`, `:1002`, `:1067`). Socket replacement keeps the live state and deadlines. It does not load the DB timers again and does not charge MP.

R4/D-R3 pause the stored timers offline. These rules apply: reuse only >50 ticks, supported buffs **and debuffs** only >100, the flag with no threshold, the death penalty untimed. Smaller entries disappear on a real logout. All snapshot timers stay exact, with no threshold. Record the normalized PlayerLoad.phase3. Replay never reads the elapsed wall time. Disconnect clears offers, but not the recoverable death XP. Persist/rebase the periodic millisecond phase together with the tick remainders.

### 5.3 LearnSkill critical lifecycle

Integration: `Z/state_class.rs:415`; `A/zone_actor.rs:363` enqueue, `:650`/`:684` preflight, `:793` barriers, `:861` replies; `A/ports.rs:82`, `:279`, `:290` receipts. Add the **new** FrozenLearnResult, SkillLearningRuntime and typed replies. These extend the transfer-only Queued (`A/zone_actor.rs:263`). Never use the default transfer-history receipt fallback.

1. Authenticate the account/character/key. Make the fingerprint `v1|character|skill_id|skill_level` under `(account_id, operation=learn_skill, idempotency_key)`. The durable receipt lookup comes **before** the online/session/gameplay guards (`docs/engineering/api-guidelines.md:11`). Match → the exact frozen success, also after progression/disconnect. Mismatch → FAILED_PRECONDITION. Only an unknown key goes into the actor draft.
2. Fence the entity/account/generation/lane. Put one learn/transfer mutation per character per tick in series, and hold the conflicting lifecycle commands. These conditions are necessary: requested rank=learned+1, class lineage, level, SP, alive, not engaged/casting, trainer radius3 at (126,128). Engagement includes incoming hate/swings/readiness and casts/hostile effects (`Z/state_class.rs:192`, `:219`, `:251`, `:266`). Validate the learning-item prerequisites in the mutation: item-required → FAILED_PRECONDITION with a reason. Unknown skills are rejected. Other deferred skills keep a correct catalog status and cannot run through learning. ListLearnableSkills only reads. It shows eligibility/reasons and the learned/affordable/trainer fields.
3. Freeze the command with the exact rank/key/account/fingerprint inputs. Exclude the DB revision, the clock and the reply handle. The domain examines the guards again in command order, debits SP, installs the learned rank/passives/maxima, and emits the owner SkillLearned/StatsChanged and the private `SkillLearnCommitted { key, fingerprint, frozen_result }`. SkillLearnDef supplies the metadata (`apps/api/src/domain/class.rs:49`). Relevel autoget stays free, and the player never also pays for it (`Z/state_combat.rs:789`).
4. Run one time. Hold the outputs/replies until the durable JetStream ack. OpenGate cannot authorize learning. A transient failure keeps the identical record/RNG and stops the next draft/snapshot (`A/zone_actor.rs:181`, `:200`, `:596`, `:613`).
5. After the log ack, do an atomic checkpoint of the **final** character SP/learned/effects/progression, the frozen receipt and the ordered outbox under the revision fence. If the DB ack is lost, try the immutable request again. A stale/permanent inconsistency fences without output, and without a rollback of logged work. Recovery replays the acknowledged draft and projects its write one time (`A/checkpoint.rs:645`, `:671`, `:678`).
6. The checkpoint ack releases the RPC/events (`A/zone_actor.rs:624`, `:633`, `:636`). Disconnect cannot cancel the commitment. The frozen result includes the skill/rank, the SP spent/remaining, the resulting stats and the tick. Never make it again from the current state.

Store the receipts in `idempotency_keys` (24-hour retention; Phase9 cleanup). Do not store them in `ClassState.successful_transfer_receipts` or in the simulation digest (`apps/api/src/domain/character_progression.rs:100`). Private facts/pending checkpoint requests keep the receipts across crashes. Replay runs the recorded drafts without receipt lookups. Examine the durable receipt again inside the fenced transaction. Suppress queued duplicate keys until settlement. Make receipts only for success (`Z/state_class.rs:428`, `:501`). A failure uses neither the key nor SP. Exact-rank retries cannot buy a later rank after cleanup.

## 6. Formula APIs, source rounding and independent oracle

Keep the legacy auto-attack combat_math unchanged (`Z/combat_math.rs:1`). The new pure `skill_math.rs` functions take explicit integer/Q contexts and injected rolls. The state layer draws through `roll_below`/`roll_permille` in canonical target/handler order (`Z/state.rs:709`, `:717`). Cast/effect ids never draw RNG. The continuous comparison of `calcSkillCrit` maps to `roll_below(100*Q)` against a Q chance. Pin this deterministic representation and the boundary vectors. The ledger is a trace of callers. It is not a set of independent probabilities. A draw is used also when the comparison is necessarily false. Unsupported feature branches are deferred. They are not approximated.

| Caller/branch | Ordered calls and draws |
|---|---|
| Target collection/start | For POINT_BLANK/RANGE, sample `{offset,width}` one time when the code collects the target set of the accepted cast. width0 returns0 with **no draw**. Else the result is offset+roll_below(width), also for width1. Zero means no source count limit, but the zone population is a limit. SINGLE does not sample (`G/model/skills/Skill.java:637`; `G/model/skills/targets/AffectScope.java:382`, `:394`, `:430`). Store the selected bindings/limit. Launch/impact only filter. The reuse mastery check and each BuffInfo duration call keep their source positions. The static/zero-stat paths of mastery draw nothing (`G/model/actor/L2Character.java:1525`; `G/model/stats/Formulas.java:1408`). If the START scope is executable, it makes BuffInfo and runs the instant handlers at start (`G/model/actor/L2Character.java:1605`). |
| Per-target activation | Canonical target order. First `calcBuffDebuffReflection`: a debuff with activateRate!=-1 draws 0..99, also at zero reflect. Else there is no draw (`G/model/skills/Skill.java:1122`; `G/model/stats/Formulas.java:1601`). The normal path calls applyEffects. The reflected path applies the continuous part to the caster, then makes the original-target BuffInfo for instant scopes (`G/model/skills/Skill.java:1127`, `:1129`). |
| applyEffects, ordinary | After the source immunity/block guards, find the continuous-container success at `G/model/skills/Skill.java:1005` (source short-circuit; one effect_success roll only when necessary). Make BuffInfo/duration **also if the continuous success is false** (`:1007`; `G/model/skills/BuffInfo.java:81`). Then GENERAL, the applicable PVE/PVP, and CHANNELING in that order, each in XML order: instant calcSuccess→onStart, continuous canStart→collect (`G/model/skills/Skill.java:930`, `:1017`). Install the successful container only after these scopes (`:1024`), and apply the §1.3 lifecycle. The CHANNELING requirements stay deferred. |
| SELF section | One time after all targets, not one time per target. Remove the old same-skill effect of the caster as the source requires. self=true applyEffects still runs the initial continuous-success expression at :1005. Then a **separate** self-continuous success at :1037, then BuffInfo duration at :1039, the SELF handlers at :1044 and the source BUFF installation predicate at :1047 (`G/model/skills/Skill.java:1144`). Thus a continuous SELF skill can use two success rolls. Do not remove the duplicate. Passive reconstruction is not this live path. |
| PhysicalAttack | calcSuccess first: magic/debuff do not use evasion and have no draw. Else draw 0..99, also at P_SKILL_EVASION0. Evaded → no onStart draws. Landed: no shield draw without a shield; criticalChance>0 → calcSkillCrit draw, then damage spread, then recipient status/stun hooks→CP/HP→death draw. Last, physical vengeance draws 0..99 for nonmagic melee-range skills, also at chance0. Magic/ranged skips it. Do not suppress the remaining draw of a caller only because its first damage killed the target (`E/instant/PhysicalAttack.java:50`, `:88`, `:100`, `:104`; `G/model/stats/Formulas.java:1388`, `:1562`). Do not invent a physical-skill attack-break call. |
| MagicalAttack / MagicalAttackRange | Magic crit (0..999)→magic success1→conditional player-caster success2 on failure→spread. Positive source damage: caller attack-break check/draw→**vengeance reflection 0..99 also at stat0**→status/stun hooks of the selected recipient→CP/HP→death draw. Nonpositive damage skips this tail (`E/instant/MagicalAttack.java:73`, `:77`, `:79`, `:85`, `:89`; `E/instant/MagicalAttackRange.java:77`, `:83`, `:89`). The outer debuff reflection and vengeance are different checks. |
| Attack-break versus cancellation | calcAtkBreak draws 0..99 when its caller gets to it and init>0. Channeling, raid, invulnerability or no casting/bow init short-circuit (`G/model/stats/Formulas.java:992`). `breakCast` examines the independent cutoff and the magic/static kind **after** that draw (`G/model/actor/L2Character.java:4191`). A window/kind failure prevents the cancellation, not the draw before it. The D-R2 absorption input uses a damage projection that only reads (§3). The resource mutation follows the caller order. Auto-attacks run status/stun→CP/HP/death **before** calcAtkBreak (`G/model/actor/L2Character.java:4080`, `:4149`). Magic does not. |
| Status / periodic | Before CP/HP: damage-break removal for the target kind/cause. Then, if still stunned and neither DoT nor HP consumption, roll_below(10)==0. Else no stun draw. TickHp goes directly into DoT reduceHp (also capped0). There is no attack-break, crit or reflection draw. An immediate death penalty draws only if eligible and below the cap (§3). |
| Heal / other | A nonstatic **magic** Heal draws calcMCrit (0..999) after MAtk/recipient HEAL_EFFECT but before the recoverable-cap clamp, also for a later zero overheal. Static/nonmagic skip it. HpPerMax/Mp/Cp and GetAgro have no intrinsic RNG. Dispels keep the source draws for each candidate. Only the actual caller of calcCrit 0..999 uses it. Never use it in place of calcSkillCrit (`G/model/stats/Formulas.java:967`, `:983`; §6.1). |

Traces record the callsite, the target/handler, the bound, the injected value, the comparison and the RNG cursor after the call. This includes no-draw exits. Use this example: a mixed A2 magic debuff with no SELF/mastery/shield and an idle target that is not stunned. Before the continuous installation, its trace is `[debuff-reflect, continuous-success, duration(no draw), magic-crit, success1, (success2 if required), spread, attack-break(no draw), vengeance-reflect, status(no draw)]`. Add separate traces for: SELF double success, a stunned recipient, an interrupted/out-of-window caster, physical zero vengeance and stronger-stack suppression.

An invalid or canceled resolution uses no new handler draws. The start/target-selection draws that were already used stay used. IDs never draw. Supported natural-end (`removed=false`) END handlers use BuffInfo again after removal, and keep their instant-handler draws. Other removals skip END (`G/model/CharEffectList.java:555`). Reconstruction never runs END.

Use Q=1,000,000, signed i64 stored values and checked i128 intermediate values. `floor_div`, `ceil_div`, `round_div`, `isqrt` already exist (`Z/scaled.rs:13`, `:125`, `:133`, `:140`, `:149`). Add the explicit **new** `trunc_div` for the Java `(int)`/`(long)` conversion. It truncates toward zero, and this is different from floor for negative modifiers. Java Math.round is floor(x+0.5), negative ties included. Do not use one rounding rule everywhere.

Some exact decimal source values have more than six places (for example, the karma multiplier 0.772184315). The code cannot load them by truncation to Q without an explicit rule. Keep their literal rational numerator/denominator in the immutable source data. Use a checked rational calculation at the formula boundary. Round the resulting runtime quantity to Q only where the specification says so. Validate the denominator/product bounds or cross-cancel. An arbitrary pow(13,85) must not overflow i128.

### 6.1 Function contracts

| New pure function | Required computation and rounding |
|---|---|
| `cast_time` / `reuse_delay` | The pinned calcCastTime starts at hitTime. It scales nonstatic magic by 333/MAtkSpd and physical by 333/PAtkSpd. Static is not scaled. Shots are absent. Floor to500 ms only if the computed time<500 AND the base hitTime>500 (`G/model/stats/Formulas.java:1027`). Use the animation/launch/impact/interrupt times that §1.2 stores separately. Do not convert the animation directly to completion. beginCast uses the raw reuseDelay if locked/static. Else it uses trunc(base·MAGIC_REUSE_RATE), trunc(base·P_REUSE) or DANCE_REUSE for that source branch. Only delay>10 ms disables, anchored at start (`G/model/actor/L2Character.java:1514`, `:1542`). Unsupported mastery stays deferred. coolTime/hitCancelTime are imported but not used. There is no speed-scaled hitCancelTime window and no post-cast cool lock. |
| `physical_skill_evasion` / `physical_skill_damage` | calcPhysicalSkillEvasion tests P_SKILL_EVASION, default 0. It never uses calcHitMiss (`G/model/stats/Formulas.java:1388`). The damage uses 77·(power+PAtk·shot)/PDef, front facing, shot=1/no shield, the PvP defence/base multiplier, then trait→attribute→spread→NPC penalty→PHYSICAL_SKILL_POWER, min 1 (`:746`, `:774`, `:801`). PhysicalAttack calls with crit=false. Then it doubles the damage on its separate STR·criticalChance calcSkillCrit success (`E/instant/PhysicalAttack.java:88`, `:92`). Truncate only at the final whole-damage boundary. |
| `magic_damage` | 91·sqrt(MAtk·shot)/MDef·power; shot=1. Draw the magic critical first, as the pinned handler does. Then draw the magic-success roll. On failure for a player caster, draw a second success roll. Give half only if it succeeds **and** target level−caster level <=9. Else set the intermediate damage to 1. There is no crit multiplier on failure. On success, the crit multiplier is 3 PvE or 5/2 player-v-player. Then apply spread, the PvP scalar, attribute and the applicable NPC penalty in source order. Truncate only at the whole-damage boundary. Keep the intermediate precision (§6.2). Source: `G/model/stats/Formulas.java:815`, `:845`, `:848`, `:871`; `E/instant/MagicalAttack.java:73`. |
| `magic_success_rate` | d=target level−(magicLvl>0 ? magicLvl : caster level); rate=100−Math.round((float)((13/10)^d·NPCchancePenalty·magicSuccessRes)). The explicit binary32 cast before the rounding is important (`G/model/stats/Formulas.java:1286`, `:1301`). Compare the integer draw 0..99 < rate. A rate out of range naturally gives always/never. There is no arbitrary 1..99 clamp. |
| `effect_success` | Reject a blocked target early. activateRate=-1 or basicProperty=NONE succeeds without a draw. For a missing magic level, use the exact source condition `<= -1`, not <=0. The base is ((magicLvl−targetLvl+3)·lvlBonusRate+activateRate+30−targetBaseStat). Multiply by element, general trait, magic sqrt(MAtk)/MDef·11 if magic, and 1+calcStat(BUFF/DEBUFF_VULN, **1**)/100. Thus the default is 1.01, not 1.00. Trait immunity gives 0 before the min clamp. Else the skill min/max default is 1/99. Compare the Q-valued rate > integer roll·Q, and do not round it to a whole percent (`G/model/stats/Formulas.java:1158`, `:1178`, `:1195`, `:1224`). |
| `effect_duration_ticks` | Passive permanence is explicit, with no unsigned encoding of -1. The active buff duration comes from the source seconds. For a debuff, multiply trait·(1+lvlBonusRate/100)·(1+(effectiveMagicLvl−targetLvl)/100)·attribute / saving-stat bonus. Clamp to [base/2,base], **ceil to whole seconds**, then multiply by 10. Do not use an early tick-ceil in place of the second-ceil. Mastery doubling applies only if it is supported. Else the skills that must have it are deferred (`G/model/stats/Formulas.java:1144`, `:1761`, `:1781`). |
| `attack_break_chance` | 15+sqrt(13·damage)−(MENbonus·100−100), ATTACK_CANCEL funcs, clamp 1..99. Compare the integer draw < the fractional rate (`G/model/stats/Formulas.java:992`). The D-R2 input is the actual positive CP+HP absorbed from a hostile source, without overkill. Only living magic/static casters in start <= t < interrupt_until can be canceled. The cutoff comes from the original animation (§1.2). The source breakCast lets magical/static casts be canceled (`G/model/actor/L2Character.java:4191`). Raid/invulnerable/no-init paths use no draw. A casting target outside its abort window still uses the draw of calcAtkBreak before breakCast refuses the cancellation (§6 ledger). Hard-disable/move/death cancellation does not use this roll. |
| `attribute_bonus` | The exact piecewise H5 function. Element NONE or a weapon/skill mismatch returns 1 immediately. Auto with no attribute=1. attack<=defence=1. The coefficient pairs for the source brackets are: (0.06909,0.078), (0.0887,0.1007), (0.129,0.1473), (0.25,0.2894), (0.4,0.55). Calculate (attack+100)^2/144·a − (defence+100)^2/169·b. Clamp by the diff ranges [>=300: −50..100; >=150: −50..70; >=−150: −50..40; >=−300: −60..40; else −80..40]. Divide by100 and add 1. The player-v-player lower bound is 1. Encode each attack/defence coefficient branch, with the attack<−99 tail. Do not reduce it to a table of differences only (`G/model/stats/Formulas.java:1440`, `:1463`, `:1519`, `:1542`). |
| `karma_gain` | PK before the increment: k<99 → trunc((k/2+1)·720); 99<=k<180 → trunc((k/8+37.75)·720); >=180 →43200. Only the player branch. Boundary vectors 0,1,98,99,179,180. Do not copy the summon branch. Pets are excluded (`G/model/stats/Formulas.java:1830`). |
| `karma_lost` | Positive XP first goes through the configured karma-loss-rate division, with Java compound-assignment truncation. Then trunc(abs(XP)/exact level multiplier/30). The defaults come from the source. They are not the rounded two-decimal examples. Clamp the removal to the current karma. Use the actual monster XP credited and the level before the award, before level advancement. There is no automatic time decay (`G/model/stats/Formulas.java:1814`). |
| `regen_amounts` | The player base HP/MP/CP regen comes from the per-level template. Multiply by the level mod, and by CON for HP/CP and MEN for MP. Clamp init to at least1 **before** the regen-stat functions. Then apply the config multiplier and the posture (standing11/10, moving-running7/10). There is no sitting and no Mother Tree/estate/siege bonus. NPC HP/MP use the base NPC regen without the player CON/MEN/level/posture multipliers. CP is only for players. Add Q amounts to the fractional-resource accumulator, emit whole changes and keep the remainder. Keep the fractions after the formula (the NPC base MP0.9 first clamps to1 under the source formula) (`G/model/stats/Formulas.java:263`, `:334`, `:349`, `:357`, `:434`, `:442`; `G/model/actor/status/PcStatus.java:308`). |
| `resurrect_restore_percent` / `resurrect_xp` | If the base is 0 or 100, return it. Else r=base·WITbonus; **if r−base>20, r+=20**; then r=max(r,base), r=min(r,90), exactly (`G/model/stats/Formulas.java:1372`). Never use min(base+20,r) in its place. The percent stays Q. refund=Math.round(actual recoverable_death_xp_loss·r/100), with a cap of the actual loss/pre-death XP. Use the loss one time (`G/model/actor/instance/L2PcInstance.java:4700`). Oracle examples: base30·WIT2 →60→80; base20·WIT2 →40 (exactly20, no addition); base50·WIT2 →120→90; base0/100 bypass. Also include a reduced actual loss at the XP floor. |

Source fidelity: weapon element None/0 makes the attribute return **1.0** early, also for a WIND20 skill. Keep the extra magical-crit scaling: FuncMAtkCritical multiplies by10 when equipped. The getter truncates, then multiplies by10 again and caps at200 (`G/model/stats/functions/formulas/FuncMAtkCritical.java:42`; `G/model/actor/stat/CharStat.java:270`). BUFF/DEBUFF_VULN starts at1. Physical evasion and resurrection now agree with the pinned source, with no alternative “corrected” expected vectors. Only the offline-timer pause and the 600-tick offer lifetime use explicit deviation vectors (D-R6). The accepted Q restoration/whole-damage representation gets its own adaptation coverage.

Verification of the interruption input: the pinned auto-attack (after reduceHp) and MagicalAttack (before reduceHp) give their computed damage variable to calcAtkBreak (`G/model/actor/L2Character.java:4149`; `E/instant/MagicalAttack.java:79`). The player CP absorption occurs in reduceHp (`G/model/actor/status/PcStatus.java:185`). Thus the argument is not HP-only. The binding D-R2 whole-resource contract uses the actual absorbed CP+HP. For magic, the code projects it before the reduction. For auto-attacks, the code captures it after. Do not describe the raw source overkill as measured absorption. HitCancelTime has no getter callers (`G/model/skills/Skill.java:629`). The source sets the interrupt cutoff at `G/model/actor/L2Character.java:1508` and examines it at `:3222`.

MP1/MP2 use their separate source consume-stat paths and `(int)` truncation (`G/model/actor/stat/CharStat.java:482`, `:509`). Preflight requires MP >= MP1+MP2 (`G/model/actor/L2Character.java:1664`). The start uses MP1 and the reuse. The completion calculates and examines MP2 again (`:4617`). If MP is not sufficient, emit SkillCastCancelled{RESOURCES}. Neither MP1 nor the reuse is given back, and the unpaid MP2 is never “refunded”. An HP cost cannot kill the caster: HP>cost is necessary, and the charge occurs on completion. Dispel uses the newest-first source order, random draws for each effect and the protected flags. Keep the integer compound truncation in the cancel rate before the comparison (`G/model/stats/Formulas.java:1733`, `:1747`).

**Restoration handlers (F12).** Let curR be the whole resource + its Q restoration fraction. Let maxR be the effective maximum. Let recR be the MAX_RECOVERABLE_* getter, which the code folds and truncates separately (`G/model/actor/stat/CharStat.java:206`, `:218`, `:226`). Find the source branch/cap before the conversion to Q. Add positive restoration to the fraction accumulator, move whole units, keep the remainder, and clear it at a full clamp. The public `heal` is the actual whole HP gained, not the requested power. The `(int)amount` of packets is not a reason to discard the simulation restoration fractions. Signed drains truncate toward zero one time at the whole-drain boundary and clamp to zero. They keep the fractional credit, unless a full clamp clears it (D-R12). The setter caps stay the effective maxima. No handler gets the guards or crit behavior of a different handler.

| Handler | Executable contract (E/ paths use the pinned datapack handlers) |
|---|---|
| Heal | Skip a null/dead/door/invulnerable target. The start amount=power. There are no shots with the nongrade Squire's Sword of Phase3: staticShotBonus=0, mAtkMul is initially1, the weapon-grade branch leaves1, then the no-BSS branch adds1 → **2**. No weapon/no shot also gives2. Nonstatic: amount=power+sqrt(2×caster.MAtk(caster,null)), fold the recipient HEAL_EFFECT, then the magic-only calcMCrit and ×3 on success. Static does not use sqrt, HEAL_EFFECT or crit. Last, amount=max(0,min(amount,recHP−curHP)). The crit draw comes before this clamp, also for overheal. Shots/high-grade gear stay outside Phase3. `E/instant/Heal.java:64`, `:68`, `:84`, `:89`, `:92`, `:102`. |
| HpPerMax | Skip null/dead/door, but **not** invulnerability. Integer power: at100, amount=maxHP. Else maxHP×power/100. Clamp max(0,min(amount,recHP−curHP)). There is no MAtk/HEAL_EFFECT/crit. `E/instant/HpPerMax.java:40`, `:56`, `:62`, `:66`. |
| Mp | Skip null/dead/door/invulnerable/MP-blocked. A negative DIFF uses the amount directly. A nonnegative DIFF folds MANA_CHARGE only when nonstatic, then min(result,recMP−curMP). A negative PER uses curMP×amount/100. A nonnegative PER uses min(maxMP×amount/100,recMP−curMP), without MANA_CHARGE. Keep a negative cap result when MP is already above the recoverable max: a positive grant can become a drain. A nonnegative value restores. A negative value reduces MP, to a minimum of zero. There is no crit. `E/instant/Mp.java:57`, `:63`, `:68`, `:76`, `:86`, `:101`. |
| Cp | Only a living player. DIFF=min(amount,recCP−curCP), negative values included. A negative PER=curCP×amount/100, else min(maxCP×amount/100,recCP−curCP). Set curCP+delta with the effective CP clamp. There is no MANA_CHARGE, no invulnerability/MP-block guard and no crit. `E/instant/Cp.java:61`, `:67`, `:71`, `:81`; `G/model/actor/status/PcStatus.java:263`. |

Timing vectors are offsets from the start in ticks (the animation is after calcCastTime; `—` means an empty abort window):

| Animation A ms | Kind | Launch | Impact/complete | Interrupt cutoff |
|---|---|---:|---:|---:|
| 0 | ordinary / continuous | 0 | 1 | — |
| 0.5 | ordinary / continuous (task skillTime truncates0) | 0 | 1 | — |
| 100 | ordinary / continuous | 0 | 4 | — |
| 500 | ordinary | 1 | 5 | 3 |
| 500 | continuous | 0 | 4 | 3 |
| 550 | ordinary | 2 | 6 | 3 |
| 550 | continuous | 0 | 4 | 3 |
| 1200 | ordinary | 8 | 12 | 10 |
| 1200 | continuous | 5 | 9 | 10 (cast already idle at9) |

Test the cutoff t−1/t/t+1 independently of the impact. A cancel after the launch must cancel its pending impact. Ceil each absolute offset one time. Positive short casts with a nonzero truncated animation keep the 400-ms impact stage of the source. Validate the speed/static/floor branches separately from this schedule (`G/model/stats/Formulas.java:1027`; `G/model/actor/L2Character.java:1614`, `:1643`, `:4578`).

### 6.2 Sqrt/rounding oracle contract

The proposed runtime sqrt policy: for a nonnegative Q value x, `sqrt_q(x)=isqrt(x*Q)` is a **lower bound**. It is not automatically an approved whole-damage result. Keep the upper bound `(sqrt_q+1)/Q` through the remaining positive rational pipeline. If the two endpoints give the same final integer, that integer is proven. If they are on the two sides of an integer or of a chance comparison boundary, refine the result. Use checked i128 interval arithmetic, or a cross-multiplied squared comparison with the factors canceled before the multiplication. Do not use unchecked large products.

For admitted bounded stats, select and prove a maximum refinement precision at load time. If the code cannot prove a result, fail the registry validation. Do not let a live spell use an arbitrary guess. Store only Q state. Wider precision is local math. A12 permits one Q-sqrt only after the independent oracle agrees on the necessary vectors. That is not exhaustive equivalence.

The independent Fraction/Decimal oracle of E1 brackets irrational roots with integer inequalities over pinned literals. A pinned Java harness can add to the **future** verification. Oracle artifacts must have these items:

- The source revision/path/line and the inputs.
- The explicit injected rolls and the intermediate exact quantities.
- Each rounding site.
- The expected whole outputs/branch outcome and the vector-set hash.

Rust cannot make its own expected values. The golden vectors include:

- Each executable skill/rank and the class/level boundaries.
- Buffs in the two application orders, max caps and negative add/sub.
- Perfect/nonperfect squares, near-integer damage and near-half rounding.
- PvE/PvP crit, the two failure rolls and the >=10 level-gap branch.
- Multi-step regen/periodic fractional accumulation, the binary32 ticks multiplier, PER dynamic reads and all the timing vectors above.

For `Math.round((float)x)` and the float division of AbstractEffect, make an integer-only binary32 rounding adapter (nearest, ties-to-even significand, then Math.round). Or supply a proven bounded exact equivalent from the oracle classification for all supported inputs. A `round_div` of a Q-truncated 1.3 power alone does not include the explicit float cast. If it helps, generate bounded d=-84..84 power data from exact rationals. Make sure that probability saturation lets you use safe early bounds. Do not try 13^84 in i128. There are no floating-point tokens/types in the production zone code.

The generated provenance must have this rounding ledger:

* Accuracy getter: Math.round. Evasion built-in priority1: `(int)` truncation **before** later MUL/ADD, then getter Math.round/cap (§1.4). Physical attack speed: Math.round. The current legacy implementations at `Z/stat_sheet.rs:232`, `:246`, `:266` stay frozen for the old policy.
* Maximum HP/MP/CP and base integer stats: `(int)` truncation. Magical speed: truncate after its cap. Magic crit: truncate before the ×10 of the getter, then cap. MAtk/MDef are Q quantities, not integer getters (`G/model/actor/stat/CharStat.java:207`, `:237`, `:252`, `:270`, `:284`).
* MP costs/reuse milliseconds: `(int)` truncation after the modifiers. The cast animation milliseconds follow the source cast before the conversion to ceil ticks. If you apply the tick ceil first, that is a different rule (`G/model/actor/L2Character.java:1508`, `:1518`, `:1586`).
* Magic success: explicit float conversion, then Math.round. Effect/attack-break chance: fractional comparison, **no** whole-percent rounding. Debuff duration: Math.ceil seconds. Damage/packet amount: `(int)` truncation only at the final boundary. The existing Nightfall whole-HP combat stays. It does not change without notice to the full fractional damage state of Java.
* Karma gain/lost: truncation. Death loss and resurrected XP: Math.round. Regeneration: keep Q fractions, with no whole truncation for each pulse. Attribute bonus: no intermediate whole-percent casts.

## 7. Cost model and budgets

These are engineering estimates only. There are **no measurements**. For N=1000, E≈8000 at eight effects/entity, or <=44000 normal slots (24+12+8), plus bounded system sources. The derived BTreeSet schedules for expiry/pulses/regen/casts use canonical total orders. The code builds them again from the authoritative deadlines, validates EffectId/version, and removes replaced entries without a collection of tombstones. Cost: O((due handlers+casts+expired+regen) log scheduled + targets + dirty modifiers). There is no N² scan. Staggered regen is approximately 33/tick on average. But synchronized spawns cause 1000-entity bursts every30 ticks. Benchmark the two conditions.

Use compact ordered handler/Func Vecs and BTree* maps, compiled stat caches and dirty-only rebuilds. Do not fold 44 effects for each attack, and do not use JSON for each pulse. Use AOI candidate queries, then canonical ordinal/id sorting. Use the CellFacts routing again (`Z/state.rs:1384`, `:1390`), and use the reverse hate indexes for heal targeting. Validate ranges/intervals at load.

affectLimit is **{offset,width}**. width0→0/no draw (no source count limit). Else offset+roll_below(width), with a maximum of offset+width−1. `5-12`→5..16, `10-10`→10..19 (`G/model/skills/Skill.java:637`; `D/src/main/resources/data/stats/skills/00000-00099.xml:339`). Use checked arithmetic. Put a limit on all target sets by the configured population. There are no rerolls when there are fewer targets.

Indicative canonical bytes: idle sidecar120–220/entity, active cast100–180 plus the selected bindings, reuse12–24/entry, effect120–220 plus handlers. For 1000 entities/eight effects/~four reuses, this is ~1.2–2.2 MiB of extra hash input. For a full44 effects, it is ~6–11 MiB, without the variable cast bindings. JSON and the embedded registry add more. The digest output stays32 bytes. V5 streams SHA and uses buffers again, and does not make repeated large clones. The legacy layout does not change (`Z/state_digest.rs:44`, `:89`). Share the immutable Arc rules, and measure the actor snapshot/checkpoint cloning (`A/zone_actor.rs:624`).

CPU targets, not proven: no-effect NPC scheduling/regen overhead <=0.3ms mean; eight-thousand-effect scheduling/cache/digest <=1–3ms. Measure these items:

- 0/8/44 effects and synchronized pulses/regen.
- Full-AOI hydration.
- Critical writes caused by MP/CP/fractions, and serial checkpoints.
- Snapshot cloning/encoding and the §4.5 admission budgeting.

Coalesce the owner stats/public vitals. Do not send a large quantity of periodic countdown messages. Output digests put a limit only on the output volume.

A20 acceptance: 1k NPC zone_step mean<2ms; 1k players/observers mean<10ms; combat p99<20ms in an eight-client packaged soak with casting/long effects. Measure these items separately: simulation, digest, output encoding, JetStream and DB, with the elapsed actor work (`A/zone_actor.rs:903`). Kept Phase3 changes stay critical: one final fenced write/character/tick, immutable retries, correct indexes/batching. Each durability exception must have the approval of the architect after measurements. There are no dropped pulses, no hits in a changed order and no sampled logs. Telemetry labels are bounded outcome/effect/phase/reason kinds, never ids (A21).

## 8. Verification plan (to implement later; not executed)

### 8.1 Unit/property/oracle coverage

* Pure formulas: the independent vector manifest from §6 and the full attribute branch boundaries. Also: an explicit float-round oracle, signed truncation versus floor, negative ties, safe product bounds and min/max invariants. Property tests: monotonic base damage where clamping does not occur, conservation `damage = cp_removed + hp_removed + overkill`, no underflow, no regen above maxima or for dead entities, and deterministic fractions. Heal power10/MAtk50 gives20 (static10, magic crit60 before the cap). Independent HpPerMax/Mp/Cp guard/negative/cap vectors. affectLimit vectors: width0/no draw; width1/one draw; 5-12 rolls0/11→5/16; 10-10 rolls0/9→10/19.
* Cast machine: authorization/stale generation, exact learned rank, each rejection/cancel reason, the MP1/MP2 split, HP cost and castRange/effectRange boundaries. Also: zero cast min-one-tick, reuse boundary t−1/t/t+1, the independent interrupt cutoff from the animation, and each ordinary/continuous short/long vector in §6.1. Also: mid-launch snapshot/cancel, coolTime/hitCancelTime that are imported but not used, mutable target selection, replacement session, and source/target death/despawn/incarnation changes. There are no new handler draws for an invalid/canceled resolution. Keep the earlier selection draws. Make sure of the exact second magic-failure, zero-reflection, physical-vengeance and SELF sequences with the injected-roll callsite/cursor traces from §6. Include the out-of-window attack-break draws.
* Effects: all pools at capacity and shrink; abnormal equal/higher/lower; NONE same/different skill. Also: effect container versus child count; explicit priority/tie install order; expiry and pulse in the same tick. Also: player versus NPC sleep/damage-break removal for direct/DoT/HP-consumption and zero/CP-only damage, and stun rolls0/1/9 in the exact position. Also: overlapping status providers; death/transfer retention; dispel newest/protected/rate truncation; trait immunity; TargetMe deferred with a named blocker. Make sure that the cached values agree with a cold calculation after each transition. MaxHp oldMax100×1.5 heals50 only one time across snapshot/cache/admission restore. Evasion27 before/after MUL/ADD. Ticks1/3 binary32 multipliers, no cadence drift, expiry equality, PER current-resource reads and the TickHp HP1/CP clamp.
* Tick goldens: each edge in §3, with shuffled entity UUIDs, thus the cast ordinal order is different from the auto attacker-id order. Also: saved/restored mid-cast/mid-DoT/mid-regen fraction. Also, the request order at the AI phase for: GetAgro→hate, hate→GetAgro, two GetAgro requests, a most-hated no-op and an invalid/dead-target request. Also: SP+karma+level/token changes in one kill. Assert the event order and the post-tick state, not only the final HP.
* Snapshot/replay: the legal version matrix and each illegal cross-pair. Negative nested injections are rejected under records3–6/snapshots4–8 for: Phase3 absent-vs-empty fields in old Spawn/ReplaceSession loads, NPC/slot profiles, progression/combat projections, old event tags/Phase2 JSON, dispositions and checkpoint/outbox children. The raw protobuf/JSON presence stays through validation, and digest-only reproduction cannot emit Phase3. A malformed deadline/ID/rank/hash/stack/pool/profile is refused. A mutation of a new field changes the V5 digest. A mutation of a derived index cannot change the canonical behavior after the rebuild. V1–V4 digests/records/WS frames stay byte-identical to the checked-in fixtures. Do not update their expected bytes.
* Persistence/actor: a paused JetStream ack emits no success/output, and a paused checkpoint also does not. DB commit+lost ack tries the frozen receipt again. A stale/permanent fence stops the output. A restart after the log ack and before the DB writes learns/spends one time. There is no double SP award or PK count on recovery. Also: operation-key conflict; an old successful receipt retry after more learning/class change; concurrent duplicate queued keys. Also: i64::MAX SP saturation and spend boundaries; 50/51 and 100/101 timer thresholds; periodic phase restore; replacement versus real logout. Budget fixtures: the 256-KiB normalized input edge, B−1/B/B+1 full digest records, the worst disposition/observer reserve, and 1,024 large pending loads with the eight/session limit. Also: FIFO suffix retention, oversized head rejection, a gate retry without a new run, a crash before/after the durable append and a lost terminal reply. There is no retry loop for an oversized record after the run.
* Visibility/proto: exact new tags and independent replay tag numbers. Old messages encode unchanged. Private SP/MP/offer/learning never leaks. AoE result filtering. Observer entry during cast/buff/regen. Owner reconnect rebuild. New spawns that cast on each other show before the hydration references. Off-AOI sources/targets are absent recursively. An empty baseline reset and same-tick hydration/lifecycle deltas cannot apply two times. Max-only HP changes from each §1.4 transition emit the final public vitals after the result rows, with respawn suppression included. Client predicates use only authoritative fields (A22).

Keep the future tests together with the current domain/actor/codec tests. Add the new `state_skill_tests.rs`, `state_effects_tests.rs`, `state_pvp_tests.rs` and formula-golden fixtures. `docs/engineering/architecture.md:395` describes the legacy fixture contract. `R/record.rs:151` implements the record comparison. Take new snapshots at arbitrary admitted boundaries. The suffix replay must agree with the run that had no interruption, with checkpoint-pending recovery snapshots included.

### 8.2 Recordings, scenarios and coverage schema

Add a V5 `.nfr` fixture with two players and NPCs. It must include:

- Cast success/reject/cancel.
- The physical-skill zero-evasion roll, and the injected nonzero-evasion oracle branch.
- Magic success/half/full resist and buff replace/expire.
- Nonlethal TickHp at HP1 with/without CP, and the player/NPC damage-break contrast.
- Fractional regen and long reuse/effect reconnect.
- The LearnSkill critical write/retry.
- Flag expiry/CP/PK/karma/SP and the death penalty.
- Accepted/declined/expired resurrection.

Keep the existing fixtures untouched. Run the future A22 scenario family (`3-cast-skill`, `3-cast-rejected`, `3-learn-skill`, `3-learn-rejected`, `3-buff-expires`, `3-debuff-status`, `3-regen`, observer/reconnect variants, `3-death-penalty`, `3-resurrect-a/b`, `3-pvp-flag-a/b`, `3-pvp-karma-decay`). Use only guarded **pre-admission** fixture seeds. The soak includes 8 real clients that cast skills, and long effects. There is no in-zone test mutation command.

Extend `Coverage` and its Cartesian-product `table` (`apps/api/src/bin/replay/coverage.rs:20`, `:35`, `:64`) from the current schema2 to **schema3**. Keep all NPC/attack/class-transfer fields. Count the transitions one time from the zone facts, not one time for each observer. New arrays:

| Table/state pair | Reachable rule / evidence |
|---|---|
| cast Idle→Casting | Only an accepted start, not only an RPC/WS request. |
| cast Casting→Idle | Cancel, completion, death/invalid target. A reason histogram shows the different paths. |
| all cast self-pairs | Not reachable as transitions. If you want, a repeated idle cancel can be a no-op counter. |
| effect Absent→Active | A new EffectId is installed. |
| effect Active→Removed | Expired/replaced/overflow/dispel/damage/death/transfer; reason histogram. |
| effect Removed→Active | Not reachable for one EffectId. A replacement installs a new ID. |
| remaining effect Cartesian pairs | Not reachable. Do not label an active periodic pulse as a lifecycle transition. |
| effect_pulses / stack_rejections | Separate bounded kind/reason counts. They include the silent suppression by a stronger stack and the nonlethal TickHp clamp. There is no admitted lethal-DoT transition. |

Also add: learning command outcomes versus committed learn effects, offer state counts, CP absorption, PvP/karma transitions, regen pulses and life-resurrection counts. Dead is an entity life state. Recovering is not added, because the pinned path does not use coolTime. Optional cast-request coverage can keep the idle/pending/accepted semantics in a separate table. Never mix up “command Ack” with a successful spell effect. Initial snapshots are baselines, not synthetic transition counts. Digest-only records get facts by replay. You must make sure of them before you trust the coverage. This keeps the existing interpretation (`docs/engineering/architecture.md:473`, `:491`). Coverage merges must have equal schema and reachability metadata, and they calculate covered/total again. Do not invent a coverage threshold. If one byte of a new event changes, replay must fail, also if a final state digest agrees by chance.

## 9. Closed dispositions and implementation gates

Architecture-decisions §6–§8 closes all review points. The v3 table maps F1–F16. F3/F4 replace the completion-based cutoff of D-R2. The other binding timer/PvP/offer/formula/capability/wire/durability/fraction/codec decisions stay. No architect decision is open.

Before the fixture/schema freeze, E2 measures **compressed Snapshot9 with the full immutable registry and 1,000 entities with full buff lists**. The limits are ~1 MiB encoded/32 MiB decoded (§4.2). Do not truncate fields. Share the immutable rules. A larger limit/bundle must have a separate approval, only if the measurements show that it is necessary. Benchmark DB+log+CPU before the soak. Kept Phase3 mutations stay critical, with one fenced write/character/tick. Neither measurement gate ran here.

Independent handoffs (R15):

- The first additive proto/receipt contract of C → Z examines it.
- The E1 generated definitions/oracle of D → Z/C examine them.
- The E2 runtime/codec isolation of Z → C examines it before dependents.
- The E3/E4 learning/persistence/PvP/death of C → Z examines them.
- The E5/E6 client/harness of C → Z examines them.
- The docs of D → Codex examines them.

Split the E2 replay/runtime restore from the E2 admission persistence integration. Thus the second uses the E3 adapters without a circular prerequisite. Z keeps the ownership of shared zone/digest/codec code. C integrates isolated modules against the published hooks.

The downstream plan must use `## 4. Epics, stories, models and Done When`, separate `[client-visible] [required]` tags, the `# covers: 3 E2.1` grammar and an explicit story → scenario file → authoritative assertion matrix (R14). Keep the learn → cast → MP/reuse sequence of the first scenario and the exact legacy fixtures. This artifact does not claim builds, tests, benchmarks or implementation changes.
