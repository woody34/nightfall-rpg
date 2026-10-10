Phase 3 uses the same isolated fixture lifecycle as [phase2-fixtures.md](phase2-fixtures.md).
Fixtures migrate and seed **before API startup/admission**. They require `--api start`
(`run-sim-ci.py --fresh-stack`), distinct role accounts, fresh DB/JetStream volumes,
nondefault loopback ports, private token files, and owned teardown. The wrappers
never modify admitted characters. `SIM_PHASE3_SEEDER` selects the published seeder;
the default is `infra/scripts/seed-phase3-fixture.py`. Only explicit dev-token opt-in
plus a loopback `nf_phase3_fixture_*` DB on a non-5432 port allows a seed.

**Integration status:** Schema reconciled to migration 000008 in API commit
`0a37fb3f6c69c9b3dc3f9b9b4d1fe8291262cb5f`. Seeds set timer policy1, zero
fractions/counters/penalty, absent death record, empty modifier order and timer/
effect children. Existing 000007 SP/slots/ranks remain the only learned ledger.
The candidate migrator must contain 000008, now merged into this worktree;
an older candidate binary fails the child-table guard. Transaction failure rolls back the entire seed,
and a failed/mismatched seed never permits API startup. Retained typed payloads,
live admission and gameplay are still pending the engine/API handoff. No live
seed, scenario, Unreal build, API/Compose launch or packaged soak has been run
by this worker.

| Exact header | Roles and persisted inputs |
|---|---|
| `# fixture: phase3-trainer` | Human Mystic class10/base10, level14, SP10000, at126,128. Wind Strike1177 rank1, Ice Bolt1184/Might1068/Shield1040/Curse Weakness1164 rank1. **1177 rank2 is unlearned**, cost240/required level7 from professions/human_mystic.toml. Optional matching A/B observer pair. |
| `# fixture: phase3-pvp` | Two distinct Human Mystic accounts level14 at110,100 and112,100, outside inclusive peace square116..136. Owner SP10000,1177 rank5/1184 rank4/1068 rank1. Peer SP0,1177 rank1,HP100,CP10. |
| `# fixture: phase3-resurrect` | Cleric class15/base10 level30 with1016 rank1 and1069/1068/1216 rank1 at110,100; distinct living peer level14 at112,100,HP100,CP10. Peer dies through ordinary sentinel retaliation; no precreated offer or fabricated death loss. |
| `# fixture: phase3-regen` | Human Mystic level14,SP0,HP/MP/CP1 at126,128;1177 rank1. |

All unconfigured HP/MP are NULL (server derives maxima); CP defaults0. XP comes
from shipped experience.toml at the seed level; active class-slot SP equals
characters.sp. Learned ranks use existing `l2.skill.<id>` keys. The owner character
is `01970000-0000-7000-8000-000000000030`, peer `...0031`. Accounts are random per
unit. The manifest includes exact role inputs, account/character identifiers,
binary/migrator/seeder hashes, every input data hash and the final overlay-zone
hash. Generated overlay inputs stay under the private fixture directory.

The overlay copies shipped data and makes keltirs passive/no-clan-help while
preserving XP/SP rewards. One keltir remains uniquely targetable. `skill_target`
is a passive million-HP combat target; `skill_sentinel` is the Phase2 sentinel
renamed. Template construction reuses token-fixtures.py's audited inputs; neither
is a gameplay formula/oracle. Class Master stays126,128. Numeric fixture inputs
are test choices, not claims about pinned gameplay. Source skill durations/costs
remain unchanged. Oracle assertions require independent, hash-pinned E1 vectors;
those vectors have not landed in this worktree and no oracle pass is claimed.

The 17 matrix files retain their stable names. Two additional `-peer.nfs` files
provide real cooperating clients for the solo-named karma/aggro stories.
`Scenarios/phase3-roles.json` binds each primary to its companion; both reports
have unique filenames/artifact folders and independent accounts. Missing roles,
repeated paths, mismatched fixture headers and duplicate artifact names fail.
Existing A/B units remain atomic. Coordination uses received server state only.

