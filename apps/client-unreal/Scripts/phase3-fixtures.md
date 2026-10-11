Phase 3 uses the same isolated fixture lifecycle as [phase2-fixtures.md](phase2-fixtures.md).
Fixtures migrate and seed **before API startup/admission**. They require `--api start`
(`run-sim-ci.py --fresh-stack`), distinct role accounts, fresh DB/JetStream volumes,
nondefault loopback ports, private token files, and owned teardown. The wrappers
never modify admitted characters. `SIM_PHASE3_SEEDER` selects the published seeder;
the default is `infra/scripts/seed-phase3-fixture.py`. Only explicit dev-token opt-in
plus a loopback `nf_phase3_fixture_*` DB on a non-5432 port allows a seed.

**Integration fix status:** Every generated NPC now inherits `sp_reward` and the
complete committed keltir Phase 3 suffix, including provenance. The capped penalty
pack stores the typed `{"kind":"death_penalty"}` installation source in
`characters.modifier_install_order`; other fresh packs retain empty modifier order.
`PHASE3_ENABLED=1` selects Phase 3 packs, and `0` explicitly selects older packs.
The Rust regression `apps/api/tests/phase3_fixture_overlays.rs` invokes the Python
generator, validates the complete overlay via `prepare_phase3_bootstrap`, and tests
all packs/paired roles plus casting soak through actual `stage_restore`,
`SpawnPlayer`, and cold snapshot restore. Missing profile/SP/penalty source are
negative cases. Postgres-backed admission and live Unreal/soak remain unrun in this
fix session because Docker access is unavailable.

| Exact header | Roles and persisted inputs |
|---|---|
| `# fixture: phase3-trainer` | Human Mystic class10/base10, level14, SP10000, at126,128. Wind Strike1177 rank1, Ice Bolt1184/Might1068/Shield1040/Curse Weakness1164 rank1. **1177 rank2 is unlearned**, cost240/required level7 from professions/human_mystic.toml. Optional matching A/B observer pair. |
| `# fixture: phase3-pvp` | Two distinct Human Mystic accounts level14 at110,100 and112,100, outside inclusive peace square116..136. Owner SP10000,1177 rank5/1184 rank4/1068 rank1. Peer SP0,1177 rank1,HP100,CP10. |
| `# fixture: phase3-resurrect` | Human Mystic class10/base10 level30 at126,128 with1068/1216 rank1; caster performs real Cleric15 transfer and learns1016 rank1 before moving to110,100; distinct living peer level14 at112,100,HP100,CP10. Peer dies through ordinary sentinel retaliation; no precreated offer or fabricated death loss. |
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
committed skill-oracle.json is available, but live replay formula inputs and ordered draw evidence are still a required producer handoff; no live oracle pass is claimed.

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
Client waits use source budgets. Post-run replay oracle comparisons run only against authoritative entity/tick evidence.

