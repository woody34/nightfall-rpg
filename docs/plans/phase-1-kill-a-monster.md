# Phase 1 Plan: Kill a Monster

**Status:** COMPLETE 2026-10-09. **Status (original):** DRAFT 2026-10-08. **Planning model:** Codex; each story gives its implementation model and effort.

## 1. Goal

This is the smallest Lineage 2 loop. A player targets and auto-attacks one NPC type, takes damage,
kills it, gets XP, and levels up. The player can die, and then respawns to fight again. Release it in the
256×256 flat test zone that exists, with the login, WebSocket, AOI, SeaORM and replay stack that exist.
Acceptance is one playable loop and a byte-identical headless replay, which includes a restore in the middle of a fight.

## 2. Decisions

| # | Decision | Choice | Rejected | Why |
|---|----------|--------|----------|-----|
| D1 | Scope | One level-1 melee monster template, one fixed starter weapon stat block; two instances are permitted to show social aggro | Skills, loot or class progression | This is the smallest complete combat loop |
| D2 | Formula authority | High Five formulas/tables from the planning references, transcribed with no change into TOML in `packages/data/`. The physical damage coefficient is the value that the pinned L2J High Five `Formulas.java` uses for a normal attack (E1.1 resolved it: **76**; 77 is the crit-additive and skill constant). The attack interval is **500000 / pAtkSpd** (HF), not the Interlude 470000 | Interlude XP/death brackets, simplified linear stats, invented balancing formulas | The phase decision of the user supersedes older decisions that do not agree with it. The list below gives the source conflicts |
| D3 | Arithmetic | Only integer/fixed-point in `domain/zone`; decimal data is compiled before admission, with a fixed rounding contract (§3.1) | Runtime floating point, damage that the client calculates | The same snapshot, inputs and seed give an identical state and identical bytes |
| D4 | NPC AI | L2J intention subset: Idle, Active, Attack, ReturnHome, Dead; aggro range, social aggro, hate list, leash, seeded respawn jitter | Behaviour trees, timers outside the actor, UE navigation as authority | The tick controls it, and you can examine and replay it |
| D5 | Durability | All new simulation state is in `ZoneState` and `ZoneSnapshot`, and all new facts are in `AppliedTick`; keep draft → run → durable ack → release | Combat side effects before the durable gate, replay from the session audit | It extends the current actor without a second simulation |
| D6 | Persistence/events | SeaORM transaction for XP/level/HP and the related state/outbox; periodic and logout saves; `CharacterLeveled` / `CharacterDied` through the outbox relay that exists | DB writes for each hit, direct NATS publishing from the domain | Atomic aggregate updates, bounded persistence lag |
| D7 | Art | Free Fab/Quixel/Game Animation Sample assets; exactly **one owner step**, a bundled download through the Epic launcher/Fab site; the agent does the import/retarget/wiring | Custom art production, repeated manual editor setup | A monster that you can see, with idle/walk/attack/death animations, at a low production cost |

**Source precedence and fidelity gate.** Read [stats](../planning/01-stat-formulas.md) §§2–3,
[combat](../planning/03-combat-and-skills.md) §§2.1, 2.3–2.4, 2.7–2.8, and
[world](../planning/06-world-and-content.md) §§2.4–2.5. These references do not all agree:

- Stats §3 selects the Interlude curves, examples, XP and death brackets. This phase selects the
  **HF** curves in §2.1, the HF XP column in §2.6 (through 85), and the HF death percentages in §2.7.
- Combat §2.1 gives **76** for normal attacks, and 77 is in other locations. E1.1 resolves the
  coefficient from the pinned L2J HF source and records it (no architect override).
  Stats §2.4/§3.3 gives **470000** (Interlude). D2 selects the **500000** of combat §2.4 (HF).
- Accuracy/evasion do not agree (`sqrt(DEX) * 5` in stats, but `* 6` in combat). Use the detailed
  stat derivations in stats §2.4, which include its listed high-level accuracy additions and the evasion
  addition above 69. Record that precedence. Do not silently mix the two variants.
- The full HF starting profiles/bonus lookup rows and the level-86 XP sentinel are not fully given.
  E1.1 must complete them from the primary L2J source that the docs cite, pin its revision,
  and document the missing rows. Do not relabel Interlude profiles as HF, and do not invent missing values.
- **Resolved by E1.1** against the pinned L2J HF (`l2j-server-game@abfde049`,
  `l2j-server-datapack@3ca488dd`; ledger and errata in [SOURCES](../../packages/data/SOURCES.md)):
  K = **76**. The interval is 500000. Accuracy/evasion use `sqrt(DEX) * 6` with the level
  additions of the source (this supersedes the stats §2.4 precedence above). The HF stat bonuses are the
  `statBonus.xml` tables (the `1.009^(s−49)` curves and STR 88 in the docs are Ertheia data, E-1).
  The "HF" XP column of the docs is also Ertheia (E-2). The weapon stats replace the fist values (E-5).