Run the first scenario with `Scripts/run-sim.sh --api start --artifacts <new-dir>
Scenarios/3-cast-skill.nfs`. Run cooperating roles with `run-sim-multi.sh` and
both paths. Source-duration waits remain full length: Might1200s →3600s timeout;
regen30ticks×100ms×3→9s/pulse; flag1200ticks×100ms×3→360s. Each scenario's global
budget contains the sum of its per-step waits plus exit grace. Wrappers/CI extend
process budgets to at least `nf.Within +30s`, retaining all final evidence gates.
No duration or cost is recomputed from client combat stats in Python/Bash.

**Final authoritative DSL handoff (merged client `3b21cd7`).** This single table
covers every command and predicate used by the Phase3 scenarios. Names use the
existing registry where semantics match: `rejected`, `private_stats_leaks`, and
`own_hp > 0`. Numeric comparisons use `<op> <unsigned n>`; explicit skills use
`<skill_id>` and entities use `<own|target|uuid>`. Pending names remain assertions
and MUST fail parsing; UNKNOWN must never become a successful zero/absence check.
All history is fenced by transport generation, entity incarnation, tick and the
cast/effect/offer identity. Vector comparisons consume expected values from an
independent, hash-pinned artifact; the harness does not calculate gameplay results.

| Command or predicate | Merged registry status | Exact received message / action and required evidence |
|---|---|---|
| `nf.Login` | Registered | Login response/access token; authenticates using the private role token. |
| `nf.EnterWorld` | Registered | Play ticket RPC then WebSocket admission; waits for own EntitySpawn. |
| `nf.ClickMove <x> <y>` | Registered | Sends MoveTo intent; confirmation is EntityMove, not local interpolation. |
| `nf.Target <uuid|template:id>` | Registered | Sends SetTarget; confirmation is TargetChanged. |
| `nf.Attack / nf.StopAttack` | Registered | Sends Attack/StopAttack; intent receipt and attack lifecycle confirm outcome. |
| `nf.Cast <id> [self|target|uuid]` | Registered | Sends CastSkill.skill_id/target; rank is chosen by the server; waits for Started/Result/Cancelled. |
| `nf.CancelCast` | Registered | Sends CancelCast; terminal SkillCastCancelled carries authoritative reason. |
| `nf.Learn <id> <rank> [uuid]` | Registered | LearnSkillRequest.character_id/skill_id/skill_level/idempotency_key; completed response/status, SkillLearned and StatsChanged are authoritative. Key reuse never buys next rank. |
| `nf.LearnableSkills / nf.SkillCatalog` | Registered | ListLearnableSkills/GetSkillCatalog unary RPCs; completed response only. |
| `nf.AcceptResurrect [latest|offer_id] [accept|decline]` | Registered | Reads the latest received ResurrectOffer.offer_id or explicit id; sends AcceptResurrect.offer_id/accept. |
| `nf.Respawn` | Registered | Sends RespawnRequest; waits for EntityRespawned + VitalsChanged + owner StatsChanged. |
| `nf.DropSocket` | Registered | Drops transport; no gameplay mutation. Reconnect requires fresh admission baselines. |
| `nf.WaitFor / nf.Expect / nf.Within` | Registered | Scenario parser directives; WaitFor/Expect read the registered predicate, Within sets only the runner deadline. |
| `connected` | `connected`: registered | Login/auth response access token; authenticated local transport state, not a zone event. |
| `in_world`<br>`reconnects` | `in_world`: registered<br>`reconnects`: registered | WebSocket transport generation and owner EntitySpawn.entity/incarnation; no synthetic gameplay facts. |
| `own_at`<br>`entity_at` | `own_at`: registered<br>`entity_at`: REQUIRED HANDOFF | EntitySpawn.position and EntityMove.entity/position/tick/incarnation; newest received sample, never client pawn position. |
| `entity_in_view` | `entity_in_view`: REQUIRED HANDOFF | EntitySpawn/EntityDespawn admitted known-entity identity cache. |
| `own_hp`<br>`own_dead`<br>`entity_alive`<br>`entity_dead` | `own_hp`: registered<br>`own_dead`: registered<br>`entity_alive`: REQUIRED HANDOFF<br>`entity_dead`: REQUIRED HANDOFF | EntitySpawn.hp, EntityDied.entity/incarnation, EntityRespawned, AttackResult.target_hp_after and VitalsChanged.hp; alive requires a known matching incarnation. |
| `own_mp`<br>`own_cp`<br>`initial_mp`<br>`initial_cp`<br>`initial_hp` | `own_mp`: registered<br>`own_cp`: registered<br>`initial_mp`: REQUIRED HANDOFF<br>`initial_cp`: REQUIRED HANDOFF<br>`initial_hp`: REQUIRED HANDOFF | Owner StatsChanged.mp/cp and EntitySpawn.hp; initial_* freezes the first owner admission sample, never an uninitialized zero. |
| `target` | `target`: registered | Owner TargetChanged.target; server-confirmed selection. |
| `attack_state` | `attack_state`: registered | IntentAck/IntentRejected.seq correlated to Attack, AttackStarted.attacker/tick and AttackStopped; pending is local intent state. |
| `rejected` | `rejected`: registered | IntentRejected.seq/reason; spelling is the existing client registry name (replaces last_reject). |
| `private_stats_leaks` | `private_stats_leaks`: registered | StatsChanged.entity received for a non-owner; existing ClassStateSubsystem leak counter (replaces observer_private_stats). |
| `own_sp`<br>`learned_rank` | `own_sp`: registered<br>`learned_rank`: registered | SkillStateSync.sp/learned, SkillLearned.skill_id/skill_level/sp_remaining and owner StatsChanged.sp. |
| `sp_gained_total`<br>`sp_gained_events` | `sp_gained_total`: registered<br>`sp_gained_events`: registered | Owner SpGained.amount/total/tick/incarnation; total sums actual received credits, events counts admitted envelopes. |
| `skill_learned_events` | `skill_learned_events`: registered | Owner SkillLearned.skill_id/skill_level/tick/incarnation; baseline is not a learned event. |
| `catalogue_skill_status` | `catalogue_skill_status`: registered | GetSkillCatalogResponse.skills[].skill_id/effect_implemented/deferred_reasons. |
| `learnable_contains` | `learnable_contains`: registered | ListLearnableSkillsResponse.skills[].skill.skill_id/skill_level; pending RPC cannot match. |
| `learn_rpc_error` | `learn_rpc_error`: registered | Completed LearnSkill unary canonical gRPC status; pending does not equal success. |
| `learn_receipt_matches_first` | `learn_receipt_matches_first`: registered | Two successful LearnSkillResponse messages for the same request UUID/fingerprint, all frozen response fields equal. |
| `skill_ready`<br>`skill_reuse_remaining` | `skill_ready`: registered<br>`skill_reuse_remaining`: registered | Owner SkillReuse.skill_id/ready_tick and SkillStateSync.reuse[].ready_tick vs newest received authoritative tick. No predicted reuse formula. |
| `cast_state`<br>`entity_casting` | `cast_state`: registered<br>`entity_casting`: REQUIRED HANDOFF | SkillCastStarted.caster/skill_id/cast_id/caster_incarnation/complete_at_tick, SkillCastResult and SkillCastCancelled of the same identity. Baseline hydration upserts without event counts. |
| `cast_starts`<br>`cast_results`<br>`cast_cancellations`<br>`observed_cast_results` | `cast_starts`: registered<br>`cast_results`: registered<br>`cast_cancellations`: registered<br>`observed_cast_results`: registered | Distinct nonbaseline SkillCastStarted/SkillCastResult/SkillCastCancelled; owner versus observer audience. Generic counters are insufficient for skill-specific observer termination. |
| `cast_cancel_reason`<br>`observed_cast_cancel_reason`<br>`cast_dispositions_once` | `cast_cancel_reason`: REQUIRED HANDOFF<br>`observed_cast_cancel_reason`: REQUIRED HANDOFF<br>`cast_dispositions_once`: REQUIRED HANDOFF | SkillCastStarted.cast_id + SkillCastResult.cast_id or SkillCastCancelled.cast_id/reason, with caster/incarnation and IntentAck/IntentRejected.seq. Exactly one terminal event per accepted cast; no late intent rejection. |
| `cancelled_cast_costs_retained`<br>`own_mp_cost_matches` | `cancelled_cast_costs_retained`: REQUIRED HANDOFF<br>`own_mp_cost_matches`: UNKNOWN: missing cost/vector input | Owner StatsChanged.mp before/start/completion, SkillReuse.ready_tick and matching cast identity. own_mp_cost_matches needs independently pinned per-rank MP1/MP2 expected values; GetSkillCatalogResponse currently carries NO costs. Registered but always UNKNOWN. |
| `skill_damage_matches_oracle`<br>`skill_effect_matches_oracle`<br>`stat_matches_oracle` | `skill_damage_matches_oracle`: REQUIRED HANDOFF<br>`skill_effect_matches_oracle`: REQUIRED HANDOFF<br>`stat_matches_oracle`: REQUIRED HANDOFF | SkillCastResult.targets[].resolution_kind/outcome/rolled/cp_absorbed/hp_removed/heal/applied_effect_ids + EffectApplied + owner StatsChanged.effective_stats compared to independent hash-pinned E1 vectors. No vector data has been supplied; no local gameplay formulas. |
| `buff_active`<br>`buff_inactive`<br>`debuff_active`<br>`entity_effect_active`<br>`entity_effect_inactive`<br>`effect_count` | `buff_active`: registered<br>`buff_inactive`: registered<br>`debuff_active`: registered<br>`entity_effect_active`: registered<br>`entity_effect_inactive`: registered<br>`effect_count`: registered | EffectApplied.target/effect_id/skill_id/pool/end_tick and EffectRemoved.effect_id plus EntitySpawn.active_effects or baseline hydration. Absence only matches a known baseline, never missing state. |
| `buff_replacements`<br>`replacement_remove_before_apply`<br>`effect_identity_coherent`<br>`effect_remove_reason` | `buff_replacements`: REQUIRED HANDOFF<br>`replacement_remove_before_apply`: REQUIRED HANDOFF<br>`effect_identity_coherent`: REQUIRED HANDOFF<br>`effect_remove_reason`: REQUIRED HANDOFF | EffectRemoved.target/incarnation/effect_id/reason/tick then EffectApplied with a NEW effect_id; retained stable IDs/deadlines and observed order, not elapsed client time. |
| `retained_effect_identity_coherent`<br>`restore_did_not_reapply_hooks` | `retained_effect_identity_coherent`: REQUIRED HANDOFF<br>`restore_did_not_reapply_hooks`: REQUIRED HANDOFF | Baseline EffectApplied.skill_id/skill_level/pool/caster/applied_tick/end_tick and owner reuse sync compared before/after admission plus replay/checkpoint adapter evidence. Wire lacks modifier install order and periodic phase; those MUST come from server precision evidence. |
| `owner_skill_baseline_coherent`<br>`observer_skill_baseline_coherent`<br>`spawn_before_skill_references` | `owner_skill_baseline_coherent`: REQUIRED HANDOFF<br>`observer_skill_baseline_coherent`: REQUIRED HANDOFF<br>`spawn_before_skill_references`: REQUIRED HANDOFF | Identity EntitySpawn/EntityRespawned batch, baseline SkillCastStarted/EffectApplied/PvpStateChanged and owner SkillStateSync/StatsChanged; every nested reference has an already-known matching incarnation, omitted hidden source pairs remain absent. |
| `public_vitals_coherent`<br>`public_revival_vitals_coherent` | `public_vitals_coherent`: REQUIRED HANDOFF<br>`public_revival_vitals_coherent`: REQUIRED HANDOFF | EntitySpawn final hp/max_hp; EntityRespawned then baseline VitalsChanged.hp/max_hp/tick/incarnation, agreeing with public SkillCastResult target_hp_after. Never observer MP/CP. |
| `regen_pulses`<br>`moving_regen_pulses`<br>`regen_matches_oracle`<br>`moving_regen_matches_oracle` | `regen_pulses`: REQUIRED HANDOFF<br>`moving_regen_pulses`: REQUIRED HANDOFF<br>`regen_matches_oracle`: REQUIRED HANDOFF<br>`moving_regen_matches_oracle`: REQUIRED HANDOFF | Owner StatsChanged.hp/mp/cp and VitalsChanged.hp/max_hp with EntityMove samples and independent E1 vectors; wire has NO regen-cause/pulse event. Exact pulse/moving/dead/full distinctions additionally need correlated actor/replay regen facts; do not infer a pulse from any HP increase. |
| `own_flag`<br>`target_flag`<br>`target_flag_remaining`<br>`own_karma`<br>`own_pk`<br>`own_pvp`<br>`own_flag_deadline_matches_received` | `own_flag`: registered<br>`target_flag`: registered<br>`target_flag_remaining`: REQUIRED HANDOFF<br>`own_karma`: registered<br>`own_pk`: registered<br>`own_pvp`: registered<br>`own_flag_deadline_matches_received`: REQUIRED HANDOFF | PvpStateChanged.entity/flag_until_tick/karma/pk_count/pvp_count/tick/incarnation plus EntitySpawn.flag_until_tick/karma and owner SkillStateSync. remaining uses received tick only; classify chaotic from received karma. |
| `own_cp_damage_total`<br>`player_cp_damage_total`<br>`cp_absorption_matches_received` | `own_cp_damage_total`: REQUIRED HANDOFF<br>`player_cp_damage_total`: REQUIRED HANDOFF<br>`cp_absorption_matches_received`: REQUIRED HANDOFF | Player-sourced AttackResult.cp_damage/damage/target_hp_after and SkillCastResult.targets[].cp_absorbed/hp_removed/target_hp_after with owner StatsChanged.cp. Nonzero witnessed absorption required. |
| `victim_flag_not_refreshed`<br>`flagged_kill_matches_received`<br>`impact_counter_uses_received_flag`<br>`target_flag_expired_at_last_impact`<br>`own_flag_expired_at_last_impact` | `victim_flag_not_refreshed`: REQUIRED HANDOFF<br>`flagged_kill_matches_received`: REQUIRED HANDOFF<br>`impact_counter_uses_received_flag`: REQUIRED HANDOFF<br>`target_flag_expired_at_last_impact`: REQUIRED HANDOFF<br>`own_flag_expired_at_last_impact`: REQUIRED HANDOFF | AttackStarted or SkillCastStarted at a witnessed pre-expiry tick, matching AttackResult/SkillCastResult impact tick, EntityDied, and committed PvpStateChanged counters/deadline. Require start-before and impact-at-or-after expiry; no vacuous absent flag success. |
| `peace_rejection_has_no_start`<br>`peace_impact_has_no_damage` | `peace_rejection_has_no_start`: REQUIRED HANDOFF<br>`peace_impact_has_no_damage`: REQUIRED HANDOFF | IntentRejected.reason=PEACE_ZONE/seq without corresponding start; or accepted SkillCastStarted followed by SkillCastCancelled.reason=PEACE_ZONE, and same target life HP/CP unchanged at that impact. Fixture and actor evidence distinguish both endpoint/pulse policies. |
| `pvp_private_stats_coherent` | `pvp_private_stats_coherent`: REQUIRED HANDOFF | Owner StatsChanged.karma/pk_count/pvp_count/flag_until_tick agree with PvpStateChanged and SkillStateSync for the same tick/incarnation; private stats remain owner-only. |
| `karma_gain_matches_oracle`<br>`karma_decay_matches_oracle` | `karma_gain_matches_oracle`: REQUIRED HANDOFF<br>`karma_decay_matches_oracle`: REQUIRED HANDOFF | Committed PvpStateChanged.karma/pk_count/pvp_count with EntityDied, XpGained and StatsChanged.level/xp compared to E1 frozen expected vectors (pre-award level and pre-kill counter). |
| `attacked_by`<br>`npcs_fighting`<br>`players_attacking_me`<br>`npc_target_is_owner`<br>`skill_hate_outcome_coherent` | `attacked_by`: registered<br>`npcs_fighting`: registered<br>`players_attacking_me`: REQUIRED HANDOFF<br>`npc_target_is_owner`: REQUIRED HANDOFF<br>`skill_hate_outcome_coherent`: REQUIRED HANDOFF | AttackStarted.attacker/target and AttackResult, TargetChanged, EntityDied/Despawn; existing attacked_by/npcs_fighting count distinct NPCs from received attack history. No NPC hate ledger on wire; exact AddHate/GetAgro/heal/tie requires actor evidence. |
| `own_death_penalty`<br>`death_penalty_matches_before_disconnect` | `own_death_penalty`: registered<br>`death_penalty_matches_before_disconnect`: REQUIRED HANDOFF | Owner SkillStateSync.death_penalty_level and StatsChanged.death_penalty_level, frozen before disconnect. Deterministic success/failure/cap additionally requires seeded actor vectors. |
| `death_record_matches_fixture`<br>`death_record_matches_before_disconnect`<br>`death_xp_matches_fixture` | `death_record_matches_fixture`: REQUIRED HANDOFF<br>`death_record_matches_before_disconnect`: REQUIRED HANDOFF<br>`death_xp_matches_fixture`: REQUIRED HANDOFF | EntityDied + owner StatsChanged.xp/level and E1 loss vector. pre_death_xp/death_incarnation/recoverable_death_xp_loss are NOT on SkillStateSync; checkpoint/adapter evidence is required for the complete death record. |
| `town_respawn_vitals_match_fixture`<br>`resurrection_vitals_match_fixture` | `town_respawn_vitals_match_fixture`: REQUIRED HANDOFF<br>`resurrection_vitals_match_fixture`: REQUIRED HANDOFF | EntityRespawned + final VitalsChanged and owner StatsChanged.hp/mp/cp compared to frozen external expected vector. Town and skill revival have separate pinned semantics; no client percentage calculation. |
| `resurrect_offer_pending`<br>`resurrect_offer_matches_death`<br>`resurrect_refund_matches_oracle`<br>`resurrection_refunds` | `resurrect_offer_pending`: registered<br>`resurrect_offer_matches_death`: REQUIRED HANDOFF<br>`resurrect_refund_matches_oracle`: REQUIRED HANDOFF<br>`resurrection_refunds`: REQUIRED HANDOFF | Owner ResurrectOffer.target/incarnation/offer_id/restore_percent_q/recoverable_xp/expiry_tick/state/tick, accepted EntityRespawned and owner StatsChanged.xp; correlate one refund per offer/death incarnation against E1 vectors. |

