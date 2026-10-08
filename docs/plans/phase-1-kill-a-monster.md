# Phase 1 Plan: Kill a Monster

**Status:** draft for owner review (2026-10-08). **Status (original):** DRAFT 2026-10-08. **Planning model:** Codex; implementation model and effort per story.

## 1. Goal

The smallest Lineage 2 loop: a player targets and auto-attacks one NPC type, takes damage,
kills it, gains XP, levels up, can die, and respawns to fight again. Ship it in the existing
256×256 flat test zone with the existing login, WebSocket, AOI, SeaORM and replay stack.
Acceptance is one playable loop plus byte-identical headless replay, including a mid-fight restore.

## 2. Decisions

| # | Decision | Choice | Rejected | Why |
|---|----------|--------|----------|-----|
| D1 | Scope | One level-1 melee monster template, one fixed starter weapon stat block; two instances allowed to demonstrate social aggro | Skills, loot or class progression | Smallest complete combat loop |
| D2 | Formula authority | High Five formulas/tables from the planning references, transcribed verbatim into TOML under `packages/data/`; the physical damage coefficient is whatever the pinned L2J High Five `Formulas.java` uses for a normal attack (resolved by E1.1: **76**; the 77 is the crit-additive and skill constant), and the attack interval is **500000 / pAtkSpd** (HF), not Interlude's 470000 | Interlude XP/death brackets, simplified linear stats, invented balancing formulas | User's phase decision supersedes conflicting older decisions; source conflicts are enumerated below |
| D3 | Arithmetic | Integer/fixed-point only inside `domain/zone`; decimal data compiled before admission, fixed rounding contract (§3.1) | Runtime floating point, client-calculated damage | Same snapshot, inputs and seed produce identical state and bytes |
| D4 | NPC AI | L2J intention subset: Idle, Active, Attack, ReturnHome, Dead; aggro range, social aggro, hate list, leash, seeded respawn jitter | Behaviour trees, timers outside the actor, UE navigation as authority | Tick-driven, inspectable and replayable |
| D5 | Durability | All new simulation state in `ZoneState` and `ZoneSnapshot`, all new facts in `AppliedTick`; preserve draft → run → durable ack → release | Combat side effects before the durable gate, replay from session audit | Extends the current actor without a second simulation |
| D6 | Persistence/events | SeaORM transaction for XP/level/HP and associated state/outbox; periodic and logout saves; `CharacterLeveled` / `CharacterDied` through the existing outbox relay | Per-hit DB writes, direct NATS publishing from the domain | Atomic aggregate updates, bounded persistence lag |
| D7 | Art | Free Fab/Quixel/Game Animation Sample assets; exactly **one owner step**, a bundled download through Epic launcher/Fab site; agent performs import/retarget/wiring | Custom art production, repeated manual editor setup | A visible monster with idle/walk/attack/death animations at low production cost |

**Source precedence and fidelity gate.** Read [stats](../planning/01-stat-formulas.md) §§2–3,
[combat](../planning/03-combat-and-skills.md) §§2.1, 2.3–2.4, 2.7–2.8, and
[world](../planning/06-world-and-content.md) §§2.4–2.5. These references are not internally uniform:

- Stats §3 chooses Interlude curves, examples, XP and death brackets; this phase chooses the
  **HF** curves in §2.1, HF XP column in §2.6 (through 85), and HF death percentages in §2.7.
- Combat §2.1 prints **76** for normal attacks and 77 appears elsewhere; E1.1 resolves the
  coefficient from the pinned L2J HF source and records it (no architect override).
  Stats §2.4/§3.3 prints **470000** (Interlude); D2 selects combat §2.4's **500000** (HF).
- Accuracy/evasion disagree (`sqrt(DEX) * 5` in stats versus `* 6` in combat). Use the detailed
  stat derivations in stats §2.4, including its listed high-level accuracy additions and evasion
  addition above 69; record that precedence, rather than silently mixing both variants.
- Full HF starting profiles/bonus lookup rows and the level-86 XP sentinel are not printed in full.
  E1.1 must complete those from the primary L2J source already cited by the docs, pin its revision,
  and document the missing rows. Do not relabel Interlude profiles as HF or invent missing values.