**Final authoritative DSL handoff (merged client `3b21cd7`).** This single table
covers every command and predicate used by the Phase3 scenarios. Names use the
existing registry where semantics match: `rejected`, `private_stats_leaks`, and
`own_hp > 0`. Numeric comparisons use `<op> <unsigned n>`; explicit skills use
`<skill_id>` and entities use `<own|target|uuid>`. The twelve server-only names in C11 are replay-gate declarations, not client
predicates. Unknown DSL names still fail parsing; no UNKNOWN allowlisting is used.
All history is fenced by transport generation, entity incarnation, tick and the
cast/effect/offer identity. Vector comparisons consume expected values from an
independent, hash-pinned artifact; the replay consumer compares raw authoritative inputs/outcomes with independent source expectations.

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
| `in_world`<br>`reconnects` | `in_world`: registered<br>`reconnects`: registered | WebSocket transport generation and owner EntitySpawn.entity_id/life_incarnation; no synthetic gameplay facts. |
| `own_at`<br>`entity_at` | `own_at`: registered<br>`entity_at`: REQUIRED HANDOFF | EntitySpawn.entity_id/position/life_incarnation and EntityMove.entity_id/position/tick scoped to the admitted identity; newest received sample, never client pawn position. |
| `entity_in_view` | `entity_in_view`: REQUIRED HANDOFF | EntitySpawn/EntityDespawn admitted known-entity identity cache. |
| `own_hp`<br>`own_dead`<br>`entity_alive`<br>`entity_dead` | `own_hp`: registered<br>`own_dead`: registered<br>`entity_alive`: REQUIRED HANDOFF<br>`entity_dead`: REQUIRED HANDOFF | EntitySpawn.hp, EntityDied.entity/incarnation, EntityRespawned, AttackResult.target_hp_after and VitalsChanged.hp; alive requires a known matching incarnation. |
| `own_mp`<br>`own_cp`<br>`initial_mp`<br>`initial_cp`<br>`initial_hp` | `own_mp`: registered<br>`own_cp`: registered<br>`initial_mp`: REQUIRED HANDOFF<br>`initial_cp`: REQUIRED HANDOFF<br>`initial_hp`: REQUIRED HANDOFF | Owner StatsChanged.mp/cp and EntitySpawn.hp; initial_* freezes the first owner admission sample, never an uninitialized zero. |
| `target` | `target`: registered | Owner TargetChanged.target; server-confirmed selection. |
| `attack_state` | `attack_state`: registered | IntentAck/IntentRejected.seq correlated to Attack, AttackStarted.attacker/tick, AttackResult and AttackCancelled.reason; pending is local intent state. |
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
| `cast_state`<br>`entity_casting` | `cast_state`: registered<br>`entity_casting`: REQUIRED HANDOFF | SkillCastStarted.caster/skill_id/cast_id/caster_incarnation/complete_tick, SkillCastResult and SkillCastCancelled of the same identity. Baseline hydration upserts without event counts. |
| `cast_starts`<br>`cast_results`<br>`cast_cancellations`<br>`observed_cast_results` | `cast_starts`: registered<br>`cast_results`: registered<br>`cast_cancellations`: registered<br>`observed_cast_results`: registered | Distinct nonbaseline SkillCastStarted/SkillCastResult/SkillCastCancelled; owner versus observer audience. Generic counters are insufficient for skill-specific observer termination. |
| `cast_cancel_reason`<br>`observed_cast_cancel_reason`<br>`cast_dispositions_once` | `cast_cancel_reason`: REQUIRED HANDOFF<br>`observed_cast_cancel_reason`: REQUIRED HANDOFF<br>`cast_dispositions_once`: REQUIRED HANDOFF | SkillCastStarted.cast_id + SkillCastResult.cast_id or SkillCastCancelled.cast_id/reason, with caster/incarnation and IntentAck/IntentRejected.seq. Exactly one terminal event per accepted cast; no late intent rejection. |
| `cancelled_cast_costs_retained`<br>`own_mp_cost_matches` | Received-state predicates | Owner StatsChanged.mp, SkillReuse.ready_tick, cast identity, and per-rank costs from authoritative GetSkillCatalog (architecture C6). |
| `skill_damage_matches_oracle`<br>`skill_effect_matches_oracle`<br>`stat_matches_oracle` | `skill_damage_matches_oracle`: C11 replay gate<br>`skill_effect_matches_oracle`: C11 replay gate<br>`stat_matches_oracle`: C11 replay gate | SkillCastResult.targets[].resolution_kind/outcome/rolled/cp_absorbed/hp_removed/heal/applied_effect_ids + EffectApplied + owner StatsChanged.effective_stats compared to independent hash-pinned E1 vectors. No vector data has been supplied; no local gameplay formulas. |
| `buff_active`<br>`buff_inactive`<br>`debuff_active`<br>`entity_effect_active`<br>`entity_effect_inactive`<br>`effect_count` | `buff_active`: registered<br>`buff_inactive`: registered<br>`debuff_active`: registered<br>`entity_effect_active`: registered<br>`entity_effect_inactive`: registered<br>`effect_count`: registered | EffectApplied.target/effect_id/skill_id/pool/end_tick and EffectRemoved.effect_id plus EntitySpawn.active_effects or baseline hydration. Absence only matches a known baseline, never missing state. |
| `buff_replacements`<br>`replacement_remove_before_apply`<br>`effect_identity_coherent`<br>`effect_remove_reason` | `buff_replacements`: REQUIRED HANDOFF<br>`replacement_remove_before_apply`: REQUIRED HANDOFF<br>`effect_identity_coherent`: REQUIRED HANDOFF<br>`effect_remove_reason`: REQUIRED HANDOFF | EffectRemoved.target/incarnation/effect_id/reason/tick then EffectApplied with a NEW effect_id; retained stable IDs/deadlines and observed order, not elapsed client time. |
| `retained_effect_identity_coherent`<br>`restore_did_not_reapply_hooks` | `retained_effect_identity_coherent`: received-state predicate<br>`restore_did_not_reapply_hooks`: C11 replay gate | Baseline EffectApplied.skill_id/skill_level/pool/caster/applied_tick/end_tick and owner reuse sync compared before/after admission plus replay/checkpoint adapter evidence. Wire lacks modifier install order and periodic phase; those MUST come from server precision evidence. |
| `owner_skill_baseline_coherent`<br>`observer_skill_baseline_coherent`<br>`spawn_before_skill_references` | `owner_skill_baseline_coherent`: REQUIRED HANDOFF<br>`observer_skill_baseline_coherent`: REQUIRED HANDOFF<br>`spawn_before_skill_references`: REQUIRED HANDOFF | Identity EntitySpawn/EntityRespawned batch, baseline SkillCastStarted/EffectApplied/PvpStateChanged and owner SkillStateSync/StatsChanged; every nested reference has an already-known matching incarnation, omitted hidden source pairs remain absent. |
| `public_vitals_coherent`<br>`public_revival_vitals_coherent` | `public_vitals_coherent`: REQUIRED HANDOFF<br>`public_revival_vitals_coherent`: REQUIRED HANDOFF | EntitySpawn final hp/max_hp; EntityRespawned then baseline VitalsChanged.hp/max_hp/tick/incarnation, agreeing with public SkillCastResult target_hp_after. Never observer MP/CP. |
| `regen_pulses`<br>`moving_regen_pulses`<br>`regen_matches_oracle`<br>`moving_regen_matches_oracle` | `regen_pulses`: C11 replay gate<br>`moving_regen_pulses`: C11 replay gate<br>`regen_matches_oracle`: C11 replay gate<br>`moving_regen_matches_oracle`: C11 replay gate | Owner StatsChanged.hp/mp/cp and VitalsChanged.hp/max_hp with EntityMove samples and independent E1 vectors; wire has NO regen-cause/pulse event. Exact pulse/moving/dead/full distinctions additionally need correlated actor/replay regen facts; do not infer a pulse from any HP increase. |
| `own_flag`<br>`target_flag`<br>`target_flag_remaining`<br>`own_karma`<br>`own_pk`<br>`own_pvp`<br>`own_flag_deadline_matches_received` | `own_flag`: registered<br>`target_flag`: registered<br>`target_flag_remaining`: REQUIRED HANDOFF<br>`own_karma`: registered<br>`own_pk`: registered<br>`own_pvp`: registered<br>`own_flag_deadline_matches_received`: REQUIRED HANDOFF | PvpStateChanged.entity/flag_until_tick/karma/pk_count/pvp_count/tick/incarnation plus EntitySpawn.flag_until_tick/karma and owner SkillStateSync. remaining uses received tick only; classify chaotic from received karma. |
| `own_cp_damage_total`<br>`player_cp_damage_total`<br>`cp_absorption_matches_received` | `own_cp_damage_total`: REQUIRED HANDOFF<br>`player_cp_damage_total`: REQUIRED HANDOFF<br>`cp_absorption_matches_received`: REQUIRED HANDOFF | Player-sourced AttackResult.cp_damage/damage/target_hp_after and SkillCastResult.targets[].cp_absorbed/hp_removed/target_hp_after with owner StatsChanged.cp. Nonzero witnessed absorption required. |
| `victim_flag_not_refreshed`<br>`flagged_kill_matches_received`<br>`impact_counter_uses_received_flag`<br>`target_flag_expired_at_last_impact`<br>`own_flag_expired_at_last_impact` | `victim_flag_not_refreshed`: REQUIRED HANDOFF<br>`flagged_kill_matches_received`: REQUIRED HANDOFF<br>`impact_counter_uses_received_flag`: REQUIRED HANDOFF<br>`target_flag_expired_at_last_impact`: REQUIRED HANDOFF<br>`own_flag_expired_at_last_impact`: REQUIRED HANDOFF | AttackStarted or SkillCastStarted at a witnessed pre-expiry tick, matching AttackResult/SkillCastResult impact tick, EntityDied, and committed PvpStateChanged counters/deadline. Require start-before and impact-at-or-after expiry; no vacuous absent flag success. |
| `peace_rejection_has_no_start`<br>`peace_impact_has_no_damage` | `peace_rejection_has_no_start`: REQUIRED HANDOFF<br>`peace_impact_has_no_damage`: REQUIRED HANDOFF | IntentRejected.reason=PEACE_ZONE/seq without corresponding start; or accepted SkillCastStarted followed by SkillCastCancelled.reason=PEACE_ZONE, and same target life HP/CP unchanged at that impact. Fixture and actor evidence distinguish both endpoint/pulse policies. |
| `pvp_private_stats_coherent` | `pvp_private_stats_coherent`: REQUIRED HANDOFF | Owner StatsChanged.karma/pk_count/pvp_count/flag_until_tick agree with PvpStateChanged and SkillStateSync for the same tick/incarnation; private stats remain owner-only. |
| `karma_gain_matches_oracle`<br>`karma_decay_matches_oracle` | `karma_gain_matches_oracle`: REQUIRED HANDOFF<br>`karma_decay_matches_oracle`: REQUIRED HANDOFF | Committed PvpStateChanged.karma/pk_count/pvp_count with EntityDied, XpGained and StatsChanged.level/xp compared to E1 frozen expected vectors (pre-award level and pre-kill counter). |
| `attacked_by`<br>`npcs_fighting`<br>`players_attacking_me`<br>`npc_target_is_owner`<br>`skill_hate_outcome_coherent` | `attacked_by`: received-state predicate<br>`npcs_fighting`: received-state predicate<br>`players_attacking_me`: received-state predicate<br>`npc_target_is_owner`: received-state predicate<br>`skill_hate_outcome_coherent`: C11 replay gate | AttackStarted.attacker/target and AttackResult, TargetChanged, EntityDied/Despawn; existing attacked_by/npcs_fighting count distinct NPCs from received attack history. No NPC hate ledger on wire; exact AddHate/GetAgro/heal/tie requires actor evidence. |
| `own_death_penalty`<br>`death_penalty_matches_before_disconnect` | `own_death_penalty`: registered<br>`death_penalty_matches_before_disconnect`: REQUIRED HANDOFF | Owner SkillStateSync.death_penalty_level and StatsChanged.death_penalty_level, frozen before disconnect. Deterministic success/failure/cap additionally requires seeded actor vectors. |
| `death_record_matches_fixture`<br>`death_record_matches_before_disconnect`<br>`death_xp_matches_fixture` | `death_record_matches_fixture`: C11 replay gate<br>`death_record_matches_before_disconnect`: C11 replay gate<br>`death_xp_matches_fixture`: received-state predicate | EntityDied + owner StatsChanged.xp/level and E1 loss vector. pre_death_xp/death_incarnation/recoverable_death_xp_loss are NOT on SkillStateSync; checkpoint/adapter evidence is required for the complete death record. |
| `town_respawn_vitals_match_fixture`<br>`resurrection_vitals_match_fixture` | `town_respawn_vitals_match_fixture`: REQUIRED HANDOFF<br>`resurrection_vitals_match_fixture`: REQUIRED HANDOFF | EntityRespawned + final VitalsChanged and owner StatsChanged.hp/mp/cp compared to frozen external expected vector. Town and skill revival have separate pinned semantics; no client percentage calculation. |
| `resurrect_offer_pending`<br>`resurrect_offer_matches_death`<br>`resurrect_refund_matches_oracle`<br>`resurrection_refunds` | `resurrect_offer_pending`: received-state predicate<br>`resurrect_offer_matches_death`: received-state predicate<br>`resurrect_refund_matches_oracle`: C11 replay gate<br>`resurrection_refunds`: received-state predicate | Owner ResurrectOffer.target/incarnation/offer_id/restore_percent_q/recoverable_xp/expiry_tick/state/tick, accepted EntityRespawned and owner StatsChanged.xp; correlate one refund per offer/death incarnation against E1 vectors. |