Run `python3 Scripts/sim-parser-check.py Scenarios/3-*.nfs` as the strict source
compatibility gate. It reads BotPredicates/ClassBotPredicates/SkillBotPredicates
and future predicate files from this merged tree; unknown names and known UNKNOWN
projections cause exit1. There is no allowlist for missing names. The host regression
`Scripts/test/phase3-parser.test.py` verifies that rejection over **every** scenario,
including unknown-name and malformed-command mutations. Its PASS attests validator
behavior, not scenario compatibility or the native Unreal parser. The strict gate
currently FAILS; client implementation and vector/wire handoffs are still required.

Current files cover core flows, **not the entire matrix's variants**. Pending
client/engine integration includes exact 50/51 and100/101 checkpoint boundaries,
seeded resist/status/stun/physical-skill fixtures, stronger/lower/overflow/dispel
branches, nonlethal player/NPC DoT, real regen dead/full/NPC variants, full PvP
endpoint/AoE/pulse cross-products, learning cap/competing-kill
and historical-receipt variants, resurrection decline/expiry/replacement/town
race/crash matrix, and AddHate/heal/GetAgro/tie variants. These require published
runtime/data/vector/DSL contracts; source files and covers headers are not pass
attestations. Actor/adapter precision tests remain the server owners' work.