- **Resolved by E1.1** against pinned L2J HF (`l2j-server-game@abfde049`,
  `l2j-server-datapack@3ca488dd`; ledger and errata in [SOURCES](../../packages/data/SOURCES.md)):
  K = **76**; interval 500000; accuracy/evasion use `sqrt(DEX) * 6` with the source's level
  additions (superseding the stats §2.4 precedence above); HF stat bonuses are the
  `statBonus.xml` tables (the docs' `1.009^(s−49)` curves and STR 88 are Ertheia data, E-1);
  the docs' "HF" XP column is Ertheia too (E-2); weapon stats replace the fist values (E-5).
- Preserve literal source formulas and table values alongside scaled data, with document/section
  and upstream revision provenance. The one explicit selection (500000 over 470000) stays labelled as an HF-vs-Interlude choice.
  No undocumented tuning or runtime interpolation; inconsistent worked-example rows get explicit
  errata and a rational calculation, never wider tolerances just to make a test pass.

## 3. Architecture and deterministic contract

Keep [architecture](../engineering/architecture.md)'s inward dependencies: TOML loading in
infrastructure, immutable validated rules injected through application bootstrap, pure stat/combat/AI
calculations in `apps/api/src/domain/zone`, wire conversion in `interface::zone_mapping`.
Extend `packages/proto/nightfall/v1/world.proto` first and regenerate Rust/TurboLink bindings.
The current `Fixed(i32)` means milli-tiles, `Speed` means milli-tiles/tick, `Tick` is 100 ms,
and AOI is 3×3 cells of 32 tiles. Retain those units and `ChaCha12` key/stream/word-position state.

### 3.1 Formula representation, scaling and examples

`Q = 1_000_000`. Bonuses, coefficients and derived fractional stats use signed `i64` Q units;
base attributes/levels, live HP/MP, damage, hate and XP are whole integers (XP `u64`). Use checked
`i128/u128` intermediates; reject data exceeding proven bounds, never wrap. Decimal TOML literals
are strings or explicit scaled integers, parsed exactly without binary floating point. Store formula
IDs, literal source expressions and constants in `tables/formulas.toml`; a typed evaluator implements
only the specified operations. Static curves become checked-in lookup tables, not runtime `pow`.
NPC template combat stats are final values; do not apply player class bonuses to them a second time.
The chosen level-1 monster does not trigger HF's level-78+ NPC damage penalty (stats §2.9).

Let `F` mean mathematical floor, `C` ceiling; signed divisions must implement floor explicitly.
Evaluate each expression below as one rational product before its stated final division. Do not
truncate bonuses, P.Atk or P.Def to whole units before damage. Resource maxima floor to whole units;
UI rounding never feeds back into simulation. `isqrt(n)` is the floor integer square root.

| Calculation | Data and exact evaluation/rounding |
|-------------|------------------------------------|
| Six stat bonuses | HF `statBonus.xml` 2-decimal table values for indices 0–99 (exact at Q), with their generating comments (`1.036^(STR−34.845)` …) kept as provenance; the generator re-derives each row from its comment in high-precision decimal, doubling precision until rounding agrees, and records disagreements as errata. Lookups outside 0–99 are errors. (The previous `1.009^(s−49)` set is Ertheia, errata E-1.) |
| Level modifier | `LM_Q = (L+89)*10000`, exact representation of `(L+89)/100`. |
| P.Atk / P.Def | `A_Q = F(atk_Q*STR_Q*LM_Q/Q²)` where `atk` is the weapon's P.Atk when one is held (L2J `<set>` replaces the fist value, E-5) else the class fist P.Atk; `D_Q = F(baseUnarmouredDef_Q*LM_Q/Q)`. No occupied armour slots in this phase. |
| HP / MP | `n=L−1`; class table values from stats §2.5: `H_Q=baseHP_Q+aHP_Q*n+bHP_Q*n²`, analogously `M_Q`; `maxHP=F(H_Q*CON_Q/Q²)`, `maxMP=F(M_Q*MEN_Q/Q²)`. Reconcile expanded rows with the source tables. |
| Accuracy / evasion | `root_Q=isqrt(DEX*Q²)`; `acc_Q=6*root_Q+L*Q+accAdd_Q[L]+weaponAccuracy_Q`; `eva_Q=min(250*Q,6*root_Q+L*Q+evaAdd_Q[L])`. HF additions as exact per-level entries: accuracy `+(L−69)` above 69 and `+(L−76)` above 77; evasion `+(L−69)` from 70, ×1.2 from 78 (E-3). |
| Hit | Flat/front condition multiplier is Q. `chance‰=clamp(F((800*Q+20*(acc_Q−eva_Q))/Q),200,980)`. Preserve the docs' comparator: **hit iff roll ≤ chance**, roll uniform `0..999`; thus “80%” nominal is 801/1000 outcomes. Test this boundary explicitly, do not quietly change `>=` to `>`. |
| Physical crit | `crit‰=min(500,F(base*DEX_Q*10/Q))`, base = HF fist 4 or the weapon's (Squire's Sword 8); crit iff roll `< crit‰`; multiplier 2, additive crit modifiers 0. |
| Physical damage | `max(1,F(K*A_Q*criticalFactor*(100+j)/(D_Q*100)))` with `K` = **76** (resolved, `calcPhysDam`), integer `j` uniform in `[-r,r]` from the weapon's random-damage radius (Squire's Sword 10; bare hands `5+isqrt(L)`). This is the documented physical pipeline with D2's constant; soulshots, position, traits, attributes, PvP and buffs are neutral (1 or 0). No skills. |
| Attack timing | `speed_Q=min(1500*Q,F(baseAtkSpeed_Q*DEX_Q/Q))`, base = weapon speed when held (379) else fist (300), strictly positive. Exact interval is `500000*Q/speed_Q` ms. Impact after `max(1,C(500000*Q/(2*speed_Q*100)))` ticks; next cycle after `max(1,C(500000*Q/(speed_Q*100)))` ticks. Quantize independently from the exact fraction, never from a rounded millisecond display. |
| Hate | On landed damage `d`, add `F(d*100/(npcLevel+7))`, cap total at `999999999`; track actual damage separately. Auto/social aggro adds 1 hate. |
| XP / death | HF cumulative XP `X[L]` in `experience.toml` (L1–85 plus sentinel `X[86]`; XP capped at `X[86]−1`), rate 1 and template reward, no Nightfall level-gap XP multiplier. `loss=F((X[L+1]−X[L])*loss_Q[L]/Q)` then `xp=max(0,xp−loss)`; derive level by threshold search. HF loss is 10% at L1, −0.125 percentage points/level through L49, 4% through L75, 2.5/2/1.5% at L76/77/78, 1% at L79–85. Store every row; e.g. L2 fraction is 98750/Q. |
| Respawn | NPC delay in integer ticks: `deathTick+10*(delaySeconds+uniform(0..randomSeconds))`, inclusive random endpoints. Player town respawn: `HP=max(1,F(maxHP*65/100))`, MP 0, protection `PlayerSpawnProtection` = 600 **s** = 6000 ticks (E-6); Attack ends protection. |