- Keep the literal source formulas and table values together with the scaled data, with the document/section
  and upstream revision provenance. The one explicit selection (500000, not 470000) keeps its label as an HF-against-Interlude selection.
  There is no undocumented tuning and no runtime interpolation. Worked-example rows that do not agree get explicit
  errata and a rational calculation. Never make tolerances wider only to make a test pass.

## 3. Architecture and deterministic contract

Keep the inward dependencies of the [architecture](../engineering/architecture.md): TOML loading in
infrastructure, immutable validated rules injected through the application bootstrap, pure stat/combat/AI
calculations in `apps/api/src/domain/zone`, and wire conversion in `interface::zone_mapping`.
Extend `packages/proto/nightfall/v1/world.proto` first, and make the Rust/TurboLink bindings again.
The current `Fixed(i32)` means milli-tiles, `Speed` means milli-tiles/tick, `Tick` is 100 ms,
and AOI is 3×3 cells of 32 tiles. Keep those units and the `ChaCha12` key/stream/word-position state.

### 3.1 Formula representation, scaling and examples

`Q = 1_000_000`. Bonuses, coefficients and derived fractional stats use signed `i64` Q units.
Base attributes/levels, live HP/MP, damage, hate and XP are whole integers (XP `u64`). Use checked
`i128/u128` intermediate values. Reject data that is more than the proven bounds. Never wrap. Decimal TOML literals
are strings or explicit scaled integers, which are parsed exactly without binary floating point. Store the formula
IDs, the literal source expressions and the constants in `tables/formulas.toml`. A typed evaluator implements
only the specified operations. Static curves become checked-in lookup tables, not runtime `pow`.
The NPC template combat stats are final values. Do not apply player class bonuses to them a second time.
The selected level-1 monster does not start the HF NPC damage penalty for level 78+ (stats §2.9).

Let `F` mean mathematical floor, `C` ceiling, and `R(x)=F(x+1/2)` the source
`Math.round` rule (ties go in the direction of positive infinity). Signed divisions implement floor explicitly.
Evaluate each expression below as one rational product before its stated final division. Do not
truncate bonuses, P.Atk or P.Def to whole units before damage. Resource maxima floor to whole units.
UI rounding never goes back into the simulation. `isqrt(n)` is the floor integer square root.