Schema3 coverage consumers use `player_cast_states` and `effect_states` plus the
maps listed in sim-gates.py. They retain every schema2 field, reject missing or
unknown facts, validate Cartesian reachability, deduplicate canonical recordings,
and reject mixed-schema merges. Schema2 remains readable without invented zero
Phase3 observations. **TODO-RECONCILE producer field names** when Z publishes
coverage.rs. Synthetic schema3 test fixtures are not native replay evidence.

Host checks: `TMPDIR=/tmp python3 Scripts/test/phase3-fixture.test.py`,
`python3 Scripts/test/proto-optionals.test.py`, `python3 Scripts/test/transition-v3.test.py`.
These use command stubs and do not launch Unreal/API/Compose. While the plan owner
reviews Phase 3, run `python3 Scripts/sim-traceability.py --extra-plan
<phase3-plan-worktree>/docs/plans/phase-3-combat-and-skills.md`. The default checker
still rejects unknown Phase3 tags until that plan is integrated; the regression
test uses an explicit stable-title/matrix snapshot, without marking any row complete.
Peace checks additionally require the engine's published inclusive square configuration;
the current legacy zone loader has no peace fields. Reconcile that overlay when its
schema lands. Resist/penalty branches require published seed/vector controls; they
are not deterministic from a client login's wall-clock timing.