Worked vectors (tests must assert the integer encodings as well as displayed values):

1. Docs' `LM(20)=1.09` → `1090000`; no-armour P.Def 80 → `87200000` (87.2).
   HF Human Fighter STR 40 gives `1200000`; Squire's Sword P.Atk 6 at L1:
   `F(6000000*1200000*900000/10¹²)=6480000` (bare hands 4 → `4320000`).
   Legacy arithmetic only (Ertheia STR 88, E-1): `F(12000000*1418259*900000/10¹²)=15317197`.
2. Docs' Human Fighter raw L40 HP/MP: `80+11.765*39+0.065*39²=637.700`,
   `30+5.430*39+0.030*39²=287.400`. With supplied fixture bonuses CON=1.57, MEN=1.28,
   maxima are `floor(1001.189)=1001` HP and `floor(367.872)=367` MP.
   Those bonuses test arithmetic; the HF table has CON 43 = 1.58 (E-4), giving 1007 HP.
3. Accuracy 40 versus evasion 35 → 900‰; rolls 900/901 hit/miss. Differences −31/+10
   clamp to 200/980‰. HF DEX 30 bonus 1.10 gives fist crit 44‰: rolls 43/44 crit/normal;
   with Squire's Sword (base 8) 88‰. HF Human Fighter L1 accuracy `6*5477225+1Q=33863350`.
   Stats §3.3's Interlude base-44 example instead floors to 48‰; test it only as a labelled legacy vector.