| Calculation | Data and exact evaluation/rounding |
|-------------|------------------------------------|
| Six stat bonuses | HF `statBonus.xml` 2-decimal table values for indices 0–99 (exact at Q), with their generating comments (`1.036^(STR−34.845)` …) kept as provenance. The generator derives each row again from its comment in high-precision decimal. It doubles the precision until the rounding agrees, and records disagreements as errata. Lookups outside 0–99 are errors. (The previous `1.009^(s−49)` set is Ertheia, errata E-1.) |
| Level modifier | `LM_Q = (L+89)*10000`, the exact representation of `(L+89)/100`. |
| P.Atk / P.Def | `A_Q = F(atk_Q*STR_Q*LM_Q/Q²)`. Here `atk` is the P.Atk of the weapon when the character holds one (the L2J `<set>` replaces the fist value, E-5); if not, it is the class fist P.Atk. `D_Q = F(baseUnarmouredDef_Q*LM_Q/Q)`. There are no occupied armour slots in this phase. |
| HP / MP | `n=L−1`; class table values from stats §2.5: `H_Q=baseHP_Q+aHP_Q*n+bHP_Q*n²`, and `M_Q` in the same way; `maxHP=F(H_Q*CON_Q/Q²)`, `maxMP=F(M_Q*MEN_Q/Q²)`. Reconcile the expanded rows with the source tables. |
| Accuracy / evasion | `root_Q=isqrt(DEX*Q²)`; `acc_Q=Q*R((6*root_Q+L*Q+accAdd_Q[L]+weaponAccuracy_Q)/Q)`; `eva_Q=Q*min(250,R((6*root_Q+L*Q+evaAdd_Q[L])/Q))`. Round each getter before the hit calculation (E1.4). The independent bounded-root oracle proves that the Q root truncation does not change a rounded result for DEX 0–99. The HF additions are exact entries for each level: accuracy `+(L−69)` above 69 and `+(L−76)` above 77; evasion `+(L−69)` from 70, ×1.2 from 78 (E-3). |
| Hit | The flat/front condition multiplier is Q. `chance‰=clamp(800+20*(R(acc_Q/Q)−R(eva_Q/Q)),200,980)`. This also rounds the final NPC accuracy/evasion inputs, as the source getters do. Keep the comparator of the docs: **hit iff roll ≤ chance**, with a uniform roll `0..999`. Thus a nominal “80%” is 801/1000 outcomes. Test this boundary explicitly. Do not quietly change `>=` to `>`. |
| Physical crit | `crit‰=min(500,F(base*DEX_Q*10/Q))`, base = HF fist 4 or the value of the weapon (Squire's Sword 8); crit iff roll `< crit‰`; multiplier 2, additive crit modifiers 0. |
| Physical damage | `max(1,F(K*A_Q*criticalFactor*(100+j)/(D_Q*100)))` with `K` = **76** (resolved, `calcPhysDam`). The integer `j` is uniform in `[-r,r]`, from the random-damage radius of the weapon (Squire's Sword 10; bare hands `5+isqrt(L)`). This is the documented physical pipeline with the constant of D2. Soulshots, position, traits, attributes, PvP and buffs are neutral (1 or 0). No skills. |
| Attack timing | `speed_Q=Q*min(1500,R(baseAtkSpeed*DEX_Q/Q))`. The base is the weapon speed when the character holds a weapon (379); if not, it is the fist (300). It is always more than zero. Follow the HF casts: `interval_ms=F(500000*Q/speed_Q)`, `impact_ms=F(interval_ms/2)`. Then the impact occurs after `max(1,C(impact_ms/100))` ticks, and the next cycle after `max(1,C(interval_ms/100))` ticks. E1.4 replaces the earlier exact-fraction schedule, because it had a difference of up to one tick. |
| Hate | On landed damage `d`, add `F(d*100/(npcLevel+7))`, and cap the total at `999999999`. Record the actual damage separately. Auto/social aggro adds 1 hate. |
| XP / death | HF cumulative XP `X[L]` in `experience.toml` (L1–85 plus the sentinel `X[86]`; XP has a cap at `X[86]−1`), rate 1 and the template reward, no Nightfall level-gap XP multiplier. `loss=R((X[L+1]−X[L])*loss_Q[L]/Q)`, then `xp=max(0,xp−loss)`. Find the level with a threshold search. The HF loss is 10% at L1, −0.125 percentage points for each level through L49, 4% through L75, 2.5/2/1.5% at L76/77/78, and 1% at L79–85. Store each row; for example, the L2 fraction is 98750/Q. |
| Respawn | The NPC delay in integer ticks: `deathTick+10*(delaySeconds+uniform(0..randomSeconds))`, with inclusive random endpoints. Player town respawn: `HP=max(1,F(maxHP*65/100))`, MP 0, protection `PlayerSpawnProtection` = 600 **s** = 6000 ticks (E-6). Attack stops the protection. |

Worked vectors (the tests must assert the integer encodings and also the displayed values):

1. `LM(20)=1.09` of the docs → `1090000`; no-armour P.Def 80 → `87200000` (87.2).
   HF Human Fighter STR 40 gives `1200000`; Squire's Sword P.Atk 6 at L1:
   `F(6000000*1200000*900000/10¹²)=6480000` (bare hands 4 → `4320000`).
   Only legacy arithmetic (Ertheia STR 88, E-1): `F(12000000*1418259*900000/10¹²)=15317197`.
2. Raw L40 HP/MP of the Human Fighter in the docs: `80+11.765*39+0.065*39²=637.700`,
   `30+5.430*39+0.030*39²=287.400`. With the supplied fixture bonuses CON=1.57, MEN=1.28,
   the maxima are `floor(1001.189)=1001` HP and `floor(367.872)=367` MP.
   Those bonuses test the arithmetic. The HF table has CON 43 = 1.58 (E-4), which gives 1007 HP.
3. Accuracy 40 against evasion 35 → 900‰; the rolls 900/901 hit/miss. The differences −31/+10
   clamp to 200/980‰. The HF DEX 30 bonus 1.10 gives a fist crit of 44‰: the rolls 43/44 crit/normal;
   with Squire's Sword (base 8) 88‰. HF Human Fighter L1 accuracy
   `Q*R((6*5477225+1Q)/Q)=34000000` (34).
   The Interlude base-44 example of stats §3.3 floors to 48‰ instead. Test it only as a legacy vector with a label.
4. K=76 (resolved): P.Atk 100, P.Def 50, spread 0: `76*100/50=152`; crit = 304. Spread −10 gives
   `floor(136.8)=136`. A miss = 0 damage, with no crit/spread draw. Damage 152 to an L20 mob adds `floor(152*100/27)=562` hate.
5. Speed 300: raw interval 1666⅔ ms → source interval 1666 ms, half 833 ms,
   impact tick +9, next swing +17. Speed 1500: interval 333 ms, half 166 ms, impact +2,
   next swing +4. Squire's Sword at DEX 30: `R(379*1.10)=417`, interval 1199 ms,
   half 599 ms, impact +6, next swing +12. Bare hands DEX 17: speed 294, interval
   1700 ms → cycle +17 (the superseded exact-fraction contract gave +18).
6. The HF thresholds of the docs `X[1]=0, X[2]=68, X[3]=363`: XP 60 + reward 10 → 70, level 2.
   Death at L2 loses `R((363−68)*0.09875)=29`, and XP 41 and level 1 stay.
   At maxHP 126, the town respawn restores 81 HP. XP stops at `X[86]−1 = 16890558727` (level 85).
   The L86 sentinel gives the death-loss span of L85 (`R(3710077625*0.01)=37100776`). It is not a level.

E1.4 fidelity review: The independent rational oracle and the measured discrepancy ledger are in
[SOURCES.md §E1.4](../../packages/data/SOURCES.md#e14-independent-fidelity-review).
The former fractional hit calculation had a difference of **19‰**. The floored death loss had a difference of
**1 XP**, and it could prevent delevelling (L5, XP 3183: old 2884/L5, source 2883/L4).
Both agree with the source rounding now. HP/MP keep the exact decimal source rows under D3. Across the extended attribute grid, the Java
binary32 storage has a difference of a maximum of **1 resource unit**.
For all nine unmodified profiles at all 85 levels, the difference is **0**.

### 3.2 Commands, state, AI and replay

- Add the session intents `SetTarget` (nullable target), `Attack` (enable/disable), and `Respawn`.
  The session identity gives the actor. There are no client damage/stat inputs. Validate the alive state, the target
  existence/life incarnation, the attackability, the range/AOI, and the session generation. An accepted Attack
  chases into melee range, then repeats. A duplicate enable must not reset the cooldown or make a swing.
  MoveTo cancels auto-attack. If the target is cleared or changed, the pending swing is cancelled. Players cannot attack players.
- Add the proto combat events `AttackResult`, `EntityDied`, `EntityRespawned`, `StatsChanged`, `XpGained`,
  `LevelUp`. They carry the tick, a stable event index, the entity IDs and the life incarnation. AttackResult has miss/crit,
  damage and remaining HP. Add the target/attack-cycle state (start/impact/ready ticks) to the authoritative
  state updates, so that animations can wind up before impact. EntitySpawn includes the template, life state,
  public HP/maxHP/level and the current cycle for AOI entry. MP/XP/private stats are only for the owner.
- Snapshot these items: base/derived stats, HP/MP, XP/level, target, auto-attack flag, pending swing/target incarnation,
  cooldowns, protection, death/corpse/respawn deadlines, NPC template/spawn slot/home, intention,
  hate/damage maps, last-hit/think/wander timers, path destination, event counters and pending save state.
  Store the immutable resolved rules/templates in the snapshot. Hash **all** the dependent TOML, not only the zone
  bounds. Restore never reads the current data. Increase the snapshot/log schema versions, and reject unsupported versions.
- Tick order: admitted inputs by ordinal → due respawns → AI think/commands → movement → due impacts
  by attacker ID → immediate damage/death/XP/level results → start of the eligible next cycles → AOI
  outputs/checkpoint facts. If an earlier impact kills an attacker, its later impact is cancelled.
  Generated AI commands have a separate ordered field in AppliedTick. Replay makes them again and compares
  them. It never sends them back as external inputs, and it never does them two times.
- Each new world/internal event is in AppliedTick and in its durable record codec, not only in the observer outputs.
  This includes off-AOI AI changes and persistence facts. Also record a canonical end-of-tick state
  digest, so that invisible state drift fails the replay. Keep the response-first order, then the
  sorted AOI despawns/spawns/moves, then the combat facts in causal event-index order. A spawn comes before all
  facts that refer to a newly visible entity. Emit combat facts between entities only when the two entities are known.
  The owner StatsChanged still delivers the damage/death state when the attacker is outside AOI.
- The NPC thinks every 10 ticks. Idle wakes when players are near. Active scans the aggro range and seeds hate.
  If not, it rolls 1-in-30 to wander. Attack selects the most hated, chases and swings. For a tie, it keeps the
  current target, then the lowest EntityId. A clan call gives 1 hate to idle/active allies that are near, one time for each
  engagement. Process the allies in ID order, without recursive calls. If no players are near, this must not stop
  a return, a corpse expiry or a respawn timer that is already scheduled.
- ReturnHome occurs on a leash breach, on an unreachable/invalid target, or after 1200 ticks without a successful hit.
  It clears the hate/cycle, walks to home, restores the full NPC HP there, and then goes to Active. Returning NPCs cannot
  get damage or aggro again. Dead clears the movement/target/hate, and keeps its corpse until the corpse deadline.
  Then it hides the corpse. Its spawn slot/timer stays. Respawn restores the template stats with a new life
  incarnation. The same one NPC type can have two spawn slots for the social-aggro tests.
- Flat-zone pathing uses the `Vec2Fixed::step_toward` that exists, squared integer range checks and stable
  tie rules. There are no obstacles, no actor collision avoidance and no asynchronous nav queries. Data ranges use
  explicit milli-tiles. Never use L2 world units as tiles implicitly. Enforce the bounds and the leash.
- The RNG order is fixed: due spawn slots in slot-ID order, AI by entity ID, impacts by attacker ID.
  At a valid impact, draw hit, then (only on hit) crit and spread. Invalid/cancelled swings draw nothing.
  Draw the respawn jitter one time in the death results. A zero spread/jitter radius uses no draw.
  The wander direction/distance and the respawn jitter use specified inclusive integer ranges and unbiased
  rejection sampling from the ChaCha12 stream that exists. In tests, snapshot at each timer boundary.

### 3.3 Progression and persistence boundary

One kill gives the full XP of the template one time, to the eligible live player that lands the killing blow.
Cancelled or unvalidated swings in the same tick get no credit, also if they did damage before.
There is no party or shared reward system. Process more than one
threshold in order. Emit LevelUp for each level that the player gets, and a final StatsChanged. Also calculate the stats again on
level loss (StatsChanged carries the old/new level). Keep the current HP/MP, clamped to the new maxima.
Death sets HP to zero, cancels combat, charges the XP loss one time, and keeps the dead state through reconnect.
Respawn is an explicit request from a dead player to the safe point of the zone. There is no resurrection skill and no XP refund.

Emit a full persistence checkpoint every 50 ticks (5 s), on a death/level change, and before the logout
despawn. Include XP, level, HP, MP, alive/dead state, position, character revision and pending events.
The application worker stores the checkpoint and the outbox entries in **one SeaORM transaction**. Then it
returns a recorded system acknowledgement. Pending checkpoints/events stay in ZoneState/Snapshot
until they are acknowledged. Logout/admission waits for the final save. Stale generations/revisions cannot
overwrite a replacement. The retry keys are `(zone, epoch, character, checkpoint_revision)` with a
payload hash. An identical retry does nothing. A changed payload is a conflict. Event IDs are deterministic.
There is no I/O in the domain, and no new rule that uses the DB completion time. After a crash, recover the acknowledged
checkpoint facts from the durable applied-log prefix before the admission of the character. This projection
does not bypass the current refusal of incomplete epochs by the replay verifier. Ordinary unsaved progress
can roll back by a maximum of 5 s. You must recover the death/level checkpoints, which include their outbox facts.

## 4. Epics, stories, tasks

Models: **Sonnet medium** for mechanical work, **Opus high** for work with much design,
**Codex high/medium** for integration, review and docs, **agy medium** for diagrams that are only docs.
Each Done when includes tests or document validation. Completion also must have the applicable
[API guidelines §4](../engineering/api-guidelines.md#4-required-tests-per-endpoint) matrix:

- **U** = unit happy path, each typed error, validation with no state/event/RNG change.
- **W** = real-socket happy path/all fields, each documented error, authentication/ownership, retry,
  read-after-write.
- **A** = repository round trip, typed constraints, rollback and concurrent retries.

For WS mutations, keep the §3b monotonic seq (a repeated seq closes 4400). Test repeated intents with a new seq
for semantic idempotence. Keyed save/RPC mutations test the same-key replay and the changed-body conflict.
If there is no endpoint, W does not apply. But this is not a reason to omit its domain/adapter/UE tests.

> [!NOTE]
> The `[client-visible]` tag identifies a contract and state behavior that the client can see. The simulation traceability checker examines completed tagged stories against the scenario coverage (`.nfs`). Parts that are only for rendering keep their automation and cook validation. But these stories keep their tags, and the checker reports the cases that have no coverage. New plans must add tags independently before you write scenarios.

### E1: Stat engine as data

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 1.1 Source and TOML tables | Resolve the §2 fidelity ledger; add `packages/data/tables/{stat_bonus,formulas,experience,penalties}.toml`, the HF starter profiles/HP/MP tables in `classes/`, and a fixed starter weapon block; keep the literals, source revisions and the decimal table generator | Opus high | The golden transcriptions agree with each selected printed value; the source supports the missing HF rows and the L86 sentinel; discrepancy tests identify incompatible Interlude/rounded examples explicitly |
| ✅ 1.2 Validated loading | Extend the infrastructure data loader: exact decimal parsing, table coverage, monotone XP, nonzero defence/speed, coefficient bounds, IDs across files and a canonical config hash; inject the resolved rules | Sonnet medium | U: malformed decimals, holes, overflow, bad references and invalid caps stop the startup; identical files give identical rules/hash; one changed coefficient changes the hash |
| ✅ 1.3 Pure domain stat calculation | Implement §3.1 in domain/zone with typed scales, integer sqrt, caps, XP lookup/loss; extend the no-float/source scans to each new module | Opus high | U: each worked vector agrees with the exact scaled values; generated legal inputs satisfy the resource bounds, monotone XP/level, capped hit/crit/speed and overflow safety; the clippy/source scans pass |
| ✅ 1.4 Formula fidelity review | Add property tests against the worked examples of the docs and an independent rational oracle, for all supported levels/profiles; compare legacy vectors only with labelled legacy inputs; examine the errata | Codex high | U: the golden and property suites pass with documented display rounding; no unexplained mismatches, no copied implementation oracle, no substituted Interlude production table and no hidden runtime float |

### E2: Combat core in the zone actor

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 2.1 Contract and intent plumbing [client-visible] | Add Attack/SetTarget/Respawn, six necessary combat events and the cycle/state fields; append proto tags/reasons, make the bindings again; map the session identity and generation | Codex high | U+W: encode/decode of all fields, malformed/unknown/dead/nonattackable/out-of-AOI targets, ownership/stale session, auth, seq/rate limits, repeated intents and applied-tick Ack/rejection pass; no old tag changes |
| ✅ 2.2 Complete combat state and log [client-visible] | Extend Entity/ZoneState/ZoneSnapshot, the SpawnPlayer loaded state, the AppliedTick events/generated commands/state digest and the durable codec; immutable data bundle and schema versions; keep the AOI order/privacy | Opus high | U: a mid-swing snapshot round trip is equal to uninterrupted ticks, also the RNG/bytes; off-AOI facts stay; AOI entry and replacement reconstruct the combat state; an unsupported schema is rejected; a gate failure releases nothing |
| ✅ 2.3 Auto-attack and damage [client-visible] | Target/enable/disable/chase, integer impact/ready deadlines, range check again at impact, miss/crit/spread, damage and cycle events for player and NPC | Opus high | U+W: the §3.1 vectors, a repeated Attack without an extra swing, cancellation, target replacement, movement out of reach, simultaneous lethal impacts and seeded hit/crit boundaries pass; the client cannot supply damage |
| ✅ 2.4 Death and player respawn [client-visible] | One-shot death results, HP zero, cycle cancellation, H5 XP loss/delevel, dead-state admission, safe-point Respawn and protection | Opus high | U+W: duplicate lethal/respawn attempts cannot charge two times or revive living actors; dead movement/attack is rejected; 65% HP/0 MP/protection expiry and early Attack cancellation are tested; reconnect keeps the death |
| ✅ 2.5 Aggro and hate core [client-visible] | Ordered hate/damage ledger, H5 damage hate/cap, target eligibility, retain-current/ID ties, forget on death/despawn; give E3 access to deterministic aggression commands/events | Opus high | U: arithmetic/cap/tie vectors, two attackers, disconnect/death cleanup and identical-command/seed properties pass; rejected attacks add no hate |
| ✅ 2.6 XP and level transition [client-visible] | Kill credit exactly one time, H5 threshold search/cap, LevelUp/XpGained/StatsChanged, HP/MP clamp on recalculation, checkpoint facts | Sonnet medium | U+W: the XP 60→70→41 example, a multi-level reward, capped XP, and attackers in the same tick where only the landed killing blow gets credit; a repeated death gives no second reward; owner-only XP and read-after-write state are asserted |

### E3: NPC data and AI

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 3.1 Templates and spawn data [client-visible] | Add `packages/data/npcs/*.toml`: one attackable melee type, level/base stats, XP, movement/range/collision, clan/help/aggro/leash, corpse/respawn fields; extend `zones/test_zone.toml` with spawn slots that refer to templates, count/home and a player safe point | Sonnet medium | U: references/units/bounds/delays validate; the old Gatekeeper/Wanderer stay noncombat fixtures or are explicitly migrated; the 256×256 zone boots; a second monster type is not necessary |
| ✅ 3.2 Intention state machine [client-visible] | Implement the §3.2 Idle/Active/Attack/ReturnHome/Dead transitions and events, the 10-tick think, auto/social aggro and the 1-in-30 wander; use the E2 hate and attacks | Opus high | U: the transition table covers each guard; same-clan help, no help for out-of-range/non-clan, no recursive help, dead target and empty-region timers pass with fixed ticks/seed |
| ✅ 3.3 Deterministic chase and leash [client-visible] | Use the integer flat-zone movement again for chase/wander/return; bound the destinations, attack range and home radius; no-hit timeout, exact arrival, full HP on return | Opus high | U: diagonal/edge/zero-speed cases, tie order, the 1200-tick timeout and the leash reset pass; a mid-path snapshot restores identical events/bytes; a property test proves progress without overshoot |
| ✅ 3.4 Corpse and respawn scheduler [client-visible] | Stable spawn-slot ownership; death → corpse expiry → delay plus seeded random delay → new life; keep the slot when the corpse leaves AOI | Sonnet medium | U: min/max jitter, zero jitter, corpse before respawn, never a double spawn, same-seed sequences, rejection of a stale-incarnation target and a snapshot during Dead all pass |

### E4: Persistence and events

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 4.1 Character repository transaction | Migrate the XP/level/HP and the MP/life/revision state, generated SeaORM entities; extend the memory/Postgres ports for an atomic checkpoint+outbox and idempotency | Sonnet medium | U+A: round trip, valid XP/level/HP constraints, no partial update/outbox on failure, concurrent identical retries exactly one time, a conflicting body/stale revision is rejected; the entity regeneration is clean |
| ✅ 4.2 Checkpoint and admission integration [client-visible] | Five-second tick checkpoint, death/level checkpoint, logout final save before despawn, recorded save ack; admission loads the committed state; recover the durable checkpoint prefix after a crash | Codex high | U+A+W: periodic/logout/reconnect keep the fields; a delayed DB and a generation replacement cannot overwrite a newer state; a crash before/after the transaction and the log ack recovers the checkpoint facts; a failed save is kept/retried |
| ✅ 4.3 Domain events through outbox | Map the deterministic facts to CharacterLeveled/CharacterDied, an ordered aggregate sequence and stable IDs; stage them with the checkpoint, use the relay again on `nightfall.character.leveled` / `.died` | Codex high | U+A: one logical event for each transition, more than one level-up kept, a failed commit publishes none; a crash after commit/before publish and a duplicate delivery prove retry/dedupe; a death delevel is reported consistently |

### E5: Client

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 5.1 Asset acquisition and import | Prepare one manifest of Fab/Quixel/Game Animation Sample assets that are free now: IDs/URLs, version, licence/attribution, skeleton and all four necessary clips. **One owner action: download the complete manifest in one Epic launcher/Fab-site session to the agreed staging folder.** Then script the import/migrate, retarget, materials, animation assets and proxy wiring; keep vendor content outside the Input/Blueprints/UI/Maps that the script makes again | Codex high | The manifest/licence validation and the scripted repeat-import test pass; the editor asset-load/cook tests find mesh+idle/walk/attack/death; the owner does no manual import/retarget. Verify the clip coverage before you request the download; if Game Animation Sample does not have attack/death, select a free compatible pack |
| ✅ 5.2 Typed combat projection and HUD [client-visible] | Extend the generated-code bridge, NetClient, WorldProxySubsystem and OwnEntityComponent; target frame, player/NPC HP bars, XP/level display and authoritative target clearing | Sonnet medium | UE automation tests send recorded events and assert each field, owner privacy, AOI entry/exit, stale life/generation rejection and reconnect state; no local damage calculation |
| ✅ 5.3 Click attack and damage numbers [client-visible] | An NPC raycast sends SetTarget, then Attack; a ground click keeps MoveTo/cancel; render miss/crit/damage one time for each event; show the rejection and the pending state | Sonnet medium | UE automation and real-server W: an NPC click attacks, a ground click moves, a repeated click cannot make attacks faster, a rejected/dead target clears correctly, a replayed event cannot duplicate a number |
| ✅ 5.4 Animated proxies and death UI [client-visible] | NPC idle/walk/attack/death from the selected pack/Game Animation Sample, cycle timing from the server, frozen corpse then removal; dead-player overlay and Respawn button; target/HP reset after respawn | Codex high | UE automation covers the life/animation/UI transitions; a two-client scripted playtest completes kill→XP→level and death→respawn, which includes late AOI entry and 150 ms interpolation; cooked assets load with no manual editor wiring |

### E6: Replay, telemetry, docs

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| ✅ 6.1 Fight recording and CI | Extend the two-player recording scenario with target/chase, hit/miss/crit, social aggro/leash, NPC death/respawn, XP/level, player death/delevel/respawn and disconnect; write the versioned `apps/api/fixtures/sessions/two-players-fight-v2.nfr`; update the replay-check and codec fixtures | Codex high | The byte/digest replay passes from the epoch and from a mid-fight snapshot; the old movement coverage stays under an explicit schema policy; a changed coefficient, RNG draw, AI event or off-AOI state causes a failure; gaps/an incomplete watermark are still refused |
| ✅ 6.2 Combat telemetry | Add bounded-label counters for combat event/hit/miss/crit/death/respawn/level, AI intention counts, combat tick duration, save lag/failures; use only admitted events, update the Grafana dashboard | Sonnet medium | Instrumentation tests count each durable event one time, no entity/account IDs in metric labels, no replay publishing; the load test stays in the 200-session tick p99 <20 ms budget that exists, with a defined monster count |
| ✅ 6.3 Three diagrams | Use the [diagram-design skill](/home/matt-woodruff/.claude/plugins/marketplaces/diagram-design/skills/diagram-design/SKILL.md) and the [diagram README](../diagrams/README.md): `combat-sequence.html`, `npc-state-machine.html`, `stat-derivation-data-flow.html`; minimal-light/default, static inline SVG, fitted sequence/state and doc-inline data flow; link from the README | agy medium | Docs-only validation: the matching type/semantic/style/output references are read, self_check and the available geometry checker pass, the rendered desktop/narrow/print views are examined; the diagrams agree with the final code, and include the durable gate and the data provenance |
| ✅ 6.4 Decision record and integration review | Write one short `docs/decisions/phase-1-combat.md` that covers the HF precedence, the resolved damage coefficient, 500000, fixed scales, AI/replay, asset selection/licence and limitations; update the architecture/API/client/replay instructions and the diagram links | Codex high | The link/schema checks and the code-to-doc review pass; the necessary U/W/A matrix, UE tests, fight replay and `moon ci` are green; record all source errata and the measured playtest/load results, and do not claim checks that did not run |
| ✅ 6.5 Tick digest and AOI output optimisation (follow-up) | Replace the canonical JSON state digest with a versioned binary digest, and share the AOI output generation for each cell; the replay fixtures must still agree or get a new version | Codex medium | Done when the release p99 for 200x200 is less than 1 ms and the fixtures replay |

The release build measurements on Ryzen 9 3900X (Rust 1.98.1) were in the <20 ms budget:

- 1,000 moving NPCs: p50 0.418 ms / p99 0.445 ms.
- 200 players + 200 keltirs in a fight: p50 0.604 ms / p99 2.215 ms.
- 50 players + 50 keltirs: p50 0.131 ms / p99 0.317 ms (profiling: 72% JSON state digest, 24% AOI output).

The release-only `api:perf-check` (`moon run api:perf-check`) asserts p99 < 20 ms.

Snapshot schema 5 adds persistence lanes. Replay can still read schema 4. Record schema 3
and the `.nfr` fixtures that exist do not change. Recovery uses the durable `zone_epochs` index and
refreshed recovery snapshots. The refresh occurs a minimum of one time each day (23-hour cadence), and on a clean shutdown. Recovery refuses
admission when unresolved history is missing. See architecture §2.8 for the fail-closed semantics.

## 5. Order and estimate

1. E1.1 fidelity gate, E1.2–1.4 and the E2.1 contract: **4–5 days**. Prepare the E5.1 manifest early.
2. E2.2–2.6 core: **5–7 days**. The E3.1 data can start immediately after the contract.
3. E3.2–3.4 AI/pathing/spawns: **3–4 days**. E4 persistence after the checkpoint contract: **3–4 days**.
4. E5 client/import: **4–5 days**, after the one owner download and a stable proto. Placeholder
   rendering lets the client plumbing continue while that download is pending.
5. E6 telemetry/docs at the same time as the implementation, then replay/integration acceptance: **3–4 days**.

**Total: 22–29 focused engineering days (approximately 5–6 weeks), 6 epics / 25 stories.** The download wait
and the source-data gaps are schedule risks, not extra owner setup stories. Examine the work with much design
before the dependent mechanical stories. Keep this phase to a playable fight. Do not add more content.

## 6. Risks

| # | Risk | Mitigation |
|---|------|------------|
| R1 | Fixed-point formula fidelity: HF/Interlude conflicts, rounded examples, integer quantization and overflow | E1 source ledger, the coefficient that the source resolves and the explicit 500000 selection, exact decimal tables, independent rational oracle, worked vectors and caps; close the missing HF rows before you accept gameplay |
| R2 | AI determinism with pathing: iteration order, RNG consumption, timer drift, hidden path state | Flat integer paths, ordered maps/commands, specified RNG draws, all timers/state in the snapshot, restore tests during chase/swing/dead, off-AOI event comparison |
| R3 | Asset licensing/availability and missing animations: “free” is not a licence, and it does not make sure of combat clips | Verify the applicable terms, version and clip coverage of each listing before the one download; record the permitted project use/redistribution, and do not redistribute raw assets outside those terms; select a compatible free pack before the manifest handoff |
| R4 | Scope creep into skills, buffs, equipment or a full AI framework | One weapon block/type, neutral optional modifiers, fixed intention subset; new skill mechanics/content go into a later phase |
| R5 | Persistence lag, duplicate events or stale logout overwrites | Atomic checkpoint/outbox, generation/revision fence, deterministic IDs, crash/retry tests and the recovered durable checkpoint prefix before admission |

## 7. Out of scope for this phase

- Headless simulation scenarios for the loop above: Phase 1a
  ([phase-1a-simulation-testing.md](phase-1a-simulation-testing.md)) adds them later for Phases 0b–1,
  after this phase closes. Until then, the two-client scripted playtest of E5.4 stays manual.

- Skills, spells, buffs/debuffs (which include the Death Penalty/Lucky passives), resurrection skills/scrolls.
- Items, loot, inventory, equipment or enhancement other than the fixed starter weapon stat block;
  shields, soulshots, bows, dual attacks, polearm sweeps and positional/elemental combat modifiers.
- More than one zone, parties/shared XP, PvP/karma, raids/minions/champions, quests, class advancement.
- Obstacles/geodata/A*, collision avoidance, terrain/height/weather effects, combat with UE as the authority.
- Passive resource regeneration, full CP/magic combat, production-scale balance, or an art pipeline
  other than the one imported monster/proxy set. The movement/login behaviour that exists stays supported.

## 9. Outcome

- **Playtest verified:** A two-client playtest with the imported Animated Cockatrice (Fab EULA, author Marko Jäntti) and animated proxies completed the full kill → level 2 and death → respawn loop ([screenshot](../images/e5.4-playtest-victim-dead.jpg)).
- **All stories merged:** All stories in E1.1–E1.4, E2.1–E2.6, E3.1–E3.4, E4.1–E4.3, E5.1–E5.4, and E6.1–E6.5 were on main on 2026-10-09.
- **Server test suite:** ~535 server tests pass in the unit, repository transaction, and real-socket WebSocket integration suites.
- **Deterministic replay:** Two replay fixtures (467-tick movement, 908-tick fight) replay byte-identically in CI, from the zone epoch start and from a mid-fight snapshot.
- **Tick performance:** The release-build tick p99 for 200 players against 200 keltirs is 0.70 ms after the E6.5 digest/AOI optimization (much less than the 20 ms budget).
- **Independent Codex review:** An independent Codex review gave 10 findings. All of them were repaired (fail-closed epoch recovery, killing-blow credit, checkpoint lanes in snapshots, client XP/incarnation fences, real E2E test).
- **Lesson — The source-fidelity trap:** The nominal "High Five" tables in the research docs were Ertheia-era data. We replaced them directly from the pinned official L2J Bitbucket source, with a fixed-point engine that an oracle verifies.
- **Lesson — Property tests + replay for determinism bugs:** Property tests and byte-identical replay fixtures found small determinism bugs that the unit tests did not find (arrival order, rounding divergence).
- **Lesson — Codex for integration/mechanical stories:** Codex was very effective for mechanical table transcriptions, integration plumbing across the stack, and adversarial review.