Casting soak defaults to eight clients and1200s, all roles at fixture level7 casting
Wind Strike and long Might buffs. The low-HP NPC keeps source XP/SP rewards; the
role's received kill/XP/reuse/effect checks contain no gameplay formulas and do not
require the currently absent MP-cost metadata. `soak-fixtures.py --casting` emits mode0600 tokens and an exact
`roles.json` manifest; seed-phase3-soak.py validates canonical inputs/unique UUIDs
and seeds all roles in one fresh transaction before API startup. The casting
NPC overlay has HP1 as an explicit test input; its source rewards are unchanged.
Gauntlet/report filenames stay `1-kill-one-monster` for existing artifact consumers.
Each role must supply positive cast_skill, skill_cast_result and effect_applied
runtime counts, plus verified native schema3 cast/effect transitions. Missing
casting evidence fails, including a missing role manifest and a transition report
whose SHA-256 does not identify the replay-verified recording. `--legacy-attacks`
retains the old workload and cannot count as Phase3 acceptance. A shorter or smaller
casting smoke run can pass its own checks but `phase3_acceptance` stays false until
eight clients each run at least1200s with every evidence gate passing. Standalone
seeders remove inherited libpq settings before supplying the guarded endpoint.

**Client-owned NightfallSoak.cs handoff:** consume `-SoakRoleManifest=<roles.json>`,
validate schema_version1/casting/count/order, use each role's scenario and
`-DevTokenFile=<token_file>` (never inline tokens), retain unique UserDir/BotOutDir,
and allow the final360s casting scenario plus boot grace (replace the earlier120s
request: the full sum of movement/cast/state waits now fits the scenario budget).
Consume `-SoakScenarioSeconds=360` and include that final scenario allowance plus
boot grace in both GetConfiguration.MaxDuration and TickTest's process deadline.
Do not change client
skill formulas. Harness run-soak.sh refuses casting before infrastructure if the
client source lacks this hook. C# and its build are outside C-HARNESS ownership;
no Gauntlet/packaged soak pass is claimed. Merged client M3 `3b21cd7` provides core DSL/manifest support; its C# still lacks
the final SoakScenarioSeconds budget parameter. Commands/counts/inactive effects match the
brief, but the added impact/initial-resource/causal predicates and oracle/death/
retention/AI assertions remain handoffs. Integrate and reconcile the final client
M3 commit before running scenarios; unsupported names must continue to fail parsing.