4. K=76 (resolved): P.Atk 100, P.Def 50, spread 0: `76*100/50=152`; crit = 304. Spread −10 gives
   `floor(136.8)=136`; miss = 0 damage, no crit/spread draw. Damage 152 to an L20 mob adds `floor(152*100/27)=562` hate.
5. Docs' speed 300: exact interval 1666⅔ ms, impact tick +9, next swing +17.
   Speed 1500: 333⅓ ms, impact +2, next swing +4. Displayed 1667/333 ms is not scheduling input.
   Squire's Sword at DEX 30: speed `F(379*1.10)=416.9`, impact +6, next swing +12.
6. Docs' HF thresholds `X[1]=0, X[2]=68, X[3]=363`: XP 60 + reward 10 → 70, level 2.
   Death at L2 loses `floor((363−68)*0.09875)=29`, leaving XP 41 and level 1.
   At maxHP 126, town respawn restores 81 HP. XP stops at `X[86]−1 = 16890558727` (level 85);
   the L86 sentinel supplies L85's death-loss span (`F(3710077625*0.01)=37100776`), not a level.

### 3.2 Commands, state, AI and replay

- Add session intents `SetTarget` (nullable target), `Attack` (enable/disable), and `Respawn`.
  Session identity supplies the actor; no client damage/stat inputs. Validate alive state, target
  existence/life incarnation, attackability, range/AOI, and session generation. An accepted Attack
  chases into melee range, then repeats; duplicate enable must not reset cooldown or create a swing.
  MoveTo cancels auto-attack; clearing/changing target cancels a pending swing. Players cannot attack players.
- Add proto combat events `AttackResult`, `EntityDied`, `EntityRespawned`, `StatsChanged`, `XpGained`,
  `LevelUp`. Carry tick, stable event index, entity IDs and life incarnation; AttackResult has miss/crit,
  damage and remaining HP. Add target/attack-cycle state (start/impact/ready ticks) to authoritative
  state updates so animations can wind up before impact. EntitySpawn includes template, life state,
  public HP/maxHP/level and current cycle for AOI entry. MP/XP/private stats are owner-only.
- Snapshot base/derived stats, HP/MP, XP/level, target, auto-attack flag, pending swing/target incarnation,
  cooldowns, protection, death/corpse/respawn deadlines, NPC template/spawn slot/home, intention,
  hate/damage maps, last-hit/think/wander timers, path destination, event counters and pending save state.
  Store immutable resolved rules/templates in the snapshot; hash **all** dependent TOML, not just zone
  bounds. Restore never consults current data. Bump snapshot/log schemas and reject unsupported versions.
- Tick order: admitted inputs by ordinal → due respawns → AI think/commands → movement → due impacts
  by attacker ID → immediate damage/death/XP/level consequences → start eligible next cycles → AOI
  outputs/checkpoint facts. If an earlier impact kills an attacker, its later impact is cancelled.
  Generated AI commands have a separate ordered field in AppliedTick; replay regenerates and compares
  them, never feeds them back as external inputs and executes them twice.
- Every new world/internal event, including off-AOI AI changes and persistence facts, is in AppliedTick
  and its durable record codec, not only the observer outputs. Record a canonical end-of-tick state
  digest too, so invisible state drift fails replay. Preserve response-first ordering, then
  sorted AOI despawns/spawns/moves, then combat facts in causal event-index order; spawn precedes any
  fact referencing a newly visible entity. Only emit cross-entity combat facts when both are known;
  owner StatsChanged still delivers damage/death state when the attacker is outside AOI.
- NPC think every 10 ticks: Idle wakes when players are nearby; Active scans aggro range, seeds hate,
  and otherwise rolls 1-in-30 to wander; Attack selects most hated, chases and swings. Ties retain the
  current target, then lowest EntityId. A clan call gives nearby idle/active allies 1 hate, once per
  engagement; process allies in ID order without recursive calls. No players nearby must not stop
  an already scheduled return, corpse expiry or respawn timer.
- ReturnHome on leash breach, unreachable/invalid target or 1200 ticks without a successful hit;
  clear hate/cycle, walk to home, restore full NPC HP there, then Active. Returning NPCs cannot be
  damaged or re-aggroed. Dead clears movement/target/hate, keeps its corpse until corpse deadline,
  then hides it; its spawn slot/timer survives. Respawn restores template stats with a new life
  incarnation. The same one NPC type may have two spawn slots for social-aggro tests.