| `own_level` | Registered | Owner StatsChanged.level and EntitySpawn.level; received level only. |
| `target_distance` | Registered | TargetChanged selection plus newest EntitySpawn/EntityMove positions for owner and selected target; scheduling/range evidence, no skill formula. |
| `resurrect_offer_state` | REQUIRED HANDOFF | Owner ResurrectOffer.offer_id/target_incarnation/state/tick for the newest witnessed offer, preserving DECLINED/EXPIRED terminal history; absent/pending cannot satisfy it. |

| `cast_result_for <own|uuid> <skill_id>` | REQUIRED HANDOFF | Latch a witnessed SkillCastStarted.caster/caster_incarnation/skill_id/**cast_id**, including an explicit baseline; require SkillCastResult matching all four. A Might result or different cast cannot satisfy Wind Strike. UNKNOWN until both start and matching result exist. |
| `cast_terminal_once_for <own|uuid> <skill_id>` | REQUIRED HANDOFF | Same latched cast identity as cast_result_for; count exactly one matching SkillCastResult or SkillCastCancelled, require the designated result actually witnessed. Hydration is never a new start. |
| `observed_cast_baseline_for <uuid> <skill_id>` | REQUIRED HANDOFF | SkillCastStarted.baseline=true, original start_tick < baseline_tick < complete_tick, same caster/incarnation/skill/cast_id on first AOI hydration; preserve identity for subsequent terminal correlation. An ordinary Started or completed baseline cannot pass. |
| `entity_not_in_view <uuid>` | REQUIRED HANDOFF | Previously witnessed EntitySpawn.entity_id then EntityDespawn.entity_id removal from the admitted identity cache; before a valid transport baseline UNKNOWN, never success from absent initialization. |

| `cast_damage_results` (casting soak) | Registered | Owner SkillCastResult.targets[].resolution_kind PHYSICAL/MAGICAL with witnessed cast identity; nonbaseline result history, no calculation of damage. |
| `proxies` (casting soak) | Registered | Net admitted EntitySpawn.entity_id/EntityDespawn.entity_id cache, excluding owner. |
| `xp_known`, `own_xp` (casting soak) | Registered | Owner StatsChanged.xp / XpGained.total and scope reset after admission; missing owner sample UNKNOWN. |
| `kill_xp_matches_fixture`, `own_level_matches_xp` (casting soak) | Registered legacy fixture predicates | Received XpGained/StatsChanged.xp/level and selected NPC EntitySpawn.template_id plus the existing BotFixtureData shipped XP/experience rule contract. No new Phase3 formula or Python/Bash gameplay calculation. |

Run `python3 Scripts/sim-parser-check.py Scenarios/3-*.nfs` as the strict source
compatibility gate. It reads BotPredicates/ClassBotPredicates/SkillBotPredicates
and future predicate files from this merged tree; unknown names and known UNKNOWN
projections cause exit1. There is no allowlist for missing names. The host regression
`Scripts/test/phase3-parser.test.py` verifies that rejection over **every** scenario,
including unknown-name and malformed-command mutations. Its PASS attests validator
behavior, not scenario compatibility or the native Unreal parser. The strict gate
currently FAILS; client implementation and vector/wire handoffs are still required.

The canonical17 files and `Scenarios/variants/3-*.nfs` are source contracts,
not execution attestations. `Scenarios/phase3-variants.json` maps named variants
back to the canonical matrix row, exact required assertions, pre-admission pack,
and mandatory actor/adapter precision reports. CI and traceability discover nested
`.nfs` files. Orphan variants, missing assertions, changed fixture headers, and
insufficient wait budgets fail validation. A covers header by itself cannot pass.

Additional seed packs: `phase3-trainer-low-sp` (SP0),
`phase3-trainer-low-level` (level4), `phase3-trainer-low-mp` (MP0,110/100),
`phase3-penalty-cap` (untimed penalty5,110/100), `phase3-sp-near-cap`
(SP=i64MAX-1,110/100). They retain the original guarded pre-admission lifecycle.
Low-MP asserts received MP<9 before Wind Strike; if login/regen erased the input,
the scenario fails rather than pretending a resource rejection was exercised.
The capped penalty scenario requires **exactly5** before death, after death and
reconnect; an always-zero implementation cannot pass. Seed SQL writes declared
penalty/PvP inputs rather than silently substituting zero.

Deterministic land/resist and penalty-success/failure fixture definitions are in
`precision_cases[].deterministic_roll_fixture`: test-only actor draw-site injection0/99 with inclusive bounds0..99. For death
penalty this follows binding design v3 §3 roll_below(100)<20; the fixture also
records corresponding pinned source Rnd.get(1,100) values1/100
(L2PcInstance.java:9363). Ineligible/capped deaths consume no draw per binding design, requiring the **complete ordered RNG trace**, including earlier
success/reflection/duration draws. They are **actor fixture handoffs**, not supported
live-server seed controls: no chosen wall-clock login or small cast sample proves
an outcome. Engine owners must consume these fixtures and publish the named test
and hash-linked actual trace. No test-only RNG override enters production WS/DB.

Run `python3 Scripts/phase3-variants.py` for source validation. Run it with
`--evidence <precision-report-dir>` for required evidence completeness. Missing
reports, missing expected assertions, wrong fixture hashes, missing retained test
artifacts and missing actual injected-draw traces fail. Every `evidence: null`
means **not tested**; source validation never changes that status. Reports name the
requested test, canonical case hash, boolean verdict for every listed invariant,
and relative artifact->SHA256 links. These reports supplement native recording,
byte replay, pipeline and contract gates; they do not replace them.

| Named runtime variant | Canonical matrix file | Fixture / required behavior |
|---|---|---|
| `cast-unlearned` | `3-cast-rejected.nfs` | `phase3-trainer`; `nf.WaitFor rejected == skill_not_learned 3`; `nf.Expect cast_starts == 0` |
| `cast-invalid-target` | `3-cast-rejected.nfs` | `phase3-trainer`; `nf.WaitFor rejected == invalid_target_type 3`; `nf.Expect cast_starts == 0` |
| `cast-low-mp` | `3-cast-rejected.nfs` | `phase3-trainer-low-mp`; `nf.Expect own_mp < 9`; `nf.WaitFor rejected == insufficient_mp 3`; `nf.Expect cast_starts == 0` |
| `cast-range` | `3-cast-rejected.nfs` | `phase3-trainer`; `nf.Expect target_distance > 30`; `nf.WaitFor rejected == out_of_range 3`; `nf.Expect cast_starts == 0` |
| `cast-move-cancel` | `3-cast-rejected.nfs` | `phase3-trainer`; `nf.WaitFor cast_cancel_reason == moved 3`; `nf.Expect cancelled_cast_costs_retained`; `nf.Expect cast_results == 0` |
| `learn-low-sp` | `3-learn-rejected.nfs` | `phase3-trainer-low-sp`; `nf.Expect own_sp == 0`; `nf.Expect learned_rank 1177 == 1`; `nf.Expect skill_learned_events == 0` |
| `learn-low-level` | `3-learn-rejected.nfs` | `phase3-trainer-low-level`; `nf.Expect own_level == 4`; `nf.Expect own_sp == 10000`; `nf.Expect skill_learned_events == 0` |
| `learn-unknown` | `3-learn-rejected.nfs` | `phase3-trainer`; `nf.WaitFor learn_rpc_error == invalid_argument 15`; `nf.Expect own_sp == 10000`; `nf.Expect skill_learned_events == 0` |
| `learn-key-conflict` | `3-learn-rejected.nfs` | `phase3-trainer`; `nf.WaitFor learn_rpc_error == failed_precondition 15`; `nf.Expect own_sp == 9760`; `nf.Expect skill_learned_events == 1` |
| `learn-casting` | `3-learn-rejected.nfs` | `phase3-trainer`; `nf.WaitFor cast_state == casting 12`; `nf.Expect own_sp == 10000`; `nf.Expect skill_learned_events == 0` |
| `learn-dead` | `3-learn-rejected.nfs` | `phase3-trainer`; `nf.WaitFor own_dead 90`; `nf.Expect own_sp == 10000`; `nf.Expect skill_learned_events == 0` |
| `death-penalty-cap` | `3-death-penalty.nfs` | `phase3-penalty-cap`; `nf.Expect own_death_penalty == 5`; `# replay-gate: death_record_matches_before_disconnect` |
| `resurrect-decline-a` | `3-resurrect-a.nfs` | `phase3-resurrect`; `nf.Expect entity_dead 01970000-0000-7000-8000-000000000031`; `nf.Expect cast_results == 1` |
| `resurrect-decline-b` | `3-resurrect-b.nfs` | `phase3-resurrect`; `nf.WaitFor resurrect_offer_state == declined 3`; `nf.Expect resurrection_refunds == 0`; `nf.Expect own_dead` |
| `resurrect-expiry-a` | `3-resurrect-a.nfs` | `phase3-resurrect`; `nf.Expect entity_dead 01970000-0000-7000-8000-000000000031`; `nf.Expect cast_results == 1` |
| `resurrect-expiry-b` | `3-resurrect-b.nfs` | `phase3-resurrect`; `nf.WaitFor resurrect_offer_state == expired 180`; `nf.Expect resurrection_refunds == 0`; `nf.Expect own_dead` |
| `learn-sp-near-cap` | `3-learn-skill.nfs` | `phase3-sp-near-cap`; `nf.WaitFor sp_gained_total == 1 90`; `nf.Expect own_sp == 9223372036854775807`; `nf.Expect sp_gained_events == 1`; `nf.Expect sp_gained_events == 0` |

| Required precision variant (all pending engine/adapter execution) | Matrix file | Exact required named test / setup |
|---|---|---|
| `cast-reuse-boundary` | `3-cast-rejected.nfs` | `phase3_cast_reuse_boundary`; reuse ready-1 / ready tick |
| `cast-status-guards` | `3-cast-rejected.nfs` | `phase3_cast_status_guards`; stun/silence/physical-mute statuses independently |
| `cast-real-weapon` | `3-cast-rejected.nfs` | `phase3_cast_real_weapon`; actual Squire sword versus using kind condition |
| `cast-completion-resources` | `3-cast-rejected.nfs` | `phase3_cast_completion_resources`; accepted start then resources consumed by an ordinary competing action |
| `cast-completion-range` | `3-cast-rejected.nfs` | `phase3_cast_completion_range`; target crosses effectRange after accepted start |
| `cast-damage-break` | `3-cast-rejected.nfs` | `phase3_cast_damage_break`; pinned hitCancelTime cutoffs with injected break rolls |
| `cast-already-casting` | `3-cast-rejected.nfs` | `phase3_cast_already_casting`; different learned ready skill while existing cast pending |
| `learn-lineage-item-capability` | `3-learn-rejected.nfs` | `phase3_learn_lineage_item_capability`; valid other-lineage skill, item-learning skill, deferred skill separately |
| `learn-engagement-guards` | `3-learn-rejected.nfs` | `phase3_learn_engagement_guards`; incoming hate/swings/readiness/hostile effects within trainer radius |
| `learn-auth-generation` | `3-learn-rejected.nfs` | `phase3_learn_auth_generation`; missing token / foreign character / missing character / stale session generation |
| `learn-queued-duplicates` | `3-learn-skill.nfs` | `phase3_learn_queued_duplicates`; same UUID twice then conflicting body; two keys same rank |
| `learn-historical-receipt` | `3-learn-skill.nfs` | `phase3_learn_historical_receipt`; retry same successful key after later rank/class/progression/disconnect |
| `sp-competing-keltir` | `3-pvp-flag-a.nfs` | `phase3_sp_competing_keltir`; two living players hit one keltir; A nonlethal then B lethal; reverse roles |
| `sp-signed-cap` | `3-pvp-flag-b.nfs` | `phase3_sp_signed_cap`; killer SP=i64MAX-1, other role0; reverse roles |
| `buff-higher-lower-overflow` | `3-buff-expires.nfs` | `phase3_buff_higher_lower_overflow`; rank1 then rank2 then rank1 PA_UP; full buff pool then next distinct buff |
| `buff-dispel-capped-heal` | `3-buff-expires.nfs` | `phase3_buff_dispel_capped_heal`; capped actual heal and DispelBySlot/Category on supported effects |
| `debuff-land` | `3-debuff-status.nfs` | `phase3_debuff_land`; eligible target and injected calcEffectSuccess bounded draw0 |
| `debuff-resist` | `3-debuff-status.nfs` | `phase3_debuff_resist`; same eligible target and injected calcEffectSuccess bounded draw99 |
| `debuff-immune` | `3-debuff-status.nfs` | `phase3_debuff_immune`; immune target trait, independent source vector |
| `physical-skill` | `3-debuff-status.nfs` | `phase3_physical_skill`; real supported weapon; physical evasion0 and injected nonzero branch |
| `status-stun-root-overlap` | `3-debuff-status.nfs` | `phase3_status_stun_root_overlap`; same tick stun versus swing, root versus movement, overlapping status expiries |
| `dot-nonlethal-break` | `3-debuff-status.nfs` | `phase3_dot_nonlethal_break`; TickHp on player/NPC atHP1 withCP0/positive and direct damage/DoT |
| `regen-standing-moving-npc` | `3-regen.nfs` | `phase3_regen_standing_moving_npc`; 30tick player HP/MP/CP; moving and NPC HP/MP, multiple fractional pulses |
| `regen-dead-full` | `3-regen.nfs` | `phase3_regen_dead_full`; dead and full player/NPC resources across multiple pulse boundaries |
| `reconnect-reuse-50-51` | `3-cast-reconnect.nfs` | `phase3_reconnect_reuse_50_51`; freeze checkpoint at remaining50 then51 via actor scheduler |
| `reconnect-effects-100-101` | `3-cast-reconnect.nfs` | `phase3_reconnect_effects_100_101`; buff and debuff each at remaining100 then101, with periodic phase/install order |
| `replacement-live-cast-offer` | `3-cast-reconnect.nfs` | `phase3_replacement_live_cast_offer`; replace session during pending cast and valid resurrection offer |
| `death-penalty-roll-success` | `3-death-penalty.nfs` | `phase3_death_penalty_roll_success`; NPC or chaotic-player death; injected eligible roll0; old penalty0 |
| `death-penalty-roll-failure` | `3-death-penalty.nfs` | `phase3_death_penalty_roll_failure`; same eligible death; injected roll99; old penalty0 |
| `death-penalty-ineligible` | `3-death-penalty.nfs` | `phase3_death_penalty_ineligible`; neutral-player death and peace endpoints independently |
| `death-penalty-cap-actor` | `3-death-penalty.nfs` | `phase3_death_penalty_cap_actor`; eligible NPC death; old penalty5, success and failure rolls |
| `resurrect-exact-expiry` | `3-resurrect-b.nfs` | `phase3_resurrect_exact_expiry`; accept at expiry-1 and expiry tick using actor scheduler |
| `resurrect-competing-offers` | `3-resurrect-a.nfs` | `phase3_resurrect_competing_offers`; two caster completions ordered same tick for same corpse |
| `resurrect-town-race` | `3-resurrect-b.nfs` | `phase3_resurrect_town_race`; queued town respawn and accept in both ordinal orders |
| `resurrect-disconnect-new-offer` | `3-resurrect-b.nfs` | `phase3_resurrect_disconnect_new_offer`; disconnect vs replacement while offered; reconnect then new offer |
| `resurrect-crash-once` | `3-resurrect-b.nfs` | `phase3_resurrect_crash_once`; log/checkpoint/receipt/outbox lost-ack/crash stages via adapters |
| `hate-effectpoint-addhate` | `3-aggro-skill-hate.nfs` | `phase3_hate_effectpoint_addhate`; multiple NPC ledgers, two owners; effectPoint and AddHate |
| `hate-heal-overheal` | `3-aggro-skill-hate.nfs` | `phase3_hate_heal_overheal`; all NPCs hating healed target; actual heal and zero overheal |
| `hate-tie-getagro` | `3-aggro-skill-hate.nfs` | `phase3_hate_tie_getagro`; tied ledgers current target versus none, GetAgro request ordinal interactions |
| `peace-endpoint-aoe-pulse` | `3-pvp-flag-a.nfs` | `phase3_peace_endpoint_aoe_pulse`; caster/target/both inclusive square; start versus impact/AoE/pulse |
| `karma-no-decay-paths` | `3-pvp-karma-decay.nfs` | `phase3_karma_no_decay_paths`; chaotic-unflagged kill, healer, time, death vs monster XP pre-award level |

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

Observer correlation correction: canonical A/B both wait for `cast_result_for`
Wind Strike1177, then `cast_terminal_once_for` on the same received identity.
A remains admitted until B's post-result Might1068 self-buff is received. The
trainer peer is explicitly seeded with Might rank1; no unlearned handshake.
`variants/3-cast-observer-late-a/b.nfs` use `phase3-observer-late`: A110/100,
B65/100->63/100->65/100 across the32-tile AOI cell boundary. They must witness
original pending-cast hydration strictly during the cast or fail; ordinary Start
cannot stand in for mid-cast entry. Runtime frame scheduling can miss this window;
`phase3_observer_mid_cast_entry` must additionally prove exact actor-tick ordering.

`Scenarios/fixtures/phase3-observer-mutual.json` is an **authored wire fixture**,
not an engine-produced recording. It defines two mutually referencing newcomers,
all identity spawns before either baseline cast, different incarnations/IDs,
original deadlines, a misleading Might result and the two designated Wind Strike
terminal results. Its SHA is bound into the named `observer-mutual-reference`
precision case. Engine/adapter/client owners must consume it and execute the
negative mutations (interleaved hydration, stale incarnation/wrong cast or skill,
ordinary start, completed baseline). The companion mutual-swing/hidden-source
precision case covers pending-swing hydration and absent-source pairs. These three
precision cases are evidence:null; the source fixes do not claim native observer
or full finding3 acceptance until the required client predicates and traces land.

C11 post-run gate contract (integration fix)
------------------------------------------

`# replay-gate: <name> [arguments]` moves server-only assertions out of the DSL.
The wrappers run `phase3-evidence.py` after replay/coverage, bind facts to the
current JUnit `own_entity_id` and the unit recording SHA, and classify missing or
mismatched evidence as `phase3_replay_evidence` infrastructure failure. Gates are
mandatory even with `--no-replay`; affected units require exactly one recording.
Received cast/resource/effect/offer/target assertions remain in each scenario.
Static registry audit passes all Phase 3 scenario files without an allowlist.
That audit proves source compatibility only.

The producer command is `nightfall-replay export --file <unit.nfr> --session
<entity> --evidence-out <gate.raw.jsonl>` after an independent `check` on the same
file. `SIM_PHASE3_EVIDENCE_CMD` overrides the CLI prefix. The exporter checks the
whole recording before publishing a fresh destination. The binding JSONL schema,
source anchors and private-data contract are in
[replay-evidence.md](../../../docs/engineering/replay-evidence.md).

The consumer requires a manifest bound to the exact recording SHA, consistent
zone/epoch/snapshot tick/watermark, consecutive fact sequences in tick order and a
complete trailer. Each fact has `{kind, entity, tick, sequence, data}`. Multiple
facts of one kind/entity/tick are valid; sequence identifies them. Unknown fields
or categories, duplicate JSON keys, non-exact numbers, invalid life bindings,
producer acceptance flags and incomplete files fail closed. Standalone RNG draws
are counted once; resolution slices must reference them in the same order without
duplicates. A scheduled pulse cannot be counted twice at the same entity/tick.

| Declaration | Actual C14 facts and independent check |
| --- | --- |
| `regen_pulses`, `moving_regen_pulses` | `regen_pulse`, owner and Standing/Moving posture; distinct scheduled ticks, plus validation of the raw formula and resource credit. |
| `regen_matches_oracle`, `moving_regen_matches_oracle` | `regen_pulse`; exact Fraction arithmetic over recorded base/stat/config inputs, ordered supported Funcs, whole grants, clamps and Q carries (game `Formulas.java:263,334,351,357,442`; binding D-R12). |
| `death_record_matches_fixture` | Player `death_record`; recorded XP span/loss fraction and actual recoverable loss, incarnation, eligibility/penalty draw (Nightfall `domain/zone/progression.rs:42`; binding A15/design §3). |
| `death_record_matches_before_disconnect` | Earlier reproduced `death_record` plus later `admission_reconstruction`; exact saved XP facts and rebinding to the new corpse incarnation. An initial snapshot or admission payload alone cannot prove before-disconnect equality. |
| `restore_did_not_reapply_hooks` | `admission_reconstruction` with nonempty retained timed effects; exact source/rank/relative install order, paused periodic phase, explicit persisted resources and absence of hook execution. Cache reconstruction alone does not prove a reconnect (datapack `pump/MaxHp.java:50,83,98`; binding R4). |
| `skill_hate_outcome_coherent` | NPC-subject `hate_delta` with the owner as contributor, correlated same-tick `formula_resolution`, then `hate_selection` or lethal ledger removal. Checks effectPoint, attacked notification, rolled-damage hate, ledger cap and tie selection (game `L2Character.java:4603`; `L2Attackable.java:571,606,658,666`; Nightfall `combat.rs:349`). |
| `skill_damage_matches_oracle` | `damage_formula` and its unique enclosing skill/rank/caster/target/life/tick `formula_resolution`. Complete projected inputs must match an independent committed vector; unsupported spread/modifier inputs fail. |
| `skill_effect_matches_oracle` | `effect_formula` and enclosing resolution; conditional land-rate and duration inputs must match independent vectors. Unconditional success cannot consume a success draw. |
| `stat_matches_oracle active\|removed` | Actual timed-effect presence/removal in `modifier_reconstruction`, then independent base-stat vectors and supported ordered folds. Unmodified bootstrap stats and unsupported affected stats cannot pass. |
| `resurrect_refund_matches_oracle` | Unique preceding `resurrection_offer`, caster resolution/WIT source profile and accepted `resurrection_refund`; source percent quirk, independent Math.round refund, one-time death-record consumption and incarnation transition (game `Formulas.java:1372`; `L2PcInstance.java:4702`). Declines are checked but cannot replace an accepted refund. |

The native regression builds `nightfall-replay` and exports the unchanged committed
`phase3-gameplay-v5.nfr` twice. It checks native positive standing-regeneration,
death, hate and accepted-refund facts, and keeps missing moving/admission/effect/stat
transitions and unsupported damage-vector inputs red. The recording starts from
an existing snapshot; earlier transitions are not invented. The tests also pass
real replay exports through both wrappers per unit, including `--no-replay`, using
stub bot/JUnit/health carriers. These are offline harness/recording checks, not
Unreal scenario or live-service acceptance.

The producer dependency is resolved. Full twelve-gate acceptance still requires
recordings containing each scenario's actual transition and independently covered
numeric inputs. Unsupported inputs never inherit a guessed oracle index or an
acceptance flag; synthetic or corrupted test rows are not acceptance evidence.