- Flat-zone pathing uses existing `Vec2Fixed::step_toward`, squared integer range checks and stable
  tie rules; no obstacles, actor collision avoidance or asynchronous nav queries. Data ranges use
  explicit milli-tiles, never implicitly treat L2 world units as tiles. Enforce bounds and leash.
- RNG order is fixed: due spawn slots in slot-ID order, AI by entity ID, impacts by attacker ID.
  At a valid impact draw hit, then (only on hit) crit and spread; invalid/cancelled swings draw nothing.
  Draw respawn jitter once in the death consequences; a zero spread/jitter radius consumes no draw.
  Wander direction/distance and respawn jitter use specified inclusive integer ranges and unbiased
  rejection sampling from the existing ChaCha12 stream. Snapshot at every timer boundary in tests.

### 3.3 Progression and persistence boundary

One kill grants the template's full XP once to the eligible live player landing the killing blow;
this slice's allocation policy is explicit, with no party or shared reward system. Process multiple
thresholds in order, emitting LevelUp per gained level and final StatsChanged. Recalculate stats on
level loss too (StatsChanged carries old/new level); keep current HP/MP, clamped to new maxima.
Death sets HP zero, cancels combat, charges XP loss once, and retains dead state through reconnect.
Respawn is an explicit dead-player request to the zone's safe point; no resurrection skill or XP refund.

Emit a full persistence checkpoint every 50 ticks (5 s), on death/level change and before logout
despawn. Include XP, level, HP, MP, alive/dead state, position, character revision and pending events.
Application worker persists the checkpoint plus outbox entries in **one SeaORM transaction**, then
returns a recorded system acknowledgement; pending checkpoints/events live in ZoneState/Snapshot
until acknowledged. Logout/admission waits for the final save; stale generations/revisions cannot
overwrite a replacement. Retry keys are `(zone, epoch, character, checkpoint_revision)` with a
payload hash: identical retry is a no-op; changed payload conflicts. Event IDs are deterministic.
No I/O in the domain or new rule based on DB completion time. After a crash, recover acknowledged
checkpoint facts from the durable applied-log prefix before admitting the character; this projection
does not bypass the replay verifier's existing refusal of incomplete epochs. Ordinary unsaved progress
may roll back by up to 5 s; death/level checkpoints must be recovered, including their outbox facts.

## 4. Epics, stories, tasks

Models: **Sonnet medium** for mechanical work, **Opus high** for design-heavy work,
**Codex high/medium** for integration, review and docs, **agy medium** for docs-only diagrams.
Every Done when includes tests or document validation; completion also requires the applicable
[API guidelines §4](../engineering/api-guidelines.md#4-required-tests-per-endpoint) matrix:
**U** = unit happy path, every typed error, validation with no state/event/RNG change;
**W** = real-socket happy path/all fields, every documented error, authentication/ownership, retry,
read-after-write; **A** = repository round-trip, typed constraints, rollback and concurrent retries.
For WS mutations, retain §3b monotonic seq (repeated seq closes 4400); test fresh-seq repeated intents
for semantic idempotence. Keyed save/RPC mutations test same-key replay and changed-body conflict.
No endpoint means W is inapplicable, not an excuse to omit its domain/adapter/UE tests.

### E1: Stat engine as data

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 1.1 Source and TOML tables | Resolve §2 fidelity ledger; add `packages/data/tables/{stat_bonus,formulas,experience,penalties}.toml`, HF starter profiles/HP/MP tables in `classes/`, fixed starter weapon block; preserve literals, source revisions and decimal table generator | Opus high | Golden transcriptions match every selected printed value; missing HF rows and L86 sentinel are source-backed; discrepancy tests identify incompatible Interlude/rounded examples explicitly |
| 1.2 Validated loading | Extend infrastructure data loader; exact decimal parsing, table coverage, monotone XP, nonzero defence/speed, coefficient bounds, cross-file IDs and canonical config hash; inject resolved rules | Sonnet medium | U: malformed decimals, holes, overflow, bad references and invalid caps reject startup; identical files yield identical rules/hash; one changed coefficient changes hash |
| 1.3 Pure domain stat calculation | Implement §3.1 in domain/zone with typed scales, integer sqrt, caps, XP lookup/loss; extend no-float/source scans for every new module | Opus high | U: each worked vector matches exact scaled values; generated legal inputs satisfy resource bounds, monotone XP/level, capped hit/crit/speed and overflow safety; clippy/source scans pass |
| 1.4 Formula fidelity review | Add property tests against docs' worked examples and independent rational oracle, all supported levels/profiles; compare legacy vectors only under labelled legacy inputs; review errata | Codex high | U: golden and property suites pass with documented display rounding; no unexplained mismatches, copied implementation oracle, substituted Interlude production table or hidden runtime float |

### E2: Combat core in the zone actor

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 2.1 Contract and intent plumbing | Add Attack/SetTarget/Respawn, six required combat events and cycle/state fields; append proto tags/reasons, regenerate bindings; map session identity and generation | Codex high | U+W: encode/decode all fields, malformed/unknown/dead/nonattackable/out-of-AOI targets, ownership/stale session, auth, seq/rate limits, repeated intents and applied-tick Ack/rejection pass; no old tag changes |
| 2.2 Complete combat state and log | Extend Entity/ZoneState/ZoneSnapshot, SpawnPlayer loaded state, AppliedTick events/generated commands/state digest and durable codec; immutable data bundle and schema versions; preserve AOI ordering/privacy | Opus high | U: mid-swing snapshot round-trip equals uninterrupted ticks including RNG/bytes; off-AOI facts persist; AOI entry and replacement reconstruct combat state; unsupported schema rejected; gate failure releases nothing |
| 2.3 Auto-attack and damage | Target/enable/disable/chase, integer impact/ready deadlines, range recheck at impact, miss/crit/spread, damage and cycle events for player and NPC | Opus high | U+W: §3.1 vectors, repeat Attack without extra swing, cancellation, target replacement, moving out of reach, simultaneous lethal impacts and seeded hit/crit boundaries pass; client cannot supply damage |
| 2.4 Death and player respawn | One-shot death consequences, HP zero, cycle cancellation, H5 XP loss/delevel, dead-state admission, safe-point Respawn and protection | Opus high | U+W: duplicate lethal/respawn attempts cannot charge twice or revive living actors; dead movement/attack rejected; 65% HP/0 MP/protection expiry and early Attack cancellation tested; reconnect preserves death |
| 2.5 Aggro and hate core | Ordered hate/damage ledger, H5 damage hate/cap, target eligibility, retain-current/ID ties, forget on death/despawn; expose deterministic aggression commands/events to E3 | Opus high | U: arithmetic/cap/tie vectors, two attackers, disconnect/death cleanup and identical-command/seed properties pass; rejected attacks add no hate |
| 2.6 XP and level transition | Exactly-once kill credit, H5 threshold search/cap, LevelUp/XpGained/StatsChanged, HP/MP clamp on recalc, checkpoint facts | Sonnet medium | U+W: XP 60→70→41 example, multi-level reward, capped XP, two same-tick killers and repeated death yield one reward; owner-only XP and read-after-write state asserted |

### E3: NPC data and AI

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 3.1 Templates and spawn data | Add `packages/data/npcs/*.toml`: one attackable melee type, level/base stats, XP, movement/range/collision, clan/help/aggro/leash, corpse/respawn fields; extend `zones/test_zone.toml` with template-referencing spawn slots/count/home and player safe point | Sonnet medium | U: references/units/bounds/delays validate, old Gatekeeper/Wanderer remain noncombat fixtures or are explicitly migrated; 256×256 zone boots; no second monster type required |
| 3.2 Intention state machine | Implement §3.2 Idle/Active/Attack/ReturnHome/Dead transitions and events, 10-tick think, auto/social aggro and 1-in-30 wander; use E2 hate and attacks | Opus high | U: transition table covers every guard; same-clan help, out-of-range/non-clan non-help, no recursive help, dead target and empty-region timers pass under fixed ticks/seed |
| 3.3 Deterministic chase and leash | Reuse integer flat-zone movement for chase/wander/return; bound destinations, attack range and home radius; no-hit timeout, exact arrival, full HP on return | Opus high | U: diagonal/edge/zero-speed cases, tie ordering, 1200-tick timeout and leash reset pass; mid-path snapshot restores identical events/bytes; property test proves progress without overshoot |
| 3.4 Corpse and respawn scheduler | Stable spawn-slot ownership; death → corpse expiry → delay plus seeded random delay → new life; preserve slot when corpse leaves AOI | Sonnet medium | U: min/max jitter, zero jitter, corpse before respawn, never-double-spawn, same-seed sequences, stale-incarnation target rejection and snapshot during Dead all pass |

### E4: Persistence and events

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 4.1 Character repository transaction | Migrate XP/level/HP plus MP/life/revision state, generated SeaORM entities; extend memory/Postgres ports for atomic checkpoint+outbox and idempotency | Sonnet medium | U+A: round-trip, valid XP/level/HP constraints, no partial update/outbox on failure, concurrent identical retries exactly once, conflicting body/stale revision rejected; entity regeneration clean |
| 4.2 Checkpoint and admission integration | Five-second tick checkpoint, death/level checkpoint, logout final save before despawn, recorded save ack; admission loads committed state; recover durable checkpoint prefix after crash | Codex high | U+A+W: periodic/logout/reconnect preserve fields; delayed DB and generation replacement cannot overwrite newer state; crash before/after transaction and log ack recovers checkpoint facts; failed save retained/retried |
| 4.3 Domain events through outbox | Map deterministic facts to CharacterLeveled/CharacterDied, ordered aggregate sequence and stable IDs; stage with checkpoint, reuse relay on `nightfall.character.leveled` / `.died` | Codex high | U+A: one logical event per transition, multiple level-ups retained, failed commit publishes none; crash after commit/before publish and duplicate delivery prove retry/dedupe; death delevel reported consistently |

### E5: Client

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 5.1 Asset acquisition and import | Prepare one manifest of currently free Fab/Quixel/Game Animation Sample assets: IDs/URLs, version, licence/attribution, skeleton and all four required clips. **One owner action: download the complete manifest in one Epic launcher/Fab-site session to the agreed staging folder.** Then script import/migrate, retarget, materials, animation assets and proxy wiring; keep vendor content outside regenerated Input/Blueprints/UI/Maps | Codex high | Manifest/licence validation and scripted repeat-import test pass; editor asset-load/cook tests find mesh+idle/walk/attack/death; owner does no manual import/retarget. Verify clip coverage before requesting download; choose a free compatible pack if Game Animation Sample lacks attack/death |
| 5.2 Typed combat projection and HUD | Extend generated-code bridge, NetClient, WorldProxySubsystem and OwnEntityComponent; target frame, player/NPC HP bars, XP/level display and authoritative target clearing | Sonnet medium | UE automation tests feed recorded events and assert every field, owner privacy, AOI entry/exit, stale life/generation rejection and reconnect state; no local damage calculation |
| 5.3 Click attack and damage numbers | NPC raycast sends SetTarget then Attack; ground click keeps MoveTo/cancel; render miss/crit/damage once per event; show rejection and pending state | Sonnet medium | UE automation plus real-server W: NPC click attacks, ground click moves, repeated click cannot accelerate attacks, rejected/dead target clears correctly, replayed event cannot duplicate a number |
| 5.4 Animated proxies and death UI | NPC idle/walk/attack/death from chosen pack/Game Animation Sample, cycle timing from server, frozen corpse then removal; dead-player overlay and Respawn button; target/HP reset after respawn | Codex high | UE automation covers life/animation/UI transitions; two-client scripted playtest completes kill→XP→level and death→respawn, including late AOI entry and 150 ms interpolation; cooked assets load with no manual editor wiring |

### E6: Replay, telemetry, docs

| Story | Tasks | Model | Done when |
|-------|-------|-------|-----------|
| 6.1 Fight recording and CI | Extend the two-player recording scenario with target/chase, hit/miss/crit, social aggro/leash, NPC death/respawn, XP/level, player death/delevel/respawn and disconnect; write versioned `apps/api/fixtures/sessions/two-players-fight-v2.nfr`; update replay-check and codec fixtures | Codex high | Byte/digest replay passes from epoch and mid-fight snapshot; old movement coverage retained under explicit schema policy; changed coefficient, RNG draw, AI event or off-AOI state causes failure; gaps/incomplete watermark still refused |
| 6.2 Combat telemetry | Add bounded-label combat event/hit/miss/crit/death/respawn/level counters, AI intention counts, combat tick duration, save lag/failures; consume admitted events only, update Grafana dashboard | Sonnet medium | Instrumentation tests count each durable event once, no entity/account IDs in metric labels, no replay publishing; load test stays within existing 200-session tick p99 <20 ms budget with defined monster count |
| 6.3 Three diagrams | Use [diagram-design skill](/home/matt-woodruff/.claude/plugins/marketplaces/diagram-design/skills/diagram-design/SKILL.md) and [diagram README](../diagrams/README.md): `combat-sequence.html`, `npc-state-machine.html`, `stat-derivation-data-flow.html`; minimal-light/default, static inline SVG, fitted sequence/state and doc-inline data flow; link from README | agy medium | Docs-only validation: matching type/semantic/style/output references read, self_check and available geometry checker pass, rendered desktop/narrow/print inspected; diagrams match final code, include durable gate and data provenance |
| 6.4 Decision record and integration review | Write one short `docs/decisions/phase-1-combat.md` covering HF precedence, the resolved damage coefficient, 500000, fixed scales, AI/replay, asset choice/licence and limitations; update architecture/API/client/replay instructions and diagram links | Codex high | Link/schema checks and code-to-doc review pass; required U/W/A matrix, UE tests, fight replay and `moon ci` green; record all source errata and measured playtest/load results without claiming unrun checks |

## 5. Order and estimate

1. E1.1 fidelity gate, E1.2–1.4 and E2.1 contract: **4–5 days**. Prepare E5.1 manifest early.
2. E2.2–2.6 core: **5–7 days**; E3.1 data can follow the contract immediately.
3. E3.2–3.4 AI/pathing/spawns: **3–4 days**; E4 persistence after checkpoint contract: **3–4 days**.
4. E5 client/import: **4–5 days**, after the single owner download and stable proto; placeholder
   rendering lets client plumbing proceed while that download is pending.
5. E6 telemetry/docs alongside implementation, then replay/integration acceptance: **3–4 days**.

**Total: 22–29 focused engineering days (about 5–6 weeks), 6 epics / 25 stories.** Download wait
and source-data gaps are schedule risks, not extra owner setup stories. Review design-heavy work
before dependent mechanical stories; keep this phase to a playable fight rather than expanding content.

## 6. Risks

| # | Risk | Mitigation |
|---|------|------------|
| R1 | Fixed-point formula fidelity: HF/Interlude conflicts, rounded examples, integer quantization and overflow | E1 source ledger, source-resolved coefficient and the explicit 500000 selection, exact decimal tables, independent rational oracle, worked vectors and caps; close missing HF rows before accepting gameplay |
| R2 | AI determinism with pathing: iteration order, RNG consumption, timer drift, hidden path state | Flat integer paths, ordered maps/commands, specified RNG draws, all timers/state snapshotted, restore tests during chase/swing/dead, off-AOI event comparison |
| R3 | Asset licensing/availability and missing animations: “free” is not a licence or a guarantee of combat clips | Verify each listing's applicable terms, version and clip coverage before the one download; record permitted project use/redistribution, avoid raw-asset redistribution outside those terms; choose compatible free pack before manifest handoff |
| R4 | Scope creep into skills, buffs, equipment or a full AI framework | One weapon block/type, neutral optional modifiers, fixed intention subset; new skill mechanics/content go into a later phase |
| R5 | Persistence lag, duplicate events or stale logout overwrites | Atomic checkpoint/outbox, generation/revision fence, deterministic IDs, crash/retry tests and recovered durable checkpoint prefix before admission |

## 7. Out of scope for this phase

- Skills, spells, buffs/debuffs (including Death Penalty/Lucky passives), resurrection skills/scrolls.
- Items, loot, inventory, equipment or enhancement beyond the fixed starter weapon stat block;
  shields, soulshots, bows, dual attacks, polearm sweeps and positional/elemental combat modifiers.
- Multiple zones, parties/shared XP, PvP/karma, raids/minions/champions, quests, class advancement.
- Obstacles/geodata/A*, collision avoidance, terrain/height/weather effects, UE-authoritative combat.
- Passive resource regeneration, full CP/magic combat, production-scale balance or an art pipeline
  beyond the one imported monster/proxy set; existing movement/login behaviour remains supported.
